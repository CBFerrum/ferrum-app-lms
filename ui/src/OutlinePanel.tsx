// The course's outline beside the lesson: chapters that fold, a tick on each finished lesson, the
// open one marked, and a lock where a lesson is not yet the viewer's to open. It is a list of
// links, so it works with a keyboard and a screen reader like any other page.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { Badge } from "@ferrum/kit";
import { duration } from "./format";
import { Glyph, lessonGlyph, LESSON_LABEL } from "./Glyph";
import { chapterCounts, progressText, totals } from "./outline";
import { ProgressLine } from "./parts";
import type { OutlineChapter, OutlineLesson, Progress } from "./types";

/** A big course starts with only the open lesson's chapter unfolded. */
const UNFOLDED = 8;

function Row({ href, lesson, current, reachable, visitor }: { href: string; lesson: OutlineLesson; current: boolean; reachable: boolean; visitor: boolean }) {
  const done = !!lesson.completed;
  const mark = done ? (
    <span className="lms-tick" data-state="done">
      <Glyph name="check" size={11} />
      <span className="sr-only">Completed. </span>
    </span>
  ) : !reachable ? (
    <span className="lms-tick" data-state="locked">
      <Glyph name="lock" size={11} />
      <span className="sr-only">Locked. </span>
    </span>
  ) : (
    <span className="lms-tick" aria-hidden="true" />
  );
  const body = (
    <>
      {mark}
      <span className="lms-row-title">{lesson.title}</span>
      <span className="lms-row-meta faint">
        <Glyph name={lessonGlyph(lesson.lesson_type)} size={12} />
        <span className="sr-only">{LESSON_LABEL[lesson.lesson_type]}. </span>
        {duration(lesson.duration_minutes)}
      </span>
      {visitor && lesson.is_preview && <Badge tone="blue">Free preview</Badge>}
      {!lesson.required && <Badge>Optional</Badge>}
    </>
  );
  return reachable ? (
    <Link className="lms-row" to={href} aria-current={current ? "page" : undefined} data-state={current ? "current" : done ? "done" : undefined}>
      {body}
    </Link>
  ) : (
    <span className="lms-row" data-state="locked" title="Enrol to open this lesson">
      {body}
    </span>
  );
}

export function OutlinePanel({
  href,
  chapters,
  currentId,
  progress,
  visitor,
  footer,
}: {
  /** Where a lesson opens. */
  href: (lesson: OutlineLesson) => string;
  chapters: OutlineChapter[];
  currentId?: string;
  progress?: Progress | null;
  /** Someone who may open only the free previews: the rest show a lock. */
  visitor: boolean;
  footer?: ReactNode;
}) {
  const sum = totals(chapters);
  const currentChapter = chapters.find((c) => c.lessons.some((l) => l.id === currentId))?.id;
  const [open, setOpen] = useState<Set<string>>(() => new Set(chapters.length <= UNFOLDED ? chapters.map((c) => c.id) : currentChapter ? [currentChapter] : []));
  // Going to a lesson in a folded chapter unfolds it.
  useEffect(() => {
    if (currentChapter) setOpen((o) => (o.has(currentChapter) ? o : new Set(o).add(currentChapter)));
  }, [currentChapter]);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.querySelector('[aria-current="page"]')?.scrollIntoView({ block: "nearest" });
  }, [currentId]);

  return (
    <div className="lms-outline-inner" ref={box}>
      <div className="lms-outline-head">
        <h2>Course content</h2>
        {progress && progress.status !== "Withdrawn" ? (
          <>
            <ProgressLine value={progress.progress_percent} label="Your progress in this course" />
            <p className="faint lms-outline-count">{progressText(progress.completed_lessons, progress.total_lessons, sum.lessons)}</p>
          </>
        ) : (
          <p className="faint lms-outline-count">
            {sum.lessons} {sum.lessons === 1 ? "lesson" : "lessons"}
            {sum.minutes ? ` · ${duration(sum.minutes)}` : ""}
          </p>
        )}
        {footer}
      </div>
      {chapters.map((c) => {
        const n = chapterCounts(c);
        return (
          <details
            key={c.id}
            className="lms-chapter"
            open={open.has(c.id)}
            onToggle={(e) => {
              const isOpen = e.currentTarget.open;
              setOpen((o) => (o.has(c.id) === isOpen ? o : isOpen ? new Set(o).add(c.id) : new Set([...o].filter((x) => x !== c.id))));
            }}
          >
            <summary>
              <Glyph name="chevronRight" size={12} />
              <span className="lms-chapter-title">{c.title}</span>
              <span className="faint lms-chapter-count">
                {n.done}/{n.total}
                <span className="sr-only"> lessons completed</span>
              </span>
            </summary>
            <div className="lms-rows">
              {c.lessons.map((l) => (
                <Row key={l.id} href={href(l)} lesson={l} current={l.id === currentId} reachable={!visitor || l.is_preview} visitor={visitor} />
              ))}
              {!c.lessons.length && <p className="faint lms-empty-chapter">No lessons yet.</p>}
            </div>
          </details>
        );
      })}
    </div>
  );
}

