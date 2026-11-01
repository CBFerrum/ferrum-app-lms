// One course as its instructor sees it: how many are learning and how far they are, who has gone
// quiet, each learner's lessons and quiz attempts a click away, people to add by their addresses,
// and the work waiting for a grade. The server counts everything; this arranges it for the
// questions an instructor asks: who needs a nudge, and what is waiting for me.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, Fragment, useCallback, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { ApiError, errorMessage, formatDateTime, relativeTime } from "@ferrum/client";
import { Alert, Badge, Button, Dialog, EmptyState, ErrorState, Field, PageLoading, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { enrolLearners, type EnrolResult, getCourse, getRoster, keys, rowsOf } from "./api";
import { Glyph } from "./Glyph";
import { Grading } from "./Grading";
import { paths } from "./lesson";
import { ProgressLine } from "./parts";
import { cohortsOf, distribution, filterRoster, isQuiet, progressOf, rosterFilterFrom, type RosterFilter, rosterParams, SORTS, sortRoster, summarize } from "./roster";
import type { OutlineChapter, Roster, RosterLearner } from "./types";

// ───────────────────────────── one learner ─────────────────────────────

interface Done {
  lesson: string;
  completed_at: string;
  completed_via: string;
}

interface Attempt {
  id: string;
  quiz_title: string;
  attempt_no: number;
  status: string;
  percentage: string | null;
  passed: boolean;
  submitted_at: string | null;
}

/** A learner's lessons, ticked, and their quiz attempts: read when the row is opened. */
function LearnerDetail({ l, slug }: { l: RosterLearner; slug: string }) {
  const course = useQuery({ queryKey: keys.course(slug), queryFn: () => getCourse(slug), staleTime: 60_000 });
  const done = useQuery({
    queryKey: ["lms", "learner-lessons", l.enrollment],
    queryFn: () => rowsOf<Done>("LmsLessonProgress", { filters: JSON.stringify({ enrollment: l.enrollment }), fields: "lesson,completed_at,completed_via", limit: 500 }),
  });
  const attempts = useQuery({
    queryKey: ["lms", "learner-attempts", l.enrollment],
    queryFn: () =>
      rowsOf<Attempt>("LmsQuizAttempt", { filters: JSON.stringify({ enrollment: l.enrollment }), fields: "quiz_title,attempt_no,status,percentage,passed,submitted_at", sort: "created_at desc", limit: 50 }),
  });
  const when = useMemo(() => new Map((done.data ?? []).map((d) => [d.lesson, d])), [done.data]);
  const chapters: OutlineChapter[] = course.data?.chapters ?? [];

  const again = () => {
    void done.refetch();
    void course.refetch();
  };
  if (done.error || course.error) return <ErrorState error={done.error ?? course.error} onRetry={again} title="Could not load this learner" />;
  if (!done.data || !course.data) return <PageLoading label="Loading this learner" />;
  return (
    <div className="lms-detail">
      <div>
        <h4 className="lms-subhead">Lessons</h4>
        {chapters.map((c) => (
          <div key={c.id} className="lms-detail-chapter">
            <p className="lms-detail-title">{c.title}</p>
            <ul className="lms-plain">
              {c.lessons.map((x) => {
                const d = when.get(x.id);
                return (
                  <li key={x.id} className="lms-detail-lesson" data-done={!!d || undefined}>
                    <Glyph name={d ? "circleCheck" : "circle"} size={14} />
                    <span>{x.title}</span>
                    {d ? <span className="faint">{formatDateTime(d.completed_at)}</span> : x.required ? null : <span className="faint">optional</span>}
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
      <div>
        <h4 className="lms-subhead">Quiz attempts</h4>
        {attempts.data?.length ? (
          <ul className="lms-plain">
            {attempts.data.map((a) => (
              <li key={a.id}>
                <strong>{a.quiz_title}</strong> <span className="faint">attempt {a.attempt_no}</span>
                <br />
                {a.status === "In Progress" ? (
                  <Badge tone="blue">In progress</Badge>
                ) : (
                  <>
                    {a.percentage !== null && <span>{Number(a.percentage)}% </span>}
                    <Badge tone={a.passed ? "green" : "red"}>{a.passed ? "Passed" : a.status === "Timed Out" ? "Timed out" : "Not passed"}</Badge>
                  </>
                )}
                {a.submitted_at && <span className="faint"> {formatDateTime(a.submitted_at)}</span>}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">No quiz attempts yet.</p>
        )}
        <p className="lms-detail-link">
          <Link to={paths.record("LmsEnrollment", l.enrollment)}>Open the enrolment record</Link>
        </p>
      </div>
    </div>
  );
}

// ───────────────────────────── the learners ─────────────────────────────

const STATUS_TONE = { Active: "blue", Completed: "green", Withdrawn: "neutral" } as const;

function Learners({ roster, now }: { roster: Roster; now: number }) {
  const [sp, setSp] = useSearchParams();
  const f = rosterFilterFrom(sp);
  const set = (patch: Partial<RosterFilter>) => {
    const next = rosterParams({ ...f, ...patch });
    for (const keep of ["tab"]) if (sp.get(keep)) next.set(keep, sp.get(keep)!);
    setSp(next, { replace: true });
  };
  const [open, setOpen] = useState<string | null>(null);
  const shown = useMemo(() => sortRoster(filterRoster(roster.learners, f), f.sort), [roster.learners, f.q, f.status, f.cohort, f.sort]);
  const cohorts = cohortsOf(roster.learners);

  return (
    <>
      <div className="lms-filters" role="search">
        <input className="input lms-search" type="search" aria-label="Find a learner" placeholder="Find a learner…" value={f.q} onChange={(e) => set({ q: e.target.value })} />
        <select className="select lms-select" aria-label="Status" value={f.status} onChange={(e) => set({ status: e.target.value as RosterFilter["status"] })}>
          <option value="">Everyone</option>
          <option value="Active">Learning now</option>
          <option value="Completed">Finished</option>
          <option value="Withdrawn">Left the course</option>
        </select>
        {cohorts.length > 0 && (
          <select className="select lms-select" aria-label="Cohort" value={f.cohort} onChange={(e) => set({ cohort: e.target.value })}>
            <option value="">All cohorts</option>
            {cohorts.map((c) => (
              <option key={c.id} value={c.id}>
                {c.title}
              </option>
            ))}
          </select>
        )}
        <select className="select lms-select" aria-label="Order" value={f.sort} onChange={(e) => set({ sort: e.target.value as RosterFilter["sort"] })}>
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
      </div>
      {!shown.length ? (
        <EmptyState title={roster.learners.length ? "No learner matches" : "Nobody is enrolled yet"} icon="user" />
      ) : (
        <div className="lms-panel">
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Learner</th>
                  <th scope="col">Progress</th>
                  <th scope="col">Last opened</th>
                  <th scope="col">Quizzes</th>
                  <th scope="col">Status</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((l) => {
                  const expanded = open === l.enrollment;
                  const quiet = isQuiet(l, now);
                  return (
                    <Fragment key={l.enrollment}>
                      <tr>
                        <td>
                          <button type="button" className="lms-expander" aria-expanded={expanded} onClick={() => setOpen(expanded ? null : l.enrollment)}>
                            <Glyph name="chevronRight" size={12} />
                            <span className="lms-who">
                              <strong>{l.learner_name || l.learner}</strong>
                              {l.learner_name && <span className="faint">{l.learner}</span>}
                            </span>
                          </button>
                          <div className="lms-marks">
                            {quiet && <Badge tone="amber">Quiet</Badge>}
                            {l.submissions_to_grade > 0 && <Badge tone="blue">{l.submissions_to_grade} to grade</Badge>}
                            {l.cohort?.title && <span className="faint">{l.cohort.title}</span>}
                          </div>
                        </td>
                        <td className="lms-col-progress">
                          <ProgressLine value={progressOf(l)} label={`Progress of ${l.learner_name || l.learner}`} />
                          <span className="faint lms-lessons-done">
                            {l.completed_lessons} of {l.total_lessons} lessons
                          </span>
                        </td>
                        <td>{l.last_activity_at ? <span title={formatDateTime(l.last_activity_at)}>{relativeTime(l.last_activity_at)}</span> : <span className="faint">Not started</span>}</td>
                        <td>
                          {l.quizzes_passed || l.best_quiz_percentage ? (
                            <>
                              {l.quizzes_passed} passed
                              {l.best_quiz_percentage && <span className="faint"> · best {Number(l.best_quiz_percentage)}%</span>}
                            </>
                          ) : (
                            <span className="faint">None yet</span>
                          )}
                        </td>
                        <td>
                          <Badge tone={STATUS_TONE[l.status]}>{l.status === "Active" ? "Learning" : l.status === "Completed" ? "Finished" : "Left"}</Badge>
                        </td>
                      </tr>
                      {expanded && (
                        <tr className="lms-detail-row">
                          <td colSpan={5}>
                            <LearnerDetail l={l} slug={roster.course.slug} />
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </>
  );
}

// ───────────────────────────── adding people ─────────────────────────────

function AddLearners({ course, open, onClose, onDone }: { course: string; open: boolean; onClose: () => void; onDone: () => void }) {
  const [text, setText] = useState("");
  const [cohort, setCohort] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<EnrolResult | null>(null);
  const cohorts = useQuery({
    queryKey: ["lms", "cohorts", course],
    enabled: open,
    queryFn: () => rowsOf<{ id: string; title: string }>("LmsCohort", { filters: JSON.stringify({ course }), fields: "title", sort: "start_date desc", limit: 100 }),
  });
  const close = () => {
    setResult(null);
    setError(null);
    onClose();
  };
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      setResult(await enrolLearners(course, text, cohort || undefined));
      setText("");
      onDone();
    } catch (err) {
      setError(err instanceof ApiError && err.fields.length ? err.fields.map((f) => f.message).join(" ") : errorMessage(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onClose={close} title="Add learners">
      {result ? (
        <div className="stack">
          <Alert tone={result.enrolled.length ? "success" : "info"}>
            {result.enrolled.length} added{result.already_enrolled.length ? `, ${result.already_enrolled.length} already in the course` : ""}
            {result.skipped.length ? `, ${result.skipped.length} could not be added` : ""}.
          </Alert>
          {result.skipped.length > 0 && (
            <ul className="lms-plain">
              {result.skipped.map((s) => (
                <li key={s.learner}>
                  <strong>{s.learner}</strong> <span className="muted">{s.reason}</span>
                </li>
              ))}
            </ul>
          )}
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </div>
        </div>
      ) : (
        <form className="stack" onSubmit={(e) => void submit(e)}>
          {error && <Alert tone="error">{error}</Alert>}
          <Field label="Email addresses" htmlFor="lms-add" required hint="The addresses they sign in with, one to a line or separated by commas. Up to 500.">
            <textarea id="lms-add" className="textarea" rows={6} value={text} onChange={(e) => setText(e.target.value)} placeholder={"asha@school.test\nbo@school.test"} />
          </Field>
          {(cohorts.data?.length ?? 0) > 0 && (
            <Field label="Cohort" htmlFor="lms-add-group">
              <select id="lms-add-group" className="select" value={cohort} onChange={(e) => setCohort(e.target.value)}>
                <option value="">No cohort</option>
                {cohorts.data!.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.title}
                  </option>
                ))}
              </select>
            </Field>
          )}
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <Button onClick={close}>Cancel</Button>
            <Button type="submit" variant="primary" loading={busy} disabled={!text.trim()}>
              Add learners
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

// ───────────────────────────── the page ─────────────────────────────

function Figure({ value, label, tone }: { value: number | string; label: string; tone?: "warn" }) {
  return (
    <div className="lms-figure" data-tone={tone}>
      <strong>{value}</strong>
      <span className="faint">{label}</span>
    </div>
  );
}

export function CourseDashboardRoute() {
  const { course = "" } = useParams();
  return <CourseDashboard key={course} course={course} />;
}

function CourseDashboard({ course }: { course: string }) {
  const qc = useQueryClient();
  const [sp, setSp] = useSearchParams();
  const tab = sp.get("tab") === "grading" ? "grading" : "learners";
  const [adding, setAdding] = useState(false);
  const q = useQuery({ queryKey: keys.roster(course), queryFn: () => getRoster(course), staleTime: 0 });
  const refresh = useCallback(() => void qc.invalidateQueries({ queryKey: ["lms", "roster"] }), [qc]);
  useLiveEntity("LmsEnrollment", refresh);
  useLiveEntity("LmsAssignmentSubmission", refresh);
  const now = useMemo(() => Date.now(), [q.data]);
  useShellTitle(q.data ? `${q.data.course.title} · Learners` : null);

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not load this course's learners" />;
  const roster = q.data;
  if (!roster) return <PageLoading label="Loading the learners" />;
  const sum = summarize(roster, now);
  const bands = distribution(roster.learners);
  const widest = Math.max(1, ...bands.map((b) => b.count));
  const go = (t: "learners" | "grading") => setSp(t === "grading" ? { tab: "grading" } : {}, { replace: true });

  return (
    <div className="lms-page">
      <nav className="crumbs" aria-label="Breadcrumb">
        <Link to={paths.teaching()}>Teaching</Link>
        <Glyph name="chevronRight" size={12} />
      </nav>
      <header className="lms-head">
        <div>
          <h1 className="page-title">{roster.course.title}</h1>
          <p className="page-sub">Your learners</p>
        </div>
        <div className="lms-tools">
          <Button icon="plus" onClick={() => setAdding(true)}>
            Add learners
          </Button>
          <Link className="btn btn-default btn-md" to={paths.build(roster.course.id)}>
            Build
          </Link>
          <Link className="btn btn-default btn-md" to={paths.course(roster.course.slug)}>
            Course page
          </Link>
        </div>
      </header>
      <div className="lms-figures">
        <Figure value={sum.enrolled} label="enrolled" />
        <Figure value={sum.completed} label={`finished (${Number(roster.completion_rate)}%)`} />
        <Figure value={`${sum.average}%`} label="average progress" />
        <Figure value={sum.quiet} label="gone quiet" tone={sum.quiet ? "warn" : undefined} />
        <Figure value={sum.toGrade} label="to grade" tone={sum.toGrade ? "warn" : undefined} />
      </div>
      {sum.enrolled > 0 && (
        <section aria-label="Where the class is">
          <h2 className="lms-group">Where the class is</h2>
          <ul className="lms-bands">
            {bands.map((b) => (
              <li key={b.label}>
                <span>{b.label}</span>
                <span className="lms-band-bar" aria-hidden="true">
                  <span style={{ width: `${(b.count / widest) * 100}%` }} />
                </span>
                <span className="num">{b.count}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <div className="lms-switch" role="tablist" aria-label="Learners and grading">
        <button type="button" role="tab" aria-selected={tab === "learners"} onClick={() => go("learners")}>
          Learners
        </button>
        <button type="button" role="tab" aria-selected={tab === "grading"} onClick={() => go("grading")}>
          To grade {sum.toGrade > 0 && <Badge tone="amber">{sum.toGrade}</Badge>}
        </button>
      </div>
      {tab === "learners" ? <Learners roster={roster} now={now} /> : <Grading course={course} />}
      <AddLearners course={course} open={adding} onClose={() => setAdding(false)} onDone={refresh} />
    </div>
  );
}
