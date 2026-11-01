// What stands between a course and its learners, asked before trying to publish. The server's own
// rules (Learn's course hook) are the last word: a course needs a lesson, and every quiz lesson a
// quiz that has questions; the same for assignments, which the server leaves to learners to find
// out. This mirrors them so the builder can say what to fix in advance.

import type { LessonType } from "./types";

export interface Publishable {
  id: string;
  title: string;
  lesson_type: LessonType;
  quiz: string | null;
  assignment: string | null;
}

export interface Issue {
  /** The lesson to open to fix it. */
  lesson?: string;
  text: string;
}

export function publishIssues(lessons: Publishable[], quizzes: Map<string, number>): Issue[] {
  if (!lessons.length) return [{ text: "Add at least one lesson." }];
  const out: Issue[] = [];
  for (const l of lessons) {
    if (l.lesson_type === "Quiz") {
      if (!l.quiz) out.push({ lesson: l.id, text: `"${l.title}" needs a quiz.` });
      else if (!(quizzes.get(l.quiz) ?? 0)) out.push({ lesson: l.id, text: `The quiz of "${l.title}" has no questions yet.` });
    } else if (l.lesson_type === "Assignment" && !l.assignment) out.push({ lesson: l.id, text: `"${l.title}" needs an assignment.` });
  }
  return out;
}
