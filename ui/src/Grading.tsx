// The grading queue of a course: the work waiting for a grade, one piece open at a time, with the
// learner's answer or file, a score, feedback, and whether it passes or goes back for changes.
// Saving a grade moves to the next piece. The server decides what a score passes and tells the
// learner; this shows its answer.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { api, docPath, errorMessage, formatDateTime, relativeTime } from "@ferrum/client";
import { Alert, Badge, Button, EmptyState, ErrorState, Field, PageLoading, toast, useLiveEntity } from "@ferrum/kit";
import { rowsOf } from "./api";
import { Glyph } from "./Glyph";
import { Md } from "./Md";

interface Submission {
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
}

const KEY = (course: string) => ["lms", "grading", course] as const;

function GradePanel({ s, position, count, onGraded, onSkip }: { s: Submission; position: number; count: number; onGraded: () => void; onSkip: () => void }) {
  const info = useQuery({ queryKey: ["lms", "assignment", s.assignment], queryFn: () => api.get<AssignmentInfo>(docPath("LmsAssignment", s.assignment)), staleTime: 300_000 });
  const max = Number(s.max_score ?? info.data?.max_score ?? 0);
  const pass = Number(info.data?.passing_score ?? 0);
  const [score, setScore] = useState("");
  const [feedback, setFeedback] = useState("");
  const [outcome, setOutcome] = useState<"Graded" | "Needs Revision">("Graded");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const n = Number(score);
  const bad = outcome === "Graded" && (score.trim() === "" || !Number.isFinite(n) || n < 0 || (max > 0 && n > max));
  const verdict = outcome === "Graded" && !bad ? (n >= pass ? "passes" : "does not pass") : null;

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (bad || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.post(docPath("LmsAssignmentSubmission", s.id, "actions", "grade"), {
        outcome,
        ...(score.trim() !== "" ? { score: score.trim() } : {}),
        ...(feedback.trim() ? { feedback: feedback.trim() } : {}),
      });
      toast(outcome === "Graded" ? "Graded" : "Sent back for changes");
      onGraded();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <section className="lms-grade" aria-label="Grade this work">
      <header className="lms-grade-head">
        <div>
          <h3>{s.learner_name || s.learner}</h3>
          <p className="faint">
            {s.assignment_title} · submission {s.submission_no} · {s.submitted_at ? formatDateTime(s.submitted_at) : ""}
            {s.late && (
              <>
                {" "}
                <Badge tone="amber">Late</Badge>
              </>
            )}
          </p>
        </div>
        <span className="faint lms-grade-pos">
          {position} of {count}
        </span>
      </header>
      {s.answer ? <Md source={s.answer} /> : <p className="muted">No text, only a file.</p>}
      {s.attachment && (
        <p>
          <a className="btn btn-default btn-md" href={s.attachment} target="_blank" rel="noopener noreferrer">
            <Glyph name="doc" size={14} /> Open the file
          </a>
        </p>
      )}
      <form className="lms-form stack" onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void submit()}>
        {error && <Alert tone="error">{error}</Alert>}
        <div className="lms-outcome" role="radiogroup" aria-label="What happens to this work">
          <label className="lms-option" data-chosen={outcome === "Graded" || undefined}>
            <input type="radio" name="outcome" checked={outcome === "Graded"} onChange={() => setOutcome("Graded")} />
            <span>Grade it</span>
          </label>
          <label className="lms-option" data-chosen={outcome === "Needs Revision" || undefined}>
            <input type="radio" name="outcome" checked={outcome === "Needs Revision"} onChange={() => setOutcome("Needs Revision")} />
            <span>Ask for changes</span>
          </label>
        </div>
        <Field label={`Score${max ? ` out of ${max}` : ""}`} htmlFor="lms-score" required={outcome === "Graded"} hint={verdict ? `This ${verdict}${pass ? ` (pass mark ${pass})` : ""}.` : undefined}>
          <input id="lms-score" className="input num lms-score-input" inputMode="decimal" value={score} onChange={(e) => setScore(e.target.value)} aria-invalid={outcome === "Graded" && score !== "" && bad ? true : undefined} />
        </Field>
        <Field label="Feedback" htmlFor="lms-feedback" hint="The learner sees this. Markdown works.">
          <textarea id="lms-feedback" className="textarea" rows={5} value={feedback} onChange={(e) => setFeedback(e.target.value)} />
        </Field>
        <div className="row">
          <Button type="submit" variant="primary" loading={busy} disabled={bad}>
            {outcome === "Graded" ? "Save grade" : "Send back"}
            {count > 1 && " and go on"}
          </Button>
          {count > 1 && <Button onClick={onSkip}>Skip for now</Button>}
        </div>
      </form>
    </section>
  );
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
  useLiveEntity("LmsAssignmentSubmission", useCallback(() => void qc.invalidateQueries({ queryKey: KEY(course) }), [qc, course]));
  const list = q.data ?? [];
  const picked = sp.get("grade");
  const at = Math.max(0, list.findIndex((s) => s.id === picked));
  const current = list[at];

  // A piece of work is always open while there is one.
  useEffect(() => {
    if (current && current.id !== picked) {
      const next = new URLSearchParams(sp);
      next.set("grade", current.id);
      setSp(next, { replace: true });
    }
  }, [current, picked, sp, setSp]);

  const open = (id: string) => {
    const next = new URLSearchParams(sp);
    next.set("grade", id);
    setSp(next, { replace: true });
  };

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not load the work to grade" />;
  if (!q.data) return <PageLoading label="Loading the work to grade" />;
  if (!list.length)
    return (
      <EmptyState title="Nothing waits for a grade" icon="check">
        <p className="muted">New work appears here as learners hand it in.</p>
      </EmptyState>
    );

  const after = (list[at + 1] ?? list[at - 1])?.id;
  const graded = () => {
    qc.setQueryData<Submission[]>(KEY(course), (cur) => cur?.filter((s) => s.id !== current.id));
    void qc.invalidateQueries({ queryKey: ["lms", "roster"] });
    void qc.invalidateQueries({ queryKey: ["lms", "teaching"] });
    if (after) open(after);
  };

  return (
    <div className="lms-grading">
      <ul className="lms-queue" aria-label="Work to grade">
        {list.map((s) => (
          <li key={s.id}>
            <button type="button" className="lms-queue-item" aria-current={s.id === current.id ? "true" : undefined} onClick={() => open(s.id)}>
              <strong>{s.learner_name || s.learner}</strong>
              <span className="faint">{s.assignment_title}</span>
              <span className="faint">
                {s.submitted_at ? relativeTime(s.submitted_at) : ""}
                {s.late ? " · late" : ""}
              </span>
            </button>
          </li>
        ))}
      </ul>
      <GradePanel key={current.id} s={current} position={at + 1} count={list.length} onGraded={graded} onSkip={() => after && open(after)} />
    </div>
  );
}
