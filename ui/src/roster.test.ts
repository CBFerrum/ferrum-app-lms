import { describe, expect, it } from "vitest";
import { cohortsOf, distribution, filterRoster, idleDays, isQuiet, NO_ROSTER_FILTER, rosterFilterFrom, rosterParams, sortRoster, summarize } from "./roster";
import type { RosterLearner } from "./types";

const NOW = Date.parse("2026-11-20T12:00:00Z");
const day = (n: number) => new Date(NOW - n * 86_400_000).toISOString();

const learner = (id: string, over: Partial<RosterLearner> = {}): RosterLearner => ({
  enrollment: id,
  number: `ENR-${id}`,
  learner: `${id}@school.test`,
  learner_name: id.toUpperCase(),
  status: "Active",
  progress_percent: "0",
  completed_lessons: 0,
  total_lessons: 10,
  enrolled_on: "2026-10-01",
  last_activity_at: null,
  completed_at: null,
  cohort: null,
  certificate: null,
  quizzes_passed: 0,
  best_quiz_percentage: null,
  submissions_to_grade: 0,
  ...over,
});

const class_ = [
  learner("asha", { progress_percent: "80", last_activity_at: day(1) }),
  learner("bo", { progress_percent: "10", last_activity_at: day(20), cohort: { id: "c1", title: "Autumn" } }),
  learner("chen", { progress_percent: "100", status: "Completed", last_activity_at: day(3), cohort: { id: "c1", title: "Autumn" }, submissions_to_grade: 0 }),
  learner("dia", { progress_percent: "0" }),
  learner("eli", { progress_percent: "40", status: "Withdrawn", last_activity_at: day(40) }),
  learner("fay", { progress_percent: "55.5", last_activity_at: day(5), submissions_to_grade: 2 }),
];

describe("an instructor's class list", () => {
  it("finds learners by name or address, status and cohort", () => {
    const f = NO_ROSTER_FILTER;
    expect(filterRoster(class_, { ...f, q: "  CHE " }).map((l) => l.enrollment)).toEqual(["chen"]);
    expect(filterRoster(class_, { ...f, q: "school.test" })).toHaveLength(6);
    expect(filterRoster(class_, { ...f, status: "Withdrawn" }).map((l) => l.enrollment)).toEqual(["eli"]);
    expect(filterRoster(class_, { ...f, cohort: "c1" }).map((l) => l.enrollment)).toEqual(["bo", "chen"]);
  });

  it("sorts by recent activity, never-active last", () => {
    expect(sortRoster(class_, "recent").map((l) => l.enrollment)).toEqual(["asha", "chen", "fay", "bo", "eli", "dia"]);
  });

  it("sorts the quietest first, those who never opened anything before all", () => {
    expect(sortRoster(class_, "quiet").map((l) => l.enrollment)).toEqual(["dia", "eli", "bo", "fay", "chen", "asha"]);
  });

  it("sorts by lowest progress, then name", () => {
    expect(sortRoster(class_, "progress").map((l) => l.enrollment)).toEqual(["dia", "bo", "eli", "fay", "asha", "chen"]);
    expect(sortRoster(class_, "name").map((l) => l.enrollment)).toEqual(["asha", "bo", "chen", "dia", "eli", "fay"]);
  });

  it("does not change the list it sorts", () => {
    const copy = [...class_];
    sortRoster(class_, "name");
    expect(class_).toEqual(copy);
  });

  it("knows who has gone quiet: still learning, and away for two weeks or never started", () => {
    expect(idleDays(class_[1], NOW)).toBe(20);
    expect(idleDays(class_[3], NOW)).toBeNull();
    expect(class_.filter((l) => isQuiet(l, NOW)).map((l) => l.enrollment)).toEqual(["bo", "dia"]);
  });

  it("counts learners into bands, leaving out those who left", () => {
    expect(distribution(class_).map((b) => b.count)).toEqual([1, 1, 0, 1, 1, 1]);
    expect(distribution([]).every((b) => b.count === 0)).toBe(true);
  });

  it("sums up the class", () => {
    expect(summarize({ learners: class_ }, NOW)).toEqual({ enrolled: 5, completed: 1, average: 49.1, quiet: 2, toGrade: 2 });
    expect(summarize({ learners: [] }, NOW)).toEqual({ enrolled: 0, completed: 0, average: 0, quiet: 0, toGrade: 0 });
  });

  it("lists the cohorts once each", () => {
    expect(cohortsOf(class_)).toEqual([{ id: "c1", title: "Autumn" }]);
  });

  it("keeps its filter in the address", () => {
    const f = rosterFilterFrom(new URLSearchParams("q=asha&status=Completed&sort=quiet&cohort=c1"));
    expect(f).toEqual({ q: "asha", status: "Completed", cohort: "c1", sort: "quiet" });
    expect(rosterParams(f).toString()).toBe("q=asha&status=Completed&cohort=c1&sort=quiet");
    expect(rosterParams(NO_ROSTER_FILTER).toString()).toBe("");
    expect(rosterFilterFrom(new URLSearchParams("status=Bogus&sort=bogus"))).toEqual(NO_ROSTER_FILTER);
  });
});
