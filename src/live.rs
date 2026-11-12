//! Live sessions in the product (audit LM-20, LM-21). A cohort's live session without a link of its
//! own gets a meeting of the Mail, Chat and Calendar app (Comms 0.14.0, with a meeting server):
//! made when the session is saved, moved with it, ended with it. Members get their own join link
//! where they see the session (the lesson page's data, the dashboard, the reminder).
//!
//! Attendance is kept per enrolled learner and session (`lms_session_attendance`): from the meeting,
//! read once it is over (maintenance, ten minutes after it was due to end), or marked by an
//! instructor (`mark_attendance`, which also corrects what the meeting said). A learner counts as
//! attending when they were in the meeting for the share of the session the session asks for
//! (`credit_percent`, 50 by default), and an attended session credits the lesson it names (a lesson
//! of type Live) as done. Nothing is taken back when an instructor later marks someone absent:
//! progress and certificates stay as they were given.
//!
//! Without Comms or a meeting server, and for a session with a link of its own, a session is what it
//! was: a typed link, and attendance marked by the instructor.

use crate::enrollment;
use crate::util::{self, authored_course, doc_id, id, int, rows, s, time};
use chrono::{DateTime, Duration, Utc};
use ferrum_app_comms::meetings::{self, MeetingError, NewMeeting};
use ferrum_kernel::{Ctx, Doc, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// Minutes after a session was due to end before its meeting is read and closed.
const SETTLE_MINUTES: i64 = 10;

fn minutes(row: &Doc) -> i64 {
    match int(row, "duration_minutes") {
        0 => 60,
        m => m,
    }
}

fn meeting_of(row: &Doc) -> Option<Uuid> {
    s(row, "meeting").and_then(|m| m.parse().ok())
}

/// A member's way into a session: its meeting's link, else the link typed on it.
pub async fn join_link(k: &Kernel, tenant: Uuid, meeting: Option<&str>, typed: Option<&str>) -> Option<String> {
    if let Some(m) = meeting.and_then(|m| m.parse().ok())
        && let Ok(link) = meetings::member_link(k, tenant, m).await
    {
        return Some(link);
    }
    typed.map(str::to_string)
}

/// Brings the sessions' meetings in step with a saved cohort (`old`: what it was, if it existed).
pub async fn sync(hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
    let tenant = hc.ctx.tenant;
    if meetings::available(hc.kernel, tenant).await.is_err() {
        return Ok(());
    }
    let first_instructor = rows(doc, "instructors").iter().find_map(|r| s(r, "instructor").map(str::to_lowercase));
    let sessions = rows(doc, "sessions");
    for r in &sessions {
        let (Some(row), Some(start)) = (id(r, "id"), time(r, "starts_at")) else { continue };
        let end = start + Duration::minutes(minutes(r));
        match meeting_of(r) {
            Some(m) => {
                let moved = meetings::get(hc.kernel, tenant, m).await.is_ok_and(|x| x.ended_at.is_none() && (x.starts_at, x.ends_at) != (start, end));
                if moved && let Err(e) = meetings::reschedule(hc.kernel, tenant, m, start, end).await {
                    tracing::warn!(%tenant, session = %row, "the session's meeting was not moved: {e}");
                }
            }
            None if s(r, "join_url").is_none() && end > util::now() => {
                let host = match s(r, "instructor").map(str::to_lowercase).or_else(|| first_instructor.clone()) {
                    Some(email) => util::user(hc.tx, tenant, &email).await?.map(|u| u.id),
                    None => None,
                }
                .unwrap_or(hc.ctx.user);
                let title = s(r, "title").unwrap_or("Live session");
                match meetings::create(hc.kernel, tenant, NewMeeting { entity: "LmsCohortSession", record: row, title, starts_at: start, ends_at: end, host })
                    .await
                {
                    Ok(m) => {
                        sqlx::query("update e_lms_cohort__sessions set meeting = $3 where tenant_id = $1 and id = $2")
                            .bind(tenant)
                            .bind(row)
                            .bind(m.id.to_string())
                            .execute(&mut **hc.tx)
                            .await?;
                    }
                    Err(MeetingError::NotInstalled | MeetingError::NotConfigured) => {}
                    Err(e) => tracing::warn!(%tenant, session = %row, "no meeting for the session: {e}"),
                }
            }
            None => {}
        }
    }
    // Sessions taken away end with their meetings.
    if let Some(old) = old {
        let kept: Vec<Uuid> = sessions.iter().filter_map(|r| id(r, "id")).collect();
        for r in rows(old, "sessions") {
            if let (Some(row), Some(m)) = (id(&r, "id"), meeting_of(&r))
                && !kept.contains(&row)
            {
                end_meeting(hc.kernel, tenant, m).await;
            }
        }
    }
    Ok(())
}

/// Ends the meetings of every session of a cohort that is being deleted.
pub async fn end_all(hc: &mut HookCtx<'_>, doc: &Doc) {
    for r in rows(doc, "sessions") {
        if let Some(m) = meeting_of(&r) {
            end_meeting(hc.kernel, hc.ctx.tenant, m).await;
        }
    }
}

async fn end_meeting(k: &Kernel, tenant: Uuid, meeting: Uuid) {
    if let Err(e) = meetings::end(k, tenant, meeting).await
        && !matches!(e, MeetingError::NotInstalled | MeetingError::NotConfigured | MeetingError::NotFound)
    {
        tracing::warn!(%tenant, %meeting, "a session's meeting was not ended: {e}");
    }
}

/// A lesson of type Live named by a session must be one of the cohort's course.
pub async fn check(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<Vec<ferrum_kernel::FieldError>> {
    let mut errors = vec![];
    for (i, r) in rows(doc, "sessions").iter().enumerate() {
        let Some(lesson) = id(r, "lesson") else { continue };
        let row: Option<(String, Option<Uuid>)> = sqlx::query_as("select lesson_type, course from e_lms_lesson where tenant_id = $1 and id = $2")
            .bind(hc.ctx.tenant)
            .bind(lesson)
            .fetch_optional(&mut **hc.tx)
            .await?;
        let ok = row.is_some_and(|(kind, course)| kind == "Live" && course == id(doc, "course"));
        if !ok {
            errors.push(ferrum_kernel::FieldError::new("lesson", "use a lesson of type Live in this cohort's course").in_row("sessions", i));
        }
    }
    Ok(errors)
}

/// What a lesson's page says of its live sessions: those of the learner's cohort (every cohort of the
/// course, for an author), each with the learner's own join link and what was recorded of them.
pub async fn for_lesson(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lesson: Uuid, enrollment: Option<&Doc>) -> Result<Value> {
    let cohort = enrollment.and_then(|e| id(e, "cohort"));
    let learner = enrollment.and_then(|e| s(e, "learner")).unwrap_or_default().to_lowercase();
    if enrollment.is_some() && cohort.is_none() {
        return Ok(json!([]));
    }
    let found: Vec<(Uuid, Uuid, String, DateTime<Utc>, Option<i64>, Option<String>, Option<String>, Option<bool>, Option<i32>)> = sqlx::query_as(
        "select s.id, s.parent_id, s.title, s.starts_at, s.duration_minutes, s.join_url, s.meeting, a.attended, a.minutes \
         from e_lms_cohort__sessions s \
         left join lms_session_attendance a on a.tenant_id = s.tenant_id and a.session_id = s.id and a.learner = $4 \
         where s.tenant_id = $1 and s.lesson = $2 and ($3::uuid is null or s.parent_id = $3) order by s.starts_at limit 100",
    )
    .bind(ctx.tenant)
    .bind(lesson)
    .bind(cohort)
    .bind(&learner)
    .fetch_all(&mut **tx)
    .await?;
    let mut out = vec![];
    for (session, cohort, title, at, length, typed, meeting, attended, spent) in found {
        let link = join_link(k, ctx.tenant, meeting.as_deref(), typed.as_deref()).await;
        out.push(json!({
            "session": session, "cohort": cohort, "title": title, "starts_at": util::ts(at), "duration_minutes": length.unwrap_or(60),
            "join_url": link, "has_meeting": meeting.is_some(), "attended": attended, "minutes": spent,
        }));
    }
    Ok(json!(out))
}

struct Ended {
    session: Uuid,
    cohort: Uuid,
    duration: i64,
    meeting: Uuid,
    percent: i64,
    lesson: Option<Uuid>,
}

/// Maintenance: reads the meeting of every session that is over, records who attended and credits
/// the lesson. A session is closed once, whatever came of it; one whose meeting cannot be read
/// now (no meeting server) is tried again later.
pub async fn close_ended(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    if meetings::available(k, ctx.tenant).await.is_err() {
        return Ok(0);
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let due: Vec<(Uuid, Uuid, Option<i64>, String, Option<i64>, Option<Uuid>)> = sqlx::query_as(
        "select s.id, s.parent_id, s.duration_minutes, s.meeting, s.credit_percent, s.lesson from e_lms_cohort__sessions s \
         where s.tenant_id = $1 and s.meeting is not null \
           and s.starts_at + make_interval(mins => coalesce(s.duration_minutes, 60)::int + $2) < now() \
           and not exists (select 1 from lms_session_closed c where c.tenant_id = s.tenant_id and c.session_id = s.id) \
         order by s.starts_at limit 50",
    )
    .bind(ctx.tenant)
    .bind(SETTLE_MINUTES as i32)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut closed = 0;
    for (session, cohort, duration, meeting, percent, lesson) in due {
        let Some(meeting) = meeting.parse().ok() else { continue };
        let e = Ended { session, cohort, duration: duration.unwrap_or(60), meeting, percent: percent.unwrap_or(50), lesson };
        match settle(k, ctx, &e).await {
            Ok(()) => closed += 1,
            Err(MeetingError::NotConfigured | MeetingError::NotInstalled) => {}
            Err(MeetingError::NotFound) => {
                // The meeting is gone: nothing to read, so the session is closed without it.
                close(k, ctx, session).await?;
            }
            Err(e) => tracing::warn!(tenant = %ctx.tenant, %session, "attendance of the session was not read: {e}"),
        }
    }
    Ok(closed)
}

async fn close(k: &Kernel, ctx: &Ctx, session: Uuid) -> Result<()> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    sqlx::query("insert into lms_session_closed (tenant_id, session_id) values ($1, $2) on conflict do nothing")
        .bind(ctx.tenant)
        .bind(session)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn settle(k: &Kernel, ctx: &Ctx, e: &Ended) -> std::result::Result<(), MeetingError> {
    meetings::end(k, ctx.tenant, e.meeting).await?;
    let present = meetings::attendance(k, ctx.tenant, e.meeting).await?;
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let members: Vec<(Uuid, String)> =
        sqlx::query_as("select id, lower(learner) from e_lms_enrollment where tenant_id = $1 and cohort = $2 and status <> 'Withdrawn'")
            .bind(ctx.tenant)
            .bind(e.cohort)
            .fetch_all(&mut *tx)
            .await?;
    let users: Vec<(Uuid, String)> = sqlx::query_as("select id, lower(email::text) from app_user where tenant_id = $1 and id = any($2)")
        .bind(ctx.tenant)
        .bind(present.iter().filter_map(|a| a.user).collect::<Vec<_>>())
        .fetch_all(&mut *tx)
        .await?;
    let mut credit = vec![];
    for (enrollment, learner) in members {
        let seconds: i64 =
            present.iter().filter(|a| a.user.is_some_and(|u| users.iter().any(|(id, email)| *id == u && *email == learner))).map(|a| a.seconds).sum();
        let attended = seconds > 0 && seconds * 100 >= e.duration * 60 * e.percent;
        // An instructor's mark stands; the meeting's word is only for the rest.
        let wrote = sqlx::query(
            "insert into lms_session_attendance (tenant_id, session_id, learner, attended, minutes, source) values ($1, $2, $3, $4, $5, 'Meeting') \
             on conflict (tenant_id, session_id, learner) do update set attended = excluded.attended, minutes = excluded.minutes, marked_at = now() \
             where lms_session_attendance.source = 'Meeting'",
        )
        .bind(ctx.tenant)
        .bind(e.session)
        .bind(&learner)
        .bind(attended)
        .bind(((seconds + 30) / 60) as i32)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if attended && wrote == 1 {
            credit.push(enrollment);
        }
    }
    sqlx::query("insert into lms_session_closed (tenant_id, session_id) values ($1, $2) on conflict do nothing")
        .bind(ctx.tenant)
        .bind(e.session)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(lesson) = e.lesson {
        for enrollment in credit {
            credit_lesson(k, ctx, enrollment, lesson).await;
        }
    }
    Ok(())
}

/// The lesson is done for a learner who attended; a lesson that cannot be credited (it was removed
/// from the course) costs the learner nothing else.
async fn credit_lesson(k: &Kernel, ctx: &Ctx, enrollment: Uuid, lesson: Uuid) {
    let done = ferrum_kernel::with_retry(|| async {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        enrollment::record_completion(k, ctx, &mut tx, enrollment, lesson, "Live Session").await?;
        tx.commit().await?;
        Ok(())
    })
    .await;
    if let Err(e) = done {
        tracing::warn!(tenant = %ctx.tenant, %enrollment, %lesson, "a live lesson was not credited: {e}");
    }
}

fn session_row(cohort: &Doc, session: &Value) -> Result<Doc> {
    let wanted: Uuid =
        session.as_str().and_then(|v| v.parse().ok()).ok_or_else(|| Error::field("session", "is the id of one of this cohort's live sessions"))?;
    rows(cohort, "sessions").into_iter().find(|r| id(r, "id") == Some(wanted)).ok_or_else(|| Error::field("session", "is not a live session of this cohort"))
}

/// Cohort actions: `mark_attendance` and `session_attendance`.
pub async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    if !matches!(name, "mark_attendance" | "session_attendance") {
        return Ok(None);
    }
    let course = id(doc, "course").unwrap_or_default();
    authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
    let tenant = hc.ctx.tenant;
    let row = session_row(doc, params.get("session").unwrap_or(&Value::Null))?;
    let session = doc_id(&row)?;
    if name == "session_attendance" {
        let found: Vec<(String, bool, i32, String, DateTime<Utc>)> = sqlx::query_as(
            "select learner, attended, minutes, source, marked_at from lms_session_attendance where tenant_id = $1 and session_id = $2 order by learner",
        )
        .bind(tenant)
        .bind(session)
        .fetch_all(&mut **hc.tx)
        .await?;
        let learners: Vec<Value> = found
            .into_iter()
            .map(|(learner, attended, minutes, source, at)| json!({"learner": learner, "attended": attended, "minutes": minutes, "source": source, "marked_at": util::ts(at)}))
            .collect();
        return Ok(Some(json!({"session": session, "title": row.get("title"), "has_meeting": meeting_of(&row).is_some(), "learners": learners})));
    }
    let learner = params
        .get("learner")
        .and_then(Value::as_str)
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .ok_or_else(|| Error::field("learner", "is required"))?;
    let enrollment: Option<Uuid> =
        sqlx::query_scalar("select id from e_lms_enrollment where tenant_id = $1 and cohort = $2 and lower(learner) = $3 and status <> 'Withdrawn'")
            .bind(tenant)
            .bind(doc_id(doc)?)
            .bind(&learner)
            .fetch_optional(&mut **hc.tx)
            .await?;
    let Some(enrollment) = enrollment else { return Err(Error::field("learner", "is not a learner of this cohort")) };
    let attended = params.get("attended").and_then(Value::as_bool).unwrap_or(true);
    let minutes = match params.get("minutes") {
        None | Some(Value::Null) => {
            if attended {
                minutes(&row)
            } else {
                0
            }
        }
        Some(v) => v.as_i64().filter(|m| (0..=1440).contains(m)).ok_or_else(|| Error::field("minutes", "use a whole number from 0 to 1440"))?,
    };
    sqlx::query(
        "insert into lms_session_attendance (tenant_id, session_id, learner, attended, minutes, source, marked_by) values ($1, $2, $3, $4, $5, 'Instructor', $6) \
         on conflict (tenant_id, session_id, learner) do update set attended = excluded.attended, minutes = excluded.minutes, source = 'Instructor', \
         marked_by = excluded.marked_by, marked_at = now()",
    )
    .bind(tenant)
    .bind(session)
    .bind(&learner)
    .bind(attended)
    .bind(minutes as i32)
    .bind(hc.ctx.email.to_lowercase())
    .execute(&mut **hc.tx)
    .await?;
    if attended && let Some(lesson) = id(&row, "lesson") {
        enrollment::record_completion(hc.kernel, hc.ctx, hc.tx, enrollment, lesson, "Instructor").await?;
    }
    Ok(Some(json!({"session": session, "learner": learner, "attended": attended, "minutes": minutes, "source": "Instructor"})))
}
