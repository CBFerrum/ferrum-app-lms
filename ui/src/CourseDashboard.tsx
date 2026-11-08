// One course as its instructor sees it: how many are learning and how far they are, who has gone
// quiet, each learner's lessons and quiz attempts a click away, people to add by their addresses,
// and the work waiting for a grade. The server counts everything; this arranges it for the
// questions an instructor asks: who needs a nudge, and what is waiting for me.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, Fragment, useCallback, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { ApiError, errorMessage, formatDateTime, relativeTime, t as tr, isolate } from "@ferrum/client";
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
  if (done.error || course.error) return <ErrorState error={done.error ?? course.error} onRetry={again} title={tr("Could not load this learner")} />;
  if (!done.data || !course.data) return <PageLoading label={tr("Loading this learner")} />;
  return (
    <div className="lms-detail">
      <div>
        <h4 className="lms-subhead">{tr("Lessons")}</h4>
        {chapters.map((c) => (
          <div key={c.id} className="lms-detail-chapter">
            <p className="lms-detail-title">{isolate(c.title)}</p>
            <ul className="lms-plain">
              {c.lessons.map((x) => {
                const d = when.get(x.id);
                return (
                  <li key={x.id} className="lms-detail-lesson" data-done={!!d || undefined}>
                    <Glyph name={d ? "circleCheck" : "circle"} size={14} />
                    <span>{isolate(x.title)}</span>
                    {d ? <span className="faint">{isolate(formatDateTime(d.completed_at))}</span> : x.required ? null : <span className="faint">{tr("optional")}</span>}
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
      <div>
        <h4 className="lms-subhead">{tr("Quiz attempts")}</h4>
        {attempts.data?.length ? (
          <ul className="lms-plain">
            {attempts.data.map((a) => (
              <li key={a.id}>
                <strong>{isolate(a.quiz_title)}</strong> <span className="faint">{tr("attempt {attempt_no}", { attempt_no: a.attempt_no })}</span>
                <br />
                {a.status === "In Progress" ? (
                  <Badge tone="blue">{tr("In progress")}</Badge>
                ) : (
                  <>
                    {a.percentage !== null && <span>{Number(a.percentage)}% </span>}
                    <Badge tone={a.passed ? "green" : "red"}>{a.passed ? tr("Passed") : a.status === "Timed Out" ? tr("Timed out") : tr("Not passed")}</Badge>
                  </>
                )}
                {a.submitted_at && <span className="faint"> {isolate(formatDateTime(a.submitted_at))}</span>}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">{tr("No quiz attempts yet.")}</p>
        )}
        <p className="lms-detail-link">
          <Link to={paths.record("LmsEnrollment", l.enrollment)}>{tr("Open the enrolment record")}</Link>
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
        <input className="input lms-search" type="search" aria-label={tr("Find a learner")} placeholder={tr("Find a learner…")} value={f.q} onChange={(e) => set({ q: e.target.value })} />
        <select className="select lms-select" aria-label={tr("Status")} value={f.status} onChange={(e) => set({ status: e.target.value as RosterFilter["status"] })}>
          <option value="">{tr("Everyone")}</option>
          <option value="Active">{tr("Learning now")}</option>
          <option value="Completed">{tr("Finished")}</option>
          <option value="Withdrawn">{tr("Left the course")}</option>
        </select>
        {cohorts.length > 0 && (
          <select className="select lms-select" aria-label={tr("Cohort")} value={f.cohort} onChange={(e) => set({ cohort: e.target.value })}>
            <option value="">{tr("All cohorts")}</option>
            {cohorts.map((c) => (
              <option key={c.id} value={c.id}>
                {isolate(c.title)}
              </option>
            ))}
          </select>
        )}
        <select className="select lms-select" aria-label={tr("Order")} value={f.sort} onChange={(e) => set({ sort: e.target.value as RosterFilter["sort"] })}>
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {isolate(s.label)}
            </option>
          ))}
        </select>
      </div>
      {!shown.length ? (
        <EmptyState title={roster.learners.length ? tr("No learner matches") : tr("Nobody is enrolled yet")} icon="user" />
      ) : (
        <div className="lms-panel">
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{tr("Learner")}</th>
                  <th scope="col">{tr("Progress")}</th>
                  <th scope="col">{tr("Last opened")}</th>
                  <th scope="col">{tr("Quizzes")}</th>
                  <th scope="col">{tr("Status")}</th>
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
                              {l.learner_name && <span className="faint">{isolate(l.learner)}</span>}
                            </span>
                          </button>
                          <div className="lms-marks">
                            {quiet && <Badge tone="amber">{tr("Quiet")}</Badge>}
                            {l.submissions_to_grade > 0 && <Badge tone="blue">{tr("{submissions_to_grade} to grade", { submissions_to_grade: l.submissions_to_grade })}</Badge>}
                            {l.cohort?.title && <span className="faint">{isolate(l.cohort.title)}</span>}
                          </div>
                        </td>
                        <td className="lms-col-progress">
                          <ProgressLine value={progressOf(l)} label={tr("Progress of {value}", { value: l.learner_name || l.learner })} />
                          <span className="faint lms-lessons-done">
                            {tr("{completed_lessons} of {total_lessons} lessons", { completed_lessons: l.completed_lessons, total_lessons: l.total_lessons })}
                          </span>
                        </td>
                        <td>{l.last_activity_at ? <span title={formatDateTime(l.last_activity_at)}>{isolate(relativeTime(l.last_activity_at))}</span> : <span className="faint">{tr("Not started")}</span>}</td>
                        <td>
                          {l.quizzes_passed || l.best_quiz_percentage ? (
                            <>
                              {tr("{quizzes_passed} passed", { quizzes_passed: l.quizzes_passed })}
                              {l.best_quiz_percentage && <span className="faint"> {tr("· best")} {Number(l.best_quiz_percentage)}%</span>}
                            </>
                          ) : (
                            <span className="faint">{tr("None yet")}</span>
                          )}
                        </td>
                        <td>
                          <Badge tone={STATUS_TONE[l.status]}>{l.status === "Active" ? tr("Learning") : l.status === "Completed" ? tr("Finished") : tr("Left")}</Badge>
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
    <Dialog open={open} onClose={close} title={tr("Add learners")}>
      {result ? (
        <div className="stack">
          <Alert tone={result.enrolled.length ? "success" : "info"}>
            {result.already_enrolled.length ? (result.skipped.length ? tr("{added} added, {already} already in the course, {skipped} could not be added", { added: result.enrolled.length, already: result.already_enrolled.length, skipped: result.skipped.length }) : tr("{added} added, {already} already in the course", { added: result.enrolled.length, already: result.already_enrolled.length })) : result.skipped.length ? tr("{added} added, {skipped} could not be added", { added: result.enrolled.length, skipped: result.skipped.length }) : tr("{length} added", { length: result.enrolled.length })}.
          </Alert>
          {result.skipped.length > 0 && (
            <ul className="lms-plain">
              {result.skipped.map((s) => (
                <li key={s.learner}>
                  <strong>{isolate(s.learner)}</strong> <span className="muted">{isolate(s.reason)}</span>
                </li>
              ))}
            </ul>
          )}
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <Button variant="primary" onClick={close}>
              {tr("Done")}
            </Button>
          </div>
        </div>
      ) : (
        <form className="stack" onSubmit={(e) => void submit(e)}>
          {error && <Alert tone="error">{isolate(error)}</Alert>}
          <Field label={tr("Email addresses")} htmlFor="lms-add" required hint={tr("The addresses they sign in with, one to a line or separated by commas. Up to 500.")}>
            <textarea id="lms-add" className="textarea" rows={6} value={text} onChange={(e) => setText(e.target.value)} placeholder={tr("asha@school.test\nbo@school.test")} />
          </Field>
          {(cohorts.data?.length ?? 0) > 0 && (
            <Field label={tr("Cohort")} htmlFor="lms-add-group">
              <select id="lms-add-group" className="select" value={cohort} onChange={(e) => setCohort(e.target.value)}>
                <option value="">{tr("No cohort")}</option>
                {cohorts.data!.map((c) => (
                  <option key={c.id} value={c.id}>
                    {isolate(c.title)}
                  </option>
                ))}
              </select>
            </Field>
          )}
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <Button onClick={close}>{tr("Cancel")}</Button>
            <Button type="submit" variant="primary" loading={busy} disabled={!text.trim()}>
              {tr("Add learners")}
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
      <span className="faint">{isolate(label)}</span>
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
  useShellTitle(q.data ? tr("{title} · Learners", { title: q.data.course.title }) : null);

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load this course's learners")} />;
  const roster = q.data;
  if (!roster) return <PageLoading label={tr("Loading the learners")} />;
  const sum = summarize(roster, now);
  const bands = distribution(roster.learners);
  const widest = Math.max(1, ...bands.map((b) => b.count));
  const go = (t: "learners" | "grading") => setSp(t === "grading" ? { tab: "grading" } : {}, { replace: true });

  return (
    <div className="lms-page">
      <nav className="crumbs" aria-label={tr("Breadcrumb")}>
        <Link to={paths.teaching()}>{tr("Teaching")}</Link>
        <Glyph name="chevronRight" size={12} />
      </nav>
      <header className="lms-head">
        <div>
          <h1 className="page-title">{isolate(roster.course.title)}</h1>
          <p className="page-sub">{tr("Your learners")}</p>
        </div>
        <div className="lms-tools">
          <Button icon="plus" onClick={() => setAdding(true)}>
            {tr("Add learners")}
          </Button>
          <Link className="btn btn-default btn-md" to={paths.build(roster.course.id)}>
            {tr("Build")}
          </Link>
          <Link className="btn btn-default btn-md" to={paths.course(roster.course.slug)}>
            {tr("Course page")}
          </Link>
        </div>
      </header>
      <div className="lms-figures">
        <Figure value={sum.enrolled} label="enrolled" />
        <Figure value={sum.completed} label={tr("finished ({value}%)", { value: Number(roster.completion_rate) })} />
        <Figure value={`${sum.average}%`} label={tr("average progress")} />
        <Figure value={sum.quiet} label={tr("gone quiet")} tone={sum.quiet ? "warn" : undefined} />
        <Figure value={sum.toGrade} label={tr("to grade")} tone={sum.toGrade ? "warn" : undefined} />
      </div>
      {sum.enrolled > 0 && (
        <section aria-label={tr("Where the class is")}>
          <h2 className="lms-group">{tr("Where the class is")}</h2>
          <ul className="lms-bands">
            {bands.map((b) => (
              <li key={b.label}>
                <span>{isolate(b.label)}</span>
                <span className="lms-band-bar" aria-hidden="true">
                  <span style={{ width: `${(b.count / widest) * 100}%` }} />
                </span>
                <span className="num">{b.count}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <div className="lms-switch" role="tablist" aria-label={tr("Learners and grading")}>
        <button type="button" role="tab" aria-selected={tab === "learners"} onClick={() => go("learners")}>
          {tr("Learners")}
        </button>
        <button type="button" role="tab" aria-selected={tab === "grading"} onClick={() => go("grading")}>
          {tr("To grade")} {sum.toGrade > 0 && <Badge tone="amber">{sum.toGrade}</Badge>}
        </button>
      </div>
      {tab === "learners" ? <Learners roster={roster} now={now} /> : <Grading course={course} />}
      <AddLearners course={course} open={adding} onClose={() => setAdding(false)} onDone={refresh} />
    </div>
  );
}
