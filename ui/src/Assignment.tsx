// An assignment lesson: what is asked, when it is due, handing work in (text, a file, or both),
// changing it until it is graded, and reading the grade and feedback. The server decides whether
// work may be handed in and says why when it may not; this shows its reason.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { api, ApiError, docPath, errorMessage, formatDate, formatDateTime, todayIso, uploadFile, t, isolate } from "@ferrum/client";
import { Alert, Badge, Button, Field, Spinner, toast } from "@ferrum/kit";
import { keys } from "./api";
import { dueLabel } from "./format";
import { Md } from "./Md";
import { Glyph } from "./Glyph";
import type { AssignmentState, SubmissionRow } from "./types";

interface SubmissionDoc {
  id: string;
  version: number;
  answer: string | null;
  attachment: string | null;
}

/** A draft kept on this device while the learner writes, so a closed tab does not lose an essay. */
function useDraft(key: string): [string, (v: string) => void, () => void] {
  const read = () => {
    try {
      return localStorage.getItem(key) ?? "";
    } catch {
      return "";
    }
  };
  const [text, setText] = useState(read);
  const set = (v: string) => {
    setText(v);
    try {
      if (v) localStorage.setItem(key, v);
      else localStorage.removeItem(key);
    } catch {
      /* a private window keeps no drafts */
    }
  };
  return [text, set, () => set("")];
}

const fileName = (path: string) => (path.startsWith("/files/") ? t("Attached file") : path);

function Form({ state, editing, onDone, onCancel }: { state: AssignmentState; editing: SubmissionRow | null; onDone: () => void; onCancel?: () => void }) {
  const wantsText = state.submission_type !== "File";
  const wantsFile = state.submission_type !== "Text";
  const doc = useQuery({
    queryKey: ["lms", "submission", editing?.id],
    enabled: !!editing,
    queryFn: () => api.get<SubmissionDoc>(docPath("LmsAssignmentSubmission", editing!.id)),
    staleTime: 0,
  });
  const [draft, setDraft, clearDraft] = useDraft(`lms.draft.${state.id}`);
  const [edited, setEdited] = useState<string | null>(null);
  const [attachment, setAttachment] = useState<string | null | undefined>(undefined);
  const [uploading, setUploading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError | string | null>(null);
  const pick = useRef<HTMLInputElement>(null);

  // What is in the box: the draft for new work, the stored text when changing handed-in work.
  const text = editing ? (edited ?? doc.data?.answer ?? "") : draft;
  const file = attachment === undefined ? (doc.data?.attachment ?? null) : attachment;
  const setText = (v: string) => (editing ? setEdited(v) : setDraft(v));
  const empty = (!wantsText || !text.trim()) && (!wantsFile || !file);

  const choose = async (f: File | undefined) => {
    if (!f) return;
    setUploading(true);
    setError(null);
    try {
      setAttachment((await uploadFile(f)).url);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setUploading(false);
      if (pick.current) pick.current.value = "";
    }
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const body = { ...(wantsText ? { answer: text.trim() || null } : {}), ...(wantsFile ? { attachment: file } : {}) };
    try {
      if (editing) await api.patch(docPath("LmsAssignmentSubmission", editing.id), body, { ifMatch: doc.data?.version });
      else await api.post(docPath("LmsAssignmentSubmission"), { assignment: state.id, ...body });
      clearDraft();
      toast(editing ? t("Changes saved") : t("Handed in"));
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err : errorMessage(err));
      setBusy(false);
    }
  };

  const fieldError = (name: string) => (error instanceof ApiError ? error.fields.find((f) => f.field === name)?.message : undefined);
  const general = error instanceof ApiError ? (error.fields.length ? null : error.message) : error;

  if (editing && doc.isLoading) return <Spinner label={t("Opening your work")} />;
  return (
    <form className="lms-form stack" onSubmit={(e) => void submit(e)}>
      {general && <Alert tone="error">{isolate(general)}</Alert>}
      {wantsText && (
        <Field label={t("Your answer")} htmlFor="lms-answer" error={fieldError("answer")} hint={editing ? undefined : t("Saved on this device as you write. Markdown works.")}>
          <textarea id="lms-answer" className="textarea lms-essay" rows={9} value={text} onChange={(e) => setText(e.target.value)} />
        </Field>
      )}
      {wantsFile && (
        <Field label={t("File")} htmlFor="lms-file" error={fieldError("attachment")}>
          <div className="row">
            <label className="btn btn-default btn-md" aria-busy={uploading || undefined}>
              <input id="lms-file" ref={pick} className="sr-only" type="file" onChange={(e) => void choose(e.target.files?.[0])} />
              {uploading ? <Spinner small /> : <Glyph name="upload" />}
              {file ? t("Replace file") : t("Choose a file")}
            </label>
            {file && (
              <>
                <a href={file} target="_blank" rel="noopener noreferrer">
                  {isolate(fileName(file))}
                </a>
                <button type="button" className="lms-linklike" onClick={() => setAttachment(null)}>
                  {t("Remove")}
                </button>
              </>
            )}
          </div>
        </Field>
      )}
      <div className="row">
        <Button type="submit" variant="primary" loading={busy} disabled={empty || uploading}>
          {editing ? t("Save changes") : t("Hand in")}
        </Button>
        {onCancel && <Button onClick={onCancel}>{t("Cancel")}</Button>}
      </div>
    </form>
  );
}

function Standing({ s, state }: { s: SubmissionRow; state: AssignmentState }) {
  const when = s.submitted_at ? formatDateTime(s.submitted_at) : "";
  const score = `${Number(s.score ?? 0)} of ${Number(state.max_score)}`;
  const feedback = s.feedback ? (
    <div className="lms-feedback">
      <strong>{t("Feedback")}</strong>
      <Md source={s.feedback} />
    </div>
  ) : null;
  if (s.status === "Submitted")
    return <Alert tone="info">{t("Handed in {when}", { when })}{s.late ? t(" (late)") : ""}{t(". Your instructor will grade it; you can change it until then.")}</Alert>;
  if (s.status === "Needs Revision")
    return (
      <Alert tone="warning">
        <strong>{t("Your instructor asked for changes.")}</strong> {t("Hand in a new version below.")}
        {feedback}
      </Alert>
    );
  return (
    <Alert tone={s.passed ? "success" : "warning"}>
      <strong>{s.passed ? t("Passed: {score}", { score }) : t("Not passed: {score}, the pass mark is {value}", { score, value: Number(state.passing_score) })}</strong>
      {feedback}
    </Alert>
  );
}

/** The assignment of a lesson. `onProgress` tells the page a grade may have finished the lesson. */
export function AssignmentPanel({ state, lessonId, onProgress }: { state: AssignmentState; lessonId: string; onProgress: () => void }) {
  const qc = useQueryClient();
  const latest = state.submissions.at(-1) ?? null;
  const pending = latest?.status === "Submitted" ? latest : null;
  const [changing, setChanging] = useState(false);
  const refresh = () => {
    setChanging(false);
    void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
    void qc.invalidateQueries({ queryKey: keys.dashboard });
    onProgress();
  };
  // A grade that arrives while the form is open shuts it.
  useEffect(() => {
    if (!pending) setChanging(false);
  }, [pending]);

  const today = todayIso();
  const due = dueLabel(state.due_date, today);
  const earlier = state.submissions.slice(0, -1).reverse();
  const open = state.can_submit && !pending;

  return (
    <section className="lms-assignment" aria-label={t("Assignment")}>
      <div className="lms-quiz-head">
        <h2>{isolate(state.title)}</h2>
        {state.passed && <Badge tone="green">{t("Passed")}</Badge>}
      </div>
      <Md source={state.instructions} />
      <dl className="lms-facts">
        {state.due_date && (
          <div>
            <dt>{t("Due")}</dt>
            <dd className={state.overdue ? "lms-late" : undefined}>
              {isolate(formatDate(state.due_date))} <span className="faint">· {isolate(due)}</span>
            </dd>
          </div>
        )}
        <div>
          <dt>{t("Marked out of")}</dt>
          <dd>{Number(state.max_score)}</dd>
        </div>
        <div>
          <dt>{t("Pass mark")}</dt>
          <dd>{Number(state.passing_score) || t("Any grade passes")}</dd>
        </div>
        <div>
          <dt>{t("Hand in")}</dt>
          <dd>{state.submission_type === "Text" ? t("Text") : state.submission_type === "File" ? t("A file") : t("Text, a file, or both")}</dd>
        </div>
        {state.due_date && (
          <div>
            <dt>{t("Late work")}</dt>
            <dd>{state.accept_late ? t("Accepted") : t("Not accepted")}</dd>
          </div>
        )}
      </dl>
      {latest && <Standing s={latest} state={state} />}
      {pending && !changing && (
        <div className="row">
          <Button onClick={() => setChanging(true)}>{t("Change my work")}</Button>
        </div>
      )}
      {pending && changing && <Form state={state} editing={pending} onDone={refresh} onCancel={() => setChanging(false)} />}
      {open && <Form key={latest?.id ?? "new"} state={state} editing={null} onDone={refresh} />}
      {!open && !pending && state.reason && !state.passed && <Alert tone="info">{state.reason.charAt(0).toUpperCase() + state.reason.slice(1)}.</Alert>}
      {earlier.length > 0 && (
        <details className="lms-earlier">
          <summary>{t("Earlier submissions ({length})", { length: earlier.length })}</summary>
          <ul className="lms-history">
            {earlier.map((s) => (
              <li key={s.id}>
                <span className="lms-history-no">{t("Submission {submission_no}", { submission_no: s.submission_no })}</span>
                <span className="faint">{s.submitted_at ? formatDateTime(s.submitted_at) : ""}</span>
                <Badge tone={s.passed ? "green" : s.status === "Needs Revision" ? "amber" : "red"}>{s.status === "Graded" ? (s.passed ? t("Passed") : t("Not passed")) : s.status}</Badge>
                {s.score !== null && <span>{t("{value} of {value2}", { value: Number(s.score), value2: Number(state.max_score) })}</span>}
                {s.feedback && <div className="lms-feedback lms-feedback-wide"><Md source={s.feedback} /></div>}
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
