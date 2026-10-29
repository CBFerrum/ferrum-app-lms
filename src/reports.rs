//! Learn reports: completion rate by course, quiz score distribution, learner progress and
//! overdue assignments. Every query carries the caller's record rules on its main entity, so a
//! learner sees only their own rows and an instructor limited by course scopes only those courses.

use crate::enrollment;
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::reports::{Params, ReportSource, col, read_tx, today};
use ferrum_domain::util::money;
use ferrum_kernel::{Action, Ctx, Error, Kernel, Result, Tx, row_scope};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

pub struct LearnReports;

fn link(name: &str, label: &str, to: &str) -> Value {
    json!({"name": name, "label": label, "type": "link", "to": to})
}

fn date(name: &str, label: &str) -> Value {
    json!({"name": name, "label": label, "type": "date"})
}

/// `select {columns} from` the caller's visible rows of `entity` (alias `a`).
fn visible(ctx: &Ctx, entity: &str, columns: &str, joins: &str) -> Result<QueryBuilder<Postgres>> {
    let ent = ctx.schema.entity(entity).ok_or_else(|| Error::NotFound(entity.into()))?;
    let scope = row_scope(ctx, ent, Action::Read)?;
    let mut qb = QueryBuilder::new(format!("select {columns} from {} a {joins} where a.tenant_id = ", ferrum_kernel::ddl::q(&ent.table)));
    qb.push_bind(ctx.tenant);
    scope.push_sql(&mut qb, "a");
    Ok(qb)
}

fn uuid_filter(qb: &mut QueryBuilder<Postgres>, p: &Params<'_>, param: &str, column: &str) -> Result<()> {
    if let Some(v) = p.uuid(param)? {
        qb.push(format!(" and {column} = ")).push_bind(v);
    }
    Ok(())
}

/// `from_date` / `to_date` on a date column, or on an instant read as a date in `zone` (the
/// workspace's time zone, bound as a parameter).
fn date_range(qb: &mut QueryBuilder<Postgres>, p: &Params<'_>, column: &str, zone: Option<&str>) -> Result<(Option<NaiveDate>, Option<NaiveDate>)> {
    let (from, to) = (p.date("from_date")?, p.date("to_date")?);
    if let (Some(f), Some(t)) = (from, to)
        && f > t
    {
        return Err(Error::field("from_date", "must not be after the to date"));
    }
    let push_column = |qb: &mut QueryBuilder<Postgres>| match zone {
        Some(z) => {
            qb.push(format!(" and ({column} at time zone ")).push_bind(z.to_string()).push(")::date");
        }
        None => {
            qb.push(format!(" and {column}"));
        }
    };
    if let Some(f) = from {
        push_column(qb);
        qb.push(" >= ").push_bind(f);
    }
    if let Some(t) = to {
        push_column(qb);
        qb.push(" < ").push_bind(t).push(" + 1");
    }
    Ok((from, to))
}

fn pct(part: i64, whole: i64) -> Value {
    if whole == 0 { Value::Null } else { money(enrollment::percent(part, whole)) }
}

fn dec(v: Option<Decimal>) -> Value {
    v.map_or(Value::Null, |d| money(d.round_dp(2)))
}

async fn completion(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let mut qb = visible(
        ctx,
        "LmsEnrollment",
        "co.id, co.title, count(*) filter (where a.status <> 'Withdrawn'), count(*) filter (where a.status = 'Active'), \
         count(*) filter (where a.status = 'Completed'), count(*) filter (where a.status = 'Withdrawn'), \
         avg(a.progress_percent) filter (where a.status <> 'Withdrawn'), count(a.certificate)",
        "join e_lms_course co on co.tenant_id = a.tenant_id and co.id = a.course",
    )?;
    uuid_filter(&mut qb, p, "course", "a.course")?;
    uuid_filter(&mut qb, p, "category", "co.category")?;
    let (from, to) = date_range(&mut qb, p, "a.enrolled_on", None)?;
    qb.push(" group by co.id, co.title order by co.title");
    let data: Vec<(Uuid, String, i64, i64, i64, i64, Option<Decimal>, i64)> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let mut t = (0i64, 0i64, 0i64, 0i64, 0i64);
    let rows: Vec<Value> = data
        .into_iter()
        .map(|(id, title, enrolled, active, completed, withdrawn, avg, certs)| {
            t = (t.0 + enrolled, t.1 + active, t.2 + completed, t.3 + withdrawn, t.4 + certs);
            json!({"course": id, "course_title": title, "enrolled": enrolled, "active": active, "completed": completed, "withdrawn": withdrawn,
                   "completion_rate": pct(completed, enrolled), "average_progress": dec(avg), "certificates": certs})
        })
        .collect();
    Ok(json!({
        "columns": [col("course_title", "Course", "data"), col("enrolled", "Enrolled", "int"), col("active", "In Progress", "int"),
            col("completed", "Completed", "int"), col("withdrawn", "Withdrawn", "int"), col("completion_rate", "Completion Rate %", "percent"),
            col("average_progress", "Average Progress %", "percent"), col("certificates", "Certificates", "int")],
        "rows": rows,
        "totals": {"enrolled": t.0, "active": t.1, "completed": t.2, "withdrawn": t.3, "completion_rate": pct(t.2, t.0), "certificates": t.4},
        "meta": {"from_date": from, "to_date": to, "note": "Enrolled counts learners not withdrawn; the rate is completed / enrolled."},
    }))
}

/// Buckets of ten points: 0–<10, 10–<20, … 90–100.
const BUCKETS: [&str; 10] = ["0-10", "10-20", "20-30", "30-40", "40-50", "50-60", "60-70", "70-80", "80-90", "90-100"];

async fn quiz_scores(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let mut buckets = String::new();
    for i in 1..=10 {
        buckets.push_str(&format!(", count(*) filter (where least(width_bucket(a.percentage, 0, 100, 10), 10) = {i})"));
    }
    let mut qb = visible(
        ctx,
        "LmsQuizAttempt",
        &format!(
            "q.id, q.title, co.title, count(*), count(distinct a.learner), avg(a.percentage), min(a.percentage), max(a.percentage), \
             count(*) filter (where a.passed), count(distinct a.learner) filter (where a.passed){buckets}"
        ),
        "join e_lms_quiz q on q.tenant_id = a.tenant_id and q.id = a.quiz join e_lms_course co on co.tenant_id = a.tenant_id and co.id = a.course",
    )?;
    qb.push(" and a.status <> 'In Progress'");
    uuid_filter(&mut qb, p, "course", "a.course")?;
    uuid_filter(&mut qb, p, "quiz", "a.quiz")?;
    let (from, to) = date_range(&mut qb, p, "a.submitted_at", Some(&ctx.schema.timezone))?;
    qb.push(" group by q.id, q.title, co.title order by co.title, q.title");
    let data: Vec<sqlx::postgres::PgRow> = qb.build().fetch_all(&mut **tx).await?;
    use sqlx::Row;
    let mut rows = Vec::with_capacity(data.len());
    for r in &data {
        let attempts: i64 = r.try_get(3)?;
        let passed: i64 = r.try_get(8)?;
        let mut row = json!({
            "quiz": r.try_get::<Uuid, _>(0)?, "quiz_title": r.try_get::<String, _>(1)?, "course_title": r.try_get::<String, _>(2)?,
            "attempts": attempts, "learners": r.try_get::<i64, _>(4)?, "average": dec(r.try_get(5)?), "lowest": dec(r.try_get(6)?),
            "highest": dec(r.try_get(7)?), "pass_rate": pct(passed, attempts), "learners_passed": r.try_get::<i64, _>(9)?,
        });
        for (i, b) in BUCKETS.iter().enumerate() {
            row[format!("pct_{}", b.replace('-', "_"))] = json!(r.try_get::<i64, _>(10 + i)?);
        }
        rows.push(row);
    }
    let mut columns = vec![
        col("quiz_title", "Quiz", "data"),
        col("course_title", "Course", "data"),
        col("attempts", "Attempts", "int"),
        col("learners", "Learners", "int"),
        col("average", "Average %", "percent"),
        col("lowest", "Lowest %", "percent"),
        col("highest", "Highest %", "percent"),
        col("pass_rate", "Attempts Passed %", "percent"),
        col("learners_passed", "Learners Passed", "int"),
    ];
    columns.extend(BUCKETS.iter().map(|b| col(&format!("pct_{}", b.replace('-', "_")), &format!("{b}%"), "int")));
    Ok(
        json!({"columns": columns, "rows": rows, "meta": {"from_date": from, "to_date": to, "note": "Submitted and timed-out attempts; buckets by percentage."}}),
    )
}

#[derive(sqlx::FromRow)]
struct ProgressRow {
    number: Option<String>,
    learner: String,
    learner_name: Option<String>,
    course_title: String,
    cohort_title: Option<String>,
    status: String,
    progress_percent: Option<Decimal>,
    completed_lessons: Option<i64>,
    total_lessons: Option<i64>,
    enrolled_on: Option<NaiveDate>,
    last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    quizzes_passed: i64,
    submissions_to_grade: i64,
    certificate: Option<String>,
}

async fn learner_progress(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let mut qb = visible(
        ctx,
        "LmsEnrollment",
        "a.number, a.learner, a.learner_name, co.title as course_title, ch.title as cohort_title, a.status, a.progress_percent, a.completed_lessons, \
         a.total_lessons, a.enrolled_on, a.last_activity_at, a.completed_at, \
         (select count(distinct t.quiz) from e_lms_quiz_attempt t where t.tenant_id = a.tenant_id and t.enrollment = a.id and t.passed) as quizzes_passed, \
         (select count(*) from e_lms_assignment_submission s where s.tenant_id = a.tenant_id and s.enrollment = a.id and s.status = 'Submitted') as submissions_to_grade, \
         cert.number as certificate",
        "join e_lms_course co on co.tenant_id = a.tenant_id and co.id = a.course \
         left join e_lms_cohort ch on ch.tenant_id = a.tenant_id and ch.id = a.cohort \
         left join e_lms_certificate cert on cert.tenant_id = a.tenant_id and cert.id = a.certificate",
    )?;
    uuid_filter(&mut qb, p, "course", "a.course")?;
    uuid_filter(&mut qb, p, "cohort", "a.cohort")?;
    if let Some(status) = p.str("status") {
        if !["Active", "Completed", "Withdrawn"].contains(&status) {
            return Err(Error::field("status", "use Active, Completed or Withdrawn"));
        }
        qb.push(" and a.status = ").push_bind(status.to_string());
    }
    if let Some(learner) = p.str("learner") {
        qb.push(" and a.learner = ").push_bind(learner.to_lowercase());
    }
    qb.push(" order by co.title, lower(coalesce(a.learner_name, a.learner)) limit 10000");
    let data: Vec<ProgressRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let rows: Vec<Value> = data
        .iter()
        .map(|r| {
            json!({
                "enrollment": r.number, "learner": r.learner, "learner_name": r.learner_name, "course_title": r.course_title, "cohort": r.cohort_title,
                "status": r.status, "progress_percent": dec(r.progress_percent), "completed_lessons": r.completed_lessons.unwrap_or(0),
                "total_lessons": r.total_lessons.unwrap_or(0), "enrolled_on": r.enrolled_on, "last_activity_at": r.last_activity_at.map(crate::util::ts),
                "completed_at": r.completed_at.map(crate::util::ts), "quizzes_passed": r.quizzes_passed, "submissions_to_grade": r.submissions_to_grade,
                "certificate": r.certificate,
            })
        })
        .collect();
    let avg = if data.is_empty() {
        Value::Null
    } else {
        dec(Some(data.iter().map(|r| r.progress_percent.unwrap_or_default()).sum::<Decimal>() / Decimal::from(data.len())))
    };
    Ok(json!({
        "columns": [col("learner_name", "Learner", "data"), col("learner", "Email", "data"), col("course_title", "Course", "data"),
            col("cohort", "Cohort", "data"), col("status", "Status", "data"), col("progress_percent", "Progress %", "percent"),
            col("completed_lessons", "Lessons Done", "int"), col("total_lessons", "Required Lessons", "int"), col("enrolled_on", "Enrolled On", "date"),
            col("last_activity_at", "Last Activity", "datetime"), col("completed_at", "Completed At", "datetime"),
            col("quizzes_passed", "Quizzes Passed", "int"), col("submissions_to_grade", "To Grade", "int"), col("certificate", "Certificate", "data")],
        "rows": rows,
        "totals": {"learners": data.len(), "average_progress": avg},
    }))
}

#[derive(sqlx::FromRow)]
struct OwedRow {
    learner: String,
    learner_name: Option<String>,
    course_title: String,
    assignment_title: String,
    due: NaiveDate,
    last_status: Option<String>,
    last_passed: Option<bool>,
}

async fn overdue(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let as_of = p.date("as_of")?.unwrap_or_else(today);
    let mut qb = visible(
        ctx,
        "LmsEnrollment",
        "a.learner, a.learner_name, co.title as course_title, asg.title as assignment_title, \
         coalesce(asg.due_date, a.enrolled_on + asg.due_days::int) as due, last.status as last_status, last.passed as last_passed",
        "join e_lms_course co on co.tenant_id = a.tenant_id and co.id = a.course \
         join e_lms_lesson l on l.tenant_id = a.tenant_id and l.course = a.course and l.assignment is not null \
         join e_lms_assignment asg on asg.tenant_id = l.tenant_id and asg.id = l.assignment \
         left join lateral (select s.status, s.passed from e_lms_assignment_submission s where s.tenant_id = a.tenant_id \
              and s.assignment = asg.id and s.learner = a.learner order by s.submission_no desc limit 1) last on true",
    )?;
    qb.push(" and a.status = 'Active' and (asg.due_date is not null or coalesce(asg.due_days, 0) > 0)");
    qb.push(" and coalesce(asg.due_date, a.enrolled_on + asg.due_days::int) < ").push_bind(as_of);
    qb.push(" and (last.status is null or last.status = 'Needs Revision' or (last.status = 'Graded' and not last.passed))");
    uuid_filter(&mut qb, p, "course", "a.course")?;
    qb.push(" order by due, co.title, lower(coalesce(a.learner_name, a.learner)) limit 10000");
    let data: Vec<OwedRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let rows: Vec<Value> = data
        .iter()
        .map(|r| {
            let status = match (r.last_status.as_deref(), r.last_passed) {
                (None, _) => "Not Submitted",
                (Some("Graded"), _) => "Failed",
                (Some(other), _) => other,
            };
            json!({"learner": r.learner, "learner_name": r.learner_name, "course_title": r.course_title, "assignment_title": r.assignment_title,
                   "due_date": r.due, "days_overdue": (as_of - r.due).num_days(), "status": status})
        })
        .collect();
    Ok(json!({
        "columns": [col("learner_name", "Learner", "data"), col("learner", "Email", "data"), col("course_title", "Course", "data"),
            col("assignment_title", "Assignment", "data"), col("due_date", "Due", "date"), col("days_overdue", "Days Overdue", "int"),
            col("status", "Status", "data")],
        "rows": rows,
        "totals": {"overdue": data.len()},
        "meta": {"as_of": as_of, "note": "Active learners whose work is missing, failed or returned for revision after its due date."},
    }))
}

#[async_trait]
impl ReportSource for LearnReports {
    fn catalog(&self) -> Vec<Value> {
        let entry = |name: &str, label: &str, entity: &str, params: Vec<Value>| json!({"name": name, "label": label, "module": "learn", "entity": entity, "params": params});
        let course = || link("course", "Course", "LmsCourse");
        vec![
            entry(
                "lms_course_completion",
                "Course Completion",
                "LmsEnrollment",
                vec![course(), link("category", "Category", "LmsCategory"), date("from_date", "Enrolled From"), date("to_date", "Enrolled To")],
            ),
            entry(
                "lms_quiz_scores",
                "Quiz Score Distribution",
                "LmsQuizAttempt",
                vec![course(), link("quiz", "Quiz", "LmsQuiz"), date("from_date", "Submitted From"), date("to_date", "Submitted To")],
            ),
            entry(
                "lms_learner_progress",
                "Learner Progress",
                "LmsEnrollment",
                vec![
                    course(),
                    link("cohort", "Cohort", "LmsCohort"),
                    json!({"name": "status", "label": "Status", "type": "select", "options": ["Active", "Completed", "Withdrawn"]}),
                ],
            ),
            entry(
                "lms_overdue_assignments",
                "Overdue Assignments",
                "LmsEnrollment",
                vec![course(), json!({"name": "as_of", "label": "As Of", "type": "date", "default": "today"})],
            ),
        ]
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, params: &HashMap<String, String>) -> Result<Value> {
        let p = Params { raw: params };
        let mut tx = read_tx(k, ctx).await?;
        let out = match name {
            "lms_course_completion" => completion(ctx, &p, &mut tx).await,
            "lms_quiz_scores" => quiz_scores(ctx, &p, &mut tx).await,
            "lms_learner_progress" => learner_progress(ctx, &p, &mut tx).await,
            "lms_overdue_assignments" => overdue(ctx, &p, &mut tx).await,
            _ => Err(Error::NotFound(format!("report {name}"))),
        };
        tx.rollback().await?;
        out
    }
}
