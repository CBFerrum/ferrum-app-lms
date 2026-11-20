import { describe, expect, it } from "vitest";
import { cellText, markOf } from "./gradeview";

const mark = (item: string, over = {}) => ({ item, points: null, excused: false, late_days: 0, late_penalty_percent: "0", overridden: false, comment: null, ...over });

describe("reading a gradebook", () => {
  it("a_mark_is_found_by_its_item_not_its_place", () => {
    const learner = { marks: [mark("b", { points: "5" }), mark("a", { points: "9" })] };
    expect(markOf(learner, "a")?.points).toBe("9");
    expect(markOf(learner, "zz")).toBeUndefined();
  });
  it("a cell reads as points, excused or nothing", () => {
    expect(cellText(mark("a", { points: "9" }))).toEqual({ kind: "points", points: "9" });
    expect(cellText(mark("a", { excused: true, points: "3" }))).toEqual({ kind: "excused" });
    expect(cellText(mark("a"))).toEqual({ kind: "none" });
    expect(cellText(undefined)).toEqual({ kind: "none" });
  });
});
