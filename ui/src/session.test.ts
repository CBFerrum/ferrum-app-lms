import { describe, expect, it } from "vitest";
import { outcome, phase } from "./session";

const at = "2030-01-01T10:00:00.000Z";
const t0 = Date.parse(at);
const s = { starts_at: at, duration_minutes: 60 };

describe("a live session's phase", () => {
  it("is not open until ten minutes before it starts", () => {
    expect(phase(s, t0 - 11 * 60_000)).toBe("soon");
    expect(phase(s, t0 - 10 * 60_000)).toBe("open");
    expect(phase(s, t0 - 1)).toBe("open");
  });
  it("is live from its start to its end, then ended", () => {
    expect(phase(s, t0)).toBe("live");
    expect(phase(s, t0 + 59 * 60_000)).toBe("live");
    expect(phase(s, t0 + 60 * 60_000)).toBe("ended");
  });
  it("counts an hour when the length is missing", () => {
    expect(phase({ starts_at: at, duration_minutes: 0 }, t0 + 30 * 60_000)).toBe("live");
  });
});

describe("what became of the learner", () => {
  it("is credited, short or not yet read", () => {
    expect(outcome({ attended: true })).toBe("credited");
    expect(outcome({ attended: false })).toBe("short");
    expect(outcome({ attended: null })).toBe("pending");
  });
});
