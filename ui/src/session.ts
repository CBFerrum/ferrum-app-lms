// What a live session is at a moment, and what became of the learner's part in it. Pure, with the
// time passed in, so they are tested without a clock.

import type { LiveSession } from "./types";

/** Minutes before the start from which a session is open to join. */
export const OPENS_BEFORE = 10;

export type SessionPhase = "soon" | "open" | "live" | "ended";

/** Where a session is at `now`: "soon" until it opens, "open" in the minutes before its start,
 *  "live" while it runs, "ended" after. */
export function phase(s: Pick<LiveSession, "starts_at" | "duration_minutes">, now: number): SessionPhase {
  const start = Date.parse(s.starts_at);
  if (Number.isNaN(start)) return "soon";
  const end = start + (s.duration_minutes || 60) * 60_000;
  if (now >= end) return "ended";
  if (now >= start) return "live";
  return now >= start - OPENS_BEFORE * 60_000 ? "open" : "soon";
}

export type Outcome = "credited" | "short" | "pending";

/** What the server recorded of the learner after the session: attended (credited), was there too
 *  little (`attended` false), or nothing yet (read about ten minutes after the end). */
export function outcome(s: Pick<LiveSession, "attended">): Outcome {
  return s.attended === true ? "credited" : s.attended === false ? "short" : "pending";
}
