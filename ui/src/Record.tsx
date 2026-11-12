// A record of Learn's in the records frame, with what the generic form cannot say: an enrolment
// rule's Preview (how many people it would enrol) and a cohort's attendance at each live session.
// Every other record is the frame's form as it was.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useParams } from "react-router";
import { api, docPath, errorMessage, formatMoment, isolate, t } from "@ferrum/client";
import { Alert, Badge, Button, toast } from "@ferrum/kit";
import { Failed } from "./states";
import { getRoster, keys, markAttendance, previewRule, sessionAttendance } from "./api";
import { FormView } from "./records";
import type { RulePreview } from "./types";

// ───────────────────────────── an enrolment rule ─────────────────────────────

/** What a Preview says, as a sentence. */
export function previewText(p: RulePreview): string {
  return p.would_enrol === 1 ? t("1 person would be enrolled at the next upkeep.") : t("{n} people would be enrolled at the next upkeep.", { n: p.would_enrol });
}

function RulePreviewPanel({ id }: { id: string }) {
  const run = useMutation({ mutationFn: () => previewRule(id) });
  const p = run.data;
  return (
    <section className="lms-box" aria-label={t("Preview")}>
      <h3 className="lms-box-head">{t("Preview")}</h3>
      <p className="faint lms-box-line">{t("Who this rule would enrol now, from the saved rule. Nothing is changed.")}</p>
      <div className="lms-box-actions">
        <Button loading={run.isPending} onClick={() => run.mutate()}>
          {p ? t("Preview again") : t("Preview who it would enrol")}
        </Button>
      </div>
      {run.error && <Alert tone="error">{isolate(errorMessage(run.error))}</Alert>}
      {p && (
        <div aria-live="polite">
          <Alert tone={p.would_enrol ? "info" : "warning"}>{previewText(p)}</Alert>
          {p.already_enrolled > 0 && <p className="faint lms-box-line">{p.already_enrolled === 1 ? t("1 more person matches but is already in the course.") : t("{n} more people match but are already in the course.", { n: p.already_enrolled })}</p>}
          <p className="faint lms-box-line">
            {p.switched_on ? t("The rule is switched on.") : t("The rule is switched off: nobody is enrolled until it is switched on.")}
          </p>
        </div>
      )}
    </section>
  );
}

// ───────────────────────────── a cohort ─────────────────────────────

interface CohortDoc {
  course: string;
  sessions?: { id: string; title: string; starts_at: string }[];
}

function Attendance({ cohort, course, session }: { cohort: string; course: string; session: string }) {
  const qc = useQueryClient();
  const marks = useQuery({ queryKey: keys.attendance(cohort, session), queryFn: () => sessionAttendance(cohort, session), staleTime: 0 });
  const roster = useQuery({ queryKey: keys.roster(course, cohort), queryFn: () => getRoster(course, cohort), staleTime: 0 });
  const mark = useMutation({
    mutationFn: (v: { learner: string; attended: boolean }) => markAttendance(cohort, session, v.learner, v.attended),
    onSuccess: () => {
      toast(t("Attendance saved"));
      void qc.invalidateQueries({ queryKey: keys.attendance(cohort, session) });
    },
    onError: (e) => toast(errorMessage(e), "error"),
  });
  const error = marks.error ?? roster.error;
  if (error) return <Failed error={error} onRetry={() => (void marks.refetch(), void roster.refetch())} title={t("Could not load the attendance")} />;
  if (!marks.data || !roster.data) return <p className="faint" role="status">{t("Loading the attendance")}</p>;
  const by = new Map(marks.data.learners.map((l) => [l.learner.toLowerCase(), l]));
  const learners = roster.data.learners.filter((l) => l.status !== "Withdrawn");
  if (!learners.length) return <p className="faint">{t("No learner is in this cohort yet.")}</p>;
  return (
    <>
      {!marks.data.has_meeting && <p className="faint">{t("This session has no meeting, so attendance is marked here by hand.")}</p>}
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th scope="col">{t("Learner")}</th>
              <th scope="col">{t("Joined")}</th>
              <th scope="col" className="num">{t("Minutes")}</th>
              <th scope="col">{t("Recorded by")}</th>
              <th scope="col">
                <span className="sr-only">{t("Mark")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {learners.map((l) => {
              const m = by.get(l.learner.toLowerCase());
              const name = l.learner_name || l.learner;
              return (
                <tr key={l.enrollment}>
                  <td>
                    {isolate(name)}
                    {l.learner_name && <div className="faint">{isolate(l.learner)}</div>}
                  </td>
                  <td>{m ? <Badge tone={m.attended ? "green" : "neutral"}>{m.attended ? t("Joined") : t("Did not join")}</Badge> : <span className="faint">{t("Not recorded")}</span>}</td>
                  <td className="num">{m ? m.minutes : ""}</td>
                  <td>{m ? (m.source === "Meeting" ? t("Meeting") : t("Instructor")) : ""}</td>
                  <td>
                    <div className="lms-row-actions">
                      <Button size="sm" disabled={mark.isPending} onClick={() => mark.mutate({ learner: l.learner, attended: true })}>
                        {t("Mark present")}
                      </Button>
                      <Button size="sm" variant="ghost" disabled={mark.isPending} onClick={() => mark.mutate({ learner: l.learner, attended: false })}>
                        {t("Mark absent")}
                      </Button>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

function CohortAttendance({ id }: { id: string }) {
  const q = useQuery({ queryKey: keys.cohort(id), queryFn: () => api.get<CohortDoc>(docPath("LmsCohort", id)), staleTime: 0 });
  const [picked, setPicked] = useState("");
  if (q.error) return <Failed error={q.error} onRetry={() => void q.refetch()} title={t("Could not load the cohort")} />;
  const sessions = q.data?.sessions ?? [];
  if (!q.data || !sessions.length) return null;
  const chosen = sessions.find((s) => s.id === picked) ?? sessions[0];
  return (
    <section className="lms-box" aria-label={t("Attendance")}>
      <h3 className="lms-box-head">{t("Attendance")}</h3>
      <div className="lms-box-actions">
        <label htmlFor="lms-att-session" className="faint">
          {t("Live session")}
        </label>
        <select id="lms-att-session" className="select" value={chosen.id} onChange={(e) => setPicked(e.target.value)}>
          {sessions.map((s) => (
            <option key={s.id} value={s.id}>
              {isolate(`${s.title} · ${formatMoment(s.starts_at, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" })}`)}
            </option>
          ))}
        </select>
      </div>
      <Attendance key={chosen.id} cohort={id} course={q.data.course} session={chosen.id} />
    </section>
  );
}

// ───────────────────────────── the route ─────────────────────────────

export function RecordRoute() {
  const { entity = "", id = "" } = useParams();
  const saved = id !== "new";
  return (
    <>
      <FormView />
      {saved && entity === "LmsEnrollmentRule" && <RulePreviewPanel id={id} />}
      {saved && entity === "LmsCohort" && <CohortAttendance id={id} />}
    </>
  );
}
