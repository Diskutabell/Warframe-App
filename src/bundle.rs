//! The prepared game data as one file: CI downloads every source once a day, prepares and
//! checks it (`warframeproject build-catalog`), and publishes it on the repository's `catalog`
//! release as `catalog-<hash>.json.zst` with a small `manifest.json` naming it. A client then
//! asks one small file a day (revalidated with its ETag) and downloads the data only when CI
//! published new data, instead of asking some forty files from five hosts and repairing them
//! itself. When the bundle cannot be had, `gamedata::load` builds from the sources as before.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::gamedata::{self, Catalog, Prepared};
use crate::{config, net};

/// The layout of `Prepared`: a client only takes a bundle of the layout it was built for.
pub const SCHEMA: u32 = 1;
const RELEASE: &str = "https://github.com/Diskutabell/Warframe-App/releases/download/catalog/";
/// The bundle is about a tenth of the data; both are capped well above their size.
const MAX_BUNDLE: u64 = 128 * 1024 * 1024;
const MAX_DATA: u64 = 1024 * 1024 * 1024;
/// zstd's level 19: CI packs once, every client downloads.
const LEVEL: i32 = 19;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// When the data was prepared, epoch seconds.
    pub built: f64,
    /// The bundle's file name on the release.
    pub file: String,
    /// SHA-256 of the bundle, hex.
    pub sha256: String,
    pub size: u64,
    /// What it holds, for a look at the release page (and the checks).
    #[serde(default)]
    pub counts: serde_json::Map<String, serde_json::Value>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Packs prepared data; the manifest says what was packed.
pub fn encode(prepared: &Prepared, counts: serde_json::Map<String, serde_json::Value>) -> Result<(Vec<u8>, Manifest)> {
    let json = serde_json::to_vec(prepared)?;
    let packed = zstd::bulk::compress(&json, LEVEL).context("packing the game data")?;
    let sha256 = sha256_hex(&packed);
    let manifest = Manifest {
        schema: SCHEMA,
        built: prepared.built,
        file: format!("catalog-{}.json.zst", &sha256[..16]),
        size: packed.len() as u64,
        sha256,
        counts,
    };
    Ok((packed, manifest))
}

/// Unpacks a bundle, which must be the one the manifest names.
pub fn decode(packed: &[u8], manifest: &Manifest) -> Result<Prepared> {
    if sha256_hex(packed) != manifest.sha256 {
        bail!("the game data does not match its manifest");
    }
    let mut json = Vec::new();
    let decoder = zstd::stream::read::Decoder::new(packed).context("unpacking the game data")?;
    std::io::Read::read_to_end(&mut std::io::Read::take(decoder, MAX_DATA + 1), &mut json)
        .context("unpacking the game data")?;
    if json.len() as u64 > MAX_DATA {
        bail!("the game data unpacks to more than {MAX_DATA} bytes");
    }
    serde_json::from_slice(&json).context("reading the game data")
}

/// The manifest as a download reader: one of another layout is no use and is not cached.
fn manifest_reader() -> gamedata::Reader<Manifest> {
    Arc::new(|bytes: &[u8]| {
        let m: Manifest = serde_json::from_slice(bytes).context("invalid manifest")?;
        if m.schema != SCHEMA {
            bail!("the published game data has layout {}, this version reads {SCHEMA}", m.schema);
        }
        if !valid_file_name(&m.file) || m.sha256.len() != 64 {
            bail!("the manifest names no valid file");
        }
        Ok(m)
    })
}

/// A bundle's name as `encode` makes it: nothing that could leave the release folder or the
/// cache folder.
fn valid_file_name(name: &str) -> bool {
    name.strip_prefix("catalog-")
        .and_then(|n| n.strip_suffix(".json.zst"))
        .is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The prepared data CI published: the cached copy when the manifest still names it, else a
/// fresh download. Offline, or with GitHub out of reach, the cached copy (if any) is used.
pub async fn fetch(force: bool) -> Result<Prepared> {
    let cache = config::dirs().cache.join("catalog");
    std::fs::create_dir_all(&cache).with_context(|| format!("creating {}", cache.display()))?;
    let manifest_path = cache.join("manifest.json");
    let (manifest, _) = gamedata::fetch_cached(
        &format!("{RELEASE}manifest.json"),
        &manifest_path,
        force,
        &gamedata::Network,
        manifest_reader(),
    )
    .await?;
    let path = cache.join(&manifest.file);
    let packed = match tokio::fs::read(&path).await {
        Ok(bytes) if sha256_hex(&bytes) == manifest.sha256 => bytes,
        _ => download(&manifest, &path).await?,
    };
    let manifest = manifest.clone();
    let prepared = tokio::task::spawn_blocking(move || decode(&packed, &manifest))
        .await
        .map_err(|e| anyhow!("unpacking the game data failed: {e}"))??;
    forget_older(&cache, &path);
    Ok(prepared)
}

async fn download(manifest: &Manifest, path: &Path) -> Result<Vec<u8>> {
    let url = format!("{RELEASE}{}", manifest.file);
    let resp = net::get_with(&url, &[], Duration::from_secs(120), MAX_BUNDLE, net::Redirects::Follow).await?;
    if resp.status != 200 {
        return Err(net::HttpStatus(resp.status)).with_context(|| format!("GET {url}"));
    }
    if sha256_hex(&resp.body) != manifest.sha256 {
        bail!("the downloaded game data does not match its manifest");
    }
    config::write_atomic(path, &resp.body)?;
    Ok(resp.body)
}

/// Removes the bundles the manifest no longer names.
fn forget_older(cache: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name();
        if path != keep && valid_file_name(&name.to_string_lossy()) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// What prepared data must hold before it is published. Each line is one failed check; none
/// means it may go out. The floors sit well under what the data held when they were set
/// (2026-11-21: 810 items, 1,493 parts, 799 relics, 2,024 recipes, 354 nodes, 40 syndicates,
/// 11,945 inventory names), so an upstream change that empties or halves a part of it stops the release.
pub fn check(p: &Prepared, c: &Catalog) -> Vec<String> {
    let mut failed = Vec::new();
    let mut at_least = |what: &str, n: usize, floor: usize| {
        if n < floor {
            failed.push(format!("{what}: {n}, expected at least {floor}"));
        }
    };
    at_least("masterable items", c.items.len(), 700);
    at_least("parts", c.parts.len(), 1250);
    at_least("relics", c.relics.len(), 650);
    let with_rewards = c.relics.values().filter(|r| !r.rewards.is_empty()).count();
    at_least("relics with rewards", with_rewards, c.relics.len().saturating_sub(c.relics.len() / 20));
    at_least("unvaulted relics", c.relics.values().filter(|r| !r.vaulted).count(), 20);
    at_least("part recipes", c.part_recipes.len(), 1700);
    at_least("drop tables", p.drops.len(), gamedata::drop_table_count());
    at_least("star chart nodes", c.nodes.len(), 300);
    at_least("syndicates", c.syndicates.list.len(), 30);
    at_least("inventory names", c.inventory.len(), 10_000);
    at_least("arbitration hours ahead", c.arbitrations.slots.len(), 24 * 30);
    at_least("arbitration tiers", c.arbitrations.tiers.len(), 20);
    failed
}

/// The numbers the manifest shows.
pub fn counts(c: &Catalog) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in [
        ("items", c.items.len()),
        ("parts", c.parts.len()),
        ("relics", c.relics.len()),
        ("recipes", c.part_recipes.len()),
        ("nodes", c.nodes.len()),
        ("syndicates", c.syndicates.list.len()),
        ("inventory", c.inventory.len()),
        ("relics with rewards", c.relics.values().filter(|r| !r.rewards.is_empty()).count()),
        ("unvaulted relics", c.relics.values().filter(|r| !r.vaulted).count()),
        ("arbitration hours", c.arbitrations.slots.len()),
        ("arbitration tiers", c.arbitrations.tiers.len()),
    ] {
        m.insert(k.into(), v.into());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_comes_back_as_packed_and_only_with_its_manifest() {
        let mut p = Prepared { built: 5.0, ..Prepared::default() };
        p.files.insert("Warframes".into(), serde_json::json!([{"uniqueName": "/Ash", "name": "Ash"}]));
        let (packed, manifest) = encode(&p, serde_json::Map::new()).unwrap();
        assert!(valid_file_name(&manifest.file), "{}", manifest.file);
        let back = decode(&packed, &manifest).unwrap();
        assert_eq!((back.built, back.files["Warframes"][0]["name"].as_str()), (5.0, Some("Ash")));
        let mut other = manifest.clone();
        other.sha256 = sha256_hex(b"something else");
        assert!(decode(&packed, &other).is_err(), "a bundle the manifest does not name is refused");
    }

    #[test]
    fn a_manifest_of_another_layout_or_naming_another_path_is_refused() {
        let read = manifest_reader();
        let m = |schema: u32, file: &str| {
            serde_json::to_vec(&serde_json::json!({"schema": schema, "built": 1.0, "file": file,
                "sha256": "0".repeat(64), "size": 1}))
            .unwrap()
        };
        assert!(read(&m(SCHEMA, "catalog-0123456789abcdef.json.zst")).is_ok());
        assert!(read(&m(SCHEMA + 1, "catalog-0123456789abcdef.json.zst")).is_err());
        assert!(read(&m(SCHEMA, "../catalog-0123456789abcdef.json.zst")).is_err());
        assert!(read(&m(SCHEMA, "catalog-0123456789abcdeg.json.zst")).is_err());
    }
}
