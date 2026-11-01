// Reordering a course: chapters and the lessons in them. The outline is a list of lists; a move
// makes a new one, and the edits it takes to save it are only the records whose place changed.
// The server keeps no "reorder" of its own, so a move is a few edits of `position` (and `chapter`),
// each against the version the builder holds. Pure, so every move is tested without a browser.

export interface ChapterRow {
  id: string;
  title: string;
  position: number | null;
  version: number;
}

export interface LessonRow {
  id: string;
  title: string;
  chapter: string;
  position: number | null;
  version: number;
}

export interface Layout {
  chapters: { id: string; lessons: string[] }[];
}

const place = (p: number | null) => p ?? Number.MAX_SAFE_INTEGER;

/** By position (a record without one last), the order received breaking ties. */
function order<T extends { position: number | null }>(rows: T[]): T[] {
  return rows
    .map((r, i) => [r, i] as const)
    .sort((a, b) => place(a[0].position) - place(b[0].position) || a[1] - b[1])
    .map(([r]) => r);
}

/** The outline in the order a learner meets it. */
export function layoutOf(chapters: ChapterRow[], lessons: LessonRow[]): Layout {
  return { chapters: order(chapters).map((c) => ({ id: c.id, lessons: order(lessons.filter((l) => l.chapter === c.id)).map((l) => l.id) })) };
}

const copy = (l: Layout): Layout => ({ chapters: l.chapters.map((c) => ({ id: c.id, lessons: [...c.lessons] })) });

export function find(layout: Layout, lesson: string): { chapter: number; index: number } | null {
  for (let c = 0; c < layout.chapters.length; c++) {
    const index = layout.chapters[c].lessons.indexOf(lesson);
    if (index >= 0) return { chapter: c, index };
  }
  return null;
}

/** A lesson to another place: `index` counts in the destination without the lesson, so 0 is first
 *  and the destination's length is last. */
export function moveLesson(layout: Layout, lesson: string, toChapter: string, index: number): Layout {
  const from = find(layout, lesson);
  const dest = layout.chapters.findIndex((c) => c.id === toChapter);
  if (!from || dest < 0) return layout;
  const next = copy(layout);
  next.chapters[from.chapter].lessons.splice(from.index, 1);
  const list = next.chapters[dest].lessons;
  list.splice(Math.min(Math.max(0, index), list.length), 0, lesson);
  return next;
}

/** A chapter to another place among the chapters (`index` as for lessons). */
export function moveChapter(layout: Layout, chapter: string, index: number): Layout {
  const from = layout.chapters.findIndex((c) => c.id === chapter);
  if (from < 0) return layout;
  const next = copy(layout);
  const [moved] = next.chapters.splice(from, 1);
  next.chapters.splice(Math.min(Math.max(0, index), next.chapters.length), 0, moved);
  return next;
}

/** One place up or down for a lesson, by keyboard or button: past the end of its chapter it goes
 *  into the next chapter. Nothing changes at the very start or end of the course. */
export function shiftLesson(layout: Layout, lesson: string, by: -1 | 1): Layout {
  const at = find(layout, lesson);
  if (!at) return layout;
  const list = layout.chapters[at.chapter].lessons;
  if (by < 0) {
    if (at.index > 0) return moveLesson(layout, lesson, layout.chapters[at.chapter].id, at.index - 1);
    const prev = layout.chapters[at.chapter - 1];
    return prev ? moveLesson(layout, lesson, prev.id, prev.lessons.length) : layout;
  }
  if (at.index < list.length - 1) return moveLesson(layout, lesson, layout.chapters[at.chapter].id, at.index + 1);
  const next = layout.chapters[at.chapter + 1];
  return next ? moveLesson(layout, lesson, next.id, 0) : layout;
}

export function shiftChapter(layout: Layout, chapter: string, by: -1 | 1): Layout {
  const at = layout.chapters.findIndex((c) => c.id === chapter);
  const to = at + by;
  return at < 0 || to < 0 || to >= layout.chapters.length ? layout : moveChapter(layout, chapter, to);
}

export interface Patch {
  entity: "LmsChapter" | "LmsLesson";
  id: string;
  version: number;
  body: Record<string, unknown>;
}

/** The edits that turn what is stored into `layout`: positions count from 1, and a lesson in
 *  another chapter says so. Only what differs. */
export function patchesFor(layout: Layout, chapters: ChapterRow[], lessons: LessonRow[]): Patch[] {
  const byChapter = new Map(chapters.map((c) => [c.id, c]));
  const byLesson = new Map(lessons.map((l) => [l.id, l]));
  const out: Patch[] = [];
  layout.chapters.forEach((c, i) => {
    const row = byChapter.get(c.id);
    if (row && row.position !== i + 1) out.push({ entity: "LmsChapter", id: c.id, version: row.version, body: { position: i + 1 } });
    c.lessons.forEach((id, j) => {
      const l = byLesson.get(id);
      if (!l) return;
      const body: Record<string, unknown> = {};
      if (l.chapter !== c.id) body.chapter = c.id;
      if (l.position !== j + 1) body.position = j + 1;
      if (Object.keys(body).length) out.push({ entity: "LmsLesson", id, version: l.version, body });
    });
  });
  return out;
}

/** Rows as they are once the edits are saved (each edit's answer brings the new version). */
export function applyPatches<T extends { id: string; version: number }>(rows: T[], done: { id: string; version: number; body: Record<string, unknown> }[]): T[] {
  const by = new Map(done.map((d) => [d.id, d]));
  return rows.map((r) => {
    const d = by.get(r.id);
    return d ? { ...r, ...d.body, version: d.version } : r;
  });
}

export type Edge = "before" | "after";

/** A lesson dropped on the upper or lower half of another lesson. */
export function dropLesson(layout: Layout, lesson: string, target: string, edge: Edge): Layout {
  const to = find(layout, target);
  const from = find(layout, lesson);
  if (!to || !from || lesson === target) return layout;
  let index = edge === "before" ? to.index : to.index + 1;
  // Taking the lesson out of the same chapter first shifts the places after it up by one.
  if (from.chapter === to.chapter && from.index < index) index--;
  return moveLesson(layout, lesson, layout.chapters[to.chapter].id, index);
}

/** A lesson dropped on a chapter itself (its heading, or its empty list): it goes last. */
export function dropLessonOnChapter(layout: Layout, lesson: string, chapter: string): Layout {
  const to = layout.chapters.find((c) => c.id === chapter);
  const from = find(layout, lesson);
  if (!to || !from) return layout;
  return moveLesson(layout, lesson, chapter, to.lessons.length);
}

/** A chapter dropped on the upper or lower half of another. */
export function dropChapter(layout: Layout, chapter: string, target: string, edge: Edge): Layout {
  const from = layout.chapters.findIndex((c) => c.id === chapter);
  const to = layout.chapters.findIndex((c) => c.id === target);
  if (from < 0 || to < 0 || from === to) return layout;
  let index = edge === "before" ? to : to + 1;
  if (from < index) index--;
  return moveChapter(layout, chapter, index);
}
