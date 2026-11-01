// A course's learners as an instructor reads them: found by name or address, sorted by how recently
// they were active, and counted into the bands that show where a class is. Pure, so the numbers an
// instructor acts on are tested. The server computes each learner's progress; this only arranges it.

import type { EnrollmentStatus, Roster, RosterLearner } from "./types";

export type Sort = "recent" | "quiet" | "progress" | "name";

export interface RosterFilter {
  q: string;
  status: "" | EnrollmentStatus;
  cohort: string;
  sort: Sort;
}

export const SORTS: { value: Sort; label: string }[] = [
  { value: "recent", label: "Most recent activity" },
  { value: "quiet", label: "Quiet the longest" },
  { value: "progress", label: "Lowest progress" },
  { value: "name", label: "Name" },
];

/** How many days without opening anything make a learner "quiet". */
export const QUIET_DAYS = 14;

export const NO_ROSTER_FILTER: RosterFilter = { q: "", status: "", cohort: "", sort: "recent" };

export const progressOf = (l: Pick<RosterLearner, "progress_percent">): number => {
  const n = Number(l.progress_percent ?? 0);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : 0;
};

const name = (l: RosterLearner) => (l.learner_name || l.learner).toLowerCase();
const at = (l: RosterLearner) => (l.last_activity_at ? Date.parse(l.last_activity_at) : null);

export function filterRoster(learners: RosterLearner[], f: RosterFilter): RosterLearner[] {
  const q = f.q.trim().toLowerCase();
  return learners.filter(
    (l) => (!q || name(l).includes(q) || l.learner.toLowerCase().includes(q)) && (!f.status || l.status === f.status) && (!f.cohort || l.cohort?.id === f.cohort),
  );
}

/** Learners with no activity last; among those, by name. */
export function sortRoster(learners: RosterLearner[], sort: Sort): RosterLearner[] {
  const byName = (a: RosterLearner, b: RosterLearner) => name(a).localeCompare(name(b));
  const list = [...learners];
  switch (sort) {
    case "name":
      return list.sort(byName);
    case "progress":
      return list.sort((a, b) => progressOf(a) - progressOf(b) || byName(a, b));
    case "quiet":
      // Never opened anything is the quietest of all.
      return list.sort((a, b) => (at(a) ?? -Infinity) - (at(b) ?? -Infinity) || byName(a, b));
    default:
      return list.sort((a, b) => (at(b) ?? -Infinity) - (at(a) ?? -Infinity) || byName(a, b));
  }
}

/** Whole days since a learner last opened anything; null if never. */
export function idleDays(l: Pick<RosterLearner, "last_activity_at">, now: number): number | null {
  const t = l.last_activity_at ? Date.parse(l.last_activity_at) : NaN;
  return Number.isNaN(t) ? null : Math.max(0, Math.floor((now - t) / 86_400_000));
}

/** Someone still taking the course who has gone quiet. */
export function isQuiet(l: RosterLearner, now: number, days = QUIET_DAYS): boolean {
  if (l.status !== "Active") return false;
  const idle = idleDays(l, now);
  return idle === null || idle >= days;
}

export interface Band {
  label: string;
  count: number;
}

/** Learners (not those who left) by how far they are: not started, quarters, finished. */
export function distribution(learners: RosterLearner[]): Band[] {
  const live = learners.filter((l) => l.status !== "Withdrawn");
  const bands: Band[] = [
    { label: "Not started", count: 0 },
    { label: "Under 25%", count: 0 },
    { label: "25% to 49%", count: 0 },
    { label: "50% to 74%", count: 0 },
    { label: "75% or more", count: 0 },
    { label: "Finished", count: 0 },
  ];
  for (const l of live) {
    const p = progressOf(l);
    bands[l.status === "Completed" || p >= 100 ? 5 : p >= 75 ? 4 : p >= 50 ? 3 : p >= 25 ? 2 : p > 0 ? 1 : 0].count++;
  }
  return bands;
}

export interface Summary {
  enrolled: number;
  completed: number;
  /** Mean progress of everyone who has not left, 0 to 100. */
  average: number;
  quiet: number;
  toGrade: number;
}

export function summarize(roster: Pick<Roster, "learners">, now: number): Summary {
  const live = roster.learners.filter((l) => l.status !== "Withdrawn");
  return {
    enrolled: live.length,
    completed: live.filter((l) => l.status === "Completed").length,
    average: live.length ? Math.round((live.reduce((s, l) => s + progressOf(l), 0) / live.length) * 10) / 10 : 0,
    quiet: live.filter((l) => isQuiet(l, now)).length,
    toGrade: live.reduce((s, l) => s + l.submissions_to_grade, 0),
  };
}

/** The cohorts the learners belong to, once each, by name. */
export function cohortsOf(learners: RosterLearner[]): { id: string; title: string }[] {
  const seen = new Map<string, string>();
  for (const l of learners) if (l.cohort) seen.set(l.cohort.id, l.cohort.title ?? "Cohort");
  return [...seen].map(([id, title]) => ({ id, title })).sort((a, b) => a.title.localeCompare(b.title));
}

/** The address' query for a filter: only what differs from the default. */
export function rosterParams(f: Partial<RosterFilter>): URLSearchParams {
  const sp = new URLSearchParams();
  if (f.q) sp.set("q", f.q);
  if (f.status) sp.set("status", f.status);
  if (f.cohort) sp.set("cohort", f.cohort);
  if (f.sort && f.sort !== NO_ROSTER_FILTER.sort) sp.set("sort", f.sort);
  return sp;
}

export function rosterFilterFrom(sp: URLSearchParams): RosterFilter {
  const sort = sp.get("sort") as Sort;
  const status = sp.get("status") ?? "";
  return {
    q: (sp.get("q") ?? "").slice(0, 100),
    status: (["Active", "Completed", "Withdrawn"] as string[]).includes(status) ? (status as EnrollmentStatus) : "",
    cohort: sp.get("cohort") ?? "",
    sort: SORTS.some((s) => s.value === sort) ? sort : "recent",
  };
}
