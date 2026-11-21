// A page's data: fetched when the page is shown, again whenever the engine announces one of its
// topics (or a resync), and while the game data is still downloading. Everything stops when the
// page goes: a late answer never reaches another page.

import { api, isLoading } from "./api";
import { on, type Topic } from "./app.svelte";
import { debounce } from "./format";

export type Loaded<T> = {
  readonly data: T | null;
  readonly error: string | null;
  /** The game data is still downloading (with its error, if the download failed). */
  readonly downloading: string | true | null;
  refresh(): void;
};

export function loader<T>(fetch: string | ((signal: AbortSignal) => Promise<T>), topics: Topic[] = []): Loaded<T> {
  const get = typeof fetch === "string" ? (signal: AbortSignal) => api<T>(fetch, undefined, signal) : fetch;
  const s = $state<{ data: T | null; error: string | null; downloading: string | true | null }>({
    data: null,
    error: null,
    downloading: null,
  });
  let ctl: AbortController | null = null;
  let retry: ReturnType<typeof setTimeout> | undefined;

  async function run() {
    ctl?.abort();
    clearTimeout(retry);
    const mine = (ctl = new AbortController());
    try {
      const d = await get(mine.signal);
      if (mine.signal.aborted) return;
      if (isLoading(d)) {
        s.downloading = d.error ?? true;
        retry = setTimeout(run, 2500);
        return;
      }
      s.downloading = null;
      s.data = d;
      s.error = null;
    } catch (e) {
      if (mine.signal.aborted) return;
      s.error = (e as Error).message;
    }
  }
  const soon = debounce(run, 300);

  $effect(() => {
    run();
    const off = on([...topics, "resync", "account"], soon);
    return () => {
      off();
      ctl?.abort();
      clearTimeout(retry);
    };
  });

  return {
    get data() { return s.data; },
    get error() { return s.error; },
    get downloading() { return s.downloading; },
    refresh: run,
  };
}

/** Runs `fn` every `ms` while the component lives. */
export function every(fn: () => void, ms: number) {
  $effect(() => {
    const id = setInterval(fn, ms);
    return () => clearInterval(id);
  });
}
