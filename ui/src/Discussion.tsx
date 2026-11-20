// A course's discussion: threads for the whole course and for each lesson, replies, following, reports,
// and the moderation an instructor does (pin, close, mark an answer, remove with a reason). The server
// decides who may post and moderate and never sends a learner a removed post's text; this draws it.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { ApiError, errorMessage, formatDateTime, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, Dialog, EmptyState, Field, PageHead, PageLoading, PageStack, Spinner, toast } from "@ferrum/kit";
import { closeThread, follow, getCourse, getReported, getThread, getThreads, keys, markAnswer, pin, removePost, reply, reportPost, startThread } from "./api";
import { Md } from "./Md";
import { paths } from "./lesson";
import { Failed } from "./states";
import type { Post } from "./types";

const fieldMessages = (e: unknown): { general: string | null; fields: Record<string, string> } =>
  e instanceof ApiError && e.fields.length ? { general: null, fields: Object.fromEntries(e.fields.map((f) => [f.field, f.message])) } : { general: errorMessage(e), fields: {} };

/** A person asked for a reason: reporting a post, or removing one. */
function ReasonDialog({ open, title, hint, confirm, onClose, onSend }: { open: boolean; title: string; hint?: string; confirm: string; onClose: () => void; onSend: (reason: string) => Promise<void> }) {
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!reason.trim()) return setError(tr("Say why."));
    setBusy(true);
    setError(null);
    try {
      await onSend(reason.trim());
      setReason("");
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onClose={onClose} title={title}>
      <form className="stack" onSubmit={(e) => void submit(e)}>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        {hint && <p className="faint">{hint}</p>}
        <Field label={tr("Reason")} htmlFor="lms-reason" required>
          <textarea id="lms-reason" className="textarea" rows={4} value={reason} onChange={(e) => setReason(e.target.value)} />
        </Field>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <Button variant="ghost" onClick={onClose}>
            {tr("Cancel")}
          </Button>
          <Button variant="primary" type="submit" loading={busy}>
            {confirm}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function Marks({ p, moderator }: { p: Post; moderator: boolean }) {
  return (
    <>
      {p.by_instructor && <Badge tone="blue">{tr("Instructor")}</Badge>}
      {p.pinned && <Badge tone="amber">{tr("Pinned")}</Badge>}
      {p.closed && <Badge tone="neutral">{tr("Closed")}</Badge>}
      {p.answer && <Badge tone="green">{tr("Instructor's answer")}</Badge>}
      {moderator && p.reported && !p.removed && <Badge tone="red">{tr("Reported")}</Badge>}
    </>
  );
}

// ───────────────────────────── the list ─────────────────────────────

function NewThread({ course, lesson, onDone }: { course: string; lesson?: string; onDone: (post: Post) => void }) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ general: string | null; fields: Record<string, string> }>({ general: null, fields: {} });
  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    setBusy(true);
    setError({ general: null, fields: {} });
    try {
      const r = await startThread(course, { title: title.trim(), body, ...(lesson ? { lesson } : {}) });
      setOpen(false);
      setTitle("");
      setBody("");
      onDone(r.post);
    } catch (err) {
      setError(fieldMessages(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button icon="plus" onClick={() => setOpen(true)}>
        {lesson ? tr("Ask about this lesson") : tr("New thread")}
      </Button>
      <Dialog open={open} onClose={() => setOpen(false)} title={lesson ? tr("Ask about this lesson") : tr("New thread")}>
        <form className="stack" onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void submit()}>
          {error.general && <Alert tone="error">{isolate(error.general)}</Alert>}
          <Field label={tr("Title")} htmlFor="lms-thread-title" required error={error.fields.title}>
            <input id="lms-thread-title" className="input" maxLength={200} value={title} onChange={(e) => setTitle(e.target.value)} />
          </Field>
          <Field label={tr("Your post")} htmlFor="lms-thread-body" required error={error.fields.body} hint={tr("Markdown works.")}>
            <textarea id="lms-thread-body" className="textarea" rows={8} maxLength={20000} value={body} onChange={(e) => setBody(e.target.value)} />
          </Field>
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              {tr("Cancel")}
            </Button>
            <Button variant="primary" type="submit" loading={busy}>
              {tr("Post")}
            </Button>
          </div>
        </form>
      </Dialog>
    </>
  );
}

function ThreadRows({ slug, threads, moderator }: { slug: string; threads: Post[]; moderator: boolean }) {
  return (
    <ul className="lms-threads lms-plain">
      {threads.map((p) => (
        <li key={p.id}>
          <Link to={paths.thread(slug, p.id)}>
            <strong>{isolate(p.title ?? "")}</strong>
          </Link>{" "}
          <Marks p={p} moderator={moderator} />
          <div className="faint">
            {isolate(p.author_name ?? "")} · {tr("{n} replies", { n: p.replies })}
            {p.last_post_at ? <> · {formatDateTime(p.last_post_at)}</> : null}
          </div>
        </li>
      ))}
    </ul>
  );
}

export function DiscussionRoute() {
  const { slug = "" } = useParams();
  const nav = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.threads(slug), queryFn: () => getThreads(slug), staleTime: 0, retry: false });
  const course = useQuery({ queryKey: keys.course(slug), queryFn: () => getCourse(slug), staleTime: 60_000, retry: false });
  if (q.error instanceof ApiError && q.error.status === 404) return <OutsideCourse slug={slug} />;
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load the discussion")} />;
  if (!q.data) return <PageLoading label={tr("Loading the discussion")} />;
  return (
    <PageStack className="lms-page">
      <PageHead
        back={{ to: paths.course(slug), label: tr("Back to the course") }}
        title={tr("Discussion")}
        actions={
          course.data ? (
            <NewThread
              course={course.data.id}
              onDone={(p) => {
                void qc.invalidateQueries({ queryKey: ["lms", "threads", slug] });
                nav(paths.thread(slug, p.id));
              }}
            />
          ) : undefined
        }
      />
      {q.data.threads.length ? <ThreadRows slug={slug} threads={q.data.threads} moderator={q.data.moderator} /> : <EmptyState title={tr("No threads yet")}>{tr("Start the first one.")}</EmptyState>}
    </PageStack>
  );
}

export function OutsideCourse({ slug }: { slug: string }) {
  return (
    <EmptyState title={tr("The discussion is for the course's learners and instructors")}>
      <Link to={paths.course(slug)}>{tr("Go to the course page")}</Link>
    </EmptyState>
  );
}

// ───────────────────────────── a thread ─────────────────────────────

function PostView({ p, course, moderator, isHead, onChanged }: { p: Post; course: string; moderator: boolean; isHead: boolean; onChanged: () => void }) {
  const [reporting, setReporting] = useState(false);
  const [removing, setRemoving] = useState(false);
  const act = async (f: () => Promise<unknown>) => {
    try {
      await f();
      onChanged();
    } catch (e) {
      toast(errorMessage(e));
    }
  };
  return (
    <article className="lms-post" aria-label={isolate(p.title ?? p.author_name ?? "")} data-removed={p.removed || undefined}>
      <div className="row">
        <strong>{p.removed && p.author_name === null ? tr("A removed post") : isolate(p.author_name ?? "")}</strong> <Marks p={p} moderator={moderator} />
        <span className="faint">{formatDateTime(p.created_at)}</span>
      </div>
      {p.removed && p.body === null ? (
        <p className="faint">{tr("This post was removed")}</p>
      ) : (
        <>
          {p.removed && <Alert tone="warning">{tr("Removed by {who}: {why}", { who: p.removed_by ?? "", why: p.removed_reason ?? "" })}</Alert>}
          {p.body && <Md source={p.body} />}
        </>
      )}
      <div className="row">
        {!p.mine && !p.removed && (
          <button type="button" className="lms-linklike" onClick={() => setReporting(true)}>
            {tr("Report")}
          </button>
        )}
        {moderator && isHead && (
          <>
            <button type="button" className="lms-linklike" onClick={() => void act(() => pin(p.id))}>
              {p.pinned ? tr("Unpin") : tr("Pin")}
            </button>
            <button type="button" className="lms-linklike" onClick={() => void act(() => closeThread(p.id))}>
              {p.closed ? tr("Reopen") : tr("Close")}
            </button>
          </>
        )}
        {moderator && !isHead && !p.removed && (
          <button type="button" className="lms-linklike" onClick={() => void act(() => markAnswer(p.id))}>
            {p.answer ? tr("Unmark the answer") : tr("Mark as answer")}
          </button>
        )}
        {moderator && !p.removed && (
          <button type="button" className="lms-linklike" onClick={() => setRemoving(true)}>
            {tr("Remove")}
          </button>
        )}
      </div>
      <ReasonDialog
        open={reporting}
        title={tr("Report this post")}
        confirm={tr("Report")}
        onClose={() => setReporting(false)}
        onSend={async (reason) => {
          await reportPost(course, p.id, reason);
          toast(tr("Reported to the instructors"));
        }}
      />
      <ReasonDialog
        open={removing}
        title={tr("Remove this post")}
        hint={tr("The post stays on record. Learners no longer read it.")}
        confirm={tr("Remove")}
        onClose={() => setRemoving(false)}
        onSend={async (reason) => {
          await removePost(p.id, reason);
          onChanged();
        }}
      />
    </article>
  );
}

export function ThreadRoute() {
  const { slug = "", thread = "" } = useParams();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.thread(thread), queryFn: () => getThread(slug, thread), staleTime: 0, retry: false });
  const courseQ = useQuery({ queryKey: keys.course(slug), queryFn: () => getCourse(slug), staleTime: 60_000, retry: false });
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const course = courseQ.data?.id ?? "";
  const changed = () => {
    void qc.invalidateQueries({ queryKey: keys.thread(thread) });
    void qc.invalidateQueries({ queryKey: ["lms", "threads", slug] });
    void qc.invalidateQueries({ queryKey: ["lms", "reported"] });
  };
  if (q.error instanceof ApiError && q.error.status === 404) return <OutsideCourse slug={slug} />;
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load this thread")} />;
  const page = q.data;
  if (!page) return <PageLoading label={tr("Loading the thread")} />;
  const open = !page.thread.closed && !page.thread.removed;
  const send = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await reply(course, thread, text);
      setText("");
      changed();
    } catch (err) {
      setError(errorMessage(err));
      changed();
    } finally {
      setBusy(false);
    }
  };
  const answer = page.replies.find((r) => r.answer && !r.removed);
  return (
    <PageStack className="lms-page">
      <PageHead
        back={{ to: paths.discussion(slug), label: tr("Discussion") }}
        title={<>{isolate(page.thread.title ?? "")}</>}
        actions={
          <Button
            variant={page.following ? "default" : "primary"}
            onClick={() =>
              void follow(course, thread, !page.following)
                .then(changed)
                .catch((e) => toast(errorMessage(e)))
            }
          >
            {page.following ? tr("Following") : tr("Follow")}
          </Button>
        }
      />
      <PostView p={page.thread} course={course} moderator={page.moderator} isHead onChanged={changed} />
      {answer && (
        <section aria-label={tr("Instructor's answer")}>
          <h2 className="lms-group">{tr("Instructor's answer")}</h2>
          <PostView p={answer} course={course} moderator={page.moderator} isHead={false} onChanged={changed} />
        </section>
      )}
      <section aria-label={tr("Replies")}>
        <h2 className="lms-group">{tr("{n} replies", { n: page.replies.length })}</h2>
        {page.replies.map((r) => (
          <PostView key={r.id} p={r} course={course} moderator={page.moderator} isHead={false} onChanged={changed} />
        ))}
      </section>
      {open ? (
        <form className="stack" onSubmit={(e) => void send(e)} onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && void send()}>
          {error && <Alert tone="error">{isolate(error)}</Alert>}
          <Field label={tr("Your reply")} htmlFor="lms-reply" hint={tr("Markdown works.")}>
            <textarea id="lms-reply" className="textarea" rows={5} maxLength={20000} value={text} onChange={(e) => setText(e.target.value)} />
          </Field>
          <div className="row">
            <Button variant="primary" type="submit" loading={busy} disabled={!text.trim() || !course}>
              {tr("Reply")}
            </Button>
          </div>
        </form>
      ) : (
        <p className="faint">{tr("This thread takes no more replies.")}</p>
      )}
    </PageStack>
  );
}

// ───────────────────────────── under a lesson ─────────────────────────────

/** A lesson's own threads, under its foot, for an enrolled learner or an author. */
export function LessonThreads({ slug, course, lesson }: { slug: string; course: string; lesson: string }) {
  const nav = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.threads(slug, lesson), queryFn: () => getThreads(slug, lesson), staleTime: 0, retry: false });
  return (
    <section className="lms-lesson-discussion" aria-label={tr("Discussion")}>
      <div className="row">
        <h2 className="lms-group">{tr("Discussion")}</h2>
        <span className="spacer" />
        <NewThread
          course={course}
          lesson={lesson}
          onDone={(p) => {
            void qc.invalidateQueries({ queryKey: ["lms", "threads", slug] });
            nav(paths.thread(slug, p.id));
          }}
        />
      </div>
      {q.error ? <Failed error={q.error} onRetry={() => void q.refetch()} /> : !q.data ? <Spinner label={tr("Loading the discussion")} /> : q.data.threads.length ? <ThreadRows slug={slug} threads={q.data.threads} moderator={q.data.moderator} /> : <p className="faint">{tr("No questions about this lesson yet.")}</p>}
    </section>
  );
}

// ───────────────────────────── the moderators' queue ─────────────────────────────

export function Reported({ course, slug }: { course: string; slug: string }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.reported(course), queryFn: () => getReported(course), staleTime: 0 });
  const [removing, setRemoving] = useState<string | null>(null);
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not load the reported posts")} />;
  if (!q.data) return <PageLoading label={tr("Loading the reported posts")} />;
  if (!q.data.length) return <EmptyState title={tr("Nothing is reported")} />;
  return (
    <section aria-label={tr("Reported posts")}>
      <ul className="lms-plain">
        {q.data.map((p) => (
          <li key={p.id}>
            <strong>{isolate(p.author_name ?? p.author ?? "")}</strong> <span className="faint">{isolate(p.author ?? "")}</span>
            {p.body && <Md source={p.body} />}
            <p className="faint">
              {tr("Reported by {who}: {why}", { who: p.reported_by ?? "", why: p.report_reason ?? "" })}
            </p>
            <div className="row">
              <Link className="btn btn-default btn-sm" to={paths.thread(slug, p.thread ?? p.id)}>
                {tr("Open the thread")}
              </Link>
              <Button size="sm" variant="ghost" onClick={() => setRemoving(p.id)}>
                {tr("Remove")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <ReasonDialog
        open={removing !== null}
        title={tr("Remove this post")}
        hint={tr("The post stays on record. Learners no longer read it.")}
        confirm={tr("Remove")}
        onClose={() => setRemoving(null)}
        onSend={async (reason) => {
          await removePost(removing!, reason);
          void qc.invalidateQueries({ queryKey: keys.reported(course) });
        }}
      />
    </section>
  );
}
