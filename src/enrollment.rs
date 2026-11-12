//! Enrolments and progress.
//!
//! * Enrolling: learners enrol themselves in published courses that allow it once their
//!   prerequisites are completed; a course's instructors and LMS managers enrol anyone (also in
//!   bulk). A cohort's capacity is checked under a lock on the cohort, so concurrent requests can
//!   never overfill it; one enrolment per learner and course is a unique key.
//! * Progress: completing a lesson records an `LmsLessonProgress` row (once) and recomputes the
//!   enrolment's percentage from the course's current required lessons. When every required lesson
//!   is done — quiz lessons count once the quiz is passed, assignment lessons once a submission
//!   passes — the enrolment completes and, when the course gives one, its certificate is issued.
//!   All of it happens under the enrolment's row lock, so the certificate is issued exactly once.

use crate::catalog::{effective_total, self_paced};
use crate::notice::{self, Notice};
use crate::util::{self, authors, b, doc_id, elevated, id, int, is_new, load, load_locked, rows, s, same_email, sys};
use crate::{certificate, quiz};
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::util::money;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

pub const ACTIVE: &str = "Active";
pub const COMPLETED: &str = "Completed";
pub const WITHDRAWN: &str = "Withdrawn";

/// Most learners one bulk enrolment takes.
const MAX_BULK: usize = 500;

pub struct EnrollmentHook;

#[async_trait]
impl DocHook for EnrollmentHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !is_new(doc) {
            return Ok(());
        }
        if s(doc, "learner").is_none() && !hc.ctx.user.is_nil() && !hc.ctx.email.is_empty() {
            doc.insert("learner".into(), json!(hc.ctx.email.to_lowercase()));
        }
        let now = util::now();
        for (k, v) in [
            ("status", json!(ACTIVE)),
            ("enrolled_at", json!(util::ts(now))),
            ("enrolled_on", json!(hc.ctx.today().to_string())),
            ("enrolled_by", if hc.ctx.user.is_nil() { Value::Null } else { json!(hc.ctx.email.to_lowercase()) }),
            ("progress_percent", json!("0")),
            ("cycle", json!(int(doc, "cycle").max(1))),
            ("completed_lessons", json!(0)),
            ("completed_at", Value::Null),
            ("certificate", Value::Null),
            ("last_lesson", Value::Null),
        ] {
            doc.insert(k.into(), v);
        }
        if let Some(learner) = s(doc, "learner").map(str::to_string) {
            let name = util::user(hc.tx, hc.ctx.tenant, &learner).await?.map(|u| u.name());
            doc.insert("learner_name".into(), name.map_or(Value::Null, Value::String));
        }
        if let Some(course) = id(doc, "course") {
            let (total, _) = counts(hc.tx, hc.ctx.tenant, course, None).await?;
            doc.insert("total_lessons".into(), json!(total));
            // The due date: whoever authors the course (or the engine, for rules and recertification) may state
            // it; otherwise it comes from the course's due days.
            if let Ok(c) = load(hc.kernel, hc.ctx, hc.tx, "LmsCourse", course).await {
                if !authors(hc.ctx, &c) {
                    doc.remove("due_date");
                }
                let days = int(&c, "due_days");
                if s(doc, "due_date").is_none() && days > 0 {
                    doc.insert("due_date".into(), json!((hc.ctx.today() + chrono::Duration::days(days)).to_string()));
                }
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if is_new(doc) && !hc.ctx.is_system {
            check_new(hc.kernel, hc.ctx, hc.tx, doc).await?;
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if old.is_some_and(|o| o.get("due_date") != doc.get("due_date")) {
            // A new due date earns its reminders again.
            let fresh = obj(&[("due_reminder_at", Value::Null), ("overdue_reminder_at", Value::Null)]);
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "LmsEnrollment", doc_id(doc)?, fresh).await?;
        }
        match old {
            None => announce(hc.kernel, hc.ctx, hc.tx, doc).await,
            Some(old) if !hc.ctx.is_system => {
                if old.get("course") != doc.get("course") || old.get("learner") != doc.get("learner") {
                    return Err(Error::invalid("the course and the learner of an enrolment cannot change"));
                }
                if old.get("cohort") != doc.get("cohort") {
                    let course = util::authored_course(hc.kernel, hc.ctx, hc.tx, id(doc, "course").unwrap_or_default()).await?;
                    if let Some(cohort) = id(doc, "cohort") {
                        check_cohort(hc.ctx, hc.tx, &course, cohort, Some(doc_id(doc)?), true).await?;
                    }
                }
                Ok(())
            }
            Some(_) => Ok(()),
        }
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if !["complete_lesson", "open_lesson", "start_quiz", "scorm_commit", "outline", "withdraw", "reactivate"].contains(&name) {
            return Ok(None);
        }
        let (k, ctx) = (hc.kernel, hc.ctx);
        let course = load(k, ctx, hc.tx, "LmsCourse", id(doc, "course").unwrap_or_default()).await?;
        let own = same_email(s(doc, "learner").unwrap_or_default(), &ctx.email);
        let author = authors(ctx, &course);
        if !own && !author {
            return Err(Error::Forbidden("only the learner and the course's instructors can do this".into()));
        }
        let me = doc_id(doc)?;
        let out = match name {
            "complete_lesson" => {
                let lesson = uuid_param(params, "lesson")?;
                let l = lesson_of(k, ctx, hc.tx, lesson, &course).await?;
                let by_learner = self_paced(s(&l, "lesson_type").unwrap_or_default())
                    || (s(&l, "lesson_type") == Some("LTI") && crate::lti::learner_marks_done(hc.tx, ctx.tenant, &l).await?);
                if !author && !by_learner {
                    return Err(Error::invalid("quiz lessons are completed by passing the quiz, assignment lessons by a passing submission, live lessons by attending the live session".to_string()));
                }
                let via = if own && !author { "Learner" } else { "Instructor" };
                record_completion(k, ctx, hc.tx, me, lesson, via).await?
            }
            "open_lesson" => open_lesson(k, ctx, hc.tx, Some(doc), &course, uuid_param(params, "lesson")?).await?,
            "start_quiz" => {
                if !own {
                    return Err(Error::Forbidden("only the learner can take the quiz".into()));
                }
                quiz::start(k, ctx, hc.tx, doc, uuid_param(params, "quiz")?).await?
            }
            "scorm_commit" => {
                if !own {
                    return Err(Error::Forbidden("only the learner records their own SCORM progress".into()));
                }
                let lesson = uuid_param(params, "lesson")?;
                crate::scorm::commit(k, ctx, hc.tx, doc, &course, lesson, params.get("cmi").unwrap_or(&Value::Null)).await?
            }
            "outline" => outline(k, ctx, hc.tx, &course, Some(me)).await?,
            "withdraw" => {
                let done = k.update_tx(&elevated(ctx), hc.tx, "LmsEnrollment", me, obj(&[("status", json!(WITHDRAWN))]), None).await?;
                json!({"enrollment": done})
            }
            _ => {
                // Coming back: the same checks as a new enrolment, the cohort place included.
                if !author {
                    if !crate::catalog::is_published(&course) || !b(&course, "self_enrollment") {
                        return Err(Error::invalid("this course takes no self-enrolments now: ask an instructor"));
                    }
                    let missing = missing_prerequisites(ctx, hc.tx, &course, s(doc, "learner").unwrap_or_default()).await?;
                    if !missing.is_empty() {
                        return Err(Error::invalid(format!("complete these courses first: {}", missing.join(", "))));
                    }
                    crate::payments::check_paid(ctx, hc.tx, &course, s(doc, "learner").unwrap_or_default()).await?;
                }
                if let Some(cohort) = id(doc, "cohort") {
                    check_cohort(ctx, hc.tx, &course, cohort, Some(me), author).await?;
                }
                k.update_tx(&elevated(ctx), hc.tx, "LmsEnrollment", me, obj(&[("status", json!(ACTIVE))]), None).await?;
                let fresh = load_locked(k, ctx, hc.tx, "LmsEnrollment", me).await?;
                let done = refresh(k, ctx, hc.tx, &fresh, None).await?;
                json!({"enrollment": done})
            }
        };
        Ok(Some(out))
    }
}

/// Lesson progress is recorded by Learn itself, never written directly.
pub struct ProgressHook;

#[async_trait]
impl DocHook for ProgressHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid(
                "progress is recorded by completing lessons (complete_lesson on the enrolment), passing quizzes and passing assignments",
            ));
        }
        Ok(())
    }
}

fn obj(pairs: &[(&str, Value)]) -> Doc {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

pub fn uuid_param(params: &Value, key: &str) -> Result<Uuid> {
    params.get(key).and_then(Value::as_str).and_then(|v| v.trim().parse().ok()).ok_or_else(|| Error::field(key, "is required (a record id)"))
}

/// A lesson of `course` (anything else reads as not found).
async fn lesson_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lesson: Uuid, course: &Doc) -> Result<Doc> {
    match load(k, ctx, tx, "LmsLesson", lesson).await {
        Ok(l) if id(&l, "course") == id(course, "id") => Ok(l),
        Ok(_) | Err(Error::NotFound(_)) => Err(Error::field("lesson", "is not a lesson of this course")),
        Err(e) => Err(e),
    }
}

// ───────────────────────────── enrolling ─────────────────────────────

/// Rules for a new enrolment made by a user (bulk ones included).
async fn check_new(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<()> {
    let learner = s(doc, "learner").ok_or_else(|| Error::field("learner", "is required"))?;
    if util::user(tx, ctx.tenant, learner).await?.is_none() {
        return Err(Error::field("learner", "no active user signs in with this email"));
    }
    let Some(course_id) = id(doc, "course") else { return Ok(()) };
    let course = load(k, ctx, tx, "LmsCourse", course_id).await?;
    let author = authors(ctx, &course);
    if !same_email(learner, &ctx.email) && !author {
        return Err(Error::Forbidden("only the course's instructors and LMS managers can enrol other people".into()));
    }
    match s(&course, "status") {
        Some("Archived") => return Err(Error::invalid("the course is archived and takes no new enrolments")),
        Some("Published") => {}
        _ if author => {}
        _ => return Err(Error::invalid("the course is not open for enrolment yet")),
    }
    if !author {
        if !b(&course, "self_enrollment") {
            return Err(Error::invalid("enrolment in this course is by invitation: ask an instructor"));
        }
        let missing = missing_prerequisites(ctx, tx, &course, learner).await?;
        if !missing.is_empty() {
            return Err(Error::field("course", format!("complete these courses first: {}", missing.join(", "))));
        }
        crate::payments::check_paid(ctx, tx, &course, learner).await?;
    }
    if let Some(cohort) = id(doc, "cohort") {
        check_cohort(ctx, tx, &course, cohort, None, author).await?;
    }
    Ok(())
}

/// Titles of the course's prerequisites the learner has not completed.
pub async fn missing_prerequisites(ctx: &Ctx, tx: &mut Tx, course: &Doc, learner: &str) -> Result<Vec<String>> {
    if rows(course, "prerequisites").is_empty() {
        return Ok(vec![]);
    }
    Ok(sqlx::query_scalar(
        "select c.title from e_lms_course__prerequisites p join e_lms_course c on c.tenant_id = p.tenant_id and c.id = p.course \
         where p.tenant_id = $1 and p.parent_id = $2 and not exists (select 1 from e_lms_enrollment e where e.tenant_id = p.tenant_id \
         and e.course = p.course and e.learner = $3 and e.status = 'Completed') order by p.idx",
    )
    .bind(ctx.tenant)
    .bind(doc_id(course)?)
    .bind(learner.trim().to_lowercase())
    .fetch_all(&mut **tx)
    .await?)
}

/// The cohort must be the course's, open (for self-enrolment) and have a free place. The cohort
/// row is locked first, so concurrent enrolments count each other.
async fn check_cohort(ctx: &Ctx, tx: &mut Tx, course: &Doc, cohort: Uuid, me: Option<Uuid>, author: bool) -> Result<()> {
    let row: Option<(Uuid, i64, NaiveDate, String)> =
        sqlx::query_as("select course, coalesce(capacity, 0), end_date, title from e_lms_cohort where tenant_id = $1 and id = $2 for update")
            .bind(ctx.tenant)
            .bind(cohort)
            .fetch_optional(&mut **tx)
            .await?;
    let (cohort_course, capacity, end, title) = row.ok_or_else(|| Error::field("cohort", "does not exist"))?;
    if Some(cohort_course) != id(course, "id") {
        return Err(Error::field("cohort", "belongs to another course"));
    }
    if !author && end < ctx.today() {
        return Err(Error::field("cohort", format!("{title} ended on {end}")));
    }
    if capacity > 0 {
        let taken: i64 = sqlx::query_scalar(
            "select count(*) from e_lms_enrollment where tenant_id = $1 and cohort = $2 and status <> 'Withdrawn' and id is distinct from $3",
        )
        .bind(ctx.tenant)
        .bind(cohort)
        .bind(me)
        .fetch_one(&mut **tx)
        .await?;
        if taken >= capacity {
            return Err(Error::Conflict(format!("the cohort {title} is full ({capacity} places)")));
        }
    }
    Ok(())
}

async fn announce(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<()> {
    let (Some(learner), Some(course)) = (s(doc, "learner"), id(doc, "course")) else { return Ok(()) };
    let title = load(k, ctx, tx, "LmsCourse", course).await.ok().and_then(|c| s(&c, "title").map(str::to_string)).unwrap_or_default();
    let (heading, body) = if id(doc, "previous").is_some() {
        let due = s(doc, "due_date").map(|d| format!(", due {d}")).unwrap_or_default();
        ("Recertification", format!("Your certificate for {title} is expiring: you are enrolled again{due}."))
    } else {
        (
            "Enrolled",
            if same_email(learner, &ctx.email) || ctx.user.is_nil() {
                format!("You are enrolled in {title}.")
            } else {
                format!("{} enrolled you in {title}.", if ctx.full_name.trim().is_empty() { &ctx.email } else { &ctx.full_name })
            },
        )
    };
    let n = Notice { title: format!("{heading}: {title}"), body, entity: "LmsEnrollment", record: doc_id(doc)? };
    notice::send(k, ctx, tx, &[learner.to_string()], &n).await?;
    Ok(())
}

/// What happened to one learner of a bulk enrolment.
pub enum Outcome {
    Enrolled(Doc),
    Already(Value),
    Refused(String),
}

/// Enrols one learner inside a savepoint, so a refusal leaves the surrounding transaction usable.
/// Transient conflicts propagate (the whole request is retried).
pub async fn try_enroll(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: Uuid, learner: &str, cohort: Option<Uuid>) -> Result<Outcome> {
    try_enroll_with(k, ctx, tx, course, learner, cohort, Doc::new()).await
}

/// [`try_enroll`] with more fields on the new enrolment (the rule that made it, its due date).
pub async fn try_enroll_with(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: Uuid, learner: &str, cohort: Option<Uuid>, extra: Doc) -> Result<Outcome> {
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "select id, status from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 order by coalesce(cycle, 1) desc limit 1",
    )
    .bind(ctx.tenant)
    .bind(course)
    .bind(learner)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((e, status)) = existing {
        return Ok(Outcome::Already(json!({"id": e, "status": status})));
    }
    let mut doc = obj(&[("course", json!(course.to_string())), ("learner", json!(learner))]);
    if let Some(c) = cohort {
        doc.insert("cohort".into(), json!(c.to_string()));
    }
    doc.extend(extra);
    sqlx::query("savepoint lms_enrol").execute(&mut **tx).await?;
    match k.insert_tx(ctx, tx, "LmsEnrollment", doc).await {
        Ok(d) => {
            sqlx::query("release savepoint lms_enrol").execute(&mut **tx).await?;
            Ok(Outcome::Enrolled(d))
        }
        Err(e) if e.is_retryable() => Err(e),
        Err(e) => {
            sqlx::query("rollback to savepoint lms_enrol").execute(&mut **tx).await?;
            Ok(Outcome::Refused(reason(&e)))
        }
    }
}

fn reason(e: &Error) -> String {
    match e {
        Error::Validation(fields) => fields.iter().map(|f| format!("{}: {}", f.field, f.message)).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

/// `enroll_learners` on a course: each address enrolled, already enrolled or refused (with why).
pub async fn enroll_many(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, learners: &[String], cohort: Option<Uuid>) -> Result<Value> {
    let mut emails: Vec<String> = learners.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
    let mut seen = std::collections::HashSet::new();
    emails.retain(|e| seen.insert(e.clone()));
    if emails.is_empty() {
        return Err(Error::field("learners", "list at least one email"));
    }
    if emails.len() > MAX_BULK {
        return Err(Error::field("learners", format!("at most {MAX_BULK} learners at a time")));
    }
    let course_id = doc_id(course)?;
    let (mut enrolled, mut already, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for email in emails {
        match try_enroll(k, ctx, tx, course_id, &email, cohort).await? {
            Outcome::Enrolled(d) => enrolled.push(json!({"learner": email, "enrollment": d.get("id"), "number": d.get("number")})),
            Outcome::Already(e) => already.push(json!({"learner": email, "enrollment": e})),
            Outcome::Refused(why) => skipped.push(json!({"learner": email, "reason": why})),
        }
    }
    Ok(json!({"enrolled": enrolled, "already_enrolled": already, "skipped": skipped}))
}

// ───────────────────────────── progress ─────────────────────────────

/// `(required lessons, of them completed)` for an enrolment of `course` (or just the course's
/// total when `enrollment` is `None`). With no lesson marked required, every lesson counts.
pub async fn counts(tx: &mut Tx, tenant: Uuid, course: Uuid, enrollment: Option<Uuid>) -> Result<(i64, i64)> {
    let (required, all, required_done, all_done): (i64, i64, i64, i64) = sqlx::query_as(
        "select count(*) filter (where l.required), count(*), count(p.id) filter (where l.required), count(p.id) \
         from e_lms_lesson l left join e_lms_lesson_progress p on p.tenant_id = l.tenant_id and p.lesson = l.id and p.enrollment = $3 \
         where l.tenant_id = $1 and l.course = $2",
    )
    .bind(tenant)
    .bind(course)
    .bind(enrollment.unwrap_or_else(Uuid::nil))
    .fetch_one(&mut **tx)
    .await?;
    Ok(if required > 0 { (required, required_done) } else { (effective_total(required, all), all_done) })
}

pub fn percent(done: i64, total: i64) -> Decimal {
    if total <= 0 {
        return Decimal::ZERO;
    }
    (Decimal::from(done.min(total)) * Decimal::ONE_HUNDRED / Decimal::from(total))
        .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
}

/// Recomputes every live enrolment of a course after its lessons changed (one statement;
/// `excluding` is a lesson being deleted). Enrolments this completes are finished by maintenance.
pub async fn refresh_course(tx: &mut Tx, tenant: Uuid, course: Uuid, excluding: Option<Uuid>) -> Result<()> {
    sqlx::query(
        "with t as (select count(*) filter (where required) as req, count(*) as alll from e_lms_lesson \
                    where tenant_id = $1 and course = $2 and id is distinct from $3), \
              d as (select e.id, count(l.id) filter (where l.required) as req_done, count(l.id) as all_done \
                    from e_lms_enrollment e \
                    left join e_lms_lesson_progress p on p.tenant_id = e.tenant_id and p.enrollment = e.id \
                    left join e_lms_lesson l on l.tenant_id = p.tenant_id and l.id = p.lesson and l.course = e.course and l.id is distinct from $3 \
                    where e.tenant_id = $1 and e.course = $2 and e.status <> 'Withdrawn' group by e.id), \
              v as (select d.id, case when t.req > 0 then t.req else t.alll end as total, \
                           case when t.req > 0 then d.req_done else d.all_done end as done from d, t) \
         update e_lms_enrollment e set total_lessons = v.total, completed_lessons = v.done, \
                progress_percent = case when v.total = 0 then 0 else round(least(v.done, v.total) * 100.0 / v.total, 2) end, updated_at = now() \
         from v where e.tenant_id = $1 and e.id = v.id",
    )
    .bind(tenant)
    .bind(course)
    .bind(excluding)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records that `lesson` is done (once), then refreshes the enrolment — completing it, and issuing
/// its certificate, when that was the last required lesson. Returns the progress summary.
pub async fn record_completion(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: Uuid, lesson: Uuid, via: &str) -> Result<Value> {
    let e = load_locked(k, ctx, tx, "LmsEnrollment", enrollment).await?;
    if s(&e, "status") == Some(WITHDRAWN) {
        return Err(Error::invalid("this enrolment is withdrawn: enrol again to continue"));
    }
    let done: bool = sqlx::query_scalar("select exists(select 1 from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and lesson = $3)")
        .bind(ctx.tenant)
        .bind(enrollment)
        .bind(lesson)
        .fetch_one(&mut **tx)
        .await?;
    if !done {
        let row = obj(&[
            ("enrollment", json!(enrollment.to_string())),
            ("lesson", json!(lesson.to_string())),
            ("course", e.get("course").cloned().unwrap_or(Value::Null)),
            ("learner", e.get("learner").cloned().unwrap_or(Value::Null)),
            ("completed_at", json!(util::ts(util::now()))),
            ("completed_via", json!(via)),
            ("completed_by", if ctx.user.is_nil() { Value::Null } else { json!(ctx.email.to_lowercase()) }),
        ]);
        k.insert_tx(&elevated(ctx), tx, "LmsLessonProgress", row).await?;
    }
    let after = refresh(k, ctx, tx, &e, Some(lesson)).await?;
    Ok(summary(&after, !done))
}

/// Like [`record_completion`] for results (quiz passed, assignment passed): a withdrawn enrolment
/// keeps its result but makes no progress.
pub async fn record_result(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: Uuid, lesson: Uuid, via: &str) -> Result<()> {
    let status: Option<String> = sqlx::query_scalar("select status from e_lms_enrollment where tenant_id = $1 and id = $2")
        .bind(ctx.tenant)
        .bind(enrollment)
        .fetch_optional(&mut **tx)
        .await?;
    if status.as_deref().is_some_and(|st| st != WITHDRAWN) {
        record_completion(k, ctx, tx, enrollment, lesson, via).await?;
    }
    Ok(())
}

pub fn summary(e: &Doc, newly: bool) -> Value {
    json!({
        "enrollment": e.get("id"), "status": e.get("status"), "progress_percent": e.get("progress_percent"),
        "completed_lessons": e.get("completed_lessons"), "total_lessons": e.get("total_lessons"),
        "completed_at": e.get("completed_at"), "certificate": e.get("certificate"), "newly_completed": newly,
    })
}

/// Recomputes the enrolment's progress (it must be locked); completes it when every required
/// lesson is done, issuing the certificate when the course gives one.
pub async fn refresh(k: &Kernel, ctx: &Ctx, tx: &mut Tx, e: &Doc, touched: Option<Uuid>) -> Result<Doc> {
    let me = doc_id(e)?;
    let course_id = id(e, "course").ok_or_else(|| Error::internal("enrolment without course"))?;
    let (total, done) = counts(tx, ctx.tenant, course_id, Some(me)).await?;
    let mut values = obj(&[("progress_percent", money(percent(done, total))), ("completed_lessons", json!(done)), ("total_lessons", json!(total))]);
    if let Some(lesson) = touched {
        values.insert("last_lesson".into(), json!(lesson.to_string()));
        values.insert("last_activity_at".into(), json!(util::ts(util::now())));
    }
    let completing = s(e, "status") == Some(ACTIVE) && total > 0 && done >= total;
    if !completing {
        k.set_values_tx(&sys(ctx), tx, "LmsEnrollment", me, values.clone()).await?;
        let mut out = e.clone();
        out.extend(values);
        return Ok(out);
    }
    let course = load(k, ctx, tx, "LmsCourse", course_id).await?;
    values.insert("status".into(), json!(COMPLETED));
    values.insert("completed_at".into(), json!(util::ts(util::now())));
    let cert = if b(&course, "issue_certificate") { Some(certificate::issue(k, ctx, tx, e, &course).await?) } else { None };
    if let Some(c) = &cert {
        values.insert("certificate".into(), c.get("id").cloned().unwrap_or(Value::Null));
    }
    let updated = k.update_tx(&elevated(ctx), tx, "LmsEnrollment", me, values, None).await?;
    hr_training_record(k, ctx, tx, s(e, "learner").unwrap_or_default(), &course, cert.as_ref()).await?;
    let title = s(&course, "title").unwrap_or_default();
    let n = match &cert {
        Some(c) => Notice {
            title: format!("Certificate issued: {title}"),
            body: format!(
                "Congratulations — you completed {title}. Your certificate {} is ready; anyone can verify it with the code {}.",
                s(c, "number").unwrap_or_default(),
                s(c, "verification_code").unwrap_or_default()
            ),
            entity: "LmsCertificate",
            record: doc_id(c)?,
        },
        None => Notice {
            title: format!("Course completed: {title}"),
            body: format!("Congratulations — you completed {title}."),
            entity: "LmsEnrollment",
            record: me,
        },
    };
    notice::send(k, ctx, tx, &[s(e, "learner").unwrap_or_default().to_string()], &n).await?;
    Ok(updated)
}

/// Where the HR app is installed, a completed course goes on the employee's HR training record (the
/// course's name as the HR training program, made when HR has none of that name), valid until the
/// certificate's last day. Learn keeps no record of its own beside it.
async fn hr_training_record(k: &Kernel, ctx: &Ctx, tx: &mut Tx, learner: &str, course: &Doc, cert: Option<&Doc>) -> Result<()> {
    let (Some(employee), Some(program), Some(_)) = (ctx.schema.entity("Employee"), ctx.schema.entity("TrainingProgram"), ctx.schema.entity("TrainingRecord"))
    else {
        return Ok(());
    };
    let found: Option<(Uuid, Uuid)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select id, company from {} where tenant_id = $1 and lower(\"user\"::text) = lower($2) limit 1",
        ferrum_kernel::ddl::q(&employee.table)
    )))
    .bind(ctx.tenant)
    .bind(learner)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((emp, company)) = found else { return Ok(()) };
    let title = s(course, "title").unwrap_or_default();
    let known: Option<Uuid> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select id from {} where tenant_id = $1 and program_name = $2", ferrum_kernel::ddl::q(&program.table))))
            .bind(ctx.tenant)
            .bind(title)
            .fetch_optional(&mut **tx)
            .await?;
    let program_id = match known {
        Some(p) => p,
        None => {
            let made = obj(&[("program_name", json!(title)), ("company", json!(company.to_string()))]);
            doc_id(&k.insert_tx(&sys(ctx), tx, "TrainingProgram", made).await?)?
        }
    };
    let record = obj(&[
        ("employee", json!(emp.to_string())),
        ("program", json!(program_id.to_string())),
        ("program_name", json!(title)),
        ("company", json!(company.to_string())),
        ("completed_on", json!(ctx.today().to_string())),
        ("valid_until", cert.and_then(|c| c.get("expires_on")).cloned().unwrap_or(Value::Null)),
    ]);
    k.insert_tx(&sys(ctx), tx, "TrainingRecord", record).await?;
    Ok(())
}

/// Maintenance: a learner whose certificate expires within the course's days (or Learn Settings') is
/// enrolled again, due on the day it expires, in a new enrolment that points at the one it follows (that
/// one, its progress and its certificate stay as they were). Completing it issues the certificate that
/// supersedes the old. One re-enrolment per expiry: an enrolment that already has a successor is skipped,
/// whatever became of it.
pub async fn recertify(k: &Kernel, ctx: &Ctx, at: chrono::DateTime<chrono::Utc>) -> Result<usize> {
    let today = util::today_at(ctx, at);
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let lead = sqlx::query_scalar("select recertify_before_days from e_lms_settings where tenant_id = $1 limit 1")
        .bind(ctx.tenant)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .unwrap_or(30i64);
    let due: Vec<(Uuid, Uuid, String, i64, NaiveDate, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "select e.id, e.course, e.learner, coalesce(e.cycle, 1), c.expires_on, e.rule, e.programme from e_lms_enrollment e \
         join e_lms_certificate c on c.tenant_id = e.tenant_id and c.id = e.certificate \
         join e_lms_course co on co.tenant_id = e.tenant_id and co.id = e.course \
         where e.tenant_id = $1 and e.status = 'Completed' and c.status in ('Valid', 'Expired') and c.expires_on is not null \
           and co.status = 'Published' and coalesce(co.recertify_before_days, $2) > 0 \
           and (e.programme is null or coalesce((select pc.repeats from e_lms_programme__courses pc where pc.tenant_id = e.tenant_id \
                and pc.parent_id = e.programme and pc.course = e.course limit 1), true)) \
           and $3 >= c.expires_on - coalesce(co.recertify_before_days, $2)::int \
           and not exists (select 1 from e_lms_enrollment n where n.tenant_id = e.tenant_id and n.previous = e.id) \
           and exists (select 1 from app_user u where u.tenant_id = e.tenant_id and u.is_active and lower(u.email::text) = lower(e.learner)) \
         order by c.expires_on limit 500",
    )
    .bind(ctx.tenant)
    .bind(lead)
    .bind(today)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for (previous, course, learner, cycle, expires, rule, programme) in &due {
        ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let doc = obj(&[
                ("course", json!(course.to_string())),
                ("learner", json!(learner)),
                ("previous", json!(previous.to_string())),
                ("cycle", json!(cycle + 1)),
                ("due_date", json!(expires.to_string())),
            ]);
            let doc: Doc = doc
                .into_iter()
                .chain(rule.map(|r| ("rule".to_string(), json!(r.to_string()))))
                .chain(programme.map(|p| ("programme".to_string(), json!(p.to_string()))))
                .collect();
            k.insert_tx(ctx, &mut tx, "LmsEnrollment", doc).await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
    }
    Ok(due.len())
}

/// Maintenance: each Active enrolment with a due date is reminded once before it (within the course's
/// or Learn Settings' days of it) and once after it fell due; the reminder is claimed in the same
/// transaction that tells the learner, so two replicas never both send it. Completed and withdrawn
/// enrolments are never reminded.
pub async fn send_due_reminders(k: &Kernel, ctx: &Ctx, at: chrono::DateTime<chrono::Utc>) -> Result<usize> {
    let today = util::today_at(ctx, at);
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let st = util::settings(&mut tx, ctx.tenant).await?;
    let due: Vec<(Uuid, String, NaiveDate, bool)> = sqlx::query_as(
        "select id, title, due_date, before from (select e.id, co.title, e.due_date, \
           (e.due_reminder_at is null and coalesce(co.due_reminder_days, $2) > 0 and $4 <= e.due_date \
            and $4 >= e.due_date - coalesce(co.due_reminder_days, $2)::int) as before, \
           (e.overdue_reminder_at is null and coalesce(co.overdue_reminder_days, $3) > 0 and $4 > e.due_date \
            and $4 >= e.due_date + coalesce(co.overdue_reminder_days, $3)::int) as after \
          from e_lms_enrollment e join e_lms_course co on co.tenant_id = e.tenant_id and co.id = e.course \
          where e.tenant_id = $1 and e.status = 'Active' and e.due_date is not null) x where before or after order by due_date limit 500",
    )
    .bind(ctx.tenant)
    .bind(st.due_reminder_days)
    .bind(st.overdue_reminder_days)
    .bind(today)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut sent = 0;
    for (e, title, due_on, before) in due {
        let column = if before { "due_reminder_at" } else { "overdue_reminder_at" };
        let told = ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let claim = format!(
                "update e_lms_enrollment set {column} = $3 where tenant_id = $1 and id = $2 and status = 'Active' and {column} is null returning learner"
            );
            let learner: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(claim)).bind(ctx.tenant).bind(e).bind(at).fetch_optional(&mut *tx).await?;
            let Some(learner) = learner else {
                tx.rollback().await?;
                return Ok(false);
            };
            let n = if before {
                Notice { title: format!("Due soon: {title}"), body: format!("{title} is due on {due_on}."), entity: "LmsEnrollment", record: e }
            } else {
                Notice {
                    title: format!("Overdue: {title}"),
                    body: format!("{title} was due on {due_on} and is not completed yet."),
                    entity: "LmsEnrollment",
                    record: e,
                }
            };
            notice::send(k, ctx, &mut tx, &[learner], &n).await?;
            tx.commit().await?;
            Ok(true)
        })
        .await?;
        sent += usize::from(told);
    }
    Ok(sent)
}

/// Maintenance: enrolments that reached every required lesson through course changes are
/// completed, and completed enrolments of courses that now give certificates get theirs.
pub async fn complete_ready(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let ready: Vec<Uuid> = sqlx::query_scalar(
        "select e.id from e_lms_enrollment e where e.tenant_id = $1 and ((e.status = 'Active' and e.total_lessons > 0 \
         and e.completed_lessons >= e.total_lessons) or (e.status = 'Completed' and e.certificate is null and exists \
         (select 1 from e_lms_course c where c.tenant_id = e.tenant_id and c.id = e.course and c.issue_certificate))) limit 500",
    )
    .bind(ctx.tenant)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut n = 0;
    for e in ready {
        ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let doc = load_locked(k, ctx, &mut tx, "LmsEnrollment", e).await?;
            if s(&doc, "status") == Some(ACTIVE) {
                refresh(k, ctx, &mut tx, &doc, None).await?;
            } else if s(&doc, "status") == Some(COMPLETED) && id(&doc, "certificate").is_none() {
                let course = load(k, ctx, &mut tx, "LmsCourse", id(&doc, "course").unwrap_or_default()).await?;
                if b(&course, "issue_certificate") {
                    let c = certificate::issue(k, ctx, &mut tx, &doc, &course).await?;
                    k.set_values_tx(&sys(ctx), &mut tx, "LmsEnrollment", e, obj(&[("certificate", c.get("id").cloned().unwrap_or(Value::Null))])).await?;
                    let title = s(&course, "title").unwrap_or_default();
                    let n = Notice {
                        title: format!("Certificate issued: {title}"),
                        body: format!("Your certificate {} for {title} is ready.", s(&c, "number").unwrap_or_default()),
                        entity: "LmsCertificate",
                        record: doc_id(&c)?,
                    };
                    notice::send(k, ctx, &mut tx, &[s(&doc, "learner").unwrap_or_default().to_string()], &n).await?;
                }
            }
            tx.commit().await?;
            Ok(())
        })
        .await?;
        n += 1;
    }
    Ok(n)
}

// ───────────────────────────── outline and lessons ─────────────────────────────

#[derive(sqlx::FromRow)]
struct OutlineRow {
    chapter: Uuid,
    chapter_title: String,
    chapter_summary: Option<String>,
    lesson: Option<Uuid>,
    title: Option<String>,
    lesson_type: Option<String>,
    duration_minutes: Option<i64>,
    is_preview: Option<bool>,
    required: Option<bool>,
    quiz: Option<Uuid>,
    assignment: Option<Uuid>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The course's chapters and lessons in order, with the enrolment's completed lessons and the next
/// lesson to take (the first incomplete required one, else the first incomplete one).
pub async fn outline(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, enrollment: Option<Uuid>) -> Result<Value> {
    let rows: Vec<OutlineRow> = sqlx::query_as(
        "select c.id as chapter, c.title as chapter_title, c.summary as chapter_summary, l.id as lesson, l.title, l.lesson_type, \
                l.duration_minutes, l.is_preview, l.required, l.quiz, l.assignment, p.completed_at \
         from e_lms_chapter c left join e_lms_lesson l on l.tenant_id = c.tenant_id and l.chapter = c.id \
         left join e_lms_lesson_progress p on p.tenant_id = l.tenant_id and p.lesson = l.id and p.enrollment = $3 \
         where c.tenant_id = $1 and c.course = $2 \
         order by c.position nulls last, c.created_at, c.id, l.position nulls last, l.created_at, l.id",
    )
    .bind(ctx.tenant)
    .bind(doc_id(course)?)
    .bind(enrollment.unwrap_or_else(Uuid::nil))
    .fetch_all(&mut **tx)
    .await?;
    let mut chapters: Vec<Value> = Vec::new();
    let (mut next_required, mut next_any) = (None, None);
    for r in &rows {
        if chapters.last().is_none_or(|c| c["id"] != json!(r.chapter)) {
            chapters.push(json!({"id": r.chapter, "title": r.chapter_title, "summary": r.chapter_summary, "lessons": []}));
        }
        let Some(lesson) = r.lesson else { continue };
        let done = r.completed_at.is_some();
        let item = json!({
            "id": lesson, "title": r.title, "lesson_type": r.lesson_type, "duration_minutes": r.duration_minutes.unwrap_or(0),
            "is_preview": r.is_preview.unwrap_or(false), "required": r.required.unwrap_or(true), "quiz": r.quiz, "assignment": r.assignment,
            "completed": done, "completed_at": r.completed_at.map(util::ts),
        });
        if !done {
            if r.required.unwrap_or(true) && next_required.is_none() {
                next_required = Some(item.clone());
            }
            if next_any.is_none() {
                next_any = Some(item.clone());
            }
        }
        if let Some(Value::Array(list)) = chapters.last_mut().and_then(|c| c.get_mut("lessons")) {
            list.push(item);
        }
    }
    let progress = match enrollment {
        Some(e) => {
            let doc = load(k, ctx, tx, "LmsEnrollment", e).await?;
            json!({"enrollment": e, "status": doc.get("status"), "progress_percent": doc.get("progress_percent"),
                   "completed_lessons": doc.get("completed_lessons"), "total_lessons": doc.get("total_lessons"),
                   "cohort": doc.get("cohort"), "certificate": doc.get("certificate")})
        }
        None => Value::Null,
    };
    Ok(json!({
        "course": {"id": course.get("id"), "title": course.get("title"), "slug": course.get("slug"), "status": course.get("status")},
        "progress": progress,
        "next_lesson": if enrollment.is_some() { next_required.or(next_any) } else { None },
        "chapters": chapters,
    }))
}

/// Effective due date of an assignment for a learner enrolled on `enrolled_on`.
pub fn due_date(assignment: &Doc, enrolled_on: Option<NaiveDate>) -> Option<NaiveDate> {
    if let Some(d) = s(assignment, "due_date").and_then(|v| v.parse::<NaiveDate>().ok()) {
        return Some(d);
    }
    let days = int(assignment, "due_days");
    if days > 0 { enrolled_on.map(|e| e + chrono::Duration::days(days)) } else { None }
}

/// Where a learner downloads a lesson's file: through Learn (enrolment-checked) for uploaded
/// files, as is for external links.
pub fn file_link(lesson: &Doc, base: &str) -> Value {
    match s(lesson, "attachment") {
        Some(p) if util::file_id(p).is_some() => json!(format!("{base}/file")),
        Some(p) => json!(p),
        None => Value::Null,
    }
}

/// A lesson as an enrolled learner (or an author) opens it: its content, whether it is done, and
/// the state of its quiz or assignment. Opening it makes it the enrolment's resume point.
pub async fn open_lesson(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: Option<&Doc>, course: &Doc, lesson: Uuid) -> Result<Value> {
    let l = lesson_of(k, ctx, tx, lesson, course).await?;
    let base = format!("/api/app/{}/lessons/{lesson}", util::PACKAGE);
    let mut out = json!({
        "lesson": {
            "id": lesson, "title": l.get("title"), "lesson_type": l.get("lesson_type"), "chapter": l.get("chapter"), "course": l.get("course"),
            "content": l.get("content"), "video_url": l.get("video_url"), "video_embed_url": l.get("video_embed_url"),
            "file_url": file_link(&l, &base), "duration_minutes": l.get("duration_minutes"), "is_preview": l.get("is_preview"), "required": l.get("required"),
        },
        "completed": false,
    });
    match s(&l, "lesson_type") {
        Some("SCORM") => out["scorm"] = crate::scorm::player(ctx, tx, &l, enrollment).await?,
        Some("xAPI") => out["xapi"] = crate::xapi::player(ctx, tx, &l, enrollment).await?,
        Some("LTI") => out["lti"] = crate::lti::player(k, ctx, tx, &l).await?,
        Some("Live") => out["live"] = crate::live::for_lesson(k, ctx, tx, lesson, enrollment).await?,
        _ => {}
    }
    let Some(e) = enrollment else { return Ok(out) };
    let eid = doc_id(e)?;
    let done: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("select completed_at from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and lesson = $3")
            .bind(ctx.tenant)
            .bind(eid)
            .bind(lesson)
            .fetch_optional(&mut **tx)
            .await?;
    out["completed"] = json!(done.is_some());
    out["completed_at"] = json!(done.map(util::ts));
    if let Some(q) = id(&l, "quiz") {
        out["quiz"] = quiz::state(k, ctx, tx, q, eid).await?;
    }
    if let Some(a) = id(&l, "assignment") {
        let enrolled_on = s(e, "enrolled_on").and_then(|v| v.parse().ok());
        out["assignment"] = crate::assignment::state(k, ctx, tx, a, eid, enrolled_on).await?;
    }
    if s(e, "status") != Some(WITHDRAWN) {
        k.set_values_tx(
            &sys(ctx),
            tx,
            "LmsEnrollment",
            eid,
            obj(&[("last_lesson", json!(lesson.to_string())), ("last_activity_at", json!(util::ts(util::now())))]),
        )
        .await?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_and_due_dates() {
        assert_eq!(percent(1, 3).to_string(), "33.33");
        assert_eq!(percent(2, 3).to_string(), "66.67");
        assert_eq!(percent(3, 3).to_string(), "100");
        assert_eq!(percent(5, 3).to_string(), "100", "never above 100");
        assert_eq!(percent(0, 0), Decimal::ZERO);
        let on = NaiveDate::from_ymd_opt(2026, 1, 30);
        let fixed: Doc = json!({"due_date": "2026-03-01", "due_days": 0}).as_object().cloned().unwrap();
        assert_eq!(due_date(&fixed, on), NaiveDate::from_ymd_opt(2026, 3, 1));
        let relative: Doc = json!({"due_date": null, "due_days": 30}).as_object().cloned().unwrap();
        assert_eq!(due_date(&relative, on), NaiveDate::from_ymd_opt(2026, 3, 1));
        let none: Doc = json!({"due_days": 0}).as_object().cloned().unwrap();
        assert_eq!(due_date(&none, on), None);
    }
}
