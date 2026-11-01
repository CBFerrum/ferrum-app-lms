// A lesson on the page: where it sits, its heading, what it shows (a video in its host's player, a
// document to open, text), and the quiz or assignment it holds. The player puts the foot below it.

import { type ReactNode, type Ref } from "react";
import { Alert, Badge } from "@ferrum/kit";
import { AssignmentPanel } from "./Assignment";
import { duration } from "./format";
import { Glyph, LESSON_LABEL } from "./Glyph";
import { embedSrc, isExternal } from "./lesson";
import { Md } from "./Md";
import { QuizPanel } from "./QuizPlayer";
import type { LessonBody, LessonView } from "./types";

export function LessonVideo({ lesson }: { lesson: Pick<LessonBody, "title" | "video_embed_url"> & { video_url?: string | null } }) {
  const src = embedSrc(lesson.video_embed_url);
  if (!src) return <Alert tone="info">The video of this lesson is not available yet.</Alert>;
  return (
    <>
      <div className="lms-video">
        <iframe
          src={src}
          title={lesson.title}
          allow="fullscreen; picture-in-picture; encrypted-media"
          allowFullScreen
          referrerPolicy="strict-origin-when-cross-origin"
          sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox"
          loading="lazy"
        />
      </div>
      <p className="faint lms-video-note">
        Not playing? <a href={lesson.video_url || src} target="_blank" rel="noopener noreferrer">Open the video in a new tab</a>.
      </p>
    </>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function LessonDocument({ url }: { url: string }) {
  const outside = isExternal(url);
  return (
    <div className="lms-doc">
      <span className="lms-doc-icon">
        <Glyph name={outside ? "link" : "doc"} size={22} />
      </span>
      <div className="lms-doc-text">
        <strong>{outside ? "A page to read" : "Document"}</strong>
        <span className="faint">{outside ? hostOf(url) : "Opens in a new tab"}</span>
      </div>
      <a className="btn btn-primary btn-md" href={url} target="_blank" rel="noopener noreferrer">
        Open
      </a>
      {!outside && (
        <a className="btn btn-default btn-md" href={url} download>
          Download
        </a>
      )}
    </div>
  );
}

export function LessonPage({
  view,
  chapterTitle,
  learning,
  enrollment,
  onProgress,
  onSitting,
  headingRef,
  children,
}: {
  view: LessonView;
  chapterTitle?: string;
  /** The viewer is enrolled: quizzes and assignments run. */
  learning: boolean;
  enrollment?: string;
  onProgress: () => void;
  onSitting?: (on: boolean) => void;
  headingRef: Ref<HTMLHeadingElement>;
  children: ReactNode;
}) {
  const l = view.lesson;
  return (
    <article className="lms-lesson" aria-labelledby="lms-lesson-title">
      <header className="lms-lesson-head">
        <p className="lms-eyebrow">
          {[chapterTitle, LESSON_LABEL[l.lesson_type], duration(l.duration_minutes)].filter(Boolean).join(" · ")}
        </p>
        <h1 id="lms-lesson-title" ref={headingRef} tabIndex={-1}>
          {l.title}
        </h1>
        {view.completed && (
          <Badge tone="green" title={view.completed_at ?? undefined}>
            Completed
          </Badge>
        )}
      </header>
      {l.lesson_type === "Video" && <LessonVideo lesson={l} />}
      {l.lesson_type === "Document" &&
        (l.file_url ? <LessonDocument url={l.file_url} /> : <Alert tone="info">No document is attached to this lesson yet.</Alert>)}
      <Md source={l.content} className="lms-reading" />
      {l.lesson_type === "Quiz" &&
        (view.quiz && enrollment ? (
          <QuizPanel quiz={view.quiz} enrollment={enrollment} lessonId={l.id} onProgress={onProgress} onSitting={onSitting} />
        ) : (
          <Alert tone="info">{learning ? "The quiz of this lesson is not ready yet." : "Quizzes run for enrolled learners. Enrol yourself in the course to try this one."}</Alert>
        ))}
      {l.lesson_type === "Assignment" &&
        (view.assignment ? (
          <AssignmentPanel state={view.assignment} lessonId={l.id} onProgress={onProgress} />
        ) : (
          <Alert tone="info">{learning ? "The assignment of this lesson is not ready yet." : "Assignments are handed in by enrolled learners. Enrol yourself in the course to try this one."}</Alert>
        ))}
      {children}
    </article>
  );
}
