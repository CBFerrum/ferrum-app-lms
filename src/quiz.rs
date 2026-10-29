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
use crate::grading::{self, Key, MULTIPLE, SHORT, SINGLE, TRUE_FALSE};
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
        doc.insert("question_count".into(), json!(list.len()));
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
                let changed = match params.get("answers") {
                    None | Some(Value::Null) => false,
                    Some(a) => apply(&mut answers, a)?,
                };
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

/// Applies `{question id: response}` to the attempt's rows; returns whether anything changed.
fn apply(rows: &mut [Doc], answers: &Value) -> Result<bool> {
    let Value::Object(map) = answers else {
        return Err(Error::field("answers", "send an object of question id → answer"));
    };
    let mut changed = false;
    for (question, response) in map {
        let Some(row) = rows.iter_mut().find(|r| s(r, "question") == Some(question.as_str())) else {
            return Err(Error::field("answers", format!("question {} is not part of this attempt", util::cut(question, 40))));
        };
        let qtype = s(row, "question_type").unwrap_or(SINGLE).to_string();
        let shown: Vec<String> = row
            .get("options")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|o| o.get("key")?.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let value = grading::parse_response(&qtype, &shown, response).map_err(|m| Error::field("answers", format!("question {question}: {m}")))?;
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
    let options: Vec<(String, String, bool)> = rows(question, "options")
        .iter()
        .filter_map(|o| Some((grading::option_key(attempt, id(o, "id")?), s(o, "option")?.to_string(), b(o, "is_correct"))))
        .collect();
    let correct: BTreeSet<String> = options.iter().filter(|o| o.2).map(|o| o.0.clone()).collect();
    let accepted: Vec<String> = rows(question, "accepted_answers").iter().filter_map(|a| s(a, "answer").map(str::to_string)).collect();
    let key = Key { qtype, correct, accepted, case_sensitive: b(question, "case_sensitive") };
    (key, options.into_iter().map(|(k, t, _)| (k, t)).collect())
}

/// What a revealed row shows as the correct answer: the correct options (as shown) or the
/// accepted short answers.
fn correct_answer(key: &Key, row: &Doc) -> Value {
    if key.qtype == SHORT {
        return json!(key.accepted);
    }
    let shown = row.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
    Value::Array(
        shown.iter().filter(|o| o.get("key").and_then(Value::as_str).is_some_and(|k| key.correct.contains(k))).filter_map(|o| o.get("text").cloned()).collect(),
    )
}

/// Grades the attempt against the current answer keys, closes it with `status`, reveals the
/// answers when the quiz says so and, when it passed, completes the quiz's lessons.
async fn finalize(k: &Kernel, ctx: &Ctx, tx: &mut Tx, attempt: &Doc, status: &str, now: DateTime<Utc>) -> Result<Doc> {
    let me = doc_id(attempt)?;
    let quiz_id = id(attempt, "quiz").ok_or_else(|| Error::internal("attempt without quiz"))?;
    let quiz = load(k, ctx, tx, "LmsQuiz", quiz_id).await?;
    let mut answers = rows(attempt, "answers");
    let mut graded = Vec::with_capacity(answers.len());
    let mut score = Decimal::ZERO;
    for row in answers.iter_mut() {
        let question = match id(row, "question") {
            Some(q) => Some(load(k, ctx, tx, "LmsQuestion", q).await?),
            None => None,
        };
        let (key, _) = question
            .as_ref()
            .map(|q| key_of(me, q))
            .unwrap_or_else(|| (Key { qtype: SINGLE.into(), correct: BTreeSet::new(), accepted: vec![], case_sensitive: false }, vec![]));
        let ok = question.is_some() && grading::is_correct(&key, row.get("response").unwrap_or(&Value::Null));
        let points = if ok { d(row, "points") } else { Decimal::ZERO };
        score += points;
        row.insert("is_correct".into(), json!(ok));
        row.insert("points_awarded".into(), money(points));
        graded.push((key, question));
    }
    let max = d(attempt, "max_score");
    let passed = grading::passed(score, max, d(attempt, "passing_percentage"));
    let learner = s(attempt, "learner").unwrap_or_default().to_string();
    let used: i64 = sqlx::query_scalar("select count(*) from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and learner = $3")
        .bind(ctx.tenant)
        .bind(quiz_id)
        .bind(&learner)
        .fetch_one(&mut **tx)
        .await?;
    let max_attempts = int(&quiz, "max_attempts");
    let reveal = match s(&quiz, "show_answers") {
        Some("After Each Attempt") => true,
        Some("Never") => false,
        _ => passed || (max_attempts > 0 && used >= max_attempts),
    };
    if reveal {
        for (row, (key, question)) in answers.iter_mut().zip(graded.iter()) {
            row.insert("correct_answer".into(), correct_answer(key, row));
            row.insert("explanation".into(), question.as_ref().and_then(|q| q.get("explanation").cloned()).unwrap_or(Value::Null));
        }
    }
    let mut patch = Map::new();
    for (k_, v) in [
        ("status", json!(status)),
        ("submitted_at", json!(util::ts(now))),
        ("score", money(score)),
        ("percentage", money(grading::percentage(score, max))),
        ("passed", json!(passed)),
        ("answers", Value::Array(answers.into_iter().map(Value::Object).collect())),
    ] {
        patch.insert(k_.into(), v);
    }
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
        sqlx::query_as("select id, status, attempt_no from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and learner = $3 order by attempt_no")
            .bind(ctx.tenant)
            .bind(quiz_id)
            .bind(&learner)
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
    if picked.is_empty() {
        return Err(Error::invalid("this quiz has no questions yet"));
    }
    let per = int(&quiz, "questions_per_attempt").max(0) as usize;
    if per > 0 && per < picked.len() {
        let mut order: Vec<usize> = (0..picked.len()).collect();
        order.shuffle(&mut rand::rng());
        order.truncate(per);
        order.sort_unstable();
        picked = order.into_iter().map(|i| picked[i].clone()).collect();
    }
    if b(&quiz, "shuffle_questions") {
        picked.shuffle(&mut rand::rng());
    }
    let attempt = Uuid::now_v7();
    let mut answers = Vec::with_capacity(picked.len());
    let mut max_score = Decimal::ZERO;
    for r in &picked {
        let qid = id(r, "question").ok_or_else(|| Error::internal("quiz row without question"))?;
        let q = load(k, ctx, tx, "LmsQuestion", qid).await?;
        let qtype = s(&q, "question_type").unwrap_or(SINGLE).to_string();
        let mut options: Vec<Value> =
            rows(&q, "options").iter().filter_map(|o| Some(json!({"key": grading::option_key(attempt, id(o, "id")?), "text": s(o, "option")?}))).collect();
        if b(&quiz, "shuffle_options") && qtype != TRUE_FALSE {
            options.shuffle(&mut rand::rng());
        }
        let points = d(r, "points");
        let points = if points > Decimal::ZERO { points } else { d(&q, "points").max(Decimal::ONE) };
        max_score += points;
        answers.push(json!({
            "question": qid.to_string(), "question_text": q.get("question"), "question_type": qtype, "options": options,
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
        let chosen: Vec<String> = match &response {
            Value::Array(keys) => keys.iter().filter_map(|k| texts.iter().find(|(key, _)| Some(key.as_str()) == k.as_str()).map(|(_, t)| t.clone())).collect(),
            Value::String(t) => vec![t.clone()],
            _ => vec![],
        };
        out.push(json!({
            "question": qid, "question_text": row.get("question_text"), "question_type": row.get("question_type"), "points": row.get("points"),
            "response": chosen, "correct_answer": correct_answer(&key, &row), "is_correct": grading::is_correct(&key, &response),
            "points_awarded": row.get("points_awarded"), "explanation": question.get("explanation"),
        }));
    }
    Ok(json!({
        "attempt": me, "number": attempt.get("number"), "learner": attempt.get("learner"), "learner_name": attempt.get("learner_name"),
        "status": attempt.get("status"), "score": attempt.get("score"), "max_score": attempt.get("max_score"),
        "percentage": attempt.get("percentage"), "passed": attempt.get("passed"), "answers": out,
    }))
}

/// A learner's standing at a quiz, for the lesson page: rules, attempts used and left, best score,
/// and the attempt in progress (if any).
pub async fn state(k: &Kernel, ctx: &Ctx, tx: &mut Tx, quiz_id: Uuid, learner: &str) -> Result<Value> {
    let quiz = load(k, ctx, tx, "LmsQuiz", quiz_id).await?;
    let attempts: Vec<(Uuid, String, Option<Decimal>, bool, Option<DateTime<Utc>>)> = sqlx::query_as(
        "select id, status, percentage, passed, deadline from e_lms_quiz_attempt where tenant_id = $1 and quiz = $2 and learner = $3 order by attempt_no",
    )
    .bind(ctx.tenant)
    .bind(quiz_id)
    .bind(learner.to_lowercase())
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
