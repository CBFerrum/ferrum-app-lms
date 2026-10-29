//! Cohorts: a course taken together, with dates, a capacity (enforced when enrolling, see
//! `enrollment`) and live sessions. Sessions are stored as instants; `schedule_sessions` lays a
//! series out at the same wall-clock time in the cohort's IANA zone (the workspace's by default),
//! so a weekly 18:00 session stays at 18:00 across a daylight-saving change. Maintenance reminds
//! the members and instructors once per session start.

use crate::notice::{self, Notice};
use crate::util::{self, authored_course, doc_id, id, int, rows, s};
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result};
use serde_json::{Map, Value, json};
use uuid::Uuid;

/// Most sessions a cohort holds.
const MAX_SESSIONS: usize = 500;

pub struct CohortHook;

/// The cohort's zone: its own, else the workspace's, else UTC.
pub fn zone(cohort_zone: Option<&str>, workspace: &str) -> Tz {
    cohort_zone.and_then(|z| z.parse::<Tz>().ok()).or_else(|| workspace.parse::<Tz>().ok()).unwrap_or(Tz::UTC)
}

/// The instant a wall-clock time in `tz` names. In a spring-forward gap it is the first minute
/// after the gap; in a fall-back overlap, the earlier of the two.
pub fn local_to_utc(tz: Tz, date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let mut local = date.and_time(time);
    for _ in 0..=180 {
        if let Some(t) = tz.from_local_datetime(&local).earliest() {
            return t.with_timezone(&Utc);
        }
        local += Duration::minutes(1);
    }
    local.and_utc()
}

/// `2026-03-31 18:00 CEST`.
pub fn local_label(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z").to_string()
}

/// `2026-03-31T18:00:00+02:00`.
pub fn local_rfc3339(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).to_rfc3339()
}

#[async_trait]
impl DocHook for CohortHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        util::people_rows(hc.tx, hc.ctx.tenant, doc, "instructors", "instructor", "instructor_name").await?;
        if let Some(z) = s(doc, "time_zone").map(str::trim).map(str::to_string) {
            let tz = z.parse::<Tz>().map_err(|_| Error::field("time_zone", "use an IANA time zone such as Europe/Berlin or Asia/Kolkata"))?;
            doc.insert("time_zone".into(), json!(tz.name()));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        let sessions = rows(doc, "sessions");
        if sessions.len() > MAX_SESSIONS {
            return Err(Error::field("sessions", format!("a cohort holds at most {MAX_SESSIONS} sessions")));
        }
        let errors: Vec<FieldError> = sessions
            .iter()
            .enumerate()
            .filter(|(_, r)| s(r, "join_url").is_some_and(|u| !u.starts_with("https://")))
            .map(|(i, _)| FieldError::new("join_url", "use an https link").in_row("sessions", i))
            .collect();
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if let Some(old) = old
            && old.get("course") != doc.get("course")
        {
            let members: i64 = sqlx::query_scalar("select count(*) from e_lms_enrollment where tenant_id = $1 and cohort = $2")
                .bind(hc.ctx.tenant)
                .bind(doc_id(doc)?)
                .fetch_one(&mut **hc.tx)
                .await?;
            if members > 0 {
                return Err(Error::field("course", "a cohort with learners cannot move to another course"));
            }
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "schedule_sessions" {
            return Ok(None);
        }
        authored_course(hc.kernel, hc.ctx, hc.tx, id(doc, "course").unwrap_or_default()).await?;
        let text = |k: &str| params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let number = |k: &str, default: i64, min: i64, max: i64| -> Result<i64> {
            match params.get(k) {
                None | Some(Value::Null) => Ok(default),
                Some(v) => v.as_i64().filter(|n| (min..=max).contains(n)).ok_or_else(|| Error::field(k, format!("use a whole number from {min} to {max}"))),
            }
        };
        let first: NaiveDate = text("first_date").and_then(|v| v.parse().ok()).ok_or_else(|| Error::field("first_date", "is required (YYYY-MM-DD)"))?;
        let at: NaiveTime = text("time")
            .and_then(|v| NaiveTime::parse_from_str(v, "%H:%M").ok())
            .ok_or_else(|| Error::field("time", "is required (HH:MM, 24-hour clock, in the cohort's time zone)"))?;
        let count = number("count", 1, 1, 100)?;
        let every = number("every_days", 7, 1, 365)?;
        let duration = number("duration_minutes", 60, 5, 1440)?;
        let title = text("title").unwrap_or("Live session");
        if title.chars().count() > 190 {
            return Err(Error::field("title", "at most 190 characters"));
        }
        let join_url = text("join_url");
        if join_url.is_some_and(|u| !u.starts_with("https://") || u.len() > 1000) {
            return Err(Error::field("join_url", "use an https link of at most 1000 characters"));
        }
        let mut sessions: Vec<Value> = rows(doc, "sessions").into_iter().map(Value::Object).collect();
        if sessions.len() + count as usize > MAX_SESSIONS {
            return Err(Error::field("count", format!("a cohort holds at most {MAX_SESSIONS} sessions")));
        }
        let tz = zone(s(doc, "time_zone"), &hc.ctx.schema.timezone);
        let mut planned = Vec::new();
        for i in 0..count {
            let starts = local_to_utc(tz, first + Duration::days(i * every), at);
            let label = if count > 1 { format!("{title} {}", i + 1) } else { title.to_string() };
            sessions.push(json!({"title": label, "starts_at": util::ts(starts), "duration_minutes": duration, "join_url": join_url}));
            planned.push(json!({"title": label, "starts_at": util::ts(starts), "local_start": local_rfc3339(starts, tz), "duration_minutes": duration}));
        }
        let mut patch = Map::new();
        patch.insert("sessions".into(), Value::Array(sessions));
        hc.kernel.update_tx(hc.ctx, hc.tx, "LmsCohort", doc_id(doc)?, patch, None).await?;
        Ok(Some(json!({"time_zone": tz.name(), "scheduled": planned})))
    }
}

#[derive(sqlx::FromRow)]
struct Due {
    session: Uuid,
    cohort: Uuid,
    title: String,
    starts_at: DateTime<Utc>,
    duration_minutes: Option<i64>,
    join_url: Option<String>,
    cohort_title: String,
    time_zone: Option<String>,
    course: Uuid,
    course_title: String,
}

/// Maintenance: live sessions starting within the reminder window are announced to the cohort's
/// learners (all but withdrawn ones) and to its and the course's instructors — once per session start: the reminder
/// row is claimed before anyone is told, so concurrent runs cannot both send.
pub async fn send_reminders(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let minutes = util::settings(&mut tx, ctx.tenant).await?.reminder_minutes;
    let due: Vec<Due> = sqlx::query_as(
        "select s.id as session, c.id as cohort, s.title, s.starts_at, s.duration_minutes, s.join_url, c.title as cohort_title, c.time_zone, \
                c.course, co.title as course_title \
         from e_lms_cohort__sessions s join e_lms_cohort c on c.tenant_id = s.tenant_id and c.id = s.parent_id \
         join e_lms_course co on co.tenant_id = c.tenant_id and co.id = c.course \
         where s.tenant_id = $1 and s.starts_at > now() and s.starts_at <= now() + make_interval(mins => $2) \
           and not exists (select 1 from lms_session_reminder r where r.tenant_id = s.tenant_id and r.session_id = s.id and r.starts_at = s.starts_at) \
         order by s.starts_at limit 500",
    )
    .bind(ctx.tenant)
    .bind(minutes as i32)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut sent = 0;
    for d in due {
        let told = ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let claimed = sqlx::query("insert into lms_session_reminder (tenant_id, session_id, starts_at) values ($1, $2, $3) on conflict do nothing")
                .bind(ctx.tenant)
                .bind(d.session)
                .bind(d.starts_at)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
            if !claimed {
                tx.rollback().await?;
                return Ok(false);
            }
            let mut people: Vec<String> =
                sqlx::query_scalar("select learner from e_lms_enrollment where tenant_id = $1 and cohort = $2 and status <> 'Withdrawn'")
                    .bind(ctx.tenant)
                    .bind(d.cohort)
                    .fetch_all(&mut *tx)
                    .await?;
            let teachers: Vec<String> = sqlx::query_scalar(
                "select instructor from e_lms_cohort__instructors where tenant_id = $1 and parent_id = $2 \
                 union select instructor from e_lms_course__instructors where tenant_id = $1 and parent_id = $3",
            )
            .bind(ctx.tenant)
            .bind(d.cohort)
            .bind(d.course)
            .fetch_all(&mut *tx)
            .await?;
            people.extend(teachers);
            let tz = zone(d.time_zone.as_deref(), &ctx.schema.timezone);
            let n = Notice {
                title: format!("Live session soon: {}", d.title),
                body: format!(
                    "{} — {}: \"{}\" starts at {} ({} minutes).{}",
                    d.course_title,
                    d.cohort_title,
                    d.title,
                    local_label(d.starts_at, tz),
                    d.duration_minutes.unwrap_or(60),
                    d.join_url.as_deref().map(|u| format!("\nJoin: {u}")).unwrap_or_default()
                ),
                entity: "LmsCohort",
                record: d.cohort,
            };
            let count = notice::send(k, ctx, &mut tx, &people, &n).await?;
            sqlx::query("update lms_session_reminder set recipients = $4 where tenant_id = $1 and session_id = $2 and starts_at = $3")
                .bind(ctx.tenant)
                .bind(d.session)
                .bind(d.starts_at)
                .bind(count as i32)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(true)
        })
        .await?;
        sent += usize::from(told);
    }
    Ok(sent)
}

/// Upcoming sessions of these cohorts (for dashboards), soonest first, join links included: only
/// call it for cohorts the reader belongs to or teaches.
pub async fn upcoming(tx: &mut ferrum_kernel::Tx, ctx: &Ctx, cohorts: &[Uuid], limit: i64) -> Result<Vec<Value>> {
    if cohorts.is_empty() {
        return Ok(vec![]);
    }
    let found: Vec<(Uuid, String, DateTime<Utc>, Option<i64>, Option<String>, Uuid, String, Option<String>, String)> = sqlx::query_as(
        "select s.id, s.title, s.starts_at, s.duration_minutes, s.join_url, c.id, c.title, c.time_zone, co.title \
         from e_lms_cohort__sessions s join e_lms_cohort c on c.tenant_id = s.tenant_id and c.id = s.parent_id \
         join e_lms_course co on co.tenant_id = c.tenant_id and co.id = c.course \
         where s.tenant_id = $1 and s.parent_id = any($2) and s.starts_at + make_interval(mins => coalesce(s.duration_minutes, 60)::int) > now() \
         order by s.starts_at limit $3",
    )
    .bind(ctx.tenant)
    .bind(cohorts)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?;
    Ok(found
        .into_iter()
        .map(|(sid, title, at, minutes, url, cid, ctitle, tz, course)| {
            let tz = zone(tz.as_deref(), &ctx.schema.timezone);
            json!({
                "session": sid, "title": title, "starts_at": util::ts(at), "local_start": local_rfc3339(at, tz), "time_zone": tz.name(),
                "duration_minutes": minutes.unwrap_or(60), "join_url": url, "cohort": cid, "cohort_title": ctitle, "course_title": course,
            })
        })
        .collect())
}

/// A cohort's capacity and how many active (not withdrawn) learners it has.
pub fn places(cohort: &Doc, taken: i64) -> Value {
    let capacity = int(cohort, "capacity");
    json!({"capacity": capacity, "taken": taken, "free": if capacity > 0 { Some((capacity - taken).max(0)) } else { None }})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_sessions_across_daylight_saving() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let six_pm = NaiveTime::from_hms_opt(18, 0, 0).unwrap();
        let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        // Summer time starts on Sunday 29 March 2026: 18:00 is 17:00 UTC before, 16:00 UTC after.
        assert_eq!(util::ts(local_to_utc(berlin, day(3, 24), six_pm)), "2026-03-24T17:00:00.000Z");
        assert_eq!(util::ts(local_to_utc(berlin, day(3, 31), six_pm)), "2026-03-31T16:00:00.000Z");
        // 02:30 on 29 March does not exist in Berlin: the first minute after the gap.
        let half_two = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        assert_eq!(util::ts(local_to_utc(berlin, day(3, 29), half_two)), "2026-03-29T01:00:00.000Z");
        // 02:30 on 25 October happens twice: the earlier one (still summer time).
        assert_eq!(util::ts(local_to_utc(berlin, day(10, 25), half_two)), "2026-10-25T00:30:00.000Z");
        assert_eq!(local_label(local_to_utc(berlin, day(3, 31), six_pm), berlin), "2026-03-31 18:00 CEST");
        assert_eq!(zone(Some("Not/AZone"), "Asia/Kolkata").name(), "Asia/Kolkata");
        assert_eq!(zone(None, "garbage").name(), "UTC");
    }
}
