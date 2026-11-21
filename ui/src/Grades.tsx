// The gradebook. An instructor reads every learner against every graded item, opens a cell to mark
// it by hand (or hand it back to what Learn works out), and downloads the book; a learner reads their
// own released marks and total. Every figure is the server's: this arranges it and never adds up.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useParams } from "react-router";
import { ApiError, errorMessage, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, EmptyState, Field, PageHead, PageLoading, PageStack, Spinner, toast, useShellTitle } from "@ferrum/kit";
import { clearGradeEntry, getGradebook, getGradeEntry, getMyGrades, gradebookCsv, keys, saveGradeEntry } from "./api";
import { cellText, markOf } from "./gradeview";
import { paths } from "./lesson";
import { percentLabel } from "./outline";
import { Failed } from "./states";
import type { GradeBook, GradeEntry, GradeItem, GradeLearner } from "./types";

// ───────────────────────────── the instructor's book ─────────────────────────────

function Cell({ learner, item, onOpen }: { learner: GradeLearner; item: GradeItem; onOpen: () => void }) {
  const mark = markOf(learner, item.id);
  const shown = cellText(mark);
  const name = learner.learner_name || learner.learner;
  return (
    <td className="num">
      <button type="button" className="btn btn-ghost btn-sm" aria-label={tr("Mark for {learner}: {item}", { learner: name, item: item.title })} onClick={onOpen}>
        {shown.kind === "excused" ? tr("Excused") : shown.kind === "points" ? isolate(shown.points) : <span className="faint">–</span>}
        {mark?.overridden && <span className="faint" title={tr("Marked by hand")}> ●</span>}
        {mark && Number(mark.late_penalty_percent) > 0 && <span className="faint"> −{Number(mark.late_penalty_percent)}%</span>}
      </button>
    </td>
  );
}

export function GradebookTab({ course }: { course: string }) {
  const q = useQuery({ queryKey: keys.gradebook(course), queryFn: () => getGradebook(course), staleTime: 0 });
  const [open, setOpen] = useState<{ learner: GradeLearner; item: GradeItem } | null>(null);
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load the gradebook")} />;
  const book = q.data;
  if (!book) return <PageLoading label={tr("Loading the gradebook")} />;
  if (!book.items.length) {
    return (
      <EmptyState title={tr("Nothing to grade yet")}>
        <p>{tr("Add the items that count towards the grade, and the categories they belong to on the course.")}</p>
        <div className="row">
          <Link className="btn btn-primary btn-md" to={paths.record("LmsGradeItem", "new")}>
            {tr("Add a grade item")}
          </Link>
          <Link className="btn btn-default btn-md" to={paths.record("LmsCourse", book.course.id)}>
            {tr("Course categories")}
          </Link>
        </div>
      </EmptyState>
    );
  }
  return (
    <section aria-label={tr("Gradebook")}>
      <div className="row">
        <h2 className="lms-group">{tr("Gradebook")}</h2>
        <span className="spacer" />
        <a className="btn btn-default btn-md" href={gradebookCsv(course)} download>
          {tr("Download CSV")}
        </a>
      </div>
      {!book.learners.length ? (
        <EmptyState title={tr("No learners yet")} />
      ) : (
        <div className="table-wrap">
          <table className="table">
            <caption className="sr-only">{tr("Every learner's marks, item by item")}</caption>
            <thead>
              <tr>
                <th scope="col">{tr("Learner")}</th>
                {book.items.map((i) => (
                  <th key={i.id} scope="col" className="num">
                    {isolate(i.title)}
                    <div className="faint">{isolate(i.category)} · {tr("out of {n}", { n: Number(i.max_points) })}</div>
                    {!i.released && <Badge tone="amber">{tr("Not released")}</Badge>}
                  </th>
                ))}
                <th scope="col" className="num">{tr("Total")}</th>
                <th scope="col">{tr("Grade")}</th>
              </tr>
            </thead>
            <tbody>
              {book.learners.map((l) => (
                <tr key={l.enrollment}>
                  <th scope="row">
                    {isolate(l.learner_name || l.learner)}
                    {l.learner_name && <div className="faint">{isolate(l.learner)}</div>}
                  </th>
                  {book.items.map((i) => (
                    <Cell key={i.id} learner={l} item={i} onOpen={() => setOpen({ learner: l, item: i })} />
                  ))}
                  <td className="num">{l.total_percent === null ? <span className="faint">–</span> : isolate(percentLabel(l.total_percent))}</td>
                  <td>{l.grade ? <Badge tone="blue">{isolate(l.grade)}</Badge> : <span className="faint">–</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {open && <MarkDialog course={course} learner={open.learner} item={open.item} onClose={() => setOpen(null)} />}
    </section>
  );
}

/** The mark of one cell: read before it is written, so a first mark is a POST and a change is a PATCH. */
function MarkDialog({ course, learner, item, onClose }: { course: string; learner: GradeLearner; item: GradeItem; onClose: () => void }) {
  const entry = useQuery({ queryKey: ["lms", "gradeentry", item.id, learner.enrollment], queryFn: () => getGradeEntry(item.id, learner.enrollment), staleTime: 0, gcTime: 0 });
  const name = learner.learner_name || learner.learner;
  return (
    <Dialog open onClose={onClose} title={tr("Mark for {learner}: {item}", { learner: name, item: item.title })}>
      {entry.error ? (
        <Failed error={entry.error} onRetry={() => void entry.refetch()} />
      ) : entry.isLoading ? (
        <Spinner label={tr("Loading the mark")} />
      ) : (
        <MarkForm key={entry.data?.id ?? "new"} course={course} learner={learner} item={item} entry={entry.data ?? null} onClose={onClose} reread={() => void entry.refetch()} />
      )}
    </Dialog>
  );
}

function MarkForm({ course, learner, item, entry, onClose, reread }: { course: string; learner: GradeLearner; item: GradeItem; entry: GradeEntry | null; onClose: () => void; reread: () => void }) {
  const qc = useQueryClient();
  const [points, setPoints] = useState(entry?.points ?? "");
  const [excused, setExcused] = useState(entry?.excused ?? false);
  const [comment, setComment] = useState(entry?.comment ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fields, setFields] = useState<Record<string, string>>({});
  const done = (message: string) => {
    void qc.invalidateQueries({ queryKey: keys.gradebook(course) });
    void qc.invalidateQueries({ queryKey: ["lms", "roster"] });
    toast(message);
    onClose();
  };
  const fail = (e: unknown) => {
    if (e instanceof ApiError && e.status === 409) {
      setError(tr("Someone else changed this mark. It was read again; check it and save once more."));
      reread();
    } else if (e instanceof ApiError && e.fields.length) {
      setFields(Object.fromEntries(e.fields.map((f) => [f.field, f.message])));
    } else setError(errorMessage(e));
  };
  const save = async (ev: FormEvent) => {
    ev.preventDefault();
    setBusy(true);
    setError(null);
    setFields({});
    try {
      await saveGradeEntry(item.id, learner.enrollment, { points: excused || points.trim() === "" ? null : points.trim(), excused, comment: comment.trim() || null }, entry);
      done(tr("Mark saved"));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const clear = async () => {
    if (!entry) return;
    const ok = await confirmDialog({ title: tr("Use what Learn works out?"), message: tr("Your mark is removed and the item counts as Learn works it out."), confirmLabel: tr("Use what Learn works out") });
    if (!ok) return;
    setBusy(true);
    try {
      await clearGradeEntry(entry);
      done(tr("Mark removed"));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="stack" onSubmit={(e) => void save(e)}>
      {error && <Alert tone="error">{isolate(error)}</Alert>}
      <Field label={tr("Points (out of {n})", { n: Number(item.max_points) })} htmlFor="lms-mark-points" error={fields.points}>
        <input id="lms-mark-points" className="input" inputMode="decimal" disabled={excused} value={points} onChange={(e) => setPoints(e.target.value)} />
      </Field>
      <label className="row">
        <input type="checkbox" checked={excused} onChange={(e) => setExcused(e.target.checked)} />
        <span>{tr("Excused")}</span>
      </label>
      <Field label={tr("Comment")} htmlFor="lms-mark-comment" error={fields.comment}>
        <textarea id="lms-mark-comment" className="textarea" rows={4} value={comment} onChange={(e) => setComment(e.target.value)} />
      </Field>
      <div className="row" style={{ justifyContent: "flex-end" }}>
        {entry && (
          <Button variant="ghost" disabled={busy} onClick={() => void clear()}>
            {tr("Use what Learn works out")}
          </Button>
        )}
        <Button variant="ghost" onClick={onClose}>
          {tr("Cancel")}
        </Button>
        <Button variant="primary" type="submit" loading={busy}>
          {tr("Save")}
        </Button>
      </div>
    </form>
  );
}

// ───────────────────────────── the learner's own marks ─────────────────────────────

/** What the server's 404 means here: the caller has no enrolment in the course. */
export function NotEnrolled({ slug }: { slug: string }) {
  return (
    <EmptyState title={tr("You are not enrolled in this course")}>
      <Link to={paths.course(slug)}>{tr("Go to the course page")}</Link>
    </EmptyState>
  );
}

export function MyGrades({ slug }: { slug: string }) {
  const q = useQuery({ queryKey: keys.grades(slug), queryFn: () => getMyGrades(slug), staleTime: 0, retry: false });
  useShellTitle(q.data ? tr("{title} · Grades", { title: q.data.course.title }) : null);
  if (q.error instanceof ApiError && q.error.status === 404) return <NotEnrolled slug={slug} />;
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load your grades")} />;
  const book: GradeBook | undefined = q.data;
  if (!book) return <PageLoading label={tr("Loading your grades")} />;
  const me = book.learners[0];
  const rows = book.items.filter((i) => i.released);
  return (
    <PageStack className="lms-page">
      <PageHead back={{ to: paths.course(slug), label: tr("Back to the course") }} title={<>{isolate(book.course.title)}</>} description={tr("Your grades")} />
      {!me || !rows.length ? (
        <EmptyState title={tr("Nothing has been graded yet")} />
      ) : (
        <>
          <ul className="lms-plain">
            {rows.map((i) => {
              const m = markOf(me, i.id);
              const shown = cellText(m);
              return (
                <li key={i.id}>
                  <strong>{isolate(i.title)}</strong>{" "}
                  <span className="muted">
                    {shown.kind === "excused" ? tr("Excused") : shown.kind === "points" ? tr("{points} of {max}", { points: Number(shown.points), max: Number(i.max_points) }) : tr("Not graded yet")}
                  </span>
                  {m && Number(m.late_penalty_percent) > 0 && <span className="faint"> · {tr("Late: −{n}%", { n: Number(m.late_penalty_percent) })}</span>}
                  {m?.comment && <p className="faint">{isolate(m.comment)}</p>}
                </li>
              );
            })}
          </ul>
          <p>
            <strong>{tr("Total")}</strong> {me.total_percent === null ? tr("Not graded yet") : isolate(percentLabel(me.total_percent))}
            {me.grade && <> · <Badge tone="blue">{isolate(me.grade)}</Badge></>}
          </p>
          {book.course.min_grade_percent && <p className="faint">{tr("The course asks for a total of {n}.", { n: percentLabel(book.course.min_grade_percent) })}</p>}
        </>
      )}
    </PageStack>
  );
}

export function MyGradesRoute() {
  const { slug = "" } = useParams();
  return <MyGrades key={slug} slug={slug} />;
}
