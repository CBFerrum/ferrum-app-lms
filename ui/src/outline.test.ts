import { describe, expect, it } from "vitest";
import { around, chapterCounts, findLesson, markDone, nextAction, nextUp, percent, percentLabel, placed, progressText, remainingRequired, resume, selfPaced, totals, whereNext } from "./outline";
import type { OutlineChapter, OutlineLesson } from "./types";

const lesson = (id: string, over: Partial<OutlineLesson> = {}): OutlineLesson => ({ id, title: `Lesson ${id}`, lesson_type: "Article", duration_minutes: 10, is_preview: false, required: true, ...over });

const course = (): OutlineChapter[] => [
  { id: "c1", title: "Basics", lessons: [lesson("a", { completed: true }), lesson("b", { completed: true }), lesson("c", { lesson_type: "Quiz" })] },
  { id: "c2", title: "More", lessons: [lesson("d", { required: false }), lesson("e", { duration_minutes: 25 })] },
  { id: "c3", title: "Empty", lessons: [] },
];

describe("a course's outline", () => {
  it("lists lessons in the order a learner meets them", () => {
    expect(placed(course()).map((p) => [p.lesson.id, p.chapter.id, p.index])).toEqual([["a", "c1", 0], ["b", "c1", 1], ["c", "c1", 2], ["d", "c2", 3], ["e", "c2", 4]]);
    expect(findLesson(course(), "d")?.chapter.title).toBe("More");
    expect(findLesson(course(), "nope")).toBeUndefined();
    expect(findLesson(course(), undefined)).toBeUndefined();
  });

  it("knows the lessons either side of one, across chapters", () => {
    expect(around(course(), "c")).toMatchObject({ prev: { lesson: { id: "b" } }, next: { lesson: { id: "d" } }, index: 2, count: 5 });
    expect(around(course(), "a").prev).toBeNull();
    expect(around(course(), "e").next).toBeNull();
    expect(around(course(), "zzz")).toMatchObject({ prev: null, next: null, index: -1 });
  });

  it("takes the first unfinished required lesson next, else the first unfinished", () => {
    expect(nextUp(course())?.id).toBe("c");
    const optionalFirst = course();
    optionalFirst[0].lessons[2].completed = true;
    // d is unfinished but optional: the required e comes first.
    expect(nextUp(optionalFirst)?.id).toBe("e");
    const onlyOptional = markDone(optionalFirst, "e");
    expect(nextUp(onlyOptional)?.id).toBe("d");
    expect(nextUp(markDone(onlyOptional, "d"))).toBeNull();
  });

  it("continues at what is next, or reads a finished course again from the start", () => {
    expect(resume(course())?.id).toBe("c");
    const done = markDone(markDone(markDone(markDone(markDone(course(), "a"), "b"), "c"), "d"), "e");
    expect(resume(done)?.id).toBe("a");
    expect(resume([])).toBeNull();
    expect(resume([{ id: "x", title: "Empty", lessons: [] }])).toBeNull();
  });

  it("moves on to the lesson after the one just finished, and at the end to what is left", () => {
    expect(whereNext(course(), "a")?.id).toBe("b");
    expect(whereNext(course(), "c")?.id).toBe("d");
    // At the last lesson with c still unfinished, Next leads there.
    expect(whereNext(course(), "e")?.id).toBe("c");
    const all = markDone(markDone(markDone(course(), "c"), "d"), "e");
    expect(whereNext(all, "e")).toBeNull();
  });

  it("marks a lesson done without touching the original", () => {
    const before = course();
    const after = markDone(before, "c", "2026-11-01T10:00:00.000Z");
    expect(after[0].lessons[2]).toMatchObject({ completed: true, completed_at: "2026-11-01T10:00:00.000Z" });
    expect(before[0].lessons[2].completed).toBeUndefined();
    expect(after[1]).toBe(before[1]);
    // Doing it twice keeps the first time.
    expect(markDone(after, "c", "2026-12-01T00:00:00.000Z")[0].lessons[2].completed_at).toBe("2026-11-01T10:00:00.000Z");
  });

  it("counts what is done and how long it takes", () => {
    expect(chapterCounts(course()[0])).toEqual({ done: 2, total: 3, minutes: 30 });
    expect(totals(course())).toEqual({ lessons: 5, done: 2, minutes: 65 });
    expect(totals([])).toEqual({ lessons: 0, done: 0, minutes: 0 });
  });

  it("leaves quizzes and assignments to the server to finish", () => {
    expect(["Article", "Video", "Document"].every((t) => selfPaced(t as never))).toBe(true);
    expect(selfPaced("Quiz")).toBe(false);
    expect(selfPaced("Assignment")).toBe(false);
  });

  it("reads the server's percentages", () => {
    expect(percent("33.33")).toBe(33.33);
    expect(percent("100")).toBe(100);
    expect(percent("140")).toBe(100);
    expect(percent(null)).toBe(0);
    expect(percent("nonsense")).toBe(0);
    expect(percentLabel("33.00")).toBe("33%");
    expect(percentLabel("66.67")).toBe("66.7%");
    expect(percentLabel(undefined)).toBe("0%");
  });

  it("counts what is left that the course asks for", () => {
    expect(remainingRequired(course())).toBe(2); // c and e; d is optional
    const noneRequired: OutlineChapter[] = [{ id: "x", title: "X", lessons: [lesson("p", { required: false }), lesson("q", { required: false, completed: true })] }];
    expect(remainingRequired(noneRequired)).toBe(1);
    expect(progressText(3, 12, 12)).toBe("3 of 12 lessons");
    expect(progressText(3, 10, 12)).toBe("3 of 10 required lessons");
    expect(progressText(null, null, 0)).toBe("0 of 0 lessons");
  });

  describe("the foot of a lesson", () => {
    it("finishes a reading and moves on in one step", () => {
      expect(nextAction(course(), { id: "d", lesson_type: "Article" }, false, true)).toMatchObject({ label: "Complete and continue", completes: true, primary: true, target: { id: "e" } });
    });

    it("finishes the course with the last lesson that stood in the way", () => {
      const almost = markDone(course(), "c");
      expect(nextAction(almost, { id: "e", lesson_type: "Article" }, false, true)).toMatchObject({ label: "Finish course", completes: true, target: null });
    });

    it("sends a learner at the end of the course back to what they skipped", () => {
      // c (a quiz) is not done: finishing e leads there rather than ending.
      expect(nextAction(course(), { id: "e", lesson_type: "Article" }, false, true)).toMatchObject({ label: "Complete and continue", target: { id: "c" } });
    });

    it("leaves quizzes to be passed, and offers a plain Next", () => {
      const a = nextAction(course(), { id: "c", lesson_type: "Quiz" }, false, true);
      expect(a).toMatchObject({ label: "Next lesson", completes: false, primary: false, target: { id: "d" } });
    });

    it("makes Next the main button once a lesson is done", () => {
      expect(nextAction(course(), { id: "a", lesson_type: "Article" }, true, true)).toMatchObject({ label: "Next lesson", completes: false, primary: true, target: { id: "b" } });
    });

    it("records nothing for someone only looking", () => {
      expect(nextAction(course(), { id: "d", lesson_type: "Article" }, false, false)).toMatchObject({ label: "Next lesson", completes: false, primary: false, target: { id: "e" } });
    });

    it("goes back to the course from the end of a finished one", () => {
      const all = markDone(markDone(markDone(markDone(markDone(course(), "a"), "b"), "c"), "d"), "e");
      expect(nextAction(all, { id: "e", lesson_type: "Article" }, true, true)).toMatchObject({ label: "Back to the course", completes: false, target: null });
    });
  });
});
