// The grading queue of a course: the work waiting for a grade, one piece open at a time, with the
// learner's answer or file, a score, feedback, and whether it passes or goes back for changes.
// Saving a grade moves to the next piece. The server decides what a score passes and tells the
// learner; this shows its answer.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { api, docPath, errorMessage, formatDateTime, relativeTime, t, isolate } from "@ferrum/client";
import { Alert, Badge, Button, Field, PageLoading, StatePanel, toast, useLiveEntity } from "@ferrum/kit";
import { Failed } from "./states";
import { getRubric, getWaitingAnswers, gradeAnswer, keys, reviewAttempt, rowsOf } from "./api";
import { aboutPoints, complete } from "./rubric";
import type { RubricDef, WaitingAttempt } from "./types";
import { Glyph } from "./Glyph";
import { Md } from "./Md";

export interface Submission {
  id: string;
  version: number;
  assignment: string;
  assignment_title: string | null;
  learner: string;
  learner_name: string | null;
  submission_no: number;
  submitted_at: string | null;
  late: boolean;
  answer: string | null;
  attachment: string | null;
  max_score: string | null;
}

interface AssignmentInfo {
  max_score: string;
  passing_score: string;
  rubric: string | null;
}

/** One radio group per criterion: its levels with their points. The choice is {criterion: level}. */
export function RubricPicker({ def, chosen, onChange, outOf }: { def: RubricDef; chosen: Record<string, string>; onChange: (c: Record<string, string>) => void; outOf: number }) {
  return (
    <fieldset className="lms-rubric stack">
      <legend>{isolate(def.title)}</legend>
      {def.criteria.map((c) => (
        <div key={c.criterion} role="radiogroup" aria-label={c.criterion} className="lms-outcome">
          <strong>{isolate(c.criterion)}</strong>
          {def.levels
            .filter((l) => l.criterion === c.criterion)
            .map((l) => (
              <label key={l.level} className="lms-option" data-chosen={chosen[c.criterion] === l.level || undefined}>
                <input type="radio" name={`rubric-${c.criterion}`} checked={chosen[c.criterion] === l.level} onChange={() => onChange({ ...chosen, [c.criterion]: l.level })} />
                <span>
                  {isolate(l.level)} <span className="faint">({Number(l.points)})</span>
                </span>
              </label>
            ))}
        </div>
      ))}
      <p className="faint">{t("About {points} of {max} points", { points: aboutPoints(def, chosen, outOf), max: outOf })}</p>
    </fieldset>
  );
}

const KEY = (course: string) => ["lms", "grading", course] as const;

function GradePanel({ s, position, count, onGraded, onSkip }: { s: Submission; position: number; count: number; onGraded: () => void; onSkip: () => void }) {
  const info = useQuery({ queryKey: ["lms", "assignment", s.assignment], queryFn: () => api.get<AssignmentInfo>(docPath("LmsAssignment", s.assignment)), staleTime: 300_000 });
  const max = Number(s.max_score ?? info.data?.max_score ?? 0);
  const pass = Number(info.data?.passing_score ?? 0);
  const [score, setScore] = useState("");
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const rubric = useQuery({ queryKey: keys.rubric(info.data?.rubric ?? ""), queryFn: () => getRubric(info.data!.rubric!), enabled: !!info.data?.rubric, staleTime: 300_000 });
  const def = rubric.data ?? null;
  const [feedback, setFeedback] = useState("");
  const [outcome, setOutcome] = useState<"Graded" | "Needs Revision">("Graded");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const n = Number(score);
  const bad = outcome === "Graded" && (def ? !complete(def, chosen) : score.trim() === "" || !Number.isFinite(n) || n < 0 || (max > 0 && n > max));
  const shownScore = def ? aboutPoints(def, chosen, max) : n;
  const verdict = outcome === "Graded" && !bad ? (shownScore >= pass ? "passes" : "does not pass") : null;

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (bad || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.post(docPath("LmsAssignmentSubmission", s.id, "actions", "grade"), {
        outcome,
        ...(def && outcome === "Graded" ? { rubric: JSON.stringify(chosen) } : score.trim() !== "" ? { score: score.trim() } : {}),
        ...(feedback.trim() ? { feedback: feedback.trim() } : {}),
      });
      toast(outcome === "Graded" ? t("Graded") : t("Sent back for changes"));
      onGraded();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <section className="lms-grade" aria-label={t("Grade this work")}>
      <header className="lms-grade-head">
        <div>
          <h3>{s.learner_name || s.learner}</h3>
          <p className="faint">
            {t("{assignment_title} · submission {submission_no} ·", { assignment_title: s.assignment_title ?? "", submission_no: s.submission_no })} {s.submitted_at ? formatDateTime(s.submitted_at) : ""}
            {s.late && (
              <>
                {isolate(" ")}
                <Badge tone="amber">{t("Late")}</Badge>
              </>
            )}
          </p>
        </div>
        <span className="faint lms-grade-pos">
          {t("{position} of {count}", { position, count })}
        </span>
      </header>
      {s.answer ? <Md source={s.answer} /> : <p className="muted">{t("No text, only a file.")}</p>}
      {s.attachment && (
        <p>
          <a className="btn btn-default btn-md" href={s.attachment} target="_blank" rel="noopener noreferrer">
            <Glyph name="doc" size={14} /> {t("Open the file")}
          </a>
        </p>
      )}
      <form className="lms-form stack" onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void submit()}>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <div className="lms-outcome" role="radiogroup" aria-label={t("What happens to this work")}>
          <label className="lms-option" data-chosen={outcome === "Graded" || undefined}>
            <input type="radio" name="outcome" checked={outcome === "Graded"} onChange={() => setOutcome("Graded")} />
            <span>{t("Grade it")}</span>
          </label>
          <label className="lms-option" data-chosen={outcome === "Needs Revision" || undefined}>
            <input type="radio" name="outcome" checked={outcome === "Needs Revision"} onChange={() => setOutcome("Needs Revision")} />
            <span>{t("Ask for changes")}</span>
          </label>
        </div>
        {def && outcome === "Graded" ? (
          <RubricPicker def={def} chosen={chosen} onChange={setChosen} outOf={max} />
        ) : (
          <Field label={(max ? t("Score out of {max}", { max }) : t("Score"))} htmlFor="lms-score" required={outcome === "Graded"} hint={verdict ? (pass ? t("This {verdict} (pass mark {pass}).", { verdict, pass }) : t("This {verdict}.", { verdict })) : undefined}>
            <input id="lms-score" className="input num lms-score-input" inputMode="decimal" value={score} onChange={(e) => setScore(e.target.value)} aria-invalid={outcome === "Graded" && score !== "" && bad ? true : undefined} />
          </Field>
        )}
        <Field label={t("Feedback")} htmlFor="lms-feedback" hint={t("The learner sees this. Markdown works.")}>
          <textarea id="lms-feedback" className="textarea" rows={5} value={feedback} onChange={(e) => setFeedback(e.target.value)} />
        </Field>
        <div className="row">
          <Button type="submit" variant="primary" loading={busy} disabled={bad}>
            {outcome === "Graded" ? t("Save grade") : t("Send back")}
            {count > 1 && t(" and go on")}
          </Button>
          {count > 1 && <Button onClick={onSkip}>{t("Skip for now")}</Button>}
        </div>
      </form>
    </section>
  );
}

/** One waiting answer of a quiz attempt: what the learner wrote or uploaded, and a grade by points or by the rubric's levels. */
function AnswerPanel({ w, question, position, count, onGraded, onSkip }: { w: WaitingAttempt; question: string; position: number; count: number; onGraded: (waitingFor?: string, mine?: boolean) => void; onSkip: () => void }) {
  const review = useQuery({ queryKey: ["lms", "review", w.attempt], queryFn: () => reviewAttempt(w.attempt), staleTime: 0 });
  const a = review.data?.answers.find((x) => x.question === question);
  const [points, setPoints] = useState("");
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (review.error) return <Failed error={review.error} onRetry={() => void review.refetch()} title={t("Could not load this answer")} />;
  if (!a) return <PageLoading label={t("Loading the answer")} />;
  const out = Number(a.points);
  const def = a.rubric;
  const n = Number(points);
  const bad = def ? !complete(def, chosen) : points.trim() === "" || !Number.isFinite(n) || n < 0 || n > out;
  const file = a.question_type === "File Upload" && typeof a.response[0] === "string" ? (a.response[0] as string) : null;
  const text = a.question_type === "Essay" && typeof a.response[0] === "string" ? (a.response[0] as string) : null;
  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (bad || busy) return;
    setBusy(true);
    setError(null);
    try {
      const r = await gradeAnswer(w.attempt, { question, ...(def ? { rubric: chosen } : { points: points.trim() }), ...(comment.trim() ? { comment: comment.trim() } : {}) });
      toast(r.waiting_for ? t("Saved. {waiting_for} grades it next", { waiting_for: r.waiting_for }) : t("Graded"));
      onGraded(r.waiting_for, true);
    } catch (err) {
      const m = errorMessage(err);
      setError(m);
      if (/graded already/.test(m)) onGraded(undefined, false);
      else if (/second grader|two graders differ/.test(m)) onGraded(m, true);
      else setBusy(false);
    }
  };
  return (
    <section className="lms-grade" aria-label={t("Grade this answer")}>
      <header className="lms-grade-head">
        <div>
          <h3>{w.learner_name ?? ""}</h3>
          <p className="faint">{isolate(w.quiz_title ?? "")} · {w.submitted_at ? formatDateTime(w.submitted_at) : ""}</p>
        </div>
        <span className="faint lms-grade-pos">{t("{position} of {count}", { position, count })}</span>
      </header>
      <Md source={a.question_text} />
      {text !== null ? <Md source={text} /> : null}
      {file && (
        <p>
          <a className="btn btn-default btn-md" href={file} target="_blank" rel="noopener noreferrer">
            <Glyph name="doc" size={14} /> {t("Open the file")}
          </a>
        </p>
      )}
      {text === null && !file && <p className="muted">{t("Nothing was answered.")}</p>}
      <form className="lms-form stack" onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void submit()}>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        {def ? (
          <RubricPicker def={def} chosen={chosen} onChange={setChosen} outOf={out} />
        ) : (
          <Field label={t("Points out of {max}", { max: out })} htmlFor="lms-points" required>
            <input id="lms-points" className="input num lms-score-input" inputMode="decimal" value={points} onChange={(e) => setPoints(e.target.value)} aria-invalid={points !== "" && bad ? true : undefined} />
          </Field>
        )}
        <Field label={t("Comment")} htmlFor="lms-answer-comment" hint={t("The learner sees this. Markdown works.")}>
          <textarea id="lms-answer-comment" className="textarea" rows={4} value={comment} onChange={(e) => setComment(e.target.value)} />
        </Field>
        <div className="row">
          <Button type="submit" variant="primary" loading={busy} disabled={bad}>
            {t("Save grade")}
          </Button>
          {count > 1 && <Button onClick={onSkip}>{t("Skip for now")}</Button>}
        </div>
      </form>
    </section>
  );
}

/** What the queue holds: a handed-in piece of work, or an answer of a quiz attempt that a person grades. */
type Item = { id: string; at: string | null; kind: "work"; s: Submission } | { id: string; at: string | null; kind: "answer"; w: WaitingAttempt; question: string; mine: boolean };

/** The items of the queue, oldest first. */
export function queueOf(works: Submission[], waiting: WaitingAttempt[], course: string): Item[] {
  const answers: Item[] = waiting
    .filter((w) => w.course.id === course)
    .flatMap((w) => w.questions.map((q) => ({ id: `${w.attempt}:${q}`, at: w.submitted_at, kind: "answer" as const, w, question: q, mine: w.graded_by_me.includes(q) })));
  const items: Item[] = [...works.map((s) => ({ id: s.id, at: s.submitted_at, kind: "work" as const, s })), ...answers];
  return items.sort((a, b) => (a.at ?? "9").localeCompare(b.at ?? "9"));
}

export function Grading({ course }: { course: string }) {
  const qc = useQueryClient();
  const [sp, setSp] = useSearchParams();
  const q = useQuery({
    queryKey: KEY(course),
    queryFn: () =>
      rowsOf<Submission>("LmsAssignmentSubmission", {
        filters: JSON.stringify({ course, status: "Submitted" }),
        fields: "assignment,assignment_title,learner,learner_name,submission_no,submitted_at,late,answer,attachment,max_score",
        sort: "submitted_at asc",
        limit: 200,
      }),
    staleTime: 0,
  });
  const waiting = useQuery({ queryKey: keys.waiting, queryFn: getWaitingAnswers, staleTime: 0, retry: false });
  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: KEY(course) });
    void qc.invalidateQueries({ queryKey: keys.waiting });
  }, [qc, course]);
  useLiveEntity("LmsAssignmentSubmission", refresh);
  const list = queueOf(q.data ?? [], waiting.data ?? [], course);
  const [mine, setMine] = useState<Set<string>>(new Set());
  const picked = sp.get("grade");
  const openable = list.filter((i) => i.kind === "work" || !(i.mine || mine.has(i.id)));
  const at = Math.max(0, list.findIndex((i) => i.id === picked));
  const current = list[at];

  // A piece of work is always open while there is one that can be graded.
  useEffect(() => {
    const target = openable.find((i) => i.id === picked) ?? openable[0];
    if (target && target.id !== picked) {
      const next = new URLSearchParams(sp);
      next.set("grade", target.id);
      setSp(next, { replace: true });
    }
  }, [openable, picked, sp, setSp]);

  const open = (id: string) => {
    const next = new URLSearchParams(sp);
    next.set("grade", id);
    setSp(next, { replace: true });
  };

  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={t("Could not load the work to grade")} />;
  if (!q.data) return <PageLoading label={t("Loading the work to grade")} />;
  if (!list.length)
    return (
      <StatePanel kind="empty" title={t("Nothing waits for a grade")}>
        {t("New work appears here as learners hand it in.")}
      </StatePanel>
    );

  const after = (openable.find((i) => i.id !== current?.id && list.indexOf(i) > at) ?? openable.find((i) => i.id !== current?.id))?.id;
  const done = () => {
    refresh();
    void qc.invalidateQueries({ queryKey: ["lms", "roster"] });
    void qc.invalidateQueries({ queryKey: ["lms", "teaching"] });
    if (after) open(after);
  };
  const graded = () => {
    qc.setQueryData<Submission[]>(KEY(course), (cur) => cur?.filter((s) => s.id !== current.id));
    done();
  };
  const shown = (i: Item) => i.kind === "work" ? (i.s.learner_name || i.s.learner) : (i.w.learner_name ?? "");
  return (
    <div className="lms-grading">
      <ul className="lms-queue" aria-label={t("Work to grade")}>
        {list.map((i) => {
          const waitingForOther = i.kind === "answer" && (i.mine || mine.has(i.id));
          return (
            <li key={i.id}>
              <button type="button" className="lms-queue-item" disabled={waitingForOther} aria-current={i.id === current?.id ? "true" : undefined} onClick={() => open(i.id)}>
                <strong>{shown(i)}</strong>
                <span className="faint">{isolate(i.kind === "work" ? (i.s.assignment_title ?? "") : (i.w.quiz_title ?? ""))}</span>
                <span className="faint">
                  {i.at ? relativeTime(i.at) : ""}
                  {i.kind === "work" && i.s.late ? t(" · late") : ""}
                  {i.kind === "answer" ? ` · ${t("Answer")}` : ""}
                </span>
                {waitingForOther && <span className="faint">{t("Graded by you: waiting for another grader")}</span>}
              </button>
            </li>
          );
        })}
      </ul>
      {current?.kind === "answer" ? (
        <AnswerPanel
          key={current.id}
          w={current.w}
          question={current.question}
          position={at + 1}
          count={list.length}
          onGraded={(waitingFor, byMe) => {
            if (waitingFor && byMe) setMine((m) => new Set(m).add(current.id));
            done();
          }}
          onSkip={() => after && open(after)}
        />
      ) : current ? (
        <GradePanel key={current.id} s={current.s} position={at + 1} count={list.length} onGraded={graded} onSkip={() => after && open(after)} />
      ) : null}
    </div>
  );
}

/** How many answers of this course wait for the reader, who has not graded them. */
export function answersWaiting(waiting: WaitingAttempt[] | undefined, course: string): number {
  return (waiting ?? []).filter((w) => w.course.id === course).reduce((n, w) => n + w.questions.filter((q) => !w.graded_by_me.includes(q)).length, 0);
}
