import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSaver, type SaveState } from "./saver";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function setup(send: (c: Record<string, string>) => Promise<void>) {
  const states: SaveState[] = [];
  const saver = createSaver<string>(send, (s) => states.push(s), 600, 4000);
  return { saver, states };
}

describe("saving answers as they change", () => {
  it("waits for a pause, then sends everything changed in one request", async () => {
    const sent: Record<string, string>[] = [];
    const { saver, states } = setup(async (c) => void sent.push(c));
    saver.add({ a: "1" });
    await vi.advanceTimersByTimeAsync(400);
    saver.add({ b: "2" });
    saver.add({ a: "3" });
    await vi.advanceTimersByTimeAsync(500);
    expect(sent).toEqual([]);
    await vi.advanceTimersByTimeAsync(200);
    expect(sent).toEqual([{ a: "3", b: "2" }]);
    expect(states.at(-1)).toBe("idle");
    expect(states[0]).toBe("waiting");
  });

  it("sends what changed while a request was away right after it, one request at a time", async () => {
    let release!: () => void;
    const sent: Record<string, string>[] = [];
    let live = 0;
    let most = 0;
    const { saver } = setup(async (c) => {
      sent.push(c);
      live++;
      most = Math.max(most, live);
      await new Promise<void>((r) => (release = r));
      live--;
    });
    saver.add({ a: "1" });
    await vi.advanceTimersByTimeAsync(700);
    saver.add({ a: "2", b: "x" });
    expect(sent).toEqual([{ a: "1" }]);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(sent).toEqual([{ a: "1" }, { a: "2", b: "x" }]);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(most).toBe(1);
  });

  it("flushes at once, for a submit", async () => {
    const sent: Record<string, string>[] = [];
    const { saver } = setup(async (c) => void sent.push(c));
    saver.add({ a: "1" });
    await expect(saver.flush()).resolves.toBe(true);
    expect(sent).toEqual([{ a: "1" }]);
    await expect(saver.flush()).resolves.toBe(true);
    expect(sent).toHaveLength(1);
  });

  it("keeps what failed to send, tries again later and prefers newer answers", async () => {
    let fail = true;
    const sent: Record<string, string>[] = [];
    const { saver, states } = setup(async (c) => {
      if (fail) throw new Error("offline");
      sent.push(c);
    });
    saver.add({ a: "1", b: "1" });
    await vi.advanceTimersByTimeAsync(700);
    expect(states.at(-1)).toBe("failed");
    saver.add({ a: "2" });
    fail = false;
    await vi.advanceTimersByTimeAsync(700);
    expect(sent).toEqual([{ a: "2", b: "1" }]);
    expect(states.at(-1)).toBe("idle");
  });

  it("retries by itself after a while", async () => {
    let fail = true;
    const sent: Record<string, string>[] = [];
    const { saver } = setup(async (c) => {
      if (fail) throw new Error("offline");
      sent.push(c);
    });
    saver.add({ a: "1" });
    await vi.advanceTimersByTimeAsync(700);
    fail = false;
    await vi.advanceTimersByTimeAsync(4100);
    expect(sent).toEqual([{ a: "1" }]);
    saver.stop();
  });

  it("sends nothing after it is stopped", async () => {
    const sent: Record<string, string>[] = [];
    const { saver } = setup(async (c) => void sent.push(c));
    saver.add({ a: "1" });
    saver.stop();
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toEqual([]);
  });
});
