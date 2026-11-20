// Small rules of the lesson page and the addresses of Learn's screens, pure so they are tested.

import type { LessonBody, LessonType } from "./types";

/** The address a video is framed from. The server only stores https links to hosts it knows, and
 *  this is the second lock: anything else is not framed. */
export function embedSrc(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === "https:" && !u.username && !u.password ? u.toString() : null;
  } catch {
    return null;
  }
}

/** An address on another site (a document that is a link, a video's page). */
export const isExternal = (url: string) => /^https?:\/\//i.test(url);

/** What a lesson page offers besides its text. */
export function hasMedia(lesson: Pick<LessonBody, "lesson_type" | "video_embed_url" | "file_url">): boolean {
  return (lesson.lesson_type === "Video" && !!lesson.video_embed_url) || (lesson.lesson_type === "Document" && !!lesson.file_url);
}

/** Whether the learner finishes this kind of lesson by saying so. */
export const finishedByLearner = (type: LessonType) => type === "Article" || type === "Video" || type === "Document";

// ───────────────────────────── addresses ─────────────────────────────

const seg = encodeURIComponent;

export const paths = {
  home: "/",
  catalog: "/catalog",
  course: (slug: string) => `/course/${seg(slug)}`,
  grades: (slug: string) => `/course/${seg(slug)}/grades`,
  discussion: (slug: string) => `/course/${seg(slug)}/discussion`,
  thread: (slug: string, thread: string) => `/course/${seg(slug)}/discussion/${seg(thread)}`,
  /** The player; without a lesson it opens where the learner left off. */
  learn: (slug: string, lesson?: string) => (lesson ? `/learn/${seg(slug)}/${seg(lesson)}` : `/learn/${seg(slug)}`),
  teaching: (course?: string) => (course ? `/teaching/${seg(course)}` : "/teaching"),
  build: (course: string, lesson?: string) => `/build/${seg(course)}${lesson ? `?lesson=${seg(lesson)}` : ""}`,
  /** A report of the records frame, with its filters and CSV download. */
  report: (name: string) => `/r/${seg(name)}`,
  record: (entity: string, id: string) => `/e/${seg(entity)}/${seg(id)}`,
};

/** The same pages for people who are not signed in (the platform serves them below /lms/c/). */
export const publicPaths = {
  catalog: (workspace: string) => `/c/${seg(workspace)}`,
  course: (workspace: string, slug: string) => `/c/${seg(workspace)}/${seg(slug)}`,
  lesson: (workspace: string, slug: string, lesson: string) => `/c/${seg(workspace)}/${seg(slug)}/lessons/${seg(lesson)}`,
  certificate: (workspace: string, code: string) => `/c/${seg(workspace)}/certificate/${seg(code)}`,
};

// ───────────────────────────── watching an uploaded video ─────────────────────────────

/** Seconds of play to add after a `timeupdate`: the step since the last one while the video plays and it is a step
 *  forward of at most two seconds. A seek, a step back and a pause add nothing. */
export function counted(last: number, now: number, playing: boolean): number {
  const step = now - last;
  return playing && step > 0 && step <= 2 ? step : 0;
}

/** The whole seconds to report (base plus what played), or null when nothing grew since the last report. */
export function toReport(base: number, played: number, sent: number): number | null {
  const whole = Math.floor(base + played);
  return whole > sent ? whole : null;
}

/** Whether the learner finishes this lesson by saying so: not when watching completes it (the server refuses it) or its tool grades. */
export function byHandOf(view: { lti?: { sends_grades: boolean } | null; video?: { watch_percent: number | null } | null }): boolean | undefined {
  if (view.lti) return !view.lti.sends_grades;
  if (view.video?.watch_percent) return false;
  return undefined;
}
