// The quiz player. A learner reads the rules, starts a sitting, answers one question at a time
// (every change is saved a moment later, so a closed tab loses nothing), submits, and reads the
// result. The clock is the server's: it says how many seconds are left and the page counts down
// from that. The server holds the answer keys and decides every score; this shows what it says.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ApiError, errorMessage, formatDateTime, t as tr, isolate, uploadFile } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, EmptyState, Spinner, toast, placed } from "@ferrum/kit";
import { getAttempt, getAttempts, keys, saveAnswers, startQuiz, submitAnswers } from "./api";
import { clock, duration } from "./format";
import { Glyph } from "./Glyph";
import { Md } from "./Md";
import { RubricTable } from "./RubricTable";
import { percentLabel } from "./outline";
import {
  type Answers,
  answersOf,
  attemptsText,
  canStart,
  choose,
  chosen,
  isAnswered,
  correctText,
  filled,
  isMultiple,
  kind,
  matched,
  moved,
  milestone,
  milestoneText,
  reading,
  remaining,
  responseText,
  revealed,
  rightCount,
  timerTone,
  unanswered,
  verdict,
} from "./quiz";
import { createSaver, type SaveState, type Saver } from "./saver";
import type { Attempt, AttemptAnswer, AttemptRow, QuizState, Response, SubmitResult } from "./types";

// ───────────────────────────── one question ─────────────────────────────

function QuestionCard({ q, index, count, response, onChange, disabled }: { q: AttemptAnswer; index: number; count: number; response: Response | undefined; onChange: (r: Response) => void; disabled: boolean }) {
  const text = useId();
  const multi = isMultiple(q.question_type);
  const k = kind(q.question_type);
  const points = Number(q.points);
  return (
    <div className="lms-question">
      <p className="lms-q-meta faint">
        {placed(tr("Question {p1} of {count} · {points}{p2}"), { p1: index + 1, count: count, points: (points === 1 ? tr("1 point") : tr("{n} points", { n: points })), p2: multi ? tr(" · Choose every answer that applies") : "" })}
        {(k === "essay" || k === "file") && <> · {tr("A person grades this answer")}</>}
      </p>
      <div id={text} className="lms-q-text">
        <Md source={q.question_text} />
      </div>
      {k === "choice" && (
        <div className="lms-options" role={multi ? "group" : "radiogroup"} aria-labelledby={text}>
          {(q.options ?? []).map((o) => {
            const on = chosen(response, o.key);
            return (
              <label key={o.key} className="lms-option" data-chosen={on || undefined}>
                <input type={multi ? "checkbox" : "radio"} name={q.question} checked={on} disabled={disabled} onChange={() => onChange(choose(q, response, o.key))} />
                <span>{isolate(o.text)}</span>
              </label>
            );
          })}
          {isAnswered(response) && !disabled && (
            <button type="button" className="lms-linklike" onClick={() => onChange(null)}>
              {tr("Clear my answer")}
            </button>
          )}
        </div>
      )}
      {k === "short" && (
        <textarea className="textarea lms-short" rows={3} maxLength={500} aria-labelledby={text} placeholder={tr("Type your answer")} disabled={disabled} value={typeof response === "string" ? response : ""} onChange={(e) => onChange(e.target.value)} />
      )}
      {k === "numeric" && (
        <input
          className="input lms-numeric"
          inputMode="decimal"
          aria-labelledby={text}
          placeholder={tr("Type a number")}
          disabled={disabled}
          value={typeof response === "string" ? response : ""}
          onChange={(e) => onChange(e.target.value.trim() === "" ? null : e.target.value.trim())}
        />
      )}
      {k === "ordering" && <Ordering q={q} labelledBy={text} response={response} onChange={onChange} disabled={disabled} />}
      {k === "matching" && (
        <ul className="lms-matching" aria-labelledby={text}>
          {(q.options ?? []).map((o) => {
            const now = response && typeof response === "object" && !Array.isArray(response) ? (response[o.key] ?? "") : "";
            return (
              <li key={o.key} className="row">
                <span id={`${text}-${o.key}`}>{isolate(o.text)}</span>
                <select className="select" aria-labelledby={`${text}-${o.key}`} disabled={disabled} value={now} onChange={(e) => onChange(matched(response, o.key, e.target.value || null))}>
                  <option value="" />
                  {(q.choices ?? []).map((c) => (
                    <option key={c.key} value={c.key}>
                      {c.text}
                    </option>
                  ))}
                </select>
              </li>
            );
          })}
        </ul>
      )}
      {k === "blanks" && (
        <div className="lms-blanks stack" role="group" aria-labelledby={text}>
          {Array.from({ length: q.blanks ?? 0 }, (_, i) => (
            <label key={i} className="lms-blank">
              <span className="faint">{tr("Blank {n}", { n: i + 1 })}</span>
              <input className="input" maxLength={500} disabled={disabled} value={Array.isArray(response) ? (response[i] ?? "") : ""} onChange={(e) => onChange(filled(response, q.blanks ?? 0, i, e.target.value))} />
            </label>
          ))}
        </div>
      )}
      {k === "essay" && (
        <textarea className="textarea lms-essay" rows={10} maxLength={50000} aria-labelledby={text} placeholder={tr("Write your answer")} disabled={disabled} value={typeof response === "string" ? response : ""} onChange={(e) => onChange(e.target.value.trim() === "" ? null : e.target.value)} />
      )}
      {k === "file" && <FileAnswer response={response} onChange={onChange} disabled={disabled} />}
    </div>
  );
}

/** The options in the learner's order, each with Move up and Move down. Until the order is touched it is not an answer. */
function Ordering({ q, labelledBy, response, onChange, disabled }: { q: AttemptAnswer; labelledBy: string; response: Response | undefined; onChange: (r: Response) => void; disabled: boolean }) {
  const shown = q.options ?? [];
  const keys = Array.isArray(response) && response.length === shown.length ? (response as string[]) : shown.map((o) => o.key);
  const text = (key: string) => shown.find((o) => o.key === key)?.text ?? "";
  return (
    <>
      {!isAnswered(response) && <p className="faint">{tr("The order shown is not an answer yet. Move an item to give one.")}</p>}
      <ol className="lms-ordering" aria-labelledby={labelledBy}>
        {keys.map((key, i) => (
          <li key={key} className="row">
            <span>{isolate(text(key))}</span>
            <span className="spacer" />
            <button type="button" className="btn btn-ghost btn-sm" disabled={disabled || i === 0} onClick={() => onChange(moved(keys, i, i - 1))}>
              {tr("Move up")}
              <span className="sr-only"> {isolate(text(key))}</span>
            </button>
            <button type="button" className="btn btn-ghost btn-sm" disabled={disabled || i === keys.length - 1} onClick={() => onChange(moved(keys, i, i + 1))}>
              {tr("Move down")}
              <span className="sr-only"> {isolate(text(key))}</span>
            </button>
          </li>
        ))}
      </ol>
    </>
  );
}

/** A file answer: uploaded as an assignment's file is; a failed upload leaves the earlier answer. */
function FileAnswer({ response, onChange, disabled }: { response: Response | undefined; onChange: (r: Response) => void; disabled: boolean }) {
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pick = useRef<HTMLInputElement>(null);
  const file = typeof response === "string" && response ? response : null;
  const choosePick = async (f: File | undefined) => {
    if (!f) return;
    setUploading(true);
    setError(null);
    try {
      onChange((await uploadFile(f)).url);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setUploading(false);
      if (pick.current) pick.current.value = "";
    }
  };
  return (
    <div className="stack">
      <div className="row">
        <label className="btn btn-default btn-md" aria-busy={uploading || undefined}>
          <input ref={pick} className="sr-only" type="file" disabled={disabled || uploading} onChange={(e) => void choosePick(e.target.files?.[0])} />
          {uploading ? <Spinner small /> : <Glyph name="upload" />}
          {file ? tr("Replace file") : tr("Choose a file")}
        </label>
        {file && (
          <>
            <a href={file} target="_blank" rel="noopener noreferrer">
              {tr("Your file")}
            </a>
            {!disabled && (
              <button type="button" className="lms-linklike" onClick={() => onChange(null)}>
                {tr("Remove the file")}
              </button>
            )}
          </>
        )}
      </div>
      {error && <Alert tone="error">{isolate(error)}</Alert>}
    </div>
  );
}

// ───────────────────────────── a sitting ─────────────────────────────

type SaveShown = SaveState | "fresh";

const SAVE_TEXT: Record<SaveShown, string> = {
  get fresh() { return tr("Your answers are saved as you go"); },
  get idle() { return tr("All answers saved"); },
  get waiting() { return tr("Saving…"); },
  get saving() { return tr("Saving…"); },
  get failed() { return tr("Could not save yet, trying again"); },
};

export function Sitting({ attempt, onFinished, onGone }: { attempt: Attempt; onFinished: (r: SubmitResult) => void; onGone: () => void }) {
  const [answers, setAnswers] = useState<Answers>(() => answersOf(attempt));
  const answersRef = useRef(answers);
  answersRef.current = answers;
  const [i, setI] = useState(0);
  const [saveState, setSaveState] = useState<SaveShown>("fresh");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [lateTry, setLateTry] = useState(false);
  // What the server said about the time left, and when we heard it.
  const [clockBase, setClockBase] = useState(() => ({ left: attempt.seconds_left ?? null, at: reading() }));
  const [now, setNow] = useState(reading);
  const left = remaining(clockBase.left, clockBase.at, now);
  const total = attempt.time_limit_minutes * 60;
  const [said, setSaid] = useState("");

  const finished = useRef(false);
  const submitting = useRef(false);
  const saver = useRef<Saver<Response> | null>(null);

  // Saves go through one saver for the sitting; a new answer is on its way within a second.
  useEffect(() => {
    const s = createSaver<Response>(
      async (changes) => {
        try {
          const r = await saveAnswers(attempt.id, changes);
          if (r.status && r.status !== "In Progress") {
            // The time ran out while the answers were in flight: this is the result.
            finished.current = true;
            onFinished(r as SubmitResult);
            return;
          }
          if (r.attempt.seconds_left !== undefined) setClockBase({ left: r.attempt.seconds_left, at: reading() });
        } catch (e) {
          // A refusal will not change by asking again (the sitting was closed elsewhere).
          if (e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 429 && e.status !== 408) {
            setProblem(errorMessage(e));
            return;
          }
          throw e;
        }
      },
      setSaveState,
    );
    saver.current = s;
    const flush = () => document.visibilityState === "hidden" && void s.flush();
    document.addEventListener("visibilitychange", flush);
    window.addEventListener("pagehide", flush);
    return () => {
      document.removeEventListener("visibilitychange", flush);
      window.removeEventListener("pagehide", flush);
      if (finished.current) s.stop();
      else void s.flush().finally(() => s.stop());
      saver.current = null;
    };
    // One saver per sitting: the handlers it calls are stable for as long as the sitting lasts.
  }, [attempt.id]);

  useEffect(() => {
    if (clockBase.left === null) return;
    const t = setInterval(() => setNow(reading()), 1000);
    return () => clearInterval(t);
  }, [clockBase.left]);

  // Say the minutes out loud once, for people who cannot see the clock.
  const before = useRef(left);
  useEffect(() => {
    if (left !== null && before.current !== null) {
      const m = milestone(before.current, left, total);
      if (m) setSaid(milestoneText(m));
    }
    before.current = left;
  }, [left, total]);

  const change = (q: AttemptAnswer, response: Response) => {
    setAnswers((a) => ({ ...a, [q.question]: response }));
    saver.current?.add({ [q.question]: response });
  };

  const submit = useCallback(
    async (auto: boolean) => {
      if (submitting.current) return;
      const open = unanswered(attempt, answersRef.current);
      if (!auto && open.length) {
        const ok = await confirmDialog({
          title: tr("{value} not answered", { value: (open.length === 1 ? tr("1 question") : tr("{n} questions", { n: open.length })) }),
          message: tr("You cannot change your answers after you submit."),
          confirmLabel: tr("Submit anyway"),
          cancelLabel: tr("Keep working"),
        });
        if (!ok) {
          setI(open[0]);
          return;
        }
        // The clock may have run out while the question was on screen, and sent the answers itself.
        if (submitting.current) return;
      }
      submitting.current = true;
      setBusy(true);
      try {
        // The full set goes with it, so a save that never got through costs nothing.
        const result = await submitAnswers(attempt.id, answersRef.current);
        finished.current = true;
        onFinished(result);
      } catch (e) {
        submitting.current = false;
        setBusy(false);
        if (auto) setLateTry(true);
        toast(errorMessage(e), "error");
      }
    },
    [attempt, onFinished],
  );

  // Time is up: what is there goes in.
  useEffect(() => {
    if (left !== null && left <= 0 && !submitting.current) void submit(true);
  }, [left, submit]);

  const q = attempt.answers[i];
  if (!q) return <EmptyState title={tr("This quiz has no questions yet")} icon="alert" />;
  const open = unanswered(attempt, answers);
  const tone = left === null ? "calm" : timerTone(left, total);
  const disabled = busy || (left !== null && left <= 0);

  return (
    <section className="lms-sitting" aria-label={attempt.quiz_title}>
      <header className="lms-sit-head">
        <div className="lms-sit-title">
          <h2>{isolate(attempt.quiz_title)}</h2>
          <p className="faint" aria-live="polite">
            {tr("Attempt {attempt_no} ·", { attempt_no: attempt.attempt_no })} {isolate(SAVE_TEXT[saveState])}
          </p>
        </div>
        {left !== null && (
          <div className="lms-timer" role="timer" aria-label={tr("Time left")} data-tone={tone}>
            <Glyph name="clock" size={15} />
            <span>{isolate(clock(Math.ceil(left)))}</span>
          </div>
        )}
      </header>
      <p className="sr-only" aria-live="polite">
        {isolate(said)}
      </p>
      {problem && (
        <Alert tone="error" action={<Button onClick={onGone}>{tr("See where it stands")}</Button>}>
          {isolate(problem)}
        </Alert>
      )}
      {lateTry && (
        <Alert tone="warning" action={<Button onClick={() => void submit(true)}>{tr("Try again")}</Button>}>
          {tr("Time is up and your answers could not be sent. Those saved before the deadline count.")}
        </Alert>
      )}
      <ol className="lms-chips" aria-label={tr("Questions")}>
        {attempt.answers.map((a, n) => {
          const done = isAnswered(answers[a.question]);
          return (
            <li key={a.question}>
              <button
                type="button"
                className="lms-chip"
                data-state={n === i ? "current" : done ? "answered" : undefined}
                aria-current={n === i ? "step" : undefined}
                aria-label={(done ? tr("Question {value}, answered", { value: n + 1 }) : tr("Question {value}, not answered", { value: n + 1 }))}
                onClick={() => setI(n)}
              >
                {n + 1}
              </button>
            </li>
          );
        })}
      </ol>
      <QuestionCard key={q.question} q={q} index={i} count={attempt.answers.length} response={answers[q.question]} onChange={(r) => change(q, r)} disabled={disabled} />
      <footer className="lms-sit-foot">
        <Button disabled={i === 0 || busy} onClick={() => setI(i - 1)}>
          <Glyph name="arrowLeft" size={14} /> {tr("Previous")}
        </Button>
        <Button disabled={i === attempt.answers.length - 1 || busy} onClick={() => setI(i + 1)}>
          {tr("Next")} <Glyph name="arrowRight" size={14} />
        </Button>
        <span className="spacer" />
        <span className="faint lms-sit-count">
          {attempt.answers.length - open.length} {tr("of {length} answered", { length: attempt.answers.length })}
        </span>
        <Button variant="primary" loading={busy} onClick={() => void submit(false)}>
          {tr("Submit quiz")}
        </Button>
      </footer>
    </section>
  );
}

// ───────────────────────────── the result ─────────────────────────────

function Review({ attempt }: { attempt: Attempt }) {
  const shown = revealed(attempt);
  return (
    <>
      <h3 className="lms-subhead">{tr("Your answers")}</h3>
      {!shown && <p className="faint">{tr("The correct answers are not shown for this quiz.")}</p>}
      <ol className="lms-review">
        {attempt.answers.map((a, n) => {
          const given = responseText(a);
          const key = correctText(a.correct_answer);
          const waiting = !!a.by_person && a.points_awarded === null;
          const file = typeof a.response === "string" && a.response.startsWith("/files/") ? a.response : null;
          return (
            <li key={a.question} className="lms-review-item" data-ok={a.is_correct}>
              <div className="lms-review-head">
                <span className="lms-review-no">{n + 1}</span>
                {waiting ? <Badge tone="neutral">{tr("Waiting for a grade")}</Badge> : a.is_correct ? <Badge tone="green">{tr("Correct")}</Badge> : <Badge tone="red">{given.length ? tr("Not correct") : tr("Not answered")}</Badge>}
                <span className="faint">
                  {tr("{value} of {value2}", { value: Number(a.points_awarded ?? 0), value2: ((Number(a.points)) === 1 ? tr("1 point") : tr("{n} points", { n: Number(a.points) })) })}
                </span>
              </div>
              <Md source={a.question_text} />
              <p className="lms-review-line">
                <strong>{tr("You answered:")}</strong>{" "}
                {given.length ? given.map((g, i) => <span key={i} className="lms-line">{isolate(g)}{i < given.length - 1 ? "; " : ""}</span>) : tr("nothing")}
                {file && (
                  <>
                    {" "}
                    <a href={file} target="_blank" rel="noopener noreferrer">
                      {tr("Open")}
                    </a>
                  </>
                )}
              </p>
              {!a.is_correct && key.length > 0 && (
                <p className="lms-review-line">
                  <strong>{tr("Correct answer:")}</strong> {isolate(key.join(kind(a.question_type) === "blanks" || kind(a.question_type) === "matching" ? "; " : ", "))}
                </p>
              )}
              {a.comment && (
                <div className="lms-explain">
                  <Md source={a.comment} />
                </div>
              )}
              {a.rubric_result && <RubricTable result={a.rubric_result} />}
              {a.explanation && (
                <div className="lms-explain">
                  <Md source={a.explanation} />
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </>
  );
}

export function Result({ attempt, late, quiz, onAgain, onBack, starting }: { attempt: Attempt; late: boolean; quiz: QuizState; onAgain: () => void; onBack: () => void; starting: boolean }) {
  const v = verdict(attempt);
  const right = rightCount(attempt);
  const may = canStart({ ...quiz, in_progress: null }) && v !== "waiting";
  return (
    <section className="lms-result" data-verdict={v} aria-labelledby="lms-result-h">
      <div className="lms-score">
        <div className="lms-score-num" aria-hidden="true">
          {isolate(percentLabel(attempt.percentage))}
        </div>
        <div>
          <h2 id="lms-result-h">{v === "passed" ? tr("You passed") : v === "timed-out" ? tr("Time ran out") : v === "waiting" ? tr("Waiting for a grade") : tr("Not passed this time")}</h2>
          {v === "waiting" && <p className="muted">{tr("Some answers wait for a person to grade them, so this result is not final.")}</p>}
          <p className="muted">
            {tr("{right} of {length} questions right ·", { right, length: attempt.answers.length })} {tr("{value} of {value2} points", { value: Number(attempt.score ?? 0), value2: Number(attempt.max_score) })}
          </p>
          <p className="faint">{tr("Pass mark")} {isolate(percentLabel(attempt.passing_percentage))}</p>
        </div>
      </div>
      {late && <Alert tone="warning">{tr("Time ran out before this was sent. Answers saved before the deadline were scored.")}</Alert>}
      {v === "passed" && <Alert tone="success">{tr("This lesson is complete.")}</Alert>}
      <div className="row">
        {may && (
          <Button variant={v === "passed" ? "default" : "primary"} loading={starting} onClick={onAgain}>
            {v === "passed" ? tr("Take it again") : tr("Try again")}
            {quiz.attempts_left !== null && <span className="faint"> {tr("({attempts_left} left)", { attempts_left: quiz.attempts_left })}</span>}
          </Button>
        )}
        {!may && !quiz.passed && v !== "waiting" && <span className="muted">{isolate(attemptsText(quiz))}.</span>}
        <Button variant="ghost" onClick={onBack}>
          {tr("Back to the quiz")}
        </Button>
      </div>
      <Review attempt={attempt} />
    </section>
  );
}

// ───────────────────────────── before the quiz ─────────────────────────────

const STATUS_BADGE: Record<string, { tone: "green" | "red" | "amber" | "neutral"; text: string }> = {
  waiting: { tone: "neutral", get text() { return tr("Waiting for a grade"); } },
  passed: { tone: "green", get text() { return tr("Passed"); } },
  failed: { tone: "red", get text() { return tr("Not passed"); } },
  "timed-out": { tone: "amber", get text() { return tr("Timed out"); } },
};

function History({ quiz, onReview }: { quiz: QuizState; onReview: (id: string) => void }) {
  const rows = useQuery({ queryKey: keys.attempts(quiz.id), queryFn: () => getAttempts(quiz.id), enabled: quiz.attempts_used > 0, staleTime: 0 });
  if (quiz.attempts_used === 0) return null;
  const done = (rows.data ?? []).filter((r: AttemptRow) => r.status !== "In Progress");
  if (!rows.data && rows.isLoading) return <Spinner label={tr("Loading your attempts")} />;
  if (!done.length) return null;
  return (
    <>
      <h3 className="lms-subhead">{tr("Your attempts")}</h3>
      <ul className="lms-history">
        {done.map((r) => {
          const b = STATUS_BADGE[r.status === "Awaiting Grading" ? "waiting" : r.passed ? "passed" : r.status === "Timed Out" ? "timed-out" : "failed"];
          return (
            <li key={r.id}>
              <span className="lms-history-no">{tr("Attempt {attempt_no}", { attempt_no: r.attempt_no })}</span>
              <span className="faint">{r.submitted_at ? formatDateTime(r.submitted_at) : ""}</span>
              <span className="lms-history-score">{r.status === "Awaiting Grading" ? tr("Waiting for a grade") : isolate(percentLabel(r.percentage))}</span>
              <Badge tone={b.tone}>{isolate(b.text)}</Badge>
              <span className="spacer" />
              <button type="button" className="btn btn-ghost btn-sm" onClick={() => onReview(r.id)}>
                {tr("Review")}
              </button>
            </li>
          );
        })}
      </ul>
    </>
  );
}

export function Intro({ quiz, starting, error, onStart, onReview }: { quiz: QuizState; starting: boolean; error: string | null; onStart: () => void; onReview: (id: string) => void }) {
  const may = canStart(quiz);
  const under = quiz.in_progress;
  return (
    <section className="lms-quiz-intro" aria-label={tr("Quiz")}>
      <div className="lms-quiz-head">
        <h2>{isolate(quiz.title)}</h2>
        {quiz.passed && <Badge tone="green">{tr("Passed")}</Badge>}
      </div>
      <Md source={quiz.description} />
      <dl className="lms-facts">
        <div>
          <dt>{tr("Questions")}</dt>
          <dd>{quiz.questions}</dd>
        </div>
        <div>
          <dt>{tr("Pass mark")}</dt>
          <dd>{isolate(percentLabel(quiz.passing_percentage))}</dd>
        </div>
        <div>
          <dt>{tr("Time")}</dt>
          <dd>{quiz.time_limit_minutes ? duration(quiz.time_limit_minutes) : tr("No limit")}</dd>
        </div>
        <div>
          <dt>{tr("Attempts")}</dt>
          <dd>{isolate(attemptsText(quiz))}</dd>
        </div>
        {quiz.best_percentage && (
          <div>
            <dt>{tr("Your best")}</dt>
            <dd>{isolate(percentLabel(quiz.best_percentage))}</dd>
          </div>
        )}
      </dl>
      {under ? (
        <Alert tone="info">
          {placed(tr("You have an attempt under way{p1}. The answers you gave are saved; resume to carry on."), { p1: under.deadline ? tr(", which ends at {value}", { value: formatDateTime(under.deadline) }) : "" })}
        </Alert>
      ) : (
        quiz.time_limit_minutes > 0 && <p className="faint">{tr("The clock starts when you begin and keeps running if you leave the page.")}</p>
      )}
      {error && <Alert tone="error">{isolate(error)}</Alert>}
      <div className="row">
        <Button variant="primary" loading={starting} disabled={!may} onClick={onStart}>
          {quiz.in_progress ? tr("Resume quiz") : quiz.attempts_used ? (quiz.passed ? tr("Take it again") : tr("Try again")) : tr("Start quiz")}
        </Button>
        {!may && <span className="muted">{isolate(attemptsText(quiz))}.</span>}
      </div>
      <History quiz={quiz} onReview={onReview} />
    </section>
  );
}

// ───────────────────────────── the panel ─────────────────────────────

/** The quiz of a lesson: its rules, the sitting, the result and the attempts so far. `onProgress`
 *  tells the page a result may have finished the lesson (or the course). A sitting under way is
 *  resumed by the learner, never by the page: asking the server to start a quiz whose attempt has
 *  just run out would open a new one. */
export function QuizPanel({ quiz, enrollment, lessonId, onProgress, onSitting }: { quiz: QuizState; enrollment: string; lessonId: string; onProgress: () => void; onSitting?: (on: boolean) => void }) {
  const qc = useQueryClient();
  const [sitting, setSitting] = useState<Attempt | null>(null);
  const [shown, setShown] = useState<{ attempt: Attempt; late: boolean } | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadingReview, setLoadingReview] = useState(false);

  const begin = useCallback(async () => {
    setStarting(true);
    setError(null);
    try {
      const { attempt } = await startQuiz(enrollment, quiz.id);
      setSitting(attempt);
      setShown(null);
      void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
    } catch (e) {
      setError(errorMessage(e));
      void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
    } finally {
      setStarting(false);
    }
  }, [enrollment, quiz.id, lessonId, qc]);

  // While a sitting is under way the page keeps its own way on out of reach.
  const sat = !!sitting;
  useEffect(() => {
    onSitting?.(sat);
    return () => onSitting?.(false);
  }, [sat, onSitting]);

  const finished = useCallback(
    (r: SubmitResult) => {
      setSitting(null);
      setShown({ attempt: r.attempt, late: r.late_answers_ignored });
      void qc.invalidateQueries({ queryKey: keys.attempts(quiz.id) });
      onProgress();
    },
    [qc, quiz.id, onProgress],
  );

  const review = async (id: string) => {
    setLoadingReview(true);
    try {
      setShown({ attempt: await getAttempt(id), late: false });
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setLoadingReview(false);
    }
  };

  if (sitting)
    return (
      <Sitting
        key={sitting.id}
        attempt={sitting}
        onFinished={finished}
        onGone={() => {
          setSitting(null);
          onProgress();
        }}
      />
    );
  if (shown) return <Result attempt={shown.attempt} late={shown.late} quiz={quiz} starting={starting} onAgain={() => void begin()} onBack={() => setShown(null)} />;
  if (loadingReview) return <EmptyState title={tr("Opening the attempt")} icon="refresh" />;
  return <Intro quiz={quiz} starting={starting} error={error} onStart={() => void begin()} onReview={(id) => void review(id)} />;
}
