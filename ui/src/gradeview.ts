// What the gradebook screens decide for themselves (everything numeric is the server's).

import type { GradeLearner, GradeMark } from "./types";

/** A learner's mark for an item, found by the item and not by its place in the list. */
export function markOf(learner: Pick<GradeLearner, "marks">, item: string): GradeMark | undefined {
  return learner.marks.find((m) => m.item === item);
}

/** How a cell reads: "Excused", the points, or null when nothing is graded yet (the screen draws a dash). */
export function cellText(mark: GradeMark | undefined): { kind: "excused" } | { kind: "points"; points: string } | { kind: "none" } {
  if (!mark) return { kind: "none" };
  if (mark.excused) return { kind: "excused" };
  return mark.points === null ? { kind: "none" } : { kind: "points", points: mark.points };
}
