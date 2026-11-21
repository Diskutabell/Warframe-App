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

/// A manifest as published. Its layout is checked by the caller: a manifest of another layout
/// means this version cannot read what CI publishes now.
fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    let m: Manifest = serde_json::from_slice(bytes).context("invalid manifest")?;
    if !valid_file_name(&m.file) || m.sha256.len() != 64 {
        bail!("the manifest names no valid file");
    }
    Ok(m)
}

/// A bundle's name as `encode` makes it: nothing that could leave the release folder or the
/// cache folder.
fn valid_file_name(name: &str) -> bool {
    name.strip_prefix("catalog-")
        .and_then(|n| n.strip_suffix(".json.zst"))
        .is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// How long a manifest is used before GitHub is asked again (revalidated with its ETag, so an
/// unchanged one costs a 304).
const MANIFEST_AGE: Duration = Duration::from_secs(6 * 3600);

/// The prepared data CI published. The cache holds one manifest and the bundle it names, and
/// a new pair replaces them only once the new bundle has been downloaded, checked against its
/// SHA-256 and unpacked: whatever goes wrong with a newer one, the last good pair stays in use.
/// A manifest of another layout is an error (the caller then builds from the sources): this
/// version is too old or too new for what CI publishes now.
pub async fn fetch(force: bool) -> Result<Prepared> {
    let cache = config::dirs().cache.join("catalog");
    std::fs::create_dir_all(&cache).with_context(|| format!("creating {}", cache.display()))?;
    let (manifest_path, etag_path) = (cache.join("manifest.json"), cache.join("manifest.etag"));
    // The pair in use, if any.
    let held = std::fs::read(&manifest_path).ok().and_then(|b| parse_manifest(&b).ok()).filter(|m| m.schema == SCHEMA);
    let age = std::fs::metadata(&manifest_path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
    if let Some(m) = held.as_ref().filter(|_| !force && age.is_some_and(|a| a < MANIFEST_AGE)) {
        match unpack_cached(&cache, m).await {
            Ok(p) => return Ok(p),
            Err(e) => eprintln!("the cached game data is unusable, asking for it again: {e:#}"),
        }
    }
    let etag = held.as_ref().and_then(|_| std::fs::read_to_string(&etag_path).ok());
    let headers: Vec<(&str, &str)> = etag.iter().map(|t| ("If-None-Match", t.as_str())).collect();
    let answer = net::get_with(
        &format!("{RELEASE}manifest.json"),
        &headers,
        Duration::from_secs(30),
        64 * 1024,
        net::Redirects::Follow,
    )
    .await;
    let fall_back = |why: anyhow::Error| async {
        match &held {
            Some(m) => {
                eprintln!("using the cached game data: {why:#}");
                unpack_cached(&cache, m).await
            }
            None => Err(why),
        }
    };
    let resp = match answer {
        Ok(resp) if resp.status == 304 && held.is_some() => {
            // Unchanged: the pair in use is good for another while.
            if let Ok(f) = std::fs::File::options().append(true).open(&manifest_path) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
            return fall_back(anyhow!("unchanged")).await;
        }
        Ok(resp) if resp.status == 200 => resp,
        Ok(resp) => return fall_back(anyhow!("HTTP {} for the manifest", resp.status)).await,
        Err(e) => return fall_back(e).await,
    };
    let manifest = match parse_manifest(&resp.body) {
        Ok(m) => m,
        Err(e) => return fall_back(e).await,
    };
    if manifest.schema != SCHEMA {
        bail!("the published game data has layout {}, this version reads {SCHEMA}", manifest.schema);
    }
    let path = cache.join(&manifest.file);
    let fresh = async {
        let (packed, downloaded) = match tokio::fs::read(&path).await {
            Ok(bytes) if sha256_hex(&bytes) == manifest.sha256 => (bytes, false),
            _ => (download(&manifest).await?, true),
        };
        let (m, packed) = (manifest.clone(), Arc::new(packed));
        let bytes = Arc::clone(&packed);
        let prepared = tokio::task::spawn_blocking(move || decode(&bytes, &m))
            .await
            .map_err(|e| anyhow!("unpacking the game data failed: {e}"))??;
        anyhow::Ok((prepared, downloaded.then_some(packed)))
    };
    match fresh.await {
        Ok((prepared, downloaded)) => {
            // Only now is the new pair the one in use.
            if let Some(packed) = downloaded {
                config::write_atomic(&path, &packed)?;
            }
            config::write_atomic(&manifest_path, &resp.body)?;
            match &resp.etag {
                Some(tag) => config::write_atomic(&etag_path, tag.as_bytes())?,
                None => {
                    let _ = std::fs::remove_file(&etag_path);
                }
            }
            forget_older(&cache, &path);
            Ok(prepared)
        }
        Err(e) => fall_back(e.context(format!("the new game data ({})", manifest.file))).await,
    }
}

/// The cached bundle a manifest names, unpacked.
async fn unpack_cached(cache: &Path, manifest: &Manifest) -> Result<Prepared> {
    let packed = tokio::fs::read(cache.join(&manifest.file)).await.context("reading the cached game data")?;
    let m = manifest.clone();
    tokio::task::spawn_blocking(move || decode(&packed, &m))
        .await
        .map_err(|e| anyhow!("unpacking the game data failed: {e}"))?
}

async fn download(manifest: &Manifest) -> Result<Vec<u8>> {
    let url = format!("{RELEASE}{}", manifest.file);
    let resp = net::get_with(&url, &[], Duration::from_secs(120), MAX_BUNDLE, net::Redirects::Follow).await?;
    if resp.status != 200 {
        return Err(net::HttpStatus(resp.status)).with_context(|| format!("GET {url}"));
    }
    if sha256_hex(&resp.body) != manifest.sha256 {
        bail!("the downloaded game data does not match its manifest");
    }
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
/// 11,945 inventory entries of which 11,337 named, 2,022 names with drops, 169 nodes with mastery),
/// so an upstream change that empties or halves a part of it stops the release.
pub fn check(p: &Prepared, c: &Catalog) -> Vec<String> {
    // Every source, or nothing is published: a source that failed now is there tomorrow.
    let mut failed: Vec<String> = p.missing.iter().map(|m| format!("source missing: {m}")).collect();
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
    // Counted by what they hold, not by rows: an empty table or dictionary still makes rows.
    at_least("names with drop sources", c.drops.names().count(), 1700);
    at_least("named star chart nodes", c.nodes.values().filter(|n| !n.name.is_empty()).count(), 300);
    at_least("star chart nodes with mastery", c.node_xp.len(), 140);
    at_least("syndicates", c.syndicates.list.len(), 30);
    at_least("named inventory entries", c.inventory.values().filter(|e| e.name.is_some()).count(), 9500);
    at_least("inventory exports", p.held.len(), crate::inventory::EXPORTS.len());
    at_least("mods", p.files.get("Mods").and_then(|m| m.as_array()).map_or(0, Vec::len), 1000);
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
        ("names with drops", c.drops.names().count()),
        ("named nodes", c.nodes.values().filter(|n| !n.name.is_empty()).count()),
        ("nodes with mastery", c.node_xp.len()),
        ("named inventory", c.inventory.values().filter(|e| e.name.is_some()).count()),
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
    fn a_source_that_could_not_be_had_stops_the_release() {
        let p = Prepared { missing: vec!["mod catalog unavailable: HTTP 503".into()], ..Prepared::default() };
        let failed = check(&p, &Catalog::default());
        assert!(failed.iter().any(|f| f == "source missing: mod catalog unavailable: HTTP 503"), "{failed:?}");
        // An old bundle, written before sources were recorded, reads as complete.
        let old: Prepared = serde_json::from_value(serde_json::json!({"built": 1.0, "files": {}, "drops": {},
            "regions": null, "syndicate_export": null, "dict": null, "held": {},
            "arbitrations": {"slots": [], "tiers": {}}}))
        .unwrap();
        assert!(old.missing.is_empty());
    }

    #[test]
    fn a_manifest_naming_another_path_is_refused_and_its_layout_is_kept() {
        let read = parse_manifest;
        let m = |schema: u32, file: &str| {
            serde_json::to_vec(&serde_json::json!({"schema": schema, "built": 1.0, "file": file,
                "sha256": "0".repeat(64), "size": 1}))
            .unwrap()
        };
        assert!(read(&m(SCHEMA, "catalog-0123456789abcdef.json.zst")).is_ok());
        // Another layout is read, so that the caller can tell "too old" from "broken".
        assert_eq!(read(&m(SCHEMA + 1, "catalog-0123456789abcdef.json.zst")).unwrap().schema, SCHEMA + 1);
        assert!(read(&m(SCHEMA, "../catalog-0123456789abcdef.json.zst")).is_err());
        assert!(read(&m(SCHEMA, "catalog-0123456789abcdeg.json.zst")).is_err());
    }
}
