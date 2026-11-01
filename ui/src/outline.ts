// A course's outline as the player uses it: lessons in the order a learner meets them, the ones
// either side of the one open, and what comes next. Pure, so the player's moves are tested without
// a browser. The server decides what is done and what is required; this only follows its rules.

import type { LessonType, OutlineChapter, OutlineLesson } from "./types";

export interface Placed {
  lesson: OutlineLesson;
  chapter: OutlineChapter;
  /** Zero-based place among all the course's lessons. */
  index: number;
}

/** Every lesson, chapter by chapter. */
export function placed(chapters: OutlineChapter[]): Placed[] {
  const out: Placed[] = [];
  for (const chapter of chapters) for (const lesson of chapter.lessons) out.push({ lesson, chapter, index: out.length });
  return out;
}

export function findLesson(chapters: OutlineChapter[], id: string | undefined): Placed | undefined {
  return id ? placed(chapters).find((p) => p.lesson.id === id) : undefined;
}

/** The lessons either side of one, and where it stands. */
export function around(chapters: OutlineChapter[], id: string | undefined): { prev: Placed | null; next: Placed | null; index: number; count: number } {
  const all = placed(chapters);
  const at = all.findIndex((p) => p.lesson.id === id);
  return { prev: at > 0 ? all[at - 1] : null, next: at >= 0 && at < all.length - 1 ? all[at + 1] : null, index: at, count: all.length };
}

/** What to take next, by the server's rule: the first unfinished required lesson, else the first
 *  unfinished one. Null when everything is done. */
export function nextUp(chapters: OutlineChapter[]): OutlineLesson | null {
  const open = placed(chapters).filter((p) => !p.lesson.completed);
  return (open.find((p) => p.lesson.required) ?? open[0])?.lesson ?? null;
}

/** Where a button that continues the course goes: what is next, or the first lesson of a course
 *  that is done (to read it again). */
export function resume(chapters: OutlineChapter[]): OutlineLesson | null {
  return nextUp(chapters) ?? chapters.find((c) => c.lessons.length)?.lessons[0] ?? null;
}

/** Lessons the course still asks for (every lesson counts when none is marked required, as the
 *  server counts them), in order. */
export function pendingRequired(chapters: OutlineChapter[]): OutlineLesson[] {
  const all = placed(chapters);
  const required = all.filter((p) => p.lesson.required);
  return (required.length ? required : all).filter((p) => !p.lesson.completed).map((p) => p.lesson);
}

export const remainingRequired = (chapters: OutlineChapter[]): number => pendingRequired(chapters).length;

/** Where "Next" leads from a lesson: the one after it; after the last, back to a required lesson
 *  skipped on the way; null when the course is done (optional lessons left over do not count). */
export function whereNext(chapters: OutlineChapter[], id: string): OutlineLesson | null {
  return around(chapters, id).next?.lesson ?? pendingRequired(chapters)[0] ?? null;
}

/** The outline with a lesson marked done. */
export function markDone(chapters: OutlineChapter[], id: string, at = new Date().toISOString()): OutlineChapter[] {
  return chapters.map((c) => (c.lessons.some((l) => l.id === id) ? { ...c, lessons: c.lessons.map((l) => (l.id === id ? { ...l, completed: true, completed_at: l.completed_at ?? at } : l)) } : c));
}

export function chapterCounts(chapter: OutlineChapter): { done: number; total: number; minutes: number } {
  return { done: chapter.lessons.filter((l) => l.completed).length, total: chapter.lessons.length, minutes: chapter.lessons.reduce((m, l) => m + (l.duration_minutes || 0), 0) };
}

export function totals(chapters: OutlineChapter[]): { lessons: number; done: number; minutes: number } {
  return chapters.reduce(
    (t, c) => {
      const n = chapterCounts(c);
      return { lessons: t.lessons + n.total, done: t.done + n.done, minutes: t.minutes + n.minutes };
    },
    { lessons: 0, done: 0, minutes: 0 },
  );
}

/** The lessons the learner finishes by marking them done; quizzes and assignments finish when
 *  passed, which the server sees to. */
export function selfPaced(type: LessonType): boolean {
  return type === "Article" || type === "Video" || type === "Document";
}

/** 0 to 100 from the server's decimal ("33.33"). */
export function percent(value: string | number | null | undefined): number {
  const n = typeof value === "number" ? value : Number(value ?? 0);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : 0;
}

/** "33%" or "33.3%": whole numbers when they are, else one place. */
export function percentLabel(value: string | number | null | undefined): string {
  const n = percent(value);
  return `${Number.isInteger(n) ? n : Math.round(n * 10) / 10}%`;
}

/** The lesson ids of a course in order, for a link to the lesson after one without the outline. */
export const ids = (chapters: OutlineChapter[]): string[] => placed(chapters).map((p) => p.lesson.id);

/** "3 of 12 lessons", or "3 of 10 required lessons" when some lessons are optional. */
export function progressText(done: number | null | undefined, total: number | null | undefined, lessonsInCourse: number): string {
  const [d, t] = [done ?? 0, total ?? 0];
  return `${d} of ${t} ${t === lessonsInCourse ? "lessons" : "required lessons"}`;
}

/** What the button at the foot of a lesson does. */
export interface NextAction {
  label: string;
  /** Saying the lesson is done comes first (articles, videos and documents). */
  completes: boolean;
  primary: boolean;
  /** Where it leads afterwards; null: back to the course. */
  target: OutlineLesson | null;
}

/** The foot of a lesson for a learner: finishing and moving on in one step where the lesson is
 *  finished by saying so, a plain "Next" where it is not (quizzes and assignments finish when
 *  passed) or already done. `learning`: the viewer is enrolled and may record progress. */
export function nextAction(chapters: OutlineChapter[], lesson: Pick<OutlineLesson, "id" | "lesson_type">, completed: boolean, learning: boolean): NextAction {
  const completes = learning && !completed && selfPaced(lesson.lesson_type);
  const after = completes ? markDone(chapters, lesson.id) : chapters;
  const found = whereNext(after, lesson.id);
  const target = found && found.id !== lesson.id ? found : null;
  if (completes) {
    const last = remainingRequired(after) === 0 && !around(after, lesson.id).next;
    return { label: target ? "Complete and continue" : last ? "Finish course" : "Mark as complete", completes, primary: true, target };
  }
  return { label: target ? "Next lesson" : "Back to the course", completes: false, primary: completed && !!target, target };
}
