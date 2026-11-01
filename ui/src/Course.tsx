// A course's page for someone signed in: what it is, and the one thing to do next. Not enrolled:
// enrol, with the exact reasons when the server would refuse and the cohorts that have a place.
// Enrolled: continue where they stopped, or see the certificate they earned. An instructor of the
// course also finds the builder and the learners here.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router";
import { api, docPath, errorMessage, formatDate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, ErrorState, PageLoading, toast, useShellTitle } from "@ferrum/kit";
import { enrol, getCourse, keys, reactivate, withdraw } from "./api";
import { CertificateActions } from "./Certificate";
import { CourseLayout, Prerequisites } from "./CourseLayout";
import { plural, sessionStart } from "./format";
import { Glyph } from "./Glyph";
import { paths } from "./lesson";
import { progressText, totals } from "./outline";
import { ProgressLine } from "./parts";
import { paramsFor } from "./filters";
import type { CohortInfo, CoursePage } from "./types";

const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Record links (a notification, the bar's search) name a course by id; its page is named by slug. */
function ById({ id }: { id: string }) {
  const q = useQuery({ queryKey: ["lms", "slug", id], queryFn: () => api.get<{ slug: string }>(docPath("LmsCourse", id)) });
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not find the course" />;
  if (!q.data) return <PageLoading label="Opening the course" />;
  return <Navigate to={paths.course(q.data.slug)} replace />;
}

export function CourseRoute() {
  const { ref = "" } = useParams();
  return ID.test(ref) ? <ById id={ref} /> : <CourseView key={ref} slug={ref} />;
}

const placesText = (c: CohortInfo) => (c.places.free === null ? "No limit on places" : c.places.free === 0 ? "Full" : `${plural(c.places.free, "place")} left`);

function CohortChoice({ cohorts, value, onChange }: { cohorts: CohortInfo[]; value: string | null; onChange: (id: string | null) => void }) {
  if (!cohorts.length) return null;
  return (
    <fieldset className="lms-cohorts">
      <legend>Choose a cohort</legend>
      <label className="lms-option" data-chosen={value === null || undefined}>
        <input type="radio" name="cohort" checked={value === null} onChange={() => onChange(null)} />
        <span>
          <strong>On my own</strong>
          <span className="faint lms-option-sub">Work through it at your own pace</span>
        </span>
      </label>
      {cohorts.map((c) => (
        <label key={c.id} className="lms-option" data-chosen={value === c.id || undefined}>
          <input type="radio" name="cohort" disabled={c.places.free === 0} checked={value === c.id} onChange={() => onChange(c.id)} />
          <span>
            <strong>{c.title}</strong>
            <span className="faint lms-option-sub">
              {formatDate(c.start_date)} to {formatDate(c.end_date)} · {placesText(c)}
            </span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

function Box({ c, slug }: { c: CoursePage; slug: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [cohort, setCohort] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const p = c.my_enrollment;
  const everything = () => qc.invalidateQueries({ queryKey: ["lms"] });
  const join = useMutation({
    mutationFn: () => enrol(c.id, cohort),
    onSuccess: async () => {
      await everything();
      navigate(paths.learn(slug));
    },
    onError: (e) => setProblem(errorMessage(e)),
  });
  const back = useMutation({
    mutationFn: () => reactivate(p!.enrollment),
    onSuccess: async () => {
      await everything();
      navigate(paths.learn(slug));
    },
    onError: (e) => toast(errorMessage(e), "error"),
  });
  const leave = async () => {
    const ok = await confirmDialog({ title: `Leave ${c.title}?`, message: "Your progress is kept if you enrol again.", confirmLabel: "Leave course", danger: true });
    if (!ok) return;
    try {
      await withdraw(p!.enrollment);
      await everything();
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const sum = totals(c.chapters);

  return (
    <>
      {c.is_author && (
        <div className="lms-box">
          <h3 className="lms-box-head">You teach this course</h3>
          <div className="lms-box-actions">
            <Link className="btn btn-primary btn-md" to={paths.build(c.id)}>
              Open the builder
            </Link>
            <Link className="btn btn-default btn-md" to={paths.teaching(c.id)}>
              <Glyph name="users" size={14} /> Learners
            </Link>
            <Link className="btn btn-default btn-md" to={paths.learn(slug)}>
              <Glyph name="eye" size={14} /> Preview as a learner
            </Link>
          </div>
        </div>
      )}
      <div className="lms-box">
        {p && p.status === "Active" ? (
          <>
            <h3 className="lms-box-head">Your progress</h3>
            <ProgressLine value={p.progress_percent} label="Your progress in this course" />
            <p className="faint lms-box-line">{progressText(p.completed_lessons, p.total_lessons, sum.lessons)}</p>
            <div className="lms-box-actions">
              <Link className="btn btn-primary btn-md" to={paths.learn(slug)}>
                {p.completed_lessons ? "Continue" : "Start the course"} <Glyph name="arrowRight" size={14} />
              </Link>
              <Button variant="ghost" onClick={() => void leave()}>
                Leave course
              </Button>
            </div>
          </>
        ) : p && p.status === "Completed" ? (
          <>
            <h3 className="lms-box-head">
              You completed this course <Badge tone="green">Done</Badge>
            </h3>
            <div className="lms-box-actions">
              <Link className="btn btn-default btn-md" to={paths.learn(slug)}>
                Review the course
              </Link>
              {p.certificate && <CertificateActions id={p.certificate} />}
            </div>
          </>
        ) : p && p.status === "Withdrawn" ? (
          <>
            <h3 className="lms-box-head">You left this course</h3>
            <p className="faint lms-box-line">Your progress is kept if you enrol again.</p>
            <div className="lms-box-actions">
              <Button variant="primary" loading={back.isPending} onClick={() => back.mutate()}>
                Enrol again
              </Button>
            </div>
          </>
        ) : c.can_enroll || c.is_author ? (
          <>
            <h3 className="lms-box-head">{c.can_enroll ? "Take this course" : "Try it as a learner"}</h3>
            <CohortChoice cohorts={c.cohorts} value={cohort} onChange={setCohort} />
            {problem && <Alert tone="error">{problem}</Alert>}
            <div className="lms-box-actions">
              <Button variant="primary" loading={join.isPending} onClick={() => (setProblem(null), join.mutate())}>
                Enrol
              </Button>
            </div>
          </>
        ) : (
          <>
            <h3 className="lms-box-head">You cannot enrol yet</h3>
            <ul className="lms-plain">
              {(c.enroll_blockers ?? []).map((b) => (
                <li key={b}>{b.charAt(0).toUpperCase() + b.slice(1)}</li>
              ))}
            </ul>
          </>
        )}
        <Prerequisites course={c} href={paths.course} />
      </div>
      {p && p.status !== "Withdrawn" && (c.upcoming_sessions?.length ?? 0) > 0 && (
        <div className="lms-box">
          <h3 className="lms-box-head">Live sessions</h3>
          <ul className="lms-plain">
            {c.upcoming_sessions!.slice(0, 4).map((s) => (
              <li key={s.session}>
                <strong>{s.title}</strong>
                <br />
                <span className="faint">{sessionStart(s.starts_at, s.time_zone)}</span>
                {s.join_url && (
                  <>
                    {" "}
                    <a href={s.join_url} target="_blank" rel="noopener noreferrer">
                      Join
                    </a>
                  </>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

function CourseView({ slug }: { slug: string }) {
  const q = useQuery({ queryKey: keys.course(slug), queryFn: () => getCourse(slug), staleTime: 0 });
  useShellTitle(q.data?.title);
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="Could not open the course" />;
  const c = q.data;
  if (!c) return <PageLoading label="Opening the course" />;
  const learning = !!c.my_enrollment && c.my_enrollment.status !== "Withdrawn";
  return (
    <CourseLayout
      course={c}
      crumb={{ to: paths.catalog, label: "Catalog" }}
      tagHref={(t) => `${paths.catalog}?${paramsFor({ tag: t })}`}
      lessonHref={(l) => paths.learn(slug, l.id)}
      visitor={!learning && !c.is_author}
      aside={<Box c={c} slug={slug} />}
    />
  );
}
