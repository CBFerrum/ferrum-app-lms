//! xAPI (Tin Can) packages as lessons, and the xAPI endpoint they talk to.
//!
//! An xAPI lesson's package (a zip with `tincan.xml`) names its activity and the file it starts
//! from. Learn's player opens that file with the launch parameters such packages read (`endpoint`,
//! `auth`, `actor`, `registration`, `activity_id`). The endpoint is Learn's own, below the lesson
//! (`/api/app/lms/xapi/{lesson}/`), and the learner's session is its credential: a package speaks
//! only as the signed-in learner, about the lesson of their enrolment. It keeps what packages need
//! of an LRS: statements (stored, voided, read back) and state documents (bookmarks and resume
//! data). A statement that the lesson's activity was completed or passed completes the lesson.
//!
//! Not an LRS for anything else: no activity or agent profiles, no attachments, and statement
//! queries return the learner's own statements about the lesson, newest first, without paging.

use crate::enrollment::{self, WITHDRAWN};
use crate::scorm;
use crate::util::{self, authors, doc_id, elevated, id, load, s, sys};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, SiteRequest, SiteResponse, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The xAPI version this endpoint speaks.
pub const VERSION: &str = "1.0.3";
/// The largest statement batch or state document it takes.
pub const MAX_BODY: usize = 1 << 20;
/// The most statements one query returns.
const MAX_QUERY: i64 = 100;

const VOIDED: &str = "http://adlnet.gov/expapi/verbs/voided";
const COMPLETED: &str = "http://adlnet.gov/expapi/verbs/completed";
const PASSED: &str = "http://adlnet.gov/expapi/verbs/passed";
const FAILED: &str = "http://adlnet.gov/expapi/verbs/failed";

/// Reads an xAPI package: its first activity with a launch file, as (launch path, activity id).
pub fn inspect(zip: &[u8]) -> std::result::Result<(String, String), String> {
    let (names, base, xml) = scorm::descriptor(zip, "tincan.xml", "an xAPI (Tin Can) package")?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("has a tincan.xml that is not well-formed XML ({e})"))?;
    let named = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name);
    let (activity, launch) = doc
        .root_element()
        .descendants()
        .filter(|n| named(n, "activity"))
        .find_map(|a| {
            let launch = a.children().find(|c| named(c, "launch")).and_then(|l| l.text()).map(str::trim).filter(|l| !l.is_empty())?;
            Some((a.attribute("id")?.trim().to_string(), launch.to_string()))
        })
        .ok_or_else(|| "has no activity to launch in its tincan.xml".to_string())?;
    if !is_iri(&activity) {
        return Err(format!("names an activity that is not an IRI ({activity})"));
    }
    Ok((scorm::launchable(&names, &base, &launch)?, activity))
}

fn is_iri(v: &str) -> bool {
    v.split_once(':')
        .is_some_and(|(scheme, rest)| !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) && !rest.is_empty())
        && !v.chars().any(char::is_whitespace)
}

/// The endpoint's address for a lesson, as the player gives it to the package.
pub fn endpoint(lesson: Uuid) -> String {
    format!("/api/app/{}/xapi/{lesson}/", util::PACKAGE)
}

/// What the player needs to launch an xAPI lesson: the launch file, the endpoint, and who and which
/// registration the package speaks for (an instructor's preview speaks as them, and keeps nothing).
pub async fn player(ctx: &Ctx, _tx: &mut Tx, lesson: &Doc, enrollment: Option<&Doc>) -> Result<Value> {
    let lid = doc_id(lesson)?;
    let (Some(launch), Some(activity), Some(file)) =
        (s(lesson, "package_launch"), s(lesson, "package_activity"), s(lesson, "package_file").and_then(util::file_id))
    else {
        return Ok(Value::Null);
    };
    let (email, name) = match enrollment {
        Some(e) => (s(e, "learner").unwrap_or_default().to_string(), s(e, "learner_name").or(s(e, "learner")).unwrap_or_default().to_string()),
        None => (ctx.email.to_lowercase(), ctx.email.to_lowercase()),
    };
    Ok(json!({
        "launch_url": scorm::file_url(lid, file, launch), "endpoint": endpoint(lid), "activity_id": activity,
        "actor": {"objectType": "Agent", "name": name, "mbox": format!("mailto:{email}")},
        "registration": enrollment.map(|e| e.get("id").cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null),
        "preview": enrollment.is_none(),
    }))
}

/// Who the endpoint is talking to, about which lesson.
struct Caller<'a> {
    ctx: &'a Ctx,
    lesson: Doc,
    course: Doc,
    /// The learner's enrolment; none for an instructor's preview, whose writes are not kept.
    enrollment: Option<Doc>,
    email: String,
}

fn respond(status: u16, body: Option<Value>) -> SiteResponse {
    let mut r = match body {
        Some(v) => SiteResponse::json(status, &v),
        None => SiteResponse { status, content_type: "application/json".into(), headers: vec![], body: vec![] },
    };
    r.headers.push(("x-experience-api-version".into(), VERSION.into()));
    r.headers.push(("cache-control".into(), "no-store".into()));
    r
}

fn param<'a>(req: &'a SiteRequest, key: &str) -> Option<&'a str> {
    req.query.get(key).and_then(Value::as_str)
}

fn header<'a>(req: &'a SiteRequest, key: &str) -> Option<&'a str> {
    req.headers.get(key).and_then(Value::as_str)
}

/// Answers a request to a lesson's xAPI endpoint (`path` after `xapi/{lesson}/`).
pub async fn handle(k: &Kernel, user: &Ctx, req: &SiteRequest, lesson: Uuid, path: &[&str]) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(user.tenant).await?;
    let l = match load(k, user, &mut tx, "LmsLesson", lesson).await {
        Ok(l) if s(&l, "lesson_type") == Some("xAPI") => l,
        Ok(_) | Err(Error::NotFound(_)) => return Err(Error::NotFound("xAPI lesson".into())),
        Err(e) => return Err(e),
    };
    let course = load(k, user, &mut tx, "LmsCourse", id(&l, "course").unwrap_or_default()).await?;
    let mine: Option<Uuid> =
        sqlx::query_scalar("select id from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 and status <> 'Withdrawn' order by coalesce(cycle, 1) desc limit 1")
            .bind(user.tenant)
            .bind(doc_id(&course)?)
            .bind(user.email.to_lowercase())
            .fetch_optional(&mut *tx)
            .await?;
    let enrollment = match mine {
        Some(e) => Some(load(k, user, &mut tx, "LmsEnrollment", e).await?),
        None if authors(user, &course) => None,
        None => return Err(Error::NotFound("xAPI lesson".into())),
    };
    let caller = Caller { ctx: user, lesson: l, course, enrollment, email: user.email.to_lowercase() };
    let out = match (req.method.as_str(), path) {
        ("GET", ["about"]) => respond(200, Some(json!({"version": [VERSION]}))),
        (_, ["statements"]) => statements(k, &caller, &mut tx, req).await?,
        (_, ["activities", "state"]) => state(&caller, &mut tx, req).await?,
        _ => return Err(Error::NotFound("this xAPI resource (Learn keeps statements and state)".into())),
    };
    tx.commit().await?;
    Ok(out)
}

/// The agent a statement or a state request names is the signed-in learner.
fn is_me(agent: &Value, email: &str) -> bool {
    if agent.get("objectType").and_then(Value::as_str).is_some_and(|t| t != "Agent") {
        return false;
    }
    let mbox = agent.get("mbox").and_then(Value::as_str).and_then(|m| m.strip_prefix("mailto:"));
    let account = agent.pointer("/account/name").and_then(Value::as_str);
    mbox.or(account).is_some_and(|m| util::same_email(m, email))
}

/// Checks a statement and completes what the endpoint adds (`id`, `stored`, `timestamp`,
/// `authority`, `version`).
fn prepare(mut st: Value, id: Option<Uuid>, email: &str) -> Result<(Uuid, Value)> {
    let bad = |m: &str| Error::field("statement", m.to_string());
    let obj = st.as_object_mut().ok_or_else(|| bad("must be an object"))?;
    let actor = obj.get("actor").ok_or_else(|| bad("needs an actor"))?;
    if !is_me(actor, email) {
        return Err(Error::Forbidden("statements here are about the signed-in learner".into()));
    }
    let verb = obj.get("verb").and_then(|v| v.get("id")).and_then(Value::as_str).ok_or_else(|| bad("needs a verb with an id"))?;
    if !is_iri(verb) {
        return Err(bad("has a verb id that is not an IRI"));
    }
    let object = obj.get("object").ok_or_else(|| bad("needs an object"))?;
    match object.get("objectType").and_then(Value::as_str).unwrap_or("Activity") {
        "Activity" => {
            if !object.get("id").and_then(Value::as_str).is_some_and(is_iri) {
                return Err(bad("has an activity without an IRI id"));
            }
        }
        "StatementRef" => {
            object.get("id").and_then(Value::as_str).and_then(|v| v.parse::<Uuid>().ok()).ok_or_else(|| bad("refers to a statement without a UUID"))?;
        }
        "Agent" | "Group" | "SubStatement" => {}
        _ => return Err(bad("has an object of no known type")),
    }
    if let Some(scaled) = obj.get("result").and_then(|r| r.pointer("/score/scaled"))
        && !scaled.as_f64().is_some_and(|v| (-1.0..=1.0).contains(&v))
    {
        return Err(bad("has a scaled score outside -1 to 1"));
    }
    let given = match obj.get("id") {
        Some(v) => Some(v.as_str().and_then(|v| v.parse::<Uuid>().ok()).ok_or_else(|| bad("has an id that is not a UUID"))?),
        None => None,
    };
    let sid = match (id, given) {
        (Some(a), Some(b)) if a != b => return Err(bad("has an id other than the statementId it is put at")),
        (a, b) => a.or(b).unwrap_or_else(Uuid::now_v7),
    };
    let now = util::ts(util::now());
    obj.insert("id".into(), json!(sid.to_string()));
    obj.entry("timestamp").or_insert_with(|| json!(now.clone()));
    obj.entry("version").or_insert_with(|| json!("1.0.0"));
    obj.insert("stored".into(), json!(now));
    obj.insert("authority".into(), json!({"objectType": "Agent", "name": "Learn", "account": {"homePage": "urn:ferrum:lms", "name": "lms"}}));
    Ok((sid, st))
}

/// A statement as it compares with another put at the same id (what the endpoint adds aside).
fn essence(st: &Value) -> Value {
    let mut v = st.clone();
    if let Some(o) = v.as_object_mut() {
        for k in ["stored", "authority", "version", "timestamp"] {
            o.remove(k);
        }
    }
    v
}

async fn statements(k: &Kernel, c: &Caller<'_>, tx: &mut Tx, req: &SiteRequest) -> Result<SiteResponse> {
    match req.method.as_str() {
        "GET" => {
            let Some(e) = &c.enrollment else { return Ok(respond(200, Some(json!({"statements": [], "more": ""})))) };
            let eid = doc_id(e)?;
            if let Some(sid) = param(req, "statementId").or(param(req, "voidedStatementId")) {
                let sid: Uuid = sid.parse().map_err(|_| Error::field("statementId", "must be a UUID"))?;
                let voided = param(req, "voidedStatementId").is_some();
                let found: Option<Value> = sqlx::query_scalar(
                    "select statement from lms_xapi_statement where tenant_id = $1 and enrollment = $2 and lesson = $3 and id = $4 and voided = $5",
                )
                .bind(c.ctx.tenant)
                .bind(eid)
                .bind(doc_id(&c.lesson)?)
                .bind(sid)
                .bind(voided)
                .fetch_optional(&mut **tx)
                .await?;
                return found.map(|st| respond(200, Some(st))).ok_or_else(|| Error::NotFound("statement".into()));
            }
            let limit = match param(req, "limit").map(str::parse::<i64>) {
                None | Some(Ok(0)) => MAX_QUERY,
                Some(Ok(n)) if n > 0 => n.min(MAX_QUERY),
                _ => return Err(Error::field("limit", "must be a whole number")),
            };
            let rows: Vec<Value> = sqlx::query_scalar(
                "select statement from lms_xapi_statement where tenant_id = $1 and enrollment = $2 and lesson = $3 and not voided \
                 and ($4::text is null or verb = $4) and ($5::text is null or object_id = $5) and ($6::uuid is null or registration = $6) \
                 and ($7::timestamptz is null or stored > $7) order by stored desc, id desc limit $8",
            )
            .bind(c.ctx.tenant)
            .bind(eid)
            .bind(doc_id(&c.lesson)?)
            .bind(param(req, "verb"))
            .bind(param(req, "activity"))
            .bind(param(req, "registration").map(|r| r.parse::<Uuid>()).transpose().map_err(|_| Error::field("registration", "must be a UUID"))?)
            .bind(param(req, "since").map(ferrum_kernel::value::parse_datetime).transpose().map_err(|_| Error::field("since", "must be a time"))?)
            .bind(limit)
            .fetch_all(&mut **tx)
            .await?;
            Ok(respond(200, Some(json!({"statements": rows, "more": ""}))))
        }
        "PUT" | "POST" => {
            let body: Value = serde_json::from_slice(&req.body).map_err(|_| Error::field("statement", "is not JSON"))?;
            let at = match (req.method.as_str(), param(req, "statementId")) {
                ("PUT", Some(sid)) => Some(sid.parse::<Uuid>().map_err(|_| Error::field("statementId", "must be a UUID"))?),
                ("PUT", None) => return Err(Error::field("statementId", "is required to put a statement")),
                _ => None,
            };
            let batch = match (at, body) {
                (Some(_), Value::Array(_)) => return Err(Error::field("statement", "a put takes one statement")),
                (_, Value::Array(list)) => list,
                (_, one) => vec![one],
            };
            if batch.is_empty() {
                return Err(Error::field("statement", "none sent"));
            }
            let mut ids = Vec::with_capacity(batch.len());
            for st in batch {
                ids.push(store(k, c, tx, st, at).await?);
            }
            Ok(if at.is_some() { respond(204, None) } else { respond(200, Some(json!(ids))) })
        }
        _ => Err(Error::NotFound("this xAPI resource".into())),
    }
}

/// Stores one statement (a repeat of a stored one changes nothing; another at its id is a
/// conflict), voids what it voids, and records what it says about the lesson.
async fn store(k: &Kernel, c: &Caller<'_>, tx: &mut Tx, st: Value, at: Option<Uuid>) -> Result<String> {
    let (sid, st) = prepare(st, at, &c.email)?;
    let Some(e) = &c.enrollment else { return Ok(sid.to_string()) };
    let (eid, lid) = (doc_id(e)?, doc_id(&c.lesson)?);
    let verb = st.pointer("/verb/id").and_then(Value::as_str).unwrap_or_default().to_string();
    let object = st.pointer("/object/id").and_then(Value::as_str).map(str::to_string);
    let registration = st.pointer("/context/registration").and_then(Value::as_str).and_then(|r| r.parse::<Uuid>().ok());
    let existing: Option<Value> = sqlx::query_scalar("select statement from lms_xapi_statement where tenant_id = $1 and id = $2")
        .bind(c.ctx.tenant)
        .bind(sid)
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(prior) = existing {
        return if essence(&prior) == essence(&st) {
            Ok(sid.to_string())
        } else {
            Err(Error::Conflict(format!("statement {sid} is stored already, differently")))
        };
    }
    if verb == VOIDED {
        // A voiding statement voids the learner's statement about this lesson it names, never another
        // voiding one; it is kept even when that statement is not here (xAPI 1.0.3, 2.4.8).
        let target: Uuid = object.as_deref().and_then(|o| o.parse().ok()).ok_or_else(|| Error::field("statement", "voids no statement"))?;
        sqlx::query("update lms_xapi_statement set voided = true where tenant_id = $1 and enrollment = $2 and lesson = $3 and id = $4 and verb <> $5")
            .bind(c.ctx.tenant)
            .bind(eid)
            .bind(lid)
            .bind(target)
            .bind(VOIDED)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "insert into lms_xapi_statement (tenant_id, id, lesson, enrollment, learner, verb, object_id, registration, statement) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(c.ctx.tenant)
    .bind(sid)
    .bind(lid)
    .bind(eid)
    .bind(&c.email)
    .bind(&verb)
    .bind(&object)
    .bind(registration)
    .bind(&st)
    .execute(&mut **tx)
    .await?;
    record(k, c, tx, e, &st).await?;
    Ok(sid.to_string())
}

/// What a statement says about the lesson's activity.
#[derive(Debug, PartialEq)]
pub struct Said {
    pub status: Option<&'static str>,
    pub score: Option<Decimal>,
    pub seconds: Option<i64>,
}

/// A statement about `activity`: passed, completed (unless its result failed) or failed, its
/// score (`result.score.scaled`, else raw against min and max) and duration. Statements about
/// anything else (a question, a slide) say nothing of the lesson.
pub fn said(st: &Value, activity: &str) -> Said {
    let none = Said { status: None, score: None, seconds: None };
    if st.pointer("/object/id").and_then(Value::as_str) != Some(activity) {
        return none;
    }
    let verb = st.pointer("/verb/id").and_then(Value::as_str).unwrap_or_default();
    let success = st.pointer("/result/success").and_then(Value::as_bool);
    let completion = st.pointer("/result/completion").and_then(Value::as_bool);
    let status = match verb {
        PASSED => Some("Passed"),
        FAILED => Some("Failed"),
        COMPLETED if success == Some(false) => Some("Failed"),
        COMPLETED => Some("Completed"),
        _ if completion == Some(true) && success != Some(false) => Some("Completed"),
        _ => Some("Incomplete"),
    };
    let num = |p: &str| st.pointer(p).and_then(|v| v.as_f64()).and_then(|v| Decimal::try_from(v).ok());
    let score = match num("/result/score/scaled") {
        Some(s) => Some((s * Decimal::ONE_HUNDRED).clamp(Decimal::ZERO, Decimal::ONE_HUNDRED).round_dp(2)),
        None => match (num("/result/score/raw"), num("/result/score/min").unwrap_or(Decimal::ZERO), num("/result/score/max")) {
            (Some(raw), min, Some(max)) if max > min => {
                Some(((raw - min) * Decimal::ONE_HUNDRED / (max - min)).clamp(Decimal::ZERO, Decimal::ONE_HUNDRED).round_dp(2))
            }
            _ => None,
        },
    };
    let seconds = st.pointer("/result/duration").and_then(Value::as_str).and_then(scorm::seconds);
    Said { status, score, seconds }
}

/// Keeps the learner's Package Progress, and completes the lesson when a statement says it is
/// completed or passed. Once done it stays done: a later failed retake keeps the completion.
async fn record(k: &Kernel, c: &Caller<'_>, tx: &mut Tx, e: &Doc, st: &Value) -> Result<()> {
    let activity = s(&c.lesson, "package_activity").unwrap_or_default().to_string();
    let what = said(st, &activity);
    let (eid, lid) = (doc_id(e)?, doc_id(&c.lesson)?);
    let row: Option<(Uuid, String, i64)> = sqlx::query_as(
        "select id, status, coalesce(commits, 0) from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3 for update",
    )
    .bind(c.ctx.tenant)
    .bind(eid)
    .bind(lid)
    .fetch_optional(&mut **tx)
    .await?;
    let finished = |st: &str| matches!(st, "Passed" | "Completed");
    let mut v = Doc::new();
    v.insert("last_commit_at".into(), json!(util::ts(util::now())));
    if let Some(status) = what.status
        && !row.as_ref().is_some_and(|(_, prior, _)| finished(prior) && !finished(status))
    {
        v.insert("status".into(), json!(status));
    }
    if let Some(p) = what.score {
        v.insert("score_percent".into(), json!(p.to_string()));
    }
    if let Some(t) = what.seconds {
        v.insert("total_seconds".into(), json!(t));
    }
    match row {
        Some((pid, _, commits)) => {
            v.insert("commits".into(), json!(commits + 1));
            k.set_values_tx(&sys(c.ctx), tx, "LmsPackageProgress", pid, v).await?;
        }
        None => {
            v.entry("status").or_insert(json!("Incomplete"));
            for (f, val) in [
                ("enrollment", json!(eid.to_string())),
                ("lesson", json!(lid.to_string())),
                ("course", c.course.get("id").cloned().unwrap_or(Value::Null)),
                ("learner", e.get("learner").cloned().unwrap_or(Value::Null)),
                ("commits", json!(1)),
            ] {
                v.insert(f.into(), val);
            }
            k.insert_tx(&elevated(c.ctx), tx, "LmsPackageProgress", v).await?;
        }
    }
    if what.status.is_some_and(finished) && s(e, "status") != Some(WITHDRAWN) {
        enrollment::record_completion(k, c.ctx, tx, eid, lid, "xAPI").await?;
    }
    Ok(())
}

/// The state documents of the lesson's activity for the learner (`activityId`, `agent`,
/// `registration`, `stateId`): get (one, or the ids), put (replace), post (merge JSON objects),
/// delete (one, or all).
async fn state(c: &Caller<'_>, tx: &mut Tx, req: &SiteRequest) -> Result<SiteResponse> {
    let activity = param(req, "activityId").filter(|a| is_iri(a)).ok_or_else(|| Error::field("activityId", "an IRI is required"))?;
    let agent: Value = param(req, "agent").and_then(|a| serde_json::from_str(a).ok()).ok_or_else(|| Error::field("agent", "the agent (JSON) is required"))?;
    if !is_me(&agent, &c.email) {
        return Err(Error::Forbidden("state here is the signed-in learner's".into()));
    }
    let registration = param(req, "registration").unwrap_or_default();
    if !registration.is_empty() && registration.parse::<Uuid>().is_err() {
        return Err(Error::field("registration", "must be a UUID"));
    }
    let state_id = param(req, "stateId");
    let Some(e) = &c.enrollment else {
        // An instructor's preview: nothing is kept, so nothing is there.
        return match (req.method.as_str(), state_id) {
            ("GET", None) => Ok(respond(200, Some(json!([])))),
            ("GET", Some(_)) => Err(Error::NotFound("state document".into())),
            _ => Ok(respond(204, None)),
        };
    };
    let (eid, lid, tenant) = (doc_id(e)?, doc_id(&c.lesson)?, c.ctx.tenant);
    let current: Option<(String, Vec<u8>)> = match state_id {
        Some(sid) => {
            sqlx::query_as(
                "select content_type, body from lms_xapi_state where tenant_id = $1 and enrollment = $2 and lesson = $3 and activity_id = $4 \
                 and registration = $5 and state_id = $6",
            )
            .bind(tenant)
            .bind(eid)
            .bind(lid)
            .bind(activity)
            .bind(registration)
            .bind(sid)
            .fetch_optional(&mut **tx)
            .await?
        }
        None => None,
    };
    let etag = |body: &[u8]| format!("\"{}\"", hex::encode(Sha256::digest(body)));
    // A put or post that names the version it changes, or that it creates.
    let precondition = |current: &Option<(String, Vec<u8>)>| -> Result<()> {
        if let Some(want) = header(req, "if-match")
            && current.as_ref().is_none_or(|(_, b)| want.trim() != "*" && want.trim() != etag(b))
        {
            return Err(Error::Conflict("the state document changed (If-Match)".into()));
        }
        if header(req, "if-none-match").is_some_and(|v| v.trim() == "*") && current.is_some() {
            return Err(Error::Conflict("the state document exists (If-None-Match)".into()));
        }
        Ok(())
    };
    match (req.method.as_str(), state_id) {
        ("GET", Some(_)) => {
            let (ctype, body) = current.ok_or_else(|| Error::NotFound("state document".into()))?;
            let tag = etag(&body);
            let mut r = respond(200, None);
            r.content_type = ctype;
            r.body = body;
            r.headers.push(("etag".into(), tag));
            Ok(r)
        }
        ("GET", None) => {
            let ids: Vec<String> = sqlx::query_scalar(
                "select state_id from lms_xapi_state where tenant_id = $1 and enrollment = $2 and lesson = $3 and activity_id = $4 and registration = $5 \
                 and ($6::timestamptz is null or updated_at > $6) order by state_id",
            )
            .bind(tenant)
            .bind(eid)
            .bind(lid)
            .bind(activity)
            .bind(registration)
            .bind(param(req, "since").map(ferrum_kernel::value::parse_datetime).transpose().map_err(|_| Error::field("since", "must be a time"))?)
            .fetch_all(&mut **tx)
            .await?;
            Ok(respond(200, Some(json!(ids))))
        }
        ("PUT" | "POST", Some(sid)) => {
            precondition(&current)?;
            let ctype = header(req, "content-type").unwrap_or("application/octet-stream").to_string();
            let json_body = ctype.starts_with("application/json");
            let body = match (req.method.as_str(), &current) {
                ("POST", Some((prior_type, prior))) => {
                    // Posting merges JSON objects into what is there; anything else cannot merge.
                    let (Ok(Value::Object(mut base)), Ok(Value::Object(more))) =
                        (serde_json::from_slice::<Value>(prior), serde_json::from_slice::<Value>(&req.body))
                    else {
                        return Err(Error::field("stateId", "posting merges JSON objects only; put to replace the document"));
                    };
                    if !prior_type.starts_with("application/json") || !json_body {
                        return Err(Error::field("stateId", "posting merges JSON objects only; put to replace the document"));
                    }
                    base.extend(more);
                    serde_json::to_vec(&base).map_err(|_| Error::internal("state document"))?
                }
                ("POST", None) if json_body && !serde_json::from_slice::<Value>(&req.body).is_ok_and(|v| v.is_object()) => {
                    return Err(Error::field("stateId", "a posted JSON document is an object"));
                }
                _ => req.body.clone(),
            };
            sqlx::query(
                "insert into lms_xapi_state (tenant_id, enrollment, lesson, activity_id, registration, state_id, content_type, body) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8) on conflict (tenant_id, enrollment, lesson, activity_id, registration, state_id) \
                 do update set content_type = excluded.content_type, body = excluded.body, updated_at = now()",
            )
            .bind(tenant)
            .bind(eid)
            .bind(lid)
            .bind(activity)
            .bind(registration)
            .bind(sid)
            .bind(&ctype)
            .bind(&body)
            .execute(&mut **tx)
            .await?;
            Ok(respond(204, None))
        }
        ("DELETE", sid) => {
            sqlx::query(
                "delete from lms_xapi_state where tenant_id = $1 and enrollment = $2 and lesson = $3 and activity_id = $4 and registration = $5 \
                 and ($6::text is null or state_id = $6)",
            )
            .bind(tenant)
            .bind(eid)
            .bind(lid)
            .bind(activity)
            .bind(registration)
            .bind(sid)
            .execute(&mut **tx)
            .await?;
            Ok(respond(204, None))
        }
        _ => Err(Error::field("stateId", "is required to put or post a state document")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(body.as_bytes()).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    const TINCAN: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<tincan xmlns="http://projecttincan.com/tincan.xsd"><activities>
  <activity id="https://acme.test/rust/q1" type="http://adlnet.gov/expapi/activities/cmi.interaction"><name>Q1</name></activity>
  <activity id="https://acme.test/rust" type="http://adlnet.gov/expapi/activities/course"><name>Rust</name><launch lang="und">story.html</launch></activity>
</activities></tincan>"#;

    #[test]
    fn the_first_activity_with_a_launch_file() {
        let got = inspect(&zip(&[("tincan.xml", TINCAN), ("story.html", "")])).unwrap();
        assert_eq!(got, ("story.html".to_string(), "https://acme.test/rust".to_string()));
        assert!(inspect(&zip(&[("tincan.xml", TINCAN)])).unwrap_err().contains("does not hold"));
        assert!(inspect(&zip(&[("imsmanifest.xml", "<manifest/>")])).unwrap_err().contains("tincan.xml"));
        let not_iri = TINCAN.replace("https://acme.test/rust\"", "rust course\"");
        assert!(inspect(&zip(&[("tincan.xml", &not_iri), ("story.html", "")])).unwrap_err().contains("not an IRI"));
    }

    fn st(verb: &str, result: Value) -> Value {
        json!({"actor": {"mbox": "mailto:Asha@Learn.test"}, "verb": {"id": verb}, "object": {"id": "https://acme.test/rust"}, "result": result})
    }

    #[test]
    fn statements_name_the_learner_and_get_what_the_endpoint_adds() {
        let (sid, done) = prepare(st(PASSED, json!({})), None, "asha@learn.test").unwrap();
        assert_eq!((done["id"].clone(), done["version"].clone(), done["authority"]["name"].clone()), (json!(sid.to_string()), json!("1.0.0"), json!("Learn")));
        assert!(matches!(prepare(st(PASSED, json!({})), None, "ben@learn.test"), Err(Error::Forbidden(_))), "not about someone else");
        let mut as_group = st(PASSED, json!({}));
        as_group["actor"]["objectType"] = json!("Group");
        assert!(matches!(prepare(as_group, None, "asha@learn.test"), Err(Error::Forbidden(_))));
        let at = Uuid::now_v7();
        let mut other = st(PASSED, json!({}));
        other["id"] = json!(Uuid::now_v7().to_string());
        assert!(prepare(other, Some(at), "asha@learn.test").is_err(), "an id other than where it is put");
        assert!(prepare(st("passed", json!({})), None, "asha@learn.test").is_err(), "a verb that is not an IRI");
        assert!(prepare(st(PASSED, json!({"score": {"scaled": 1.5}})), None, "asha@learn.test").is_err());
        let account = json!({"actor": {"account": {"homePage": "https://w.test", "name": "asha@learn.test"}}, "verb": {"id": COMPLETED}, "object": {"id": "https://acme.test/rust"}});
        assert!(prepare(account, None, "asha@learn.test").is_ok(), "an account naming the learner");
    }

    #[test]
    fn what_statements_say_of_the_lesson() {
        let a = "https://acme.test/rust";
        assert_eq!(
            said(&st(PASSED, json!({"score": {"scaled": 0.92}, "duration": "PT12M30S"})), a),
            Said { status: Some("Passed"), score: Some(Decimal::from(92)), seconds: Some(750) }
        );
        assert_eq!(said(&st(COMPLETED, json!({"success": false, "score": {"raw": 3, "max": 10}})), a).status, Some("Failed"), "completed, but failed");
        assert_eq!(
            said(&st(COMPLETED, json!({"score": {"raw": 7, "max": 10}})), a),
            Said { status: Some("Completed"), score: Some(Decimal::from(70)), seconds: None }
        );
        assert_eq!(said(&st("http://adlnet.gov/expapi/verbs/experienced", json!({"completion": true})), a).status, Some("Completed"));
        assert_eq!(said(&st("http://adlnet.gov/expapi/verbs/attempted", json!({})), a).status, Some("Incomplete"));
        let mut question = st(PASSED, json!({}));
        question["object"]["id"] = json!("https://acme.test/rust/q1");
        assert_eq!(said(&question, a), Said { status: None, score: None, seconds: None }, "a question says nothing of the lesson");
    }
}
