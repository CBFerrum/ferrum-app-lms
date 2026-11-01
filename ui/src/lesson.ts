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
  /** The player; without a lesson it opens where the learner left off. */
  learn: (slug: string, lesson?: string) => (lesson ? `/learn/${seg(slug)}/${seg(lesson)}` : `/learn/${seg(slug)}`),
  teaching: (course?: string) => (course ? `/teaching/${seg(course)}` : "/teaching"),
  build: (course: string, lesson?: string) => `/build/${seg(course)}${lesson ? `?lesson=${seg(lesson)}` : ""}`,
  record: (entity: string, id: string) => `/e/${seg(entity)}/${seg(id)}`,
};

/** The same pages for people who are not signed in (the platform serves them below /lms/c/). */
export const publicPaths = {
  catalog: (workspace: string) => `/c/${seg(workspace)}`,
  course: (workspace: string, slug: string) => `/c/${seg(workspace)}/${seg(slug)}`,
  lesson: (workspace: string, slug: string, lesson: string) => `/c/${seg(workspace)}/${seg(slug)}/lessons/${seg(lesson)}`,
  certificate: (workspace: string, code: string) => `/c/${seg(workspace)}/certificate/${seg(code)}`,
};
