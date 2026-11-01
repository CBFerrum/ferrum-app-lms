// Saves changes shortly after they stop, one request at a time, and again for what changed while
// it was away. A quiz uses it so a closed tab loses nothing; the attempt's own submit carries the
// full set of answers too, so a save that never got through costs nothing at the end.

export type SaveState = "idle" | "waiting" | "saving" | "failed";

export interface Saver<T> {
  /** Queues changes (a later change to the same key replaces an earlier one). */
  add(changes: Record<string, T>): void;
  /** Sends whatever is queued now; true when everything queued has been saved. */
  flush(): Promise<boolean>;
  /** Stops the timers: nothing more is sent. */
  stop(): void;
}

export function createSaver<T>(send: (changes: Record<string, T>) => Promise<void>, onState: (state: SaveState) => void = () => {}, delay = 600, retry = 4000): Saver<T> {
  let pending: Record<string, T> = {};
  let timer: ReturnType<typeof setTimeout> | undefined;
  let inFlight: Promise<boolean> | null = null;
  let stopped = false;
  const queued = () => Object.keys(pending).length > 0;

  const arm = (ms: number) => {
    clearTimeout(timer);
    timer = setTimeout(() => void run(), ms);
  };

  async function run(): Promise<boolean> {
    clearTimeout(timer);
    if (inFlight) return inFlight;
    if (!queued() || stopped) return !queued();
    inFlight = (async () => {
      let ok = true;
      while (queued() && !stopped) {
        const batch = pending;
        pending = {};
        onState("saving");
        try {
          await send(batch);
        } catch {
          // What changed meanwhile wins over what was in flight.
          pending = { ...batch, ...pending };
          ok = false;
          break;
        }
      }
      inFlight = null;
      if (!stopped) {
        onState(ok ? "idle" : "failed");
        if (!ok) arm(retry);
      }
      return ok;
    })();
    return inFlight;
  }

  return {
    add(changes) {
      if (stopped) return;
      pending = { ...pending, ...changes };
      onState("waiting");
      if (!inFlight) arm(delay);
    },
    async flush() {
      if (inFlight) await inFlight;
      return run();
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
