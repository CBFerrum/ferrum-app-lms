//! Helpers shared by the package: roles and course authorship, Learn Settings, users and the
//! workspace, time, text and request plumbing.

use chrono::{DateTime, SecondsFormat, Utc};
pub use ferrum_domain::util::{b, d, elevated, id, rows, s, sys};
use ferrum_kernel::value::parse_datetime;
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx};
use serde_json::Value;
use uuid::Uuid;

/// The package's name (schema membership, site prefix).
pub const PACKAGE: &str = "lms";
pub const MANAGER: &str = "LMS Manager";
pub const INSTRUCTOR: &str = "Instructor";
pub const LEARNER: &str = "Learner";

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// RFC 3339 UTC with milliseconds, the kernel's canonical date-time form.
pub fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn time(doc: &Doc, k: &str) -> Option<DateTime<Utc>> {
    s(doc, k).and_then(|v| parse_datetime(v).ok())
}

/// A whole-number field (missing or empty reads as 0).
pub fn int(doc: &Doc, k: &str) -> i64 {
    doc.get(k).and_then(Value::as_i64).unwrap_or(0)
}

pub fn doc_id(doc: &Doc) -> Result<Uuid> {
    id(doc, "id").ok_or_else(|| Error::internal("record without id"))
}

/// A document being created (hooks see stored documents with `created_at`).
pub fn is_new(doc: &Doc) -> bool {
    !doc.contains_key("created_at")
}

pub fn same_email(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim())
}

/// The engine, administrators and LMS managers: everything in Learn.
pub fn is_manager(ctx: &Ctx) -> bool {
    ctx.is_system || ctx.is_admin || ctx.roles.iter().any(|r| r == MANAGER)
}

/// Whether the caller may use Learn's signed-in endpoints.
pub fn is_member(ctx: &Ctx) -> bool {
    is_manager(ctx) || ctx.roles.iter().any(|r| r == INSTRUCTOR || r == LEARNER)
}

/// The course's instructors (lower-case emails).
pub fn instructors(course: &Doc) -> Vec<String> {
    rows(course, "instructors").iter().filter_map(|r| s(r, "instructor")).map(str::to_lowercase).collect()
}

/// Whether the caller authors `course`: LMS managers, its creator and its instructors.
pub fn authors(ctx: &Ctx, course: &Doc) -> bool {
    is_manager(ctx) || (!ctx.user.is_nil() && id(course, "created_by") == Some(ctx.user)) || instructors(course).iter().any(|e| same_email(e, &ctx.email))
}

/// A record read with engine privileges (the callers check what the user may see).
pub async fn load(k: &Kernel, ctx: &Ctx, tx: &mut Tx, entity: &str, record: Uuid) -> Result<Doc> {
    k.get_tx(&sys(ctx), tx, entity, record, false).await
}

/// Like [`load`], holding the row lock until the transaction ends.
pub async fn load_locked(k: &Kernel, ctx: &Ctx, tx: &mut Tx, entity: &str, record: Uuid) -> Result<Doc> {
    k.get_tx(&sys(ctx), tx, entity, record, true).await
}

/// The course, which the caller must author.
pub async fn authored_course(k: &Kernel, ctx: &Ctx, tx: &mut Tx, course: Uuid) -> Result<Doc> {
    let doc = load(k, ctx, tx, "LmsCourse", course).await?;
    if !authors(ctx, &doc) {
        return Err(Error::Forbidden(format!("only the instructors of \"{}\" and LMS managers can do this", s(&doc, "title").unwrap_or("this course"))));
    }
    Ok(doc)
}

// ───────────────────────────── settings, workspace, users ─────────────────────────────

/// Learn Settings with their defaults.
#[derive(Debug, Clone)]
pub struct Settings {
    pub public_url: Option<String>,
    pub email: bool,
    pub reminder_minutes: i64,
    pub issuer: Option<String>,
    pub signatory: Option<String>,
    pub signatory_title: Option<String>,
    pub video_hosts: Vec<String>,
}

type SettingsRow = (Option<String>, Option<bool>, Option<i64>, Option<String>, Option<String>, Option<String>, Option<String>);

pub async fn settings(tx: &mut Tx, tenant: Uuid) -> Result<Settings> {
    let row: Option<SettingsRow> = sqlx::query_as(
        "select public_url, email_notifications, session_reminder_minutes, certificate_issuer, certificate_signatory, \
         certificate_signatory_title, video_hosts from e_lms_settings where tenant_id = $1 limit 1",
    )
    .bind(tenant)
    .fetch_optional(&mut **tx)
    .await?;
    let (url, email, minutes, issuer, signatory, title, hosts) = row.unwrap_or_default();
    let text = |v: Option<String>| v.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    Ok(Settings {
        public_url: text(url).map(|u| u.trim_end_matches('/').to_string()),
        email: email.unwrap_or(true),
        reminder_minutes: minutes.unwrap_or(60).clamp(5, 10_080),
        issuer: text(issuer),
        signatory: text(signatory),
        signatory_title: text(title),
        video_hosts: crate::video::parse_hosts(hosts.as_deref().unwrap_or_default()).unwrap_or_default(),
    })
}

/// The workspace's slug (public site paths) and name.
pub async fn workspace(tx: &mut Tx, tenant: Uuid) -> Result<(String, String)> {
    Ok(sqlx::query_as("select slug, name from tenant where id = $1").bind(tenant).fetch_one(&mut **tx).await?)
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
}

impl User {
    pub fn name(&self) -> String {
        if self.full_name.trim().is_empty() { self.email.clone() } else { self.full_name.clone() }
    }
}

/// Active users with these emails.
pub async fn users(tx: &mut Tx, tenant: Uuid, emails: &[String]) -> Result<Vec<User>> {
    if emails.is_empty() {
        return Ok(vec![]);
    }
    let lower: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).collect();
    let rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("select id, lower(email::text), full_name from app_user where tenant_id = $1 and is_active and lower(email::text) = any($2)")
            .bind(tenant)
            .bind(&lower)
            .fetch_all(&mut **tx)
            .await?;
    Ok(rows.into_iter().map(|(id, email, full_name)| User { id, email, full_name }).collect())
}

pub async fn user(tx: &mut Tx, tenant: Uuid, email: &str) -> Result<Option<User>> {
    Ok(users(tx, tenant, &[email.to_string()]).await?.into_iter().next())
}

/// Fills `name_field` of each row from the active user named in `email_field`; unknown or
/// repeated addresses are validation errors on that row.
pub async fn people_rows(tx: &mut Tx, tenant: Uuid, doc: &mut Doc, table: &str, email_field: &str, name_field: &str) -> Result<()> {
    let mut list = rows(doc, table);
    if list.is_empty() {
        return Ok(());
    }
    let emails: Vec<String> = list.iter().filter_map(|r| s(r, email_field)).map(str::to_lowercase).collect();
    let found = users(tx, tenant, &emails).await?;
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, r) in list.iter_mut().enumerate() {
        let email = s(r, email_field).unwrap_or_default().to_lowercase();
        if !seen.insert(email.clone()) {
            errors.push(ferrum_kernel::FieldError::new(email_field, "is listed twice").in_row(table, i));
            continue;
        }
        match found.iter().find(|u| u.email == email) {
            Some(u) => {
                r.insert(name_field.into(), Value::String(u.name()));
            }
            None => errors.push(ferrum_kernel::FieldError::new(email_field, "no active user signs in with this email").in_row(table, i)),
        }
    }
    if !errors.is_empty() {
        return Err(Error::Validation(errors));
    }
    doc.insert(table.into(), Value::Array(list.into_iter().map(Value::Object).collect()));
    Ok(())
}

// ───────────────────────────── text ─────────────────────────────

pub fn html_escape(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Cuts text to `max` characters.
pub fn cut(v: &str, max: usize) -> String {
    if v.chars().count() <= max { v.to_string() } else { v.chars().take(max).collect() }
}

/// `"Intro to Rust!"` → `"intro-to-rust"`: lower-case ASCII letters and digits joined by single
/// hyphens, at most 80 characters.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 80 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 100 && slug.split('-').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// `" Rust,  SAFETY ,rust"` → `"rust, safety"`: trimmed, lower-case, unique, in order.
pub fn normalize_tags(raw: &str) -> String {
    tag_list(raw).join(", ")
}

pub fn tag_list(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split(',') {
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(cut(&t, 40));
        }
    }
    out
}

/// A `/files/{id}` or `/files/public/{id}` path of a stored file.
pub fn file_id(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/files/")?;
    rest.strip_prefix("public/").unwrap_or(rest).parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_tags_and_files() {
        assert_eq!(slugify("Intro to Rust! (2026 edition)"), "intro-to-rust-2026-edition");
        assert_eq!(slugify("  --Ünïcode--  "), "n-code");
        assert_eq!(slugify("日本語"), "");
        assert!(slugify(&"word ".repeat(40)).len() <= 80);
        assert!(valid_slug("intro-to-rust-2"));
        for bad in ["", "Intro", "a--b", "-a", "a-", "a_b", "a b"] {
            assert!(!valid_slug(bad), "{bad}");
        }
        assert_eq!(normalize_tags(" Rust,  SAFETY ,rust,, Fire  Drill "), "rust, safety, fire drill");
        let f = Uuid::now_v7();
        assert_eq!(file_id(&format!("/files/{f}")), Some(f));
        assert_eq!(file_id(&format!("/files/public/{f}")), Some(f));
        assert_eq!(file_id("/files/../etc"), None);
        assert_eq!(html_escape("<b a=\"x\">'&"), "&lt;b a=&quot;x&quot;&gt;&#39;&amp;");
        assert!(same_email(" A@x.test", "a@X.TEST") && !same_email("", ""));
    }
}
