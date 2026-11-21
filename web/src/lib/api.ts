// The engine's local API. Every answer is typed from the Rust side (src/types, made by ts-rs).

import { toast } from "./toasts.svelte";

export class ApiError extends Error {}

/** GET `path` (no body) or POST `body` to `/api/<path>`. Throws with the engine's own message. */
export async function api<T>(path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const init: RequestInit =
    body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
  // Called off already: nothing is asked.
  signal?.throwIfAborted();
  const ctl = new AbortController();
  const slow = /scan$/.test(path);
  const timer = setTimeout(() => ctl.abort(), slow ? 60000 : 20000);
  signal?.addEventListener("abort", () => ctl.abort(), { once: true });
  let r: Response;
  try {
    r = await fetch("/api/" + path, { ...init, signal: ctl.signal });
  } catch (e) {
    clearTimeout(timer);
    if (signal?.aborted) throw e;
    const err = e as Error;
    throw new ApiError(err.name === "AbortError" ? `No answer from /api/${path}.` : `Can't reach the engine: ${err.message}`);
  }
  let j: unknown;
  try {
    j = await r.json();
  } catch (e) {
    // Called off while the body arrived: the caller's abort, or our time limit - never an empty answer.
    if (signal?.aborted) throw e;
    if (ctl.signal.aborted) throw new ApiError(`No answer from /api/${path}.`);
    if (r.ok) throw new ApiError(`The engine's answer to /api/${path} is not JSON.`);
    j = {};
  } finally {
    clearTimeout(timer);
  }
  if (!r.ok) throw new ApiError((j as { error?: string }).error || `HTTP ${r.status}`);
  return j as T;
}

/** A page's data while the game data is still downloading. */
export type Loading = { loading: true; error: string | null };

export function isLoading(v: unknown): v is Loading {
  return typeof v === "object" && v !== null && (v as { loading?: unknown }).loading === true;
}

/** Calls the API and reports a failure as a toast; the answer, or null after a failure. */
export async function act<T = unknown>(path: string, body?: unknown, okMsg?: string): Promise<T | null> {
  try {
    const r = await api<T>(path, body ?? {});
    if (okMsg) toast(okMsg);
    return r;
  } catch (e) {
    toast((e as Error).message, true);
    return null;
  }
}

/** Reads a JSON file the user picks; the parsed value, or null. */
export function pickJson(): Promise<unknown | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,application/json";
    input.onchange = async () => {
      const f = input.files?.[0];
      if (!f) return resolve(null);
      try {
        resolve(JSON.parse(await f.text()));
      } catch {
        toast("That file is not valid JSON.", true);
        resolve(null);
      }
    };
    input.click();
  });
}
