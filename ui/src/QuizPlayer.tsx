// The quiz player. A learner reads the rules, starts a sitting, answers one question at a time
// (every change is saved a moment later, so a closed tab loses nothing), submits, and reads the
// result. The clock is the server's: it says how many seconds are left and the page counts down
// from that. The server holds the answer keys and decides every score; this shows what it says.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ApiError, errorMessage, formatDateTime } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, EmptyState, Spinner, toast } from "@ferrum/kit";
import { getAttempt, getAttempts, keys, saveAnswers, startQuiz, submitAnswers } from "./api";
import { clock, duration, plural } from "./format";
import { Glyph } from "./Glyph";
import { Md } from "./Md";
import { percentLabel } from "./outline";
import {
  type Answers,
  answersOf,
  attemptsText,
  canStart,
  choose,
  chosen,
  isAnswered,
  isChoice,
  isMultiple,
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
  const points = Number(q.points);
  return (
    <div className="lms-question">
      <p className="lms-q-meta faint">
        Question {index + 1} of {count} · {plural(points, "point")}
        {multi ? " · Choose every answer that applies" : ""}
      </p>
      <div id={text} className="lms-q-text">
        <Md source={q.question_text} />
      </div>
      {isChoice(q.question_type) ? (
        <div className="lms-options" role={multi ? "group" : "radiogroup"} aria-labelledby={text}>
          {(q.options ?? []).map((o) => {
            const on = chosen(response, o.key);
            return (
              <label key={o.key} className="lms-option" data-chosen={on || undefined}>
                <input type={multi ? "checkbox" : "radio"} name={q.question} checked={on} disabled={disabled} onChange={() => onChange(choose(q, response, o.key))} />
                <span>{o.text}</span>
              </label>
            );
          })}
          {isAnswered(response) && !disabled && (
            <button type="button" className="lms-linklike" onClick={() => onChange(null)}>
              Clear my answer
            </button>
          )}
        </div>
      ) : (
        <textarea
          className="textarea lms-short"
          rows={3}
          maxLength={500}
          aria-labelledby={text}
          placeholder="Type your answer"
          disabled={disabled}
          value={typeof response === "string" ? response : ""}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
    </div>
  );
}

// ───────────────────────────── a sitting ─────────────────────────────

type SaveShown = SaveState | "fresh";

const SAVE_TEXT: Record<SaveShown, string> = {
  fresh: "Your answers are saved as you go",
  idle: "All answers saved",
  waiting: "Saving…",
  saving: "Saving…",
  failed: "Could not save yet, trying again",
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
          title: `${plural(open.length, "question")} not answered`,
          message: "You cannot change your answers after you submit.",
          confirmLabel: "Submit anyway",
          cancelLabel: "Keep working",
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
  if (!q) return <EmptyState title="This quiz has no questions yet" icon="alert" />;
  const open = unanswered(attempt, answers);
  const tone = left === null ? "calm" : timerTone(left, total);
  const disabled = busy || (left !== null && left <= 0);

  return (
    <section className="lms-sitting" aria-label={attempt.quiz_title}>
      <header className="lms-sit-head">
        <div className="lms-sit-title">
          <h2>{attempt.quiz_title}</h2>
          <p className="faint" aria-live="polite">
            Attempt {attempt.attempt_no} · {SAVE_TEXT[saveState]}
          </p>
        </div>
        {left !== null && (
          <div className="lms-timer" role="timer" aria-label="Time left" data-tone={tone}>
            <Glyph name="clock" size={15} />
            <span>{clock(Math.ceil(left))}</span>
          </div>
        )}
      </header>
      <p className="sr-only" aria-live="polite">
        {said}
      </p>
      {problem && (
        <Alert tone="error" action={<Button onClick={onGone}>See where it stands</Button>}>
          {problem}
        </Alert>
      )}
      {lateTry && (
        <Alert tone="warning" action={<Button onClick={() => void submit(true)}>Try again</Button>}>
          Time is up and your answers could not be sent. Those saved before the deadline count.
        </Alert>
      )}
      <ol className="lms-chips" aria-label="Questions">
        {attempt.answers.map((a, n) => {
          const done = isAnswered(answers[a.question]);
          return (
            <li key={a.question}>
              <button
                type="button"
                className="lms-chip"
                data-state={n === i ? "current" : done ? "answered" : undefined}
                aria-current={n === i ? "step" : undefined}
                aria-label={`Question ${n + 1}, ${done ? "answered" : "not answered"}`}
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
          <Glyph name="arrowLeft" size={14} /> Previous
        </Button>
        <Button disabled={i === attempt.answers.length - 1 || busy} onClick={() => setI(i + 1)}>
          Next <Glyph name="arrowRight" size={14} />
        </Button>
        <span className="spacer" />
        <span className="faint lms-sit-count">
          {attempt.answers.length - open.length} of {attempt.answers.length} answered
        </span>
        <Button variant="primary" loading={busy} onClick={() => void submit(false)}>
          Submit quiz
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
      <h3 className="lms-subhead">Your answers</h3>
      {!shown && <p className="faint">The correct answers are not shown for this quiz.</p>}
      <ol className="lms-review">
        {attempt.answers.map((a, n) => {
          const given = responseText(a);
          return (
            <li key={a.question} className="lms-review-item" data-ok={a.is_correct}>
              <div className="lms-review-head">
                <span className="lms-review-no">{n + 1}</span>
                {a.is_correct ? <Badge tone="green">Correct</Badge> : <Badge tone="red">{given.length ? "Not correct" : "Not answered"}</Badge>}
                <span className="faint">
                  {Number(a.points_awarded ?? 0)} of {plural(Number(a.points), "point")}
                </span>
              </div>
              <Md source={a.question_text} />
              <p className="lms-review-line">
                <strong>You answered:</strong> {given.length ? given.join(", ") : "nothing"}
              </p>
              {!a.is_correct && a.correct_answer && a.correct_answer.length > 0 && (
                <p className="lms-review-line">
                  <strong>Correct answer:</strong> {a.correct_answer.join(", ")}
                </p>
              )}
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
  const may = canStart({ ...quiz, in_progress: null });
  return (
    <section className="lms-result" data-verdict={v} aria-labelledby="lms-result-h">
      <div className="lms-score">
        <div className="lms-score-num" aria-hidden="true">
          {percentLabel(attempt.percentage)}
        </div>
        <div>
          <h2 id="lms-result-h">{v === "passed" ? "You passed" : v === "timed-out" ? "Time ran out" : "Not passed this time"}</h2>
          <p className="muted">
            {right} of {attempt.answers.length} questions right · {Number(attempt.score ?? 0)} of {Number(attempt.max_score)} points
          </p>
          <p className="faint">Pass mark {percentLabel(attempt.passing_percentage)}</p>
        </div>
      </div>
      {late && <Alert tone="warning">Time ran out before this was sent. Answers saved before the deadline were scored.</Alert>}
      {v === "passed" && <Alert tone="success">This lesson is complete.</Alert>}
      <div className="row">
        {may && (
          <Button variant={v === "passed" ? "default" : "primary"} loading={starting} onClick={onAgain}>
            {v === "passed" ? "Take it again" : "Try again"}
            {quiz.attempts_left !== null && <span className="faint"> ({quiz.attempts_left} left)</span>}
          </Button>
        )}
        {!may && !quiz.passed && <span className="muted">{attemptsText(quiz)}.</span>}
        <Button variant="ghost" onClick={onBack}>
          Back to the quiz
        </Button>
      </div>
      <Review attempt={attempt} />
    </section>
  );
}

// ───────────────────────────── before the quiz ─────────────────────────────

const STATUS_BADGE: Record<string, { tone: "green" | "red" | "amber"; text: string }> = {
  passed: { tone: "green", text: "Passed" },
  failed: { tone: "red", text: "Not passed" },
  "timed-out": { tone: "amber", text: "Timed out" },
};

function History({ quiz, onReview }: { quiz: QuizState; onReview: (id: string) => void }) {
  const rows = useQuery({ queryKey: keys.attempts(quiz.id), queryFn: () => getAttempts(quiz.id), enabled: quiz.attempts_used > 0, staleTime: 0 });
  if (quiz.attempts_used === 0) return null;
  const done = (rows.data ?? []).filter((r: AttemptRow) => r.status !== "In Progress");
  if (!rows.data && rows.isLoading) return <Spinner label="Loading your attempts" />;
  if (!done.length) return null;
  return (
    <>
      <h3 className="lms-subhead">Your attempts</h3>
      <ul className="lms-history">
        {done.map((r) => {
          const b = STATUS_BADGE[r.passed ? "passed" : r.status === "Timed Out" ? "timed-out" : "failed"];
          return (
            <li key={r.id}>
              <span className="lms-history-no">Attempt {r.attempt_no}</span>
              <span className="faint">{r.submitted_at ? formatDateTime(r.submitted_at) : ""}</span>
              <span className="lms-history-score">{percentLabel(r.percentage)}</span>
              <Badge tone={b.tone}>{b.text}</Badge>
              <span className="spacer" />
              <button type="button" className="btn btn-ghost btn-sm" onClick={() => onReview(r.id)}>
                Review
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
    <section className="lms-quiz-intro" aria-label="Quiz">
      <div className="lms-quiz-head">
        <h2>{quiz.title}</h2>
        {quiz.passed && <Badge tone="green">Passed</Badge>}
      </div>
      <Md source={quiz.description} />
      <dl className="lms-facts">
        <div>
          <dt>Questions</dt>
          <dd>{quiz.questions}</dd>
        </div>
        <div>
          <dt>Pass mark</dt>
          <dd>{percentLabel(quiz.passing_percentage)}</dd>
        </div>
        <div>
          <dt>Time</dt>
          <dd>{quiz.time_limit_minutes ? duration(quiz.time_limit_minutes) : "No limit"}</dd>
        </div>
        <div>
          <dt>Attempts</dt>
          <dd>{attemptsText(quiz)}</dd>
        </div>
        {quiz.best_percentage && (
          <div>
            <dt>Your best</dt>
            <dd>{percentLabel(quiz.best_percentage)}</dd>
          </div>
        )}
      </dl>
      {under ? (
        <Alert tone="info">
          You have an attempt under way{under.deadline ? `, which ends at ${formatDateTime(under.deadline)}` : ""}. The answers you gave are saved; resume to carry on.
        </Alert>
      ) : (
        quiz.time_limit_minutes > 0 && <p className="faint">The clock starts when you begin and keeps running if you leave the page.</p>
      )}
      {error && <Alert tone="error">{error}</Alert>}
      <div className="row">
        <Button variant="primary" loading={starting} disabled={!may} onClick={onStart}>
          {quiz.in_progress ? "Resume quiz" : quiz.attempts_used ? (quiz.passed ? "Take it again" : "Try again") : "Start quiz"}
        </Button>
        {!may && <span className="muted">{attemptsText(quiz)}.</span>}
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
  if (loadingReview) return <EmptyState title="Opening the attempt" icon="refresh" />;
  return <Intro quiz={quiz} starting={starting} error={error} onStart={() => void begin()} onReview={(id) => void review(id)} />;
}
