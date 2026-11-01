// The lesson editor of the course builder: the fields of one lesson, saved a moment after they
// stop changing (or at once with Ctrl+S), each against the version on screen. The kind of lesson
// decides which fields show. The server decides what is valid and says why; its reasons appear
// under the fields they are about, and nothing typed is lost while they do.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { api, ApiError, docPath, errorMessage, uploadFile } from "@ferrum/client";
import { Alert, Button, confirmDialog, ErrorState, Field, PageLoading, Spinner, toast } from "@ferrum/kit";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@ferrum/kit/primitives";
import { LinkInput } from "@ferrum/records";
import { Glyph, LESSON_LABEL } from "./Glyph";
import { paths } from "./lesson";
import { Md } from "./Md";
import { createSaver, type SaveState, type Saver } from "./saver";
import type { LessonType } from "./types";

export interface LessonDoc {
  id: string;
  version: number;
  title: string;
  chapter: string;
  position: number | null;
  lesson_type: LessonType;
  duration_minutes: number | null;
  is_preview: boolean;
  required: boolean;
  content: string | null;
  video_url: string | null;
  video_embed_url: string | null;
  attachment: string | null;
  quiz: string | null;
  assignment: string | null;
}

export const LESSON_TYPES: LessonType[] = ["Article", "Video", "Document", "Quiz", "Assignment"];
export const docKey = (id: string) => ["lms", "lesson-doc", id] as const;

const STATE_TEXT: Record<SaveState | "clean", string> = { clean: "All changes saved", idle: "All changes saved", waiting: "Saving…", saving: "Saving…", failed: "Could not save, trying again" };

function Check({ id, checked, onChange, children }: { id: string; checked: boolean; onChange: (v: boolean) => void; children: ReactNode }) {
  return (
    <label className="lms-check" htmlFor={id}>
      <input id={id} type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}

export function LessonEditor({ id, course, slug, onChanged, onDeleted }: { id: string; course: string; slug: string; onChanged: (saved: LessonDoc) => void; onDeleted: () => void }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: docKey(id), queryFn: () => api.get<LessonDoc>(docPath("LmsLesson", id)), staleTime: 0 });
  const doc = q.data;
  const docRef = useRef(doc);
  docRef.current = doc;
  const [draft, setDraft] = useState<Partial<LessonDoc>>({});
  const [state, setState] = useState<SaveState | "clean">("clean");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [problem, setProblem] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [view, setView] = useState<"write" | "preview">("write");
  const [uploading, setUploading] = useState(false);
  const saver = useRef<Saver<unknown> | null>(null);
  const unsaved = useRef<Record<string, unknown>>({});

  useEffect(() => {
    const s = createSaver<unknown>(
      async (changes) => {
        const batch = { ...unsaved.current, ...changes };
        try {
          // The version the server last gave, read now: a save right behind another cannot wait for a render.
          const version = (qc.getQueryData<LessonDoc>(docKey(id)) ?? docRef.current)?.version;
          const saved = await api.patch<LessonDoc>(docPath("LmsLesson", id), batch, { ifMatch: version });
          unsaved.current = {};
          setErrors({});
          setProblem(null);
          qc.setQueryData(docKey(id), saved);
          // What was typed since this went out stays; what went out is no longer a draft.
          setDraft((d) => Object.fromEntries(Object.entries(d).filter(([k, v]) => batch[k] !== v)));
          onChanged(saved);
        } catch (e) {
          if (e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 429 && e.status !== 408) {
            // Kept, and sent again with the next change, so a type and the link it needs go together.
            unsaved.current = batch;
            if (e.status === 409 || e.status === 412) setConflict(true);
            else {
              setErrors(Object.fromEntries(e.fields.map((f) => [f.field, f.message])));
              setProblem(e.fields.length ? null : e.message);
            }
            return;
          }
          throw e;
        }
      },
      setState,
    );
    saver.current = s;
    return () => {
      void s.flush().finally(() => s.stop());
      saver.current = null;
    };
    // One saver per lesson: the callbacks it calls are stable for as long as the editor is open.
  }, [id]);

  // Ctrl+S saves now.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void saver.current?.flush();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not open the lesson" />;
  if (!doc) return <PageLoading label="Opening the lesson" />;

  const value = <K extends keyof LessonDoc>(k: K): LessonDoc[K] => (k in draft ? (draft[k] as LessonDoc[K]) : doc[k]);
  const set = <K extends keyof LessonDoc>(k: K, v: LessonDoc[K]) => {
    setDraft((d) => ({ ...d, [k]: v }));
    saver.current?.add({ [k]: v });
  };
  const type = value("lesson_type");

  const reload = async () => {
    unsaved.current = {};
    setDraft({});
    setConflict(false);
    setErrors({});
    await q.refetch();
  };
  const remove = async () => {
    const ok = await confirmDialog({ title: `Delete "${doc.title}"?`, message: "This cannot be undone. A lesson that learners have completed cannot be deleted: make it optional instead.", confirmLabel: "Delete lesson", danger: true });
    if (!ok) return;
    try {
      unsaved.current = {};
      saver.current?.stop();
      await api.del(docPath("LmsLesson", id));
      onDeleted();
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const upload = async (file: File | undefined) => {
    if (!file) return;
    setUploading(true);
    try {
      set("attachment", (await uploadFile(file, { entity: "LmsLesson", id })).url);
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setUploading(false);
    }
  };

  return (
    <section className="lms-editor" aria-label="Lesson editor">
      <header className="lms-editor-head">
        <div>
          <p className="lms-eyebrow">{LESSON_LABEL[type]} lesson</p>
          <p className="faint lms-editor-state" aria-live="polite">
            {unsaved.current && Object.keys(unsaved.current).length ? "Not saved yet: see the notes below" : STATE_TEXT[state]}
          </p>
        </div>
        <div className="lms-tools">
          <Link className="btn btn-default btn-sm" to={paths.learn(slug, id)}>
            <Glyph name="eye" size={14} /> Preview
          </Link>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button type="button" className="icon-btn" aria-label="More for this lesson">
                <Glyph name="more" size={16} />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => void saver.current?.flush()}>Save now</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => void remove()}>Delete lesson…</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </header>
      {conflict && (
        <Alert tone="warning" action={<Button onClick={() => void reload()}>Reload the lesson</Button>}>
          Someone else changed this lesson. Reload it to see their version; your unsaved changes here will be dropped.
        </Alert>
      )}
      {problem && <Alert tone="error">{problem}</Alert>}
      <div className="stack lms-editor-fields">
        <Field label="Title" htmlFor="le-title" required error={errors.title}>
          <input id="le-title" className="input" value={value("title")} maxLength={200} onChange={(e) => set("title", e.target.value)} />
        </Field>
        <div className="lms-editor-row">
          <Field label="Kind of lesson" htmlFor="le-type" error={errors.lesson_type}>
            <select id="le-type" className="select" value={type} onChange={(e) => set("lesson_type", e.target.value as LessonType)}>
              {LESSON_TYPES.map((t) => (
                <option key={t} value={t}>
                  {LESSON_LABEL[t]}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Minutes it takes" htmlFor="le-min" error={errors.duration_minutes}>
            <input id="le-min" className="input num" type="number" min={0} max={100000} value={value("duration_minutes") ?? ""} onChange={(e) => set("duration_minutes", e.target.value === "" ? null : Number(e.target.value))} />
          </Field>
        </div>
        <div className="lms-editor-checks">
          <Check id="le-preview" checked={value("is_preview")} onChange={(v) => set("is_preview", v)}>
            Free preview <span className="faint">anyone can open it before enrolling</span>
          </Check>
          <Check id="le-required" checked={value("required")} onChange={(v) => set("required", v)}>
            Required <span className="faint">counts towards finishing the course</span>
          </Check>
        </div>
        {type === "Video" && (
          <Field
            label="Video link"
            htmlFor="le-video"
            required
            error={errors.video_url}
            hint={doc.video_embed_url && !errors.video_url ? "Plays in the lesson from the host's own player." : "A link on YouTube, Vimeo, Loom, Wistia or Google Drive, or a host your administrator allowed."}
          >
            <input id="le-video" className="input" type="url" inputMode="url" placeholder="https://" value={value("video_url") ?? ""} onChange={(e) => set("video_url", e.target.value || null)} />
          </Field>
        )}
        {type === "Document" && (
          <Field label="Document" htmlFor="le-file" required error={errors.attachment} hint="A file to open or download, or a link to a page.">
            <div className="lms-stack-sm">
              <div className="row">
                <label className="btn btn-default btn-md" aria-busy={uploading || undefined}>
                  <input id="le-file" className="sr-only" type="file" onChange={(e) => void upload(e.target.files?.[0])} />
                  {uploading ? <Spinner small /> : <Glyph name="upload" />}
                  {value("attachment") ? "Replace file" : "Upload a file"}
                </label>
                {value("attachment")?.startsWith("/files/") && <span className="faint">A file is attached.</span>}
              </div>
              <input className="input" type="url" aria-label="Or a link to a page" placeholder="Or paste a link: https://" value={value("attachment")?.startsWith("/files/") ? "" : (value("attachment") ?? "")} onChange={(e) => set("attachment", e.target.value || null)} />
            </div>
          </Field>
        )}
        {type === "Quiz" && (
          <Field label="Quiz" htmlFor="le-quiz" required error={errors.quiz} hint="One of this course's quizzes; each belongs to one lesson.">
            <div className="row">
              <div className="lms-grow">
                <LinkInput to="LmsQuiz" id="le-quiz" label="Quiz" value={value("quiz")} linkFilters={{ course }} onChange={(v) => set("quiz", v)} />
              </div>
              <Link className="btn btn-default btn-md" to={`/e/LmsQuiz/new?course=${course}`}>
                New quiz
              </Link>
            </div>
          </Field>
        )}
        {type === "Assignment" && (
          <Field label="Assignment" htmlFor="le-assignment" required error={errors.assignment} hint="One of this course's assignments; each belongs to one lesson.">
            <div className="row">
              <div className="lms-grow">
                <LinkInput to="LmsAssignment" id="le-assignment" label="Assignment" value={value("assignment")} linkFilters={{ course }} onChange={(v) => set("assignment", v)} />
              </div>
              <Link className="btn btn-default btn-md" to={`/e/LmsAssignment/new?course=${course}`}>
                New assignment
              </Link>
            </div>
          </Field>
        )}
        <div className="field">
          <div className="lms-editor-tabs" role="tablist" aria-label="Content">
            <span className="field-label">Text of the lesson</span>
            <span className="spacer" />
            {(["write", "preview"] as const).map((v) => (
              <button key={v} type="button" role="tab" aria-selected={view === v} onClick={() => setView(v)}>
                {v === "write" ? "Write" : "Preview"}
              </button>
            ))}
          </div>
          {view === "write" ? (
            <textarea
              id="le-content"
              className="textarea lms-content"
              rows={16}
              aria-label="Text of the lesson"
              placeholder="Write the lesson in Markdown: # headings, **bold**, lists, `code`, ```code blocks```, > quotes, [links](https://…)."
              value={value("content") ?? ""}
              onChange={(e) => set("content", e.target.value || null)}
            />
          ) : (
            <div className="lms-content-preview">{value("content") ? <Md source={value("content")} /> : <p className="muted">Nothing written yet.</p>}</div>
          )}
          {errors.content && <p className="field-error">{errors.content}</p>}
        </div>
      </div>
    </section>
  );
}
