// Numbers, times and names as the pages show them.

export const fmt = (n: number | null | undefined) => (n == null ? "–" : Number(n).toLocaleString());

/** "just now", "5 min ago", "3 h ago" for an epoch-seconds time in the past. */
export function ago(t: number | null | undefined): string {
  if (!t) return "never";
  const s = Math.round(Date.now() / 1000 - t);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

/** Seconds since the epoch from an ISO time or an epoch number. */
export function epoch(t: string | number | null | undefined): number {
  if (t == null || t === "") return NaN;
  if (typeof t === "number" || /^\d+(\.\d+)?$/.test(t)) return Number(t);
  return Date.parse(t) / 1000;
}

/** "2h 14m", "14m 05s", "now" from `now` (epoch seconds) until `t`. */
export function left(t: string | number | null | undefined, now = Date.now() / 1000): string {
  const s = Math.round(epoch(t) - now);
  if (!Number.isFinite(s)) return "";
  if (s <= 0) return "now";
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${String(m).padStart(2, "0")}m`;
  return `${m}m ${String(s % 60).padStart(2, "0")}s`;
}

/** The engine's copy of an image, scaled for a slot `px` CSS pixels wide (src/art.rs). */
export const artUrl = (src: string, px: number) =>
  `api/art?s=${Math.ceil(px * (window.devicePixelRatio || 1))}&u=${encodeURIComponent(src)}`;

export const wiki = (name: string) => `https://wiki.warframe.com/w/${encodeURIComponent(name.replace(/ /g, "_"))}`;

export const STATUS: Record<string, string> = { mastered: "Mastered", leveling: "Leveling", missing: "Not levelled" };
/** A farming task's colour (src/tasks.rs): red, yellow, green. */
export const TASK_STATE: Record<string, string> = { missing: "Missing", resources: "Needs resources", done: "Done" };
export const TASK_PILL: Record<string, string> = { missing: "bad", resources: "amber", done: "good" };

/** "MR 16", "MR 16-18" (a badge's band), "Legendary 2", "MR ?". */
export function rankText(a: { masteryRank: number | null; masteryRankMax: number | null }): string {
  if (a.masteryRank == null) return "MR ?";
  if (a.masteryRankMax != null && a.masteryRankMax !== a.masteryRank) return `MR ${a.masteryRank}-${a.masteryRankMax}`;
  return a.masteryRank > 30 ? `Legendary ${a.masteryRank - 30}` : `MR ${a.masteryRank}`;
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}
