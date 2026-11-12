// A course's page for someone signed in: what it is, and the one thing to do next. Not enrolled:
// enrol, with the exact reasons when the server would refuse and the cohorts that have a place.
// Enrolled: continue where they stopped, or see the certificate they earned. An instructor of the
// course also finds the builder and the learners here.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router";
import { api, docPath, errorMessage, formatDate, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, PageLoading, toast, useShellTitle } from "@ferrum/kit";
import { Failed } from "./states";
import { buy, enrol, getCourse, keys, reactivate, withdraw } from "./api";
import { CertificateActions } from "./Certificate";
import { DueMarks, useDues } from "./Due";
import { CourseLayout, Prerequisites } from "./CourseLayout";
import { priceText, sessionStart } from "./format";
import { Glyph } from "./Glyph";
import { paths } from "./lesson";
import { progressText, totals } from "./outline";
import { ProgressLine } from "./parts";
import { paramsFor } from "./filters";
import type { CohortInfo, CoursePage, Sale } from "./types";

const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Record links (a notification, the bar's search) name a course by id; its page is named by slug. */
function ById({ id }: { id: string }) {
  const q = useQuery({ queryKey: ["lms", "slug", id], queryFn: () => api.get<{ slug: string }>(docPath("LmsCourse", id)) });
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not find the course")} />;
  if (!q.data) return <PageLoading label={tr("Opening the course")} />;
  return <Navigate to={paths.course(q.data.slug)} replace />;
}

export function CourseRoute() {
  const { ref = "" } = useParams();
  return ID.test(ref) ? <ById id={ref} /> : <CourseView key={ref} slug={ref} />;
}

const placesText = (c: CohortInfo) => (c.places.free === null ? tr("No limit on places") : c.places.free === 0 ? tr("Full") : tr("{value} left", { value: (c.places.free === 1 ? tr("1 place") : tr("{n} places", { n: c.places.free })) }));

function CohortChoice({ cohorts, value, onChange }: { cohorts: CohortInfo[]; value: string | null; onChange: (id: string | null) => void }) {
  if (!cohorts.length) return null;
  return (
    <fieldset className="lms-cohorts">
      <legend>{tr("Choose a cohort")}</legend>
      <label className="lms-option" data-chosen={value === null || undefined}>
        <input type="radio" name="cohort" checked={value === null} onChange={() => onChange(null)} />
        <span>
          <strong>{tr("On my own")}</strong>
          <span className="faint lms-option-sub">{tr("Work through it at your own pace")}</span>
        </span>
      </label>
      {cohorts.map((c) => (
        <label key={c.id} className="lms-option" data-chosen={value === c.id || undefined}>
          <input type="radio" name="cohort" disabled={c.places.free === 0} checked={value === c.id} onChange={() => onChange(c.id)} />
          <span>
            <strong>{isolate(c.title)}</strong>
            <span className="faint lms-option-sub">
              {tr("{value} to {value2} · {c}", { value: formatDate(c.start_date), value2: formatDate(c.end_date), c: placesText(c) })}
            </span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

/** A paid course not paid for: buy it, then pay for it at the store's payment page. This page
 *  checks again every few seconds while the payment is due, so it opens up when the money arrives. */
function BuyBox({ c, sale, left }: { c: CoursePage; sale: Sale; left: boolean }) {
  const qc = useQueryClient();
  const [problem, setProblem] = useState<string | null>(null);
  const go = useMutation({
    mutationFn: () => buy(c.id),
    onSuccess: async (r) => {
      const url = r.purchase.payment?.status === "Initiated" ? r.purchase.payment.payment_url : null;
      if (url) window.location.assign(url);
      else await qc.invalidateQueries({ queryKey: keys.course(c.slug) });
    },
    onError: (e) => setProblem(errorMessage(e)),
  });
  const price = priceText(sale.price);
  const due = sale.purchase?.status === "Awaiting Payment" ? sale.purchase : null;
  const request = due?.payment ?? null;
  if (due && request?.status === "Initiated") {
    return (
      <>
        <h3 className="lms-box-head">{tr("Waiting for your payment")}</h3>
        <p className="faint lms-box-line">
          {request.payment_url ? tr("Pay {price} to start the course. This page opens it when the payment arrives.", { price }) : tr("Your payment page is being prepared. This page updates by itself.")}
        </p>
        <div className="lms-box-actions">
          {request.payment_url ? (
            <a className="btn btn-primary btn-md" href={request.payment_url}>
              {tr("Pay {price}", { price })}
            </a>
          ) : (
            <Button variant="primary" loading disabled>
              {tr("Preparing the payment")}
            </Button>
          )}
        </div>
      </>
    );
  }
  if (due || sale.can_buy) {
    return (
      <>
        <h3 className="lms-box-head">{due ? tr("Your payment did not go through") : left ? tr("Buy the course again") : tr("Buy this course")}</h3>
        <p className="faint lms-box-line">
          {due ? tr("Nothing was taken. Try again to get a new payment page.") : left ? tr("You left it and were refunded. Your progress is kept if you buy it again for {price}.", { price }) : tr("{price}, paid online. You are enrolled as soon as it arrives.", { price })}
        </p>
        {problem && <Alert tone="error">{isolate(problem)}</Alert>}
        <div className="lms-box-actions">
          <Button variant="primary" loading={go.isPending} onClick={() => (setProblem(null), go.mutate())}>
            {due ? tr("Try again") : tr("Buy for {price}", { price })}
          </Button>
        </div>
      </>
    );
  }
  const blockers = c.enroll_blockers ?? [];
  return (
    <>
      <h3 className="lms-box-head">{tr("This course is not on sale yet")}</h3>
      {blockers.length ? (
        <ul className="lms-plain">
          {blockers.map((b) => (
            <li key={b}>{b.charAt(0).toUpperCase() + b.slice(1)}</li>
          ))}
        </ul>
      ) : (
        <p className="faint lms-box-line">{tr("Ask an instructor how to join it.")}</p>
      )}
    </>
  );
}

function Box({ c, slug }: { c: CoursePage; slug: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [cohort, setCohort] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const p = c.my_enrollment;
  const dues = useDues();
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
    const ok = await confirmDialog({ title: tr("Leave {title}?", { title: c.title }), message: tr("Your progress is kept if you enrol again."), confirmLabel: tr("Leave course"), danger: true });
    if (!ok) return;
    try {
      await withdraw(p!.enrollment);
      await everything();
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const sum = totals(c.chapters);
  // A paid course is bought before it is enrolled in (its instructors try it without paying).
  const unpaid = !!c.sale && !c.sale.paid && !c.is_author && (!p || p.status === "Withdrawn");

  return (
    <>
      {c.is_author && (
        <div className="lms-box">
          <h3 className="lms-box-head">{tr("You teach this course")}</h3>
          <div className="lms-box-actions">
            <Link className="btn btn-primary btn-md" to={paths.build(c.id)}>
              {tr("Open the builder")}
            </Link>
            <Link className="btn btn-default btn-md" to={paths.teaching(c.id)}>
              <Glyph name="users" size={14} /> {tr("Learners")}
            </Link>
            <Link className="btn btn-default btn-md" to={paths.learn(slug)}>
              <Glyph name="eye" size={14} /> {tr("Preview as a learner")}
            </Link>
          </div>
        </div>
      )}
      <div className="lms-box">
        {p && p.status === "Active" ? (
          <>
            <h3 className="lms-box-head">{tr("Your progress")}</h3>
            <ProgressLine value={p.progress_percent} label={tr("Your progress in this course")} />
            <p className="faint lms-box-line">{isolate(progressText(p.completed_lessons, p.total_lessons, sum.lessons))}</p>
            <DueMarks enrollment={p.enrollment} dues={dues} />
            <div className="lms-box-actions">
              <Link className="btn btn-primary btn-md" to={paths.learn(slug)}>
                {p.completed_lessons ? tr("Continue") : tr("Start the course")} <Glyph name="arrowRight" size={14} />
              </Link>
              <Button variant="ghost" onClick={() => void leave()}>
                {tr("Leave course")}
              </Button>
            </div>
          </>
        ) : p && p.status === "Completed" ? (
          <>
            <h3 className="lms-box-head">
              {tr("You completed this course")} <Badge tone="green">{tr("Done")}</Badge>
            </h3>
            <div className="lms-box-actions">
              <Link className="btn btn-default btn-md" to={paths.learn(slug)}>
                {tr("Review the course")}
              </Link>
              {p.certificate && <CertificateActions id={p.certificate} />}
            </div>
          </>
        ) : unpaid ? (
          <BuyBox c={c} sale={c.sale!} left={p?.status === "Withdrawn"} />
        ) : p && p.status === "Withdrawn" ? (
          <>
            <h3 className="lms-box-head">{tr("You left this course")}</h3>
            <p className="faint lms-box-line">{tr("Your progress is kept if you enrol again.")}</p>
            <div className="lms-box-actions">
              <Button variant="primary" loading={back.isPending} onClick={() => back.mutate()}>
                {tr("Enrol again")}
              </Button>
            </div>
          </>
        ) : c.can_enroll || c.is_author ? (
          <>
            <h3 className="lms-box-head">{c.can_enroll ? tr("Take this course") : tr("Try it as a learner")}</h3>
            <CohortChoice cohorts={c.cohorts} value={cohort} onChange={setCohort} />
            {problem && <Alert tone="error">{isolate(problem)}</Alert>}
            <div className="lms-box-actions">
              <Button variant="primary" loading={join.isPending} onClick={() => (setProblem(null), join.mutate())}>
                {tr("Enrol")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <h3 className="lms-box-head">{tr("You cannot enrol yet")}</h3>
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
          <h3 className="lms-box-head">{tr("Live sessions")}</h3>
          <ul className="lms-plain">
            {c.upcoming_sessions!.slice(0, 4).map((s) => (
              <li key={s.session}>
                <strong>{isolate(s.title)}</strong>
                <br />
                <span className="faint">{isolate(sessionStart(s.starts_at, s.time_zone))}</span>
                {s.join_url && (
                  <>
                    {isolate(" ")}
                    <a href={s.join_url} target="_blank" rel="noopener noreferrer">
                      {tr("Join")}
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
  const q = useQuery({
    queryKey: keys.course(slug),
    queryFn: () => getCourse(slug),
    staleTime: 0,
    // While a payment is due, the page looks again until it arrives.
    refetchInterval: (query) => (query.state.data?.sale?.purchase?.status === "Awaiting Payment" ? 4000 : false),
  });
  useShellTitle(q.data?.title);
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={tr("Could not open the course")} />;
  const c = q.data;
  if (!c) return <PageLoading label={tr("Opening the course")} />;
  const learning = !!c.my_enrollment && c.my_enrollment.status !== "Withdrawn";
  return (
    <CourseLayout
      course={c}
      crumb={{ to: paths.catalog, label: tr("Catalog") }}
      tagHref={(t) => `${paths.catalog}?${paramsFor({ tag: t })}`}
      lessonHref={(l) => paths.learn(slug, l.id)}
      visitor={!learning && !c.is_author}
      aside={<Box c={c} slug={slug} />}
    />
  );
}
