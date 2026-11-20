// A rubric on screen: what a choice of levels comes to. The server fills the rubric and scales it; these
// figures are only the "about" the grader sees while choosing.

import type { RubricDef } from "./types";

/** The points of the chosen levels, added (a criterion without a choice adds nothing). */
export function chosenPoints(def: Pick<RubricDef, "levels">, chosen: Record<string, string>): number {
  return Object.entries(chosen).reduce((sum, [criterion, level]) => sum + Number(def.levels.find((l) => l.criterion === criterion && l.level === level)?.points ?? 0), 0);
}

/** About what the server will award out of `outOf`: the chosen points over the rubric's maximum, scaled. */
export function aboutPoints(def: Pick<RubricDef, "levels" | "max_points">, chosen: Record<string, string>, outOf: number): number {
  const max = Number(def.max_points);
  return max > 0 ? Math.round((chosenPoints(def, chosen) / max) * outOf * 100) / 100 : 0;
}

/** Whether every criterion has a level. */
export function complete(def: Pick<RubricDef, "criteria">, chosen: Record<string, string>): boolean {
  return def.criteria.length > 0 && def.criteria.every((c) => !!chosen[c.criterion]);
}
