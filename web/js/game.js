// Game art: textures from Warframe's own interface, which the engine fetches, scales and caches
// (src/art.rs). Paths are relative to /Lotus/Interface/Icons/ in the game's files. A texture
// that cannot be loaded removes itself, so the layout must not depend on any of them.

import { artUrl, countdown, esc } from "./lib.js";

const BASE = "https://browse.wf/Lotus/Interface/Icons/";

/** URL of a game texture, scaled for a slot `px` CSS pixels wide. */
export const tex = (path, px) => artUrl(BASE + path, px);

/** A game texture as an image of `px` by `px`. */
export const gimg = (path, px, cls = "", title = "") => (path
  ? `<img class="gi ${cls}" alt="" ${title ? `title="${esc(title)}"` : ""} width="${px}" height="${px}" loading="lazy" src="${esc(tex(path, px))}" onerror="this.remove()">`
  : "");

/** The same in a framed square, for the head of a card or a list row. */
export const emblem = (path, cls = "", title = "") => `<span class="emblem ${cls}" ${title ? `title="${esc(title)}"` : ""}>${gimg(path, cls.includes("sm") ? 22 : 36)}</span>`;

// ------------------------------------------------------------------ who and what

export const ICON = {
  mastery: "MasteryRankIcon.png",
  sun: "SunIconGraded.png",
  moon: "MoonIconGraded.png",
  alert: "StarchartIconsBakedGlow/Alert.png",
  daily: "DailyTimerIcon.png",
  weekly: "WeeklyChallengeIcon.png",
  elite: "WeeklyHardcoreChallengeIcon.png",
  steel: "SteelEssenceIcon.png",
  vault: "VaultIcon.png",
  darvo: "DarvoIcon.png",
  archon: "GenericArchonIcon.png",
};

export const PORTRAIT = {
  baro: "Player/BaroKiteerAvatar.png",
  nora: "Player/NoraNightGlyphA.png",
  teshin: "Player/VEDTeshin.png",
};

// The glyphs players can pick as avatars are the factions' own emblems, in colour.
const FACTIONS = [
  [/grineer/i, "Player/FactionGrineer.png", "#c9503f"],
  [/corpus/i, "Player/FactionCorpus.png", "#4f9bd6"],
  [/infest/i, "Player/FactionInfested.png", "#7fbf58"],
  [/orokin|corrupted/i, "Player/FactionOrokin.png", "#d4b26a"],
  [/murmur|man in the wall/i, "Player/FactionMurmur.png", "#b9a58a"],
  [/scaldra/i, "Player/FactionScaldra.png", "#d6c24a"],
  [/techrot/i, "Player/FactionTechrot.png", "#e0875e"],
  [/sentient/i, "SentientFactionIcon.png", "#c96a8f"],
  [/narmer/i, "Syndicates/FactionSigilNarmer.png", "#d4b26a"],
];
const faction = (name) => FACTIONS.find(([re]) => re.test(name || ""));
export const factionIcon = (name) => faction(name)?.[1];
export const factionColor = (name) => faction(name)?.[2] || "#5d6d7b";
/** The world state's name for a faction as the game shows it: "Man in the Wall" is the Murmur. */
export const foeName = (name) => (/man in the wall/i.test(name || "") ? "The Murmur" : name || "");
/** Faction emblem and name side by side. */
export const factionTag = (name, px = 18) => (name ? `<span class="ftag">${gimg(factionIcon(name), px)}${esc(foeName(name))}</span>` : "");

// An arbitration node's community tier (arbi.guide, through browse.wf), gold for S down to grey.
const TIER = { S: "gold", A: "good", B: "lotus", C: "", D: "missing", F: "missing" };
export const arbTier = (tier) => (tier
  ? `<span class="pill tier ${TIER[tier] ?? ""}" title="Community tier from arbi.guide: S is the best for farming">${esc(tier)} tier</span>`
  : "");

const SYNDICATES = [
  [/ostron/i, "Syndicates/FactionSigilEidolonCetusElder.png"],
  [/solaris/i, "Syndicates/FactionSigilSolarisUnited.png"],
  [/entrati/i, "Syndicates/FactionSigilEntrati.png"],
  [/cavia/i, "Syndicates/FactionSigilCavia.png"],
  [/holdfast|zariman/i, "Syndicates/Zariman/FactionSigilZariman.png"],
  [/hex/i, "Syndicates/FactionHex.png"],
  [/necraloid/i, "Syndicates/FactionSigilNecraloid.png"],
  [/quills/i, "Syndicates/FactionSigilEidolonHunter.png"],
  [/vent/i, "Syndicates/FactionVentKidz.png"],
  [/kahl/i, "Syndicates/FactionKahl.png"],
];
export const syndicateIcon = (name) => SYNDICATES.find(([re]) => re.test(name || ""))?.[1] || "Syndicates/NeutralSyndicateIcon.png";

const ARCHONS = [[/nira/i, "Nira"], [/amar/i, "Amar"], [/boreal/i, "Boreal"]];
/** The archon's portrait from the hunt's own panel. */
export const archonPortrait = (boss) => {
  const who = ARCHONS.find(([re]) => re.test(boss || ""))?.[1];
  return who ? `NarmerSortie/NarmerArchon${who}.png` : null;
};

// The codex's picture for each mission type (CodexGameModes); kinds without one draw nothing.
const MISSION_SCENES = {
  Assassination: "Assassination", Capture: "Capture", Defense: "Defense", Excavation: "Excavation",
  Extermination: "Exterminate", Hijack: "Hijack", Interception: "Interception", "Mobile Defense": "MobileDefense",
  Rescue: "Rescue", Sabotage: "Sabotage", Spy: "Spy", Survival: "Survival", Recovery: "Recovery",
};
export const missionScene = (mission) => (MISSION_SCENES[mission] ? `CodexGameModes/${MISSION_SCENES[mission]}.png` : null);

const RELICS = { Lith: "Lith", Meso: "Meso", Neo: "Neo", Axi: "Axi", Requiem: "Immortal" };
export const relicIcon = (era) => (RELICS[era] ? `Relics/Relic${RELICS[era]}A.png` : null);

/** The badge the game draws for a mastery rank: its own for rank 0, then one per three ranks
 *  (Rank1to3 ... Rank49to51, legendary ranks included). */
export function mrBadge(rank) {
  const dir = "MasteryRanks/NewMasteryRanks/";
  if (rank == null) return null;
  if (rank <= 0) return `${dir}Rank0_d.png`;
  const from = 1 + 3 * Math.floor((Math.min(rank, 51) - 1) / 3);
  return `${dir}Rank${from}to${from + 2}_d.png`;
}

// ------------------------------------------------------------------ open-world cycles

// Captura scene thumbnails: the only wide shots of these places among the interface textures.
const SCENES = "StoreIcons/Captura/";
const PLACES = {
  Cetus: "PlainsOfEidolonLandscape.png",
  "Orb Vallis": "VallisLandscape.png",
  "Cambion Drift": "DeimosCambionDriftLandscape.png",
  Zariman: "Zariman/ZarParkHub.png",
  Duviri: "Duviri/DuviriMainlandA.png",
  Earth: "GrineerForestFactory.png",
};
// The half of each cycle that is dark or cold: its picture is dimmed and it gets the moon.
const DARK = new Set(["night", "cold", "vome"]);

/** One cycle as a tile over a picture of the place. */
export function cycleTile(c, withPlace = true) {
  const state = String(c.state || "").toLowerCase();
  const scene = PLACES[c.name];
  const mark = c.name === "Zariman" ? gimg(factionIcon(state), 26, "state")
    : c.name === "Duviri" ? ""
    : gimg(DARK.has(state) ? ICON.moon : ICON.sun, 26, "state");
  return `<div class="cycle scenic ${DARK.has(state) ? "dim" : ""}">
      ${scene ? `<img class="bg" alt="" loading="lazy" src="${esc(tex(SCENES + scene, 256))}" onerror="this.remove()">` : ""}<div class="shade"></div>
      ${mark}
      <div class="n">${esc(c.name)}</div><div class="s">${esc(c.state)}</div>
      <div class="t">${countdown(c.expiry, "changing")}${withPlace && c.place ? ` <span class="faint">· ${esc(c.place)}</span>` : ""}</div></div>`;
}
