import { describe, expect, it } from "vitest";
import { publishIssues, type Publishable } from "./publish";

const lesson = (id: string, over: Partial<Publishable> = {}): Publishable => ({ id, title: `Lesson ${id}`, lesson_type: "Article", quiz: null, assignment: null, ...over });

describe("what stands between a course and its learners", () => {
  it("asks for a lesson", () => {
    expect(publishIssues([], new Map())).toEqual([{ text: "Add at least one lesson." }]);
  });

  it("is happy with readings, videos and documents", () => {
    expect(publishIssues([lesson("a"), lesson("b", { lesson_type: "Video" }), lesson("c", { lesson_type: "Document" })], new Map())).toEqual([]);
  });

  it("wants every quiz lesson to have a quiz with questions", () => {
    const quizzes = new Map([["q1", 3], ["q2", 0]]);
    const issues = publishIssues(
      [lesson("a", { lesson_type: "Quiz", quiz: "q1" }), lesson("b", { lesson_type: "Quiz", quiz: "q2" }), lesson("c", { lesson_type: "Quiz" }), lesson("d", { lesson_type: "Quiz", quiz: "gone" })],
      quizzes,
    );
    expect(issues.map((i) => [i.lesson, i.text])).toEqual([
      ["b", 'The quiz of "Lesson b" has no questions yet.'],
      ["c", '"Lesson c" needs a quiz.'],
      ["d", 'The quiz of "Lesson d" has no questions yet.'],
    ]);
  });

  it("wants every assignment lesson to have an assignment", () => {
    expect(publishIssues([lesson("a", { lesson_type: "Assignment" }), lesson("b", { lesson_type: "Assignment", assignment: "x" })], new Map()).map((i) => i.lesson)).toEqual(["a"]);
  });
});
