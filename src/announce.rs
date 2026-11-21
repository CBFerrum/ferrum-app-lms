//! Course announcements (LMS-12): something an instructor tells everyone in a course. The course's `announce`
//! action keeps the announcement and queues a message to each learner whose enrolment is not withdrawn, in one
//! transaction; the course page reads them through `courses/{slug}/announcements`. Nobody writes an announcement
//! record directly.

use crate::util::{self, authored_course, doc_id, elevated, same_email};
use async_trait::async_trait;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Result, Tx};
use serde_json::{Map, Value, json};
use uuid::Uuid;

const MAX_BODY: usize = 20_000;
const MAX_TITLE: usize = 200;
/// Most learners one announcement is queued for.
const MAX_TOLD: usize = 5000;

fn checked<'a>(params: &'a Value, key: &str, max: usize, multiline: bool) -> Result<&'a str> {
    let v = params.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| Error::field(key, "is required"))?;
    if v.chars().count() > max {
        return Err(Error::field(key, format!("at most {max} characters")));
    }
    if v.chars().any(|c| c.is_control() && !(multiline && (c == '\n' || c == '\t' || c == '\r'))) {
        return Err(Error::field(key, "holds a character that cannot be kept"));
    }
    Ok(v)
}

/// On courses: the `announce` action.
pub struct AnnounceHook;

#[async_trait]
impl DocHook for AnnounceHook {
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "announce" {
            return Ok(None);
        }
        let (k, ctx) = (hc.kernel, hc.ctx);
        let course = authored_course(k, ctx, hc.tx, doc_id(doc)?).await?;
        let title = checked(params, "title", MAX_TITLE, false)?;
        let body = checked(params, "body", MAX_BODY, true)?;
        let me = ctx.email.to_lowercase();
        // Everyone with a live enrolment, once, but the author.
        let mut to: Vec<String> = sqlx::query_scalar(
            "select distinct learner from e_lms_enrollment where tenant_id = $1 and course = $2 and status <> 'Withdrawn' order by learner limit $3",
        )
        .bind(ctx.tenant)
        .bind(doc_id(&course)?)
        .bind(MAX_TOLD as i64 + 1)
        .fetch_all(&mut **hc.tx)
        .await?;
        if to.len() > MAX_TOLD {
            return Err(Error::invalid(format!("this course has more than {MAX_TOLD} learners: announcing to it in parts is not possible yet")));
        }
        to.retain(|e| !same_email(e, &me));
        let name = if ctx.full_name.trim().is_empty() { me.clone() } else { ctx.full_name.clone() };
        let mut a = Map::new();
        for (f, v) in [
            ("course", json!(doc_id(&course)?.to_string())),
            ("title", json!(title)),
            ("body", json!(body)),
            ("author", json!(me)),
            ("author_name", json!(name)),
            ("sent_at", json!(util::ts(util::now()))),
            ("told", json!(to.len())),
        ] {
            a.insert(f.into(), v);
        }
        let kept = k.insert_tx(&elevated(ctx), hc.tx, "LmsAnnouncement", a).await?;
        let mut values = Map::new();
        values.insert("course_title".into(), course.get("title").cloned().unwrap_or(Value::Null));
        util::tell(k, ctx, hc.tx, "lms.course_announcement", "LmsAnnouncement", doc_id(&kept)?, &to, values).await?;
        Ok(Some(json!({"announcement": doc_id(&kept)?, "told": to.len()})))
    }
}

/// On announcements: written only by the course's action.
pub struct NoticeHook;

#[async_trait]
impl DocHook for NoticeHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid("announcements are written with the course's Announce action"));
        }
        Ok(())
    }
}

/// The course's announcements, newest first, for whoever is in the course (its authors and learners with a live enrolment).
pub async fn page(ctx: &Ctx, tx: &mut Tx, course: &Doc) -> Result<Value> {
    let author = util::authors(ctx, course);
    if !author {
        let enrolled: bool =
            sqlx::query_scalar("select exists(select 1 from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 and status <> 'Withdrawn')")
                .bind(ctx.tenant)
                .bind(doc_id(course)?)
                .bind(ctx.email.to_lowercase())
                .fetch_one(&mut **tx)
                .await?;
        if !enrolled {
            return Err(Error::NotFound("announcements".into()));
        }
    }
    let found: Vec<(Uuid, String, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "select id, title, body, author_name, sent_at from e_lms_announcement where tenant_id = $1 and course = $2 order by sent_at desc nulls last, id desc limit 100",
    )
    .bind(ctx.tenant)
    .bind(doc_id(course)?)
    .fetch_all(&mut **tx)
    .await?;
    let list: Vec<Value> = found
        .into_iter()
        .map(|(id, title, body, by, at)| json!({"id": id, "title": title, "body": body, "author_name": by, "sent_at": at.map(util::ts)}))
        .collect();
    Ok(json!({"announcements": list, "author": author}))
}
