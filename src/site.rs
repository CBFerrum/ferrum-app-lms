//! The package's HTTP endpoints (see packages/lms/README.md):
//!
//! * `/api/site/{tenant}/lms/...` — the public: the catalog of public published courses, course
//!   pages (outline; content of free-preview lessons only) and certificate verification. Read-only;
//!   anything not public answers exactly like something missing.
//! * `/api/app/lms/...` — signed-in people with a Learn role: the catalog, course and lesson pages
//!   (content for enrolled learners, authors and previews), the learner dashboard (my courses,
//!   next lessons, due assignments, upcoming sessions, certificates) and the instructor dashboard
//!   (my courses, roster with progress).
//!
//! Every request acts with the engine's context after its own checks, and returns only what the
//! caller may see. Changes go through the generic API (enrolments, attempts, submissions).

use crate::certificate;
use crate::cohort;
use crate::enrollment::{self, WITHDRAWN};
use crate::util::{self, authors, id, is_manager, is_member, load, rows, s, sys, valid_slug};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrum_domain::util::money;
use ferrum_kernel::{Action, Ctx, Doc, Error, Kernel, Result, Site, SiteRequest, SiteResponse, Tx, row_scope};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

/// Most courses one catalog page lists.
const MAX_PAGE: i64 = 50;

pub struct LearnSite;

#[async_trait]
impl Site for LearnSite {
    fn max_body(&self) -> usize {
        16 * 1024
    }

    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        let path: Vec<&str> = req.path.split('/').filter(|p| !p.is_empty()).collect();
        if req.method != "GET" {
            return Err(Error::NotFound("endpoint".into()));
        }
        match &req.user {
            None => public(k, &req, &path).await,
            Some(user) => {
                if !is_member(user) {
                    return Err(Error::Forbidden("Learn needs the Learner, Instructor or LMS Manager role".into()));
                }
                signed_in(k, user, &req, &path).await
            }
        }
    }
}

fn param<'a>(req: &'a SiteRequest, key: &str) -> Option<&'a str> {
    req.query.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

fn json_response(v: &Value, cache: &str) -> SiteResponse {
    let mut r = SiteResponse::json(200, v);
    r.headers.push(("cache-control".into(), cache.into()));
    r
}

fn not_found(what: &str) -> Error {
    Error::NotFound(what.into())
}

fn uuid(seg: &str) -> Result<Uuid> {
    seg.parse().map_err(|_| not_found("record"))
}

// ───────────────────────────── catalog ─────────────────────────────

/// Who is looking at the catalog.
#[derive(Clone, Copy, PartialEq)]
enum Audience {
    Public,
    Members,
}

#[derive(sqlx::FromRow)]
struct Card {
    id: Uuid,
    slug: String,
    title: String,
    summary: Option<String>,
    cover_image: Option<String>,
    level: Option<String>,
    tags: Option<String>,
    estimated_hours: Option<Decimal>,
    lesson_count: Option<i64>,
    total_minutes: Option<i64>,
    issue_certificate: bool,
    published_at: Option<DateTime<Utc>>,
    visibility: String,
    category: Option<Uuid>,
    category_name: Option<String>,
}

const CARD_COLUMNS: &str = "c.id, c.slug, c.title, c.summary, c.cover_image, c.level, c.tags, c.estimated_hours, c.lesson_count, c.total_minutes, \
     c.issue_certificate, c.published_at, c.visibility, c.category, cat.category_name";

fn card(c: &Card, instructors: &[(Uuid, String)]) -> Value {
    json!({
        "id": c.id, "slug": c.slug, "title": c.title, "summary": c.summary, "cover_image": c.cover_image, "level": c.level,
        "tags": util::tag_list(c.tags.as_deref().unwrap_or_default()), "estimated_hours": c.estimated_hours.map(money),
        "lessons": c.lesson_count.unwrap_or(0), "total_minutes": c.total_minutes.unwrap_or(0), "certificate": c.issue_certificate,
        "published_at": c.published_at.map(util::ts), "visibility": c.visibility,
        "category": c.category.map(|id| json!({"id": id, "name": c.category_name})),
        "instructors": instructors.iter().filter(|(cid, _)| *cid == c.id).map(|(_, n)| n.clone()).collect::<Vec<_>>(),
    })
}

async fn instructor_names(tx: &mut Tx, tenant: Uuid, courses: &[Uuid]) -> Result<Vec<(Uuid, String)>> {
    if courses.is_empty() {
        return Ok(vec![]);
    }
    Ok(sqlx::query_as(
        "select parent_id, coalesce(nullif(instructor_name, ''), instructor) from e_lms_course__instructors where tenant_id = $1 and parent_id = any($2) order by parent_id, idx",
    )
    .bind(tenant)
    .bind(courses)
    .fetch_all(&mut **tx)
    .await?)
}

/// A page of published courses, newest first: `q` (title, summary, tags), `category` (id),
/// `level`, `tag`, `limit` (≤ 50) and `cursor` (from `next_cursor`).
async fn catalog(tx: &mut Tx, tenant: Uuid, req: &SiteRequest, audience: Audience) -> Result<(Vec<Value>, Vec<Uuid>, Option<String>)> {
    let limit = match param(req, "limit") {
        None => 20,
        Some(v) => v.parse::<i64>().ok().filter(|n| (1..=MAX_PAGE).contains(n)).ok_or_else(|| Error::field("limit", format!("use 1 to {MAX_PAGE}")))?,
    };
    let mut qb = QueryBuilder::<Postgres>::new(format!(
        "select {CARD_COLUMNS} from e_lms_course c left join e_lms_category cat on cat.tenant_id = c.tenant_id and cat.id = c.category \
         where c.tenant_id = "
    ));
    qb.push_bind(tenant).push(" and c.status = 'Published'");
    if audience == Audience::Public {
        qb.push(" and c.visibility = 'Public'");
    }
    if let Some(q) = param(req, "q") {
        if q.chars().count() > 100 {
            return Err(Error::field("q", "at most 100 characters"));
        }
        let like = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        qb.push(" and (c.title ilike ")
            .push_bind(like.clone())
            .push(" or c.summary ilike ")
            .push_bind(like.clone())
            .push(" or c.tags ilike ")
            .push_bind(like)
            .push(")");
    }
    if let Some(cat) = param(req, "category") {
        qb.push(" and c.category = ").push_bind(cat.parse::<Uuid>().map_err(|_| Error::field("category", "must be a category id"))?);
    }
    if let Some(level) = param(req, "level") {
        if !["Beginner", "Intermediate", "Advanced"].contains(&level) {
            return Err(Error::field("level", "use Beginner, Intermediate or Advanced"));
        }
        qb.push(" and c.level = ").push_bind(level.to_string());
    }
    if let Some(tag) = param(req, "tag") {
        let tag = util::tag_list(tag).into_iter().next().ok_or_else(|| Error::field("tag", "is empty"))?;
        qb.push(" and position(', ' || ").push_bind(tag).push(" || ',' in ', ' || coalesce(c.tags, '') || ',') > 0");
    }
    if let Some(cursor) = param(req, "cursor") {
        let bad = || Error::field("cursor", "is not a cursor from this list");
        let raw = String::from_utf8(hex::decode(cursor).map_err(|_| bad())?).map_err(|_| bad())?;
        let (at, cid) = raw.split_once('|').ok_or_else(bad)?;
        let at = ferrum_kernel::value::parse_datetime(at).map_err(|_| bad())?;
        qb.push(" and (c.published_at, c.id) < (").push_bind(at).push(", ").push_bind(cid.parse::<Uuid>().map_err(|_| bad())?).push(")");
    }
    qb.push(" order by c.published_at desc nulls last, c.id desc limit ").push_bind(limit + 1);
    let mut found: Vec<Card> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let next = if found.len() as i64 > limit {
        found.truncate(limit as usize);
        found.last().and_then(|c| c.published_at.map(|at| hex::encode(format!("{}|{}", util::ts(at), c.id))))
    } else {
        None
    };
    let ids: Vec<Uuid> = found.iter().map(|c| c.id).collect();
    let names = instructor_names(tx, tenant, &ids).await?;
    Ok((found.iter().map(|c| card(c, &names)).collect(), ids, next))
}

/// A course by slug, as the given audience may see it (`None`: as missing).
async fn course_by_slug(k: &Kernel, ctx: &Ctx, tx: &mut Tx, slug: &str) -> Result<Option<Doc>> {
    if !valid_slug(slug) {
        return Ok(None);
    }
    let found: Option<Uuid> =
        sqlx::query_scalar("select id from e_lms_course where tenant_id = $1 and slug = $2").bind(ctx.tenant).bind(slug).fetch_optional(&mut **tx).await?;
    match found {
        Some(c) => Ok(Some(load(k, ctx, tx, "LmsCourse", c).await?)),
        None => Ok(None),
    }
}

fn public_course(c: &Doc) -> bool {
    s(c, "status") == Some("Published") && s(c, "visibility") == Some("Public")
}

/// The course page: its description, instructors, prerequisites (those the audience may see),
/// cohorts with free places, and the outline without anyone's progress.
async fn course_page(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: &Doc, audience: Audience) -> Result<Value> {
    let me = util::doc_id(course)?;
    let mut prerequisites = Vec::new();
    for p in rows(course, "prerequisites") {
        let Some(pid) = id(&p, "course") else { continue };
        let pc = load(k, ctx, tx, "LmsCourse", pid).await?;
        let visible = match audience {
            Audience::Public => public_course(&pc),
            Audience::Members => true,
        };
        if visible {
            prerequisites.push(json!({"id": pid, "slug": pc.get("slug"), "title": pc.get("title")}));
        }
    }
    let mut outline = enrollment::outline(k, ctx, tx, course, None).await?;
    for ch in outline["chapters"].as_array_mut().into_iter().flatten() {
        for l in ch["lessons"].as_array_mut().into_iter().flatten() {
            if let Value::Object(m) = l {
                for key in ["quiz", "assignment", "completed", "completed_at"] {
                    m.remove(key);
                }
            }
        }
    }
    let cohorts: Vec<(Uuid, String, chrono::NaiveDate, chrono::NaiveDate, Option<i64>, i64)> = sqlx::query_as(
        "select c.id, c.title, c.start_date, c.end_date, c.capacity, (select count(*) from e_lms_enrollment e where e.tenant_id = c.tenant_id \
         and e.cohort = c.id and e.status <> 'Withdrawn') from e_lms_cohort c where c.tenant_id = $1 and c.course = $2 and c.end_date >= $3 \
         order by c.start_date",
    )
    .bind(ctx.tenant)
    .bind(me)
    .bind(ctx.today())
    .fetch_all(&mut **tx)
    .await?;
    let category = match id(course, "category") {
        Some(cat) => sqlx::query_scalar::<_, String>("select category_name from e_lms_category where tenant_id = $1 and id = $2")
            .bind(ctx.tenant)
            .bind(cat)
            .fetch_optional(&mut **tx)
            .await?
            .map(|name| json!({"id": cat, "name": name})),
        None => None,
    };
    Ok(json!({
        "id": me, "slug": course.get("slug"), "title": course.get("title"), "summary": course.get("summary"), "description": course.get("description"),
        "cover_image": course.get("cover_image"), "level": course.get("level"), "tags": util::tag_list(s(course, "tags").unwrap_or_default()),
        "category": category, "estimated_hours": course.get("estimated_hours"), "lessons": course.get("lesson_count"),
        "total_minutes": course.get("total_minutes"), "certificate": course.get("issue_certificate"), "published_at": course.get("published_at"),
        "self_enrollment": course.get("self_enrollment"),
        "instructors": rows(course, "instructors").iter().filter_map(|r| s(r, "instructor_name").or(s(r, "instructor")).map(str::to_string)).collect::<Vec<_>>(),
        "prerequisites": prerequisites,
        "cohorts": cohorts.iter().map(|(cid, title, start, end, capacity, taken)| {
            let mut c = Doc::new();
            c.insert("capacity".into(), json!(capacity.unwrap_or(0)));
            json!({"id": cid, "title": title, "start_date": start.to_string(), "end_date": end.to_string(), "places": cohort::places(&c, *taken)})
        }).collect::<Vec<_>>(),
        "chapters": outline["chapters"].take(),
    }))
}

/// A lesson's body as shown before enrolling (free previews only).
fn preview_body(lesson: &Doc, file_base: &str) -> Value {
    json!({
        "id": lesson.get("id"), "title": lesson.get("title"), "lesson_type": lesson.get("lesson_type"),
        "duration_minutes": lesson.get("duration_minutes"), "content": lesson.get("content"),
        "video_embed_url": lesson.get("video_embed_url"), "file_url": enrollment::file_link(lesson, file_base),
    })
}

/// A stored lesson file, inline for PDFs and images and as a download otherwise.
async fn send_file(k: &Kernel, ctx: &Ctx, lesson: &Doc) -> Result<SiteResponse> {
    let file = s(lesson, "attachment").and_then(util::file_id).ok_or_else(|| not_found("file"))?;
    let (name, ctype, data) = k.read_file(&sys(ctx), file).await?;
    let inline = ctype == "application/pdf" || (ctype.starts_with("image/") && ctype != "image/svg+xml");
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' }).collect();
    Ok(SiteResponse {
        status: 200,
        content_type: if inline { ctype } else { "application/octet-stream".into() },
        headers: vec![
            ("content-disposition".into(), format!("{}; filename=\"{}\"", if inline { "inline" } else { "attachment" }, ascii.replace('"', ""))),
            ("x-content-type-options".into(), "nosniff".into()),
            ("content-security-policy".into(), "default-src 'none'; sandbox".into()),
            ("cache-control".into(), "private, max-age=300".into()),
        ],
        body: data,
    })
}

// ───────────────────────────── public ─────────────────────────────

async fn public(k: &Kernel, req: &SiteRequest, path: &[&str]) -> Result<SiteResponse> {
    let ctx = Ctx::system(req.tenant, k.registry.schema(req.tenant).await?);
    let mut tx = k.db.tenant_tx(req.tenant).await?;
    let site = format!("/api/site/{}/{}", util::workspace(&mut tx, req.tenant).await?.0, util::PACKAGE);
    let out = match path {
        ["courses"] => {
            let (courses, _, next) = catalog(&mut tx, req.tenant, req, Audience::Public).await?;
            json_response(&json!({"courses": courses, "next_cursor": next}), "public, max-age=60")
        }
        ["categories"] => {
            let cats: Vec<(Uuid, String, i64)> = sqlx::query_as(
                "select cat.id, cat.category_name, count(c.id) from e_lms_category cat join e_lms_course c on c.tenant_id = cat.tenant_id \
                 and c.category = cat.id and c.status = 'Published' and c.visibility = 'Public' where cat.tenant_id = $1 \
                 group by cat.id, cat.category_name order by cat.category_name",
            )
            .bind(req.tenant)
            .fetch_all(&mut *tx)
            .await?;
            let list: Vec<Value> = cats.into_iter().map(|(id, name, n)| json!({"id": id, "name": name, "courses": n})).collect();
            json_response(&json!({"categories": list}), "public, max-age=60")
        }
        ["courses", slug] => {
            let course = course_by_slug(k, &ctx, &mut tx, slug).await?.filter(public_course).ok_or_else(|| not_found("course"))?;
            json_response(&json!({"course": course_page(k, &ctx, &mut tx, &course, Audience::Public).await?}), "public, max-age=60")
        }
        ["courses", slug, "lessons", lesson, rest @ ..] if rest.is_empty() || rest == ["file"] => {
            let course = course_by_slug(k, &ctx, &mut tx, slug).await?.filter(public_course).ok_or_else(|| not_found("lesson"))?;
            let lesson = match load(k, &ctx, &mut tx, "LmsLesson", uuid(lesson)?).await {
                Ok(l) if id(&l, "course") == id(&course, "id") && util::b(&l, "is_preview") => l,
                Ok(_) | Err(Error::NotFound(_)) => return Err(not_found("lesson")),
                Err(e) => return Err(e),
            };
            tx.commit().await?;
            let base = format!("{site}/courses/{slug}/lessons/{}", util::doc_id(&lesson)?);
            return if rest.is_empty() {
                Ok(json_response(&json!({"lesson": preview_body(&lesson, &base)}), "public, max-age=60"))
            } else {
                send_file(k, &ctx, &lesson).await
            };
        }
        ["certificates", code, rest @ ..] if rest.is_empty() || rest == ["view"] => {
            let cert = certificate::by_code(&mut tx, req.tenant, code).await?.ok_or_else(|| not_found("certificate"))?;
            if rest.is_empty() {
                let doc = load(k, &ctx, &mut tx, "LmsCertificate", cert).await?;
                json_response(&json!({"certificate": certificate::public_view(&doc)}), "no-store")
            } else {
                tx.commit().await?;
                let html = k.render_print(&ctx, "LmsCertificate", cert).await?;
                let mut r = SiteResponse::text(200, "text/html; charset=utf-8", html);
                for (h, v) in [
                    (
                        "content-security-policy",
                        "default-src 'none'; style-src 'unsafe-inline'; img-src data: https:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
                    ),
                    ("x-content-type-options", "nosniff"),
                    ("referrer-policy", "no-referrer"),
                    ("cache-control", "no-store"),
                ] {
                    r.headers.push((h.into(), v.into()));
                }
                return Ok(r);
            }
        }
        _ => return Err(not_found("page")),
    };
    tx.commit().await?;
    Ok(out)
}

// ───────────────────────────── signed in ─────────────────────────────

/// The caller's enrolment in a course, if any.
async fn my_enrollment(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: Uuid) -> Result<Option<Doc>> {
    let found: Option<Uuid> = sqlx::query_scalar("select id from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3")
        .bind(ctx.tenant)
        .bind(course)
        .bind(ctx.email.to_lowercase())
        .fetch_optional(&mut **tx)
        .await?;
    match found {
        Some(e) => Ok(Some(load(k, ctx, tx, "LmsEnrollment", e).await?)),
        None => Ok(None),
    }
}

async fn signed_in(k: &Kernel, user: &Ctx, req: &SiteRequest, path: &[&str]) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(user.tenant).await?;
    let out = match path {
        ["catalog"] => {
            let (courses, ids, next) = catalog(&mut tx, user.tenant, req, Audience::Members).await?;
            let mine: Vec<(Uuid, Uuid, String, Option<Decimal>)> = if ids.is_empty() {
                vec![]
            } else {
                sqlx::query_as("select course, id, status, progress_percent from e_lms_enrollment where tenant_id = $1 and learner = $2 and course = any($3)")
                    .bind(user.tenant)
                    .bind(user.email.to_lowercase())
                    .bind(&ids)
                    .fetch_all(&mut *tx)
                    .await?
            };
            let courses: Vec<Value> = courses
                .into_iter()
                .zip(ids.iter())
                .map(|(mut c, cid)| {
                    c["my_enrollment"] =
                        mine.iter().find(|m| m.0 == *cid).map(|m| json!({"id": m.1, "status": m.2, "progress_percent": m.3.map(money)})).unwrap_or(Value::Null);
                    c
                })
                .collect();
            let paths: Vec<(Uuid, String, Option<String>, i64)> = sqlx::query_as(
                "select p.id, p.title, p.summary, (select count(*) from e_lms_learning_path__courses c where c.tenant_id = p.tenant_id and c.parent_id = p.id) \
                 from e_lms_learning_path p where p.tenant_id = $1 and p.status = 'Published' order by p.title limit 200",
            )
            .bind(user.tenant)
            .fetch_all(&mut *tx)
            .await?;
            let paths: Vec<Value> =
                paths.into_iter().map(|(id, title, summary, n)| json!({"id": id, "title": title, "summary": summary, "courses": n})).collect();
            json_response(&json!({"courses": courses, "next_cursor": next, "learning_paths": paths}), "no-store")
        }
        ["courses", slug] => {
            let course = course_by_slug(k, user, &mut tx, slug).await?.ok_or_else(|| not_found("course"))?;
            let cid = util::doc_id(&course)?;
            let mine = my_enrollment(k, user, &mut tx, cid).await?;
            let author = authors(user, &course);
            if s(&course, "status") != Some("Published") && !author && mine.is_none() {
                return Err(not_found("course"));
            }
            let mut page = course_page(k, user, &mut tx, &course, Audience::Members).await?;
            let mut blockers = Vec::new();
            if mine.is_none() {
                if s(&course, "status") != Some("Published") {
                    blockers.push("the course is not published".to_string());
                }
                if !util::b(&course, "self_enrollment") {
                    blockers.push("enrolment is by invitation".to_string());
                }
                let missing = enrollment::missing_prerequisites(user, &mut tx, &course, &user.email).await?;
                if !missing.is_empty() {
                    blockers.push(format!("complete these courses first: {}", missing.join(", ")));
                }
            }
            page["can_enroll"] = json!(mine.is_none() && blockers.is_empty());
            page["enroll_blockers"] = json!(blockers);
            page["is_author"] = json!(author);
            if let Some(e) = &mine {
                let eid = util::doc_id(e)?;
                let outline = enrollment::outline(k, user, &mut tx, &course, Some(eid)).await?;
                page["my_enrollment"] = outline["progress"].clone();
                page["next_lesson"] = outline["next_lesson"].clone();
                page["chapters"] = outline["chapters"].clone();
                let sessions = match id(e, "cohort") {
                    Some(c) if s(e, "status") != Some(WITHDRAWN) => cohort::upcoming(&mut tx, user, &[c], 50).await?,
                    _ => vec![],
                };
                page["upcoming_sessions"] = json!(sessions);
            }
            json_response(&json!({"course": page}), "no-store")
        }
        ["lessons", lesson, rest @ ..] if rest.is_empty() || rest == ["file"] => {
            let lesson = match load(k, user, &mut tx, "LmsLesson", uuid(lesson)?).await {
                Ok(l) => l,
                Err(Error::NotFound(_)) => return Err(not_found("lesson")),
                Err(e) => return Err(e),
            };
            let course = load(k, user, &mut tx, "LmsCourse", id(&lesson, "course").unwrap_or_default()).await?;
            let mine = my_enrollment(k, user, &mut tx, util::doc_id(&course)?).await?.filter(|e| s(e, "status") != Some(WITHDRAWN));
            let author = authors(user, &course);
            let preview = util::b(&lesson, "is_preview") && s(&course, "status") == Some("Published");
            if mine.is_none() && !author && !preview {
                return Err(not_found("lesson"));
            }
            if rest == ["file"] {
                tx.commit().await?;
                return send_file(k, user, &lesson).await;
            }
            let view = enrollment::open_lesson(k, user, &mut tx, mine.as_ref(), &course, util::doc_id(&lesson)?).await?;
            json_response(&view, "no-store")
        }
        ["dashboard"] => json_response(&dashboard(k, user, &mut tx).await?, "no-store"),
        ["teaching"] => json_response(&teaching(user, &mut tx).await?, "no-store"),
        ["teaching", "courses", course, "roster"] => {
            let course = match util::authored_course(k, user, &mut tx, uuid(course)?).await {
                Err(Error::NotFound(_)) => return Err(not_found("course")),
                other => other?,
            };
            let cohort = param(req, "cohort").map(|c| c.parse::<Uuid>().map_err(|_| Error::field("cohort", "must be a cohort id"))).transpose()?;
            json_response(&roster(user, &mut tx, &course, cohort).await?, "no-store")
        }
        _ => return Err(not_found("endpoint")),
    };
    tx.commit().await?;
    Ok(out)
}

#[derive(sqlx::FromRow)]
struct Mine {
    id: Uuid,
    course: Uuid,
    slug: String,
    title: String,
    cover_image: Option<String>,
    status: String,
    progress_percent: Option<Decimal>,
    completed_lessons: Option<i64>,
    total_lessons: Option<i64>,
    enrolled_on: Option<chrono::NaiveDate>,
    completed_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    cohort: Option<Uuid>,
    cohort_title: Option<String>,
    certificate: Option<Uuid>,
    certificate_number: Option<String>,
    verification_code: Option<String>,
}

/// The learner dashboard: my courses (progress, next lesson, certificate), assignments due or
/// overdue, upcoming live sessions of my cohorts, and my learning paths.
async fn dashboard(k: &Kernel, user: &Ctx, tx: &mut Tx) -> Result<Value> {
    let me = user.email.to_lowercase();
    let mine: Vec<Mine> = sqlx::query_as(
        "select e.id, e.course, c.slug, c.title, c.cover_image, e.status, e.progress_percent, e.completed_lessons, e.total_lessons, e.enrolled_on, \
                e.completed_at, e.last_activity_at, e.cohort, co.title as cohort_title, e.certificate, cert.number as certificate_number, \
                cert.verification_code \
         from e_lms_enrollment e join e_lms_course c on c.tenant_id = e.tenant_id and c.id = e.course \
         left join e_lms_cohort co on co.tenant_id = e.tenant_id and co.id = e.cohort \
         left join e_lms_certificate cert on cert.tenant_id = e.tenant_id and cert.id = e.certificate \
         where e.tenant_id = $1 and e.learner = $2 and e.status <> 'Withdrawn' \
         order by (e.status = 'Completed'), e.last_activity_at desc nulls last, e.enrolled_at desc limit 500",
    )
    .bind(user.tenant)
    .bind(&me)
    .fetch_all(&mut **tx)
    .await?;
    let mut courses = Vec::with_capacity(mine.len());
    for m in &mine {
        let course = load(k, user, tx, "LmsCourse", m.course).await?;
        let outline = enrollment::outline(k, user, tx, &course, Some(m.id)).await?;
        courses.push(json!({
            "enrollment": m.id, "course": {"id": m.course, "slug": m.slug, "title": m.title, "cover_image": m.cover_image},
            "status": m.status, "progress_percent": m.progress_percent.map(money), "completed_lessons": m.completed_lessons.unwrap_or(0),
            "total_lessons": m.total_lessons.unwrap_or(0), "enrolled_on": m.enrolled_on.map(|d| d.to_string()),
            "last_activity_at": m.last_activity_at.map(util::ts), "completed_at": m.completed_at.map(util::ts),
            "next_lesson": outline["next_lesson"], "cohort": m.cohort.map(|c| json!({"id": c, "title": m.cohort_title})),
            "certificate": m.certificate.map(|c| json!({"id": c, "number": m.certificate_number, "verification_code": m.verification_code})),
        }));
    }
    // Assignments of my active courses I still owe: no passing submission yet.
    let owed: Vec<(Uuid, String, Uuid, String, Uuid, Option<chrono::NaiveDate>, Option<i64>, Option<chrono::NaiveDate>, Option<String>, Option<bool>)> =
        sqlx::query_as(
            "select a.id, a.title, c.id, c.title, l.id, a.due_date, a.due_days, e.enrolled_on, last.status, last.passed \
         from e_lms_enrollment e join e_lms_course c on c.tenant_id = e.tenant_id and c.id = e.course \
         join e_lms_lesson l on l.tenant_id = e.tenant_id and l.course = e.course and l.assignment is not null \
         join e_lms_assignment a on a.tenant_id = l.tenant_id and a.id = l.assignment \
         left join lateral (select s.status, s.passed from e_lms_assignment_submission s where s.tenant_id = e.tenant_id \
              and s.assignment = a.id and s.learner = e.learner order by s.submission_no desc limit 1) last on true \
         where e.tenant_id = $1 and e.learner = $2 and e.status = 'Active' \
           and not exists (select 1 from e_lms_assignment_submission p where p.tenant_id = e.tenant_id and p.assignment = a.id \
                and p.learner = e.learner and p.passed) \
         order by c.title, l.position limit 500",
        )
        .bind(user.tenant)
        .bind(&me)
        .fetch_all(&mut **tx)
        .await?;
    let today = user.today();
    let mut due: Vec<Value> = owed
        .into_iter()
        .map(|(aid, title, cid, course_title, lesson, fixed, days, enrolled_on, last, passed)| {
            let mut a = Doc::new();
            a.insert("due_date".into(), fixed.map_or(Value::Null, |d| json!(d.to_string())));
            a.insert("due_days".into(), json!(days.unwrap_or(0)));
            let due = enrollment::due_date(&a, enrolled_on);
            let status = match (last.as_deref(), passed) {
                (None, _) => "Not Submitted",
                (Some("Graded"), Some(false)) => "Failed",
                (Some(other), _) => other,
            };
            json!({
                "assignment": aid, "title": title, "course": {"id": cid, "title": course_title}, "lesson": lesson,
                "due_date": due.map(|d| d.to_string()), "status": status,
                "overdue": due.is_some_and(|d| today > d) && status != "Submitted",
            })
        })
        .collect();
    due.sort_by(|a, b| {
        let key =
            |v: &Value| (v["due_date"].as_str().map(str::to_string).unwrap_or_else(|| "9999".into()), v["title"].as_str().unwrap_or_default().to_string());
        key(a).cmp(&key(b))
    });
    let cohorts: Vec<Uuid> = mine.iter().filter_map(|m| m.cohort).collect();
    let sessions = cohort::upcoming(tx, user, &cohorts, 20).await?;
    let certificates: Vec<Value> = mine
        .iter()
        .filter_map(|m| {
            m.certificate.map(|c| json!({"id": c, "number": m.certificate_number, "course_title": m.title, "verification_code": m.verification_code}))
        })
        .collect();
    let paths: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        "select p.id, p.title, count(pc.id), count(e.id) filter (where e.status = 'Completed') \
         from e_lms_learning_path p join e_lms_learning_path__courses pc on pc.tenant_id = p.tenant_id and pc.parent_id = p.id \
         left join e_lms_enrollment e on e.tenant_id = pc.tenant_id and e.course = pc.course and e.learner = $2 \
         where p.tenant_id = $1 and p.status = 'Published' group by p.id, p.title \
         having count(e.id) filter (where e.status <> 'Withdrawn') > 0 order by p.title",
    )
    .bind(user.tenant)
    .bind(&me)
    .fetch_all(&mut **tx)
    .await?;
    let paths: Vec<Value> = paths
        .into_iter()
        .map(|(id, title, total, done)| json!({"id": id, "title": title, "courses": total, "completed": done, "progress_percent": money(enrollment::percent(done, total))}))
        .collect();
    Ok(json!({"courses": courses, "due_assignments": due, "upcoming_sessions": sessions, "certificates": certificates, "learning_paths": paths}))
}

/// The instructor dashboard: the courses I author (all of them for LMS managers, within my
/// record scopes) with enrolment, completion and grading counts.
async fn teaching(user: &Ctx, tx: &mut Tx) -> Result<Value> {
    let ent = user.schema.entity("LmsCourse").ok_or_else(|| not_found("LmsCourse"))?;
    let scope = row_scope(user, ent, Action::Read)?;
    let mut qb = QueryBuilder::<Postgres>::new(
        "select a.id, a.slug, a.title, a.status, count(e.id) filter (where e.status <> 'Withdrawn'), count(e.id) filter (where e.status = 'Active'), \
                count(e.id) filter (where e.status = 'Completed'), avg(e.progress_percent) filter (where e.status <> 'Withdrawn'), \
                (select count(*) from e_lms_assignment_submission s where s.tenant_id = a.tenant_id and s.course = a.id and s.status = 'Submitted') \
         from e_lms_course a left join e_lms_enrollment e on e.tenant_id = a.tenant_id and e.course = a.id where a.tenant_id = ",
    );
    qb.push_bind(user.tenant);
    scope.push_sql(&mut qb, "a");
    if !is_manager(user) {
        qb.push(" and (a.created_by = ")
            .push_bind(user.user)
            .push(" or exists (select 1 from e_lms_course__instructors i where i.tenant_id = a.tenant_id and i.parent_id = a.id and i.instructor = ")
            .push_bind(user.email.to_lowercase())
            .push("))");
    }
    qb.push(" group by a.tenant_id, a.id, a.slug, a.title, a.status order by a.title limit 1000");
    let found: Vec<(Uuid, String, String, String, i64, i64, i64, Option<Decimal>, i64)> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let courses: Vec<Value> = found
        .into_iter()
        .map(|(id, slug, title, status, enrolled, active, completed, avg, pending)| {
            json!({
                "id": id, "slug": slug, "title": title, "status": status, "enrolled": enrolled, "active": active, "completed": completed,
                "completion_rate": money(enrollment::percent(completed, enrolled)),
                "average_progress": money(avg.unwrap_or_default().round_dp(2)), "submissions_to_grade": pending,
            })
        })
        .collect();
    Ok(json!({"courses": courses}))
}

#[derive(sqlx::FromRow)]
struct RosterRow {
    id: Uuid,
    number: Option<String>,
    learner: String,
    learner_name: Option<String>,
    status: String,
    progress_percent: Option<Decimal>,
    completed_lessons: Option<i64>,
    total_lessons: Option<i64>,
    enrolled_on: Option<chrono::NaiveDate>,
    last_activity_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    cohort: Option<Uuid>,
    cohort_title: Option<String>,
    certificate_number: Option<String>,
    quizzes_passed: i64,
    best_quiz_percentage: Option<Decimal>,
    submissions_to_grade: i64,
}

/// A course's learners with their progress, quiz results and work waiting to be graded.
async fn roster(user: &Ctx, tx: &mut Tx, course: &Doc, cohort: Option<Uuid>) -> Result<Value> {
    let cid = util::doc_id(course)?;
    let found: Vec<RosterRow> = sqlx::query_as(
        "select e.id, e.number, e.learner, e.learner_name, e.status, e.progress_percent, e.completed_lessons, e.total_lessons, e.enrolled_on, \
                e.last_activity_at, e.completed_at, e.cohort, co.title as cohort_title, cert.number as certificate_number, \
                (select count(distinct a.quiz) from e_lms_quiz_attempt a where a.tenant_id = e.tenant_id and a.enrollment = e.id and a.passed) as quizzes_passed, \
                (select max(a.percentage) from e_lms_quiz_attempt a where a.tenant_id = e.tenant_id and a.enrollment = e.id) as best_quiz_percentage, \
                (select count(*) from e_lms_assignment_submission s where s.tenant_id = e.tenant_id and s.enrollment = e.id and s.status = 'Submitted') as submissions_to_grade \
         from e_lms_enrollment e left join e_lms_cohort co on co.tenant_id = e.tenant_id and co.id = e.cohort \
         left join e_lms_certificate cert on cert.tenant_id = e.tenant_id and cert.id = e.certificate \
         where e.tenant_id = $1 and e.course = $2 and ($3::uuid is null or e.cohort = $3) \
         order by lower(coalesce(e.learner_name, e.learner)), e.learner limit 5000",
    )
    .bind(user.tenant)
    .bind(cid)
    .bind(cohort)
    .fetch_all(&mut **tx)
    .await?;
    let learners: Vec<Value> = found
        .iter()
        .map(|r| {
            json!({
                "enrollment": r.id, "number": r.number, "learner": r.learner, "learner_name": r.learner_name, "status": r.status,
                "progress_percent": r.progress_percent.map(money), "completed_lessons": r.completed_lessons.unwrap_or(0),
                "total_lessons": r.total_lessons.unwrap_or(0), "enrolled_on": r.enrolled_on.map(|d| d.to_string()),
                "last_activity_at": r.last_activity_at.map(util::ts), "completed_at": r.completed_at.map(util::ts),
                "cohort": r.cohort.map(|c| json!({"id": c, "title": r.cohort_title})), "certificate": r.certificate_number,
                "quizzes_passed": r.quizzes_passed, "best_quiz_percentage": r.best_quiz_percentage.map(money), "submissions_to_grade": r.submissions_to_grade,
            })
        })
        .collect();
    let live = found.iter().filter(|r| r.status != WITHDRAWN).count() as i64;
    let completed = found.iter().filter(|r| r.status == "Completed").count() as i64;
    Ok(json!({
        "course": {"id": cid, "slug": course.get("slug"), "title": course.get("title")},
        "enrolled": live, "completed": completed, "completion_rate": money(enrollment::percent(completed, live)),
        "learners": learners,
    }))
}
