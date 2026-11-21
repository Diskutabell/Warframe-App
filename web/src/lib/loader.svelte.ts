// A page's data: fetched when the page is shown, again whenever the engine announces one of its
// topics (or a resync), and while the game data is still downloading. Everything stops when the
// page goes: a late answer never reaches another page.

import { api, isLoading } from "./api";
import { on, type Topic } from "./app.svelte";

export type Loaded<T> = {
  readonly data: T | null;
  readonly error: string | null;
  /** The game data is still downloading (with its error, if the download failed). */
  readonly downloading: string | true | null;
  refresh(): Promise<void>;
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
  let later: ReturnType<typeof setTimeout> | undefined;
  // Once the page is gone nothing of it runs again: not a late topic, not a retry.
  let alive = true;

  async function run() {
    if (!alive) return;
    ctl?.abort();
    clearTimeout(retry);
    clearTimeout(later);
    const mine = (ctl = new AbortController());
    try {
      const d = await get(mine.signal);
      if (mine.signal.aborted || !alive) return;
      if (isLoading(d)) {
        s.downloading = d.error ?? true;
        retry = setTimeout(run, 2500);
        return;
      }
      s.downloading = null;
      s.data = d;
      s.error = null;
    } catch (e) {
      if (mine.signal.aborted || !alive) return;
      s.error = (e as Error).message;
    }
  }
  /** A burst of topics is one reload. What was asked before the change can only be older. */
  function soon() {
    ctl?.abort();
    clearTimeout(retry);
    clearTimeout(later);
    later = setTimeout(run, 300);
  }

  $effect(() => {
    alive = true;
    run();
    const off = on([...topics, "resync"], soon);
    // Another account: what the page shows is not its, so it goes at once, and nothing of the
    // last account's request may land.
    const offAccount = on("account", () => {
      s.data = null;
      s.error = null;
      s.downloading = null;
      run();
    });
    return () => {
      alive = false;
      off();
      offAccount();
      ctl?.abort();
      clearTimeout(retry);
      clearTimeout(later);
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
