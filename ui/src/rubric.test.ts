import { describe, expect, it } from "vitest";
import { aboutPoints, chosenPoints, complete } from "./rubric";

const def = { criteria: [{ criterion: "Argument" }, { criterion: "Style" }], levels: [{ criterion: "Argument", level: "Strong", points: "6" }, { criterion: "Argument", level: "Weak", points: "2" }, { criterion: "Style", level: "Clear", points: "4" }, { criterion: "Style", level: "Muddled", points: "1" }], max_points: "10" };

describe("a rubric's choice", () => {
  it("adds the chosen levels and scales them to the question", () => {
    expect(chosenPoints(def, { Argument: "Strong", Style: "Muddled" })).toBe(7);
    expect(aboutPoints(def, { Argument: "Strong", Style: "Muddled" }, 4)).toBe(2.8);
  });
  it("is complete only when every criterion has a level", () => {
    expect(complete(def, { Argument: "Strong" })).toBe(false);
    expect(complete(def, { Argument: "Strong", Style: "Clear" })).toBe(true);
  });
});
