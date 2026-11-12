//! Quizzes, question banks and attempts.
//!
//! Answer keys live only in `LmsQuestion` (options marked correct, accepted short answers) — an
//! entity learners have no permission on, so no read, list, filter, export, audit trail or MCP call
//! of theirs reaches them. A learner starts an attempt with `start_quiz` on their enrolment; the
//! attempt carries the questions as shown (text, options under per-attempt keys, points) and nothing
//! that tells which answer is right until it is submitted. The clock is the server's: the deadline
//! is fixed when the attempt starts, answers arriving after it (plus a short grace for the network)
//! are not accepted, and maintenance closes attempts whose time ran out. Starting takes the
//! enrolment's row lock, so the attempt limit holds under concurrent requests; scores are exact.

use crate::enrollment;
use crate::grading::{self, BLANKS, ESSAY, FILE, Key, MATCHING, MULTIPLE, NUMERIC, ORDERING, SHORT, SINGLE, TRUE_FALSE};
use crate::util::{self, authored_course, b, d, doc_id, elevated, id, int, load, load_locked, rows, s, same_email};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::money;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx};
use rand::seq::SliceRandom;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

pub const IN_PROGRESS: &str = "In Progress";
pub const SUBMITTED: &str = "Submitted";
pub const TIMED_OUT: &str = "Timed Out";
pub const AWAITING: &str = "Awaiting Grading";
/// Answers this long after the deadline still count (network latency); later ones do not.
pub const GRACE_SECONDS: i64 = 15;
const MAX_OPTIONS: usize = 20;
const MAX_ACCEPTED: usize = 50;

// ───────────────────────────── quizzes ─────────────────────────────

pub struct QuizHook;

#[async_trait]
impl DocHook for QuizHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let mut list = rows(doc, "questions");
        let ids: Vec<Uuid> = list.iter().filter_map(|r| id(r, "question")).collect();
        let defaults: Vec<(Uuid, Decimal)> = if ids.is_empty() {
            vec![]
        } else {
            sqlx::query_as("select id, coalesce(points, 1) from e_lms_question where tenant_id = $1 and id = any($2)")
                .bind(hc.ctx.tenant)
                .bind(&ids)
                .fetch_all(&mut **hc.tx)
                .await?
        };
        let mut total = Decimal::ZERO;
        for r in list.iter_mut() {
            if d(r, "points") <= Decimal::ZERO
                && let Some(q) = id(r, "question")
                && let Some((_, p)) = defaults.iter().find(|(i, _)| *i == q)
            {
                r.insert("points".into(), money(*p));
            }
            total += d(r, "points");
        }
        // Drawn questions count too; their points are the draw's (or each question's own, unknown until drawn).
        let draws = rows(doc, "draws");
        let drawn: i64 = draws.iter().map(|r| int(r, "count").max(0)).sum();
        total += draws.iter().map(|r| d(r, "points") * Decimal::from(int(r, "count").max(0))).sum::<Decimal>();
        doc.insert("question_count".into(), json!(list.len() as i64 + drawn));
        doc.insert("total_points".into(), money(total));
        doc.insert("questions".into(), Value::Array(list.into_iter().map(Value::Object).collect()));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(course) = id(doc, "course") else { return Ok(()) };
        authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        let list = rows(doc, "questions");
        let ids: Vec<Uuid> = list.iter().filter_map(|r| id(r, "question")).collect();
        let owners: Vec<(Uuid, Uuid)> = if ids.is_empty() {
            vec![]
        } else {
            sqlx::query_as("select id, course from e_lms_question where tenant_id = $1 and id = any($2)")
                .bind(hc.ctx.tenant)
                .bind(&ids)
                .fetch_all(&mut **hc.tx)
                .await?
        };
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for (i, q) in list.iter().filter_map(|r| id(r, "question")).enumerate() {
            if !seen.insert(q) {
                errors.push(FieldError::new("question", "is in the quiz twice").in_row("questions", i));
            } else if owners.iter().any(|(qid, c)| *qid == q && *c != course) {
                errors.push(FieldError::new("question", "belongs to another course's question bank").in_row("questions", i));
            }
        }
        let per = int(doc, "questions_per_attempt");
        if per > 0 && per as usize > list.len() {
            errors.push(FieldError::new("questions_per_attempt", format!("the quiz has only {} questions", list.len())));
        }
        for (i, draw) in rows(doc, "draws").iter().enumerate() {
            let Some(category) = id(draw, "category") else { continue };
            let there = bank(hc.tx, hc.ctx.tenant, course, category, s(draw, "tag"), &[], i64::MAX).await?.len() as i64;
            if there < int(draw, "count") {
                errors.push(FieldError::new("count", format!("the course's bank has {there} such questions")).in_row("draws", i));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        Ok(())
    }
}

// ───────────────────────────── questions ─────────────────────────────

pub struct QuestionHook;

#[async_trait]
impl DocHook for QuestionHook {
    async fn before_validate(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // True/False questions always offer exactly "True" and "False"; the author marks one.
        if s(doc, "question_type") == Some(TRUE_FALSE) {
            let current = rows(doc, "options");
            let mut out = Vec::with_capacity(2);
            for (i, label) in ["True", "False"].into_iter().enumerate() {
                let mut row = current.iter().find(|r| s(r, "option").is_some_and(|o| o.trim().eq_ignore_ascii_case(label))).cloned().unwrap_or_default();
                if id(&row, "id").is_none() {
                    row.insert("id".into(), json!(Uuid::now_v7().to_string()));
                }
                let correct = b(&row, "is_correct");
                row.insert("option".into(), json!(label));
                row.insert("is_correct".into(), json!(correct));
                row.insert("idx".into(), json!(i));
                out.push(Value::Object(row));
            }
            doc.insert("options".into(), Value::Array(out));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(course) = id(doc, "course") else { return Ok(()) };
        authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        let qtype = s(doc, "question_type").unwrap_or(SINGLE);
        let options = rows(doc, "options");
        let accepted = rows(doc, "accepted_answers");
        let mut errors = Vec::new();
        if let Some(rubric) = id(doc, "rubric") {
            if !grading::by_person(qtype) {
                errors.push(FieldError::new("rubric", "only essays and files are graded with a rubric"));
            } else if load(hc.kernel, hc.ctx, hc.tx, "LmsRubric", rubric).await.ok().and_then(|r| id(&r, "course")) != Some(course) {
                errors.push(FieldError::new("rubric", "is another course's rubric"));
            }
        }
        if accepted.iter().any(|a| int(a, "blank") > 0) && qtype != BLANKS {
            errors.push(FieldError::new("accepted_answers", "only fill-in questions answer blanks"));
        }
        if matches!(qtype, NUMERIC | ORDERING | MATCHING | BLANKS | ESSAY | FILE) {
            let needs_options = matches!(qtype, ORDERING | MATCHING);
            if needs_options && (options.len() < 2 || options.len() > MAX_OPTIONS) {
                errors.push(FieldError::new("options", format!("give 2 to {MAX_OPTIONS} options")));
            }
            if !needs_options && !options.is_empty() {
                errors.push(FieldError::new("options", format!("{qtype} questions have no options")));
            }
            if qtype != BLANKS && !accepted.is_empty() {
                errors.push(FieldError::new("accepted_answers", "only short-answer and fill-in questions have accepted answers"));
            }
            if qtype == MATCHING {
                let mut seen = HashSet::new();
                for (i, o) in options.iter().enumerate() {
                    match s(o, "match_text") {
                        None => errors.push(FieldError::new("match_text", "give what it matches").in_row("options", i)),
                        Some(m) if !seen.insert(grading::normalize_answer(m, false)) => {
                            errors.push(FieldError::new("match_text", "two items match the same answer").in_row("options", i))
                        }
                        _ => {}
                    }
                }
            }
            if qtype == BLANKS {
                let n = grading::blanks_in(s(doc, "question").unwrap_or_default());
                if n == 0 {
                    errors.push(FieldError::new("question", "mark the blanks in the question as [[1]], [[2]] and so on"));
                }
                for b in 1..=n as i64 {
                    if !accepted.iter().any(|a| int(a, "blank") == b) {
                        errors.push(FieldError::new("accepted_answers", format!("give an accepted answer for blank {b}")));
                    }
                }
                for (i, a) in accepted.iter().enumerate() {
                    if int(a, "blank") < 1 || int(a, "blank") > n as i64 {
                        errors.push(FieldError::new("blank", format!("name a blank from 1 to {n}")).in_row("accepted_answers", i));
                    }
                }
            }
            return if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) };
        }
        if qtype == SHORT {
            if !options.is_empty() {
                errors.push(FieldError::new("options", "short-answer questions have accepted answers, not options"));
            }
            if accepted.is_empty() {
                errors.push(FieldError::new("accepted_answers", "add at least one accepted answer"));
            }
            if accepted.len() > MAX_ACCEPTED {
                errors.push(FieldError::new("accepted_answers", format!("at most {MAX_ACCEPTED} accepted answers")));
            }
        } else {
            if !accepted.is_empty() {
                errors.push(FieldError::new("accepted_answers", "only short-answer questions have accepted answers"));
            }
            if options.len() < 2 || options.len() > MAX_OPTIONS {
                errors.push(FieldError::new("options", format!("give 2 to {MAX_OPTIONS} options")));
            }
            let mut seen = HashSet::new();
            for (i, o) in options.iter().enumerate() {
                if !seen.insert(grading::normalize_answer(s(o, "option").unwrap_or_default(), false)) {
                    errors.push(FieldError::new("option", "two options read the same").in_row("options", i));
                }
            }
            let correct = options.iter().filter(|o| b(o, "is_correct")).count();
            match qtype {
                MULTIPLE if correct == 0 => errors.push(FieldError::new("options", "mark at least one option correct")),
                MULTIPLE => {}
                _ if correct != 1 => errors.push(FieldError::new("options", "mark exactly one option correct")),
                _ => {}
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let Some(old) = old else { return Ok(()) };
        let me = doc_id(doc)?;
        if old.get("question_type") != doc.get("question_type") {
            let answered: bool = sqlx::query_scalar("select exists(select 1 from e_lms_quiz_attempt__answers where tenant_id = $1 and question = $2)")
                .bind(hc.ctx.tenant)
                .bind(me)
                .fetch_one(&mut **hc.tx)
                .await?;
            if answered {
                return Err(Error::field("question_type", "learners have answered this question: write a new question instead of changing its type"));
            }
        }
        if old.get("course") != doc.get("course") {
            let used: bool = sqlx::query_scalar("select exists(select 1 from e_lms_quiz__questions where tenant_id = $1 and question = $2)")
                .bind(hc.ctx.tenant)
                .bind(me)
                .fetch_one(&mut **hc.tx)
                .await?;
            if used {
                return Err(Error::field("course", "a quiz uses this question: it cannot move to another course"));
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
}

// ───────────────────────────── attempts ─────────────────────────────

pub struct AttemptHook;

#[async_trait]
impl DocHook for AttemptHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid(
                "quiz attempts are started with the start_quiz action of an enrolment and answered with save_answers and submit_answers",
            ));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let (k, ctx) = (hc.kernel, hc.ctx);
        match name {
            "save_answers" | "submit_answers" => {
                if !same_email(s(doc, "learner").unwrap_or_default(), &ctx.email) {
                    return Err(Error::Forbidden("only the learner answers their attempt".into()));
                }
                if s(doc, "status") != Some(IN_PROGRESS) {
                    return Err(Error::invalid("this attempt is already submitted"));
                }
                let now = util::now();
                if late(doc, now) {
                    // Time is up: the answers saved in time count, these do not.
                    let done = finalize(k, ctx, hc.tx, doc, TIMED_OUT, now).await?;
                    return Ok(Some(result(&done, true)));
                }
                let mut answers = rows(doc, "answers");
                let mut files = Vec::new();
                let changed = match params.get("answers") {
                    None | Some(Value::Null) => false,
                    Some(a) => apply_listing(&mut answers, a, &mut files)?,
                };
                // A file answer is one the learner uploaded themselves.
                for f in files {
                    let theirs: bool = sqlx::query_scalar("select exists(select 1 from file_object where tenant_id = $1 and id = $2 and created_by = $3)")
                        .bind(ctx.tenant)
                        .bind(f)
                        .bind(ctx.user)
                        .fetch_one(&mut **hc.tx)
                        .await?;
                    if !theirs {
                        return Err(Error::field("answers", "send a file you uploaded with /api/files"));
                    }
                }
                let saved = if changed {
                    let mut patch = Doc::new();
                    patch.insert("answers".into(), Value::Array(answers.into_iter().map(Value::Object).collect()));
                    k.update_tx(&elevated(ctx), hc.tx, "LmsQuizAttempt", doc_id(doc)?, patch, None).await?
                } else {
                    doc.clone()
                };
                if name == "save_answers" {
                    return Ok(Some(json!({"attempt": view(&saved, now)})));
                }
                let done = finalize(k, ctx, hc.tx, &saved, SUBMITTED, now).await?;
                Ok(Some(result(&done, false)))
            }
            "review" => {
                authored_course(k, ctx, hc.tx, id(doc, "course").unwrap_or_default()).await?;
                Ok(Some(review(k, ctx, hc.tx, doc).await?))
            }
            "grade_answers" => Ok(Some(grade_answers(k, ctx, hc.tx, doc, params).await?)),
            _ => Ok(None),
        }
    }
}

/// Whether the attempt's time (plus the grace) has run out at `now`.
fn late(attempt: &Doc, now: DateTime<Utc>) -> bool {
    util::time(attempt, "deadline").is_some_and(|d| now > d + Duration::seconds(GRACE_SECONDS))
}

/// The attempt as its learner sees it, with the seconds left on the clock.
fn view(attempt: &Doc, now: DateTime<Utc>) -> Value {
    let mut v = Value::Object(attempt.clone());
    let left = match (s(attempt, "status"), util::time(attempt, "deadline")) {
        (Some(IN_PROGRESS), Some(dl)) => json!((dl - now).num_seconds().max(0)),
        _ => Value::Null,
    };
    v["seconds_left"] = left;
    v
}

fn result(attempt: &Doc, late_answers_ignored: bool) -> Value {
    json!({
        "attempt": view(attempt, util::now()),
        "status": attempt.get("status"), "score": attempt.get("score"), "max_score": attempt.get("max_score"),
        "percentage": attempt.get("percentage"), "passed": attempt.get("passed"), "late_answers_ignored": late_answers_ignored,
    })
}

/// Applies `{question id: response}` to the attempt's rows, listing the files the answers name (whose they are is checked against the database).
fn apply_listing(rows: &mut [Doc], answers: &Value, files: &mut Vec<Uuid>) -> Result<bool> {
    let Value::Object(map) = answers else {
        return Err(Error::field("answers", "send an object of question id → answer"));
    };
    let mut changed = false;
    for (question, response) in map {
        let Some(row) = rows.iter_mut().find(|r| s(r, "question") == Some(question.as_str())) else {
            return Err(Error::field("answers", format!("question {} is not part of this attempt", util::cut(question, 40))));
        };
        let qtype = s(row, "question_type").unwrap_or(SINGLE).to_string();
        let keys = |list: &str| -> Vec<String> {
            row.get(list).and_then(Value::as_array).map(|a| a.iter().filter_map(|o| o.get("key")?.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        let (shown, right) = (keys("options"), keys("choices"));
        let blanks = int(row, "blanks").max(0) as usize;
        let value =
            grading::parse_response_of(&qtype, &shown, &right, blanks, response).map_err(|m| Error::field("answers", format!("question {question}: {m}")))?;
        if let Some(f) = value.as_str().filter(|_| qtype == FILE).and_then(util::file_id) {
            files.push(f);
        }
        if row.get("response") != Some(&value) {
            row.insert("response".into(), value);
            changed = true;
        }
    }
    Ok(changed)
}

/// The answer key of a bank question for one attempt, and what to show once answers are revealed.
fn key_of(attempt: Uuid, question: &Doc) -> (Key, Vec<(String, String)>) {
    let qtype = s(question, "question_type").unwrap_or(SINGLE).to_string();
    let rows_ = rows(question, "options");
    let options: Vec<(String, String, bool)> =
        rows_.iter().filter_map(|o| Some((grading::option_key(attempt, id(o, "id")?), s(o, "option")?.to_string(), b(o, "is_correct")))).collect();
    let correct: BTreeSet<String> = options.iter().filter(|o| o.2).map(|o| o.0.clone()).collect();
    let answers = rows(question, "accepted_answers");
    let texts = |blank: i64, pattern: bool| -> Vec<String> {
        answers.iter().filter(|a| int(a, "blank") == blank && b(a, "pattern") == pattern).filter_map(|a| s(a, "answer").map(str::to_string)).collect()
    };
    let blanks = (1..=grading::blanks_in(s(question, "question").unwrap_or_default()) as i64).map(|n| (texts(n, false), texts(n, true))).collect();
    let pairs = rows_.iter().filter_map(|o| Some((grading::option_key(attempt, id(o, "id")?), grading::match_key(attempt, id(o, "id")?)))).collect();
    let numeric = (qtype == NUMERIC).then(|| (d(question, "numeric_answer"), d(question, "tolerance")));
    let key = Key {
        order: options.iter().map(|o| o.0.clone()).collect(),
        qtype,
        correct,
        accepted: texts(0, false),
        patterns: texts(0, true),
        case_sensitive: b(question, "case_sensitive"),
        pairs,
        blanks,
        numeric,
    };
    let mut shown: Vec<(String, String)> = options.into_iter().map(|(k, t, _)| (k, t)).collect();
    // A matching question's answers are named by their own keys.
    shown.extend(rows_.iter().filter_map(|o| Some((grading::match_key(attempt, id(o, "id")?), s(o, "match_text")?.to_string()))));
    (key, shown)
}

/// What a revealed row shows as the correct answer: the correct options (as shown) or the
/// accepted short answers.
fn correct_answer(key: &Key, row: &Doc) -> Value {
    let text_of = |list: &str, k: &str| -> Value {
        row.get(list)
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|o| o.get("key").and_then(Value::as_str) == Some(k)))
            .and_then(|o| o.get("text").cloned())
            .unwrap_or(Value::Null)
    };
    match key.qtype.as_str() {
        SHORT => return json!(key.accepted.iter().chain(&key.patterns).collect::<Vec<_>>()),
        NUMERIC => return key.numeric.map_or(Value::Null, |(a, t)| json!({"answer": a.normalize().to_string(), "tolerance": t.normalize().to_string()})),
        ORDERING => return Value::Array(key.order.iter().map(|k| text_of("options", k)).collect()),
        MATCHING => return Value::Array(key.pairs.iter().map(|(l, r)| json!({"item": text_of("options", l), "matches": text_of("choices", r)})).collect()),
        BLANKS => return Value::Array(key.blanks.iter().map(|(a, p)| json!(a.iter().chain(p).collect::<Vec<_>>())).collect()),
        ESSAY | FILE => return Value::Null,
        _ => {}
    }
    let shown = row.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
    Value::Array(
        shown.iter().filter(|o| o.get("key").and_then(Value::as_str).is_some_and(|k| key.correct.contains(k))).filter_map(|o| o.get("text").cloned()).collect(),
    )
}

/// Grades the attempt against the current answer keys and closes it as `status` (with the quiz's
/// partial credit and negative marking). Answers a person grades wait for them: the attempt is then
/// Awaiting Grading, its result not final, until [`grade_answers`] grades the last of them.
async fn finalize(k: &Kernel, ctx: &Ctx, tx: &mut Tx, attempt: &Doc, status: &str, now: DateTime<Utc>) -> Result<Doc> {
    let me = doc_id(attempt)?;
    let quiz = load(k, ctx, tx, "LmsQuiz", id(attempt, "quiz").ok_or_else(|| Error::internal("attempt without quiz"))?).await?;
    let (partial, negative) = (b(&quiz, "partial_credit"), d(&quiz, "negative_marking"));
    let mut answers = rows(attempt, "answers");
    for row in answers.iter_mut() {
        if b(row, "by_person") {
            continue;
        }
        let question = match id(row, "question") {
            Some(q) => Some(load(k, ctx, tx, "LmsQuestion", q).await?),
            None => None,
        };
        let response = row.get("response").cloned().unwrap_or(Value::Null);
        let fraction = question.as_ref().map(|q| grading::fraction(&key_of(me, q).0, &response)).unwrap_or_default();
        let points = grading::award(d(row, "points"), fraction, !response.is_null(), partial, negative);
        row.insert("is_correct".into(), json!(fraction == Decimal::ONE));
        row.insert("points_awarded".into(), money(points));
    }
    let mut patch = Map::new();
    patch.insert("submitted_at".into(), json!(util::ts(now)));
    patch.insert("closed_as".into(), json!(status));
    patch.insert("answers".into(), Value::Array(answers.into_iter().map(Value::Object).collect()));
    let graded = k.update_tx(&elevated(ctx), tx, "LmsQuizAttempt", me, patch, None).await?;
    close(k, ctx, tx, &graded, &quiz).await
}

/// Settles an attempt whose answers are graded: its score, whether it passed, the answers revealed
/// when the quiz says so and, when it passed, the quiz's lessons completed. While an answer a person
/// grades is still ungraded it is Awaiting Grading, with nothing revealed and nothing passed.
async fn close(k: &Kernel, ctx: &Ctx, tx: &mut Tx, attempt: &Doc, quiz: &Doc) -> Result<Doc> {
    let me = doc_id(attempt)?;
    let quiz_id = doc_id(quiz)?;
    let mut answers = rows(attempt, "answers");
    let waiting = answers.iter().any(|r| b(r, "by_person") && r.get("points_awarded").is_none_or(Value::is_null));
    let score = grading::attempt_score(answers.iter().map(|r| d(r, "points_awarded")));
    let max = d(attempt, "max_score");
    let mut patch = Map::new();
    patch.insert("score".into(), money(score));
    patch.insert("percentage".into(), money(grading::percentage(score, max)));
    if waiting {
        patch.insert("status".into(), json!(AWAITING));
        patch.insert("passed".into(), json!(false));
        return k.update_tx(&elevated(ctx), tx, "LmsQuizAttempt", me, patch, None).await;
    }
    let passed = grading::passed(score, max, d(attempt, "passing_percentage"));
    let used: i64 = sqlx::query_scalar("select count(*) from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and enrollment = $3")
        .bind(ctx.tenant)
        .bind(quiz_id)
        .bind(id(attempt, "enrollment"))
        .fetch_one(&mut **tx)
        .await?;
    let max_attempts = int(quiz, "max_attempts");
    let reveal = match s(quiz, "show_answers") {
        Some("After Each Attempt") => true,
        Some("Never") => false,
        _ => passed || (max_attempts > 0 && used >= max_attempts),
    };
    if reveal {
        for row in answers.iter_mut() {
            let Some(q) = id(row, "question") else { continue };
            let question = load(k, ctx, tx, "LmsQuestion", q).await?;
            let key = key_of(me, &question).0;
            row.insert("correct_answer".into(), correct_answer(&key, row));
            row.insert("explanation".into(), question.get("explanation").cloned().unwrap_or(Value::Null));
        }
    }
    patch.insert("status".into(), json!(s(attempt, "closed_as").unwrap_or(SUBMITTED)));
    patch.insert("passed".into(), json!(passed));
    patch.insert("answers".into(), Value::Array(answers.into_iter().map(Value::Object).collect()));
    let done = k.update_tx(&elevated(ctx), tx, "LmsQuizAttempt", me, patch, None).await?;
    if passed && let (Some(enrollment), Some(course)) = (id(attempt, "enrollment"), id(attempt, "course")) {
        let lessons: Vec<Uuid> = sqlx::query_scalar("select id from e_lms_lesson where tenant_id = $1 and quiz = $2 and course = $3")
            .bind(ctx.tenant)
            .bind(quiz_id)
            .bind(course)
            .fetch_all(&mut **tx)
            .await?;
        for lesson in lessons {
            enrollment::record_result(k, ctx, tx, enrollment, lesson, "Quiz").await?;
        }
    }
    Ok(done)
}

/// The course's bank questions of a category (with a tag, when given), not in `taken`, at most
/// `limit` of them at random.
async fn bank(tx: &mut Tx, tenant: Uuid, course: Uuid, category: Uuid, tag: Option<&str>, taken: &[Uuid], limit: i64) -> Result<Vec<Uuid>> {
    let tag = tag.and_then(|t| util::tag_list(t).into_iter().next());
    Ok(sqlx::query_scalar(
        "select id from e_lms_question where tenant_id = $1 and course = $2 and category = $3 and not (id = any($4)) \
         and ($5::text is null or position(', ' || $5 || ',' in ', ' || coalesce(tags, '') || ',') > 0) order by random() limit $6",
    )
    .bind(tenant)
    .bind(course)
    .bind(category)
    .bind(taken)
    .bind(tag)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?)
}

/// `grade_answers {question, points? | rubric?, comment?}` by the course's instructors: grades one
/// answer a person grades. With the quiz's second grader, the first two graders' points must agree
/// (each grader grades once); when they differ, a third person's grade settles it. Once every such
/// answer has its grade, the attempt's result is final.
async fn grade_answers(k: &Kernel, ctx: &Ctx, tx: &mut Tx, attempt: &Doc, params: &Value) -> Result<Value> {
    authored_course(k, ctx, tx, id(attempt, "course").unwrap_or_default()).await?;
    // Graders of one attempt take turns.
    let attempt = &load_locked(k, ctx, tx, "LmsQuizAttempt", doc_id(attempt)?).await?;
    if s(attempt, "status") != Some(AWAITING) {
        return Err(Error::invalid("this attempt has nothing waiting to be graded"));
    }
    let quiz = load(k, ctx, tx, "LmsQuiz", id(attempt, "quiz").unwrap_or_default()).await?;
    let question = params.get("question").and_then(Value::as_str).ok_or_else(|| Error::field("question", "is required (a question id)"))?;
    let mut answers = rows(attempt, "answers");
    let row = answers
        .iter_mut()
        .find(|r| s(r, "question") == Some(question) && b(r, "by_person"))
        .ok_or_else(|| Error::field("question", "is not a question of this attempt a person grades"))?;
    if row.get("points_awarded").is_some_and(|v| !v.is_null()) {
        return Err(Error::field("question", "is graded already"));
    }
    let out_of = d(row, "points");
    let q = load(k, ctx, tx, "LmsQuestion", id(row, "question").unwrap_or_default()).await?;
    let (points, filled) = match (id(&q, "rubric"), params.get("rubric").filter(|v| !v.is_null())) {
        (Some(rubric), Some(chosen)) => {
            let (filled, points) = fill(k, ctx, tx, rubric, chosen, out_of).await?;
            (points, Some(filled))
        }
        (None, Some(_)) => return Err(Error::field("rubric", "this question has no rubric")),
        (_, None) => {
            let p = params.get("points").and_then(grading::parse_number).ok_or_else(|| Error::field("points", "is required (a number)"))?;
            if p < Decimal::ZERO || p > out_of {
                return Err(Error::field("points", format!("must be between 0 and {}", out_of.normalize())));
            }
            (p, None)
        }
    };
    let comment = match params.get("comment") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(c)) if c.chars().count() <= 20_000 => json!(c),
        Some(_) => return Err(Error::field("comment", "write at most 20000 characters of text")),
    };
    let me = ctx.email.to_lowercase();
    let mut comment = comment;
    let mut fin = true;
    if b(&quiz, "second_marking") {
        // Grades not yet final are kept apart from the attempt, so its learner never reads them.
        let notes: Vec<(String, Decimal, Option<String>)> = sqlx::query_as(
            "select grader, points, comment from lms_grading_note where tenant_id = $1 and attempt = $2 and question = $3 order by graded_at, grader",
        )
        .bind(ctx.tenant)
        .bind(doc_id(attempt)?)
        .bind(id(row, "question"))
        .fetch_all(&mut **tx)
        .await?;
        if notes.iter().any(|n| n.0 == me) {
            return Err(Error::invalid(if notes.len() == 1 {
                "a second grader grades it: someone else"
            } else {
                "the two graders differ: a third person settles it"
            }));
        }
        fin = match notes.as_slice() {
            [] => false,
            [first] => first.1 == points,
            _ => true,
        };
        if let ([first], true, true) = (notes.as_slice(), fin, comment.is_null()) {
            comment = first.2.clone().map_or(Value::Null, Value::String);
        }
        if fin {
            sqlx::query("delete from lms_grading_note where tenant_id = $1 and attempt = $2 and question = $3")
                .bind(ctx.tenant)
                .bind(doc_id(attempt)?)
                .bind(id(row, "question"))
                .execute(&mut **tx)
                .await?;
        } else {
            sqlx::query("insert into lms_grading_note (tenant_id, attempt, question, grader, points, comment) values ($1, $2, $3, $4, $5, $6)")
                .bind(ctx.tenant)
                .bind(doc_id(attempt)?)
                .bind(id(row, "question"))
                .bind(&me)
                .bind(points)
                .bind(comment.as_str())
                .execute(&mut **tx)
                .await?;
            return Ok(json!({"attempt": attempt, "waiting_for": if notes.is_empty() { "a second grader" } else { "a third grader to settle it" }}));
        }
    }
    if fin {
        if !comment.is_null() {
            row.insert("comment".into(), comment);
        }
        if let Some(f) = filled {
            row.insert("rubric_result".into(), f);
        }
        row.insert("points_awarded".into(), money(points));
        row.insert("is_correct".into(), json!(points == out_of));
        row.insert("graded_by".into(), json!(me));
        row.insert("graded_at".into(), json!(util::ts(util::now())));
    }
    let mut patch = Map::new();
    patch.insert("answers".into(), Value::Array(answers.into_iter().map(Value::Object).collect()));
    let saved = k.update_tx(&elevated(ctx), tx, "LmsQuizAttempt", doc_id(attempt)?, patch, None).await?;
    let done = close(k, ctx, tx, &saved, &quiz).await?;
    if let Some(e) = id(&done, "enrollment") {
        crate::gradebook::changed(k, ctx, tx, e).await?;
    }
    Ok(json!({"attempt": done}))
}

/// A rubric filled for `out_of` points: the levels chosen (an object, or its JSON as text).
pub async fn fill(k: &Kernel, ctx: &Ctx, tx: &mut Tx, rubric: Uuid, chosen: &Value, out_of: Decimal) -> Result<(Value, Decimal)> {
    let chosen = match chosen {
        Value::String(t) => serde_json::from_str(t).map_err(|_| Error::field("rubric", "send the levels as {\"criterion\": \"level\"}"))?,
        other => other.clone(),
    };
    let r = load(k, ctx, tx, "LmsRubric", rubric).await?;
    let criteria: Vec<String> = rows(&r, "criteria").iter().filter_map(|c| s(c, "criterion").map(str::to_string)).collect();
    let levels: grading::Levels =
        rows(&r, "levels").iter().filter_map(|l| Some((s(l, "criterion")?.to_string(), s(l, "level")?.to_string(), d(l, "points")))).collect();
    grading::fill_rubric(&criteria, &levels, &chosen, out_of).map_err(|m| Error::field("rubric", m))
}

/// Starts (or resumes) the learner's attempt at `quiz` for their enrolment, which the caller holds
/// locked: attempts of one learner are therefore counted one request at a time.
pub async fn start(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: &Doc, quiz_id: Uuid) -> Result<Value> {
    let course = id(enrollment, "course").ok_or_else(|| Error::internal("enrolment without course"))?;
    let learner = s(enrollment, "learner").unwrap_or_default().to_lowercase();
    let quiz = match load(k, ctx, tx, "LmsQuiz", quiz_id).await {
        Ok(q) if id(&q, "course") == Some(course) => q,
        Ok(_) | Err(Error::NotFound(_)) => return Err(Error::field("quiz", "is not a quiz of this course")),
        Err(e) => return Err(e),
    };
    let placed: bool = sqlx::query_scalar("select exists(select 1 from e_lms_lesson where tenant_id = $1 and quiz = $2 and course = $3)")
        .bind(ctx.tenant)
        .bind(quiz_id)
        .bind(course)
        .fetch_one(&mut **tx)
        .await?;
    if !placed {
        return Err(Error::field("quiz", "is not part of a lesson of this course yet"));
    }
    let now = util::now();
    let past: Vec<(Uuid, String, i64)> =
        sqlx::query_as("select id, status, attempt_no from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and enrollment = $3 order by attempt_no")
            .bind(ctx.tenant)
            .bind(quiz_id)
            .bind(doc_id(enrollment)?)
            .fetch_all(&mut **tx)
            .await?;
    if let Some((open, ..)) = past.iter().find(|a| a.1 == IN_PROGRESS) {
        let doc = load_locked(k, ctx, tx, "LmsQuizAttempt", *open).await?;
        if !late(&doc, now) {
            return Ok(json!({"resumed": true, "attempt": view(&doc, now)}));
        }
        finalize(k, ctx, tx, &doc, TIMED_OUT, now).await?;
    }
    let max_attempts = int(&quiz, "max_attempts");
    if max_attempts > 0 && past.len() as i64 >= max_attempts {
        return Err(Error::invalid(format!("you have used all {max_attempts} attempts at this quiz")));
    }
    let mut picked = rows(&quiz, "questions");
    let per = int(&quiz, "questions_per_attempt").max(0) as usize;
    if per > 0 && per < picked.len() {
        let mut order: Vec<usize> = (0..picked.len()).collect();
        order.shuffle(&mut rand::rng());
        order.truncate(per);
        order.sort_unstable();
        picked = order.into_iter().map(|i| picked[i].clone()).collect();
    }
    // The listed questions, then those drawn from the bank by category: what this attempt asks is kept with it.
    let mut chosen: Vec<(Uuid, Decimal)> = picked.iter().filter_map(|r| Some((id(r, "question")?, d(r, "points")))).collect();
    for draw in rows(&quiz, "draws") {
        let Some(category) = id(&draw, "category") else { continue };
        let taken: Vec<Uuid> = chosen.iter().map(|c| c.0).collect();
        for q in bank(tx, ctx.tenant, course, category, s(&draw, "tag"), &taken, int(&draw, "count")).await? {
            chosen.push((q, d(&draw, "points")));
        }
    }
    if chosen.is_empty() {
        return Err(Error::invalid("this quiz has no questions yet"));
    }
    if b(&quiz, "shuffle_questions") {
        chosen.shuffle(&mut rand::rng());
    }
    let attempt = Uuid::now_v7();
    let mut answers = Vec::with_capacity(chosen.len());
    let mut max_score = Decimal::ZERO;
    for (qid, points) in chosen {
        let q = load(k, ctx, tx, "LmsQuestion", qid).await?;
        let qtype = s(&q, "question_type").unwrap_or(SINGLE).to_string();
        let mut options: Vec<Value> =
            rows(&q, "options").iter().filter_map(|o| Some(json!({"key": grading::option_key(attempt, id(o, "id")?), "text": s(o, "option")?}))).collect();
        let mut choices: Vec<Value> = vec![];
        match qtype.as_str() {
            // Never in the right order: shuffled, and turned once more when the shuffle left it so.
            ORDERING => {
                let right = options.clone();
                options.shuffle(&mut rand::rng());
                if options == right && options.len() > 1 {
                    options.rotate_left(1);
                }
            }
            MATCHING => {
                choices = rows(&q, "options")
                    .iter()
                    .filter_map(|o| Some(json!({"key": grading::match_key(attempt, id(o, "id")?), "text": s(o, "match_text")?})))
                    .collect();
                choices.shuffle(&mut rand::rng());
                if b(&quiz, "shuffle_options") {
                    options.shuffle(&mut rand::rng());
                }
            }
            TRUE_FALSE => {}
            _ if b(&quiz, "shuffle_options") => options.shuffle(&mut rand::rng()),
            _ => {}
        }
        let points = if points > Decimal::ZERO { points } else { d(&q, "points").max(Decimal::ONE) };
        max_score += points;
        let blanks = if qtype == BLANKS { json!(grading::blanks_in(s(&q, "question").unwrap_or_default())) } else { Value::Null };
        answers.push(json!({
            "question": qid.to_string(), "question_text": q.get("question"), "question_type": qtype, "options": options,
            "choices": if choices.is_empty() { Value::Null } else { Value::Array(choices) }, "blanks": blanks,
            "by_person": grading::by_person(&qtype),
            "points": money(points), "response": null, "is_correct": false, "points_awarded": null, "correct_answer": null, "explanation": null,
        }));
    }
    let limit = int(&quiz, "time_limit_minutes").max(0);
    let attempt_no = past.iter().map(|a| a.2).max().unwrap_or(0) + 1;
    let mut doc = Map::new();
    for (k_, v) in [
        ("id", json!(attempt.to_string())),
        ("quiz", json!(quiz_id.to_string())),
        ("quiz_title", quiz.get("title").cloned().unwrap_or(Value::Null)),
        ("course", json!(course.to_string())),
        ("enrollment", enrollment.get("id").cloned().unwrap_or(Value::Null)),
        ("learner", json!(learner)),
        ("learner_name", enrollment.get("learner_name").cloned().unwrap_or(Value::Null)),
        ("attempt_no", json!(attempt_no)),
        ("status", json!(IN_PROGRESS)),
        ("started_at", json!(util::ts(now))),
        ("deadline", if limit > 0 { json!(util::ts(now + Duration::minutes(limit))) } else { Value::Null }),
        ("time_limit_minutes", json!(limit)),
        ("passing_percentage", quiz.get("passing_percentage").cloned().unwrap_or(json!("0"))),
        ("max_score", money(max_score)),
        ("passed", json!(false)),
        ("answers", Value::Array(answers)),
    ] {
        doc.insert(k_.into(), v);
    }
    let created = k.insert_tx(&elevated(ctx), tx, "LmsQuizAttempt", doc).await?;
    Ok(json!({"resumed": false, "attempt": view(&created, now)}))
}

/// Instructors' view of an attempt: every question with the learner's answer, the correct answer
/// and how it was graded (whatever the quiz reveals to learners).
async fn review(k: &Kernel, ctx: &Ctx, tx: &mut Tx, attempt: &Doc) -> Result<Value> {
    let me = doc_id(attempt)?;
    let mut out = Vec::new();
    for row in rows(attempt, "answers") {
        let Some(qid) = id(&row, "question") else { continue };
        let question = load(k, ctx, tx, "LmsQuestion", qid).await?;
        let (key, texts) = key_of(me, &question);
        let response = row.get("response").cloned().unwrap_or(Value::Null);
        let text = |k: &str| texts.iter().find(|(key, _)| key == k).map(|(_, t)| json!(t)).unwrap_or(Value::Null);
        let shown = match (&response, key.qtype.as_str()) {
            (Value::Array(given), BLANKS) => Value::Array(given.clone()),
            (Value::Array(keys), _) => Value::Array(keys.iter().filter_map(Value::as_str).map(text).collect()),
            (Value::Object(m), _) => Value::Array(m.iter().map(|(l, r)| json!({"item": text(l), "matches": r.as_str().map(text)})).collect()),
            (Value::Null, _) => json!([]),
            (other, _) => json!([other]),
        };
        let rubric = match id(&question, "rubric") {
            Some(r) => {
                let r = load(k, ctx, tx, "LmsRubric", r).await?;
                json!({"title": r.get("title"), "criteria": r.get("criteria"), "levels": r.get("levels"), "max_points": r.get("max_points")})
            }
            None => Value::Null,
        };
        let mut item = json!({
            "question": qid, "question_text": row.get("question_text"), "question_type": row.get("question_type"), "points": row.get("points"),
            "response": shown, "correct_answer": correct_answer(&key, &row), "is_correct": row.get("is_correct"),
            "points_awarded": row.get("points_awarded"), "explanation": question.get("explanation"), "rubric": rubric,
        });
        for f in ["by_person", "comment", "rubric_result", "graded_by"] {
            item[f] = row.get(f).cloned().unwrap_or(Value::Null);
        }
        out.push(item);
    }
    Ok(json!({
        "attempt": me, "number": attempt.get("number"), "learner": attempt.get("learner"), "learner_name": attempt.get("learner_name"),
        "status": attempt.get("status"), "score": attempt.get("score"), "max_score": attempt.get("max_score"),
        "percentage": attempt.get("percentage"), "passed": attempt.get("passed"), "answers": out,
    }))
}

/// A learner's standing at a quiz, for the lesson page: rules, attempts used and left, best score,
/// and the attempt in progress (if any).
pub async fn state(k: &Kernel, ctx: &Ctx, tx: &mut Tx, quiz_id: Uuid, enrollment: Uuid) -> Result<Value> {
    let quiz = load(k, ctx, tx, "LmsQuiz", quiz_id).await?;
    let attempts: Vec<(Uuid, String, Option<Decimal>, bool, Option<DateTime<Utc>>)> = sqlx::query_as(
        "select id, status, percentage, passed, deadline from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and enrollment = $3 order by attempt_no",
    )
    .bind(ctx.tenant)
    .bind(quiz_id)
    .bind(enrollment)
    .fetch_all(&mut **tx)
    .await?;
    let max = int(&quiz, "max_attempts");
    let per = int(&quiz, "questions_per_attempt");
    let open = attempts.iter().find(|a| a.1 == IN_PROGRESS);
    Ok(json!({
        "id": quiz_id, "title": quiz.get("title"), "description": quiz.get("description"),
        "passing_percentage": quiz.get("passing_percentage"), "time_limit_minutes": quiz.get("time_limit_minutes"),
        "questions": if per > 0 { per } else { int(&quiz, "question_count") },
        "max_attempts": max, "attempts_used": attempts.len(),
        "attempts_left": if max > 0 { Some((max - attempts.len() as i64).max(0)) } else { None },
        "best_percentage": attempts.iter().filter_map(|a| a.2).max().map(money),
        "passed": attempts.iter().any(|a| a.3),
        "in_progress": open.map(|a| json!({"attempt": a.0, "deadline": a.4.map(util::ts)})),
    }))
}

/// Maintenance: attempts whose time ran out are closed as Timed Out with the answers saved in time.
pub async fn expire_attempts(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let due: Vec<Uuid> = sqlx::query_scalar(
        "select id from e_lms_quiz_attempt where tenant_id = $1 and status = 'In Progress' and deadline < now() - make_interval(secs => $2) \
         order by deadline limit 500",
    )
    .bind(ctx.tenant)
    .bind(GRACE_SECONDS as f64)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut closed = 0;
    for attempt in due {
        let done = ferrum_kernel::with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let doc = load_locked(k, ctx, &mut tx, "LmsQuizAttempt", attempt).await?;
            let now = util::now();
            let close = s(&doc, "status") == Some(IN_PROGRESS) && late(&doc, now);
            if close {
                finalize(k, ctx, &mut tx, &doc, TIMED_OUT, now).await?;
            }
            tx.commit().await?;
            Ok(close)
        })
        .await?;
        closed += usize::from(done);
    }
    Ok(closed)
}
