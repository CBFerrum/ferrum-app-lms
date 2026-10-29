//! Assignments and submissions. A learner submits text and/or a file they uploaded for an
//! assignment of a course they are enrolled in, may edit it until it is graded, and may submit
//! again when it was returned for revision or failed (if the assignment allows it, up to its
//! limit). The course's instructors grade with the `grade` action; a passing grade completes the
//! assignment's lessons. One ungraded submission per learner is a database rule as well.

use crate::enrollment::{self, WITHDRAWN};
use crate::notice::{self, Notice};
use crate::util::{self, authored_course, b, d, doc_id, elevated, file_id, id, int, is_manager, is_new, load, s, same_email};
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::util::money;
use ferrum_kernel::value::parse_decimal;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use uuid::Uuid;

pub const SUBMITTED: &str = "Submitted";
pub const GRADED: &str = "Graded";
pub const NEEDS_REVISION: &str = "Needs Revision";
const MAX_FEEDBACK: usize = 100_000;

pub struct AssignmentHook;

#[async_trait]
impl DocHook for AssignmentHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        Ok(())
    }
}

/// A learner's earlier submissions of one assignment: `(number, status, passed)`, oldest first.
type Previous = Vec<(i64, String, bool)>;

async fn previous(tx: &mut Tx, tenant: Uuid, assignment: Uuid, learner: &str) -> Result<Previous> {
    Ok(sqlx::query_as(
        "select submission_no, status, passed from e_lms_assignment_submission where tenant_id = $1 and assignment = $2 and learner = $3 order by submission_no",
    )
    .bind(tenant)
    .bind(assignment)
    .bind(learner.to_lowercase())
    .fetch_all(&mut **tx)
    .await?)
}

/// Why the learner may not hand in a new submission now (`None`: they may).
pub fn blocked(assignment: &Doc, prev: &Previous, due: Option<NaiveDate>, today: NaiveDate) -> Option<String> {
    let returned = match prev.last() {
        Some((_, st, _)) if st == SUBMITTED => return Some("your submission is waiting to be graded: edit it instead".into()),
        Some((_, st, true)) if st == GRADED => return Some("you already passed this assignment".into()),
        Some((_, st, false)) if st == GRADED && !b(assignment, "allow_resubmission") => return Some("this assignment takes no resubmissions".into()),
        Some((_, st, _)) => st == NEEDS_REVISION,
        None => false,
    };
    let max = int(assignment, "max_submissions");
    if max > 0 && !returned && prev.len() as i64 >= max {
        return Some(format!("you have used all {max} submissions"));
    }
    if let Some(due) = due
        && today > due
        && !b(assignment, "accept_late")
    {
        return Some(format!("the deadline ({due}) has passed"));
    }
    None
}

pub struct SubmissionHook;

#[async_trait]
impl DocHook for SubmissionHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !is_new(doc) || hc.ctx.is_system {
            return Ok(());
        }
        if hc.ctx.user.is_nil() || hc.ctx.email.is_empty() {
            return Err(Error::Forbidden("sign in to submit".into()));
        }
        let learner = hc.ctx.email.to_lowercase();
        doc.insert("learner".into(), json!(learner));
        let Some(assignment_id) = id(doc, "assignment") else { return Ok(()) };
        let assignment = match load(hc.kernel, hc.ctx, hc.tx, "LmsAssignment", assignment_id).await {
            Ok(a) => a,
            Err(Error::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let course = id(&assignment, "course").ok_or_else(|| Error::internal("assignment without course"))?;
        // The enrolment row lock serializes a learner's submissions.
        let enrolled: Option<(Uuid, String, NaiveDate, Option<String>)> = sqlx::query_as(
            "select id, status, enrolled_on, learner_name from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 for update",
        )
        .bind(hc.ctx.tenant)
        .bind(course)
        .bind(&learner)
        .fetch_optional(&mut **hc.tx)
        .await?;
        let Some((enrollment, status, enrolled_on, name)) = enrolled else {
            return Err(Error::invalid("enrol in the course first"));
        };
        if status == WITHDRAWN {
            return Err(Error::invalid("your enrolment in this course is withdrawn: enrol again first"));
        }
        let placed: bool = sqlx::query_scalar("select exists(select 1 from e_lms_lesson where tenant_id = $1 and assignment = $2 and course = $3)")
            .bind(hc.ctx.tenant)
            .bind(assignment_id)
            .bind(course)
            .fetch_one(&mut **hc.tx)
            .await?;
        if !placed {
            return Err(Error::field("assignment", "is not part of a lesson of its course yet"));
        }
        let prev = previous(hc.tx, hc.ctx.tenant, assignment_id, &learner).await?;
        let due = enrollment::due_date(&assignment, Some(enrolled_on));
        let today = hc.ctx.today();
        if let Some(why) = blocked(&assignment, &prev, due, today) {
            return Err(Error::invalid(why));
        }
        let number = prev.iter().map(|p| p.0).max().unwrap_or(0) + 1;
        for (k, v) in [
            ("course", json!(course.to_string())),
            ("enrollment", json!(enrollment.to_string())),
            ("learner_name", name.map_or(json!(learner), Value::String)),
            ("submission_no", json!(number)),
            ("submitted_at", json!(util::ts(util::now()))),
            ("due_date", due.map_or(Value::Null, |d| json!(d.to_string()))),
            ("late", json!(due.is_some_and(|d| today > d))),
            ("status", json!(SUBMITTED)),
            ("max_score", assignment.get("max_score").cloned().unwrap_or(Value::Null)),
            ("score", Value::Null),
            ("passed", json!(false)),
            ("feedback", Value::Null),
            ("graded_by", Value::Null),
            ("graded_at", Value::Null),
        ] {
            doc.insert(k.into(), v);
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if hc.ctx.is_system {
            return Ok(());
        }
        if !is_new(doc) {
            if !same_email(s(doc, "learner").unwrap_or_default(), &hc.ctx.email) && !is_manager(hc.ctx) {
                return Err(Error::Forbidden("instructors grade submissions with the grade action; only the learner edits them".into()));
            }
            if s(doc, "status") != Some(SUBMITTED) {
                return Err(Error::invalid("a graded submission cannot change: hand in a new one if the assignment allows it"));
            }
        }
        let Some(assignment) = id(doc, "assignment") else { return Ok(()) };
        let kind: Option<String> = sqlx::query_scalar("select submission_type from e_lms_assignment where tenant_id = $1 and id = $2")
            .bind(hc.ctx.tenant)
            .bind(assignment)
            .fetch_optional(&mut **hc.tx)
            .await?
            .flatten();
        let text = s(doc, "answer").is_some_and(|a| !a.trim().is_empty());
        let file = s(doc, "attachment").is_some();
        match kind.as_deref().unwrap_or("Text or File") {
            "Text" if !text => return Err(Error::field("answer", "write your answer")),
            "File" if !file => return Err(Error::field("attachment", "upload your file")),
            "Text or File" if !text && !file => return Err(Error::field("answer", "write an answer or upload a file")),
            _ => {}
        }
        if let Some(path) = s(doc, "attachment") {
            let Some(file) = file_id(path) else {
                return Err(Error::field("attachment", "upload the file with /api/files and give its /files/ address"));
            };
            let row: Option<(Option<String>, Option<Uuid>, Option<Uuid>)> =
                sqlx::query_as("select entity, record_id, created_by from file_object where tenant_id = $1 and id = $2")
                    .bind(hc.ctx.tenant)
                    .bind(file)
                    .fetch_optional(&mut **hc.tx)
                    .await?;
            let me = id(doc, "id");
            let ok = match row {
                Some((Some(e), Some(r), _)) => e == "LmsAssignmentSubmission" && Some(r) == me,
                Some((None, None, by)) => by == Some(hc.ctx.user),
                _ => false,
            };
            if !ok {
                return Err(Error::field("attachment", "attach a file you uploaded yourself"));
            }
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if hc.ctx.is_system {
            return Ok(());
        }
        let me = doc_id(doc)?;
        // The file follows the submission's access rules: its learner and the course's instructors.
        if let Some(file) = s(doc, "attachment").and_then(file_id) {
            sqlx::query(
                "update file_object set entity = 'LmsAssignmentSubmission', record_id = $3 where tenant_id = $1 and id = $2 and entity is null and record_id is null",
            )
            .bind(hc.ctx.tenant)
            .bind(file)
            .bind(me)
            .execute(&mut **hc.tx)
            .await?;
        }
        if old.is_none()
            && let Some(course) = id(doc, "course")
        {
            let course = load(hc.kernel, hc.ctx, hc.tx, "LmsCourse", course).await?;
            let n = Notice {
                title: format!("New submission: {}", s(doc, "assignment_title").unwrap_or("assignment")),
                body: format!(
                    "{} handed in {} for {}{}.",
                    s(doc, "learner_name").unwrap_or_default(),
                    s(doc, "assignment_title").unwrap_or("an assignment"),
                    s(&course, "title").unwrap_or_default(),
                    if b(doc, "late") { " (late)" } else { "" }
                ),
                entity: "LmsAssignmentSubmission",
                record: me,
            };
            notice::send(hc.kernel, hc.ctx, hc.tx, &util::instructors(&course), &n).await?;
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "grade" {
            return Ok(None);
        }
        Ok(Some(grade(hc.kernel, hc.ctx, hc.tx, doc, params).await?))
    }
}

fn decimal_param(params: &Value, key: &str) -> Result<Option<Decimal>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => parse_decimal(v).map(Some).map_err(|m| Error::field(key, m)),
    }
}

/// `grade {score, feedback?, outcome?: "Graded" | "Needs Revision"}` by the course's instructors.
async fn grade(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, params: &Value) -> Result<Value> {
    let course = authored_course(k, ctx, tx, id(doc, "course").unwrap_or_default()).await?;
    let me = doc_id(doc)?;
    let outcome = params.get("outcome").and_then(Value::as_str).unwrap_or(GRADED);
    if outcome != GRADED && outcome != NEEDS_REVISION {
        return Err(Error::field("outcome", "use Graded or Needs Revision"));
    }
    let assignment = load(k, ctx, tx, "LmsAssignment", id(doc, "assignment").unwrap_or_default()).await?;
    let max = if d(doc, "max_score") > Decimal::ZERO { d(doc, "max_score") } else { d(&assignment, "max_score") };
    let score = decimal_param(params, "score")?;
    if outcome == GRADED && score.is_none() {
        return Err(Error::field("score", "is required to grade"));
    }
    if let Some(sc) = score
        && (sc < Decimal::ZERO || sc > max)
    {
        return Err(Error::field("score", format!("must be between 0 and {}", max.normalize())));
    }
    let feedback = match params.get("feedback") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if f.chars().count() <= MAX_FEEDBACK => Some(f.clone()),
        Some(_) => return Err(Error::field("feedback", format!("write at most {MAX_FEEDBACK} characters of text"))),
    };
    let passed = outcome == GRADED && score.is_some_and(|sc| sc >= d(&assignment, "passing_score"));
    let mut patch = Map::new();
    for (k_, v) in [
        ("status", json!(outcome)),
        ("score", score.map_or(Value::Null, money)),
        ("max_score", money(max)),
        ("passed", json!(passed)),
        ("feedback", feedback.map_or(Value::Null, Value::String)),
        ("graded_by", json!(ctx.email.to_lowercase())),
        ("graded_at", json!(util::ts(util::now()))),
    ] {
        patch.insert(k_.into(), v);
    }
    let done = k.update_tx(&elevated(ctx), tx, "LmsAssignmentSubmission", me, patch, None).await?;
    if passed && let Some(enrollment) = id(doc, "enrollment") {
        let lessons: Vec<Uuid> = sqlx::query_scalar("select id from e_lms_lesson where tenant_id = $1 and assignment = $2 and course = $3")
            .bind(ctx.tenant)
            .bind(doc_id(&assignment)?)
            .bind(doc_id(&course)?)
            .fetch_all(&mut **tx)
            .await?;
        for lesson in lessons {
            enrollment::record_result(k, ctx, tx, enrollment, lesson, "Assignment").await?;
        }
    }
    let title = s(&assignment, "title").unwrap_or("your assignment");
    let verdict = match (outcome, passed) {
        (NEEDS_REVISION, _) => "it needs revision — please submit it again".to_string(),
        (_, true) => "passed".to_string(),
        _ => "not passed yet".to_string(),
    };
    let score_text = score.map(|sc| format!("{} / {}: ", sc.normalize(), max.normalize())).unwrap_or_default();
    let n = Notice {
        title: format!("Assignment graded: {title}"),
        body: format!("{score_text}{verdict}.{}", s(&done, "feedback").map(|f| format!("\nFeedback: {}", util::cut(f, 2000))).unwrap_or_default()),
        entity: "LmsAssignmentSubmission",
        record: me,
    };
    notice::send(k, ctx, tx, &[s(doc, "learner").unwrap_or_default().to_string()], &n).await?;
    Ok(json!({"submission": done}))
}

/// A learner's standing at an assignment, for the lesson page.
pub async fn state(k: &Kernel, ctx: &Ctx, tx: &mut Tx, assignment_id: Uuid, learner: &str, enrolled_on: Option<NaiveDate>) -> Result<Value> {
    let assignment = load(k, ctx, tx, "LmsAssignment", assignment_id).await?;
    let subs: Vec<(Uuid, Option<String>, i64, String, Option<Decimal>, bool, Option<chrono::DateTime<chrono::Utc>>, bool, Option<String>)> = sqlx::query_as(
        "select id, number, submission_no, status, score, passed, submitted_at, late, feedback from e_lms_assignment_submission \
         where tenant_id = $1 and assignment = $2 and learner = $3 order by submission_no",
    )
    .bind(ctx.tenant)
    .bind(assignment_id)
    .bind(learner.to_lowercase())
    .fetch_all(&mut **tx)
    .await?;
    let prev: Previous = subs.iter().map(|x| (x.2, x.3.clone(), x.5)).collect();
    let due = enrollment::due_date(&assignment, enrolled_on);
    let today = ctx.today();
    let why = blocked(&assignment, &prev, due, today);
    Ok(json!({
        "id": assignment_id, "title": assignment.get("title"), "instructions": assignment.get("instructions"),
        "submission_type": assignment.get("submission_type"), "max_score": assignment.get("max_score"), "passing_score": assignment.get("passing_score"),
        "due_date": due.map(|x| x.to_string()), "overdue": due.is_some_and(|x| today > x) && !prev.iter().any(|p| p.2),
        "accept_late": assignment.get("accept_late"), "allow_resubmission": assignment.get("allow_resubmission"), "max_submissions": assignment.get("max_submissions"),
        "passed": prev.iter().any(|p| p.2),
        "can_submit": why.is_none(), "reason": why,
        "submissions": subs.iter().map(|(sid, number, no, status, score, passed, at, late, feedback)| json!({
            "id": sid, "number": number, "submission_no": no, "status": status, "score": score.map(money), "passed": passed,
            "submitted_at": at.map(util::ts), "late": late, "feedback": feedback,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignment(v: Value) -> Doc {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn resubmission_rules() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 5, d).unwrap();
        let open = assignment(json!({"allow_resubmission": true, "max_submissions": 2, "accept_late": false}));
        assert_eq!(blocked(&open, &vec![], Some(day(10)), day(9)), None);
        assert!(blocked(&open, &vec![], Some(day(10)), day(11)).unwrap().contains("deadline"));
        assert_eq!(blocked(&open, &vec![], Some(day(10)), day(10)), None, "the due day itself is in time");
        assert!(blocked(&open, &vec![(1, SUBMITTED.into(), false)], None, day(1)).unwrap().contains("waiting"));
        assert!(blocked(&open, &vec![(1, GRADED.into(), true)], None, day(1)).unwrap().contains("passed"));
        assert_eq!(blocked(&open, &vec![(1, GRADED.into(), false)], None, day(1)), None, "a failed one may be redone");
        let two_failed = vec![(1, GRADED.into(), false), (2, GRADED.into(), false)];
        assert!(blocked(&open, &two_failed, None, day(1)).unwrap().contains("all 2"));
        let returned = vec![(1, GRADED.into(), false), (2, NEEDS_REVISION.into(), false)];
        assert_eq!(blocked(&open, &returned, None, day(1)), None, "returned work may always come back");
        let once = assignment(json!({"allow_resubmission": false, "max_submissions": 0, "accept_late": true}));
        assert!(blocked(&once, &vec![(1, GRADED.into(), false)], None, day(1)).unwrap().contains("no resubmissions"));
        assert_eq!(blocked(&once, &vec![(1, NEEDS_REVISION.into(), false)], Some(day(1)), day(20)), None, "late but accepted");
    }
}
