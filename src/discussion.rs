//! Discussions (LM-80): a thread per course or per lesson, with replies, by the course's enrolled
//! learners (members and portal people alike) and its instructors. Instructors mark answers, pin,
//! close and remove; anyone in the course reports a post to the instructors. Followers of a thread
//! (its author and whoever replied, or chose to) hear of each reply through the `lms.discussion_reply`
//! message template; a report through `lms.post_reported`.
//!
//! The platform's comments are not shared with portal people, so posts are Learn's own records
//! (`LmsPost`), written by the course's `discuss` action after Learn checks the writer is in the
//! course, and read through Learn's page `courses/{slug}/discussion`. A removed post keeps its text,
//! who removed it and why (and the audit trail keeps every change); learners no longer read it.

use crate::util::{self, authored_course, authors, b, doc_id, elevated, id, int, load, load_locked, rows, s, same_email, sys};
use async_trait::async_trait;
use ferrum_kernel::messages::{Message, Recipient};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Map, Value, json};
use uuid::Uuid;

const MAX_BODY: usize = 20_000;

/// Whether the caller may take part in the course's discussion: its authors and learners with an
/// enrolment that is not withdrawn.
async fn in_course(ctx: &Ctx, tx: &mut Tx, course: &Doc) -> Result<bool> {
    if authors(ctx, course) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar("select exists(select 1 from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 and status <> 'Withdrawn')")
        .bind(ctx.tenant)
        .bind(doc_id(course)?)
        .bind(ctx.email.to_lowercase())
        .fetch_one(&mut **tx)
        .await?)
}

fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

fn uuid_of(params: &Value, key: &str) -> Result<Option<Uuid>> {
    text(params, key).map(|v| v.parse().map_err(|_| Error::field(key, "must be a record id"))).transpose()
}

/// A post of this course (anything else reads as not found).
async fn post_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, post: Uuid, lock: bool) -> Result<Doc> {
    let found = if lock { load_locked(k, ctx, tx, "LmsPost", post).await } else { load(k, ctx, tx, "LmsPost", post).await };
    match found {
        Ok(p) if id(&p, "course") == id(course, "id") => Ok(p),
        Ok(_) | Err(Error::NotFound(_)) => Err(Error::field("post", "is not a post of this course")),
        Err(e) => Err(e),
    }
}

fn with_follower(thread: &Doc, email: &str, on: bool) -> Option<Value> {
    let mut list = rows(thread, "followers");
    let there = list.iter().any(|r| s(r, "follower").is_some_and(|f| same_email(f, email)));
    if there == on {
        return None;
    }
    if on {
        let mut r = Doc::new();
        r.insert("follower".into(), json!(email.to_lowercase()));
        list.push(r);
    } else {
        list.retain(|r| !s(r, "follower").is_some_and(|f| same_email(f, email)));
    }
    Some(Value::Array(list.into_iter().map(Value::Object).collect()))
}

/// On courses: posting, following and reporting.
pub struct CourseHook;

#[async_trait]
impl DocHook for CourseHook {
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if !["discuss", "follow", "report_post"].contains(&name) {
            return Ok(None);
        }
        let (k, ctx) = (hc.kernel, hc.ctx);
        if ctx.user.is_nil() || ctx.email.is_empty() || !in_course(ctx, hc.tx, doc).await? {
            return Err(Error::Forbidden("the course's discussion is for its learners and instructors".into()));
        }
        let me = ctx.email.to_lowercase();
        let engine = elevated(ctx);
        let out = match name {
            "discuss" => {
                let body = text(params, "body").ok_or_else(|| Error::field("body", "write something"))?;
                if body.chars().count() > MAX_BODY {
                    return Err(Error::field("body", format!("at most {MAX_BODY} characters")));
                }
                let name = if ctx.full_name.trim().is_empty() { me.clone() } else { ctx.full_name.clone() };
                let mut post = Map::new();
                for (f, v) in [
                    ("course", json!(doc_id(doc)?.to_string())),
                    ("body", json!(body)),
                    ("author", json!(me)),
                    ("author_name", json!(name)),
                    ("by_instructor", json!(authors(ctx, doc))),
                    ("last_post_at", json!(util::ts(util::now()))),
                    ("replies", json!(0)),
                ] {
                    post.insert(f.into(), v);
                }
                match uuid_of(params, "thread")? {
                    Some(t) => {
                        let thread = post_of(k, ctx, hc.tx, doc, t, true).await?;
                        if id(&thread, "thread").is_some() {
                            return Err(Error::field("thread", "reply in the thread, not to a reply"));
                        }
                        if b(&thread, "closed") || b(&thread, "removed") {
                            return Err(Error::invalid("this thread takes no more replies"));
                        }
                        post.insert("thread".into(), json!(t.to_string()));
                        post.insert("lesson".into(), thread.get("lesson").cloned().unwrap_or(Value::Null));
                        let reply = k.insert_tx(&engine, hc.tx, "LmsPost", post).await?;
                        let mut patch = Map::new();
                        patch.insert("replies".into(), json!(int(&thread, "replies") + 1));
                        patch.insert("last_post_at".into(), json!(util::ts(util::now())));
                        if let Some(f) = with_follower(&thread, &me, true) {
                            patch.insert("followers".into(), f);
                        }
                        k.update_tx(&engine, hc.tx, "LmsPost", t, patch, None).await?;
                        // Followers hear of it, not its writer.
                        let to: Vec<String> =
                            rows(&thread, "followers").iter().filter_map(|r| s(r, "follower")).filter(|f| !same_email(f, &me)).map(str::to_string).collect();
                        let mut values = Map::new();
                        values.insert("thread_title".into(), thread.get("title").cloned().unwrap_or(Value::Null));
                        values.insert("course_title".into(), doc.get("title").cloned().unwrap_or(Value::Null));
                        notify(k, ctx, hc.tx, "lms.discussion_reply", doc_id(&reply)?, &to, values).await?;
                        json!({"post": reply})
                    }
                    None => {
                        let title = text(params, "title").ok_or_else(|| Error::field("title", "give the thread a title"))?;
                        if let Some(l) = uuid_of(params, "lesson")? {
                            match load(k, ctx, hc.tx, "LmsLesson", l).await {
                                Ok(lesson) if id(&lesson, "course") == id(doc, "id") => {}
                                _ => return Err(Error::field("lesson", "is not a lesson of this course")),
                            }
                            post.insert("lesson".into(), json!(l.to_string()));
                        }
                        post.insert("title".into(), json!(util::cut(title, 200)));
                        post.insert("followers".into(), json!([{"follower": me}]));
                        json!({"post": k.insert_tx(&engine, hc.tx, "LmsPost", post).await?})
                    }
                }
            }
            "follow" => {
                let t = uuid_of(params, "thread")?.ok_or_else(|| Error::field("thread", "is required"))?;
                let thread = post_of(k, ctx, hc.tx, doc, t, true).await?;
                let on = params.get("follow").and_then(Value::as_bool).unwrap_or(true);
                if let Some(f) = with_follower(&thread, &me, on) {
                    let mut patch = Map::new();
                    patch.insert("followers".into(), f);
                    k.update_tx(&engine, hc.tx, "LmsPost", t, patch, None).await?;
                }
                json!({"following": on})
            }
            _ => {
                let p = uuid_of(params, "post")?.ok_or_else(|| Error::field("post", "is required"))?;
                let reason = text(params, "reason").ok_or_else(|| Error::field("reason", "say why"))?;
                let post = post_of(k, ctx, hc.tx, doc, p, true).await?;
                let mut patch = Map::new();
                patch.insert("reported".into(), json!(true));
                patch.insert("report_reason".into(), json!(util::cut(reason, 2000)));
                patch.insert("reported_by".into(), json!(me));
                k.update_tx(&engine, hc.tx, "LmsPost", p, patch, None).await?;
                let mut values = Map::new();
                values.insert("course_title".into(), doc.get("title").cloned().unwrap_or(Value::Null));
                values.insert("reason".into(), json!(util::cut(reason, 2000)));
                notify(k, ctx, hc.tx, "lms.post_reported", doc_id(&post)?, &util::instructors(doc), values).await?;
                json!({"reported": true})
            }
        };
        Ok(Some(out))
    }
}

/// A package template sent to these people (their user, for the in-app channel, when they have one).
async fn notify(k: &Kernel, ctx: &Ctx, tx: &mut Tx, template: &str, post: Uuid, to: &[String], values: Map<String, Value>) -> Result<()> {
    if to.is_empty() {
        return Ok(());
    }
    let found = util::users(tx, ctx.tenant, to).await?;
    let people: Vec<Recipient> = to
        .iter()
        .map(|e| match found.iter().find(|u| same_email(&u.email, e)) {
            Some(u) => Recipient { user: Some(u.id), email: Some(u.email.clone()), ..Default::default() },
            None => Recipient::address(e),
        })
        .collect();
    let m = Message { template: template.into(), entity: Some("LmsPost".into()), record: Some(post), to: people, values, ..Default::default() };
    ferrum_kernel::messages::send(k, tx, &sys(ctx), &m).await?;
    Ok(())
}

/// On posts: written only by the course's actions; moderated by its instructors.
pub struct PostHook;

#[async_trait]
impl DocHook for PostHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid("posts are written on the course page (the course's discuss action) and moderated with their actions"));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if !["pin", "close", "mark_answer", "remove"].contains(&name) {
            return Ok(None);
        }
        let (k, ctx) = (hc.kernel, hc.ctx);
        authored_course(k, ctx, hc.tx, id(doc, "course").unwrap_or_default()).await?;
        let mut patch = Map::new();
        match name {
            "pin" => patch.insert("pinned".into(), json!(!b(doc, "pinned"))),
            "close" => patch.insert("closed".into(), json!(!b(doc, "closed"))),
            "mark_answer" => patch.insert("answer".into(), json!(!b(doc, "answer"))),
            _ => {
                let reason = text(params, "reason").ok_or_else(|| Error::field("reason", "say why"))?;
                patch.insert("removed".into(), json!(true));
                patch.insert("removed_reason".into(), json!(util::cut(reason, 2000)));
                patch.insert("removed_by".into(), json!(ctx.email.to_lowercase()))
            }
        };
        let done = k.update_tx(&elevated(ctx), hc.tx, "LmsPost", doc_id(doc)?, patch, None).await?;
        Ok(Some(json!({"post": done})))
    }
}

/// The course's discussion as the caller reads it (`lesson`: a lesson's; `thread`: one thread with its
/// replies). Pinned threads first, then the latest. Learners read a removed post as removed, without
/// its text; instructors read everything.
pub async fn page(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, lesson: Option<Uuid>, thread: Option<Uuid>) -> Result<Value> {
    if !in_course(ctx, tx, course).await? {
        return Err(Error::NotFound("discussion".into()));
    }
    let author = authors(ctx, course);
    let shown = |p: &Doc| -> Value {
        let gone = b(p, "removed") && !author;
        let mut v = json!({
            "id": p.get("id"), "title": p.get("title"), "body": if gone { Value::Null } else { p.get("body").cloned().unwrap_or(Value::Null) },
            "author_name": if gone { Value::Null } else { p.get("author_name").cloned().unwrap_or(Value::Null) },
            "by_instructor": p.get("by_instructor"), "answer": p.get("answer"), "pinned": p.get("pinned"), "closed": p.get("closed"),
            "removed": p.get("removed"), "replies": p.get("replies"), "last_post_at": p.get("last_post_at"), "created_at": p.get("created_at"),
            "lesson": p.get("lesson"), "mine": s(p, "author").is_some_and(|a| same_email(a, &ctx.email)),
        });
        if author {
            for f in ["author", "reported", "report_reason", "removed_reason", "removed_by"] {
                v[f] = p.get(f).cloned().unwrap_or(Value::Null);
            }
        }
        v
    };
    let cid = doc_id(course)?;
    if let Some(t) = thread {
        let head = post_of(k, ctx, tx, course, t, false).await.map_err(|_| Error::NotFound("thread".into()))?;
        let ids: Vec<Uuid> = sqlx::query_scalar("select id from e_lms_post where tenant_id = $1 and thread = $2 order by created_at, id limit 2000")
            .bind(ctx.tenant)
            .bind(t)
            .fetch_all(&mut **tx)
            .await?;
        let mut replies = Vec::with_capacity(ids.len());
        for r in ids {
            replies.push(shown(&load(k, ctx, tx, "LmsPost", r).await?));
        }
        let following = rows(&head, "followers").iter().any(|r| s(r, "follower").is_some_and(|f| same_email(f, &ctx.email)));
        return Ok(json!({"thread": shown(&head), "replies": replies, "following": following, "moderator": author}));
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "select id from e_lms_post where tenant_id = $1 and course = $2 and thread is null and lesson is not distinct from $3 \
         order by coalesce(pinned, false) desc, last_post_at desc nulls last, id desc limit 500",
    )
    .bind(ctx.tenant)
    .bind(cid)
    .bind(lesson)
    .fetch_all(&mut **tx)
    .await?;
    let mut threads = Vec::with_capacity(ids.len());
    for t in ids {
        let p = load(k, ctx, tx, "LmsPost", t).await?;
        if b(&p, "removed") && !author {
            continue;
        }
        threads.push(shown(&p));
    }
    Ok(json!({"threads": threads, "moderator": author}))
}
