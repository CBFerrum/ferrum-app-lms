import { describe, expect, it } from "vitest";
import { answersOf, attemptsText, canStart, choose, chosen, isAnswered, milestone, milestoneText, remaining, responseText, revealed, rightCount, timerTone, unanswered, verdict } from "./quiz";
import type { Attempt, AttemptAnswer, QuizState } from "./types";

const question = (id: string, type: AttemptAnswer["question_type"] = "Single Choice", over: Partial<AttemptAnswer> = {}): AttemptAnswer => ({
  question: id,
  question_text: `Question ${id}`,
  question_type: type,
  options: type === "Short Answer" ? null : [{ key: "k1", text: "One" }, { key: "k2", text: "Two" }, { key: "k3", text: "Three" }],
  points: "1.00",
  response: null,
  is_correct: false,
  points_awarded: null,
  correct_answer: null,
  explanation: null,
  ...over,
});

const attempt = (answers: AttemptAnswer[], over: Partial<Attempt> = {}): Attempt => ({
  id: "att",
  quiz: "q",
  quiz_title: "Quiz",
  attempt_no: 1,
  status: "In Progress",
  started_at: "2026-11-01T10:00:00.000Z",
  deadline: null,
  time_limit_minutes: 0,
  passing_percentage: "70.00",
  max_score: "3.00",
  passed: false,
  answers,
  seconds_left: null,
  ...over,
});

describe("what counts as an answer", () => {
  it("is a chosen option or some text, not an empty list or blanks", () => {
    expect([null, undefined, [], "", "   "].map(isAnswered)).toEqual([false, false, false, false, false]);
    expect([["k1"], "Paris", " x "].map(isAnswered)).toEqual([true, true, true]);
  });

  it("finds the questions still open, by place", () => {
    const a = attempt([question("a"), question("b"), question("c", "Short Answer")]);
    expect(unanswered(a, { a: ["k1"], b: null, c: "  " })).toEqual([1, 2]);
    expect(unanswered(a, { a: ["k1"], b: ["k2"], c: "Paris" })).toEqual([]);
  });

  it("starts from the answers an attempt already holds", () => {
    const a = attempt([question("a", "Single Choice", { response: ["k2"] }), question("b")]);
    expect(answersOf(a)).toEqual({ a: ["k2"], b: null });
  });
});

describe("choosing", () => {
  it("replaces the answer to a single-choice or true/false question", () => {
    const q = question("a");
    expect(choose(q, ["k1"], "k3")).toEqual(["k3"]);
    expect(choose(question("b", "True/False"), undefined, "k2")).toEqual(["k2"]);
  });

  it("adds and takes out options of a multiple-choice question, in the order shown", () => {
    const q = question("a", "Multiple Choice");
    expect(choose(q, null, "k3")).toEqual(["k3"]);
    expect(choose(q, ["k3"], "k1")).toEqual(["k1", "k3"]);
    expect(choose(q, ["k1", "k3"], "k1")).toEqual(["k3"]);
    expect(choose(q, ["k3"], "k3")).toBeNull();
    expect(chosen(["k1"], "k1") && !chosen(["k1"], "k2") && !chosen(null, "k1")).toBe(true);
  });
});

describe("the clock", () => {
  it("counts down from what the server said, on whichever clock saw more time pass", () => {
    const at = { mono: 1_000, wall: 5_000_000 };
    expect(remaining(600, at, at)).toBe(600);
    expect(remaining(600, at, { mono: 31_000, wall: 5_030_000 })).toBe(570);
    // The device slept for ten minutes: the steady clock stood still, the date moved on.
    expect(remaining(900, at, { mono: 2_000, wall: 5_000_000 + 600_000 })).toBe(300);
    // The date was set back: the steady clock still counts.
    expect(remaining(600, at, { mono: 61_000, wall: 4_000_000 })).toBe(540);
    expect(remaining(10, at, { mono: 61_000, wall: 5_060_000 })).toBe(0);
    expect(remaining(null, at, { mono: 1e9, wall: 1e12 })).toBeNull();
  });

  it("goes amber with a quarter of the time left (at most five minutes) and red near the end", () => {
    const total = 1800; // 30 minutes
    expect(timerTone(1700, total)).toBe("calm");
    expect(timerTone(300, total)).toBe("warn");
    expect(timerTone(61, total)).toBe("warn");
    expect(timerTone(60, total)).toBe("urgent");
    expect(timerTone(0, total)).toBe("urgent");
    // A three-minute quiz: amber at 45 s, red at 18 s.
    expect(timerTone(100, 180)).toBe("calm");
    expect(timerTone(45, 180)).toBe("warn");
    expect(timerTone(18, 180)).toBe("urgent");
    // Never red for less than ten seconds' warning.
    expect(timerTone(10, 60)).toBe("urgent");
    expect(timerTone(15, 60)).toBe("warn");
    expect(timerTone(20, 60)).toBe("calm");
  });

  it("names the milestone just passed, once", () => {
    expect(milestone(301, 299, 1800)).toBe(300);
    expect(milestone(299, 250, 1800)).toBeNull();
    expect(milestone(61, 59, 1800)).toBe(60);
    expect(milestone(11, 9, 1800)).toBe(10);
    // No five-minute warning for a quiz that is five minutes long.
    expect(milestone(301, 299, 300)).toBeNull();
    expect(milestoneText(300)).toBe("5 minutes left");
    expect(milestoneText(60)).toBe("1 minute left");
    expect(milestoneText(10)).toBe("10 seconds left");
  });
});

describe("the result", () => {
  it("is passed, failed or out of time", () => {
    expect(verdict({ passed: true, status: "Submitted" })).toBe("passed");
    expect(verdict({ passed: false, status: "Submitted" })).toBe("failed");
    expect(verdict({ passed: false, status: "Timed Out" })).toBe("timed-out");
  });

  it("counts what was right and knows whether the answers were shown", () => {
    const done = attempt([question("a", "Single Choice", { is_correct: true }), question("b"), question("c", "Short Answer", { is_correct: true, correct_answer: ["Paris"] })]);
    expect(rightCount(done)).toBe(2);
    expect(revealed(done)).toBe(true);
    expect(revealed(attempt([question("a")]))).toBe(false);
  });

  it("shows the text of what was chosen", () => {
    expect(responseText(question("a", "Multiple Choice", { response: ["k1", "k3"] }))).toEqual(["One", "Three"]);
    expect(responseText(question("a", "Short Answer", { response: "Paris" }))).toEqual(["Paris"]);
    expect(responseText(question("a"))).toEqual([]);
  });
});

describe("before the quiz", () => {
  type State = Pick<QuizState, "max_attempts" | "attempts_used" | "attempts_left" | "in_progress">;
  const state = (over: Partial<State> = {}): State => ({ max_attempts: 3, attempts_used: 1, attempts_left: 2, in_progress: null, ...over });

  it("lets a learner begin while attempts remain, or go on with one under way", () => {
    expect(canStart(state())).toBe(true);
    expect(canStart(state({ attempts_used: 3, attempts_left: 0 }))).toBe(false);
    expect(canStart(state({ attempts_used: 3, attempts_left: 0, in_progress: { attempt: "a", deadline: null } }))).toBe(true);
    expect(canStart(state({ max_attempts: 0, attempts_used: 9, attempts_left: null }))).toBe(true);
  });

  it("says how many attempts are left", () => {
    expect(attemptsText(state())).toBe("2 of 3 attempts left");
    expect(attemptsText(state({ max_attempts: 1, attempts_used: 0, attempts_left: 1 }))).toBe("1 of 1 attempt left");
    expect(attemptsText(state({ attempts_used: 3, attempts_left: 0 }))).toBe("All 3 attempts used");
    expect(attemptsText(state({ max_attempts: 0, attempts_used: 0, attempts_left: null }))).toBe("No limit on attempts");
    expect(attemptsText(state({ max_attempts: 0, attempts_used: 2, attempts_left: null }))).toBe("2 so far · no limit");
  });
});
