//! Lesson videos uploaded to the workspace (LM-01, as far as storage and delivery go): played from
//! the platform's file storage with range requests, only to people who may open the lesson, through
//! links signed for that person that expire (there is no public URL); captions as a WebVTT track;
//! the seconds watched recorded and the lesson completed at the share watched the lesson asks for.
//!
//! Not here, for a later media component: transcoding to web formats and sizes, adaptive streaming
//! (HLS or DASH renditions chosen by bandwidth), thumbnails and the length read from the file itself,
//! and watching measured by the player's own heartbeats rather than what the page reports.

use crate::enrollment::{self, WITHDRAWN};
use crate::util::{self, doc_id, int, s, sys};
use chrono::{DateTime, Duration, Utc};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, SiteResponse, Tx};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// How long a signed link plays.
const LINK_HOURS: i64 = 4;

/// The signature of a link to `kind` (video or captions) of a lesson, for one person, until `exp`.
pub fn sign(key: &str, tenant: Uuid, lesson: Uuid, user: Uuid, kind: &str, exp: i64) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    h.update(format!("\0{tenant}\0{lesson}\0{user}\0{kind}\0{exp}").as_bytes());
    hex::encode(&h.finalize()[..16])
}

/// Whether a link's signature is right and it has not expired at `now`.
pub fn valid(key: &str, tenant: Uuid, lesson: Uuid, user: Uuid, kind: &str, exp: i64, sig: &str, now: DateTime<Utc>) -> bool {
    exp >= now.timestamp() && exp <= (now + Duration::hours(LINK_HOURS + 1)).timestamp() && sign(key, tenant, lesson, user, kind, exp) == sig
}

/// The key links are signed with: the workspace's own, made on first use (two first uses at once
/// keep the one stored first).
async fn link_key(_k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<String> {
    sqlx::query("insert into lms_video_key (tenant_id, key) values ($1, $2) on conflict (tenant_id) do nothing")
        .bind(ctx.tenant)
        .bind(hex::encode(rand::random::<[u8; 32]>()))
        .execute(&mut **tx)
        .await?;
    Ok(sqlx::query_scalar("select key from lms_video_key where tenant_id = $1").bind(ctx.tenant).fetch_one(&mut **tx).await?)
}

/// The lesson's length in seconds: its own, or its minutes.
fn length(lesson: &Doc) -> i64 {
    let own = int(lesson, "video_seconds");
    if own > 0 { own } else { int(lesson, "duration_minutes") * 60 }
}

/// What the lesson page plays: the signed links, and the person's watching so far.
pub async fn player(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lesson: &Doc, enrollment: Option<&Doc>) -> Result<Value> {
    let lid = doc_id(lesson)?;
    let key = link_key(k, ctx, tx).await?;
    let exp = (util::now() + Duration::hours(LINK_HOURS)).timestamp();
    let link = |kind: &str| format!("/api/app/{}/lessons/{lid}/{kind}?exp={exp}&sig={}", util::PACKAGE, sign(&key, ctx.tenant, lid, ctx.user, kind, exp));
    let watched: Option<(Option<i64>, Option<Decimal>)> = match enrollment {
        Some(e) => {
            sqlx::query_as("select total_seconds, score_percent from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3")
                .bind(ctx.tenant)
                .bind(doc_id(e)?)
                .bind(lid)
                .fetch_optional(&mut **tx)
                .await?
        }
        None => None,
    };
    let (seconds, percent) = watched.unwrap_or_default();
    Ok(json!({
        "src": link("video"), "captions": s(lesson, "captions_file").map(|_| link("captions")), "expires_at": exp,
        "length_seconds": length(lesson), "watch_percent": lesson.get("watch_percent"),
        "seconds_watched": seconds.unwrap_or(0), "percent_watched": percent.map(ferrum_domain::util::money),
    }))
}

/// A lesson's video or captions, for the person its signed link was made for, while they may open
/// the lesson: streamed from storage, in the ranges the player asks for.
pub async fn serve(k: &Kernel, user: &Ctx, tx: &mut Tx, lesson: &Doc, kind: &str, query: &Map<String, Value>) -> Result<SiteResponse> {
    let gone = || Error::NotFound("video".into());
    let lid = doc_id(lesson)?;
    let exp = query.get("exp").and_then(Value::as_str).and_then(|v| v.parse::<i64>().ok()).ok_or_else(gone)?;
    let sig = query.get("sig").and_then(Value::as_str).ok_or_else(gone)?;
    let key = link_key(k, user, tx).await?;
    if !valid(&key, user.tenant, lid, user.user, kind, exp, sig, util::now()) {
        return Err(Error::Forbidden("this video link has expired or is not yours: open the lesson again".into()));
    }
    let field = if kind == "captions" { "captions_file" } else { "video_file" };
    let file = s(lesson, field).and_then(util::file_id).ok_or_else(gone)?;
    let info = k.file_info(&sys(user), file).await?;
    let ctype = if kind == "captions" { "text/vtt; charset=utf-8".to_string() } else { info.content_type };
    Ok(SiteResponse::file(
        200,
        &ctype,
        vec![
            ("accept-ranges".into(), "bytes".into()),
            ("x-content-type-options".into(), "nosniff".into()),
            ("content-security-policy".into(), "default-src 'none'; sandbox".into()),
            ("cache-control".into(), "private, no-store".into()),
        ],
        file,
    ))
}

/// Whether an uploaded video (and its captions) may be a lesson's: stored files of the right kind,
/// the video no larger than Learn Settings allow.
pub async fn check(tx: &mut Tx, tenant: Uuid, lesson: &Doc) -> Result<()> {
    for (field, video) in [("video_file", true), ("captions_file", false)] {
        let Some(path) = s(lesson, field) else { continue };
        let file = util::file_id(path).ok_or_else(|| Error::field(field, "use a file uploaded with /api/files"))?;
        let found: Option<(String, String, i64)> =
            sqlx::query_as("select file_name, content_type, size_bytes from file_object where tenant_id = $1 and id = $2")
                .bind(tenant)
                .bind(file)
                .fetch_optional(&mut **tx)
                .await?;
        let (name, ctype, size) = found.ok_or_else(|| Error::field(field, "no uploaded file has this address"))?;
        if video {
            if !ctype.starts_with("video/") {
                return Err(Error::field(field, "upload a video file"));
            }
            let max_mb: Option<i64> = sqlx::query_scalar("select video_max_mb from e_lms_settings where tenant_id = $1 limit 1")
                .bind(tenant)
                .fetch_optional(&mut **tx)
                .await?
                .flatten();
            let max_mb = max_mb.unwrap_or(2048).max(1);
            if size > max_mb * 1024 * 1024 {
                return Err(Error::field(field, format!("videos may be at most {max_mb} MB (Learn Settings)")));
            }
        } else if !(ctype.starts_with("text/vtt") || name.to_lowercase().ends_with(".vtt")) {
            return Err(Error::field(field, "captions are a WebVTT (.vtt) file"));
        }
    }
    Ok(())
}

/// `video_progress {lesson, seconds}` by the learner: the seconds of the video they have watched (it
/// only grows, and by no more than the time since they last said, so a page cannot claim a whole
/// video at once); at the lesson's share it is completed.
pub async fn progress(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: &Doc, lesson: &Doc, params: &Value) -> Result<Value> {
    if s(enrollment, "status") == Some(WITHDRAWN) {
        return Err(Error::invalid("this enrolment is withdrawn: enrol again to continue"));
    }
    if s(lesson, "video_file").is_none() {
        return Err(Error::field("lesson", "is not a lesson with an uploaded video"));
    }
    let said = params.get("seconds").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|x| x.trim().parse().ok()))).filter(|v| *v >= 0);
    let said = said.ok_or_else(|| Error::field("seconds", "is required (whole seconds watched)"))?;
    let (eid, lid) = (doc_id(enrollment)?, doc_id(lesson)?);
    let total = length(lesson).max(1);
    let row: Option<(Uuid, Option<i64>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "select id, total_seconds, last_commit_at from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3 for update",
    )
    .bind(ctx.tenant)
    .bind(eid)
    .bind(lid)
    .fetch_optional(&mut **tx)
    .await?;
    let now = util::now();
    let (before, last) = row.as_ref().map(|r| (r.1.unwrap_or(0), r.2)).unwrap_or((0, None));
    // ponytail: the page's word, bounded by wall-clock time; a media component's own heartbeats would replace it.
    let allowed = before + last.map_or(0, |l| (now - l).num_seconds().max(0)) + 30;
    let seconds = said.min(allowed).max(before).min(total);
    let percent = (Decimal::from(seconds) * Decimal::ONE_HUNDRED / Decimal::from(total)).round_dp(2);
    let needed = int(lesson, "watch_percent");
    let done = needed > 0 && percent >= Decimal::from(needed);
    let mut v = Doc::new();
    v.insert("total_seconds".into(), json!(seconds));
    v.insert("score_percent".into(), ferrum_domain::util::money(percent));
    v.insert("status".into(), json!(if done { "Completed" } else { "Incomplete" }));
    v.insert("last_commit_at".into(), json!(util::ts(now)));
    match row {
        Some((r, ..)) => {
            k.set_values_tx(&sys(ctx), tx, "LmsPackageProgress", r, v).await?;
        }
        None => {
            for (f, x) in [
                ("enrollment", json!(eid.to_string())),
                ("lesson", json!(lid.to_string())),
                ("course", enrollment.get("course").cloned().unwrap_or(Value::Null)),
                ("learner", enrollment.get("learner").cloned().unwrap_or(Value::Null)),
                ("commits", json!(1)),
            ] {
                v.insert(f.into(), x);
            }
            k.insert_tx(&util::elevated(ctx), tx, "LmsPackageProgress", v).await?;
        }
    }
    if done {
        enrollment::record_completion(k, ctx, tx, eid, lid, "Video").await?;
    }
    Ok(json!({"seconds_watched": seconds, "percent_watched": ferrum_domain::util::money(percent), "completed": done}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_signed_for_one_person_and_expire() {
        let (t, l, u) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let now = util::now();
        let exp = (now + Duration::hours(LINK_HOURS)).timestamp();
        let sig = sign("k", t, l, u, "video", exp);
        assert!(valid("k", t, l, u, "video", exp, &sig, now));
        assert!(!valid("k", t, l, Uuid::now_v7(), "video", exp, &sig, now), "another person");
        assert!(!valid("k", t, l, u, "captions", exp, &sig, now), "another file");
        assert!(!valid("other", t, l, u, "video", exp, &sig, now), "another key");
        assert!(!valid("k", t, l, u, "video", exp, &sig, now + Duration::hours(LINK_HOURS + 1)), "expired");
        let far = (now + Duration::days(30)).timestamp();
        assert!(!valid("k", t, l, u, "video", far, &sign("k", t, l, u, "video", far), now), "no link lives longer than the hours given");
    }
}
