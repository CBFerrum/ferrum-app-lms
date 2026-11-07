// My learning: where to go on with a course, what is due, the live classes coming up, and the
// certificates earned. One request (the server works out each course's next lesson), kept
// current as the learner's enrolments change.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { Link } from "react-router";
import { formatDate, relativeTime, todayIso, t, isolate } from "@ferrum/client";
import { Badge, EmptyState, ErrorState, PageLoading, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { getDashboard, keys } from "./api";
import { CertificateActions } from "./Certificate";
import { dueLabel, duration, plural, sessionStart } from "./format";
import { Glyph, LESSON_LABEL } from "./Glyph";
import { paths } from "./lesson";
import { Cover, LearnTabs, ProgressLine } from "./parts";
import type { DueAssignment, MyCourse } from "./types";

const DUE_TONE: Record<DueAssignment["status"], "neutral" | "blue" | "amber" | "red"> = {
  "Not Submitted": "neutral",
  Submitted: "blue",
  "Needs Revision": "amber",
  Failed: "red",
};
const DUE_TEXT: Record<DueAssignment["status"], string> = {
  get "Not Submitted"() { return t("Not handed in"); },
  get Submitted() { return t("Waiting for a grade"); },
  get "Needs Revision"() { return t("Needs changes"); },
  get Failed() { return t("Not passed"); },
};

function Hero({ c }: { c: MyCourse }) {
  const next = c.next_lesson;
  return (
    <section className="lms-hero" aria-label={t("Continue learning")}>
      <div className="lms-hero-cover">
        <Cover title={c.course.title} slug={c.course.slug} image={c.course.cover_image} />
      </div>
      <div className="lms-hero-body">
        <p className="lms-eyebrow">{t("Pick up where you left off")}</p>
        <h2>
          <Link to={paths.course(c.course.slug)}>{isolate(c.course.title)}</Link>
        </h2>
        {next && (
          <p className="muted lms-hero-next">
            <Glyph name="arrowRight" size={13} /> {t("Up next:")} <strong>{isolate(next.title)}</strong>
            <span className="faint">
              {isolate(" ")}
              · {isolate(LESSON_LABEL[next.lesson_type])}
              {next.duration_minutes ? ` · ${duration(next.duration_minutes)}` : ""}
            </span>
          </p>
        )}
        <ProgressLine value={c.progress_percent} label={t("Your progress in {title}", { title: c.course.title })} />
        <div className="row">
          <Link className="btn btn-primary btn-md" to={paths.learn(c.course.slug, next?.id)}>
            {t("Continue")}
          </Link>
          <span className="faint">{t("{completed_lessons} of {total_lessons} done", { completed_lessons: c.completed_lessons, total_lessons: c.total_lessons })}</span>
        </div>
      </div>
    </section>
  );
}

function CourseTile({ c }: { c: MyCourse }) {
  const done = c.status === "Completed";
  return (
    <article className="lms-card">
      <Cover title={c.course.title} slug={c.course.slug} image={c.course.cover_image} />
      <div className="lms-card-body">
        <h3 className="lms-card-title">
          <Link to={done ? paths.course(c.course.slug) : paths.learn(c.course.slug, c.next_lesson?.id)}>{isolate(c.course.title)}</Link>
        </h3>
        {done ? (
          <p className="lms-card-facts">
            <Badge tone="green">{t("Completed")}</Badge>
            {c.certificate && (
              <span className="lms-card-cert">
                <Glyph name="award" size={13} /> {t("Certificate")}
              </span>
            )}
          </p>
        ) : (
          <>
            {c.next_lesson && <p className="lms-card-summary">{t("Next: {title}", { title: c.next_lesson.title })}</p>}
            <ProgressLine value={c.progress_percent} label={t("Your progress in {title}", { title: c.course.title })} />
          </>
        )}
        <p className="lms-card-by faint">
          {c.cohort?.title ? `${c.cohort.title} · ` : ""}
          {c.last_activity_at ? t("Opened {value}", { value: relativeTime(c.last_activity_at) }) : t("Enrolled {value}", { value: formatDate(c.enrolled_on) })}
        </p>
      </div>
    </article>
  );
}

export function Home() {
  useShellTitle(t("My learning"));
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.dashboard, queryFn: getDashboard });
  useLiveEntity("LmsEnrollment", useCallback(() => void qc.invalidateQueries({ queryKey: keys.dashboard }), [qc]));
  const data = q.data;
  const slugOf = useMemo(() => new Map((data?.courses ?? []).map((c) => [c.course.id, c.course.slug])), [data]);
  const active = (data?.courses ?? []).filter((c) => c.status === "Active");
  const hero = active.find((c) => c.next_lesson) ?? active[0];
  const today = todayIso();

  const sub = data
    ? data.courses.length
      ? t("{value} · {length} completed", { value: plural(data.courses.length, "course"), length: data.courses.filter((c) => c.status === "Completed").length })
      : t("Nothing started yet")
    : " ";

  return (
    <div className="lms-page">
      <header className="lms-head">
        <div>
          <h1 className="sr-only">{t("My learning")}</h1>
          <LearnTabs />
          <p className="page-sub" aria-live="polite">
            {isolate(sub)}
          </p>
        </div>
        <Link className="btn btn-default btn-md" to={paths.catalog}>
          {t("Browse the catalog")}
        </Link>
      </header>
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title={t("Could not load your learning")} />
      ) : !data ? (
        <PageLoading label={t("Loading your learning")} />
      ) : !data.courses.length && !data.learning_paths.length ? (
        <EmptyState title={t("You have not started a course yet")} icon="board">
          <p className="muted">{t("Find something to learn in the catalog. Your progress, deadlines and certificates will gather here.")}</p>
          <Link className="btn btn-primary btn-md" to={paths.catalog}>
            {t("Browse the catalog")}
          </Link>
        </EmptyState>
      ) : (
        <>
          {hero && <Hero c={hero} />}
          {data.courses.length > 0 && (
            <section aria-label={t("Your courses")}>
              <h2 className="lms-group">{t("Your courses")}</h2>
              <div className="lms-grid">
                {data.courses.map((c) => (
                  <CourseTile key={c.enrollment} c={c} />
                ))}
              </div>
            </section>
          )}
          <div className="lms-two">
            {data.due_assignments.length > 0 && (
              <section aria-label={t("Assignments")}>
                <h2 className="lms-group">{t("Assignments")}</h2>
                <ul className="lms-lines">
                  {data.due_assignments.map((a) => {
                    const slug = slugOf.get(a.course.id);
                    return (
                      <li key={a.assignment} className="lms-line">
                        <div className="lms-line-main">
                          {slug ? <Link to={paths.learn(slug, a.lesson)}>{isolate(a.title)}</Link> : <span>{isolate(a.title)}</span>}
                          <span className="faint">{isolate(a.course.title)}</span>
                        </div>
                        <Badge tone={a.overdue && a.status !== "Submitted" ? "red" : DUE_TONE[a.status]}>{isolate(DUE_TEXT[a.status])}</Badge>
                        {a.due_date && <span className={a.overdue && a.status !== "Submitted" ? "lms-late lms-line-when" : "faint lms-line-when"}>{isolate(dueLabel(a.due_date, today))}</span>}
                      </li>
                    );
                  })}
                </ul>
              </section>
            )}
            {data.upcoming_sessions.length > 0 && (
              <section aria-label={t("Live sessions")}>
                <h2 className="lms-group">{t("Live sessions")}</h2>
                <ul className="lms-lines">
                  {data.upcoming_sessions.map((s) => (
                    <li key={s.session} className="lms-line">
                      <div className="lms-line-main">
                        <strong>{isolate(s.title)}</strong>
                        <span className="faint">
                          {isolate(sessionStart(s.starts_at, s.time_zone))} · {isolate(s.course_title)}
                        </span>
                      </div>
                      {s.join_url ? (
                        <a className="btn btn-default btn-sm" href={s.join_url} target="_blank" rel="noopener noreferrer">
                          {t("Join")}
                        </a>
                      ) : (
                        <span className="faint">{t("No link yet")}</span>
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
          {data.learning_paths.length > 0 && (
            <section aria-label={t("Learning paths")}>
              <h2 className="lms-group">{t("Learning paths")}</h2>
              <ul className="lms-lines">
                {data.learning_paths.map((p) => (
                  <li key={p.id} className="lms-line">
                    <div className="lms-line-main">
                      <strong>{isolate(p.title)}</strong>
                      <span className="faint">
                        {t("{completed} of {value} completed", { completed: p.completed, value: plural(p.courses, "course") })}
                      </span>
                    </div>
                    <div className="lms-line-bar">
                      <ProgressLine value={p.progress_percent} label={t("Your progress in {title}", { title: p.title })} />
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )}
          {data.certificates.length > 0 && (
            <section aria-label={t("Certificates")}>
              <h2 className="lms-group">{t("Certificates")}</h2>
              <ul className="lms-lines">
                {data.certificates.map((c) => (
                  <li key={c.id} className="lms-line">
                    <span className="lms-line-icon">
                      <Glyph name="award" size={18} />
                    </span>
                    <div className="lms-line-main">
                      <strong>{isolate(c.course_title)}</strong>
                      <span className="faint">{isolate(c.number ?? "")}</span>
                    </div>
                    <div className="row">
                      <CertificateActions id={c.id} code={c.verification_code} primary={false} />
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </>
      )}
    </div>
  );
}
