// A quiz sitting: the answers given, what is still open, the clock and the verdict. Pure, so the
// rules a learner meets (when to warn, what counts as answered) are tested without a browser. The
// server holds the answer keys and the real clock; this only shows what it says.

import type { Attempt, AttemptAnswer, QuestionType, QuizState, Response } from "./types";

export type Answers = Record<string, Response>;

export const isChoice = (type: QuestionType) => type !== "Short Answer";
export const isMultiple = (type: QuestionType) => type === "Multiple Choice";

/** The answers an attempt already holds (it resumes where the learner stopped). */
export function answersOf(attempt: Pick<Attempt, "answers">): Answers {
  return Object.fromEntries(attempt.answers.map((a) => [a.question, a.response ?? null]));
}

export function isAnswered(response: Response | undefined): boolean {
  if (Array.isArray(response)) return response.length > 0;
  return typeof response === "string" && response.trim().length > 0;
}

/** Places (zero-based) of the questions with no answer yet. */
export function unanswered(attempt: Pick<Attempt, "answers">, answers: Answers): number[] {
  return attempt.answers.flatMap((q, i) => (isAnswered(answers[q.question]) ? [] : [i]));
}

/** The response after choosing an option: the one chosen, or the option added to or taken out of
 *  those chosen (kept in the order the options are shown). */
export function choose(q: Pick<AttemptAnswer, "question_type" | "options">, current: Response | undefined, key: string): Response {
  if (!isMultiple(q.question_type)) return [key];
  const have = new Set(Array.isArray(current) ? current : []);
  if (!have.delete(key)) have.add(key);
  const kept = (q.options ?? []).map((o) => o.key).filter((k) => have.has(k));
  return kept.length ? kept : null;
}

export function chosen(response: Response | undefined, key: string): boolean {
  return Array.isArray(response) && response.includes(key);
}

// ───────────────────────────── the clock ─────────────────────────────

/** A moment on the device's two clocks. */
export interface Reading {
  /** The steady clock (performance.now): it never jumps, but it stops while the device sleeps. */
  mono: number;
  /** The date and time (Date.now): it keeps going through sleep, but the person can set it. */
  wall: number;
}

export const reading = (): Reading => ({ mono: performance.now(), wall: Date.now() });

/** Seconds left now, counted from what the server said when it said it (`left`, read at `at`). The
 *  device's idea of the date plays no part except that time passing on either clock counts, so a
 *  laptop that slept does not hand back minutes, and a clock set backwards does not either. */
export function remaining(left: number | null, at: Reading, now: Reading): number | null {
  if (left === null) return null;
  return Math.max(0, left - Math.max(now.mono - at.mono, now.wall - at.wall, 0) / 1000);
}

export type Tone = "calm" | "warn" | "urgent";

/** How the timer looks: calm, then amber with a quarter of the time (at most five minutes) left,
 *  then red for the last tenth (at most a minute, at least ten seconds). */
export function timerTone(secondsLeft: number, totalSeconds: number): Tone {
  if (secondsLeft <= Math.max(10, Math.min(60, totalSeconds * 0.1))) return "urgent";
  if (secondsLeft <= Math.min(300, totalSeconds * 0.25)) return "warn";
  return "calm";
}

/** The milestone (seconds left) the clock passed between two readings, to say aloud once. */
export function milestone(before: number, after: number, total: number): number | null {
  return [300, 60, 10].find((m) => m < total && before > m && after <= m) ?? null;
}

export function milestoneText(seconds: number): string {
  return seconds >= 60 ? `${seconds / 60} ${seconds === 60 ? "minute" : "minutes"} left` : `${seconds} seconds left`;
}

// ───────────────────────────── the verdict ─────────────────────────────

export type Verdict = "passed" | "failed" | "timed-out";

export function verdict(attempt: Pick<Attempt, "passed" | "status">): Verdict {
  return attempt.passed ? "passed" : attempt.status === "Timed Out" ? "timed-out" : "failed";
}

export function rightCount(attempt: Pick<Attempt, "answers">): number {
  return attempt.answers.filter((a) => a.is_correct).length;
}

/** Whether the quiz showed the correct answers with the result (its `show_answers` rule). */
export function revealed(attempt: Pick<Attempt, "answers">): boolean {
  return attempt.answers.some((a) => a.correct_answer !== null && a.correct_answer !== undefined);
}

/** The text of the option keys a response names, or the short answer itself. */
export function responseText(a: Pick<AttemptAnswer, "options" | "response">): string[] {
  const r = a.response;
  if (Array.isArray(r)) return r.map((k) => a.options?.find((o) => o.key === k)?.text ?? "").filter(Boolean);
  return typeof r === "string" && r ? [r] : [];
}

// ───────────────────────────── before the quiz ─────────────────────────────

/** Whether the learner may begin (or go on with) a sitting. */
export function canStart(state: Pick<QuizState, "max_attempts" | "attempts_left" | "in_progress">): boolean {
  return !!state.in_progress || state.max_attempts === 0 || (state.attempts_left ?? 0) > 0;
}

export function attemptsText(state: Pick<QuizState, "max_attempts" | "attempts_used" | "attempts_left">): string {
  if (state.max_attempts === 0) return state.attempts_used ? `${state.attempts_used} so far · no limit` : "No limit on attempts";
  const left = state.attempts_left ?? 0;
  if (left === 0) return `All ${state.max_attempts} attempts used`;
  return `${left} of ${state.max_attempts} ${state.max_attempts === 1 ? "attempt" : "attempts"} left`;
}
