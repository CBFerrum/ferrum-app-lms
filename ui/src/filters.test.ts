import { describe, expect, it } from "vitest";
import { filtersFrom, hasFilters, NO_FILTERS, paramsFor, queryFor } from "./filters";

describe("a catalog's filters in its address", () => {
  it("reads what is set and ignores what is not valid", () => {
    expect(filtersFrom(new URLSearchParams("q=rust&level=Advanced&category=c1&tag=safety"))).toEqual({ q: "rust", level: "Advanced", category: "c1", tag: "safety" });
    expect(filtersFrom(new URLSearchParams("level=Expert&q=%20%20"))).toEqual(NO_FILTERS);
    expect(filtersFrom(new URLSearchParams("q=" + "x".repeat(300))).q).toHaveLength(100);
  });

  it("writes only the filters that are set, and asks for only those", () => {
    expect(paramsFor({ q: "rust", level: "", category: "c1" }).toString()).toBe("q=rust&category=c1");
    expect(queryFor({ ...NO_FILTERS, q: "rust" }, "cur")).toEqual({ q: "rust", cursor: "cur" });
    expect(queryFor(NO_FILTERS)).toEqual({});
    expect(hasFilters(NO_FILTERS)).toBe(false);
    expect(hasFilters({ ...NO_FILTERS, tag: "x" })).toBe(true);
  });
});
