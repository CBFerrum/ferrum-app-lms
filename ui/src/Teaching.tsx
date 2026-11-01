// Teaching: the courses a person teaches (every course for an LMS manager), with how many are
// learning, how many have finished, how far along the rest are, and how much work waits to be
// graded. Each row leads to its learners and its builder.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { Link } from "react-router";
import { Badge, EmptyState, ErrorState, PageLoading, statusTone, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { getTeaching, keys } from "./api";
import { plural } from "./format";
import { paths } from "./lesson";
import { LearnTabs, ProgressLine } from "./parts";
import type { TeachingCourse } from "./types";

function Figure({ value, label }: { value: number | string; label: string }) {
  return (
    <div className="lms-figure">
      <strong>{value}</strong>
      <span className="faint">{label}</span>
    </div>
  );
}

function Row({ c }: { c: TeachingCourse }) {
  return (
    <tr>
      <td>
        <Link className="cell-title lms-wrap" to={paths.teaching(c.id)}>
          {c.title}
        </Link>
        <div>
          <Badge tone={c.status === "Published" ? "green" : statusTone(c.status)}>{c.status}</Badge>
        </div>
      </td>
      <td className="num">{c.enrolled}</td>
      <td className="num">{c.active}</td>
      <td className="num">{c.completed}</td>
      <td className="lms-col-progress">
        <ProgressLine value={c.average_progress} label={`Average progress in ${c.title}`} />
      </td>
      <td className="num">{c.submissions_to_grade > 0 ? <Badge tone="amber">{c.submissions_to_grade} to grade</Badge> : <span className="faint">None</span>}</td>
      <td>
        <div className="lms-row-actions">
          <Link className="btn btn-default btn-sm" to={paths.teaching(c.id)}>
            Learners
          </Link>
          <Link className="btn btn-default btn-sm" to={paths.build(c.id)}>
            Build
          </Link>
          <Link className="btn btn-ghost btn-sm" to={paths.course(c.slug)}>
            Open
          </Link>
        </div>
      </td>
    </tr>
  );
}

export function Teaching() {
  useShellTitle("Teaching");
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
          <h1 className="sr-only">Teaching</h1>
          <LearnTabs />
          <p className="page-sub" aria-live="polite">
            {q.data ? plural(courses.length, "course") : " "}
          </p>
        </div>
        <Link className="btn btn-primary btn-md" to={paths.record("LmsCourse", "new")}>
          New course
        </Link>
      </header>
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not load your courses" />
      ) : !q.data ? (
        <PageLoading label="Loading your courses" />
      ) : !courses.length ? (
        <EmptyState title="You do not teach a course yet" icon="board">
          <p className="muted">Create a course, add lessons, and publish it when it is ready for learners.</p>
          <Link className="btn btn-primary btn-md" to={paths.record("LmsCourse", "new")}>
            New course
          </Link>
        </EmptyState>
      ) : (
        <>
          <div className="lms-figures">
            <Figure value={learning} label="learning now" />
            <Figure value={done} label="finished" />
            <Figure value={toGrade} label="to grade" />
          </div>
          <div className="lms-panel">
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Course</th>
                    <th scope="col" className="num">
                      Learners
                    </th>
                    <th scope="col" className="num">
                      Learning
                    </th>
                    <th scope="col" className="num">
                      Finished
                    </th>
                    <th scope="col">Average progress</th>
                    <th scope="col" className="num">
                      Grading
                    </th>
                    <th scope="col">
                      <span className="sr-only">Open</span>
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
