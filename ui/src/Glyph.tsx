// The few icons Learn needs that the kit does not draw, in the kit's own stroke.

import { Icon } from "@ferrum/kit";
import type { LessonType } from "./types";

const PATHS: Record<string, string> = {
  clock: "M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z",
  lock: "M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3",
  grip: "M9 6h.01M9 12h.01M9 18h.01M15 6h.01M15 12h.01M15 18h.01",
  award: "M12 15a6 6 0 1 0 0-12 6 6 0 0 0 0 12ZM8.5 14 7 21l5-3 5 3-1.5-7",
  quiz: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.4-1 .9-1 1.7M12 17h.01",
  assignment: "M9 4h6v3H9zM7 5.5H5.5V21h13V5.5H17M8.5 12h7M8.5 16h5",
  article: "M5 4h14v16H5zM8.5 9h7M8.5 13h7M8.5 17h4",
  play: "M7 4.5v15l12-7.5z",
  arrowLeft: "M19 12H5m6-6-6 6 6 6",
  arrowRight: "M5 12h14m-6-6 6 6-6 6",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  users: "M16 20v-1.5a4 4 0 0 0-4-4H7a4 4 0 0 0-4 4V20M9.5 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7ZM21 20v-1.5a4 4 0 0 0-3-3.9M16 4.2a3.5 3.5 0 0 1 0 6.6",
  eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12ZM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z",
  more: "M12 5h.01M12 12h.01M12 19h.01",
  cap: "M2 9l10-5 10 5-10 5zM6 11v5c0 1.5 3 3 6 3s6-1.5 6-3v-5",
  circle: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z",
  circleCheck: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM8 12.5l3 3 5-6",
};

/** An icon of Learn's, or the kit's of that name. */
export function Glyph({ name, size = 16 }: { name: string; size?: number }) {
  const d = PATHS[name];
  if (!d) return <Icon name={name} size={size} />;
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d={d} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

const BY_TYPE: Record<LessonType, string> = { Article: "article", Video: "play", Document: "doc", Quiz: "quiz", Assignment: "assignment" };

export const lessonGlyph = (type: LessonType) => BY_TYPE[type] ?? "article";

/** What to call a kind of lesson. */
export const LESSON_LABEL: Record<LessonType, string> = { Article: "Reading", Video: "Video", Document: "Document", Quiz: "Quiz", Assignment: "Assignment" };
