// The course player: where learners spend their time. The lesson on the left, the course's outline
// beside it (a sheet on a phone), and at the foot of every lesson one obvious next step: finishing a
// reading, a video or a document and moving on is a single click, and quizzes and assignments
// finish when they are passed. Opening a lesson makes it the place a learner picks up from, so
// "Continue" anywhere leads to the first lesson still to do. The server decides what is done; the
// bar and ticks follow what it says at once.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router";
import { ApiError, errorMessage } from "@ferrum/client";
import { Alert, Button, EmptyState, ErrorState, PageLoading, toast, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { completeLesson, getCourse, getLesson, keys, reactivate } from "./api";
import { Done } from "./Done";
import { Glyph } from "./Glyph";
import { paths } from "./lesson";
import { LessonPage } from "./LessonPage";
import { OutlinePanel } from "./OutlinePanel";
import { around, findLesson, markDone, nextAction, nextUp, resume } from "./outline";
import { Sheet, useFocusMode } from "./parts";
import type { CoursePage, LessonView } from "./types";

export function PlayerRoute() {
  const { slug = "", lesson } = useParams();
  return <Player key={slug} slug={slug} lessonId={lesson} />;
}

/** True where the keyboard is being used to write or to move through something of its own. */
function typing(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest("input, textarea, select, pre, [contenteditable], [role=slider], dialog[open]");
}

function Player({ slug, lessonId }: { slug: string; lessonId?: string }) {
  useFocusMode();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const course = useQuery({ queryKey: keys.course(slug), queryFn: () => getCourse(slug), staleTime: 0 });
  const data = course.data;
  const progress = data?.my_enrollment;
  const learning = !!progress && progress.status !== "Withdrawn";
  const author = !!data?.is_author;
  const visitor = !learning && !author;
  const [sheet, setSheet] = useState(false);
  const [celebrate, setCelebrate] = useState(false);
  const [sitting, setSitting] = useState(false);

  const lessonQ = useQuery({ queryKey: keys.lesson(lessonId ?? ""), queryFn: () => getLesson(lessonId!), enabled: !!lessonId && !!data, staleTime: 0 });
  const view = lessonQ.data;

  /** Something may have changed what is done: a passed quiz, a graded assignment. */
  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: keys.course(slug) });
    if (lessonId) void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
    void qc.invalidateQueries({ queryKey: keys.dashboard });
  }, [qc, slug, lessonId]);

  // What happens to the enrolment elsewhere (an instructor marking lessons done) shows here too.
  // Opening a lesson itself changes the enrolment, so only the course reloads, never the lesson.
  const enrollment = progress?.enrollment;
  useLiveEntity(
    "LmsEnrollment",
    useCallback(
      (e) => {
        if (e.id === enrollment) void qc.invalidateQueries({ queryKey: keys.course(slug) });
      },
      [enrollment, qc, slug],
    ),
  );
  useLiveEntity(
    "LmsAssignmentSubmission",
    useCallback(() => {
      if (lessonId) void qc.invalidateQueries({ queryKey: keys.lesson(lessonId) });
    }, [qc, lessonId]),
  );

  // Finishing the course is said once, whichever lesson did it (the last reading, a passed quiz).
  const status = progress?.status;
  const was = useRef(status);
  useEffect(() => {
    if (was.current === "Active" && status === "Completed") setCelebrate(true);
    was.current = status;
  }, [status]);

  const complete = useMutation({
    mutationFn: (lesson: string) => completeLesson(enrollment!, lesson),
    onSuccess: (s, lesson) => {
      qc.setQueryData<CoursePage>(keys.course(slug), (cur) => {
        if (!cur) return cur;
        const chapters = markDone(cur.chapters, lesson);
        return {
          ...cur,
          chapters,
          next_lesson: nextUp(chapters),
          my_enrollment: cur.my_enrollment && {
            ...cur.my_enrollment,
            status: s.status,
            progress_percent: s.progress_percent,
            completed_lessons: s.completed_lessons,
            total_lessons: s.total_lessons,
            certificate: s.certificate,
          },
        };
      });
      qc.setQueryData<LessonView>(keys.lesson(lesson), (v) => v && { ...v, completed: true });
      void qc.invalidateQueries({ queryKey: keys.dashboard });
    },
    onError: (e) => toast(errorMessage(e), "error"),
  });
  const again = useMutation({
    mutationFn: () => reactivate(enrollment!),
    onSuccess: refresh,
    onError: (e) => toast(errorMessage(e), "error"),
  });

  const here = data ? findLesson(data.chapters, lessonId) : undefined;
  const nav = data ? around(data.chapters, lessonId) : null;
  const action = data && view ? nextAction(data.chapters, view.lesson, view.completed, learning) : null;

  const advance = async () => {
    if (!view || !action) return;
    if (action.completes) {
      try {
        await complete.mutateAsync(view.lesson.id);
      } catch {
        return;
      }
    }
    if (action.target) navigate(paths.learn(slug, action.target.id));
    else if (!action.completes) navigate(paths.course(slug));
    else if (!celebrate) toast("Lesson complete");
  };

  // Left and Right move between lessons, unless the keys are doing something else.
  const prevId = nav?.prev?.lesson.id;
  const nextId = nav?.next?.lesson.id;
  const kind = view?.lesson.lesson_type;
  useEffect(() => {
    if (kind === "Quiz" || kind === "Assignment") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || typing(e.target)) return;
      const to = e.key === "ArrowLeft" ? prevId : e.key === "ArrowRight" ? nextId : undefined;
      if (to) navigate(paths.learn(slug, to));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [kind, prevId, nextId, slug, navigate]);

  // A new lesson starts at its top, and a screen reader hears its title.
  const heading = useRef<HTMLHeadingElement>(null);
  const shown = view?.lesson.id;
  useEffect(() => {
    if (!shown) return;
    window.scrollTo(0, 0);
    heading.current?.focus({ preventScroll: true });
  }, [shown]);

  useShellTitle(data ? (view ? `${view.lesson.title} · ${data.title}` : data.title) : null);

  if (course.isLoading) return <PageLoading label="Opening the course" />;
  if (course.error || !data) return <ErrorState error={course.error} onRetry={() => void course.refetch()} title="Could not open the course" />;

  if (!lessonId) {
    const target = learning ? (data.next_lesson ?? resume(data.chapters)) : author ? resume(data.chapters) : null;
    return <Navigate to={target ? paths.learn(slug, target.id) : paths.course(slug)} replace />;
  }

  const outline = (
    <OutlinePanel
      href={(l) => paths.learn(slug, l.id)}
      chapters={data.chapters}
      currentId={lessonId}
      progress={progress}
      visitor={visitor}
      footer={
        progress?.certificate && learning ? (
          <p className="lms-outline-cert">
            <Glyph name="award" size={13} /> <Link to={paths.course(slug)}>Your certificate</Link>
          </p>
        ) : undefined
      }
    />
  );

  const left = (() => {
    if (lessonQ.error) {
      const missing = lessonQ.error instanceof ApiError && lessonQ.error.status === 404;
      if (missing && visitor)
        return (
          <EmptyState title="This lesson is part of the full course" icon="user">
            <p className="muted">Enrol to open every lesson and keep track of your progress.</p>
            <Link className="btn btn-primary btn-md" to={paths.course(slug)}>
              See the course
            </Link>
          </EmptyState>
        );
      return missing ? (
        <EmptyState title="This lesson is not available">
          <Link className="btn btn-default btn-md" to={paths.course(slug)}>
            Back to the course
          </Link>
        </EmptyState>
      ) : (
        <ErrorState error={lessonQ.error} onRetry={() => void lessonQ.refetch()} title="Could not open the lesson" />
      );
    }
    if (!view || !action || !nav) return <PageLoading label="Opening the lesson" />;
    const prev = nav.prev?.lesson;
    const kind = view.lesson.lesson_type;
    // Quizzes and assignments finish when passed; everything else, by the button, or the arrow keys.
    const hint =
      kind === "Quiz" || kind === "Assignment"
        ? !view.completed && learning && (kind === "Quiz" ? "Passing the quiz completes this lesson." : "A passing grade completes this lesson.")
        : "Tip: the arrow keys move between lessons.";
    return (
      <LessonPage view={view} chapterTitle={here?.chapter.title} learning={learning} enrollment={enrollment} onProgress={refresh} onSitting={setSitting} headingRef={heading}>
        {!sitting && (
          <>
            <nav className="lms-lesson-nav" aria-label="Lesson navigation">
              {prev ? (
                <Link className="btn btn-default btn-md" to={paths.learn(slug, prev.id)} rel="prev">
                  <Glyph name="arrowLeft" size={14} /> Previous
                </Link>
              ) : (
                <span />
              )}
              <span className="faint lms-pos">
                Lesson {nav.index + 1} of {nav.count}
              </span>
              {action.completes || action.target ? (
                <Button variant={action.primary ? "primary" : "default"} loading={complete.isPending} onClick={() => void advance()}>
                  {action.label} <Glyph name="arrowRight" size={14} />
                </Button>
              ) : (
                <Link className="btn btn-default btn-md" to={paths.course(slug)}>
                  {action.label}
                </Link>
              )}
            </nav>
            {hint && <p className="faint lms-hint">{hint}</p>}
          </>
        )}
      </LessonPage>
    );
  })();

  return (
    <div className="lms-player">
      <div className="lms-player-main">
        <div className="lms-player-top">
          <Link className="lms-back" to={paths.course(slug)}>
            <Glyph name="arrowLeft" size={14} /> {data.title}
          </Link>
          <Button size="sm" className="lms-outline-toggle" icon="list" onClick={() => setSheet(true)}>
            Outline
          </Button>
        </div>
        {celebrate && <Done title={data.title} certificate={progress?.certificate} onClose={() => setCelebrate(false)} />}
        {progress?.status === "Withdrawn" && !author && (
          <Alert tone="warning" action={<Button loading={again.isPending} onClick={() => again.mutate()}>Enrol again</Button>}>
            You left this course. Your progress is kept if you enrol again.
          </Alert>
        )}
        {visitor && progress?.status !== "Withdrawn" && (
          <Alert tone="info" action={<Link className="btn btn-primary btn-md" to={paths.course(slug)}>See the course</Link>}>
            You are looking at a free preview. Enrol to open every lesson and keep your progress.
          </Alert>
        )}
        {author && !learning && (
          <Alert tone="info" action={data.id ? <Link className="btn btn-default btn-md" to={paths.build(data.id, lessonId)}>Edit in the builder</Link> : undefined}>
            You are looking at this course as its instructor. Nothing you do here counts as progress.
          </Alert>
        )}
        {left}
      </div>
      <aside className="lms-outline" aria-label="Course outline">
        {outline}
      </aside>
      <Sheet open={sheet} onClose={() => setSheet(false)} title="Course outline">
        <div onClick={(e) => (e.target as Element).closest("a") && setSheet(false)}>{outline}</div>
      </Sheet>
    </div>
  );
}

