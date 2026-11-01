import { describe, expect, it } from "vitest";
import { applyPatches, type ChapterRow, dropChapter, dropLesson, dropLessonOnChapter, find, type Layout, layoutOf, type LessonRow, moveChapter, moveLesson, patchesFor, shiftChapter, shiftLesson } from "./reorder";

const ch = (id: string, position: number | null, version = 1): ChapterRow => ({ id, title: id, position, version });
const le = (id: string, chapter: string, position: number | null, version = 1): LessonRow => ({ id, title: id, chapter, position, version });

const chapters = [ch("c2", 2), ch("c1", 1), ch("c3", 3)];
const lessons = [le("c1a", "c1", 1), le("c1b", "c1", 2), le("c2a", "c2", 1), le("c3a", "c3", null), le("c3b", "c3", null), le("c1c", "c1", 3)];

const ids = (l: Layout) => l.chapters.map((c) => `${c.id}:${c.lessons.join(",")}`);

describe("an outline's order", () => {
  it("follows positions, with records that have none last in the order received", () => {
    expect(ids(layoutOf(chapters, lessons))).toEqual(["c1:c1a,c1b,c1c", "c2:c2a", "c3:c3a,c3b"]);
    expect(ids(layoutOf([], []))).toEqual([]);
  });

  it("finds a lesson", () => {
    const l = layoutOf(chapters, lessons);
    expect(find(l, "c2a")).toEqual({ chapter: 1, index: 0 });
    expect(find(l, "nope")).toBeNull();
  });
});

describe("moving a lesson", () => {
  const base = layoutOf(chapters, lessons);

  it("within its chapter", () => {
    expect(ids(moveLesson(base, "c1c", "c1", 0))[0]).toBe("c1:c1c,c1a,c1b");
    expect(ids(moveLesson(base, "c1a", "c1", 2))[0]).toBe("c1:c1b,c1c,c1a");
  });

  it("into another chapter, at the start, in the middle or the end", () => {
    expect(ids(moveLesson(base, "c1b", "c3", 0))).toEqual(["c1:c1a,c1c", "c2:c2a", "c3:c1b,c3a,c3b"]);
    expect(ids(moveLesson(base, "c1b", "c3", 1))[2]).toBe("c3:c3a,c1b,c3b");
    expect(ids(moveLesson(base, "c1b", "c3", 99))[2]).toBe("c3:c3a,c3b,c1b");
  });

  it("into a chapter that has no lessons", () => {
    const empty = layoutOf([...chapters, ch("c4", 4)], lessons);
    expect(ids(moveLesson(empty, "c2a", "c4", 0)).slice(1)).toEqual(["c2:", "c3:c3a,c3b", "c4:c2a"]);
  });

  it("changes nothing for a lesson or chapter that is not there, and leaves the original alone", () => {
    expect(moveLesson(base, "nope", "c1", 0)).toBe(base);
    expect(moveLesson(base, "c1a", "nope", 0)).toBe(base);
    moveLesson(base, "c1a", "c2", 0);
    expect(ids(base)[0]).toBe("c1:c1a,c1b,c1c");
  });
});

describe("moving a chapter", () => {
  const base = layoutOf(chapters, lessons);
  it("keeps its lessons with it", () => {
    expect(ids(moveChapter(base, "c3", 0))).toEqual(["c3:c3a,c3b", "c1:c1a,c1b,c1c", "c2:c2a"]);
    expect(ids(moveChapter(base, "c1", 99)).map((s) => s.split(":")[0])).toEqual(["c2", "c3", "c1"]);
    expect(moveChapter(base, "nope", 0)).toBe(base);
  });
});

describe("one place at a time, for a keyboard", () => {
  const base = layoutOf(chapters, lessons);
  it("moves a lesson up and down within its chapter", () => {
    expect(ids(shiftLesson(base, "c1b", -1))[0]).toBe("c1:c1b,c1a,c1c");
    expect(ids(shiftLesson(base, "c1b", 1))[0]).toBe("c1:c1a,c1c,c1b");
  });

  it("carries it over the edge into the next or previous chapter", () => {
    expect(ids(shiftLesson(base, "c1c", 1)).slice(0, 2)).toEqual(["c1:c1a,c1b", "c2:c1c,c2a"]);
    expect(ids(shiftLesson(base, "c2a", -1)).slice(0, 2)).toEqual(["c1:c1a,c1b,c1c,c2a", "c2:"]);
  });

  it("stops at the very start and end of the course", () => {
    expect(shiftLesson(base, "c1a", -1)).toBe(base);
    expect(shiftLesson(base, "c3b", 1)).toBe(base);
  });

  it("moves chapters", () => {
    expect(ids(shiftChapter(base, "c2", -1)).map((s) => s.split(":")[0])).toEqual(["c2", "c1", "c3"]);
    expect(shiftChapter(base, "c1", -1)).toBe(base);
    expect(shiftChapter(base, "c3", 1)).toBe(base);
  });
});

describe("saving a move", () => {
  const base = layoutOf(chapters, lessons);
  const rows = (ver = 5) => [chapters.map((c) => ({ ...c, version: ver })), lessons.map((x) => ({ ...x, version: ver }))] as const;

  it("edits only what moved", () => {
    const [c, l] = rows();
    // The stored order is already 1, 2, 3 for chapter 1, so only the lessons without a place are numbered.
    expect(patchesFor(base, c, l)).toEqual([
      { entity: "LmsLesson", id: "c3a", version: 5, body: { position: 1 } },
      { entity: "LmsLesson", id: "c3b", version: 5, body: { position: 2 } },
    ]);
  });

  it("numbers a moved lesson in its new chapter and renumbers its old neighbours", () => {
    const [c, l] = rows();
    const patches = patchesFor(moveLesson(base, "c1a", "c2", 1), c, l);
    expect(patches.filter((p) => p.id.startsWith("c1") || p.id === "c2a")).toEqual([
      { entity: "LmsLesson", id: "c1b", version: 5, body: { position: 1 } },
      { entity: "LmsLesson", id: "c1c", version: 5, body: { position: 2 } },
      { entity: "LmsLesson", id: "c1a", version: 5, body: { chapter: "c2", position: 2 } },
    ]);
  });

  it("numbers chapters after a chapter moves", () => {
    const [c, l] = rows();
    const patches = patchesFor(moveChapter(base, "c3", 0), c, l).filter((p) => p.entity === "LmsChapter");
    expect(patches.map((p) => [p.id, p.body.position])).toEqual([["c3", 1], ["c1", 2], ["c2", 3]]);
  });

  it("is nothing for a layout that is already stored", () => {
    const stored = [ch("a", 1), ch("b", 2)];
    const items = [le("a1", "a", 1), le("b1", "b", 1)];
    expect(patchesFor(layoutOf(stored, items), stored, items)).toEqual([]);
  });

  it("brings the new versions into the rows", () => {
    const [, l] = rows();
    const after = applyPatches(l, [{ id: "c1a", version: 6, body: { chapter: "c2", position: 2 } }]);
    expect(after.find((x) => x.id === "c1a")).toMatchObject({ chapter: "c2", position: 2, version: 6 });
    expect(after.find((x) => x.id === "c1b")).toBe(l.find((x) => x.id === "c1b"));
  });
});

describe("dropping", () => {
  const base = layoutOf(chapters, lessons);

  it("puts a lesson before or after the one it lands on", () => {
    expect(ids(dropLesson(base, "c1c", "c1a", "before"))[0]).toBe("c1:c1c,c1a,c1b");
    expect(ids(dropLesson(base, "c1c", "c1a", "after"))[0]).toBe("c1:c1a,c1c,c1b");
    expect(ids(dropLesson(base, "c2a", "c1b", "after")).slice(0, 2)).toEqual(["c1:c1a,c1b,c2a,c1c", "c2:"]);
  });

  it("counts places the way they are after the lesson leaves its chapter", () => {
    // c1a dropped after c1b: still between c1b and c1c.
    expect(ids(dropLesson(base, "c1a", "c1b", "after"))[0]).toBe("c1:c1b,c1a,c1c");
    expect(ids(dropLesson(base, "c1a", "c1c", "before"))[0]).toBe("c1:c1b,c1a,c1c");
    expect(ids(dropLesson(base, "c1a", "c1c", "after"))[0]).toBe("c1:c1b,c1c,c1a");
  });

  it("does nothing for a lesson dropped on itself", () => {
    expect(dropLesson(base, "c1a", "c1a", "after")).toBe(base);
    expect(dropLesson(base, "c1a", "nope", "after")).toBe(base);
  });

  it("puts a lesson dropped on a chapter last in it", () => {
    expect(ids(dropLessonOnChapter(base, "c1a", "c3"))[2]).toBe("c3:c3a,c3b,c1a");
    expect(dropLessonOnChapter(base, "c1a", "nope")).toBe(base);
  });

  it("puts a chapter before or after the one it lands on", () => {
    const order = (l: Layout) => l.chapters.map((c) => c.id).join();
    expect(order(dropChapter(base, "c3", "c1", "before"))).toBe("c3,c1,c2");
    expect(order(dropChapter(base, "c1", "c2", "after"))).toBe("c2,c1,c3");
    expect(order(dropChapter(base, "c1", "c3", "before"))).toBe("c2,c1,c3");
    expect(dropChapter(base, "c1", "c1", "after")).toBe(base);
  });
});
