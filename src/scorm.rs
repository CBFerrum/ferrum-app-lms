//! SCORM packages (SCORM 1.2 and 2004) as lessons.
//!
//! A SCORM lesson holds an uploaded package (a zip). Saving the lesson reads the package's
//! `imsmanifest.xml` (at the root or in the package's one top folder) for its SCORM version and the
//! file its first SCO starts from. Learn serves the package's files to the learners who may open the
//! lesson, from the zip, below the lesson's address; Learn's player gives them the SCORM API
//! (scorm-again) and commits what the package records to the learner's SCORM Progress: the
//! package's data (resumed next time), its status, score and time. Completed or passed, the lesson
//! is completed.
//!
//! A package's scripts run on the workspace's origin with the learner's session, as course content
//! does in other LMSs. Learn Settings must allow packages first, and only a course's authors attach
//! them.

use crate::enrollment::{self, WITHDRAWN};
use crate::util::{self, doc_id, elevated, id, s, sys};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, SiteResponse, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// The largest package a lesson takes. ponytail: packages are read whole into memory (and kept
/// there for their next requests); storing their files one by one would lift this.
pub const MAX_PACKAGE: i64 = 256 << 20;
/// The largest file served from a package once unzipped.
const MAX_ENTRY: u64 = 256 << 20;
/// The most files a package may hold.
const MAX_FILES: usize = 20_000;
/// The most a learner's SCORM data may weigh (SCORM 2004's suspend data alone may be 64,000 characters).
pub const MAX_CMI: usize = 1 << 20;
/// Packages kept in memory, by bytes.
const CACHE_BYTES: usize = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Scorm12,
    Scorm2004,
}

impl Version {
    pub fn label(self) -> &'static str {
        match self {
            Version::Scorm12 => "SCORM 1.2",
            Version::Scorm2004 => "SCORM 2004",
        }
    }

    fn parse(v: &str) -> Option<Self> {
        match v {
            "SCORM 1.2" => Some(Version::Scorm12),
            "SCORM 2004" => Some(Version::Scorm2004),
            _ => None,
        }
    }
}

/// What a package launches.
#[derive(Debug, PartialEq)]
pub struct Package {
    pub version: Version,
    /// The file the first SCO starts from, inside the zip, with its parameters.
    pub launch: String,
    /// SCORM 1.2's mastery score of that SCO, when the manifest gives one.
    pub mastery_score: Option<Decimal>,
}

/// Reads a package: refuses what is not a zip, holds too many files or a path that leaves the
/// package, and a manifest that names no SCO to launch.
pub fn inspect(zip: &[u8]) -> std::result::Result<Package, String> {
    let (names, base, xml) = descriptor(zip, "imsmanifest.xml", "a SCORM package")?;
    let (version, launch, mastery_score) = read_manifest(&xml)?;
    let launch = launchable(&names, &base, &launch)?;
    Ok(Package { version, launch, mastery_score })
}

/// A package's files, and the folder and text of its descriptor (`imsmanifest.xml`, `tincan.xml`):
/// at the package's root or in its one top folder. Refuses what is not a zip, holds too many files
/// or a path that leaves the package.
pub(crate) fn descriptor(zip: &[u8], name: &str, what: &str) -> std::result::Result<(Vec<String>, String, String), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|_| "is not a zip file".to_string())?;
    if archive.len() > MAX_FILES {
        return Err(format!("holds more than {MAX_FILES} files"));
    }
    let names: Vec<String> = archive
        .file_names()
        .map(|n| n.map(|c| c.into_owned()))
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| "holds a file whose name cannot be read".to_string())?;
    if let Some(bad) = names.iter().find(|n| clean(n).is_none() && !n.ends_with('/')) {
        return Err(format!("holds a file outside the package ({bad})"));
    }
    let found = names
        .iter()
        .filter(|n| n.rsplit('/').next() == Some(name) && n.matches('/').count() <= 1)
        .min_by_key(|n| n.len())
        .cloned()
        .ok_or_else(|| format!("has no {name} at its root: is it {what}?"))?;
    let base = found.strip_suffix(name).unwrap_or_default().to_string();
    let mut xml = String::new();
    archive
        .by_name(&found)
        .map_err(|_| format!("cannot read its {name}"))?
        .take(4 << 20)
        .read_to_string(&mut xml)
        .map_err(|_| format!("has a {name} that is not UTF-8 text"))?;
    Ok((names, base, xml))
}

/// The launch path below the descriptor's folder, when the package holds its file.
pub(crate) fn launchable(names: &[String], base: &str, launch: &str) -> std::result::Result<String, String> {
    let launch = format!("{base}{launch}");
    let file = launch.split(['?', '#']).next().unwrap_or_default();
    if clean(file).is_none() || !names.iter().any(|n| n == file) {
        return Err(format!("names {file} to launch, which it does not hold"));
    }
    Ok(launch)
}

/// The version, the launch file of the first SCO (relative to the manifest) and its mastery score.
fn read_manifest(xml: &str) -> std::result::Result<(Version, String, Option<Decimal>), String> {
    // No DTDs: a manifest has no business declaring entities.
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("has an imsmanifest.xml that is not well-formed XML ({e})"))?;
    let root = doc.root_element();
    let named = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name);
    let attr = |n: &roxmltree::Node, name: &str| n.attributes().find(|a| a.name().eq_ignore_ascii_case(name)).map(|a| a.value().trim().to_string());
    let schema_version = root.descendants().find(|n| named(n, "schemaversion")).and_then(|n| n.text()).unwrap_or_default().trim().to_string();
    let namespaces: Vec<&str> = root.descendants().flat_map(|n| n.namespaces().map(|ns| ns.uri()).collect::<Vec<_>>()).collect();
    let version = if schema_version == "1.2" {
        Version::Scorm12
    } else if schema_version.contains("2004") || schema_version.contains("CAM 1.3") || namespaces.iter().any(|u| u.contains("adlcp_v1p3")) {
        Version::Scorm2004
    } else {
        Version::Scorm12
    };
    let resources: Vec<roxmltree::Node> = root.descendants().filter(|n| named(n, "resource")).collect();
    let is_sco = |r: &roxmltree::Node| attr(r, "scormtype").is_some_and(|t| t.eq_ignore_ascii_case("sco"));
    // xml:base on the manifest, the resources and the resource itself prefixes a resource's href.
    let base_of = |r: &roxmltree::Node| -> String {
        r.ancestors()
            .filter(|n| n.is_element())
            .filter_map(|n| n.attribute(("http://www.w3.org/XML/1998/namespace", "base")))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };
    let href_of = |r: &roxmltree::Node| attr(r, "href").filter(|h| !h.is_empty()).map(|h| format!("{}{h}", base_of(r)));
    // The default organization's first item that points at a SCO, in document order.
    let organizations = root.descendants().find(|n| named(n, "organizations"));
    let default = organizations.and_then(|o| attr(&o, "default"));
    let organization = organizations.and_then(|o| {
        let all: Vec<roxmltree::Node> = o.children().filter(|n| named(n, "organization")).collect();
        all.iter().find(|n| default.is_some() && attr(n, "identifier") == default).or(all.first()).copied()
    });
    let mut found: Option<(String, Option<Decimal>)> = None;
    if let Some(org) = organization {
        for item in org.descendants().filter(|n| named(n, "item")) {
            let Some(target) = attr(&item, "identifierref") else { continue };
            let Some(resource) = resources.iter().find(|r| attr(r, "identifier").as_deref() == Some(target.as_str()) && is_sco(r)) else { continue };
            let Some(href) = href_of(resource) else { continue };
            let parameters = attr(&item, "parameters").unwrap_or_default();
            let launch = match parameters.chars().next() {
                None => href,
                Some('?' | '#') => format!("{href}{parameters}"),
                Some(_) => format!("{href}{}{parameters}", if href.contains('?') { '&' } else { '?' }),
            };
            let mastery = item.descendants().find(|n| named(n, "masteryscore")).and_then(|n| n.text()).and_then(|t| t.trim().parse::<Decimal>().ok());
            found = Some((launch, mastery));
            break;
        }
    }
    let (launch, mastery) = match found {
        Some(f) => f,
        None => (resources.iter().filter(|r| is_sco(r)).find_map(href_of).ok_or_else(|| "has no SCO to launch in its imsmanifest.xml".to_string())?, None),
    };
    Ok((version, launch, mastery))
}

/// A path inside a package as served: its segments without `.`, `..`, backslashes or a leading `/`.
fn clean(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    Some(parts.join("/"))
}

/// A file of a package, unzipped (none when the package does not hold it).
pub fn entry(zip: &[u8], path: &str) -> Result<Option<Vec<u8>>> {
    let Some(path) = clean(path) else { return Ok(None) };
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|_| Error::invalid("the lesson's package is not a zip file"))?;
    let mut file = match archive.by_name(&path) {
        Ok(f) if f.is_file() => f,
        _ => return Ok(None),
    };
    if file.size() > MAX_ENTRY {
        return Err(Error::invalid("this file of the package is too large to serve"));
    }
    let mut out = Vec::with_capacity(file.size() as usize);
    file.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|_| Error::invalid("this file of the package cannot be read"))?;
    Ok(Some(out))
}

/// The media type a package's file is served as, by its extension.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "xml" | "xsd" => "application/xml",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "vtt" => "text/vtt",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "swf" => "application/x-shockwave-flash",
        _ => "application/octet-stream",
    }
}

/// A package's file as a response: its scripts may run (inline too, as packages are written), it
/// talks to this server only, and only Learn's own pages frame it.
pub fn respond(path: &str, body: Vec<u8>) -> SiteResponse {
    let policy = "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:; style-src 'self' 'unsafe-inline' https:; \
                  img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; font-src 'self' data: https:; connect-src 'self'; \
                  frame-src 'self' https:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'";
    SiteResponse {
        status: 200,
        content_type: content_type(path).into(),
        headers: vec![
            ("content-security-policy".into(), policy.into()),
            ("x-content-type-options".into(), "nosniff".into()),
            ("referrer-policy".into(), "same-origin".into()),
            // The address carries the package's file id: a new package is a new address.
            ("cache-control".into(), "private, max-age=86400, immutable".into()),
        ],
        body,
    }
}

static CACHE: Mutex<Vec<(Uuid, Arc<Vec<u8>>)>> = Mutex::new(Vec::new());

/// A package's bytes: from memory when read lately, else from the store (and kept, the oldest
/// dropped beyond [`CACHE_BYTES`]).
pub async fn bytes(k: &Kernel, ctx: &Ctx, file: Uuid) -> Result<Arc<Vec<u8>>> {
    if let Some(hit) = {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let at = cache.iter().position(|(f, _)| *f == file);
        at.map(|i| {
            let item = cache.remove(i);
            let data = item.1.clone();
            cache.push(item);
            data
        })
    } {
        return Ok(hit);
    }
    let (_, _, data) = k.read_file(&sys(ctx), file).await?;
    let data = Arc::new(data);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(f, _)| *f != file);
    cache.push((file, data.clone()));
    let mut total: usize = cache.iter().map(|(_, d)| d.len()).sum();
    while total > CACHE_BYTES && cache.len() > 1 {
        total -= cache.remove(0).1.len();
    }
    Ok(data)
}

/// Checks a SCORM or xAPI lesson's package and finds what it launches (`package_standard`,
/// `package_launch`, and `package_mastery_score` or `package_activity`).
pub async fn on_save(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc) -> Result<()> {
    let kind = s(doc, "lesson_type").unwrap_or_default().to_string();
    if kind != "SCORM" && kind != "xAPI" {
        for f in ["package_file", "package_standard", "package_launch", "package_activity", "package_mastery_score"] {
            doc.insert(f.into(), Value::Null);
        }
        return Ok(());
    }
    if !allowed(tx, ctx.tenant).await? {
        return Err(Error::field("lesson_type", format!("{kind} packages are not allowed in Learn Settings")));
    }
    let Some(path) = s(doc, "package_file") else { return Ok(()) };
    let file = util::file_id(path).ok_or_else(|| Error::field("package_file", "upload the package with /api/files"))?;
    let row: Option<(i64, Option<String>)> = sqlx::query_as("select size_bytes, scan_status from file_object where tenant_id = $1 and id = $2")
        .bind(ctx.tenant)
        .bind(file)
        .fetch_optional(&mut **tx)
        .await?;
    let Some((size, scan)) = row else { return Err(Error::field("package_file", "no uploaded file has this address")) };
    if size > MAX_PACKAGE {
        return Err(Error::field("package_file", format!("is larger than the {} MB a package may be", MAX_PACKAGE >> 20)));
    }
    if scan.as_deref() == Some("infected") {
        return Err(Error::field("package_file", "was found infected when it was uploaded"));
    }
    let data = bytes(k, ctx, file).await?;
    let problem = |m: String| Error::field("package_file", format!("the package {m}"));
    if kind == "xAPI" {
        let (launch, activity) = crate::xapi::inspect(&data).map_err(problem)?;
        doc.insert("package_standard".into(), json!("xAPI"));
        doc.insert("package_launch".into(), json!(launch));
        doc.insert("package_activity".into(), json!(activity));
        doc.insert("package_mastery_score".into(), Value::Null);
        return Ok(());
    }
    let p = inspect(&data).map_err(problem)?;
    doc.insert("package_standard".into(), json!(p.version.label()));
    doc.insert("package_launch".into(), json!(p.launch));
    doc.insert("package_activity".into(), Value::Null);
    doc.insert("package_mastery_score".into(), p.mastery_score.map_or(Value::Null, |m| json!(m.to_string())));
    Ok(())
}

async fn allowed(tx: &mut Tx, tenant: Uuid) -> Result<bool> {
    let on: Option<bool> =
        sqlx::query_scalar("select allow_packages from e_lms_settings where tenant_id = $1 limit 1").bind(tenant).fetch_optional(&mut **tx).await?.flatten();
    Ok(on.unwrap_or(false))
}

/// The address of a package's file for Learn's player.
pub fn file_url(lesson: Uuid, package: Uuid, path: &str) -> String {
    format!("/api/app/{}/lessons/{lesson}/package/{package}/{path}", util::PACKAGE)
}

/// What the player needs to run a SCORM lesson: the version, the launch address and, for an
/// enrolled learner, their data so far (to resume) and who they are to the package.
pub async fn player(ctx: &Ctx, tx: &mut Tx, lesson: &Doc, enrollment: Option<&Doc>) -> Result<Value> {
    let lid = doc_id(lesson)?;
    let (Some(version), Some(launch), Some(package)) =
        (s(lesson, "package_standard").and_then(Version::parse), s(lesson, "package_launch"), s(lesson, "package_file").and_then(util::file_id))
    else {
        return Ok(Value::Null);
    };
    let mut saved = Value::Null;
    let mut learner = Value::Null;
    if let Some(e) = enrollment {
        let row: Option<(Option<Value>, String)> =
            sqlx::query_as("select cmi, status from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3")
                .bind(ctx.tenant)
                .bind(doc_id(e)?)
                .bind(lid)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some((Some(cmi), status)) = row {
            saved = json!({"cmi": cmi, "status": status});
        }
        learner = json!({"id": s(e, "learner"), "name": s(e, "learner_name").or(s(e, "learner"))});
    }
    Ok(json!({
        "version": version.label().trim_start_matches("SCORM "), "launch_url": file_url(lid, package, launch), "mastery_score": lesson.get("package_mastery_score"),
        "learner": learner, "saved": saved,
    }))
}

/// What a commit of SCORM data says about the lesson.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub status: &'static str,
    /// The score as a percentage, when the package reports one.
    pub score: Option<Decimal>,
    /// The learner's time in the lesson so far, in seconds.
    pub seconds: Option<i64>,
    pub done: bool,
}

/// SCORM 1.2 reads `cmi.core.lesson_status` (passed and completed finish the lesson); SCORM 2004
/// reads `cmi.completion_status` and `cmi.success_status` (completed or passed finishes it, unless it
/// failed). The score is `score.raw` against `score.min`/`score.max` (0 to 100 by default), or 2004's
/// `score.scaled`.
pub fn outcome(version: Version, cmi: &Value) -> Outcome {
    let text = |p: &str| cmi.pointer(p).and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_number().map(|n| n.to_string()))).unwrap_or_default();
    let num = |p: &str| text(p).trim().parse::<Decimal>().ok();
    let percent = |raw: Option<Decimal>, min: Option<Decimal>, max: Option<Decimal>| {
        let (min, max) = (min.unwrap_or(Decimal::ZERO), max.unwrap_or(Decimal::ONE_HUNDRED));
        raw.filter(|_| max > min).map(|r| ((r - min) * Decimal::ONE_HUNDRED / (max - min)).clamp(Decimal::ZERO, Decimal::ONE_HUNDRED).round_dp(2))
    };
    match version {
        Version::Scorm12 => {
            let status = match text("/core/lesson_status").as_str() {
                "passed" => "Passed",
                "completed" => "Completed",
                "failed" => "Failed",
                "incomplete" => "Incomplete",
                "browsed" => "Browsed",
                _ => "Not Attempted",
            };
            Outcome {
                status,
                score: percent(num("/core/score/raw"), num("/core/score/min"), num("/core/score/max")),
                seconds: seconds(&text("/core/total_time")),
                done: matches!(status, "Passed" | "Completed"),
            }
        }
        Version::Scorm2004 => {
            let (completion, success) = (text("/completion_status"), text("/success_status"));
            let status = match (completion.as_str(), success.as_str()) {
                (_, "failed") => "Failed",
                (_, "passed") => "Passed",
                ("completed", _) => "Completed",
                ("incomplete", _) => "Incomplete",
                _ => "Not Attempted",
            };
            let scaled = num("/score/scaled").map(|s| (s * Decimal::ONE_HUNDRED).clamp(Decimal::ZERO, Decimal::ONE_HUNDRED).round_dp(2));
            Outcome {
                status,
                score: scaled.or_else(|| percent(num("/score/raw"), num("/score/min"), num("/score/max"))),
                seconds: seconds(&text("/total_time")),
                done: matches!(status, "Passed" | "Completed"),
            }
        }
    }
}

/// Seconds in a SCORM 1.2 time (`HHHH:MM:SS.SS`) or a SCORM 2004 duration (`PT1H2M3.5S`, `P1DT2H`).
pub fn seconds(t: &str) -> Option<i64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix('P') {
        let (date, time) = rest.split_once('T').unwrap_or((rest, ""));
        let mut total = Decimal::ZERO;
        for (part, units) in
            [(date, &[('Y', 31_536_000i64), ('M', 2_592_000), ('W', 604_800), ('D', 86_400)][..]), (time, &[('H', 3_600i64), ('M', 60), ('S', 1)][..])]
        {
            let mut number = String::new();
            for c in part.chars() {
                if c.is_ascii_digit() || c == '.' {
                    number.push(c);
                } else {
                    let unit = units.iter().find(|(u, _)| *u == c)?.1;
                    total += number.parse::<Decimal>().ok()? * Decimal::from(unit);
                    number.clear();
                }
            }
            if !number.is_empty() {
                return None;
            }
        }
        return total.trunc().try_into().ok();
    }
    let parts: Vec<&str> = t.split(':').collect();
    let [h, m, s] = parts.as_slice() else { return None };
    let (h, m, s) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?, s.parse::<Decimal>().ok()?);
    let s: i64 = s.trunc().try_into().ok()?;
    (m < 60 && s < 60).then_some(h * 3_600 + m * 60 + s)
}

/// Records a learner's SCORM data for a lesson of their enrolment, and completes the lesson when the
/// package says it is completed or passed.
pub async fn commit(k: &Kernel, ctx: &Ctx, tx: &mut Tx, e: &Doc, course: &Doc, lesson: Uuid, cmi: &Value) -> Result<Value> {
    if s(e, "status") == Some(WITHDRAWN) {
        return Err(Error::invalid("this enrolment is withdrawn: enrol again to continue"));
    }
    if !cmi.is_object() {
        return Err(Error::field("cmi", "must be the package's SCORM data (an object)"));
    }
    if serde_json::to_vec(cmi).map(|v| v.len()).unwrap_or(usize::MAX) > MAX_CMI {
        return Err(Error::field("cmi", format!("is larger than the {} KB a lesson's SCORM data may be", MAX_CMI >> 10)));
    }
    let l = util::load(k, ctx, tx, "LmsLesson", lesson).await?;
    if id(&l, "course") != id(course, "id") || s(&l, "lesson_type") != Some("SCORM") {
        return Err(Error::field("lesson", "is not a SCORM lesson of this course"));
    }
    let version = s(&l, "package_standard").and_then(Version::parse).ok_or_else(|| Error::invalid("this lesson has no package to play yet"))?;
    let o = outcome(version, cmi);
    let enrollment = doc_id(e)?;
    let existing: Option<(Uuid, i64)> =
        sqlx::query_as("select id, coalesce(commits, 0) from e_lms_package_progress where tenant_id = $1 and enrollment = $2 and lesson = $3 for update")
            .bind(ctx.tenant)
            .bind(enrollment)
            .bind(lesson)
            .fetch_optional(&mut **tx)
            .await?;
    let mut v = Doc::new();
    v.insert("cmi".into(), cmi.clone());
    v.insert("status".into(), json!(o.status));
    v.insert("score_percent".into(), o.score.map_or(Value::Null, |p| json!(p.to_string())));
    v.insert("total_seconds".into(), o.seconds.map_or(Value::Null, |t| json!(t)));
    v.insert("last_commit_at".into(), json!(util::ts(util::now())));
    let engine = elevated(ctx);
    match existing {
        Some((row, commits)) => {
            v.insert("commits".into(), json!(commits + 1));
            k.set_values_tx(&sys(ctx), tx, "LmsPackageProgress", row, v).await?;
        }
        None => {
            for (f, val) in [
                ("enrollment", json!(enrollment.to_string())),
                ("lesson", json!(lesson.to_string())),
                ("course", course.get("id").cloned().unwrap_or(Value::Null)),
                ("learner", e.get("learner").cloned().unwrap_or(Value::Null)),
                ("commits", json!(1)),
            ] {
                v.insert(f.into(), val);
            }
            k.insert_tx(&engine, tx, "LmsPackageProgress", v).await?;
        }
    }
    let progress = if o.done { Some(enrollment::record_completion(k, ctx, tx, enrollment, lesson, "SCORM").await?) } else { None };
    Ok(json!({"status": o.status, "score_percent": o.score.map(|p| p.to_string()), "completed": o.done, "progress": progress}))
}

/// SCORM progress is written by Learn's player only (an LMS manager may delete one, to start over).
pub struct ProgressHook;

#[async_trait::async_trait]
impl ferrum_kernel::DocHook for ProgressHook {
    async fn validate(&self, hc: &mut ferrum_kernel::HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::invalid("SCORM progress is recorded by the lesson's package as the learner works through it"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(body.as_bytes()).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    const V12: &str = r#"<?xml version="1.0"?>
<manifest identifier="m" version="1" xmlns="http://www.imsproject.org/xsd/imscp_rootv1p1p2" xmlns:adlcp="http://www.adlnet.org/xsd/adlcp_rootv1p2">
  <metadata><schema>ADL SCORM</schema><schemaversion>1.2</schemaversion></metadata>
  <organizations default="org">
    <organization identifier="other"><item identifier="i0" identifierref="r0"><title>Not this one</title></item></organization>
    <organization identifier="org">
      <title>Rust</title>
      <item identifier="i1"><title>A folder</title>
        <item identifier="i2" identifierref="asset"><title>An asset</title></item>
        <item identifier="i3" identifierref="r1" parameters="?lang=en"><title>Ownership</title><adlcp:masteryscore>80</adlcp:masteryscore></item>
      </item>
    </organization>
  </organizations>
  <resources>
    <resource identifier="r0" type="webcontent" adlcp:scormtype="sco" href="zero.html"/>
    <resource identifier="asset" type="webcontent" adlcp:scormtype="asset" href="logo.png"/>
    <resource identifier="r1" type="webcontent" adlcp:scormtype="sco" xml:base="lessons/" href="index.html"><file href="lessons/index.html"/></resource>
  </resources>
</manifest>"#;

    const V2004: &str = r#"<?xml version="1.0"?>
<manifest identifier="m" xmlns="http://www.imsglobal.org/xsd/imscp_v1p1" xmlns:adlcp="http://www.adlnet.org/xsd/adlcp_v1p3">
  <metadata><schema>ADL SCORM</schema><schemaversion>2004 4th Edition</schemaversion></metadata>
  <organizations default="o"><organization identifier="o"><item identifier="i" identifierref="r"><title>Go</title></item></organization></organizations>
  <resources><resource identifier="r" type="webcontent" adlcp:scormType="sco" href="story.html"/></resources>
</manifest>"#;

    #[test]
    fn the_default_organizations_first_sco_launches() {
        let p = inspect(&zip(&[("imsmanifest.xml", V12), ("lessons/index.html", "<p>hi</p>"), ("zero.html", ""), ("logo.png", "")])).unwrap();
        assert_eq!(p, Package { version: Version::Scorm12, launch: "lessons/index.html?lang=en".into(), mastery_score: Some(Decimal::from(80)) });
        let p = inspect(&zip(&[("course/imsmanifest.xml", V2004), ("course/story.html", "")])).unwrap();
        assert_eq!((p.version, p.launch.as_str()), (Version::Scorm2004, "course/story.html"), "a package in one top folder");
    }

    #[test]
    fn packages_that_cannot_play_are_refused() {
        assert!(inspect(b"not a zip").unwrap_err().contains("not a zip"));
        assert!(inspect(&zip(&[("index.html", "")])).unwrap_err().contains("imsmanifest.xml"));
        assert!(inspect(&zip(&[("imsmanifest.xml", V2004)])).unwrap_err().contains("does not hold"), "the launch file is missing");
        assert!(inspect(&zip(&[("imsmanifest.xml", "<manifest><resources/></manifest>")])).unwrap_err().contains("no SCO"));
        assert!(inspect(&zip(&[("imsmanifest.xml", V2004), ("story.html", ""), ("../escape.html", "")])).unwrap_err().contains("outside the package"));
        let dtd = r#"<?xml version="1.0"?><!DOCTYPE m [<!ENTITY x "y">]><manifest/>"#;
        assert!(inspect(&zip(&[("imsmanifest.xml", dtd)])).unwrap_err().contains("not well-formed"), "no entity declarations");
    }

    #[test]
    fn files_are_found_only_inside_the_package() {
        let z = zip(&[("imsmanifest.xml", V2004), ("story.html", "<h1>Story</h1>"), ("a/b.js", "x()")]);
        assert_eq!(entry(&z, "a/b.js").unwrap().as_deref(), Some(&b"x()"[..]));
        for bad in ["../story.html", "/story.html", "a/../story.html", "a//b.js", "a\\b.js", "missing.html", "a/"] {
            assert_eq!(entry(&z, bad).unwrap(), None, "{bad}");
        }
        assert_eq!(content_type("A/B.HTML"), "text/html; charset=utf-8");
        assert_eq!(content_type("track.unknown"), "application/octet-stream");
    }

    #[test]
    fn what_the_package_says_about_the_lesson() {
        let o = outcome(
            Version::Scorm12,
            &json!({"core": {"lesson_status": "passed", "score": {"raw": "45", "min": "0", "max": "50"}, "total_time": "0001:02:03.50"}}),
        );
        assert_eq!(o, Outcome { status: "Passed", score: Some(Decimal::from(90)), seconds: Some(3723), done: true });
        let o = outcome(Version::Scorm12, &json!({"core": {"lesson_status": "incomplete", "score": {"raw": ""}}}));
        assert_eq!((o.status, o.score, o.done), ("Incomplete", None, false));
        let o = outcome(Version::Scorm2004, &json!({"completion_status": "completed", "success_status": "failed", "score": {"scaled": "0.4"}}));
        assert_eq!((o.status, o.score, o.done), ("Failed", Some(Decimal::from(40)), false), "completed but failed is not done");
        let o = outcome(Version::Scorm2004, &json!({"completion_status": "incomplete", "success_status": "passed", "total_time": "PT1H30M5.5S"}));
        assert_eq!((o.status, o.seconds, o.done), ("Passed", Some(5405), true));
        let o = outcome(Version::Scorm2004, &json!({"completion_status": "completed", "success_status": "unknown", "score": {"raw": 7, "max": 10}}));
        assert_eq!((o.status, o.score, o.done), ("Completed", Some(Decimal::from(70)), true));
        assert_eq!(outcome(Version::Scorm2004, &json!({})).status, "Not Attempted");
    }

    #[test]
    fn times_in_both_spellings() {
        assert_eq!(seconds("0000:00:59.99"), Some(59));
        assert_eq!(seconds("12:30:00"), Some(45_000));
        assert_eq!(seconds("P1DT2H"), Some(93_600));
        assert_eq!(seconds("PT0S"), Some(0));
        for bad in ["", "1:2", "00:61:00", "PT1X", "PT5", "abc"] {
            assert_eq!(seconds(bad), None, "{bad}");
        }
    }
}
