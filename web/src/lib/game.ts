// Game art: textures from Warframe's own interface, which the engine fetches, scales and caches
// (src/art.rs). Paths are relative to /Lotus/Interface/Icons/ in the game's files. A texture
// that cannot be loaded removes itself, so the layout must not depend on any of them.

import { artUrl } from "./format";

const BASE = "https://browse.wf/Lotus/Interface/Icons/";

/** URL of a game texture, scaled for a slot `px` CSS pixels wide. */
export const tex = (path: string, px: number) => artUrl(BASE + path, px);

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
} as const;

export const PORTRAIT = {
  baro: "Player/BaroKiteerAvatar.png",
  nora: "Player/NoraNightGlyphA.png",
  teshin: "Player/VEDTeshin.png",
} as const;

// The glyphs players can pick as avatars are the factions' own emblems, in colour.
const FACTIONS: [RegExp, string, string][] = [
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
const faction = (name: string | null | undefined) => FACTIONS.find(([re]) => re.test(name || ""));
export const factionIcon = (name: string | null | undefined) => faction(name)?.[1] ?? null;
export const factionColor = (name: string | null | undefined) => faction(name)?.[2] || "#5d6d7b";
/** The world state's name for a faction as the game shows it: "Man in the Wall" is the Murmur. */
export const foeName = (name: string | null | undefined) => (/man in the wall/i.test(name || "") ? "The Murmur" : name || "");

// An arbitration node's community tier (arbi.guide, through browse.wf): S is worth doing now (solid
// amber), A nearly so (tinted), B and C plain, D and F outlined.
export const TIER_CLASS: Record<string, string> = { S: "gold", A: "amber", B: "", C: "", D: "missing", F: "missing" };

const SYNDICATES: [RegExp, string][] = [
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
export const syndicateIcon = (name: string | null | undefined) =>
  SYNDICATES.find(([re]) => re.test(name || ""))?.[1] || "Syndicates/NeutralSyndicateIcon.png";

const ARCHONS: [RegExp, string][] = [[/nira/i, "Nira"], [/amar/i, "Amar"], [/boreal/i, "Boreal"]];
/** The archon's portrait from the hunt's own panel. */
export function archonPortrait(boss: string | null | undefined): string | null {
  const who = ARCHONS.find(([re]) => re.test(boss || ""))?.[1];
  return who ? `NarmerSortie/NarmerArchon${who}.png` : null;
}

// The codex's picture for each mission type (CodexGameModes); kinds without one draw nothing.
const MISSION_SCENES: Record<string, string> = {
  Assassination: "Assassination", Capture: "Capture", Defense: "Defense", Excavation: "Excavation",
  Extermination: "Exterminate", Hijack: "Hijack", Interception: "Interception", "Mobile Defense": "MobileDefense",
  Rescue: "Rescue", Sabotage: "Sabotage", Spy: "Spy", Survival: "Survival", Recovery: "Recovery",
};
export const missionScene = (mission: string | null | undefined) =>
  mission && MISSION_SCENES[mission] ? `CodexGameModes/${MISSION_SCENES[mission]}.png` : null;

const RELICS: Record<string, string> = { Lith: "Lith", Meso: "Meso", Neo: "Neo", Axi: "Axi", Requiem: "Immortal" };
export const relicIcon = (era: string | null | undefined) => (era && RELICS[era] ? `Relics/Relic${RELICS[era]}A.png` : null);

/** The badge the game draws for a mastery rank: its own for rank 0, then one per three ranks
 *  (Rank1to3 ... Rank49to51, legendary ranks included). */
export function mrBadge(rank: number | null | undefined): string | null {
  const dir = "MasteryRanks/NewMasteryRanks/";
  if (rank == null) return null;
  if (rank <= 0) return `${dir}Rank0_d.png`;
  const from = 1 + 3 * Math.floor((Math.min(rank, 51) - 1) / 3);
  return `${dir}Rank${from}to${from + 2}_d.png`;
}

// Captura scene thumbnails: the only wide shots of these places among the interface textures.
const SCENES = "StoreIcons/Captura/";
const PLACES: Record<string, string> = {
  Cetus: "PlainsOfEidolonLandscape.png",
  "Orb Vallis": "VallisLandscape.png",
  "Cambion Drift": "DeimosCambionDriftLandscape.png",
  Zariman: "Zariman/ZarParkHub.png",
  Duviri: "Duviri/DuviriMainlandA.png",
  Earth: "GrineerForestFactory.png",
};
export const placeScene = (name: string) => (PLACES[name] ? SCENES + PLACES[name] : null);
/** The half of each cycle that is dark or cold: its picture is dimmed and it gets the moon. */
export const DARK = new Set(["night", "cold", "vome"]);
