// The Live panel of a lesson of type Live: when the session is (on the viewer's clock), the way in
// while it is open, and once it is over whether the learner was credited, and if not, why. An
// author, who has no enrolment, sees the cohorts' sessions and where to read their attendance.

import { Link } from "react-router";
import { formatMoment, isolate, t } from "@ferrum/client";
import { Alert, Badge } from "@ferrum/kit";
import { duration } from "./format";
import { Glyph } from "./Glyph";
import { outcome, phase, type SessionPhase } from "./session";
import { paths } from "./lesson";
import type { LiveSession } from "./types";

const WHEN: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" };

const PHASE_TEXT: Record<SessionPhase, () => string> = {
  soon: () => t("Not open yet"),
  open: () => t("Opening soon"),
  live: () => t("Live now"),
  ended: () => t("Ended"),
};
const PHASE_TONE: Record<SessionPhase, "neutral" | "blue" | "green"> = { soon: "neutral", open: "blue", live: "green", ended: "neutral" };

function Verdict({ s }: { s: LiveSession }) {
  const o = outcome(s);
  if (o === "credited") return <Alert tone="success">{t("You attended for {minutes} min. This lesson is credited.", { minutes: s.minutes ?? 0 })}</Alert>;
  if (o === "short")
    return (
      <Alert tone="warning">
        {t("Not credited: you were there for {minutes} of {length} min, less than this session needs to count as attending. Ask your instructor if this is wrong.", { minutes: s.minutes ?? 0, length: s.duration_minutes })}
      </Alert>
    );
  return <p className="faint">{t("Attendance is read from the meeting shortly after the session ends.")}</p>;
}

function Session({ s, learning, now }: { s: LiveSession; learning: boolean; now: number }) {
  const p = phase(s, now);
  const joinable = p === "open" || p === "live";
  return (
    <li className="lms-line">
      <div className="lms-line-main">
        <strong>{isolate(s.title)}</strong>
        <span className="faint">{isolate([formatMoment(s.starts_at, WHEN), duration(s.duration_minutes)].filter(Boolean).join(" · "))}</span>
      </div>
      <Badge tone={PHASE_TONE[p]}>{PHASE_TEXT[p]()}</Badge>
      {learning && joinable && (
        s.join_url ? (
          <a className="btn btn-primary btn-md" href={s.join_url} target="_blank" rel="noopener noreferrer">
            <Glyph name="link" size={14} /> {t("Join")}
          </a>
        ) : (
          <span className="faint">{t("No link yet")}</span>
        )
      )}
      {!learning && (
        <Link className="btn btn-default btn-sm" to={paths.record("LmsCohort", s.cohort)}>
          {t("Attendance")}
        </Link>
      )}
      {learning && p === "soon" && <p className="faint lms-live-note">{t("The way in appears here ten minutes before it starts.")}</p>}
      {learning && p === "ended" && (
        <div className="lms-live-note">
          <Verdict s={s} />
        </div>
      )}
    </li>
  );
}

export function LivePanel({ sessions, learning, now = Date.now() }: { sessions: LiveSession[]; learning: boolean; now?: number }) {
  if (!sessions.length)
    return <Alert tone="info">{learning ? t("No live session is set for this lesson in your cohort yet.") : t("No cohort has a live session for this lesson yet.")}</Alert>;
  return (
    <section className="lms-live" aria-label={t("Live session")}>
      <ul className="lms-lines">
        {sessions.map((s) => (
          <Session key={s.session} s={s} learning={learning} now={now} />
        ))}
      </ul>
    </section>
  );
}
