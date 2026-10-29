//! The catalog: courses, chapters, lessons, learning paths and Learn Settings. Content belongs to
//! its course, and only the course's instructors (and its creator) and LMS managers change it —
//! whatever the role grants, the hooks check authorship of the course on every write.

use crate::enrollment;
use crate::util::{self, authored_course, file_id, id, rows, s, slugify, valid_slug};
use crate::video;
use async_trait::async_trait;
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

pub struct SettingsHook;

#[async_trait]
impl DocHook for SettingsHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(hosts) = s(doc, "video_hosts") {
            video::parse_hosts(hosts).map_err(|m| Error::field("video_hosts", m))?;
        }
        Ok(())
    }
}

// ───────────────────────────── courses ─────────────────────────────

pub struct CourseHook;

/// Whether another course already uses `slug`.
async fn slug_taken(hc: &mut HookCtx<'_>, slug: &str, me: Option<Uuid>) -> Result<bool> {
    Ok(sqlx::query_scalar("select exists(select 1 from e_lms_course where tenant_id = $1 and slug = $2 and id is distinct from $3)")
        .bind(hc.ctx.tenant)
        .bind(slug)
        .bind(me)
        .fetch_one(&mut **hc.tx)
        .await?)
}

#[async_trait]
impl DocHook for CourseHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let me = id(doc, "id");
        if let Some(t) = s(doc, "tags").map(util::normalize_tags) {
            doc.insert("tags".into(), if t.is_empty() { Value::Null } else { json!(t) });
        }
        match s(doc, "slug").map(|v| v.trim().to_lowercase()) {
            Some(slug) => {
                doc.insert("slug".into(), json!(slug));
            }
            None => {
                // A free slug from the title: `intro`, `intro-2`, `intro-3`...
                let mut base = slugify(s(doc, "title").unwrap_or_default());
                if base.is_empty() {
                    base = format!("course-{}", &me.unwrap_or_else(Uuid::now_v7).simple().to_string()[..8]);
                }
                let mut slug = base.clone();
                let mut n = 1;
                while slug_taken(hc, &slug, me).await? {
                    n += 1;
                    slug = if n > 50 { format!("{base}-{}", &Uuid::now_v7().simple().to_string()[24..]) } else { format!("{base}-{n}") };
                }
                doc.insert("slug".into(), json!(slug));
            }
        }
        util::people_rows(hc.tx, hc.ctx.tenant, doc, "instructors", "instructor", "instructor_name").await?;
        if s(doc, "status") == Some("Published") && s(doc, "published_at").is_none() {
            doc.insert("published_at".into(), json!(util::ts(util::now())));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = util::doc_id(doc)?;
        let mut errors = Vec::new();
        let slug = s(doc, "slug").unwrap_or_default();
        if !valid_slug(slug) {
            errors.push(FieldError::new("slug", "use lower-case letters, digits and single hyphens (at most 100 characters)"));
        } else if slug_taken(hc, slug, Some(me)).await? {
            errors.push(FieldError::new("slug", "another course already uses it"));
        }
        let prerequisites: Vec<Uuid> = rows(doc, "prerequisites").iter().filter_map(|r| id(r, "course")).collect();
        let mut seen = HashSet::new();
        for (i, p) in prerequisites.iter().enumerate() {
            if *p == me {
                errors.push(FieldError::new("course", "a course cannot require itself").in_row("prerequisites", i));
            } else if !seen.insert(*p) {
                errors.push(FieldError::new("course", "is listed twice").in_row("prerequisites", i));
            }
        }
        if !prerequisites.is_empty() && errors.is_empty() {
            // Requiring a course that (transitively) requires this one would lock both forever.
            let cycle: bool = sqlx::query_scalar(
                "with recursive pre(id, depth) as (select unnest($2::uuid[]), 1 \
                   union all select p.course, pre.depth + 1 from pre join e_lms_course__prerequisites p \
                   on p.tenant_id = $1 and p.parent_id = pre.id where pre.depth < 50) \
                 select exists(select 1 from pre where id = $3)",
            )
            .bind(hc.ctx.tenant)
            .bind(&prerequisites)
            .bind(me)
            .fetch_one(&mut **hc.tx)
            .await?;
            if cycle {
                errors.push(FieldError::new("prerequisites", "these prerequisites (through their own) require this course: that would be a cycle"));
            }
        }
        if s(doc, "status") == Some("Published") {
            let (lessons, empty_quizzes): (i64, i64) = sqlx::query_as(
                "select count(*), count(*) filter (where l.lesson_type = 'Quiz' and coalesce(q.question_count, 0) = 0) \
                 from e_lms_lesson l left join e_lms_quiz q on q.tenant_id = l.tenant_id and q.id = l.quiz \
                 where l.tenant_id = $1 and l.course = $2",
            )
            .bind(hc.ctx.tenant)
            .bind(me)
            .fetch_one(&mut **hc.tx)
            .await?;
            if lessons == 0 {
                errors.push(FieldError::new("status", "add at least one lesson before publishing"));
            } else if empty_quizzes > 0 {
                errors.push(FieldError::new("status", "every quiz lesson needs a quiz with questions before publishing"));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "enroll_learners" {
            return Ok(None);
        }
        let course = authored_course(hc.kernel, hc.ctx, hc.tx, util::doc_id(doc)?).await?;
        let learners: Vec<String> = match params.get("learners") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(Value::String(v)) => v.split([',', ';', '\n']).map(str::to_string).collect(),
            _ => return Err(Error::field("learners", "list the learners' emails")),
        };
        let cohort = match params.get("cohort") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().and_then(|c| c.parse::<Uuid>().ok()).ok_or_else(|| Error::field("cohort", "must be a cohort id"))?),
        };
        Ok(Some(enrollment::enroll_many(hc.kernel, hc.ctx, hc.tx, &course, &learners, cohort).await?))
    }
}

// ───────────────────────────── chapters ─────────────────────────────

pub struct ChapterHook;

#[async_trait]
impl DocHook for ChapterHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("position").is_none_or(Value::is_null)
            && let Some(course) = id(doc, "course")
        {
            let next: i64 = sqlx::query_scalar("select coalesce(max(position), 0) + 1 from e_lms_chapter where tenant_id = $1 and course = $2")
                .bind(hc.ctx.tenant)
                .bind(course)
                .fetch_one(&mut **hc.tx)
                .await?;
            doc.insert("position".into(), json!(next));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if let Some(old) = old
            && old.get("course") != doc.get("course")
            && lesson_count(hc, "chapter", util::doc_id(doc)?).await? > 0
        {
            return Err(Error::field("course", "a chapter with lessons cannot move to another course"));
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(course) = id(doc, "course") {
            authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        }
        if lesson_count(hc, "chapter", util::doc_id(doc)?).await? > 0 {
            return Err(Error::invalid("delete or move the chapter's lessons first"));
        }
        Ok(())
    }
}

async fn lesson_count(hc: &mut HookCtx<'_>, column: &str, value: Uuid) -> Result<i64> {
    let sql = match column {
        "chapter" => "select count(*) from e_lms_lesson where tenant_id = $1 and chapter = $2",
        _ => "select count(*) from e_lms_lesson where tenant_id = $1 and course = $2",
    };
    Ok(sqlx::query_scalar(sql).bind(hc.ctx.tenant).bind(value).fetch_one(&mut **hc.tx).await?)
}

// ───────────────────────────── lessons ─────────────────────────────

pub struct LessonHook;

#[async_trait]
impl DocHook for LessonHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(chapter) = id(doc, "chapter") {
            let course: Option<Uuid> = sqlx::query_scalar("select course from e_lms_chapter where tenant_id = $1 and id = $2")
                .bind(hc.ctx.tenant)
                .bind(chapter)
                .fetch_optional(&mut **hc.tx)
                .await?;
            doc.insert("course".into(), course.map_or(Value::Null, |c| json!(c.to_string())));
            if doc.get("position").is_none_or(Value::is_null) {
                let next: i64 = sqlx::query_scalar("select coalesce(max(position), 0) + 1 from e_lms_lesson where tenant_id = $1 and chapter = $2")
                    .bind(hc.ctx.tenant)
                    .bind(chapter)
                    .fetch_one(&mut **hc.tx)
                    .await?;
                doc.insert("position".into(), json!(next));
            }
        }
        let embed = match s(doc, "video_url") {
            Some(url) => {
                let hosts = util::settings(hc.tx, hc.ctx.tenant).await?.video_hosts;
                json!(video::embed_url(url, &hosts).map_err(|m| Error::field("video_url", format!("the video link {m}")))?)
            }
            None => Value::Null,
        };
        doc.insert("video_embed_url".into(), embed);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(course) = id(doc, "course") else { return Ok(()) };
        authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        if let Some(path) = s(doc, "attachment") {
            match file_id(path) {
                Some(file) => {
                    let exists: bool = sqlx::query_scalar("select exists(select 1 from file_object where tenant_id = $1 and id = $2)")
                        .bind(hc.ctx.tenant)
                        .bind(file)
                        .fetch_one(&mut **hc.tx)
                        .await?;
                    if !exists {
                        return Err(Error::field("attachment", "no uploaded file has this address"));
                    }
                }
                None if path.starts_with("https://") => {}
                None => return Err(Error::field("attachment", "use a file uploaded with /api/files or an https link")),
            }
        }
        for (field, table) in [("quiz", "e_lms_quiz"), ("assignment", "e_lms_assignment")] {
            if let Some(target) = id(doc, field) {
                let owner: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select course from {table} where tenant_id = $1 and id = $2")))
                    .bind(hc.ctx.tenant)
                    .bind(target)
                    .fetch_optional(&mut **hc.tx)
                    .await?;
                if owner.is_some_and(|o| o != course) {
                    return Err(Error::field(field, format!("this {field} belongs to another course")));
                }
            }
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let changed = |f: &str| old.is_none_or(|o| o.get(f) != doc.get(f));
        let me = util::doc_id(doc)?;
        let mut courses: Vec<Uuid> = id(doc, "course").into_iter().collect();
        if let Some(previous) = old.and_then(|o| id(o, "course")).filter(|p| Some(*p) != id(doc, "course")) {
            if lesson_progress(hc, me).await? > 0 {
                return Err(Error::field("chapter", "learners have completed this lesson: it cannot move to another course"));
            }
            courses.push(previous);
        }
        if changed("course") || changed("duration_minutes") {
            for c in &courses {
                refresh_course_stats(hc, *c, None).await?;
            }
        }
        if changed("course") || changed("required") {
            for c in &courses {
                enrollment::refresh_course(hc.tx, hc.ctx.tenant, *c, None).await?;
            }
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = util::doc_id(doc)?;
        let Some(course) = id(doc, "course") else { return Ok(()) };
        authored_course(hc.kernel, hc.ctx, hc.tx, course).await?;
        if lesson_progress(hc, me).await? > 0 {
            return Err(Error::invalid("learners have completed this lesson: make it optional instead of deleting it"));
        }
        refresh_course_stats(hc, course, Some(me)).await?;
        enrollment::refresh_course(hc.tx, hc.ctx.tenant, course, Some(me)).await
    }
}

async fn lesson_progress(hc: &mut HookCtx<'_>, lesson: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar("select count(*) from e_lms_lesson_progress where tenant_id = $1 and lesson = $2")
        .bind(hc.ctx.tenant)
        .bind(lesson)
        .fetch_one(&mut **hc.tx)
        .await?)
}

/// The course's lesson count and total duration (without `excluding`, a lesson being deleted).
async fn refresh_course_stats(hc: &mut HookCtx<'_>, course: Uuid, excluding: Option<Uuid>) -> Result<()> {
    sqlx::query(
        "update e_lms_course c set lesson_count = x.n, total_minutes = x.m, updated_at = now() \
         from (select count(*) as n, coalesce(sum(duration_minutes), 0) as m from e_lms_lesson \
               where tenant_id = $1 and course = $2 and id is distinct from $3) x \
         where c.tenant_id = $1 and c.id = $2",
    )
    .bind(hc.ctx.tenant)
    .bind(course)
    .bind(excluding)
    .execute(&mut **hc.tx)
    .await?;
    Ok(())
}

// ───────────────────────────── learning paths ─────────────────────────────

pub struct PathHook;

#[async_trait]
impl DocHook for PathHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let courses = rows(doc, "courses");
        if courses.len() > 50 {
            return Err(Error::field("courses", "a learning path holds at most 50 courses"));
        }
        let mut seen = HashSet::new();
        for (i, r) in courses.iter().enumerate() {
            if let Some(c) = id(r, "course")
                && !seen.insert(c)
            {
                return Err(Error::Validation(vec![FieldError::new("course", "is listed twice").in_row("courses", i)]));
            }
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "join" {
            return Ok(None);
        }
        if hc.ctx.user.is_nil() || hc.ctx.email.is_empty() {
            return Err(Error::Forbidden("sign in to join a learning path".into()));
        }
        let me = hc.ctx.email.to_lowercase();
        let mut enrolled = Vec::new();
        let mut already = Vec::new();
        let mut skipped = Vec::new();
        for r in rows(doc, "courses") {
            let Some(course) = id(&r, "course") else { continue };
            let title = s(&r, "course_title").unwrap_or_default().to_string();
            match enrollment::try_enroll(hc.kernel, hc.ctx, hc.tx, course, &me, None).await? {
                enrollment::Outcome::Enrolled(e) => enrolled.push(json!({"course": course, "course_title": title, "enrollment": e.get("id")})),
                enrollment::Outcome::Already(e) => already.push(json!({"course": course, "course_title": title, "enrollment": e})),
                enrollment::Outcome::Refused(why) => skipped.push(json!({"course": course, "course_title": title, "reason": why})),
            }
        }
        Ok(Some(json!({"enrolled": enrolled, "already_enrolled": already, "skipped": skipped})))
    }
}

/// Lesson types learners complete by marking them done.
pub fn self_paced(lesson_type: &str) -> bool {
    matches!(lesson_type, "Article" | "Video" | "Document")
}

/// The required-lesson count rule: required lessons, or every lesson when none is marked required.
pub fn effective_total(required: i64, all: i64) -> i64 {
    if required > 0 { required } else { all }
}

pub fn is_published(course: &Doc) -> bool {
    s(course, "status") == Some("Published")
}
