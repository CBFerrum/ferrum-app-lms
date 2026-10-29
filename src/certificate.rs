//! Certificates: issued once per completed enrolment (by `enrollment::refresh`, under the
//! enrolment's row lock, with a unique key on the enrolment as the last word), numbered from a
//! gapless series, printed through the `LmsCertificate` print template and verifiable by anyone
//! with their code on the public site. The code is 160 random bits in Crockford base32. It is
//! printed on the certificate, so it is kept as it is: it identifies a certificate the learner
//! chooses to share and unlocks nothing else.

use crate::notice::{self, Notice};
use crate::util::{self, doc_id, elevated, is_manager, load, s};
use async_trait::async_trait;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Map, Value, json};
use uuid::Uuid;

pub const VALID: &str = "Valid";
pub const REVOKED: &str = "Revoked";
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 32 symbols in groups of four: `7K2M-...`.
fn group(symbols: &str) -> String {
    symbols.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

fn encode(bytes: &[u8; 20]) -> String {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, String::with_capacity(32));
    for b in bytes {
        acc = ((acc << 8) | u32::from(*b)) & 0xFFFF;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    group(&out)
}

/// A fresh verification code (160 random bits).
pub fn new_code() -> String {
    encode(&rand::random::<[u8; 20]>())
}

/// A code as typed by a person — any case, with or without dashes or spaces, O for 0 and I/L
/// for 1 — in its stored form, or `None` when it cannot be one.
pub fn normalize_code(raw: &str) -> Option<String> {
    if raw.len() > 80 {
        return None;
    }
    let mut out = String::with_capacity(32);
    for c in raw.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == 32).then(|| group(&out))
}

pub struct CertificateHook;

#[async_trait]
impl DocHook for CertificateHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid("certificates are issued by Learn when a learner completes a course that gives one"));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "revoke" {
            return Ok(None);
        }
        if !is_manager(hc.ctx) {
            return Err(Error::Forbidden("only LMS managers revoke certificates".into()));
        }
        let reason = params.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty());
        let reason = reason.ok_or_else(|| Error::field("reason", "say why the certificate is revoked"))?;
        if reason.chars().count() > 1000 {
            return Err(Error::field("reason", "at most 1000 characters"));
        }
        let me = doc_id(doc)?;
        let mut patch = Map::new();
        patch.insert("status".into(), json!(REVOKED));
        patch.insert("revoked_reason".into(), json!(reason));
        let done = hc.kernel.update_tx(&elevated(hc.ctx), hc.tx, "LmsCertificate", me, patch, None).await?;
        let n = Notice {
            title: format!("Certificate revoked: {}", s(doc, "course_title").unwrap_or_default()),
            body: format!("Your certificate {} was revoked: {reason}", s(doc, "number").unwrap_or_default()),
            entity: "LmsCertificate",
            record: me,
        };
        notice::send(hc.kernel, hc.ctx, hc.tx, &[s(doc, "learner").unwrap_or_default().to_string()], &n).await?;
        Ok(Some(json!({"certificate": done})))
    }
}

/// The certificate of a completed enrolment, issued now unless it already exists.
pub async fn issue(k: &Kernel, ctx: &Ctx, tx: &mut Tx, enrollment: &Doc, course: &Doc) -> Result<Doc> {
    let eid = doc_id(enrollment)?;
    let existing: Option<Uuid> = sqlx::query_scalar("select id from e_lms_certificate where tenant_id = $1 and enrollment = $2")
        .bind(ctx.tenant)
        .bind(eid)
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(c) = existing {
        return load(k, ctx, tx, "LmsCertificate", c).await;
    }
    let settings = util::settings(tx, ctx.tenant).await?;
    let (slug, workspace) = util::workspace(tx, ctx.tenant).await?;
    let code = new_code();
    let url = settings.public_url.as_ref().map(|base| format!("{base}/api/site/{slug}/{}/certificates/{code}", util::PACKAGE));
    let learner = s(enrollment, "learner").unwrap_or_default();
    let mut doc = Map::new();
    for (k_, v) in [
        ("enrollment", json!(eid.to_string())),
        ("course", course.get("id").cloned().unwrap_or(Value::Null)),
        ("course_title", course.get("title").cloned().unwrap_or(Value::Null)),
        ("learner", json!(learner)),
        ("learner_name", json!(s(enrollment, "learner_name").unwrap_or(learner))),
        ("issued_on", json!(ctx.today().to_string())),
        ("issued_at", json!(util::ts(util::now()))),
        ("verification_code", json!(code)),
        ("verification_url", url.map_or(Value::Null, Value::String)),
        ("issuer", json!(settings.issuer.unwrap_or(workspace))),
        ("signatory", settings.signatory.map_or(Value::Null, Value::String)),
        ("signatory_title", settings.signatory_title.map_or(Value::Null, Value::String)),
        ("status", json!(VALID)),
    ] {
        doc.insert(k_.into(), v);
    }
    k.insert_tx(&elevated(ctx), tx, "LmsCertificate", doc).await
}

/// The certificate a (typed) code names, if any.
pub async fn by_code(tx: &mut Tx, tenant: Uuid, raw: &str) -> Result<Option<Uuid>> {
    let Some(code) = normalize_code(raw) else { return Ok(None) };
    Ok(sqlx::query_scalar("select id from e_lms_certificate where tenant_id = $1 and verification_code = $2")
        .bind(tenant)
        .bind(code)
        .fetch_optional(&mut **tx)
        .await?)
}

/// What the public may know about a certificate: that it exists, whom it names, for what, when.
pub fn public_view(c: &Doc) -> Value {
    json!({
        "valid": s(c, "status") == Some(VALID), "status": c.get("status"), "number": c.get("number"),
        "learner_name": c.get("learner_name"), "course_title": c.get("course_title"), "issued_on": c.get("issued_on"),
        "issuer": c.get("issuer"), "verification_code": c.get("verification_code"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_long_random_and_forgiving() {
        let a = new_code();
        assert_eq!(a.len(), 39, "{a}");
        assert_eq!(a.matches('-').count(), 7);
        assert_ne!(a, new_code());
        assert_eq!(normalize_code(&a), Some(a.clone()));
        assert_eq!(normalize_code(&a.to_lowercase().replace('-', " ")), Some(a.clone()));
        assert_eq!(encode(&[0u8; 20]), "0000-0000-0000-0000-0000-0000-0000-0000");
        assert_eq!(encode(&[255u8; 20]), "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ");
        assert_eq!(normalize_code("oooo-iiii-llll-0000-0000-0000-0000-0000"), Some("0000-1111-1111-0000-0000-0000-0000-0000".into()));
        assert_eq!(normalize_code("ABCD"), None);
        assert_eq!(normalize_code("UUUU-0000-0000-0000-0000-0000-0000-0000"), None, "U is not a Crockford symbol");
        assert_eq!(normalize_code(&"0".repeat(200)), None);
    }
}
