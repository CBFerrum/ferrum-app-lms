import { describe, expect, it } from "vitest";
import { clock, courseFacts, daysBetween, dueLabel, duration, hue, monogram, personName, plural } from "./format";

describe("words for numbers and dates", () => {
  it("says how long a lesson takes", () => {
    expect([0, null, undefined, -5].map(duration)).toEqual(["", "", "", ""]);
    expect([5, 45, 60, 90, 120, 200].map(duration)).toEqual(["5 min", "45 min", "1 h", "1 h 30 min", "2 h", "3 h 20 min"]);
  });

  it("shows seconds on a clock", () => {
    expect([0, 5, 75, 600, 3725, -3].map(clock)).toEqual(["0:00", "0:05", "1:15", "10:00", "1:02:05", "0:00"]);
  });

  it("counts days between dates, across months and daylight saving", () => {
    expect(daysBetween("2026-11-01", "2026-11-04")).toBe(3);
    expect(daysBetween("2026-10-31", "2026-11-01")).toBe(1);
    expect(daysBetween("2026-03-28", "2026-03-30")).toBe(2);
    expect(daysBetween("2026-11-05", "2026-11-01")).toBe(-4);
  });

  it("says when work is due", () => {
    const today = "2026-11-10";
    expect(dueLabel("2026-11-10", today)).toBe("Due today");
    expect(dueLabel("2026-11-11", today)).toBe("Due tomorrow");
    expect(dueLabel("2026-11-13", today)).toBe("Due in 3 days");
    expect(dueLabel("2026-11-09", today)).toBe("1 day late");
    expect(dueLabel("2026-11-05", today)).toBe("5 days late");
    expect(dueLabel("2026-12-24", today)).toMatch(/^Due /);
    expect(dueLabel(null, today)).toBe("");
  });

  it("describes a course in a line", () => {
    expect(courseFacts(12, 200)).toBe("12 lessons · 3 h 20 min");
    expect(courseFacts(1, 0, "2.50")).toBe("1 lesson · 2.5 h");
    expect(courseFacts(0, 0)).toBe("");
    expect(plural(1, "lesson")).toBe("1 lesson");
    expect(plural(0, "person", "people")).toBe("0 people");
  });

  it("names people by what is known of them", () => {
    expect(personName(" Asha Rao ", "asha@x.test")).toBe("Asha Rao");
    expect(personName("", "asha@x.test")).toBe("asha@x.test");
    expect(personName(null, null)).toBe("Someone");
  });

  it("gives a course without a picture a monogram and a colour", () => {
    expect(monogram("Intro to Rust")).toBe("IT");
    expect(monogram("Safety")).toBe("SA");
    expect(monogram("  ")).toBe("?");
    expect(monogram("Über-Kurs")).toBe("ÜK");
    expect(hue("intro-to-rust")).toBe(hue("intro-to-rust"));
    expect([hue("a"), hue("b"), hue("c")].every((n) => n >= 1 && n <= 8)).toBe(true);
  });
});
