// The course builder: the outline of chapters and lessons on the right, the lesson being written
// on the left. Chapters and lessons move by dragging, and by buttons and a menu for keyboards and
// touch screens: the same moves, shown at once and sent as a few edits of `position` against the
// versions on screen, one move after another. If the server refuses, the outline goes back to
// what it holds and says why. Publishing asks the server's own rules in advance.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type DragEvent, type KeyboardEvent, useCallback, useMemo, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { api, ApiError, docPath, errorMessage } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, EmptyState, ErrorState, Icon, PageLoading, statusTone, toast, useShellTitle } from "@ferrum/kit";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@ferrum/kit/primitives";
import { rowsOf } from "./api";
import { duration } from "./format";
import { Glyph, lessonGlyph, LESSON_LABEL } from "./Glyph";
import { docKey, type LessonDoc, LessonEditor } from "./LessonEditor";
import { paths } from "./lesson";
import { Sheet, useFocusMode } from "./parts";
import { publishIssues } from "./publish";
import { applyPatches, type ChapterRow, dropChapter, dropLesson, dropLessonOnChapter, type Edge, type Layout, layoutOf, type LessonRow, moveLesson, patchesFor, shiftChapter, shiftLesson } from "./reorder";
import type { LessonType } from "./types";

export interface CourseDoc {
  id: string;
  version: number;
  title: string;
  slug: string;
  status: "Draft" | "Published" | "Archived";
}

export interface LessonSummary extends LessonRow {
  lesson_type: LessonType;
  duration_minutes: number | null;
  is_preview: boolean;
  required: boolean;
  quiz: string | null;
  assignment: string | null;
}

export interface Data {
  course: CourseDoc;
  chapters: ChapterRow[];
  lessons: LessonSummary[];
  /** Questions in each of the course's quizzes. */
  quizzes: Record<string, number>;
}

export const KEY = (course: string) => ["lms", "build", course] as const;

async function load(course: string): Promise<Data> {
  const where = JSON.stringify({ course });
  const [doc, chapters, lessons, quizzes] = await Promise.all([
    api.get<CourseDoc>(docPath("LmsCourse", course)),
    rowsOf<ChapterRow>("LmsChapter", { filters: where, fields: "title,position,version", sort: "position asc", limit: 500 }),
    rowsOf<LessonSummary>("LmsLesson", { filters: where, fields: "title,chapter,position,lesson_type,duration_minutes,is_preview,required,quiz,assignment,version", sort: "position asc", limit: 500 }),
    rowsOf<{ id: string; question_count: number | null }>("LmsQuiz", { filters: where, fields: "question_count", limit: 500 }),
  ]);
  return { course: doc, chapters, lessons, quizzes: Object.fromEntries(quizzes.map((q) => [q.id, q.question_count ?? 0])) };
}

type Drag = { kind: "lesson" | "chapter"; id: string } | null;
type Over = { kind: "lesson" | "chapter"; id: string; edge: Edge } | null;

/** A title that is text until it is clicked, then a box; Enter or leaving saves, Escape does not. */
function Rename({ value, label, onSave }: { value: string; label: string; onSave: (title: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(value);
  // Enter, Escape and leaving the box each end the edit, and only the first counts.
  const over = useRef(false);
  const done = (save: boolean) => {
    if (over.current) return;
    over.current = true;
    setEditing(false);
    const next = text.trim();
    if (save && next && next !== value) onSave(next);
    else setText(value);
  };
  if (!editing)
    return (
      <button
        type="button"
        className="lms-b-name"
        title="Rename"
        onClick={() => {
          over.current = false;
          setText(value);
          setEditing(true);
        }}
      >
        {value}
      </button>
    );
  return (
    <input
      className="input lms-b-rename"
      aria-label={label}
      autoFocus
      value={text}
      maxLength={200}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => done(true)}
      onKeyDown={(e: KeyboardEvent) => {
        if (e.key === "Enter") done(true);
        if (e.key === "Escape") done(false);
      }}
    />
  );
}

export function BuilderRoute() {
  const { course = "" } = useParams();
  return <Builder key={course} course={course} />;
}

function Builder({ course }: { course: string }) {
  useFocusMode();
  const qc = useQueryClient();
  const [sp, setSp] = useSearchParams();
  const selected = sp.get("lesson");
  const q = useQuery({ queryKey: KEY(course), queryFn: () => load(course), staleTime: 0 });
  const data = q.data;
  // The outline as it will be once the moves in flight are saved.
  const [local, setLocal] = useState<Layout | null>(null);
  const pending = useRef(0);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const [drag, setDrag] = useState<Drag>(null);
  const [over, setOver] = useState<Over>(null);
  const [sheet, setSheet] = useState(false);
  const [publishing, setPublishing] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  useShellTitle(data ? `Build · ${data.course.title}` : null);

  const stored = useMemo(() => (data ? layoutOf(data.chapters, data.lessons) : { chapters: [] }), [data]);
  const layout = local ?? stored;
  const chapterOf = useMemo(() => new Map((data?.chapters ?? []).map((c) => [c.id, c])), [data]);
  const lessonOf = useMemo(() => new Map((data?.lessons ?? []).map((l) => [l.id, l])), [data]);
  const issues = useMemo(() => (data ? publishIssues(data.lessons, new Map(Object.entries(data.quizzes))) : []), [data]);

  const select = (id: string | null) => {
    const next = new URLSearchParams(sp);
    if (id) next.set("lesson", id);
    else next.delete("lesson");
    setSp(next, { replace: true });
    setSheet(false);
  };
  const patchData = (f: (d: Data) => Data) => qc.setQueryData<Data>(KEY(course), (d) => d && f(d));

  /** Shows a move at once and saves it after the moves before it: each edit goes against the
   *  version the server last gave, and a refusal puts the outline back to what it holds. */
  const commit = useCallback(
    (next: Layout) => {
      setLocal(next);
      pending.current++;
      queue.current = queue.current.then(async () => {
        try {
          const cur = qc.getQueryData<Data>(KEY(course));
          if (!cur) return;
          const patches = patchesFor(next, cur.chapters, cur.lessons);
          if (!patches.length) return;
          const done = await Promise.all(patches.map(async (p) => ({ ...p, version: (await api.patch<{ version: number }>(docPath(p.entity, p.id), p.body, { ifMatch: p.version })).version })));
          qc.setQueryData<Data>(KEY(course), (d) => d && { ...d, chapters: applyPatches(d.chapters, done.filter((x) => x.entity === "LmsChapter")), lessons: applyPatches(d.lessons, done.filter((x) => x.entity === "LmsLesson")) });
          // An editor open on a moved lesson keeps up with its new version.
          for (const x of done) if (x.entity === "LmsLesson") qc.setQueryData<LessonDoc>(docKey(x.id), (d) => d && { ...d, ...x.body, version: x.version });
        } catch (e) {
          toast(errorMessage(e), "error");
          await qc.invalidateQueries({ queryKey: KEY(course) });
        } finally {
          if (--pending.current === 0) setLocal(null);
        }
      });
    },
    [qc, course],
  );

  const moving = local !== null;

  // ── edits that are not moves ──

  /** One edit of a row, after the edits before it: moves and toggles share one line, so each goes
   *  against the version the one before it left. */
  const patchRow = (entity: "LmsChapter" | "LmsLesson", id: string, body: Record<string, unknown>) => {
    queue.current = queue.current.then(async () => {
      const rows = qc.getQueryData<Data>(KEY(course));
      const version = (entity === "LmsChapter" ? rows?.chapters : rows?.lessons)?.find((r) => r.id === id)?.version;
      try {
        const saved = await api.patch<{ version: number }>(docPath(entity, id), body, { ifMatch: version });
        patchData((d) => (entity === "LmsChapter" ? { ...d, chapters: applyPatches(d.chapters, [{ id, version: saved.version, body }]) } : { ...d, lessons: applyPatches(d.lessons, [{ id, version: saved.version, body }]) }));
        if (entity === "LmsLesson") qc.setQueryData<LessonDoc>(docKey(id), (d) => d && { ...d, ...body, version: saved.version });
      } catch (e) {
        toast(errorMessage(e), "error");
        await qc.invalidateQueries({ queryKey: KEY(course) });
      }
    });
  };

  const addChapter = async () => {
    setBusy(true);
    try {
      await api.post(docPath("LmsChapter"), { course, title: "New chapter" });
      await qc.invalidateQueries({ queryKey: KEY(course) });
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const addLesson = async (chapter: string) => {
    setBusy(true);
    try {
      const made = await api.post<{ id: string }>(docPath("LmsLesson"), { chapter, title: "Untitled lesson", lesson_type: "Article" });
      await qc.invalidateQueries({ queryKey: KEY(course) });
      select(made.id);
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const deleteChapter = async (id: string) => {
    const c = chapterOf.get(id);
    const ok = await confirmDialog({ title: `Delete "${c?.title ?? "this chapter"}"?`, message: "A chapter with lessons cannot be deleted: move or delete its lessons first.", confirmLabel: "Delete chapter", danger: true });
    if (!ok) return;
    try {
      await api.del(docPath("LmsChapter", id));
      await qc.invalidateQueries({ queryKey: KEY(course) });
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };

  // ── publishing ──

  const setStatus = async (status: "Published" | "Draft") => {
    if (!data) return;
    setBusy(true);
    try {
      const saved = await api.patch<CourseDoc>(docPath("LmsCourse", course), { status }, { ifMatch: data.course.version });
      patchData((d) => ({ ...d, course: { ...d.course, status: saved.status, version: saved.version } }));
      setPublishing(null);
      toast(status === "Published" ? "Published. Learners can find it in the catalog." : "Back to draft");
      void qc.invalidateQueries({ queryKey: ["lms", "catalog"] });
    } catch (e) {
      // The server's own reasons, in its words.
      setPublishing(e instanceof ApiError && e.fields.length ? e.fields.map((f) => f.message) : [errorMessage(e)]);
    } finally {
      setBusy(false);
    }
  };

  // ── dragging ──

  const edgeOf = (e: DragEvent): Edge => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientY < r.top + r.height / 2 ? "before" : "after";
  };
  const hover = (kind: "lesson" | "chapter", id: string) => (e: DragEvent) => {
    if (!drag) return;
    // A lesson lands among lessons and on chapters; a chapter among chapters.
    if (drag.kind === "chapter" && kind === "lesson") return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "move";
    const edge = edgeOf(e);
    setOver((o) => (o && o.kind === kind && o.id === id && o.edge === edge ? o : { kind, id, edge }));
  };
  const drop = (kind: "lesson" | "chapter", id: string) => (e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const d = drag;
    const edge = edgeOf(e);
    setDrag(null);
    setOver(null);
    if (!d || moving) return;
    if (d.kind === "chapter" && kind === "chapter") commit(dropChapter(layout, d.id, id, edge));
    else if (d.kind === "lesson" && kind === "lesson") commit(dropLesson(layout, d.id, id, edge));
    else if (d.kind === "lesson" && kind === "chapter") commit(dropLessonOnChapter(layout, d.id, id));
  };
  const dragProps = (kind: "lesson" | "chapter", id: string) => ({
    draggable: !moving,
    onDragStart: (e: DragEvent) => {
      e.stopPropagation();
      e.dataTransfer.setData("text/plain", id);
      e.dataTransfer.effectAllowed = "move";
      setDrag({ kind, id });
    },
    onDragEnd: () => {
      setDrag(null);
      setOver(null);
    },
    onDragOver: hover(kind, id),
    onDrop: drop(kind, id),
    "data-dragging": drag?.id === id || undefined,
    "data-drag": drag?.kind,
    "data-drop": over?.kind === kind && over.id === id && drag?.id !== id ? over.edge : undefined,
  });

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not open the course" />;
  if (!data) return <PageLoading label="Opening the course" />;

  const total = data.lessons.length;
  const outline = (
    <div className="lms-b-outline">
      <div className="lms-b-outline-head">
        <h2>Outline</h2>
        <span className="faint">
          {data.chapters.length} {data.chapters.length === 1 ? "chapter" : "chapters"} · {total} {total === 1 ? "lesson" : "lessons"}
        </span>
      </div>
      {!layout.chapters.length && (
        <p className="muted lms-b-empty">A course is chapters of lessons. Start with a chapter.</p>
      )}
      {layout.chapters.map((c, ci) => {
        const chapter = chapterOf.get(c.id);
        if (!chapter) return null;
        return (
          <section key={c.id} className="lms-b-chapter" aria-label={chapter.title} {...dragProps("chapter", c.id)}>
            <div className="lms-b-chapter-head">
              <span className="lms-grip" aria-hidden="true" title="Drag to move the chapter">
                <Glyph name="grip" size={14} />
              </span>
              <Rename value={chapter.title} label={`Name of chapter ${ci + 1}`} onSave={(title) => patchRow("LmsChapter", c.id, { title })} />
              <span className="lms-b-moves">
                <button type="button" className="icon-btn icon-btn-sm" aria-label={`Move ${chapter.title} up`} disabled={ci === 0 || moving} onClick={() => commit(shiftChapter(layout, c.id, -1))}>
                  <Icon name="up" size={14} />
                </button>
                <button type="button" className="icon-btn icon-btn-sm" aria-label={`Move ${chapter.title} down`} disabled={ci === layout.chapters.length - 1 || moving} onClick={() => commit(shiftChapter(layout, c.id, 1))}>
                  <Icon name="down" size={14} />
                </button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button type="button" className="icon-btn icon-btn-sm" aria-label={`More for ${chapter.title}`}>
                      <Glyph name="more" size={14} />
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem disabled={busy || moving} onSelect={() => void addLesson(c.id)}>
                      Add a lesson
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem disabled={busy || moving} onSelect={() => void deleteChapter(c.id)}>
                      Delete chapter…
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </span>
            </div>
            <ul className="lms-b-lessons">
              {c.lessons.map((id, li) => {
                const l = lessonOf.get(id);
                if (!l) return null;
                const active = id === selected;
                return (
                  <li key={id} className="lms-b-lesson" data-active={active || undefined} {...dragProps("lesson", id)}>
                    <div className="lms-b-lesson-main">
                      <span className="lms-grip" aria-hidden="true" title="Drag to move the lesson">
                        <Glyph name="grip" size={14} />
                      </span>
                      <button type="button" className="lms-b-open" aria-current={active ? "true" : undefined} onClick={() => select(id)}>
                        <Glyph name={lessonGlyph(l.lesson_type)} size={14} />
                        <span className="lms-b-title">{l.title}</span>
                      </button>
                      <span className="lms-b-moves">
                        <button type="button" className="icon-btn icon-btn-sm" aria-label={`Move ${l.title} up`} disabled={(ci === 0 && li === 0) || moving} onClick={() => commit(shiftLesson(layout, id, -1))}>
                          <Icon name="up" size={14} />
                        </button>
                        <button
                          type="button"
                          className="icon-btn icon-btn-sm"
                          aria-label={`Move ${l.title} down`}
                          disabled={(ci === layout.chapters.length - 1 && li === c.lessons.length - 1) || moving}
                          onClick={() => commit(shiftLesson(layout, id, 1))}
                        >
                          <Icon name="down" size={14} />
                        </button>
                        {layout.chapters.length > 1 && (
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button type="button" className="icon-btn icon-btn-sm" aria-label={`Move ${l.title} to another chapter`}>
                                <Glyph name="more" size={14} />
                              </button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end">
                              {layout.chapters
                                .filter((o) => o.id !== c.id)
                                .map((o) => (
                                  <DropdownMenuItem key={o.id} disabled={moving} onSelect={() => commit(moveLesson(layout, id, o.id, o.lessons.length))}>
                                    Move to {chapterOf.get(o.id)?.title}
                                  </DropdownMenuItem>
                                ))}
                            </DropdownMenuContent>
                          </DropdownMenu>
                        )}
                      </span>
                    </div>
                    <div className="lms-b-lesson-meta">
                      <span className="faint">
                        {LESSON_LABEL[l.lesson_type]}
                        {l.duration_minutes ? ` · ${duration(l.duration_minutes)}` : ""}
                      </span>
                      <span className="lms-b-flags">
                        <button type="button" aria-pressed={l.is_preview} disabled={moving} title="Free preview: anyone can open it before enrolling" onClick={() => patchRow("LmsLesson", id, { is_preview: !l.is_preview })}>
                          Preview
                        </button>
                        <button type="button" aria-pressed={l.required} disabled={moving} title="Required: counts towards finishing the course" onClick={() => patchRow("LmsLesson", id, { required: !l.required })}>
                          Required
                        </button>
                      </span>
                    </div>
                  </li>
                );
              })}
              {!c.lessons.length && <li className="lms-b-none faint">No lessons in this chapter yet.</li>}
            </ul>
            <button type="button" className="lms-b-add" disabled={busy || moving} onClick={() => void addLesson(c.id)}>
              <Icon name="plus" size={13} /> Add a lesson
            </button>
          </section>
        );
      })}
      <button type="button" className="btn btn-default btn-md lms-b-add-chapter" disabled={busy || moving} onClick={() => void addChapter()}>
        <Icon name="plus" size={14} /> Add a chapter
      </button>
    </div>
  );

  const status = data.course.status;
  const slug = data.course.slug;
  return (
    <div className="lms-build-page">
      <header className="lms-build-bar">
        <div className="lms-build-title">
          <nav className="crumbs" aria-label="Breadcrumb">
            <Link to={paths.teaching()}>Teaching</Link>
            <Glyph name="chevronRight" size={12} />
            <Link to={paths.teaching(course)}>{data.course.title}</Link>
          </nav>
          <h1 className="page-title">
            {data.course.title} <Badge tone={status === "Published" ? "green" : statusTone(status)}>{status}</Badge>
          </h1>
        </div>
        <div className="lms-tools">
          <Button className="lms-outline-toggle" icon="list" onClick={() => setSheet(true)}>
            Outline
          </Button>
          <Link className="btn btn-default btn-md" to={paths.record("LmsCourse", course)}>
            Settings
          </Link>
          <Link className="btn btn-default btn-md" to={paths.course(slug)}>
            <Glyph name="eye" size={14} /> Course page
          </Link>
          {status === "Published" ? (
            <Button loading={busy} onClick={() => void setStatus("Draft")}>
              Back to draft
            </Button>
          ) : (
            <Button variant="primary" loading={busy} onClick={() => setPublishing(issues.map((i) => i.text))}>
              Publish{issues.length ? ` (${issues.length} to fix)` : ""}
            </Button>
          )}
        </div>
      </header>
      <div className="lms-build">
        <div className="lms-build-main">
          {selected && lessonOf.has(selected) ? (
            <LessonEditor
              key={selected}
              id={selected}
              course={course}
              slug={slug}
              onChanged={(saved) => patchData((d) => ({ ...d, lessons: d.lessons.map((l) => (l.id === saved.id ? { ...l, ...saved } : l)) }))}
              onDeleted={() => {
                select(null);
                void qc.invalidateQueries({ queryKey: KEY(course) });
              }}
            />
          ) : (
            <EmptyState title={total ? "Choose a lesson to write" : "No lessons yet"} icon="edit">
              <p className="muted">{total ? "Pick one in the outline, or add another." : data.chapters.length ? "Add a lesson to a chapter." : "Add a chapter, then a lesson in it."}</p>
              {!data.chapters.length && (
                <Button variant="primary" loading={busy} onClick={() => void addChapter()}>
                  Add a chapter
                </Button>
              )}
            </EmptyState>
          )}
        </div>
        <aside className="lms-build-side" aria-label="Course outline">
          {outline}
        </aside>
      </div>
      <Sheet open={sheet} onClose={() => setSheet(false)} title="Outline">
        {outline}
      </Sheet>
      <Dialog
        open={publishing !== null}
        onClose={() => setPublishing(null)}
        title={publishing && publishing.length ? "Before this course goes out" : "Publish this course?"}
        footer={
          <>
            <Button onClick={() => setPublishing(null)}>{publishing && publishing.length ? "Close" : "Not yet"}</Button>
            {(!publishing || !publishing.length) && (
              <Button variant="primary" loading={busy} onClick={() => void setStatus("Published")}>
                Publish
              </Button>
            )}
          </>
        }
      >
        {publishing && publishing.length ? (
          <>
            <p className="muted">These need fixing first:</p>
            <ul className="lms-plain lms-issues">
              {publishing.map((t) => {
                const issue = issues.find((i) => i.text === t);
                return (
                  <li key={t}>
                    {issue?.lesson ? (
                      <button type="button" className="lms-linklike" onClick={() => {
                          select(issue.lesson!);
                          setPublishing(null);
                        }}>
                        {t}
                      </button>
                    ) : (
                      t
                    )}
                  </li>
                );
              })}
            </ul>
          </>
        ) : (
          <>
            <Alert tone="success">Everything the course needs is in place.</Alert>
            <p className="muted">Learners find published courses in the catalog and can enrol. You can take it back to draft at any time.</p>
          </>
        )}
      </Dialog>
    </div>
  );
}
