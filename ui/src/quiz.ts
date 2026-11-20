// A quiz sitting: the answers given, what is still open, the clock and the verdict. Pure, so the
// rules a learner meets (when to warn, what counts as answered) are tested without a browser. The
// server holds the answer keys and the real clock; this only shows what it says.

import type { Attempt, AttemptAnswer, CorrectAnswer, QuestionType, QuizState, Response } from "./types";
import { t } from "@ferrum/client";

export type Answers = Record<string, Response>;

export type Kind = "choice" | "short" | "numeric" | "ordering" | "matching" | "blanks" | "essay" | "file";

/** Which widget a question type is answered with. */
export function kind(type: QuestionType): Kind {
  switch (type) {
    case "Short Answer": return "short";
    case "Numeric": return "numeric";
    case "Ordering": return "ordering";
    case "Matching": return "matching";
    case "Fill in the Blanks": return "blanks";
    case "Essay": return "essay";
    case "File Upload": return "file";
    default: return "choice";
  }
}
export const isMultiple = (type: QuestionType) => type === "Multiple Choice";

/** The answers an attempt already holds (it resumes where the learner stopped). */
export function answersOf(attempt: Pick<Attempt, "answers">): Answers {
  return Object.fromEntries(attempt.answers.map((a) => [a.question, a.response ?? null]));
}

export function isAnswered(response: Response | undefined): boolean {
  if (Array.isArray(response)) return response.some((r) => typeof r === "string" && r.length > 0);
  if (response && typeof response === "object") return Object.keys(response).length > 0;
  return typeof response === "string" && response.trim().length > 0;
}

/** The order after moving the key at `from` to place `to` (every key stays once). */
export function moved(keys: string[], from: number, to: number): string[] {
  if (from === to || from < 0 || to < 0 || from >= keys.length || to >= keys.length) return keys;
  const out = [...keys];
  out.splice(to, 0, ...out.splice(from, 1));
  return out;
}

/** The pairs after setting (or, with `null`, clearing) the match of one left-hand item; `null` when none is left. */
export function matched(current: Response | undefined, left: string, right: string | null): Response {
  const pairs: Record<string, string> = current && typeof current === "object" && !Array.isArray(current) ? { ...current } : {};
  if (right) pairs[left] = right;
  else delete pairs[left];
  return Object.keys(pairs).length ? pairs : null;
}

/** The blanks after typing `text` in blank `index` (a list of exactly `blanks` entries); `null` when all are empty. */
export function filled(current: Response | undefined, blanks: number, index: number, text: string): Response {
  const list: (string | null)[] = Array.isArray(current) && current.length === blanks ? [...current] : Array.from({ length: blanks }, () => null);
  list[index] = text.trim() === "" ? null : text;
  return list.every((x) => x === null) ? null : list;
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
  return seconds >= 60 ? (seconds === 60 ? t("1 minute left") : t("{minutes} minutes left", { minutes: seconds / 60 })) : t("{seconds} seconds left", { seconds });
}

// ───────────────────────────── the verdict ─────────────────────────────

export type Verdict = "passed" | "failed" | "timed-out" | "waiting";

export function verdict(attempt: Pick<Attempt, "passed" | "status">): Verdict {
  if (attempt.status === "Awaiting Grading") return "waiting";
  return attempt.passed ? "passed" : attempt.status === "Timed Out" ? "timed-out" : "failed";
}

export function rightCount(attempt: Pick<Attempt, "answers">): number {
  return attempt.answers.filter((a) => a.is_correct).length;
}

/** Whether the quiz showed the correct answers with the result (its `show_answers` rule). */
export function revealed(attempt: Pick<Attempt, "answers">): boolean {
  return attempt.answers.some((a) => a.correct_answer !== null && a.correct_answer !== undefined);
}

/** What the learner answered, as lines of text: option keys read as their texts, pairs as "item → answer", blanks as the
 *  filled ones; a file as the words "A file" (the page draws its link beside it). */
export function responseText(a: Pick<AttemptAnswer, "options" | "choices" | "response">): string[] {
  const r = a.response;
  const opt = (k: string) => a.options?.find((o) => o.key === k)?.text ?? "";
  if (Array.isArray(r)) return (r as (string | null)[]).filter((k): k is string => typeof k === "string" && k !== "").map((k) => (a.options ? opt(k) : k)).filter(Boolean);
  if (r && typeof r === "object") return Object.entries(r).map(([l, x]) => `${opt(l)} → ${a.choices?.find((c) => c.key === x)?.text ?? ""}`);
  if (typeof r === "string" && r.startsWith("/files/")) return [t("A file")];
  return typeof r === "string" && r ? [r] : [];
}

/** The correct answer a revealed row carries, as lines of text. */
export function correctText(c: CorrectAnswer | undefined): string[] {
  if (!c) return [];
  if (!Array.isArray(c)) return c.answer ? [Number(c.tolerance) ? t("{answer} (± {tolerance})", { answer: c.answer, tolerance: c.tolerance }) : c.answer] : [];
  return c.map((x) => (typeof x === "string" ? x : Array.isArray(x) ? x.join(" / ") : `${x.item} → ${x.matches}`));
}

// ───────────────────────────── before the quiz ─────────────────────────────

/** Whether the learner may begin (or go on with) a sitting. */
export function canStart(state: Pick<QuizState, "max_attempts" | "attempts_left" | "in_progress">): boolean {
  return !!state.in_progress || state.max_attempts === 0 || (state.attempts_left ?? 0) > 0;
}

export function attemptsText(state: Pick<QuizState, "max_attempts" | "attempts_used" | "attempts_left">): string {
  if (state.max_attempts === 0) return state.attempts_used ? t("{attempts_used} so far · no limit", { attempts_used: state.attempts_used }) : t("No limit on attempts");
  const left = state.attempts_left ?? 0;
  if (left === 0) return t("All {max_attempts} attempts used", { max_attempts: state.max_attempts });
  return (state.max_attempts === 1 ? t("{left} of 1 attempt left", { left }) : t("{left} of {max} attempts left", { left, max: state.max_attempts }));
}
