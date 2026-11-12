//! LTI 1.3 tools as lessons: Learn is the platform.
//!
//! An LMS manager registers a tool (`LmsLtiTool`): its login and launch addresses, and its public
//! key (or the address of its keys) for when it sends grades back. Learn gives the tool a client id
//! and a deployment id, and shows the addresses the tool needs: the issuer (Learn Settings' public
//! URL), the authorization endpoint, Learn's keys and its token endpoint.
//!
//! An LTI lesson opens the tool in a new window (`/api/app/lms/lti/{lesson}/launch`): Learn starts
//! the OIDC login at the tool, the tool asks Learn's authorization endpoint for the launch, and Learn
//! answers the signed-in learner with an `id_token` (RS256, with Learn's key) posted to the tool.
//! The token names the learner by an opaque id (and by name and email only where the tool may know
//! them), the course, the lesson and their role.
//!
//! Grades come back by LTI Assignment and Grade Services: the tool signs a client assertion with
//! its key and gets an access token for an hour (an API key of the tool's own service user, which
//! can do nothing else), then posts scores to the lesson's line item. A score whose activity is
//! Completed completes the lesson for its learner. A tool that sends no grades leaves its lessons
//! for the learner to mark done.

use crate::enrollment::{self, WITHDRAWN};
use crate::util::{self, authors, doc_id, elevated, id, load, s, sys};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, SiteRequest, SiteResponse, Tx};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const LTI: &str = "https://purl.imsglobal.org/spec/lti/claim/";
const SCORE_SCOPE: &str = "https://purl.imsglobal.org/spec/lti-ags/scope/score";
const LINEITEM_SCOPE: &str = "https://purl.imsglobal.org/spec/lti-ags/scope/lineitem.readonly";
const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
/// How long an id_token, and a client assertion, may live.
const TOKEN_SECONDS: i64 = 300;
/// How long a tool's access token lasts.
const ACCESS_SECONDS: i64 = 3600;

/// The addresses a tool is registered with, from Learn Settings' public URL (`None` without one).
pub struct Platform {
    pub issuer: String,
    pub auth_url: String,
    pub jwks_url: String,
    pub token_url: String,
}

pub async fn platform(tx: &mut Tx, tenant: Uuid) -> Result<Option<Platform>> {
    let Some(base) = util::settings(tx, tenant).await?.public_url else { return Ok(None) };
    let (slug, _) = util::workspace(tx, tenant).await?;
    let site = format!("{base}/api/site/{slug}/{}/lti", util::PACKAGE);
    Ok(Some(Platform {
        issuer: base.clone(),
        auth_url: format!("{base}/api/app/{}/lti/auth", util::PACKAGE),
        jwks_url: format!("{site}/jwks"),
        token_url: format!("{site}/token"),
    }))
}

/// Learn's signing key and its id: the settings' secret field, made on first use.
async fn signing_key(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<(RsaPrivateKey, String)> {
    let settings: Option<Uuid> =
        sqlx::query_scalar("select id from e_lms_settings where tenant_id = $1 limit 1 for update").bind(ctx.tenant).fetch_optional(&mut **tx).await?;
    if let Some(sid) = settings
        && let Some(pem) = k.secret(tx, ctx.tenant, "LmsSettings", sid, "lti_private_key").await?
    {
        let key = RsaPrivateKey::from_pkcs8_pem(&pem).map_err(|_| Error::internal("Learn's LTI key cannot be read"))?;
        let kid = key_id(&key.to_public_key());
        return Ok((key, kid));
    }
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).map_err(|_| Error::internal("an LTI key could not be made"))?;
    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|_| Error::internal("the LTI key cannot be written"))?.to_string();
    let mut v = Doc::new();
    v.insert("lti_private_key".into(), json!(pem));
    let engine = sys(ctx);
    match settings {
        Some(sid) => k.update_tx(&engine, tx, "LmsSettings", sid, v, None).await?,
        None => k.insert_tx(&engine, tx, "LmsSettings", v).await?,
    };
    let kid = key_id(&key.to_public_key());
    Ok((key, kid))
}

/// A key's id: the start of the SHA-256 of its public key.
fn key_id(key: &RsaPublicKey) -> String {
    let der = key.to_public_key_der().map(|d| d.as_bytes().to_vec()).unwrap_or_default();
    hex::encode(&Sha256::digest(der)[..8])
}

fn jwk(key: &RsaPublicKey, kid: &str) -> Value {
    json!({"kty": "RSA", "alg": "RS256", "use": "sig", "kid": kid, "n": URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()), "e": URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())})
}

/// Learn's public keys, as tools fetch them.
pub async fn jwks(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<Value> {
    let (key, kid) = signing_key(k, ctx, tx).await?;
    Ok(json!({"keys": [jwk(&key.to_public_key(), &kid)]}))
}

/// On tools: their client and deployment ids, made once, and the platform's addresses to copy.
pub struct ToolHook;

#[async_trait::async_trait]
impl DocHook for ToolHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        for f in ["client_id", "deployment_id"] {
            if s(doc, f).is_none() {
                doc.insert(f.into(), json!(Uuid::now_v7().to_string()));
            }
        }
        let p = platform(hc.tx, hc.ctx.tenant).await?;
        for (f, v) in [
            ("platform_issuer", p.as_ref().map(|p| p.issuer.clone())),
            ("platform_auth_url", p.as_ref().map(|p| p.auth_url.clone())),
            ("platform_jwks_url", p.as_ref().map(|p| p.jwks_url.clone())),
            ("platform_token_url", p.as_ref().map(|p| p.token_url.clone())),
        ] {
            doc.insert(f.into(), v.map_or(Value::Null, Value::String));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        for f in ["login_url", "launch_url"] {
            if let Some(u) = s(doc, f)
                && !u.starts_with("https://")
                && !u.starts_with("http://localhost")
            {
                return Err(Error::field(f, "must be an https address"));
            }
        }
        if let Some(pem) = s(doc, "public_key")
            && RsaPublicKey::from_public_key_pem(pem).is_err()
        {
            return Err(Error::field("public_key", "is not an RSA public key in PEM (-----BEGIN PUBLIC KEY-----)"));
        }
        if util::b(doc, "sends_grades") && s(doc, "public_key").is_none() && s(doc, "jwks_url").is_none() {
            return Err(Error::field("public_key", "a tool that sends grades needs its public key or the address of its keys"));
        }
        if platform(hc.tx, hc.ctx.tenant).await?.is_none() {
            return Err(Error::field("tool_name", "set the Public URL in Learn Settings first: it is Learn's issuer to tools"));
        }
        Ok(())
    }
}

fn param<'a>(req: &'a SiteRequest, form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    req.query.get(key).and_then(Value::as_str).or_else(|| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()))
}

fn form(req: &SiteRequest) -> Vec<(String, String)> {
    url::form_urlencoded::parse(&req.body).into_owned().collect()
}

fn redirect(to: &str) -> SiteResponse {
    SiteResponse {
        status: 302,
        content_type: "text/plain".into(),
        headers: vec![("location".into(), to.into()), ("cache-control".into(), "no-store".into())],
        body: vec![],
    }
}

/// The lesson, its tool and course, for someone who may open it (an enrolled learner, or an author).
struct Launch {
    lesson: Doc,
    tool: Doc,
    course: Doc,
    enrollment: Option<Doc>,
    author: bool,
}

async fn launch_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lesson: Uuid) -> Result<Launch> {
    let gone = || Error::NotFound("LTI lesson".into());
    let l = match load(k, ctx, tx, "LmsLesson", lesson).await {
        Ok(l) if s(&l, "lesson_type") == Some("LTI") => l,
        Ok(_) | Err(Error::NotFound(_)) => return Err(gone()),
        Err(e) => return Err(e),
    };
    let tool = load(k, ctx, tx, "LmsLtiTool", id(&l, "lti_tool").ok_or_else(gone)?).await?;
    let course = load(k, ctx, tx, "LmsCourse", id(&l, "course").unwrap_or_default()).await?;
    let mine: Option<Uuid> =
        sqlx::query_scalar("select id from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 and status <> 'Withdrawn' order by coalesce(cycle, 1) desc limit 1")
            .bind(ctx.tenant)
            .bind(doc_id(&course)?)
            .bind(ctx.email.to_lowercase())
            .fetch_optional(&mut **tx)
            .await?;
    let enrollment = match mine {
        Some(e) => Some(load(k, ctx, tx, "LmsEnrollment", e).await?),
        None => None,
    };
    let author = authors(ctx, &course);
    if enrollment.is_none() && !author {
        return Err(gone());
    }
    Ok(Launch { lesson: l, tool, course, enrollment, author })
}

/// Where the lesson's launch goes: the lesson's own address at the tool, or the tool's.
fn target(l: &Launch) -> String {
    s(&l.lesson, "lti_target_url").or(s(&l.tool, "launch_url")).unwrap_or_default().to_string()
}

/// `GET lti/{lesson}/launch`: starts the OIDC login at the tool (third-party initiated login).
pub async fn start(k: &Kernel, ctx: &Ctx, lesson: Uuid) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let l = launch_of(k, ctx, &mut tx, lesson).await?;
    let p = platform(&mut tx, ctx.tenant).await?.ok_or_else(|| Error::invalid("set the Public URL in Learn Settings to launch tools"))?;
    tx.commit().await?;
    let login = s(&l.tool, "login_url").ok_or_else(|| Error::invalid("this tool has no login address"))?;
    let mut url = url::Url::parse(login).map_err(|_| Error::invalid("this tool's login address is not a URL"))?;
    url.query_pairs_mut()
        .append_pair("iss", &p.issuer)
        .append_pair("login_hint", &ctx.user.to_string())
        .append_pair("target_link_uri", &target(&l))
        .append_pair("lti_message_hint", &lesson.to_string())
        .append_pair("client_id", s(&l.tool, "client_id").unwrap_or_default())
        .append_pair("lti_deployment_id", s(&l.tool, "deployment_id").unwrap_or_default());
    Ok(redirect(url.as_str()))
}

/// The page that posts the launch to the tool. Its one script submits the form; the policy lets
/// nothing else run and the form go only to the tool.
fn form_post(action: &str, fields: &[(&str, &str)]) -> SiteResponse {
    const SUBMIT: &str = "document.forms[0].submit();";
    let esc = util::html_escape;
    let inputs: String = fields.iter().map(|(n, v)| format!("<input type=\"hidden\" name=\"{}\" value=\"{}\">", esc(n), esc(v))).collect();
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Opening the tool</title></head><body>\
         <form method=\"post\" action=\"{}\">{inputs}<noscript><button type=\"submit\">Continue to the tool</button></noscript></form>\
         <script>{SUBMIT}</script></body></html>",
        esc(action)
    );
    let origin = url::Url::parse(action).map(|u| u.origin().ascii_serialization()).unwrap_or_else(|_| "'none'".into());
    let hash = STANDARD.encode(Sha256::digest(SUBMIT.as_bytes()));
    let mut r = SiteResponse::text(200, "text/html; charset=utf-8", html);
    r.headers.push((
        "content-security-policy".into(),
        format!("default-src 'none'; script-src 'sha256-{hash}'; form-action {origin}; base-uri 'none'; frame-ancestors 'self'"),
    ));
    r.headers.push(("cache-control".into(), "no-store".into()));
    r.headers.push(("referrer-policy".into(), "no-referrer".into()));
    r
}

/// `lti/auth`: the OIDC authorization request of the tool, answered for the signed-in person
/// with a signed id_token posted to the tool's redirect address.
pub async fn authorize(k: &Kernel, ctx: &Ctx, req: &SiteRequest) -> Result<SiteResponse> {
    let body = form(req);
    let get = |key: &str| param(req, &body, key);
    let invalid = |m: &str| Error::field("authorization", m.to_string());
    if get("response_type") != Some("id_token") || !get("scope").is_some_and(|s| s.split(' ').any(|x| x == "openid")) {
        return Err(invalid("an LTI launch asks for an id_token with the openid scope"));
    }
    if get("response_mode").is_some_and(|m| m != "form_post") {
        return Err(invalid("an LTI launch is answered with form_post"));
    }
    let nonce = get("nonce").filter(|n| !n.is_empty()).ok_or_else(|| invalid("a nonce is required"))?;
    let lesson: Uuid = get("lti_message_hint").and_then(|v| v.parse().ok()).ok_or_else(|| invalid("which lesson is unknown"))?;
    if get("login_hint") != Some(ctx.user.to_string().as_str()) {
        return Err(Error::Forbidden("this launch was started for someone else: open it again from the lesson".into()));
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let l = launch_of(k, ctx, &mut tx, lesson).await?;
    if get("client_id") != s(&l.tool, "client_id") {
        return Err(invalid("the client id is not this lesson's tool"));
    }
    let redirect_uri = get("redirect_uri").ok_or_else(|| invalid("a redirect_uri is required"))?;
    let allowed: Vec<String> = [s(&l.tool, "launch_url"), s(&l.lesson, "lti_target_url")]
        .into_iter()
        .flatten()
        .map(str::to_string)
        .chain(s(&l.tool, "redirect_uris").unwrap_or_default().lines().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()))
        .collect();
    if !allowed.iter().any(|u| u == redirect_uri) {
        return Err(invalid("the redirect_uri is not one of the tool's addresses"));
    }
    let p = platform(&mut tx, ctx.tenant).await?.ok_or_else(|| Error::invalid("set the Public URL in Learn Settings to launch tools"))?;
    let (key, kid) = signing_key(k, ctx, &mut tx).await?;
    let claims = id_token_claims(&p, &l, ctx, nonce, &target(&l), &workspace_name(&mut tx, ctx.tenant).await?)?;
    tx.commit().await?;
    let der = key.to_pkcs1_der().map_err(|_| Error::internal("the LTI key cannot be used"))?;
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid);
    let token = jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_der(der.as_bytes()))
        .map_err(|e| Error::internal(format!("the launch could not be signed: {e}")))?;
    let state = get("state").unwrap_or_default();
    Ok(form_post(redirect_uri, &[("id_token", &token), ("state", state)]))
}

async fn workspace_name(tx: &mut Tx, tenant: Uuid) -> Result<String> {
    Ok(util::workspace(tx, tenant).await?.1)
}

/// The claims of a resource link launch (LTI 1.3 core, with AGS where the tool sends grades).
fn id_token_claims(p: &Platform, l: &Launch, ctx: &Ctx, nonce: &str, target: &str, workspace: &str) -> Result<Value> {
    let now = Utc::now().timestamp();
    let client = s(&l.tool, "client_id").unwrap_or_default();
    let lesson = doc_id(&l.lesson)?;
    let learner = l.enrollment.is_some() && !l.author;
    let role =
        if learner { "http://purl.imsglobal.org/vocab/lis/v2/membership#Learner" } else { "http://purl.imsglobal.org/vocab/lis/v2/membership#Instructor" };
    let custom: serde_json::Map<String, Value> = [s(&l.tool, "custom_parameters"), s(&l.lesson, "lti_custom")]
        .into_iter()
        .flatten()
        .flat_map(str::lines)
        .filter_map(|line| line.split_once('=').map(|(k, v)| (k.trim().to_string(), json!(v.trim()))))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    let mut c = json!({
        "iss": p.issuer, "aud": client, "azp": client, "sub": ctx.user.to_string(), "iat": now, "exp": now + TOKEN_SECONDS, "nonce": nonce,
        (format!("{LTI}message_type")): "LtiResourceLinkRequest",
        (format!("{LTI}version")): "1.3.0",
        (format!("{LTI}deployment_id")): s(&l.tool, "deployment_id"),
        (format!("{LTI}target_link_uri")): target,
        (format!("{LTI}resource_link")): {"id": lesson.to_string(), "title": s(&l.lesson, "title")},
        (format!("{LTI}roles")): [role],
        (format!("{LTI}context")): {"id": s(&l.course, "id"), "label": s(&l.course, "slug"), "title": s(&l.course, "title"),
            "type": ["http://purl.imsglobal.org/vocab/lis/v2/course#CourseOffering"]},
        (format!("{LTI}tool_platform")): {"guid": ctx.tenant.to_string(), "name": workspace, "product_family_code": "ferrum-learn"},
        (format!("{LTI}launch_presentation")): {"document_target": "window"},
        (format!("{LTI}custom")): custom,
    });
    if util::b(&l.tool, "share_identity") {
        c["email"] = json!(ctx.email.to_lowercase());
        c["name"] = json!(l.enrollment.as_ref().and_then(|e| s(e, "learner_name")).unwrap_or(&ctx.email));
    }
    if learner && util::b(&l.tool, "sends_grades") {
        c["https://purl.imsglobal.org/spec/lti-ags/claim/endpoint"] =
            json!({"scope": [SCORE_SCOPE, LINEITEM_SCOPE], "lineitem": lineitem_url(&p.issuer, lesson)});
    }
    Ok(c)
}

fn lineitem_url(issuer: &str, lesson: Uuid) -> String {
    format!("{issuer}/api/app/{}/lti/ags/{lesson}", util::PACKAGE)
}

/// `POST lti/token` (public): the client credentials grant with a signed client assertion
/// (private_key_jwt). The tool gets an access token for an hour: an API key of its service user.
pub async fn token(k: &Kernel, req: &SiteRequest) -> Result<SiteResponse> {
    let body = form(req);
    let get = |key: &str| body.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let fail = |error: &str, m: &str| SiteResponse::json(400, &json!({"error": error, "error_description": m}));
    if get("grant_type") != Some("client_credentials") {
        return Ok(fail("unsupported_grant_type", "use client_credentials"));
    }
    if get("client_assertion_type") != Some(ASSERTION_TYPE) {
        return Ok(fail("invalid_client", "authenticate with a JWT client assertion"));
    }
    let Some(assertion) = get("client_assertion") else { return Ok(fail("invalid_client", "no client assertion")) };
    let unverified = jsonwebtoken::dangerous::insecure_decode::<Value>(assertion).map_err(|_| Error::field("client_assertion", "is not a JWT"))?;
    let client = unverified.claims.get("iss").and_then(Value::as_str).unwrap_or_default().to_string();
    let ctx = Ctx::system(req.tenant, k.registry.schema(req.tenant).await?);
    let mut tx = k.db.tenant_tx(req.tenant).await?;
    let tool: Option<Uuid> = sqlx::query_scalar("select id from e_lms_lti_tool where tenant_id = $1 and client_id = $2 and sends_grades")
        .bind(req.tenant)
        .bind(&client)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(tool) = tool else { return Ok(fail("invalid_client", "no tool here sends grades with this client id")) };
    let tool = load(k, &ctx, &mut tx, "LmsLtiTool", tool).await?;
    let p = platform(&mut tx, req.tenant).await?.ok_or_else(|| Error::invalid("Learn has no public URL"))?;
    let key = match tool_key(k, &tool, unverified.header.kid.as_deref()).await {
        Ok(key) => key,
        Err(e) => return Ok(fail("invalid_client", &e.to_string())),
    };
    let mut v = Validation::new(Algorithm::RS256);
    v.set_audience(&[p.token_url.as_str(), p.issuer.as_str()]);
    v.set_issuer(&[client.as_str()]);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = match jsonwebtoken::decode::<Value>(assertion, &key, &v) {
        Ok(d) => d.claims,
        Err(e) => return Ok(fail("invalid_client", &format!("the client assertion is not valid: {e}"))),
    };
    if claims.get("sub").and_then(Value::as_str) != Some(client.as_str()) {
        return Ok(fail("invalid_client", "the assertion's subject is not the client"));
    }
    let exp = claims.get("exp").and_then(Value::as_i64).unwrap_or_default();
    if exp > Utc::now().timestamp() + TOKEN_SECONDS {
        return Ok(fail("invalid_client", "the client assertion lives too long"));
    }
    let jti = claims.get("jti").and_then(Value::as_str).filter(|j| !j.is_empty()).ok_or_else(|| Error::field("client_assertion", "needs a jti"))?;
    // Each assertion is used once.
    let fresh = sqlx::query("insert into lms_lti_jti (tenant_id, client_id, jti, expires_at) values ($1, $2, $3, to_timestamp($4)) on conflict do nothing")
        .bind(req.tenant)
        .bind(&client)
        .bind(jti)
        .bind(exp as f64)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if fresh == 0 {
        return Ok(fail("invalid_client", "this client assertion was used already"));
    }
    let asked: Vec<&str> = get("scope").unwrap_or_default().split(' ').filter(|s| !s.is_empty()).collect();
    let granted: Vec<&str> = asked.iter().copied().filter(|s| [SCORE_SCOPE, LINEITEM_SCOPE].contains(s)).collect();
    if granted.is_empty() {
        return Ok(fail("invalid_scope", "Learn grants the AGS score and line item scopes"));
    }
    let user = service_user(k, &ctx, &mut tx, &tool).await?;
    tx.commit().await?;
    let (_, access) = k.create_api_key(&elevated(&ctx), user, "LTI access token", Some(Utc::now() + Duration::seconds(ACCESS_SECONDS)), None).await?;
    let mut r = SiteResponse::json(200, &json!({"access_token": access, "token_type": "Bearer", "expires_in": ACCESS_SECONDS, "scope": granted.join(" ")}));
    r.headers.push(("cache-control".into(), "no-store".into()));
    Ok(r)
}

/// The key a tool signs with: its PEM, or the key of its key set with the assertion's `kid`.
async fn tool_key(k: &Kernel, tool: &Doc, kid: Option<&str>) -> Result<DecodingKey> {
    if let Some(pem) = s(tool, "public_key") {
        let key = RsaPublicKey::from_public_key_pem(pem).map_err(|_| Error::invalid("the tool's public key cannot be read"))?;
        return DecodingKey::from_rsa_components(&URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()), &URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()))
            .map_err(|_| Error::invalid("the tool's public key cannot be used"));
    }
    let url = s(tool, "jwks_url").ok_or_else(|| Error::invalid("the tool has no key"))?;
    let (client, parsed) = k.vetted_client(url, false, std::time::Duration::from_secs(10), "Ferrum Learn (LTI)").await?;
    let set: jsonwebtoken::jwk::JwkSet = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| Error::Unavailable(format!("the tool's keys cannot be fetched: {e}")))?
        .json()
        .await
        .map_err(|_| Error::invalid("the tool's key set is not JSON"))?;
    let jwk = match kid {
        Some(kid) => set.find(kid),
        None => set.keys.first(),
    }
    .ok_or_else(|| Error::invalid("the tool's key set has no key for this assertion"))?;
    DecodingKey::from_jwk(jwk).map_err(|_| Error::invalid("the tool's key cannot be used"))
}

/// The tool's service user (made on its first token): the person behind its access tokens, with
/// no role, so its keys reach nothing but its own lessons' line items.
async fn service_user(k: &Kernel, ctx: &Ctx, tx: &mut Tx, tool: &Doc) -> Result<Uuid> {
    let email = format!("lti-{}@lti.ferrum.invalid", s(tool, "client_id").unwrap_or_default());
    if let Some(u) = util::user(tx, ctx.tenant, &email).await? {
        return Ok(u.id);
    }
    let name = format!("LTI tool: {}", s(tool, "tool_name").unwrap_or_default());
    k.create_user(&elevated(ctx), &email, &name, None, &[], false).await
}

/// Which tool a caller is the service user of.
pub async fn tool_of(tx: &mut Tx, ctx: &Ctx) -> Result<Option<Uuid>> {
    let Some(client) = ctx.email.strip_prefix("lti-").and_then(|r| r.strip_suffix("@lti.ferrum.invalid")) else { return Ok(None) };
    Ok(sqlx::query_scalar("select id from e_lms_lti_tool where tenant_id = $1 and client_id = $2")
        .bind(ctx.tenant)
        .bind(client)
        .fetch_optional(&mut **tx)
        .await?)
}

/// `lti/ags/{lesson}` (the tool's line item) and `lti/ags/{lesson}/scores` (its scores), for the
/// tool's service user only.
pub async fn ags(k: &Kernel, ctx: &Ctx, req: &SiteRequest, lesson: Uuid, rest: &[&str]) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let gone = || Error::NotFound("line item".into());
    let tool = tool_of(&mut tx, ctx).await?.ok_or_else(|| Error::Forbidden("line items answer their tool only".into()))?;
    let sctx = sys(ctx);
    let l = match load(k, &sctx, &mut tx, "LmsLesson", lesson).await {
        Ok(l) if s(&l, "lesson_type") == Some("LTI") && id(&l, "lti_tool") == Some(tool) => l,
        Ok(_) | Err(Error::NotFound(_)) => return Err(gone()),
        Err(e) => return Err(e),
    };
    let t = load(k, &sctx, &mut tx, "LmsLtiTool", tool).await?;
    if !util::b(&t, "sends_grades") {
        return Err(Error::Forbidden("this tool is not one that sends grades".into()));
    }
    let p = platform(&mut tx, ctx.tenant).await?.ok_or_else(|| Error::invalid("Learn has no public URL"))?;
    let out = match (req.method.as_str(), rest) {
        ("GET", []) => {
            let mut r = SiteResponse::json(
                200,
                &json!({"id": lineitem_url(&p.issuer, lesson), "scoreMaximum": 100, "label": s(&l, "title"), "resourceLinkId": lesson.to_string()}),
            );
            r.content_type = "application/vnd.ims.lis.v2.lineitem+json".into();
            r
        }
        ("POST", ["scores"]) => {
            let score: Value = serde_json::from_slice(&req.body).map_err(|_| Error::field("score", "is not JSON"))?;
            let result = record_score(k, ctx, &mut tx, &l, &score).await?;
            let mut r = SiteResponse::json(200, &result);
            r.headers.push(("cache-control".into(), "no-store".into()));
            r
        }
        _ => return Err(gone()),
    };
    tx.commit().await?;
    Ok(out)
}

/// What a score says: its percentage (when graded) and whether the activity is done.
pub fn graded(score: &Value) -> std::result::Result<(Option<Decimal>, &'static str, bool), String> {
    let progress = score.get("activityProgress").and_then(Value::as_str).ok_or("needs activityProgress")?;
    let grading = score.get("gradingProgress").and_then(Value::as_str).ok_or("needs gradingProgress")?;
    let status = match progress {
        "Initialized" | "Started" | "InProgress" => "Incomplete",
        "Submitted" | "Completed" => "Completed",
        _ => return Err(format!("activityProgress {progress} is not one of AGS's")),
    };
    let num = |k: &str| score.get(k).and_then(Value::as_f64).and_then(|v| Decimal::try_from(v).ok());
    let percent = match (grading, num("scoreGiven"), num("scoreMaximum")) {
        ("FullyGraded", Some(given), Some(max)) if max > Decimal::ZERO && given >= Decimal::ZERO => {
            Some((given * Decimal::ONE_HUNDRED / max).clamp(Decimal::ZERO, Decimal::ONE_HUNDRED).round_dp(2))
        }
        ("FullyGraded" | "Pending" | "PendingManual" | "Failed" | "NotReady", _, _) => None,
        _ => return Err(format!("gradingProgress {grading} is not one of AGS's")),
    };
    Ok((percent, status, progress == "Completed"))
}

async fn record_score(k: &Kernel, ctx: &Ctx, tx: &mut Tx, l: &Doc, score: &Value) -> Result<Value> {
    let user: Uuid =
        score.get("userId").and_then(Value::as_str).and_then(|u| u.parse().ok()).ok_or_else(|| Error::field("userId", "is not a learner of this lesson"))?;
    let (percent, status, done) = graded(score).map_err(|m| Error::field("score", m))?;
    let email: Option<String> = sqlx::query_scalar("select lower(email::text) from app_user where tenant_id = $1 and id = $2")
        .bind(ctx.tenant)
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?;
    let lid = doc_id(l)?;
    let enrollment: Option<Uuid> = match &email {
        Some(e) => {
            sqlx::query_scalar("select id from e_lms_enrollment where tenant_id = $1 and course = $2 and learner = $3 order by coalesce(cycle, 1) desc limit 1")
                .bind(ctx.tenant)
                .bind(id(l, "course"))
                .bind(e)
                .fetch_optional(&mut **tx)
                .await?
        }
        None => None,
    };
    let eid = enrollment.ok_or_else(|| Error::field("userId", "is not a learner of this lesson"))?;
    let e = load(k, &sys(ctx), tx, "LmsEnrollment", eid).await?;
    let row: Option<(Uuid, String, i64)> = sqlx::query_as(
        "select id, status, coalesce(commits, 0) from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3 for update",
    )
    .bind(ctx.tenant)
    .bind(eid)
    .bind(lid)
    .fetch_optional(&mut **tx)
    .await?;
    let finished = |st: &str| st == "Completed" || st == "Passed";
    let mut v = Doc::new();
    v.insert("last_commit_at".into(), json!(util::ts(util::now())));
    if !row.as_ref().is_some_and(|(_, prior, _)| finished(prior) && !finished(status)) {
        v.insert("status".into(), json!(status));
    }
    if let Some(p) = percent {
        v.insert("score_percent".into(), json!(p.to_string()));
    }
    match row {
        Some((pid, _, n)) => {
            v.insert("commits".into(), json!(n + 1));
            k.set_values_tx(&sys(ctx), tx, "LmsPackageProgress", pid, v).await?;
        }
        None => {
            for (f, val) in [
                ("enrollment", json!(eid.to_string())),
                ("lesson", json!(lid.to_string())),
                ("course", l.get("course").cloned().unwrap_or(Value::Null)),
                ("learner", e.get("learner").cloned().unwrap_or(Value::Null)),
                ("commits", json!(1)),
            ] {
                v.insert(f.into(), val);
            }
            k.insert_tx(&elevated(&sys(ctx)), tx, "LmsPackageProgress", v).await?;
        }
    }
    let completed = done && s(&e, "status") != Some(WITHDRAWN);
    if completed {
        enrollment::record_completion(k, &sys(ctx), tx, eid, lid, "LTI").await?;
    }
    Ok(json!({"status": status, "score_percent": percent.map(|p| p.to_string()), "completed": completed}))
}

/// What the player shows of an LTI lesson: the tool and where its launch starts.
pub async fn player(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lesson: &Doc) -> Result<Value> {
    let Some(tool) = id(lesson, "lti_tool") else { return Ok(Value::Null) };
    let t = load(k, ctx, tx, "LmsLtiTool", tool).await?;
    Ok(json!({
        "tool_name": s(&t, "tool_name"), "launch_url": format!("/api/app/{}/lti/{}/launch", util::PACKAGE, doc_id(lesson)?),
        "sends_grades": util::b(&t, "sends_grades"),
    }))
}

/// Whether a learner marks an LTI lesson done themselves: when its tool sends no grades.
pub async fn learner_marks_done(tx: &mut Tx, tenant: Uuid, lesson: &Doc) -> Result<bool> {
    let Some(tool) = id(lesson, "lti_tool") else { return Ok(false) };
    let grades: Option<bool> = sqlx::query_scalar("select sends_grades from e_lms_lti_tool where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(tool)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
    Ok(!grades.unwrap_or(false))
}

/// Access tokens of tools that have expired (their API keys), and spent client assertions.
pub async fn purge(k: &Kernel, tenant: Uuid) -> Result<usize> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    let keys = sqlx::query(
        "delete from api_key k using app_user u where k.tenant_id = $1 and u.tenant_id = k.tenant_id and u.id = k.user_id \
         and u.email::text like 'lti-%@lti.ferrum.invalid' and k.expires_at < now()",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let spent = sqlx::query("delete from lms_lti_jti where tenant_id = $1 and expires_at < now() - interval '1 hour'")
        .bind(tenant)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok((keys + spent) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_score_says() {
        assert_eq!(
            graded(&json!({"activityProgress": "Completed", "gradingProgress": "FullyGraded", "scoreGiven": 45, "scoreMaximum": 50})),
            Ok((Some(Decimal::from(90)), "Completed", true))
        );
        assert_eq!(
            graded(&json!({"activityProgress": "Submitted", "gradingProgress": "PendingManual"})),
            Ok((None, "Completed", false)),
            "handed in, not done"
        );
        assert_eq!(graded(&json!({"activityProgress": "InProgress", "gradingProgress": "NotReady"})), Ok((None, "Incomplete", false)));
        assert!(graded(&json!({"activityProgress": "Done", "gradingProgress": "FullyGraded"})).is_err());
        assert!(graded(&json!({"activityProgress": "Completed"})).is_err());
        assert_eq!(
            graded(&json!({"activityProgress": "Completed", "gradingProgress": "FullyGraded", "scoreGiven": 70, "scoreMaximum": 50})).unwrap().0,
            Some(Decimal::from(100)),
            "capped"
        );
    }

    #[test]
    fn the_launch_page_posts_and_runs_nothing_else() {
        let r = form_post("https://tool.test/launch?x=1", &[("id_token", "a\"b<c"), ("state", "s")]);
        let html = String::from_utf8(r.body.clone()).unwrap();
        assert!(html.contains("value=\"a&quot;b&lt;c\""), "{html}");
        let csp = r.headers.iter().find(|(k, _)| k == "content-security-policy").unwrap().1.clone();
        assert!(csp.contains("form-action https://tool.test;") && csp.contains("script-src 'sha256-") && csp.starts_with("default-src 'none'"), "{csp}");
    }
}
