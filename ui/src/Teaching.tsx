// Teaching: the courses a person teaches (every course for an LMS manager), with how many are
// learning, how many have finished, how far along the rest are, and how much work waits to be
// graded. Each row leads to its learners and its builder.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { Link } from "react-router";
import { Badge, PageLoading, Stat, StatGrid, StatePanel, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { Failed } from "./states";
import { getTeaching, keys } from "./api";
import { paths } from "./lesson";
import { COURSE_TONE, LearnTabs, ProgressLine } from "./parts";
import type { TeachingCourse } from "./types";
import { t, isolate } from "@ferrum/client";

function Row({ c }: { c: TeachingCourse }) {
  return (
    <tr>
      <td>
        <Link className="cell-title lms-wrap" to={paths.teaching(c.id)}>
          {isolate(c.title)}
        </Link>
        <div>
          <Badge tone={COURSE_TONE[c.status]}>{isolate(c.status)}</Badge>
        </div>
      </td>
      <td className="num">{c.enrolled}</td>
      <td className="num">{c.active}</td>
      <td className="num">{c.completed}</td>
      <td className="lms-col-progress">
        <ProgressLine value={c.average_progress} label={t("Average progress in {title}", { title: c.title })} />
      </td>
      <td className="num">{c.submissions_to_grade > 0 ? <Badge tone="amber">{t("{submissions_to_grade} to grade", { submissions_to_grade: c.submissions_to_grade })}</Badge> : <span className="faint">{t("None")}</span>}</td>
      <td>
        <div className="lms-row-actions">
          <Link className="btn btn-default btn-sm" to={paths.teaching(c.id)}>
            {t("Learners")}
          </Link>
          <Link className="btn btn-default btn-sm" to={paths.build(c.id)}>
            {t("Build")}
          </Link>
          <Link className="btn btn-ghost btn-sm" to={paths.course(c.slug)}>
            {t("Open")}
          </Link>
        </div>
      </td>
    </tr>
  );
}

export function Teaching() {
  useShellTitle(t("Teaching"));
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.teaching, queryFn: getTeaching });
  const refresh = useCallback(() => void qc.invalidateQueries({ queryKey: keys.teaching }), [qc]);
  useLiveEntity("LmsEnrollment", refresh);
  useLiveEntity("LmsAssignmentSubmission", refresh);
  const courses = q.data ?? [];
  const learning = courses.reduce((n, c) => n + c.active, 0);
  const done = courses.reduce((n, c) => n + c.completed, 0);
  const toGrade = courses.reduce((n, c) => n + c.submissions_to_grade, 0);

  return (
    <div className="lms-page">
      <header className="lms-head">
        <div>
          <h1 className="sr-only">{t("Teaching")}</h1>
          <LearnTabs />
          <p className="page-sub" aria-live="polite">
            {q.data ? (courses.length === 1 ? t("1 course") : t("{n} courses", { n: courses.length })) : " "}
          </p>
        </div>
        <Link className="btn btn-primary btn-md" to={paths.record("LmsCourse", "new")}>
          {t("New course")}
        </Link>
      </header>
      {q.error ? (
        <Failed error={q.error} onRetry={() => void q.refetch()} title={t("Could not load your courses")} />
      ) : !q.data ? (
        <PageLoading label={t("Loading your courses")} />
      ) : !courses.length ? (
        <StatePanel
          kind="empty"
          title={t("You do not teach a course yet")}
          action={
            <Link className="btn btn-primary btn-md" to={paths.record("LmsCourse", "new")}>
              {t("New course")}
            </Link>
          }
        >
          {t("Create a course, add lessons, and publish it when it is ready for learners.")}
        </StatePanel>
      ) : (
        <>
          <StatGrid>
            <Stat value={learning} label={t("Learning now")} />
            <Stat value={done} label={t("Finished")} />
            <Stat value={toGrade} label={t("To grade")} />
          </StatGrid>
          <div className="lms-panel">
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">{t("Course")}</th>
                    <th scope="col" className="num">
                      {t("Learners")}
                    </th>
                    <th scope="col" className="num">
                      {t("Learning")}
                    </th>
                    <th scope="col" className="num">
                      {t("Finished")}
                    </th>
                    <th scope="col">{t("Average progress")}</th>
                    <th scope="col" className="num">
                      {t("Grading")}
                    </th>
                    <th scope="col">
                      <span className="sr-only">{t("Open")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {courses.map((c) => (
                    <Row key={c.id} c={c} />
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
