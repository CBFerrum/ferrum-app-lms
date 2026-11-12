//! Enrolment by rule: "everyone who matches gets this course". A rule matches the workspace's active
//! users by role or group, or, where the HR app is installed, the active employees who sign in by the
//! fields of their Employee record (read in place, never copied). At each upkeep a switched-on rule enrols
//! the people who match and have no enrolment in the course yet (so never twice, and never someone who
//! enrolled themselves or finished it), due as the rule or the course says; what becomes of an
//! enrolment when its learner stops matching is the rule's choice.

use crate::enrollment::{self, Outcome, WITHDRAWN};
use crate::util::{self, b, elevated, id, int, load, rows, s};
use async_trait::async_trait;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx, ddl::q};
use ferrum_meta::FieldType;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashSet;
use uuid::Uuid;

pub struct RuleHook;

/// One thing a person must be.
enum Criterion {
    Role(String),
    Group(String),
    /// A column of the Employee record and the text it must read as (a link by its record's id).
    Employee(String, String),
}

fn at_row(i: usize, field: &str, message: impl Into<String>) -> Error {
    Error::Validation(vec![FieldError::new(field, message).in_row("criteria", i)])
}

/// The rule's criteria, each checked against what the workspace has.
async fn criteria(ctx: &Ctx, tx: &mut Tx, rule: &Doc) -> Result<Vec<Criterion>> {
    let employees = s(rule, "audience") == Some("Employees");
    if employees && (ctx.schema.entity("Employee").is_none() || !ctx.schema.packages.iter().any(|p| p == "hr")) {
        return Err(Error::field("audience", "employees need the HR app: use users, by role or group"));
    }
    let mut out = Vec::new();
    for (i, row) in rows(rule, "criteria").iter().enumerate() {
        let (field, value) = (s(row, "field").unwrap_or_default().trim().to_string(), s(row, "value").unwrap_or_default().trim().to_string());
        let named = |sql: &'static str| sqlx::query_scalar::<_, bool>(sql);
        if !employees {
            let found = match field.to_lowercase().as_str() {
                "role" => named("select exists(select 1 from role where tenant_id = $1 and name = $2)"),
                "group" => named("select exists(select 1 from scim_group where tenant_id = $1 and display_name = $2)"),
                _ => return Err(at_row(i, "field", "use role or group")),
            };
            if !found.bind(ctx.tenant).bind(&value).fetch_one(&mut **tx).await? {
                return Err(at_row(i, "value", format!("the workspace has no {} called {value}", field.to_lowercase())));
            }
            out.push(if field.eq_ignore_ascii_case("role") { Criterion::Role(value) } else { Criterion::Group(value) });
            continue;
        }
        let employee = ctx.schema.entity("Employee").ok_or_else(|| Error::internal("Employee"))?;
        let f = employee.field(&field).filter(|f| f.ftype.is_stored() && !matches!(f.ftype, FieldType::Json | FieldType::Attach) && f.perm_group.is_none());
        let Some(f) = f else { return Err(at_row(i, "field", "is not a field of the Employee that a rule can match on")) };
        let text = match (f.ftype, f.to.as_deref().and_then(|t| ctx.schema.entity(t))) {
            (FieldType::Link, Some(target)) if value.parse::<Uuid>().is_err() => {
                let title = target.def.title_field.clone().ok_or_else(|| at_row(i, "field", "cannot be matched by name"))?;
                let sql = format!("select id::text from {} where tenant_id = $1 and lower({}::text) = lower($2) limit 2", q(&target.table), q(&title));
                let found: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(ctx.tenant).bind(&value).fetch_all(&mut **tx).await?;
                match found.as_slice() {
                    [one] => one.clone(),
                    [] => return Err(at_row(i, "value", format!("no {} is called {value}", target.def.label))),
                    _ => return Err(at_row(i, "value", format!("more than one {} is called {value}: use its id", target.def.label))),
                }
            }
            _ => value.clone(),
        };
        out.push(Criterion::Employee(field, text));
    }
    Ok(out)
}

/// The active users who match the rule (lower-case emails, in order).
pub async fn matching(ctx: &Ctx, tx: &mut Tx, rule: &Doc) -> Result<Vec<String>> {
    let crit = criteria(ctx, tx, rule).await?;
    let mut qb: QueryBuilder<Postgres>;
    if s(rule, "audience") == Some("Employees") {
        let employee = ctx.schema.entity("Employee").ok_or_else(|| Error::internal("Employee"))?;
        qb = QueryBuilder::new(format!("select lower(e.\"user\"::text) from {} e where e.tenant_id = ", q(&employee.table)));
        qb.push_bind(ctx.tenant).push(
            " and e.status = 'Active' and coalesce(e.\"user\"::text, '') <> '' and exists (select 1 from app_user u \
             where u.tenant_id = e.tenant_id and u.is_active and lower(u.email::text) = lower(e.\"user\"::text))",
        );
        for c in &crit {
            if let Criterion::Employee(field, value) = c {
                qb.push(format!(" and e.{}::text = ", q(field))).push_bind(value.clone());
            }
        }
    } else {
        qb = QueryBuilder::new("select lower(u.email::text) from app_user u where u.is_active and u.tenant_id = ");
        qb.push_bind(ctx.tenant);
        for c in &crit {
            match c {
                Criterion::Role(name) => {
                    qb.push(" and exists (select 1 from user_role r where r.tenant_id = u.tenant_id and r.user_id = u.id and r.role = ")
                        .push_bind(name.clone())
                        .push(")");
                }
                Criterion::Group(name) => {
                    qb.push(
                        " and exists (select 1 from scim_group_member m join scim_group g on g.tenant_id = m.tenant_id and g.id = m.group_id \
                         where m.tenant_id = u.tenant_id and m.user_id = u.id and g.display_name = ",
                    )
                    .push_bind(name.clone())
                    .push(")");
                }
                Criterion::Employee(..) => {}
            }
        }
    }
    qb.push(" order by 1 limit 20000");
    Ok(qb.build_query_scalar().fetch_all(&mut **tx).await?)
}

/// One course a rule enrols into, with the days after matching it is due (0: the course's own).
struct Target {
    course: Uuid,
    days: i64,
}

/// What a rule would do now: who matches, the open courses they would be enrolled in (the rule's course, or
/// each course of its programme) and the (target, person) pairs that have no enrolment yet.
struct Plan {
    matching: Vec<String>,
    programme: Option<Uuid>,
    targets: Vec<Target>,
    missing: Vec<(usize, String)>,
}

async fn plan(k: &Kernel, ctx: &Ctx, tx: &mut Tx, rule: &Doc) -> Result<Plan> {
    let matching = matching(ctx, tx, rule).await?;
    let programme = id(rule, "programme");
    let mut wanted: Vec<(Uuid, i64)> = vec![];
    if let Some(p) = programme {
        for row in rows(&load(k, ctx, tx, "LmsProgramme", p).await?, "courses") {
            if let Some(c) = id(&row, "course") {
                wanted.push((c, if int(&row, "due_days") > 0 { int(&row, "due_days") } else { int(rule, "due_days") }));
            }
        }
    } else if let Some(c) = id(rule, "course") {
        wanted.push((c, int(rule, "due_days")));
    }
    let mut targets = vec![];
    for (c, days) in wanted {
        if load(k, ctx, tx, "LmsCourse", c).await.is_ok_and(|c| s(&c, "status") == Some("Published")) {
            targets.push(Target { course: c, days });
        }
    }
    let mut missing = vec![];
    for (i, t) in targets.iter().enumerate() {
        let have: HashSet<String> = sqlx::query_scalar("select lower(learner) from e_lms_enrollment where tenant_id = $1 and course = $2")
            .bind(ctx.tenant)
            .bind(t.course)
            .fetch_all(&mut **tx)
            .await?
            .into_iter()
            .collect();
        missing.extend(matching.iter().filter(|e| !have.contains(*e)).map(|e| (i, e.clone())));
    }
    Ok(Plan { matching, programme, targets, missing })
}

#[async_trait]
impl DocHook for RuleHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if id(doc, "course").is_some() == id(doc, "programme").is_some() {
            return Err(Error::field("course", "name a course or a programme, not both and not neither"));
        }
        criteria(hc.ctx, hc.tx, doc).await.map(|_| ())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "preview" {
            return Ok(None);
        }
        let p = plan(hc.kernel, hc.ctx, hc.tx, doc).await?;
        let people: HashSet<&String> = p.missing.iter().map(|(_, e)| e).collect();
        Ok(Some(json!({
            "matching": p.matching.len(), "would_enrol": people.len(), "enrolments": p.missing.len(), "open_courses": p.targets.len(),
            "already_enrolled": p.matching.len() - people.len(), "switched_on": b(doc, "active"),
        })))
    }
}

/// Everyone a programme is for: those enrolled through it and those its switched-on rules match now.
pub async fn audience(k: &Kernel, ctx: &Ctx, tx: &mut Tx, programme: Uuid) -> Result<Vec<String>> {
    let mut out: HashSet<String> = sqlx::query_scalar("select lower(learner) from e_lms_enrollment where tenant_id = $1 and programme = $2")
        .bind(ctx.tenant)
        .bind(programme)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .collect();
    let rules: Vec<Uuid> = sqlx::query_scalar("select id from e_lms_enrollment_rule where tenant_id = $1 and programme = $2 and active")
        .bind(ctx.tenant)
        .bind(programme)
        .fetch_all(&mut **tx)
        .await?;
    for r in rules {
        let rule = load(k, ctx, tx, "LmsEnrollmentRule", r).await?;
        match matching(ctx, tx, &rule).await {
            Ok(people) => out.extend(people),
            Err(Error::Validation(_)) => {}
            Err(e) => return Err(e),
        }
    }
    let mut out: Vec<String> = out.into_iter().collect();
    out.sort();
    Ok(out)
}

/// A programme lists each course once.
pub struct ProgrammeHook;

#[async_trait]
impl DocHook for ProgrammeHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut seen = HashSet::new();
        let dup = rows(doc, "courses").iter().position(|r| id(r, "course").is_some_and(|c| !seen.insert(c)));
        match dup {
            Some(i) => Err(Error::Validation(vec![FieldError::new("course", "is listed twice").in_row("courses", i)])),
            None => Ok(()),
        }
    }
}

/// Maintenance: every switched-on rule enrols the people who match and are not enrolled, and withdraws
/// (when it says so) the enrolments it made of people who stopped matching and have not started.
/// Returns how many enrolments it made or withdrew.
pub async fn apply(k: &Kernel, ctx: &Ctx, at: chrono::DateTime<chrono::Utc>) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let rules: Vec<Uuid> = sqlx::query_scalar("select id from e_lms_enrollment_rule where tenant_id = $1 and active order by created_at, id")
        .bind(ctx.tenant)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut changed = 0;
    for rule_id in rules {
        changed += ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let rule = load(k, ctx, &mut tx, "LmsEnrollmentRule", rule_id).await?;
            let p = match plan(k, ctx, &mut tx, &rule).await {
                Ok(p) => p,
                // A rule whose criteria no longer hold (a role deleted) waits for someone to mend it.
                Err(Error::Validation(e)) => {
                    tracing::warn!(rule = %rule_id, "learn: enrolment rule skipped: {e:?}");
                    tx.rollback().await?;
                    return Ok(0);
                }
                Err(e) => return Err(e),
            };
            let mut n = 0;
            for (i, learner) in &p.missing {
                let target = &p.targets[*i];
                let mut extra = Doc::new();
                extra.insert("rule".into(), json!(rule_id.to_string()));
                if let Some(pr) = p.programme {
                    extra.insert("programme".into(), json!(pr.to_string()));
                }
                if target.days > 0 {
                    extra.insert("due_date".into(), json!((util::today_at(ctx, at) + chrono::Duration::days(target.days)).to_string()));
                }
                match enrollment::try_enroll_with(k, ctx, &mut tx, target.course, learner, None, extra).await? {
                    Outcome::Enrolled(_) => n += 1,
                    Outcome::Already(_) => {}
                    Outcome::Refused(why) => tracing::warn!(rule = %rule_id, %learner, "learn: enrolment rule could not enrol: {why}"),
                }
            }
            if s(&rule, "when_no_longer_matching") == Some("Withdraw If Not Started") {
                let kept: HashSet<&String> = p.matching.iter().collect();
                let made: Vec<(Uuid, String)> = sqlx::query_as(
                    "select id, lower(learner) from e_lms_enrollment where tenant_id = $1 and rule = $2 and status = 'Active' \
                     and coalesce(completed_lessons, 0) = 0 and last_activity_at is null",
                )
                .bind(ctx.tenant)
                .bind(rule_id)
                .fetch_all(&mut *tx)
                .await?;
                for (e, learner) in made.into_iter().filter(|(_, l)| !kept.contains(l)) {
                    let mut patch = Doc::new();
                    patch.insert("status".into(), json!(WITHDRAWN));
                    k.update_tx(&elevated(ctx), &mut tx, "LmsEnrollment", e, patch, None).await?;
                    n += 1;
                    tracing::info!(rule = %rule_id, %learner, enrolment = %e, "learn: enrolment withdrawn, the learner no longer matches");
                }
            }
            tx.commit().await?;
            Ok(n)
        })
        .await?;
    }
    Ok(changed)
}
