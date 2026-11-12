//! The gradebook (LM-45): per course, grade items (a quiz's best final attempt, an assignment's latest
//! graded submission less its late penalty, a live session attended, or marks an instructor enters),
//! each learner's marks and the course total by the course's weighted categories, with the lowest
//! items dropped as a category says, excused items left out, and a grading scale's letter. Learners
//! see their own released items and the total of those; instructors their courses' whole book, and
//! an export. A course may ask for a minimum total before it completes.

use crate::enrollment;
use crate::util::{authored_course, d, doc_id, id, int, load, load_locked, rows, s};
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::util::money;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// One learner's mark for one item, as the total counts it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub category: String,
    /// Points earned (none: not graded yet, left out).
    pub earned: Option<Decimal>,
    pub max: Decimal,
    pub excused: bool,
}

/// A category of the course: (name, weight, lowest dropped).
pub type Category = (String, Decimal, i64);

fn round2(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// The course total in percent (none while nothing counts). Without categories: points earned over
/// points possible. With them: each category's earned over possible, its `drop` lowest items (by share)
/// left out while one stays, weighed by the categories that have something graded.
pub fn total(marks: &[Mark], categories: &[Category]) -> Option<Decimal> {
    let counted: Vec<&Mark> = marks.iter().filter(|m| !m.excused && m.earned.is_some() && m.max > Decimal::ZERO).collect();
    if categories.is_empty() {
        let (e, m) = counted.iter().fold((Decimal::ZERO, Decimal::ZERO), |(e, m), x| (e + x.earned.unwrap_or_default(), m + x.max));
        return (!m.is_zero()).then(|| round2(e * Decimal::ONE_HUNDRED / m));
    }
    let (mut sum, mut weights) = (Decimal::ZERO, Decimal::ZERO);
    for (name, weight, drop) in categories {
        let mut items: Vec<&&Mark> = counted.iter().filter(|m| &m.category == name).collect();
        if items.is_empty() {
            continue;
        }
        items.sort_by_key(|a| a.earned.unwrap_or_default() / a.max);
        let keep = items.len().saturating_sub((*drop).max(0) as usize).max(1);
        let kept = &items[items.len() - keep..];
        let (e, m) = kept.iter().fold((Decimal::ZERO, Decimal::ZERO), |(e, m), x| (e + x.earned.unwrap_or_default(), m + x.max));
        sum += *weight * e / m;
        weights += *weight;
    }
    (!weights.is_zero()).then(|| round2(sum * Decimal::ONE_HUNDRED / weights))
}

/// What is taken off a submission `days` late: `per_day` percent a day, at most `most` percent.
pub fn late_penalty(days: i64, per_day: Decimal, most: Decimal) -> Decimal {
    if days <= 0 { Decimal::ZERO } else { (per_day * Decimal::from(days)).min(most).max(Decimal::ZERO) }
}

/// The grade a scale gives a total: the band with the highest start the total reaches.
pub fn letter(bands: &[(String, Decimal)], total: Decimal) -> Option<String> {
    bands.iter().filter(|b| total >= b.1).max_by(|a, b| a.1.cmp(&b.1)).map(|b| b.0.clone())
}

// ───────────────────────────── the book ─────────────────────────────

struct Item {
    id: Uuid,
    title: String,
    category: String,
    source: String,
    quiz: Option<Uuid>,
    assignment: Option<Uuid>,
    lesson: Option<Uuid>,
    max: Decimal,
    due: Option<NaiveDate>,
    per_day: Decimal,
    most: Decimal,
    released: bool,
}

/// The course's gradebook for these enrolments (none: every enrolment not withdrawn). `released_only`:
/// as learners see it.
pub async fn book(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, only: Option<Uuid>, released_only: bool) -> Result<Value> {
    let cid = doc_id(course)?;
    let t = ctx.tenant;
    let items: Vec<Item> = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Decimal,
            Option<NaiveDate>,
            Option<Decimal>,
            Option<Decimal>,
            Option<bool>,
        ),
    >(
        "select id, title, category, source, quiz, assignment, lesson, max_points, due_date, late_penalty_percent, late_penalty_max, released \
         from e_lms_grade_item where tenant_id = $1 and course = $2 order by created_at, id",
    )
    .bind(t)
    .bind(cid)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|(id, title, category, source, quiz, assignment, lesson, max, due, per_day, most, released)| Item {
        id,
        title,
        category: category.unwrap_or_default(),
        source,
        quiz,
        assignment,
        lesson,
        max,
        due,
        per_day: per_day.unwrap_or_default(),
        most: most.unwrap_or(Decimal::ONE_HUNDRED),
        released: released.unwrap_or(true),
    })
    .filter(|i| !released_only || i.released)
    .collect();
    let categories: Vec<Category> =
        rows(course, "grade_categories").iter().filter_map(|r| Some((s(r, "category")?.to_string(), d(r, "weight"), int(r, "drop_lowest")))).collect();
    let learners: Vec<(Uuid, String, Option<String>, Option<NaiveDate>)> = sqlx::query_as(
        "select id, learner, learner_name, enrolled_on from e_lms_enrollment where tenant_id = $1 and course = $2 and ($3::uuid is null or id = $3) \
         and (status <> 'Withdrawn' or $3::uuid is not null) order by lower(coalesce(learner_name, learner)), learner",
    )
    .bind(t)
    .bind(cid)
    .bind(only)
    .fetch_all(&mut **tx)
    .await?;
    let quizzes: HashMap<(Uuid, Uuid), Decimal> = sqlx::query_as::<_, (Uuid, Uuid, Decimal)>(
        "select enrollment, quiz, max(percentage) from e_lms_quiz_attempt where tenant_id = $1 and course = $2 and status in ('Submitted', 'Timed Out') \
         and percentage is not null group by enrollment, quiz",
    )
    .bind(t)
    .bind(cid)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|(e, q, p)| ((e, q), p))
    .collect();
    let submissions: HashMap<(Uuid, Uuid), (Decimal, Decimal, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as::<_, (Uuid, Uuid, Decimal, Decimal, Option<chrono::DateTime<chrono::Utc>>)>(
            "select distinct on (enrollment, assignment) enrollment, assignment, score, max_score, submitted_at from e_lms_assignment_submission \
             where tenant_id = $1 and course = $2 and status = 'Graded' and score is not null order by enrollment, assignment, submission_no desc",
        )
        .bind(t)
        .bind(cid)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|(e, a, sc, mx, at)| ((e, a), (sc, mx, at)))
        .collect();
    let attended: Vec<(Uuid, Uuid)> = sqlx::query_as("select enrollment, lesson from e_lms_lesson_progress where tenant_id = $1 and course = $2")
        .bind(t)
        .bind(cid)
        .fetch_all(&mut **tx)
        .await?;
    let entries: HashMap<(Uuid, Uuid), (Option<Decimal>, bool, Option<String>)> =
        sqlx::query_as::<_, (Uuid, Uuid, Option<Decimal>, Option<bool>, Option<String>)>(
            "select item, enrollment, points, excused, comment from e_lms_grade_entry where tenant_id = $1 and course = $2",
        )
        .bind(t)
        .bind(cid)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|(i, e, p, x, c)| ((i, e), (p, x.unwrap_or(false), c)))
        .collect();
    let mut assignment_docs: HashMap<Uuid, Doc> = HashMap::new();
    for a in items.iter().filter_map(|i| i.assignment) {
        if let std::collections::hash_map::Entry::Vacant(v) = assignment_docs.entry(a) {
            v.insert(load(k, ctx, tx, "LmsAssignment", a).await?);
        }
    }
    let bands: Vec<(String, Decimal)> = match id(course, "grading_scale") {
        Some(sc) => rows(&load(k, ctx, tx, "LmsGradingScale", sc).await?, "bands")
            .iter()
            .filter_map(|b| Some((s(b, "grade")?.to_string(), d(b, "min_percent"))))
            .collect(),
        None => vec![],
    };
    let zone = ctx.schema.timezone.parse::<chrono_tz::Tz>().ok();
    let mut out = Vec::with_capacity(learners.len());
    for (eid, learner, name, enrolled_on) in &learners {
        let mut marks = Vec::with_capacity(items.len());
        let mut shown = Vec::with_capacity(items.len());
        for it in &items {
            let entry = entries.get(&(it.id, *eid));
            let (mut earned, mut late_days, mut penalty) = (None, 0i64, Decimal::ZERO);
            match it.source.as_str() {
                "Quiz" => earned = it.quiz.and_then(|q| quizzes.get(&(*eid, q))).map(|p| round2(*p * it.max / Decimal::ONE_HUNDRED)),
                "Assignment" => {
                    if let Some((score, max, at)) = it.assignment.and_then(|a| submissions.get(&(*eid, a))) {
                        let due = it.due.or_else(|| it.assignment.and_then(|a| assignment_docs.get(&a)).and_then(|a| enrollment::due_date(a, *enrolled_on)));
                        let on = at.map(|x| match zone {
                            Some(z) => x.with_timezone(&z).date_naive(),
                            None => x.date_naive(),
                        });
                        late_days = match (on, due) {
                            (Some(on), Some(due)) => (on - due).num_days().max(0),
                            _ => 0,
                        };
                        penalty = late_penalty(late_days, it.per_day, it.most);
                        let raw = if max.is_zero() { Decimal::ZERO } else { *score * it.max / *max };
                        earned = Some(round2(raw * (Decimal::ONE_HUNDRED - penalty) / Decimal::ONE_HUNDRED));
                    }
                }
                "Live Session" => earned = it.lesson.filter(|l| attended.contains(&(*eid, *l))).map(|_| it.max),
                _ => {}
            }
            let overridden = entry.and_then(|e| e.0);
            if let Some(p) = overridden {
                earned = Some(p.min(it.max));
            }
            let excused = entry.is_some_and(|e| e.1);
            marks.push(Mark { category: it.category.clone(), earned, max: it.max, excused });
            shown.push(json!({
                "item": it.id, "points": earned.map(money), "excused": excused, "late_days": late_days, "late_penalty_percent": money(penalty),
                "overridden": overridden.is_some(), "comment": entry.and_then(|e| e.2.clone()),
            }));
        }
        let total = total(&marks, &categories);
        out.push(json!({
            "enrollment": eid, "learner": learner, "learner_name": name, "marks": shown,
            "total_percent": total.map(money), "grade": total.and_then(|x| letter(&bands, x)),
        }));
    }
    Ok(json!({
        "course": {"id": cid, "title": course.get("title"), "slug": course.get("slug"), "min_grade_percent": course.get("min_grade_percent")},
        "categories": categories.iter().map(|(n, w, dr)| json!({"category": n, "weight": money(*w), "drop_lowest": dr})).collect::<Vec<_>>(),
        "items": items.iter().map(|i| json!({"id": i.id, "title": i.title, "category": i.category, "source": i.source, "max_points": money(i.max), "released": i.released})).collect::<Vec<_>>(),
        "learners": out,
    }))
}

/// One enrolment's course total (every item, released or not), for the completion rule.
pub async fn total_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, enrollment: Uuid) -> Result<Option<Decimal>> {
    let b = book(k, ctx, tx, course, Some(enrollment), false).await?;
    Ok(b["learners"][0]["total_percent"].as_str().and_then(|v| v.parse().ok()))
}

/// The gradebook as CSV: a row a learner, a column an item, then the total and grade.
pub fn csv(book: &Value) -> String {
    let cell = |v: &str| {
        if v.contains([',', '"', '\n', '\r']) || v.starts_with(['=', '+', '-', '@']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() }
    };
    let items = book["items"].as_array().cloned().unwrap_or_default();
    let mut head = vec!["Learner".to_string(), "Email".to_string()];
    head.extend(items.iter().map(|i| cell(i["title"].as_str().unwrap_or_default())));
    head.extend(["Total (%)".to_string(), "Grade".to_string()]);
    let mut out = head.join(",") + "\n";
    for l in book["learners"].as_array().into_iter().flatten() {
        let mut row = vec![cell(l["learner_name"].as_str().unwrap_or_default()), cell(l["learner"].as_str().unwrap_or_default())];
        for m in l["marks"].as_array().into_iter().flatten() {
            row.push(if m["excused"] == json!(true) { "excused".into() } else { m["points"].as_str().unwrap_or_default().to_string() });
        }
        row.push(l["total_percent"].as_str().unwrap_or_default().to_string());
        row.push(cell(l["grade"].as_str().unwrap_or_default()));
        out += &(row.join(",") + "\n");
    }
    out
}

/// A grade of the enrolment changed (a quiz attempt or a submission graded, marks entered): a course
/// that asks for a minimum total may complete now.
pub async fn changed(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: Uuid) -> Result<()> {
    let e = load_locked(k, ctx, tx, "LmsEnrollment", enrollment).await?;
    if s(&e, "status") == Some(enrollment::ACTIVE) {
        enrollment::refresh(k, ctx, tx, &e, None).await?;
    }
    Ok(())
}

// ───────────────────────────── records ─────────────────────────────

/// Grade items: kept by the course's instructors, each naming what it grades in the course and one of
/// its categories.
pub struct ItemHook;

#[async_trait]
impl DocHook for ItemHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(cid) = id(doc, "course") else { return Ok(()) };
        let course = authored_course(hc.kernel, hc.ctx, hc.tx, cid).await?;
        let cats: Vec<String> = rows(&course, "grade_categories").iter().filter_map(|r| s(r, "category").map(str::to_string)).collect();
        match s(doc, "category") {
            Some(c) if !cats.is_empty() && !cats.iter().any(|x| x == c) => {
                return Err(Error::field("category", format!("use one of the course's: {}", cats.join(", "))));
            }
            None if !cats.is_empty() => return Err(Error::field("category", format!("use one of the course's: {}", cats.join(", ")))),
            _ => {}
        }
        for (field, entity) in [("quiz", "LmsQuiz"), ("assignment", "LmsAssignment"), ("lesson", "LmsLesson")] {
            if let Some(x) = id(doc, field)
                && load(hc.kernel, hc.ctx, hc.tx, entity, x).await.ok().and_then(|r| id(&r, "course")) != Some(cid)
            {
                return Err(Error::field(field, "is not of this course"));
            }
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        authored_course(hc.kernel, hc.ctx, hc.tx, id(doc, "course").unwrap_or_default()).await?;
        Ok(())
    }
}

/// Grade entries: an instructor's marks for one learner and item, within the item's points.
pub struct EntryHook;

#[async_trait]
impl DocHook for EntryHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let Some(e) = id(doc, "enrollment") else { return Ok(()) };
        let e = load(hc.kernel, hc.ctx, hc.tx, "LmsEnrollment", e).await?;
        for (to, from) in [("course", "course"), ("learner", "learner"), ("learner_name", "learner_name")] {
            doc.insert(to.into(), e.get(from).cloned().unwrap_or(Value::Null));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let (Some(item), Some(cid)) = (id(doc, "item"), id(doc, "course")) else { return Ok(()) };
        authored_course(hc.kernel, hc.ctx, hc.tx, cid).await?;
        let it = load(hc.kernel, hc.ctx, hc.tx, "LmsGradeItem", item).await?;
        if id(&it, "course") != Some(cid) {
            return Err(Error::field("item", "is another course's"));
        }
        if doc.get("points").is_some_and(|p| !p.is_null()) && d(doc, "points") > d(&it, "max_points") {
            return Err(Error::field("points", format!("at most {}", d(&it, "max_points").normalize())));
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, _old: Option<&Doc>) -> Result<()> {
        if let Some(e) = id(doc, "enrollment") {
            changed(hc.kernel, hc.ctx, hc.tx, e).await?;
        }
        Ok(())
    }
}

/// Whether the enrolment's course total lets it complete (no minimum: yes).
pub async fn may_complete(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, enrollment: Uuid) -> Result<bool> {
    let min = d(course, "min_grade_percent");
    if course.get("min_grade_percent").is_none_or(Value::is_null) || min <= Decimal::ZERO {
        return Ok(true);
    }
    Ok(total_of(k, ctx, tx, course, enrollment).await?.is_some_and(|t| t >= min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: &str) -> Decimal {
        v.parse().unwrap()
    }

    fn m(cat: &str, earned: Option<&str>, max: &str, excused: bool) -> Mark {
        Mark { category: cat.into(), earned: earned.map(n), max: n(max), excused }
    }

    #[test]
    fn totals_by_weight_drops_and_excuses() {
        let marks = vec![
            m("Quizzes", Some("5"), "10", false),
            m("Quizzes", Some("9"), "10", false),
            m("Quizzes", Some("1"), "10", false),
            m("Work", Some("40"), "50", false),
            m("Work", None, "50", false),
            m("Work", Some("0"), "50", true),
        ];
        assert_eq!(total(&marks, &[]), Some(n("68.75")), "55 of 80 points without categories; the ungraded and excused left out");
        let cats = vec![("Quizzes".to_string(), n("40"), 1), ("Work".to_string(), n("60"), 0)];
        // Quizzes: the lowest (1/10) dropped → 14/20 = 70%; Work 80%; 0.4·70 + 0.6·80 = 76.
        assert_eq!(total(&marks, &cats), Some(n("76")));
        let only_quizzes = vec![("Quizzes".to_string(), n("40"), 5), ("Empty".to_string(), n("60"), 0)];
        assert_eq!(total(&marks, &only_quizzes), Some(n("90")), "one item always stays; empty categories weigh nothing");
        assert_eq!(total(&[m("Quizzes", None, "10", false)], &cats), None, "nothing graded");
    }

    #[test]
    fn late_penalties_and_letters() {
        assert_eq!(late_penalty(0, n("10"), n("50")), n("0"));
        assert_eq!(late_penalty(3, n("10"), n("50")), n("30"));
        assert_eq!(late_penalty(9, n("10"), n("50")), n("50"), "capped");
        let bands = vec![("A".to_string(), n("90")), ("B".to_string(), n("75")), ("C".to_string(), n("0"))];
        assert_eq!(letter(&bands, n("90")), Some("A".into()));
        assert_eq!(letter(&bands, n("89.99")), Some("B".into()));
        assert_eq!(letter(&bands, n("10")), Some("C".into()));
        assert_eq!(letter(&[("P".to_string(), n("50"))], n("10")), None);
    }

    #[test]
    fn csv_cells_cannot_become_formulas() {
        let book = json!({"items": [{"title": "Quiz, one"}], "learners": [{"learner_name": "=HYPERLINK()", "learner": "a@x.test",
            "marks": [{"points": "5", "excused": false}], "total_percent": "50", "grade": null}]});
        assert_eq!(csv(&book), "Learner,Email,\"Quiz, one\",Total (%),Grade\n\"=HYPERLINK()\",a@x.test,5,50,\n");
    }
}
