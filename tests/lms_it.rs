//! Learn end to end against a real PostgreSQL (`FERRUM_LMS_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test`). The database is reset first.
//!
//! An instructor builds a course — chapters, lessons (an allowlisted video, a PDF, a quiz, an
//! assignment), a question bank and a timed quiz, a cohort whose sessions cross a daylight-saving
//! change. Learners enrol (draft, invitation-only and prerequisite refusals; a cohort's last place
//! raced by concurrent requests), work through lessons, sit the quiz (the attempt limit under
//! concurrency, the server's clock, exact scores, answers revealed only by the quiz's rule and
//! never before submission — checked through every generic-API path a learner has), hand in and
//! resubmit an assignment, and complete the course with one certificate (concurrent last steps)
//! that anyone verifies on the public site. Maintenance closes timed-out attempts, finishes
//! enrolments that course changes completed and reminds a cohort once; dashboards and reports show
//! the exact numbers; a second workspace with the Mail app gets the emails as `Communication`s.

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, Site, SiteRequest, SiteResponse};
use ferrum_meta::Catalog;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const PW: &str = "Correct-Horse-9";
const PDF: &[u8] = b"%PDF-1.4\n% Rust cheat sheet\n";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn n(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn dec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => Decimal::from_str(s).unwrap_or_else(|_| panic!("not a decimal: {s}")),
        Value::Number(x) => Decimal::from_str(&x.to_string()).unwrap(),
        Value::Null => Decimal::ZERO,
        other => panic!("not a decimal: {other}"),
    }
}

fn uid(doc: &Doc) -> Uuid {
    doc["id"].as_str().unwrap().parse().unwrap()
}

fn vid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Error {
    r.expect_err("expected the request to be refused")
}

fn field_error<T: std::fmt::Debug>(r: Result<T, Error>, field: &str) -> String {
    match r {
        Err(Error::Validation(f)) => f.iter().find(|x| x.field == field).map(|x| x.message.clone()).unwrap_or_else(|| panic!("no error on {field}: {f:?}")),
        other => panic!("expected a validation error on {field}, got {other:?}"),
    }
}

/// The key an attempt shows an option under.
fn key(attempt: &Value, question: Uuid, text: &str) -> String {
    let row = row(attempt, question);
    let opt = row["options"].as_array().unwrap().iter().find(|o| o["text"] == json!(text)).unwrap_or_else(|| panic!("no option {text}"));
    opt["key"].as_str().unwrap().to_string()
}

fn row(attempt: &Value, question: Uuid) -> &Value {
    attempt["answers"].as_array().unwrap().iter().find(|r| r["question"] == json!(question.to_string())).unwrap_or_else(|| panic!("no row for {question}"))
}

fn texts(v: &Value) -> BTreeSet<String> {
    v.as_array().unwrap_or_else(|| panic!("not a list: {v}")).iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

/// An empty schema. Dropping it whole takes a lock for every table at once, more than the database
/// allows when Comms and ERP are installed beside Learn: tables go in batches.
async fn reset(db: &Db) {
    loop {
        let names: Vec<String> =
            sqlx::query_scalar("select quote_ident(tablename) from pg_tables where schemaname = 'public' limit 40").fetch_all(&db.pool).await.expect("tables");
        if names.is_empty() {
            break;
        }
        let drop = format!("drop table if exists {} cascade", names.join(", "));
        sqlx::raw_sql(sqlx::AssertSqlSafe(drop)).execute(&db.pool).await.expect("drop tables");
    }
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
}

/// Uploaded files in memory (the server gives the kernel S3 or a disk).
struct MemFiles(Mutex<HashMap<String, Vec<u8>>>);

#[async_trait::async_trait]
impl ferrum_kernel::files::FileStore for MemFiles {
    async fn put(&self, key: &str, data: &[u8]) -> ferrum_kernel::Result<()> {
        self.0.lock().unwrap().insert(key.into(), data.to_vec());
        Ok(())
    }
    async fn get(&self, key: &str) -> ferrum_kernel::Result<Vec<u8>> {
        self.0.lock().unwrap().get(key).cloned().ok_or_else(|| Error::NotFound("file".into()))
    }
    async fn delete(&self, key: &str) -> ferrum_kernel::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// The Mail app's `Communication`, as far as Learn writes it (the real app delivers it; here the
/// test only checks what Learn queues).
fn mail_double() -> ferrum_meta::Package {
    let mut files = BTreeMap::new();
    files.insert("package.yaml".to_string(), "package: comms\nversion: 0.0.1\nlabel: Mail (test double)\ndepends: [core]\n".to_string());
    files.insert(
        "entities/mail.yaml".to_string(),
        "- name: Communication\n  label: Communication\n  kind: master\n  title_field: subject\n  fields:\n\
         \x20   - {name: subject, type: data, max_length: 998}\n\
         \x20   - {name: channel, type: select, options: [Email, SMS, Chat, Phone, Note, Meeting], default: Email, required: true}\n\
         \x20   - {name: direction, type: select, options: [Sent, Received], default: Sent, required: true}\n\
         \x20   - {name: recipients, type: text}\n\
         \x20   - {name: content, type: text}\n\
         \x20   - {name: text_content, type: text}\n\
         \x20   - {name: reference_entity, type: data}\n\
         \x20   - {name: reference_id, type: data}\n"
            .to_string(),
    );
    ferrum_meta::catalog::load_package_files(&files).expect("mail double")
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    site: Arc<dyn Site>,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn try_insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Result<Doc, Error> {
        self.k.insert(ctx, entity, obj(v)).await
    }
    async fn update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Doc {
        self.k.update(ctx, entity, id, obj(v), None).await.unwrap_or_else(|e| panic!("update {entity}: {e:?}"))
    }
    async fn try_update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Result<Doc, Error> {
        self.k.update(ctx, entity, id, obj(v), None).await
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn act(&self, ctx: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> Value {
        self.k.run_action(ctx, entity, id, action, params).await.unwrap_or_else(|e| panic!("{entity}.{action}: {e:?}"))
    }
    async fn try_act(&self, ctx: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> Result<Value, Error> {
        self.k.run_action(ctx, entity, id, action, params).await
    }
    async fn list(&self, ctx: &Ctx, entity: &str, filters: Value) -> Result<Vec<Doc>, Error> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        Ok(self.k.list(ctx, entity, &q).await?.rows)
    }
    async fn user(&self, email: &str, name: &str, role: &str) -> Ctx {
        let id = self.k.create_user(&self.admin, email, name, Some(PW), &[role.into()], false).await.expect("user");
        self.k.load_ctx(self.tenant, id, None, "it".into()).await.expect("ctx")
    }
    /// A count with `$1` = tenant, `$2` = an id and `$3` = a text.
    async fn count(&self, sql: &str, id: Uuid, text: &str) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).bind(text).fetch_one(&mut *tx).await.unwrap()
    }
    async fn sql(&self, sql: &str, id: Uuid) {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    /// In-app notifications of a user whose title starts with `prefix`.
    async fn notes(&self, user: &Ctx, prefix: &str) -> i64 {
        self.count("select count(*) from notification where tenant_id = $1 and user_id = $2 and title like $3 || '%'", user.user, prefix).await
    }
    async fn call(&self, user: Option<&Ctx>, method: &str, path: &str) -> Result<SiteResponse, Error> {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query: Map<String, Value> =
            query.split('&').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect();
        let req = SiteRequest {
            tenant: self.tenant,
            user: user.cloned(),
            method: method.into(),
            path: path.into(),
            query,
            headers: Map::new(),
            body: vec![],
            client_ip: "203.0.113.9".into(),
        };
        self.site.handle(&self.k, req).await
    }
    /// A request with a body and headers (`?` in `path` starts its query, values taken as they are).
    async fn send(&self, user: &Ctx, method: &str, path: &str, body: &[u8], headers: &[(&str, &str)]) -> Result<SiteResponse, Error> {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query: Map<String, Value> =
            query.split('&').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect();
        let req = SiteRequest {
            tenant: self.tenant,
            user: Some(user.clone()),
            method: method.into(),
            path: path.into(),
            query,
            headers: headers.iter().map(|(k, v)| (k.to_string(), json!(v))).collect(),
            body: body.to_vec(),
            client_ip: "203.0.113.9".into(),
        };
        self.site.handle(&self.k, req).await
    }
    async fn page(&self, user: Option<&Ctx>, path: &str) -> Value {
        let r = self.call(user, "GET", path).await.unwrap_or_else(|e| panic!("GET {path}: {e:?}"));
        assert_eq!(r.status, 200);
        assert_eq!(r.content_type, "application/json");
        serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("GET {path}: not JSON"))
    }
    async fn report(&self, ctx: &Ctx, name: &str, params: &[(&str, String)]) -> Value {
        let p: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        ferrum_domain::reports::run(&self.k, ctx, name, &p).await.unwrap_or_else(|e| panic!("report {name}: {e:?}"))
    }
    /// A chapter with lessons (articles unless given) in a course.
    async fn chapter(&self, ctx: &Ctx, course: Uuid, title: &str, lessons: &[Value]) -> (Uuid, Vec<Uuid>) {
        let ch = uid(&self.insert(ctx, "LmsChapter", json!({"course": course, "title": title})).await);
        let mut ids = vec![];
        for l in lessons {
            let mut l = l.as_object().cloned().unwrap();
            l.insert("chapter".into(), json!(ch.to_string()));
            l.entry("lesson_type").or_insert(json!("Article"));
            ids.push(uid(&self.insert(ctx, "LmsLesson", Value::Object(l)).await));
        }
        (ch, ids)
    }
}

/// `n` copies of a request at once; their results in order.
async fn race<F, Fut, R>(n: usize, f: F) -> Vec<Result<R, Error>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<R, Error>> + Send + 'static,
    R: Send + 'static,
{
    let handles: Vec<_> = (0..n).map(|_| tokio::spawn(f())).collect();
    let mut out = Vec::with_capacity(n);
    for h in handles {
        out.push(h.await.expect("task"));
    }
    out
}

/// Both scenarios reset the one test database.
static DB: Mutex<()> = Mutex::new(());

/// An answer as the server sends it: a stored file it names (`SiteResponse::file`) becomes its
/// body, as the server streams it from storage.
async fn served(k: &Kernel, tenant: Uuid, mut r: SiteResponse) -> SiteResponse {
    if let Some(i) = r.headers.iter().position(|(h, _)| h == ferrum_kernel::hooks::STREAM_FILE_HEADER) {
        let id: Uuid = r.headers.remove(i).1.parse().expect("a file id");
        assert!(r.body.is_empty(), "a named file is streamed, never read whole by the app");
        r.body = k.read_file(&Ctx::system(tenant, k.registry.schema(tenant).await.unwrap()), id).await.unwrap().2;
    }
    r
}

#[test]
fn learn_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_LMS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    reset(&db).await;
    let mut packages = ferrum_domain::packages().expect("platform packages");
    // Learn without its Commerce connector, which paid_courses_end_to_end tests.
    packages.extend(ferrum_app_lms::packages().expect("app packages").into_iter().filter(|p| p.name() != "lms_payments"));
    packages.push(mail_double());
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_lms::register(&mut hooks);
    for source in ferrum_app_lms::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    k.set_file_store(Arc::new(MemFiles(Mutex::new(HashMap::new()))));
    ferrum_domain::maintenance(&k).await.expect("domain maintenance");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "learn".into(),
            name: "Acme Academy".into(),
            admin_email: "admin@learn.test".into(),
            admin_password: PW.into(),
            admin_name: None,
            packages: vec!["lms".into()],
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Europe/Berlin"})),
        })
        .await
        .expect("tenant");
    let session = k.login("learn", "admin@learn.test", PW).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("lms").cloned().expect("site registered");
    let t = T { k: k.clone(), tenant, admin: admin.clone(), site: site.clone() };

    let manager = t.user("manager@learn.test", "Mona Manager", "LMS Manager").await;
    let teacher = t.user("teacher@learn.test", "Tara Teacher", "Instructor").await;
    let stranger = t.user("other.teacher@learn.test", "Omar Other", "Instructor").await;
    let asha = t.user("asha@learn.test", "Asha Rao", "Learner").await;
    let ben = t.user("ben@learn.test", "Ben Okafor", "Learner").await;
    let chen = t.user("chen@learn.test", "Chen Wei", "Learner").await;
    let mut racers = vec![];
    for (email, name) in [("dee@learn.test", "Dee Dunn"), ("eli@learn.test", "Eli Ezra"), ("fay@learn.test", "Fay Fox")] {
        racers.push(t.user(email, name, "Learner").await);
    }
    // Someone with no role in this workspace.
    let nobody = {
        let id = t.k.create_user(&t.admin, "nobody@learn.test", "No Body", Some(PW), &[], false).await.expect("user");
        t.k.load_ctx(t.tenant, id, None, "it".into()).await.expect("ctx")
    };
    let today = teacher.today();

    // ───────────── settings and catalog ─────────────
    assert!(field_error(t.try_insert(&manager, "LmsSettings", json!({"video_hosts": "not a host"})).await, "video_hosts").contains("host"));
    t.insert(
        &manager,
        "LmsSettings",
        json!({"public_url": "https://academy.example.com/", "certificate_issuer": "Acme Academy", "certificate_signatory": "Mona Manager",
               "certificate_signatory_title": "Head of Learning", "video_hosts": "videos.acme.test"}),
    )
    .await;
    let eng = uid(&t.insert(&manager, "LmsCategory", json!({"category_name": "Engineering"})).await);
    let general = t.list(&manager, "LmsCategory", json!({"category_name": "General"})).await.unwrap();
    assert_eq!(general.len(), 1, "the General category is seeded");
    let general = uid(&general[0]);

    let basics = t
        .insert(
            &teacher,
            "LmsCourse",
            json!({"title": "Rust Basics", "visibility": "Public", "category": eng, "tags": " Rust, SAFETY ,rust", "estimated_hours": "6.5",
                   "summary": "Ownership, borrowing and your first programs.", "issue_certificate": true,
                   "instructors": [{"instructor": "Teacher@Learn.test"}]}),
        )
        .await;
    let basics_id = uid(&basics);
    assert_eq!(basics["slug"], json!("rust-basics"));
    assert_eq!(basics["tags"], json!("rust, safety"));
    assert_eq!(basics["status"], json!("Draft"));
    assert_eq!(basics["instructors"][0]["instructor_name"], json!("Tara Teacher"));
    let ghost = t.try_insert(&teacher, "LmsCourse", json!({"title": "Ghost", "instructors": [{"instructor": "ghost@learn.test"}]})).await;
    assert!(field_error(ghost, "instructor").contains("no active user"));
    assert_eq!(t.insert(&teacher, "LmsCourse", json!({"title": "Rust Basics"})).await["slug"], json!("rust-basics-2"), "a free slug is chosen");
    assert!(field_error(t.try_insert(&teacher, "LmsCourse", json!({"title": "Other", "slug": "rust-basics"})).await, "slug").contains("already"));
    assert!(field_error(t.try_insert(&teacher, "LmsCourse", json!({"title": "Other", "slug": "Bad Slug"})).await, "slug").contains("lower-case"));
    let empty = t.try_update(&teacher, "LmsCourse", basics_id, json!({"status": "Published"})).await;
    assert!(field_error(empty, "status").contains("at least one lesson"));

    // ───────────── chapters and lessons ─────────────
    let ch1 = t.insert(&teacher, "LmsChapter", json!({"course": basics_id, "title": "Getting Started"})).await;
    let ch2 = t.insert(&teacher, "LmsChapter", json!({"course": basics_id, "title": "Ownership"})).await;
    assert_eq!((ch1["position"].clone(), ch2["position"].clone()), (json!(1), json!(2)));
    let (ch1, ch2) = (uid(&ch1), uid(&ch2));
    assert!(matches!(err(t.try_insert(&stranger, "LmsChapter", json!({"course": basics_id, "title": "Mine"})).await), Error::Forbidden(_)));
    let l1 = t
        .insert(
            &teacher,
            "LmsLesson",
            json!({"chapter": ch1, "title": "Welcome", "content": "# Welcome\nWhat we will build.", "is_preview": true, "duration_minutes": 5}),
        )
        .await;
    assert_eq!(l1["course"], json!(basics_id.to_string()));
    assert_eq!(l1["position"], json!(1));
    let l1 = uid(&l1);
    let bad_video = t
        .try_insert(
            &teacher,
            "LmsLesson",
            json!({"chapter": ch1, "title": "X", "lesson_type": "Video", "video_url": "https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ"}),
        )
        .await;
    assert!(field_error(bad_video, "video_url").contains("allowed video host"));
    let l2 = t
        .insert(
            &teacher,
            "LmsLesson",
            json!({"chapter": ch1, "title": "Installing Rust", "lesson_type": "Video", "video_url": "https://youtu.be/dQw4w9WgXcQ?t=30", "duration_minutes": 10, "content": "Follow along."}),
        )
        .await;
    assert_eq!(l2["video_embed_url"], json!("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30"));
    let l2 = uid(&l2);
    let town_hall = t
        .insert(
            &teacher,
            "LmsLesson",
            json!({"chapter": ch1, "title": "Town hall", "lesson_type": "Video", "video_url": "https://videos.acme.test/v/42", "required": false}),
        )
        .await;
    assert_eq!(town_hall["video_embed_url"], json!("https://videos.acme.test/v/42"), "a host allowed in Learn Settings embeds as is");
    k.delete(&teacher, "LmsLesson", uid(&town_hall), None).await.expect("a lesson nobody completed can be deleted");
    let pdf = k.store_file(&teacher, None, "cheatsheet.pdf", "application/pdf", PDF).await.expect("upload");
    let pdf_url = pdf["url"].as_str().unwrap().to_string();
    let missing_file = t
        .try_insert(&teacher, "LmsLesson", json!({"chapter": ch1, "title": "X", "lesson_type": "Document", "attachment": format!("/files/{}", Uuid::now_v7())}))
        .await;
    assert!(field_error(missing_file, "attachment").contains("no uploaded file"));
    let l3 = uid(&t
        .insert(&teacher, "LmsLesson", json!({"chapter": ch1, "title": "Cheat sheet", "lesson_type": "Document", "attachment": pdf_url, "required": false, "is_preview": true, "duration_minutes": 3}))
        .await);

    // ───────────── question bank, quiz, assignment ─────────────
    let q = |v: Value| {
        let mut v = v.as_object().cloned().unwrap();
        v.insert("course".into(), json!(basics_id.to_string()));
        Value::Object(v)
    };
    let qa = uid(&t
        .insert(
            &teacher,
            "LmsQuestion",
            q(json!({"question": "Which keyword declares an immutable binding?", "options": [{"option": "let", "is_correct": true}, {"option": "mut"}, {"option": "var"}, {"option": "const"}],
                     "explanation": "`let` bindings are immutable unless marked `mut`."})),
        )
        .await);
    let qb = uid(&t
        .insert(
            &teacher,
            "LmsQuestion",
            q(json!({"question": "Which of these types are Copy?", "question_type": "Multiple Choice", "points": 2,
                     "options": [{"option": "i32", "is_correct": true}, {"option": "String"}, {"option": "bool", "is_correct": true}, {"option": "Vec<u8>"}]})),
        )
        .await);
    let qc_doc = t
        .insert(&teacher, "LmsQuestion", q(json!({"question": "A value can have two owners at the same time.", "question_type": "True/False", "options": [{"option": "false", "is_correct": true}]})))
        .await;
    let tf: Vec<(String, bool)> =
        qc_doc["options"].as_array().unwrap().iter().map(|o| (o["option"].as_str().unwrap().into(), o["is_correct"].as_bool().unwrap())).collect();
    assert_eq!(tf, vec![("True".into(), false), ("False".into(), true)], "True/False options are made for the author");
    let qc = uid(&qc_doc);
    let qd = uid(&t
        .insert(
            &teacher,
            "LmsQuestion",
            q(json!({"question": "Name the tool that builds and tests Rust projects.", "question_type": "Short Answer", "accepted_answers": [{"answer": "Cargo"}],
                     "explanation": "Cargo is Rust's build tool and package manager."})),
        )
        .await);
    let two_right = t
        .try_insert(&teacher, "LmsQuestion", q(json!({"question": "?", "options": [{"option": "a", "is_correct": true}, {"option": "b", "is_correct": true}]})))
        .await;
    assert!(field_error(two_right, "options").contains("exactly one"));
    let none_right = t
        .try_insert(&teacher, "LmsQuestion", q(json!({"question": "?", "question_type": "Multiple Choice", "options": [{"option": "a"}, {"option": "b"}]})))
        .await;
    assert!(field_error(none_right, "options").contains("at least one"));
    let twins =
        t.try_insert(&teacher, "LmsQuestion", q(json!({"question": "?", "options": [{"option": "Let", "is_correct": true}, {"option": " let "}]}))).await;
    assert!(field_error(twins, "option").contains("read the same"));
    let short_opts = t.try_insert(&teacher, "LmsQuestion", q(json!({"question": "?", "question_type": "Short Answer", "options": [{"option": "a"}]}))).await;
    assert!(field_error(short_opts, "options").contains("accepted answers"));
    let dup_course = t.list(&teacher, "LmsCourse", json!({"slug": "rust-basics-2"})).await.unwrap();
    let foreign = uid(&t
        .insert(&teacher, "LmsQuestion", json!({"course": uid(&dup_course[0]), "question": "Elsewhere?", "question_type": "True/False", "options": [{"option": "True", "is_correct": true}]}))
        .await);
    assert!(matches!(
        err(t
            .try_insert(
                &stranger,
                "LmsQuestion",
                q(json!({"question": "?", "question_type": "True/False", "options": [{"option": "True", "is_correct": true}]}))
            )
            .await),
        Error::Forbidden(_)
    ));

    let quiz_rows = |extra: Value| {
        let mut rows =
            vec![json!({"question": qa}), json!({"question": qb}), json!({"question": qc, "points": "0.5"}), json!({"question": qd, "points": "1.5"})];
        if !extra.is_null() {
            rows.push(extra);
        }
        json!(rows)
    };
    let quiz_body = |questions: Value| {
        json!({"course": basics_id, "title": "Ownership quiz", "passing_percentage": 70, "max_attempts": 3, "time_limit_minutes": 10,
               "shuffle_options": true, "show_answers": "After Last Attempt", "questions": questions})
    };
    assert!(field_error(t.try_insert(&teacher, "LmsQuiz", quiz_body(quiz_rows(json!({"question": foreign})))).await, "question").contains("another course"));
    assert!(field_error(t.try_insert(&teacher, "LmsQuiz", quiz_body(quiz_rows(json!({"question": qa})))).await, "question").contains("twice"));
    let quiz = t.insert(&teacher, "LmsQuiz", quiz_body(quiz_rows(Value::Null))).await;
    assert_eq!(quiz["question_count"], json!(4));
    assert_eq!(dec(&quiz["total_points"]), n("5"));
    let points: Vec<Decimal> = quiz["questions"].as_array().unwrap().iter().map(|r| dec(&r["points"])).collect();
    assert_eq!(points, vec![n("1"), n("2"), n("0.5"), n("1.5")], "a row without points takes the question's");
    let quiz_id = uid(&quiz);
    let too_many = t.try_update(&teacher, "LmsQuiz", quiz_id, json!({"questions_per_attempt": 9})).await;
    assert!(field_error(too_many, "questions_per_attempt").contains("only 4"));

    let essay = t
        .insert(
            &teacher,
            "LmsAssignment",
            json!({"course": basics_id, "title": "Borrow checker essay", "instructions": "Explain borrowing in 300 words.", "max_score": 100, "passing_score": 60,
                   "due_days": 7, "max_submissions": 3}),
        )
        .await;
    let essay_id = uid(&essay);
    let both = t.try_insert(&teacher, "LmsAssignment", json!({"course": basics_id, "title": "X", "due_date": today.to_string(), "due_days": 3})).await;
    assert!(field_error(both, "one_deadline").contains("not both"));
    let no_quiz = t.try_insert(&teacher, "LmsLesson", json!({"chapter": ch2, "title": "Quiz without quiz", "lesson_type": "Quiz"})).await;
    assert!(field_error(no_quiz, "quiz").contains("required"));
    let l4 = uid(&t
        .insert(&teacher, "LmsLesson", json!({"chapter": ch2, "title": "Ownership quiz", "lesson_type": "Quiz", "quiz": quiz_id, "duration_minutes": 15}))
        .await);
    assert!(matches!(
        err(t.try_insert(&teacher, "LmsLesson", json!({"chapter": ch2, "title": "Same quiz", "lesson_type": "Quiz", "quiz": quiz_id})).await),
        Error::Conflict(_)
    ));
    let l5 = uid(&t
        .insert(
            &teacher,
            "LmsLesson",
            json!({"chapter": ch2, "title": "Borrow checker essay", "lesson_type": "Assignment", "assignment": essay_id, "duration_minutes": 30}),
        )
        .await);

    let basics = t.update(&teacher, "LmsCourse", basics_id, json!({"status": "Published"})).await;
    assert!(basics["published_at"].is_string());
    assert_eq!((basics["lesson_count"].clone(), basics["total_minutes"].clone()), (json!(5), json!(63)));
    assert!(t.try_update(&stranger, "LmsCourse", basics_id, json!({"summary": "Hijacked"})).await.is_err(), "only the course's instructors edit it");

    // Learners never read course content, the question bank or quizzes through the generic API.
    for entity in ["LmsLesson", "LmsChapter", "LmsQuiz", "LmsQuestion", "LmsAssignment", "LmsCohort"] {
        assert!(matches!(t.list(&asha, entity, json!([])).await, Err(Error::Forbidden(_))), "{entity}");
    }
    assert!(matches!(k.get(&asha, "LmsQuestion", qd, false).await, Err(Error::Forbidden(_))));

    // Other courses: one requiring Rust Basics, an invitation-only workshop, a public draft.
    let course = |title: &str, extra: Value| {
        let mut v = extra.as_object().cloned().unwrap_or_default();
        v.insert("title".into(), json!(title));
        v.insert("instructors".into(), json!([{"instructor": "teacher@learn.test"}]));
        Value::Object(v)
    };
    let advanced = uid(&t.insert(&teacher, "LmsCourse", course("Rust Advanced", json!({"prerequisites": [{"course": basics_id}]}))).await);
    let (_, adv_lessons) = t.chapter(&teacher, advanced, "Traits", &[json!({"title": "Traits in depth"})]).await;
    t.update(&teacher, "LmsCourse", advanced, json!({"status": "Published"})).await;
    let invite = uid(&t
        .insert(&teacher, "LmsCourse", course("Invite Only Workshop", json!({"visibility": "Public", "self_enrollment": false, "category": general})))
        .await);
    let (_, inv_lessons) = t.chapter(&teacher, invite, "Day one", &[json!({"title": "Morning"}), json!({"title": "Afternoon"})]).await;
    t.update(&teacher, "LmsCourse", invite, json!({"status": "Published"})).await;
    let draft = uid(&t.insert(&teacher, "LmsCourse", course("Public Draft", json!({"visibility": "Public"}))).await);
    t.chapter(&teacher, draft, "Soon", &[json!({"title": "Coming soon"})]).await;
    let cycle = t.try_update(&teacher, "LmsCourse", basics_id, json!({"prerequisites": [{"course": advanced}]})).await;
    assert!(field_error(cycle, "prerequisites").contains("cycle"));

    // ───────────── a cohort and its sessions across a daylight-saving change ─────────────
    let bad_zone = t
        .try_insert(
            &teacher,
            "LmsCohort",
            json!({"course": basics_id, "title": "X", "start_date": today.to_string(), "end_date": today.to_string(), "time_zone": "Mars/Olympus"}),
        )
        .await;
    assert!(field_error(bad_zone, "time_zone").contains("IANA"));
    let cohort = uid(&t
        .insert(
            &teacher,
            "LmsCohort",
            json!({"course": basics_id, "title": "Autumn cohort", "start_date": (today - chrono::Duration::days(7)).to_string(),
                   "end_date": (today + chrono::Duration::days(90)).to_string(), "time_zone": "Europe/Berlin", "capacity": 2,
                   "instructors": [{"instructor": "teacher@learn.test"}]}),
        )
        .await);
    let plan = t
        .act(
            &teacher,
            "LmsCohort",
            cohort,
            "schedule_sessions",
            json!({"first_date": "2027-03-23", "time": "18:00", "count": 2, "title": "Live Q&A", "join_url": "https://meet.acme.test/rust"}),
        )
        .await;
    assert_eq!(plan["time_zone"], json!("Europe/Berlin"));
    let starts: Vec<&str> = plan["scheduled"].as_array().unwrap().iter().map(|x| x["starts_at"].as_str().unwrap()).collect();
    assert_eq!(starts, vec!["2027-03-23T17:00:00.000Z", "2027-03-30T16:00:00.000Z"], "18:00 in Berlin on both sides of the clock change");
    assert_eq!(plan["scheduled"][1]["local_start"], json!("2027-03-30T18:00:00+02:00"));
    let stored = t.get("LmsCohort", cohort).await;
    let titles: Vec<&str> = stored["sessions"].as_array().unwrap().iter().map(|x| x["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["Live Q&A 1", "Live Q&A 2"]);
    let insecure = t
        .try_act(
            &teacher,
            "LmsCohort",
            cohort,
            "schedule_sessions",
            json!({"first_date": "2027-04-06", "time": "18:00", "join_url": "http://meet.acme.test/x"}),
        )
        .await;
    assert!(field_error(insecure, "join_url").contains("https"));
    assert!(matches!(
        err(t.try_act(&stranger, "LmsCohort", cohort, "schedule_sessions", json!({"first_date": "2027-04-06", "time": "18:00"})).await),
        Error::Forbidden(_)
    ));

    // ───────────── enrolling ─────────────
    let refused = err(t.try_insert(&asha, "LmsEnrollment", json!({"course": draft})).await);
    assert!(refused.to_string().contains("not open for enrolment"), "{refused}");
    let refused = err(t.try_insert(&ben, "LmsEnrollment", json!({"course": invite})).await);
    assert!(refused.to_string().contains("by invitation"), "{refused}");
    assert!(field_error(t.try_insert(&asha, "LmsEnrollment", json!({"course": advanced})).await, "course").contains("Rust Basics"));
    let ea = t.insert(&asha, "LmsEnrollment", json!({"course": basics_id})).await;
    assert_eq!(ea["learner"], json!("asha@learn.test"));
    assert_eq!(ea["learner_name"], json!("Asha Rao"));
    assert_eq!(ea["status"], json!("Active"));
    assert_eq!(ea["total_lessons"], json!(4), "the four required lessons");
    assert!(ea["number"].as_str().unwrap().starts_with("ENR-"));
    let ea = uid(&ea);
    assert_eq!(t.notes(&asha, "Enrolled: Rust Basics").await, 1);
    assert!(matches!(err(t.try_insert(&asha, "LmsEnrollment", json!({"course": basics_id})).await), Error::Conflict(_)), "one enrolment per course");
    assert!(matches!(err(t.try_insert(&ben, "LmsEnrollment", json!({"course": basics_id, "learner": "chen@learn.test"})).await), Error::Forbidden(_)));
    let eb = uid(&t.insert(&ben, "LmsEnrollment", json!({"course": basics_id, "cohort": cohort})).await);

    // Three learners race for the cohort's last place: exactly one gets it.
    let mut handles = vec![];
    for r in &racers {
        let (k, r) = (k.clone(), r.clone());
        let body = obj(json!({"course": basics_id, "cohort": cohort}));
        handles.push(tokio::spawn(async move { k.insert(&r, "LmsEnrollment", body).await }));
    }
    let mut outcomes = vec![];
    for h in handles {
        outcomes.push(h.await.unwrap());
    }
    assert_eq!(outcomes.iter().filter(|o| o.is_ok()).count(), 1, "{outcomes:?}");
    for o in outcomes.iter().filter_map(|o| o.as_ref().err()) {
        assert!(matches!(o, Error::Conflict(m) if m.contains("is full")), "{o:?}");
    }
    let winner_at = outcomes.iter().position(|o| o.is_ok()).unwrap();
    let winner = racers[winner_at].clone();
    let losers: Vec<Ctx> = racers.iter().enumerate().filter(|(i, _)| *i != winner_at).map(|(_, c)| c.clone()).collect();
    assert_eq!(t.count("select count(*) from e_lms_enrollment where tenant_id = $1 and cohort = $2 and $3 = $3", cohort, "").await, 2);

    let bulk =
        t.act(&teacher, "LmsCourse", basics_id, "enroll_learners", json!({"learners": ["chen@learn.test", "ASHA@learn.test", "ghost@learn.test"]})).await;
    assert_eq!(bulk["enrolled"].as_array().unwrap().len(), 1);
    assert_eq!(bulk["enrolled"][0]["learner"], json!("chen@learn.test"));
    assert_eq!(bulk["already_enrolled"][0]["learner"], json!("asha@learn.test"));
    assert!(bulk["skipped"][0]["reason"].as_str().unwrap().contains("no active user"), "{bulk}");
    let echen = vid(&bulk["enrolled"][0]["enrollment"]);
    assert_eq!(
        t.count("select count(*) from notification where tenant_id = $1 and user_id = $2 and body like $3", chen.user, "Tara Teacher enrolled you%").await,
        1
    );
    let full = t.act(&teacher, "LmsCourse", basics_id, "enroll_learners", json!({"learners": [losers[0].email.clone()], "cohort": cohort})).await;
    assert!(full["skipped"][0]["reason"].as_str().unwrap().contains("full"), "{full}");
    assert!(t.try_act(&stranger, "LmsCourse", basics_id, "enroll_learners", json!({"learners": ["dee@learn.test"]})).await.is_err());
    let invited = t.act(&teacher, "LmsCourse", invite, "enroll_learners", json!({"learners": ["ben@learn.test"]})).await;
    let einv = vid(&invited["enrolled"][0]["enrollment"]);

    // Learners see their own records only.
    let mine = t.list(&ben, "LmsEnrollment", json!([])).await.unwrap();
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().all(|e| e["learner"] == json!("ben@learn.test")));
    assert!(matches!(k.get(&ben, "LmsEnrollment", ea, false).await, Err(Error::NotFound(_))));
    assert!(matches!(err(t.try_act(&ben, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l1})).await), Error::NotFound(_)));

    // ───────────── progress ─────────────
    let p = t.act(&asha, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l1})).await;
    assert_eq!((dec(&p["progress_percent"]), p["completed_lessons"].clone(), p["total_lessons"].clone()), (n("25"), json!(1), json!(4)));
    assert_eq!(p["newly_completed"], json!(true));
    let again = t.act(&asha, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l1})).await;
    assert_eq!((dec(&again["progress_percent"]), again["newly_completed"].clone()), (n("25"), json!(false)), "completing twice changes nothing");
    let optional = t.act(&asha, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l3})).await;
    assert_eq!(dec(&optional["progress_percent"]), n("25"), "optional lessons do not count");
    let by_hand = err(t.try_act(&asha, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l4})).await);
    assert!(by_hand.to_string().contains("passing the quiz"), "{by_hand}");
    assert!(
        field_error(t.try_act(&asha, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": adv_lessons[0]})).await, "lesson")
            .contains("not a lesson of this course")
    );
    let outline = t.act(&asha, "LmsEnrollment", ea, "outline", json!({})).await;
    assert_eq!(outline["next_lesson"]["id"], json!(l2.to_string()));
    assert_eq!(outline["chapters"][0]["lessons"][0]["completed"], json!(true));
    assert_eq!(outline["chapters"][1]["lessons"][0]["lesson_type"], json!("Quiz"));
    let opened = t.act(&asha, "LmsEnrollment", ea, "open_lesson", json!({"lesson": l3})).await;
    assert_eq!(opened["lesson"]["file_url"], json!(format!("/api/app/lms/lessons/{l3}/file")));
    assert_eq!(opened["completed"], json!(true));
    assert_eq!(t.get("LmsEnrollment", ea).await["last_lesson"], json!(l3.to_string()), "opening a lesson makes it the resume point");

    // ───────────── quiz attempts ─────────────
    let started = t.act(&asha, "LmsEnrollment", ea, "start_quiz", json!({"quiz": quiz_id})).await;
    assert_eq!(started["resumed"], json!(false));
    let a1 = started["attempt"].clone();
    let a1_id = vid(&a1["id"]);
    assert_eq!((a1["attempt_no"].clone(), a1["status"].clone()), (json!(1), json!("In Progress")));
    assert_eq!(a1["answers"].as_array().unwrap().len(), 4);
    assert_eq!(dec(&a1["max_score"]), n("5"));
    let left = a1["seconds_left"].as_i64().unwrap();
    assert!((590..=600).contains(&left), "{left}");
    let resumed = t.act(&asha, "LmsEnrollment", ea, "start_quiz", json!({"quiz": quiz_id})).await;
    assert_eq!((resumed["resumed"].clone(), resumed["attempt"]["id"].clone()), (json!(true), a1["id"].clone()), "starting again resumes");

    // Answer leak: nothing a learner can reach tells the right answers before submitting.
    let mut option_ids = vec![];
    for question in [qa, qb, qc] {
        for o in k.get(&teacher, "LmsQuestion", question, false).await.unwrap()["options"].as_array().unwrap() {
            option_ids.push(o["id"].as_str().unwrap().to_string());
        }
    }
    let leaks = |what: &str, v: &Value| {
        let text = v.to_string();
        assert!(!text.contains("Cargo"), "{what} reveals the accepted answer: {text}");
        assert!(!text.contains("immutable unless"), "{what} reveals an explanation");
        assert!(!text.contains("\"is_correct\":true"), "{what} marks a correct option");
        for id in &option_ids {
            assert!(!text.contains(id.as_str()), "{what} exposes answer-key option ids");
        }
    };
    leaks("start_quiz", &started);
    leaks("get", &Value::Object(k.get(&asha, "LmsQuizAttempt", a1_id, true).await.unwrap()));
    leaks("list", &json!(t.list(&asha, "LmsQuizAttempt", json!([])).await.unwrap()));
    leaks("audit trail", &k.audit_trail(&asha, "LmsQuizAttempt", a1_id).await.unwrap());
    leaks("print", &json!(k.render_print(&asha, "LmsQuizAttempt", a1_id).await.unwrap()));
    leaks("preview", &Value::Object(k.preview(&asha, "LmsQuizAttempt", obj(json!({"quiz": quiz_id}))).await.unwrap()));
    leaks("lesson page", &t.page(Some(&asha), &format!("/lessons/{l4}")).await);
    assert!(t.list(&asha, "LmsQuizAttempt", json!([["answers.is_correct", "=", true]])).await.unwrap().is_empty(), "filters cannot probe the key either");
    assert!(matches!(k.audit_trail(&asha, "LmsQuestion", qd).await, Err(Error::Forbidden(_))), "nor the question bank's history");

    let saved = t
        .act(&asha, "LmsQuizAttempt", a1_id, "save_answers", json!({"answers": {qa.to_string(): key(&a1, qa, "mut"), qc.to_string(): key(&a1, qc, "False")}}))
        .await;
    assert_eq!(row(&saved["attempt"], qa)["response"], json!([key(&a1, qa, "mut")]));
    assert!(
        field_error(t.try_act(&asha, "LmsQuizAttempt", a1_id, "save_answers", json!({"answers": {qa.to_string(): "not-a-key"}})).await, "answers")
            .contains("not one of")
    );
    assert!(
        matches!(err(t.try_act(&ben, "LmsQuizAttempt", a1_id, "submit_answers", json!({})).await), Error::NotFound(_)),
        "other learners cannot even see it"
    );
    let r1 =
        t.act(&asha, "LmsQuizAttempt", a1_id, "submit_answers", json!({"answers": {qb.to_string(): [key(&a1, qb, "i32")], qd.to_string(): "Cargo"}})).await;
    assert_eq!((r1["status"].clone(), dec(&r1["score"]), dec(&r1["percentage"]), r1["passed"].clone()), (json!("Submitted"), n("2"), n("40"), json!(false)));
    let done = &r1["attempt"];
    let graded: Vec<(bool, Decimal)> =
        [qa, qb, qc, qd].iter().map(|q| (row(done, *q)["is_correct"].as_bool().unwrap(), dec(&row(done, *q)["points_awarded"]))).collect();
    assert_eq!(graded, vec![(false, n("0")), (false, n("0")), (true, n("0.5")), (true, n("1.5"))], "no partial credit for half the right options");
    for q in [qa, qb, qc, qd] {
        assert!(row(done, q)["correct_answer"].is_null(), "answers stay hidden until the last attempt or a pass");
    }
    assert!(t.try_act(&asha, "LmsQuizAttempt", a1_id, "save_answers", json!({})).await.is_err(), "a submitted attempt takes no answers");
    let review = t.act(&teacher, "LmsQuizAttempt", a1_id, "review", json!({})).await;
    let qd_review = review["answers"].as_array().unwrap().iter().find(|r| r["question"] == json!(qd.to_string())).unwrap();
    assert_eq!(qd_review["correct_answer"], json!(["Cargo"]));
    assert!(matches!(err(t.try_act(&asha, "LmsQuizAttempt", a1_id, "review", json!({})).await), Error::Forbidden(_)));
    assert!(matches!(err(t.try_update(&asha, "LmsQuizAttempt", a1_id, json!({"score": "5"})).await), Error::Forbidden(_)));
    assert!(t.try_update(&teacher, "LmsQuizAttempt", a1_id, json!({})).await.unwrap_err().to_string().contains("start_quiz"));

    // Attempt 2 runs out of time: what was saved in time counts, the late answers do not.
    let a2 = t.act(&asha, "LmsEnrollment", ea, "start_quiz", json!({"quiz": quiz_id})).await["attempt"].clone();
    let a2_id = vid(&a2["id"]);
    assert_eq!(a2["attempt_no"], json!(2));
    t.act(&asha, "LmsQuizAttempt", a2_id, "save_answers", json!({"answers": {qa.to_string(): key(&a2, qa, "let")}})).await;
    t.sql(
        "update e_lms_quiz_attempt set started_at = now() - interval '12 minutes', deadline = now() - interval '2 minutes' where tenant_id = $1 and id = $2",
        a2_id,
    )
    .await;
    let all_right = json!({"answers": {qa.to_string(): key(&a2, qa, "let"), qb.to_string(): [key(&a2, qb, "i32"), key(&a2, qb, "bool")],
                                       qc.to_string(): key(&a2, qc, "False"), qd.to_string(): "cargo"}});
    let r2 = t.act(&asha, "LmsQuizAttempt", a2_id, "submit_answers", all_right).await;
    assert_eq!(
        (r2["status"].clone(), r2["late_answers_ignored"].clone(), dec(&r2["score"]), dec(&r2["percentage"])),
        (json!("Timed Out"), json!(true), n("1"), n("20"))
    );

    // Attempt 3 passes exactly on the mark (3.5 of 5 = 70%) and, being a pass, reveals the answers.
    let a3 = t.act(&asha, "LmsEnrollment", ea, "start_quiz", json!({"quiz": quiz_id})).await["attempt"].clone();
    let a3_id = vid(&a3["id"]);
    let r3 = t
        .act(
            &asha,
            "LmsQuizAttempt",
            a3_id,
            "submit_answers",
            json!({"answers": {qa.to_string(): key(&a3, qa, "mut"), qb.to_string(): [key(&a3, qb, "bool"), key(&a3, qb, "i32")], qc.to_string(): key(&a3, qc, "True"), qd.to_string(): "  CARGO  "}}),
        )
        .await;
    assert_eq!((dec(&r3["score"]), dec(&r3["percentage"]), r3["passed"].clone()), (n("3.5"), n("70"), json!(true)));
    let done = &r3["attempt"];
    assert_eq!(texts(&row(done, qa)["correct_answer"]), BTreeSet::from(["let".to_string()]));
    assert_eq!(texts(&row(done, qb)["correct_answer"]), BTreeSet::from(["i32".to_string(), "bool".to_string()]));
    assert_eq!(texts(&row(done, qc)["correct_answer"]), BTreeSet::from(["False".to_string()]));
    assert_eq!(texts(&row(done, qd)["correct_answer"]), BTreeSet::from(["Cargo".to_string()]));
    assert!(row(done, qa)["explanation"].as_str().unwrap().contains("immutable"));
    let used_up = err(t.try_act(&asha, "LmsEnrollment", ea, "start_quiz", json!({"quiz": quiz_id})).await);
    assert!(used_up.to_string().contains("all 3 attempts"), "{used_up}");
    assert_eq!(dec(&t.get("LmsEnrollment", ea).await["progress_percent"]), n("50"), "passing the quiz completed its lesson");

    // Ben's attempt times out with nobody there: maintenance closes it with his saved answers.
    let b1 = t.act(&ben, "LmsEnrollment", eb, "start_quiz", json!({"quiz": quiz_id})).await["attempt"].clone();
    let b1_id = vid(&b1["id"]);
    t.act(
        &ben,
        "LmsQuizAttempt",
        b1_id,
        "save_answers",
        json!({"answers": {qa.to_string(): key(&b1, qa, "let"), qb.to_string(): [key(&b1, qb, "i32"), key(&b1, qb, "bool")]}}),
    )
    .await;
    t.sql("update e_lms_quiz_attempt set deadline = now() - interval '1 minute' where tenant_id = $1 and id = $2", b1_id).await;
    ferrum_app_lms::maintenance(&k).await.expect("maintenance");
    let b1 = t.get("LmsQuizAttempt", b1_id).await;
    assert_eq!((b1["status"].clone(), dec(&b1["score"]), dec(&b1["percentage"]), b1["passed"].clone()), (json!("Timed Out"), n("3"), n("60"), json!(false)));

    // Chen hammers the quiz: concurrent starts share one attempt, and the limit of 3 holds.
    let start_chen = || {
        let (k, ctx) = (k.clone(), chen.clone());
        async move { k.run_action(&ctx, "LmsEnrollment", echen, "start_quiz", json!({"quiz": quiz_id})).await }
    };
    let submit_chen = |attempt: Uuid| {
        let (k, ctx) = (k.clone(), chen.clone());
        async move { k.run_action(&ctx, "LmsQuizAttempt", attempt, "submit_answers", json!({})).await.expect("submit") }
    };
    let round = race(6, start_chen).await;
    let ids: BTreeSet<String> = round.iter().map(|r| r.as_ref().expect("start")["attempt"]["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 1, "six concurrent starts, one attempt");
    assert_eq!(round.iter().filter(|r| r.as_ref().unwrap()["resumed"] == json!(false)).count(), 1);
    submit_chen(ids.iter().next().unwrap().parse().unwrap()).await;
    let second = start_chen().await.expect("second attempt");
    submit_chen(vid(&second["attempt"]["id"])).await;
    let round = race(6, start_chen).await;
    let ids: BTreeSet<String> = round.iter().map(|r| r.as_ref().expect("start")["attempt"]["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 1);
    submit_chen(ids.iter().next().unwrap().parse().unwrap()).await;
    let round = race(6, start_chen).await;
    assert!(round.iter().all(|r| r.as_ref().is_err_and(|e| e.to_string().contains("all 3 attempts"))), "{round:?}");
    assert_eq!(t.count("select count(*) from e_lms_quiz_attempt where tenant_id = $1 and enrollment = $2 and $3 = $3", echen, "").await, 3);

    // ───────────── the assignment ─────────────
    let sub1 = t.insert(&asha, "LmsAssignmentSubmission", json!({"assignment": essay_id, "answer": "Ownership moves values; borrowing lends them."})).await;
    assert_eq!((sub1["status"].clone(), sub1["submission_no"].clone(), sub1["late"].clone()), (json!("Submitted"), json!(1), json!(false)));
    assert_eq!(sub1["due_date"], json!((today + chrono::Duration::days(7)).to_string()));
    let sub1 = uid(&sub1);
    assert_eq!(t.notes(&teacher, "New submission: Borrow checker essay").await, 1);
    let pending = err(t.try_insert(&asha, "LmsAssignmentSubmission", json!({"assignment": essay_id, "answer": "Again"})).await);
    assert!(pending.to_string().contains("waiting to be graded"), "{pending}");
    t.update(&asha, "LmsAssignmentSubmission", sub1, json!({"answer": "Ownership moves values; borrowing lends them temporarily."})).await;
    assert!(
        field_error(t.try_update(&asha, "LmsAssignmentSubmission", sub1, json!({"attachment": pdf_url})).await, "attachment").contains("you uploaded yourself")
    );
    let mine_file = k.store_file(&asha, None, "essay.pdf", "application/pdf", b"%PDF-1.4 essay").await.unwrap();
    let with_file = t.update(&asha, "LmsAssignmentSubmission", sub1, json!({"attachment": mine_file["url"]})).await;
    assert_eq!(with_file["attachment"], mine_file["url"]);
    assert!(k.read_file(&teacher, vid(&mine_file["id"])).await.is_ok(), "the file follows the submission: its instructors can read it");
    assert!(matches!(err(t.try_act(&stranger, "LmsAssignmentSubmission", sub1, "grade", json!({"score": 90})).await), Error::Forbidden(_)));
    assert!(matches!(err(t.try_act(&asha, "LmsAssignmentSubmission", sub1, "grade", json!({"score": 100})).await), Error::Forbidden(_)));
    assert!(field_error(t.try_act(&teacher, "LmsAssignmentSubmission", sub1, "grade", json!({"score": 101})).await, "score").contains("between 0 and 100"));
    // What the record pages can give, these actions ask for (a page's dialog sends the score as text).
    let asks = |entity: &str, name: &str| -> Vec<String> {
        let actions = &teacher.schema.entity(entity).unwrap().def.actions;
        actions.iter().find(|a| a.name == name).unwrap().params.iter().map(|p| p.name.clone()).collect()
    };
    assert_eq!(asks("LmsAssignmentSubmission", "grade"), ["outcome", "score", "feedback"]);
    assert_eq!(asks("LmsCertificate", "revoke"), ["reason"]);
    assert_eq!(asks("LmsCourse", "enroll_learners"), ["learners", "cohort"]);
    assert_eq!(asks("LmsCohort", "schedule_sessions"), ["first_date", "time", "count", "every_days", "duration_minutes", "title", "join_url"]);
    let g1 = t.act(&teacher, "LmsAssignmentSubmission", sub1, "grade", json!({"score": "40", "feedback": "Explain lifetimes too."})).await;
    assert_eq!(
        (g1["submission"]["status"].clone(), g1["submission"]["passed"].clone(), dec(&g1["submission"]["score"])),
        (json!("Graded"), json!(false), n("40"))
    );
    assert_eq!(g1["submission"]["graded_by"], json!("teacher@learn.test"));
    assert_eq!(t.notes(&asha, "Assignment graded: Borrow checker essay").await, 1);
    assert!(t.try_update(&asha, "LmsAssignmentSubmission", sub1, json!({"answer": "sneaky"})).await.unwrap_err().to_string().contains("graded"));
    let sub2 = t.insert(&asha, "LmsAssignmentSubmission", json!({"assignment": essay_id, "answer": "Now with lifetimes."})).await;
    assert_eq!(sub2["submission_no"], json!(2));
    let g2 = t.act(&teacher, "LmsAssignmentSubmission", uid(&sub2), "grade", json!({"outcome": "Needs Revision", "feedback": "Add an example."})).await;
    assert_eq!((g2["submission"]["status"].clone(), g2["submission"]["score"].clone()), (json!("Needs Revision"), Value::Null));
    let sub3 = uid(&t.insert(&asha, "LmsAssignmentSubmission", json!({"assignment": essay_id, "answer": "Lifetimes, with an example."})).await);
    let g3 = t.act(&teacher, "LmsAssignmentSubmission", sub3, "grade", json!({"score": "85.5", "feedback": "Great work."})).await;
    assert_eq!((g3["submission"]["passed"].clone(), dec(&g3["submission"]["score"])), (json!(true), n("85.5")));
    assert_eq!(t.notes(&asha, "Assignment graded").await, 3);
    let after = err(t.try_insert(&asha, "LmsAssignmentSubmission", json!({"assignment": essay_id, "answer": "More"})).await);
    assert!(after.to_string().contains("already passed"), "{after}");
    assert_eq!(dec(&t.get("LmsEnrollment", ea).await["progress_percent"]), n("75"), "the passing submission completed its lesson");

    // ───────────── completion and one certificate, whatever races ─────────────
    let finish = || {
        let (k, ctx) = (k.clone(), asha.clone());
        async move { k.run_action(&ctx, "LmsEnrollment", ea, "complete_lesson", json!({"lesson": l2})).await }
    };
    assert!(race(5, finish).await.iter().all(|r| r.is_ok()));
    let done = t.get("LmsEnrollment", ea).await;
    assert_eq!((done["status"].clone(), dec(&done["progress_percent"])), (json!("Completed"), n("100")));
    assert!(done["completed_at"].is_string());
    assert_eq!(
        t.count("select count(*) from e_lms_certificate where tenant_id = $1 and enrollment = $2 and $3 = $3", ea, "").await,
        1,
        "exactly one certificate"
    );
    let cert_id = vid(&done["certificate"]);
    let cert = t.get("LmsCertificate", cert_id).await;
    let number = cert["number"].as_str().unwrap().to_string();
    assert_eq!(number, format!("CERT-{}-000001", chrono::Datelike::year(&today)));
    let code = cert["verification_code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 39);
    assert_eq!((cert["learner_name"].clone(), cert["course_title"].clone(), cert["status"].clone()), (json!("Asha Rao"), json!("Rust Basics"), json!("Valid")));
    assert_eq!((cert["issuer"].clone(), cert["signatory"].clone()), (json!("Acme Academy"), json!("Mona Manager")));
    assert_eq!(cert["verification_url"], json!(format!("https://academy.example.com/api/site/learn/lms/certificates/{code}")));
    assert_eq!(t.notes(&asha, "Certificate issued: Rust Basics").await, 1);
    assert!(
        t.try_insert(&manager, "LmsCertificate", json!({"enrollment": ea, "course": basics_id, "learner": "asha@learn.test"})).await.is_err(),
        "certificates are never typed in"
    );
    let printed = k.render_print(&asha, "LmsCertificate", cert_id).await.expect("learner prints her certificate");
    assert!(printed.contains("Certificate of Completion") && printed.contains("Asha Rao") && printed.contains(&code));
    assert!(matches!(k.get(&ben, "LmsCertificate", cert_id, false).await, Err(Error::NotFound(_))));

    // The prerequisite is met now; Chen's is not.
    t.insert(&asha, "LmsEnrollment", json!({"course": advanced})).await;
    assert!(field_error(t.try_insert(&chen, "LmsEnrollment", json!({"course": advanced})).await, "course").contains("Rust Basics"));
    let path = uid(&t
        .insert(&teacher, "LmsLearningPath", json!({"title": "Rustacean", "status": "Published", "courses": [{"course": basics_id}, {"course": advanced}]}))
        .await);
    assert!(
        field_error(
            t.try_insert(&teacher, "LmsLearningPath", json!({"title": "Twice", "courses": [{"course": basics_id}, {"course": basics_id}]})).await,
            "course"
        )
        .contains("twice")
    );
    let joined = t.act(&chen, "LmsLearningPath", path, "join", json!({})).await;
    assert_eq!(joined["already_enrolled"].as_array().unwrap().len(), 1);
    assert!(joined["skipped"][0]["reason"].as_str().unwrap().contains("Rust Basics"), "{joined}");

    // Course changes move progress: a new required lesson, then gone again.
    let l6 = t.chapter(&teacher, basics_id, "Wrap-up", &[json!({"title": "Next steps"})]).await;
    let moved = t.get("LmsEnrollment", ea).await;
    assert_eq!((dec(&moved["progress_percent"]), moved["total_lessons"].clone(), moved["status"].clone()), (n("80"), json!(5), json!("Completed")));
    assert!(k.delete(&teacher, "LmsLesson", l3, None).await.unwrap_err().to_string().contains("make it optional"));
    k.delete(&teacher, "LmsLesson", l6.1[0], None).await.expect("delete the new lesson");
    k.delete(&teacher, "LmsChapter", l6.0, None).await.expect("and its chapter");
    assert_eq!(dec(&t.get("LmsEnrollment", ea).await["progress_percent"]), n("100"));

    // A change that completes someone: maintenance finishes the enrolment.
    t.act(&ben, "LmsEnrollment", einv, "complete_lesson", json!({"lesson": inv_lessons[0]})).await;
    t.update(&teacher, "LmsLesson", inv_lessons[1], json!({"required": false})).await;
    let ready = t.get("LmsEnrollment", einv).await;
    assert_eq!((dec(&ready["progress_percent"]), ready["status"].clone()), (n("100"), json!("Active")));
    ferrum_app_lms::maintenance(&k).await.expect("maintenance");
    let finished = t.get("LmsEnrollment", einv).await;
    assert_eq!((finished["status"].clone(), finished["certificate"].clone()), (json!("Completed"), Value::Null), "this course gives no certificate");
    assert_eq!(t.notes(&ben, "Course completed: Invite Only Workshop").await, 1);

    // Withdrawing frees the cohort place, which someone else takes.
    t.act(&ben, "LmsEnrollment", eb, "withdraw", json!({})).await;
    assert!(t.try_act(&ben, "LmsEnrollment", eb, "complete_lesson", json!({"lesson": l1})).await.is_err(), "withdrawn: no progress");
    let late_comer = losers[0].clone();
    t.insert(&late_comer, "LmsEnrollment", json!({"course": basics_id, "cohort": cohort})).await;
    let back = err(t.try_act(&ben, "LmsEnrollment", eb, "reactivate", json!({})).await);
    assert!(matches!(&back, Error::Conflict(m) if m.contains("full")), "{back:?}");

    // ───────────── reminders: once per session, to the cohort and its instructors ─────────────
    let mut sessions = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
    let soon = chrono::Utc::now() + chrono::Duration::minutes(30);
    let later = chrono::Utc::now() + chrono::Duration::hours(3);
    sessions.push(json!({"title": "Kick-off", "starts_at": soon.to_rfc3339(), "duration_minutes": 45, "join_url": "https://meet.acme.test/kickoff"}));
    sessions.push(json!({"title": "Later", "starts_at": later.to_rfc3339()}));
    t.update(&teacher, "LmsCohort", cohort, json!({"sessions": sessions})).await;
    let (x, y) = tokio::join!(ferrum_app_lms::maintenance(&k), ferrum_app_lms::maintenance(&k));
    x.expect("maintenance");
    y.expect("maintenance");
    ferrum_app_lms::maintenance(&k).await.expect("maintenance");
    for (who, expected) in [(&winner, 1), (&late_comer, 1), (&teacher, 1), (&ben, 0), (&asha, 0)] {
        assert_eq!(t.notes(who, "Live session soon: Kick-off").await, expected, "{}", who.email);
    }
    assert_eq!(t.notes(&winner, "Live session soon: Later").await, 0, "outside the reminder window");
    let body = t
        .count(
            "select count(*) from notification where tenant_id = $1 and user_id = $2 and body like $3",
            winner.user,
            "%Join: https://meet.acme.test/kickoff%",
        )
        .await;
    assert_eq!(body, 1);

    // ───────────── signed-in pages ─────────────
    assert!(matches!(err(t.call(Some(&nobody), "GET", "/catalog").await), Error::Forbidden(_)));
    let cat = t.page(Some(&chen), "/catalog").await;
    let titles: BTreeSet<String> = cat["courses"].as_array().unwrap().iter().map(|c| c["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(titles, BTreeSet::from(["Rust Basics".into(), "Rust Advanced".into(), "Invite Only Workshop".into()]));
    let mine = cat["courses"].as_array().unwrap().iter().find(|c| c["title"] == json!("Rust Basics")).unwrap();
    assert_eq!(mine["my_enrollment"]["status"], json!("Active"));
    assert_eq!(cat["learning_paths"][0]["title"], json!("Rustacean"));
    let page = t.page(Some(&chen), "/courses/rust-basics").await["course"].clone();
    assert_eq!(page["next_lesson"]["id"], json!(l1.to_string()));
    assert_eq!(page["can_enroll"], json!(false));
    assert!(page["upcoming_sessions"].as_array().unwrap().is_empty(), "Chen is in no cohort");
    let adv = t.page(Some(&chen), "/courses/rust-advanced").await["course"].clone();
    assert!(adv["enroll_blockers"][0].as_str().unwrap().contains("Rust Basics"));
    assert!(matches!(err(t.call(Some(&chen), "GET", "/courses/public-draft").await), Error::NotFound(_)));
    assert_eq!(t.page(Some(&teacher), "/courses/public-draft").await["course"]["is_author"], json!(true));
    let outsider = &losers[1];
    assert!(matches!(err(t.call(Some(outsider), "GET", &format!("/lessons/{l2}")).await), Error::NotFound(_)), "not enrolled, not a preview");
    assert_eq!(t.page(Some(outsider), &format!("/lessons/{l1}")).await["lesson"]["content"], json!("# Welcome\nWhat we will build."));
    let quiz_state = t.page(Some(&chen), &format!("/lessons/{l4}")).await["quiz"].clone();
    assert_eq!((quiz_state["attempts_used"].clone(), quiz_state["attempts_left"].clone(), quiz_state["passed"].clone()), (json!(3), json!(0), json!(false)));
    let file = served(&t.k, t.tenant, t.call(Some(&chen), "GET", &format!("/lessons/{l3}/file")).await.expect("lesson file")).await;
    assert_eq!((file.content_type.as_str(), file.body.as_slice()), ("application/pdf", PDF));

    t.sql("update e_lms_enrollment set enrolled_on = enrolled_on - 30 where tenant_id = $1 and id = $2", echen).await;
    let dash = t.page(Some(&chen), "/dashboard").await;
    let owed = &dash["due_assignments"][0];
    assert_eq!((owed["title"].clone(), owed["status"].clone(), owed["overdue"].clone()), (json!("Borrow checker essay"), json!("Not Submitted"), json!(true)));
    assert_eq!(owed["due_date"], json!((today - chrono::Duration::days(23)).to_string()));
    assert_eq!(t.page(Some(&chen), &format!("/lessons/{l5}")).await["assignment"]["can_submit"], json!(true));
    let dash = t.page(Some(&asha), "/dashboard").await;
    assert_eq!(dash["courses"].as_array().unwrap().len(), 2);
    assert_eq!(dash["courses"][1]["certificate"]["number"], json!(number), "completed courses come last");
    assert_eq!(dash["certificates"].as_array().unwrap().len(), 1);
    assert_eq!((dash["learning_paths"][0]["completed"].clone(), dec(&dash["learning_paths"][0]["progress_percent"])), (json!(1), n("50")));
    let sessions = t.page(Some(&winner), "/dashboard").await["upcoming_sessions"].clone();
    let kickoff = sessions.as_array().unwrap().iter().find(|s| s["title"] == json!("Kick-off")).expect("the cohort's next session");
    assert_eq!(kickoff["join_url"], json!("https://meet.acme.test/kickoff"));
    assert_eq!(kickoff["time_zone"], json!("Europe/Berlin"));

    let teaching = t.page(Some(&teacher), "/teaching").await;
    let b = teaching["courses"].as_array().unwrap().iter().find(|c| c["title"] == json!("Rust Basics") && c["status"] == json!("Published")).unwrap();
    assert_eq!((b["enrolled"].clone(), b["completed"].clone(), dec(&b["completion_rate"])), (json!(4), json!(1), n("25")));
    let roster = t.page(Some(&teacher), &format!("/teaching/courses/{basics_id}/roster")).await;
    assert_eq!((roster["enrolled"].clone(), roster["learners"].as_array().unwrap().len()), (json!(4), 5));
    let asha_row = roster["learners"].as_array().unwrap().iter().find(|r| r["learner"] == json!("asha@learn.test")).unwrap();
    assert_eq!((dec(&asha_row["progress_percent"]), asha_row["certificate"].clone(), asha_row["quizzes_passed"].clone()), (n("100"), json!(number), json!(1)));
    assert!(matches!(err(t.call(Some(&stranger), "GET", &format!("/teaching/courses/{basics_id}/roster")).await), Error::Forbidden(_)));

    // ───────────── the public site ─────────────
    let public = t.page(None, "/courses").await;
    let titles: BTreeSet<String> = public["courses"].as_array().unwrap().iter().map(|c| c["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(titles, BTreeSet::from(["Rust Basics".into(), "Invite Only Workshop".into()]), "only public, published courses");
    let first = t.page(None, "/courses?limit=1").await;
    assert_eq!(first["courses"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().expect("a second page").to_string();
    let second = t.page(None, &format!("/courses?limit=1&cursor={cursor}")).await;
    assert_eq!(second["courses"].as_array().unwrap().len(), 1);
    assert_ne!(second["courses"][0]["id"], first["courses"][0]["id"]);
    assert!(second["next_cursor"].is_null());
    assert_eq!(t.page(None, "/courses?tag=rust").await["courses"][0]["title"], json!("Rust Basics"));
    assert_eq!(t.page(None, "/courses?q=invite").await["courses"][0]["title"], json!("Invite Only Workshop"));
    let cats = t.page(None, "/categories").await;
    assert_eq!(cats["categories"].as_array().unwrap().len(), 2);
    let course_page = t.page(None, "/courses/rust-basics").await["course"].clone();
    assert_eq!(course_page["instructors"], json!(["Tara Teacher"]));
    assert_eq!(course_page["chapters"].as_array().unwrap().len(), 2);
    let outline_text = course_page["chapters"].to_string();
    assert!(!outline_text.contains("What we will build") && !outline_text.contains("\"quiz\""), "the public outline has titles, not content");
    assert_eq!(course_page["cohorts"][0]["places"]["free"], json!(0));
    for hidden in ["/courses/rust-advanced", "/courses/public-draft", "/courses/no-such-course", "/courses/Not_A_Slug"] {
        assert!(matches!(err(t.call(None, "GET", hidden).await), Error::NotFound(_)), "{hidden}");
    }
    let preview = t.page(None, &format!("/courses/rust-basics/lessons/{l1}")).await;
    assert_eq!(preview["lesson"]["content"], json!("# Welcome\nWhat we will build."));
    assert!(matches!(err(t.call(None, "GET", &format!("/courses/rust-basics/lessons/{l2}")).await), Error::NotFound(_)));
    assert!(matches!(err(t.call(None, "GET", &format!("/courses/rust-basics/lessons/{}", adv_lessons[0])).await), Error::NotFound(_)));
    let public_file = served(&t.k, t.tenant, t.call(None, "GET", &format!("/courses/rust-basics/lessons/{l3}/file")).await.expect("preview file")).await;
    assert_eq!(public_file.body.as_slice(), PDF);
    assert!(matches!(err(t.call(None, "POST", "/courses").await), Error::NotFound(_)));

    let verified = t.page(None, &format!("/certificates/{code}")).await["certificate"].clone();
    assert_eq!(
        (verified["valid"].clone(), verified["learner_name"].clone(), verified["course_title"].clone()),
        (json!(true), json!("Asha Rao"), json!("Rust Basics"))
    );
    assert_eq!(verified["number"], json!(number));
    let typed = code.to_lowercase().replace('-', " ");
    assert_eq!(t.page(None, &format!("/certificates/{typed}")).await["certificate"]["number"], json!(number), "codes are forgiving to type");
    for bad in ["not-a-code", "0000-0000-0000-0000-0000-0000-0000-0000"] {
        assert!(matches!(err(t.call(None, "GET", &format!("/certificates/{bad}")).await), Error::NotFound(_)));
    }
    let view = t.call(None, "GET", &format!("/certificates/{code}/view")).await.expect("certificate page");
    assert!(view.content_type.starts_with("text/html"));
    assert!(view.headers.iter().any(|(h, v)| h == "content-security-policy" && v.contains("default-src 'none'")));
    let html = String::from_utf8(view.body).unwrap();
    assert!(html.contains("Asha Rao") && html.contains(&code) && html.contains("Head of Learning"));

    // ───────────── reports ─────────────
    let completion = t.report(&manager, "lms_course_completion", &[]).await;
    let by_title = |rows: &Value, title: &str| {
        rows["rows"].as_array().unwrap().iter().find(|r| r["course_title"] == json!(title)).cloned().unwrap_or_else(|| panic!("{title}"))
    };
    let rb = by_title(&completion, "Rust Basics");
    assert_eq!(
        (rb["enrolled"].clone(), rb["active"].clone(), rb["completed"].clone(), rb["withdrawn"].clone(), rb["certificates"].clone()),
        (json!(4), json!(3), json!(1), json!(1), json!(1))
    );
    assert_eq!((dec(&rb["completion_rate"]), dec(&rb["average_progress"])), (n("25"), n("25")));
    assert_eq!(dec(&by_title(&completion, "Invite Only Workshop")["completion_rate"]), n("100"));
    assert_eq!(by_title(&completion, "Rust Advanced")["completed"], json!(0));
    assert_eq!((completion["totals"]["enrolled"].clone(), completion["totals"]["completed"].clone()), (json!(6), json!(2)));
    assert_eq!(dec(&completion["totals"]["completion_rate"]), n("33.33"));

    let scores = t.report(&teacher, "lms_quiz_scores", &[("course", basics_id.to_string())]).await;
    let s0 = &scores["rows"][0];
    assert_eq!((s0["attempts"].clone(), s0["learners"].clone(), s0["learners_passed"].clone()), (json!(7), json!(3), json!(1)));
    assert_eq!((dec(&s0["average"]), dec(&s0["lowest"]), dec(&s0["highest"]), dec(&s0["pass_rate"])), (n("27.14"), n("0"), n("70"), n("14.29")));
    let buckets: Vec<i64> = ["pct_0_10", "pct_10_20", "pct_20_30", "pct_30_40", "pct_40_50", "pct_50_60", "pct_60_70", "pct_70_80", "pct_80_90", "pct_90_100"]
        .iter()
        .map(|b| s0[*b].as_i64().unwrap())
        .collect();
    assert_eq!(buckets, vec![3, 0, 1, 0, 1, 0, 1, 1, 0, 0]);
    let span = |from: chrono::NaiveDate, to: chrono::NaiveDate| vec![("from_date", from.to_string()), ("to_date", to.to_string())];
    let todays = t.report(&manager, "lms_quiz_scores", &span(today, today)).await;
    assert_eq!(todays["rows"][0]["attempts"], json!(7), "submitted today, in the workspace's time zone");
    let tomorrow = today + chrono::Duration::days(1);
    assert!(t.report(&manager, "lms_quiz_scores", &span(tomorrow, tomorrow)).await["rows"].as_array().unwrap().is_empty());

    let own = t.report(&asha, "lms_learner_progress", &[]).await;
    assert_eq!(own["rows"].as_array().unwrap().len(), 2);
    assert!(own["rows"].as_array().unwrap().iter().all(|r| r["learner"] == json!("asha@learn.test")), "learners see only themselves");
    let all = t.report(&manager, "lms_learner_progress", &[("course", basics_id.to_string())]).await;
    assert_eq!(all["rows"].as_array().unwrap().len(), 5);
    let overdue = t.report(&manager, "lms_overdue_assignments", &[]).await;
    assert_eq!(overdue["rows"].as_array().unwrap().len(), 1);
    let o = &overdue["rows"][0];
    assert_eq!((o["learner"].clone(), o["days_overdue"].clone(), o["status"].clone()), (json!("chen@learn.test"), json!(23), json!("Not Submitted")));
    assert_eq!(t.report(&chen, "lms_overdue_assignments", &[]).await["rows"].as_array().unwrap().len(), 1);
    assert!(t.report(&asha, "lms_overdue_assignments", &[]).await["rows"].as_array().unwrap().is_empty());

    // ───────────── revocation ─────────────
    assert!(t.try_act(&teacher, "LmsCertificate", cert_id, "revoke", json!({"reason": "?"})).await.is_err(), "instructors cannot revoke");
    assert!(field_error(t.try_act(&manager, "LmsCertificate", cert_id, "revoke", json!({})).await, "reason").contains("why"));
    t.act(&manager, "LmsCertificate", cert_id, "revoke", json!({"reason": "Issued by mistake"})).await;
    let revoked = t.page(None, &format!("/certificates/{code}")).await["certificate"].clone();
    assert_eq!((revoked["valid"].clone(), revoked["status"].clone()), (json!(false), json!("Revoked")));
    assert_eq!(t.notes(&asha, "Certificate revoked").await, 1);
    assert!(k.registry.schema(tenant).await.unwrap().entity("Communication").is_none(), "without Mail, notifications stay in-app");

    // ───────────── a workspace with the Mail app: emails as Communications ─────────────
    let tenant2 = k
        .create_tenant(NewTenant {
            slug: "mailco".into(),
            name: "Mail Co".into(),
            admin_email: "admin@mail.test".into(),
            admin_password: PW.into(),
            admin_name: None,
            packages: vec!["lms".into(), "comms".into()],
            settings: None,
        })
        .await
        .expect("second tenant");
    let s2 = k.login("mailco", "admin@mail.test", PW).await.expect("login");
    let admin2 = k.load_ctx(tenant2, s2.user, Some(s2.token_version), "it".into()).await.expect("ctx");
    let t2 = T { k: k.clone(), tenant: tenant2, admin: admin2.clone(), site };
    let pupil = t2.user("pupil@mail.test", "Pia Pupil", "Learner").await;
    let pupil2 = t2.user("pupil2@mail.test", "Paul Pupil", "Learner").await;
    let c2 = uid(&t2.insert(&admin2, "LmsCourse", json!({"title": "Safety Basics", "visibility": "Public"})).await);
    t2.chapter(&admin2, c2, "Intro", &[json!({"title": "Why safety"})]).await;
    t2.update(&admin2, "LmsCourse", c2, json!({"status": "Published"})).await;
    assert_eq!(t2.page(None, "/courses").await["courses"][0]["title"], json!("Safety Basics"), "each workspace has its own catalog");
    assert!(matches!(err(t2.call(None, "GET", &format!("/certificates/{code}")).await), Error::NotFound(_)), "another workspace's code is unknown here");
    t2.insert(&pupil, "LmsEnrollment", json!({"course": c2})).await;
    let mails = t2.list(&admin2, "Communication", json!({"recipients": "pupil@mail.test"})).await.unwrap();
    assert_eq!(mails.len(), 1);
    assert_eq!(
        (mails[0]["subject"].clone(), mails[0]["channel"].clone(), mails[0]["reference_entity"].clone()),
        (json!("Enrolled: Safety Basics"), json!("Email"), json!("LmsEnrollment"))
    );
    assert_eq!(t2.notes(&pupil, "Enrolled: Safety Basics").await, 1, "and in-app as well");
    t2.insert(&admin2, "LmsSettings", json!({"email_notifications": false})).await;
    t2.insert(&pupil2, "LmsEnrollment", json!({"course": c2})).await;
    assert!(t2.list(&admin2, "Communication", json!({"recipients": "pupil2@mail.test"})).await.unwrap().is_empty(), "emails can be switched off");
}

#[test]
fn paid_courses_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(paid()).await.unwrap() });
}

/// A course of 1,500 sold through the store: bought, refunded in full when its learner withdraws
/// within the 14 days, bought again, kept when they withdraw later; an unpaid purchase lapses.
async fn paid() {
    let url = std::env::var("FERRUM_LMS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 24, ..Default::default() }).await.expect("connect");
    reset(&db).await;
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    // Commerce's own packages (its Store Subscriptions connector needs Finance, which this test leaves out).
    packages.extend(ferrum_app_commerce::packages().expect("Commerce's packages").into_iter().filter(|p| p.name() != "commerce_subscriptions"));
    packages.extend(ferrum_app_lms::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    ferrum_app_commerce::register(&mut hooks);
    ferrum_app_lms::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    ferrum_app_erp::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "academy".into(),
            name: "Acme Academy".into(),
            admin_email: "admin@academy.test".into(),
            admin_password: PW.into(),
            admin_name: None,
            packages: vec!["lms".into(), "commerce".into(), "lms_payments".into()],
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Asia/Kolkata"})),
        })
        .await
        .expect("tenant");
    let session = k.login("academy", "admin@academy.test", PW).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("lms").cloned().expect("site registered");
    let t = T { k: k.clone(), tenant, admin: admin.clone(), site };
    let a = admin;
    let one = |rows: Vec<Doc>| rows.into_iter().next().expect("one record");

    // ───────────── a store to take the money ─────────────
    let inr = one(t.list(&a, "Currency", json!({"code": "INR"})).await.unwrap());
    let company = uid(&t.insert(&a, "Company", json!({"company_name": "Acme Academy", "abbr": "AA", "default_currency": inr["id"]})).await);
    let services = one(t.list(&a, "ItemGroup", json!({"group_name": "Services"})).await.unwrap());
    let nos = one(t.list(&a, "Uom", json!({"uom_name": "Nos"})).await.unwrap());
    let item = |code: &str| json!({"item_code": code, "item_name": code, "item_group": services["id"], "stock_uom": nos["id"], "is_stock_item": false});
    let course_item = uid(&t.insert(&a, "Item", item("COURSE")).await);
    let ship = uid(&t.insert(&a, "Item", item("SHIP")).await);
    let stores = one(t.list(&a, "Warehouse", json!({"warehouse_name": "Stores", "company": company.to_string()})).await.unwrap());
    let selling = one(t.list(&a, "PriceList", json!({"price_list_name": "Standard Selling"})).await.unwrap());
    t.insert(
        &a,
        "StoreSettings",
        json!({"store_name": "Academy", "company": company.to_string(), "currency": inr["id"], "price_list": selling["id"], "fulfilment_warehouse": stores["id"],
               "shipping_item": ship.to_string(), "payment_methods": [{"gateway": "fakepay", "label": "Card", "expiry_minutes": 30}], "store_url": "https://academy.test"}),
    )
    .await;
    let fakepay = {
        let role = "plugin:fakepay".to_string();
        for entity in ["PaymentRequest", "PaymentException", "OrderRefund"] {
            let p = ferrum_meta::PermissionDef { entity: entity.into(), read: true, write: true, ..Default::default() };
            t.k.set_role_permission(&a, &role, &p).await.unwrap();
        }
        let u = t.k.create_user(&a, "fakepay@plugins.ferrum.invalid", "Plugin: fakepay", None, &[role], false).await.unwrap();
        t.k.load_ctx(tenant, u, None, "plugin-fakepay".into()).await.unwrap()
    };
    let manager = t.user("manager@academy.test", "Mona Manager", "LMS Manager").await;
    let teacher = t.user("teacher@academy.test", "Tara Teacher", "Instructor").await;
    let asha = t.user("asha@academy.test", "Asha Rao", "Learner").await;
    let ben = t.user("ben@academy.test", "Ben Okafor", "Learner").await;
    let chen = t.user("chen@academy.test", "Chen Wei", "Learner").await;
    t.insert(&manager, "LmsSettings", json!({"lpay_company": company.to_string(), "lpay_payment_method": "fakepay", "lpay_refund_days": 14})).await;

    // ───────────── a paid course and a free one ─────────────
    assert!(
        field_error(t.try_insert(&teacher, "LmsCourse", json!({"title": "No Item", "lpay_price": "100"})).await, "lpay_item").contains("required"),
        "a price needs the item it is sold as"
    );
    let course = t
        .insert(
            &teacher,
            "LmsCourse",
            json!({"title": "Rust for Teams", "visibility": "Public", "lpay_price": "1500", "lpay_item": course_item.to_string(),
                   "instructors": [{"instructor": "teacher@academy.test"}]}),
        )
        .await;
    let cid = uid(&course);
    let (_, lessons) = t.chapter(&teacher, cid, "Ownership", &[json!({"title": "Moves"}), json!({"title": "Borrows"})]).await;
    t.update(&teacher, "LmsCourse", cid, json!({"status": "Published"})).await;
    let free = uid(&t.insert(&teacher, "LmsCourse", json!({"title": "Rust Taster", "visibility": "Public"})).await);
    t.chapter(&teacher, free, "Hello", &[json!({"title": "Cargo"})]).await;
    t.update(&teacher, "LmsCourse", free, json!({"status": "Published"})).await;

    let public = t.page(None, "courses").await;
    let card = |slug: &str| public["courses"].as_array().unwrap().iter().find(|c| c["slug"] == json!(slug)).cloned().unwrap();
    assert_eq!((dec(&card("rust-for-teams")["price"]["amount"]), card("rust-for-teams")["price"]["currency"].clone()), (n("1500"), json!("INR")));
    assert_eq!(card("rust-taster")["price"], Value::Null, "free");
    let page = t.page(Some(&asha), "courses/rust-for-teams").await["course"].clone();
    assert_eq!((page["can_enroll"].clone(), page["sale"]["can_buy"].clone(), page["sale"]["purchase"].clone()), (json!(false), json!(true), Value::Null));
    assert_eq!(t.page(Some(&asha), "courses/rust-taster").await["course"]["sale"], Value::Null);

    // ───────────── nobody enrols themselves in it unpaid; instructors still enrol anyone ─────────────
    let refused = t.try_insert(&asha, "LmsEnrollment", json!({"course": cid.to_string()})).await;
    assert!(field_error(refused, "course").contains("buy it"), "paid");
    let path = uid(&t
        .insert(
            &teacher,
            "LmsLearningPath",
            json!({"title": "Rust", "status": "Published", "courses": [{"course": cid.to_string()}, {"course": free.to_string()}]}),
        )
        .await);
    let joined = t.act(&ben, "LmsLearningPath", path, "join", json!({})).await;
    assert_eq!((joined["enrolled"].as_array().unwrap().len(), joined["skipped"].as_array().unwrap().len()), (1, 1), "{joined}");
    assert!(joined["skipped"][0]["reason"].as_str().unwrap().contains("buy it"), "{joined}");
    t.insert(&teacher, "LmsEnrollment", json!({"course": cid.to_string(), "learner": "ben@academy.test"})).await;

    // ───────────── Asha buys it: invoiced once, asked for through the store ─────────────
    let bought = t.act(&asha, "LmsCourse", cid, "buy", json!({})).await;
    let purchase = vid(&bought["purchase"]["id"]);
    assert_eq!((bought["purchase"]["status"].clone(), dec(&bought["purchase"]["amount"])), (json!("Awaiting Payment"), n("1500")));
    assert_eq!(bought["purchase"]["payment"]["status"], json!("Initiated"));
    let again = t.act(&asha, "LmsCourse", cid, "buy", json!({})).await;
    assert_eq!(again["purchase"]["id"], bought["purchase"]["id"], "asked again, not invoiced twice");
    let p = t.get("LmsCoursePurchase", purchase).await;
    let invoice = vid(&p["sales_invoice"]);
    let inv = t.get("SalesInvoice", invoice).await;
    assert_eq!((inv["docstatus"].clone(), dec(&inv["grand_total"]), dec(&inv["outstanding_amount"])), (json!(1), n("1500"), n("1500")));
    assert_eq!(t.count("select count(*) from e_sales_invoice where tenant_id = $1 and customer = $2 and $3 <> ''", vid(&inv["customer"]), "x").await, 1);
    let page = t.page(Some(&asha), "courses/rust-for-teams").await["course"].clone();
    assert_eq!((page["sale"]["can_buy"].clone(), page["sale"]["purchase"]["status"].clone()), (json!(false), json!("Awaiting Payment")));
    assert!(t.k.get(&chen, "LmsCoursePurchase", purchase, false).await.is_err(), "nobody else's purchase");
    assert_eq!(t.k.get(&asha, "LmsCoursePurchase", purchase, false).await.expect("her own")["learner"], json!("asha@academy.test"));

    // ───────────── the payment arrives: Asha is enrolled ─────────────
    let request = vid(&p["payment_request"]);
    t.act(&fakepay, "PaymentRequest", request, "mark_paid", json!({"amount": "1500", "currency": "INR", "gateway_reference": "pi_1"})).await;
    let p = t.get("LmsCoursePurchase", purchase).await;
    assert_eq!(p["status"], json!("Paid"));
    let enrolment = vid(&p["enrollment"]);
    assert_eq!(t.get("LmsEnrollment", enrolment).await["status"], json!("Active"));
    assert_eq!(dec(&t.get("SalesInvoice", invoice).await["outstanding_amount"]), n("0"));
    assert_eq!(t.page(Some(&asha), &format!("lessons/{}", lessons[0])).await["lesson"]["title"], json!("Moves"));
    assert_eq!(t.page(Some(&asha), "courses/rust-for-teams").await["course"]["sale"]["paid"], json!(true));

    // ───────────── she withdraws within the 14 days: all 1,500 back ─────────────
    t.act(&asha, "LmsEnrollment", enrolment, "withdraw", json!({})).await;
    let p = t.get("LmsCoursePurchase", purchase).await;
    assert_eq!(p["status"], json!("Refund Requested"));
    let note = t.get("SalesInvoice", vid(&p["credit_note"])).await;
    assert_eq!((dec(&note["grand_total"]), note["return_against"].clone()), (n("-1500"), json!(invoice.to_string())));
    let refund = vid(&p["refund"]);
    let r = t.get("OrderRefund", refund).await;
    assert_eq!((r["kind"].clone(), dec(&r["amount"]), r["status"].clone()), (json!("Payment"), n("1500"), json!("Requested")));
    t.act(&a, "OrderRefund", refund, "approve_refund", json!({})).await;
    t.act(&fakepay, "OrderRefund", refund, "refund_done", json!({"refund_reference": "re_1"})).await;
    let r = t.get("OrderRefund", refund).await;
    assert_eq!((r["status"].clone(), r["booked"].clone()), (json!("Refunded"), json!(true)), "{}", r["booking_error"]);
    assert!(field_error(t.try_act(&asha, "LmsEnrollment", enrolment, "reactivate", json!({})).await, "course").contains("buy it"), "refunded");
    let page = t.page(Some(&asha), "courses/rust-for-teams").await["course"].clone();
    assert_eq!((page["sale"]["can_buy"].clone(), page["sale"]["paid"].clone()), (json!(true), json!(false)));

    // ───────────── bought again: the same enrolment comes back, progress kept ─────────────
    let bought = t.act(&asha, "LmsCourse", cid, "buy", json!({})).await;
    let second = vid(&bought["purchase"]["id"]);
    assert_ne!(second, purchase);
    let request = vid(&t.get("LmsCoursePurchase", second).await["payment_request"]);
    t.act(&fakepay, "PaymentRequest", request, "mark_paid", json!({"amount": "1500", "currency": "INR", "gateway_reference": "pi_2"})).await;
    let p = t.get("LmsCoursePurchase", second).await;
    assert_eq!((p["status"].clone(), p["enrollment"].clone()), (json!("Paid"), json!(enrolment.to_string())));
    assert_eq!(t.get("LmsEnrollment", enrolment).await["status"], json!("Active"));

    // ───────────── withdrawing after the 14 days keeps the money; enrolling again needs no purchase ─────────────
    t.sql("update e_lms_course_purchase set paid_at = now() - interval '20 days' where tenant_id = $1 and id = $2", second).await;
    t.act(&asha, "LmsEnrollment", enrolment, "withdraw", json!({})).await;
    let p = t.get("LmsCoursePurchase", second).await;
    assert_eq!((p["status"].clone(), p["refund"].clone()), (json!("Paid"), Value::Null));
    t.act(&asha, "LmsEnrollment", enrolment, "reactivate", json!({})).await;
    assert_eq!(t.get("LmsEnrollment", enrolment).await["status"], json!("Active"));

    // ───────────── an unpaid purchase: asked again once its request lapses, then cancelled ─────────────
    let bought = t.act(&chen, "LmsCourse", cid, "buy", json!({})).await;
    let lapsing = vid(&bought["purchase"]["id"]);
    let p = t.get("LmsCoursePurchase", lapsing).await;
    let (first_request, chen_invoice) = (vid(&p["payment_request"]), vid(&p["sales_invoice"]));
    t.sql("update e_payment_request set status = 'Expired' where tenant_id = $1 and id = $2", first_request).await;
    t.act(&chen, "LmsCourse", cid, "buy", json!({})).await;
    let p = t.get("LmsCoursePurchase", lapsing).await;
    assert_ne!(vid(&p["payment_request"]), first_request, "a new request");
    assert_eq!(vid(&p["sales_invoice"]), chen_invoice, "for the same invoice");
    let second_request = vid(&p["payment_request"]);
    t.sql("update e_payment_request set status = 'Expired', updated_at = now() - interval '2 days' where tenant_id = $1 and id = $2", second_request).await;
    ferrum_app_lms::maintenance(&t.k).await.expect("maintenance");
    assert_eq!(t.get("LmsCoursePurchase", lapsing).await["status"], json!("Cancelled"));
    assert_eq!(t.get("SalesInvoice", chen_invoice).await["docstatus"], json!(2), "its invoice cancelled");
    let page = t.page(Some(&chen), "courses/rust-for-teams").await["course"].clone();
    assert_eq!((page["sale"]["can_buy"].clone(), page["sale"]["purchase"].clone()), (json!(true), Value::Null));
}

#[test]
fn scorm_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scorm()).await.unwrap() });
}

/// A package (a zip) with these files.
fn package(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in files {
        out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(body.as_bytes()).unwrap();
    }
    out.finish().unwrap().into_inner()
}

const MANIFEST_12: &str = r#"<?xml version="1.0"?>
<manifest identifier="rust" xmlns="http://www.imsproject.org/xsd/imscp_rootv1p1p2" xmlns:adlcp="http://www.adlnet.org/xsd/adlcp_rootv1p2">
  <metadata><schema>ADL SCORM</schema><schemaversion>1.2</schemaversion></metadata>
  <organizations default="o"><organization identifier="o"><title>Rust</title>
    <item identifier="i" identifierref="r"><title>Ownership</title><adlcp:masteryscore>80</adlcp:masteryscore></item>
  </organization></organizations>
  <resources><resource identifier="r" type="webcontent" adlcp:scormtype="sco" href="index.html"><file href="index.html"/></resource></resources>
</manifest>"#;

const MANIFEST_2004: &str = r#"<?xml version="1.0"?>
<manifest identifier="b" xmlns="http://www.imsglobal.org/xsd/imscp_v1p1" xmlns:adlcp="http://www.adlnet.org/xsd/adlcp_v1p3">
  <metadata><schema>ADL SCORM</schema><schemaversion>2004 4th Edition</schemaversion></metadata>
  <organizations default="o"><organization identifier="o"><item identifier="i" identifierref="r"><title>Borrowing</title></item></organization></organizations>
  <resources><resource identifier="r" type="webcontent" adlcp:scormType="sco" href="story.html"/></resources>
</manifest>"#;

/// A SCORM 1.2 lesson and a SCORM 2004 one: allowed in Learn Settings, served to enrolled learners
/// only, resumed from what the package saved, completed when the package says passed.
async fn scorm() {
    let url = std::env::var("FERRUM_LMS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    reset(&db).await;
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_lms::packages().expect("app packages").into_iter().filter(|p| p.name() != "lms_payments"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_lms::register(&mut hooks);
    // LTI's signing key is a secret field: secrets need the kernel's key.
    let settings = KernelSettings { secret_key: Some(SecretKey::new([9u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    k.set_file_store(Arc::new(MemFiles(Mutex::new(HashMap::new()))));
    ferrum_domain::maintenance(&k).await.expect("domain maintenance");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "scorm".into(),
            name: "Scorm Academy".into(),
            admin_email: "admin@scorm.test".into(),
            admin_password: PW.into(),
            admin_name: None,
            packages: vec!["lms".into()],
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Europe/Berlin"})),
        })
        .await
        .expect("tenant");
    let session = k.login("scorm", "admin@scorm.test", PW).await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("lms").cloned().expect("site registered");
    let t = T { k: k.clone(), tenant, admin, site };
    let manager = t.user("manager@scorm.test", "Mona Manager", "LMS Manager").await;
    let teacher = t.user("teacher@scorm.test", "Tara Teacher", "Instructor").await;
    let asha = t.user("asha@scorm.test", "Asha Rao", "Learner").await;
    let ben = t.user("ben@scorm.test", "Ben Okafor", "Learner").await;

    let course = uid(&t.insert(&teacher, "LmsCourse", json!({"title": "Rust Packages", "instructors": [{"instructor": "teacher@scorm.test"}]})).await);
    let (chapter, lessons) = t.chapter(&teacher, course, "Start", &[json!({"title": "Welcome"})]).await;
    let zip12 = package(&[
        ("imsmanifest.xml", MANIFEST_12),
        ("index.html", "<script src=\"js/app.js\"></script><h1>Ownership</h1>"),
        ("js/app.js", "API.LMSInitialize('')"),
    ]);
    let file12 = vid(&t.k.store_file(&teacher, None, "rust.zip", "application/zip", &zip12).await.expect("upload")["id"]);
    let scorm_lesson =
        |title: &str, file: Uuid| json!({"title": title, "chapter": chapter.to_string(), "lesson_type": "SCORM", "package_file": format!("/files/{file}")});

    // ───────────── packages wait for Learn Settings; what cannot play is refused ─────────────
    let refused = t.try_insert(&teacher, "LmsLesson", scorm_lesson("Ownership", file12)).await;
    assert!(field_error(refused, "lesson_type").contains("SCORM packages are not allowed"), "off until an LMS manager allows them");
    t.insert(&manager, "LmsSettings", json!({"allow_packages": true})).await;
    let no_manifest = vid(&t.k.store_file(&teacher, None, "site.zip", "application/zip", &package(&[("index.html", "")])).await.expect("upload")["id"]);
    assert!(field_error(t.try_insert(&teacher, "LmsLesson", scorm_lesson("Broken", no_manifest)).await, "package_file").contains("imsmanifest.xml"));
    let pdf = vid(&t.k.store_file(&teacher, None, "notes.pdf", "application/pdf", PDF).await.expect("upload")["id"]);
    assert!(field_error(t.try_insert(&teacher, "LmsLesson", scorm_lesson("Not a zip", pdf)).await, "package_file").contains("not a zip"));
    let first = t.insert(&teacher, "LmsLesson", scorm_lesson("Ownership", file12)).await;
    assert_eq!(
        (first["package_standard"].clone(), first["package_launch"].clone(), dec(&first["package_mastery_score"])),
        (json!("SCORM 1.2"), json!("index.html"), n("80"))
    );
    let first = uid(&first);
    let zip2004 = package(&[("imsmanifest.xml", MANIFEST_2004), ("story.html", "<h1>Borrowing</h1>")]);
    let file2004 = vid(&t.k.store_file(&teacher, None, "borrow.zip", "application/zip", &zip2004).await.expect("upload")["id"]);
    let second = uid(&t.insert(&teacher, "LmsLesson", scorm_lesson("Borrowing", file2004)).await);
    assert_eq!(t.get("LmsLesson", second).await["package_standard"], json!("SCORM 2004"));
    t.update(&teacher, "LmsCourse", course, json!({"status": "Published"})).await;
    let enrolment = uid(&t.insert(&asha, "LmsEnrollment", json!({"course": course.to_string()})).await);

    // ───────────── the player gets the package; only those who may open the lesson get its files ─────────────
    let view = t.page(Some(&asha), &format!("lessons/{first}")).await;
    let launch = format!("/api/app/lms/lessons/{first}/package/{file12}/index.html");
    assert_eq!(
        (view["scorm"]["version"].clone(), view["scorm"]["launch_url"].clone(), view["scorm"]["saved"].clone()),
        (json!("1.2"), json!(launch), Value::Null)
    );
    assert_eq!(view["scorm"]["learner"], json!({"id": "asha@scorm.test", "name": "Asha Rao"}));
    let file = |user: &Ctx, path: String| {
        let t = &t;
        let user = user.clone();
        async move { t.call(Some(&user), "GET", &path).await }
    };
    let r = file(&asha, format!("lessons/{first}/package/{file12}/index.html")).await.expect("the launch file");
    assert_eq!((r.status, r.content_type.as_str()), (200, "text/html; charset=utf-8"));
    assert!(String::from_utf8_lossy(&r.body).contains("<h1>Ownership</h1>"));
    let header = |r: &SiteResponse, name: &str| r.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default();
    assert!(header(&r, "content-security-policy").contains("frame-ancestors 'self'") && header(&r, "x-content-type-options") == "nosniff");
    let js = file(&asha, format!("lessons/{first}/package/{file12}/js/app.js")).await.expect("a script of the package");
    assert_eq!((js.content_type.as_str(), js.body.as_slice()), ("text/javascript; charset=utf-8", &b"API.LMSInitialize('')"[..]));
    for path in [
        format!("lessons/{first}/package/{file12}/missing.html"),
        format!("lessons/{first}/package/{file12}/../index.html"),
        format!("lessons/{first}/package/{file2004}/story.html"),
        format!("lessons/{}/package/{file12}/index.html", lessons[0]),
    ] {
        assert!(matches!(err(file(&asha, path.clone()).await), Error::NotFound(_)), "{path}");
    }
    assert!(matches!(err(file(&ben, format!("lessons/{first}/package/{file12}/index.html")).await), Error::NotFound(_)), "not enrolled");
    assert!(file(&teacher, format!("lessons/{first}/package/{file12}/index.html")).await.is_ok(), "its instructor previews it");

    // ───────────── what the package saves is resumed; only passing completes the lesson ─────────────
    assert!(
        err(t.try_act(&asha, "LmsEnrollment", enrolment, "complete_lesson", json!({"lesson": first.to_string()})).await).to_string().contains("quiz lessons")
    );
    let partway = json!({"core": {"lesson_status": "incomplete", "lesson_location": "page-3", "total_time": "0000:05:00"}, "suspend_data": "p3;q1=b"});
    let saved = t.act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": first.to_string(), "cmi": partway})).await;
    assert_eq!((saved["status"].clone(), saved["completed"].clone()), (json!("Incomplete"), json!(false)));
    let resumed = t.page(Some(&asha), &format!("lessons/{first}")).await;
    assert_eq!((resumed["scorm"]["saved"]["cmi"]["suspend_data"].clone(), resumed["completed"].clone()), (json!("p3;q1=b"), json!(false)));
    assert!(t.try_act(&ben, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": first.to_string(), "cmi": {}})).await.is_err(), "not his enrolment");
    assert!(
        field_error(t.try_act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": lessons[0].to_string(), "cmi": {}})).await, "lesson")
            .contains("not a SCORM lesson")
    );
    let huge = json!({"suspend_data": "x".repeat(1 << 20)});
    assert!(
        field_error(t.try_act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": first.to_string(), "cmi": huge})).await, "cmi")
            .contains("larger")
    );
    let passed = json!({"core": {"lesson_status": "passed", "score": {"raw": "45", "min": "0", "max": "50"}, "total_time": "0000:12:30"}, "suspend_data": ""});
    let done = t.act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": first.to_string(), "cmi": passed})).await;
    assert_eq!((done["status"].clone(), dec(&done["score_percent"]), done["completed"].clone()), (json!("Passed"), n("90"), json!(true)));
    let e = t.get("LmsEnrollment", enrolment).await;
    assert_eq!((dec(&e["progress_percent"]), e["completed_lessons"].clone()), (n("33.33"), json!(1)), "one of three lessons");
    let rows = t.list(&asha, "LmsPackageProgress", json!({})).await.expect("her own progress");
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0]["status"].clone(), rows[0]["total_seconds"].clone(), rows[0]["commits"].clone()), (json!("Passed"), json!(750), json!(2)));
    assert_eq!(t.list(&ben, "LmsPackageProgress", json!({})).await.expect("his own").len(), 0, "nobody else's");
    let via = t.list(&t.admin, "LmsLessonProgress", json!({"lesson": first.to_string()})).await.unwrap();
    assert_eq!(via[0]["completed_via"], json!("SCORM"));
    for who in [&asha, &t.admin] {
        let r = t.try_insert(who, "LmsPackageProgress", json!({"enrollment": enrolment.to_string(), "lesson": second.to_string()})).await;
        assert!(r.is_err(), "only the player writes it, not even the administrator: {r:?}");
    }

    // ───────────── SCORM 2004: completed but failed is not done; passed is ─────────────
    let failed = json!({"completion_status": "completed", "success_status": "failed", "score": {"scaled": "0.4"}, "total_time": "PT20M"});
    let r = t.act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": second.to_string(), "cmi": failed})).await;
    assert_eq!((r["status"].clone(), dec(&r["score_percent"]), r["completed"].clone()), (json!("Failed"), n("40"), json!(false)));
    let passed = json!({"completion_status": "completed", "success_status": "passed", "score": {"scaled": "0.85"}, "total_time": "PT31M"});
    let r = t.act(&asha, "LmsEnrollment", enrolment, "scorm_commit", json!({"lesson": second.to_string(), "cmi": passed})).await;
    assert_eq!((r["status"].clone(), r["completed"].clone()), (json!("Passed"), json!(true)));
    assert_eq!(dec(&t.get("LmsEnrollment", enrolment).await["progress_percent"]), n("66.67"));

    // ───────────── a new package is a new address; the old one stops ─────────────
    let zip_v2 = package(&[("imsmanifest.xml", MANIFEST_12), ("index.html", "<h1>Ownership, second edition</h1>")]);
    let file_v2 = vid(&t.k.store_file(&teacher, None, "rust-v2.zip", "application/zip", &zip_v2).await.expect("upload")["id"]);
    t.update(&teacher, "LmsLesson", first, json!({"package_file": format!("/files/{file_v2}")})).await;
    assert!(matches!(err(file(&asha, format!("lessons/{first}/package/{file12}/index.html")).await), Error::NotFound(_)));
    let r = file(&asha, format!("lessons/{first}/package/{file_v2}/index.html")).await.expect("the new package");
    assert!(String::from_utf8_lossy(&r.body).contains("second edition"));

    // ───────────── an xAPI package: its statements and state, as the signed-in learner only ─────────────
    let tincan = r#"<?xml version="1.0"?><tincan xmlns="http://projecttincan.com/tincan.xsd"><activities>
      <activity id="https://acme.test/lifetimes" type="http://adlnet.gov/expapi/activities/course"><name>Lifetimes</name><launch lang="und">story.html</launch></activity>
    </activities></tincan>"#;
    let zip_x = package(&[("tincan.xml", tincan), ("story.html", "<h1>Lifetimes</h1>")]);
    let file_x = vid(&t.k.store_file(&teacher, None, "lifetimes.zip", "application/zip", &zip_x).await.expect("upload")["id"]);
    let as_scorm = json!({"title": "Wrong kind", "chapter": chapter.to_string(), "lesson_type": "SCORM", "package_file": format!("/files/{file_x}")});
    assert!(field_error(t.try_insert(&teacher, "LmsLesson", as_scorm).await, "package_file").contains("imsmanifest.xml"), "an xAPI package is not SCORM");
    let third = t
        .insert(
            &teacher,
            "LmsLesson",
            json!({"title": "Lifetimes", "chapter": chapter.to_string(), "lesson_type": "xAPI", "package_file": format!("/files/{file_x}")}),
        )
        .await;
    assert_eq!(
        (third["package_standard"].clone(), third["package_launch"].clone(), third["package_activity"].clone()),
        (json!("xAPI"), json!("story.html"), json!("https://acme.test/lifetimes"))
    );
    let third = uid(&third);
    let view = t.page(Some(&asha), &format!("lessons/{third}")).await["xapi"].clone();
    assert_eq!((view["endpoint"].clone(), view["registration"].clone()), (json!(format!("/api/app/lms/xapi/{third}/")), json!(enrolment.to_string())));
    assert_eq!(view["actor"]["mbox"], json!("mailto:asha@scorm.test"));
    let x = |path: &str| format!("xapi/{third}/{path}");
    let about = t.send(&asha, "GET", &x("about"), b"", &[]).await.expect("about");
    assert_eq!(serde_json::from_slice::<Value>(&about.body).unwrap(), json!({"version": ["1.0.3"]}));
    let me = json!({"objectType": "Agent", "mbox": "mailto:asha@scorm.test"});
    let statement = |verb: &str, result: Value| {
        json!({"actor": me, "verb": {"id": format!("http://adlnet.gov/expapi/verbs/{verb}")}, "object": {"id": "https://acme.test/lifetimes"},
               "result": result, "context": {"registration": enrolment.to_string()}})
    };
    let body = |v: &Value| serde_json::to_vec(v).unwrap();
    let r = t.send(&asha, "POST", &x("statements"), &body(&statement("attempted", json!({}))), &[("content-type", "application/json")]).await.expect("posted");
    let ids: Vec<String> = serde_json::from_slice(&r.body).unwrap();
    assert_eq!((r.status, ids.len(), header(&r, "x-experience-api-version")), (200, 1, "1.0.3".to_string()));
    let progress = || {
        let t = &t;
        async move { t.list(&t.admin, "LmsPackageProgress", json!({"lesson": third.to_string()})).await.unwrap() }
    };
    assert_eq!(progress().await[0]["status"], json!("Incomplete"));
    let mut ben_said = statement("passed", json!({}));
    ben_said["actor"]["mbox"] = json!("mailto:ben@scorm.test");
    assert!(matches!(err(t.send(&asha, "POST", &x("statements"), &body(&ben_said), &[]).await), Error::Forbidden(_)), "statements about someone else");
    assert!(matches!(err(t.send(&ben, "POST", &x("statements"), &body(&ben_said), &[]).await), Error::NotFound(_)), "not enrolled");

    // A put is at its id: once, again the same, never differently.
    let at = Uuid::now_v7();
    let put = |v: &Value| {
        let t = &t;
        let (asha, path, v) = (asha.clone(), x(&format!("statements?statementId={at}")), body(v));
        async move { t.send(&asha, "PUT", &path, &v, &[]).await }
    };
    let progressed = statement("progressed", json!({"score": {"scaled": 0.5}}));
    assert_eq!(put(&progressed).await.expect("put").status, 204);
    assert_eq!(put(&progressed).await.expect("the same again").status, 204);
    assert!(matches!(err(put(&statement("progressed", json!({"score": {"scaled": 0.6}}))).await), Error::Conflict(_)));

    // State: put, read with its tag, merge, guard, list, delete.
    // Query values reach the endpoint decoded, as here.
    let agent = r#"{"mbox":"mailto:asha@scorm.test"}"#;
    let doc = |id: &str| x(&format!("activities/state?activityId=https://acme.test/lifetimes&agent={agent}&stateId={id}"));
    let r = t.send(&asha, "PUT", &doc("resume"), br#"{"slide":3}"#, &[("content-type", "application/json")]).await.expect("put state");
    assert_eq!(r.status, 204);
    let got = t.send(&asha, "GET", &doc("resume"), b"", &[]).await.expect("get state");
    assert_eq!((got.body.as_slice(), got.content_type.as_str()), (&br#"{"slide":3}"#[..], "application/json"));
    let tag = header(&got, "etag");
    t.send(&asha, "POST", &doc("resume"), br#"{"seen":[1,2,3]}"#, &[("content-type", "application/json")]).await.expect("merged");
    let merged: Value = serde_json::from_slice(&t.send(&asha, "GET", &doc("resume"), b"", &[]).await.unwrap().body).unwrap();
    assert_eq!(merged, json!({"slide": 3, "seen": [1, 2, 3]}));
    assert!(matches!(err(t.send(&asha, "PUT", &doc("resume"), b"{}", &[("if-match", &tag)]).await), Error::Conflict(_)), "it changed since that tag");
    assert!(matches!(err(t.send(&asha, "PUT", &doc("resume"), b"{}", &[("if-none-match", "*")]).await), Error::Conflict(_)), "it exists");
    t.send(&asha, "PUT", &doc("bookmark"), b"slide-9", &[("content-type", "text/plain")]).await.expect("a text document");
    let list = x(&format!("activities/state?activityId=https://acme.test/lifetimes&agent={agent}"));
    let ids: Value = serde_json::from_slice(&t.send(&asha, "GET", &list, b"", &[]).await.unwrap().body).unwrap();
    assert_eq!(ids, json!(["bookmark", "resume"]));
    t.send(&asha, "DELETE", &doc("bookmark"), b"", &[]).await.expect("deleted");
    assert!(matches!(err(t.send(&asha, "GET", &doc("bookmark"), b"", &[]).await), Error::NotFound(_)));
    let bens = x("activities/state?activityId=https://acme.test/lifetimes&agent={\"mbox\":\"mailto:ben@scorm.test\"}&stateId=resume");
    assert!(matches!(err(t.send(&asha, "GET", &bens, b"", &[]).await), Error::Forbidden(_)), "nobody else's state");

    // Passed completes the lesson; a later failure keeps the completion; voided statements leave the list.
    let passed = statement("passed", json!({"score": {"scaled": 0.9}, "duration": "PT25M"}));
    t.send(&asha, "POST", &x("statements"), &body(&passed), &[]).await.expect("passed");
    let p = &progress().await[0];
    assert_eq!((p["status"].clone(), dec(&p["score_percent"]), p["total_seconds"].clone()), (json!("Passed"), n("90"), json!(1500)));
    let via = t.list(&t.admin, "LmsLessonProgress", json!({"lesson": third.to_string()})).await.unwrap();
    assert_eq!(via[0]["completed_via"], json!("xAPI"));
    assert_eq!(dec(&t.get("LmsEnrollment", enrolment).await["progress_percent"]), n("75"), "three of four lessons");
    t.send(&asha, "POST", &x("statements"), &body(&statement("failed", json!({"score": {"scaled": 0.2}}))), &[]).await.expect("a failed retake");
    assert_eq!(progress().await[0]["status"], json!("Passed"), "once done, it stays done");
    let void = json!({"actor": me, "verb": {"id": "http://adlnet.gov/expapi/verbs/voided"}, "object": {"objectType": "StatementRef", "id": at.to_string()}});
    t.send(&asha, "POST", &x("statements"), &body(&void), &[]).await.expect("voided");
    let listed: Value = serde_json::from_slice(&t.send(&asha, "GET", &x("statements"), b"", &[]).await.unwrap().body).unwrap();
    let listed_ids: Vec<String> = listed["statements"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert!(!listed_ids.contains(&at.to_string()) && listed_ids.len() == 4, "{listed_ids:?}");
    assert_eq!(listed["statements"][0]["verb"]["id"], json!("http://adlnet.gov/expapi/verbs/voided"), "newest first");
    let voided = t.send(&asha, "GET", &x(&format!("statements?voidedStatementId={at}")), b"", &[]).await.expect("still there, voided");
    assert_eq!(serde_json::from_slice::<Value>(&voided.body).unwrap()["id"], json!(at.to_string()));
    let again = t.send(&asha, "POST", &x("statements"), &body(&void), &[]).await.expect("voiding again is kept, as xAPI says");
    assert_eq!(again.status, 200);
    let void_the_void =
        json!({"actor": me, "verb": {"id": "http://adlnet.gov/expapi/verbs/voided"}, "object": {"objectType": "StatementRef", "id": listed_ids[0]}});
    t.send(&asha, "POST", &x("statements"), &body(&void_the_void), &[]).await.expect("kept");
    let listed: Value = serde_json::from_slice(&t.send(&asha, "GET", &x("statements"), b"", &[]).await.unwrap().body).unwrap();
    assert!(listed["statements"].as_array().unwrap().iter().any(|s| s["id"] == json!(listed_ids[0])), "a voiding statement is never voided");

    // An instructor's preview runs and keeps nothing.
    let before = t.count("select count(*) from lms_xapi_statement where tenant_id = $1 and lesson = $2 and $3 <> ''", third, "x").await;
    let mut tara = statement("passed", json!({}));
    tara["actor"]["mbox"] = json!("mailto:teacher@scorm.test");
    let r = t.send(&teacher, "POST", &x("statements"), &body(&tara), &[]).await.expect("a preview's statement");
    assert_eq!(r.status, 200);
    assert_eq!(t.count("select count(*) from lms_xapi_statement where tenant_id = $1 and lesson = $2 and $3 <> ''", third, "x").await, before);

    lti(&t, &manager, &teacher, &asha, &ben, course, chapter, enrolment).await;
}

/// An LTI 1.3 tool: registered, launched by OIDC with a signed id_token, sending a grade back with
/// an access token it got for a signed client assertion; that grade completes the lesson.
async fn lti(t: &T, manager: &Ctx, teacher: &Ctx, asha: &Ctx, ben: &Ctx, course: Uuid, chapter: Uuid, enrolment: Uuid) {
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::{EncodePublicKey, LineEnding};
    let header = |r: &SiteResponse, name: &str| r.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default();
    let query = |url: &str| -> HashMap<String, String> { url::Url::parse(url).unwrap().query_pairs().into_owned().collect() };

    // ───────────── a tool, registered once Learn has an issuer ─────────────
    let tool_key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let tool_pem = tool_key.to_public_key().to_public_key_pem(LineEnding::LF).unwrap();
    let lab = json!({"tool_name": "Rust Lab", "login_url": "https://lab.test/login", "launch_url": "https://lab.test/launch", "sends_grades": true,
                     "public_key": tool_pem, "custom_parameters": "level=beginner"});
    assert!(field_error(t.try_insert(manager, "LmsLtiTool", lab.clone()).await, "tool_name").contains("Public URL"), "no issuer yet");
    let settings = uid(&t.list(manager, "LmsSettings", json!({})).await.unwrap()[0]);
    t.update(manager, "LmsSettings", settings, json!({"public_url": "https://academy.example"})).await;
    assert!(t.try_insert(teacher, "LmsLtiTool", lab.clone()).await.is_err(), "instructors read tools, LMS managers register them");
    let mut no_key = lab.clone();
    no_key["tool_name"] = json!("Keyless");
    no_key["public_key"] = Value::Null;
    assert!(field_error(t.try_insert(manager, "LmsLtiTool", no_key).await, "public_key").contains("public key"));
    let tool = t.insert(manager, "LmsLtiTool", lab).await;
    let client = tool["client_id"].as_str().unwrap().to_string();
    assert_eq!(
        (tool["platform_issuer"].clone(), tool["platform_auth_url"].clone(), tool["platform_jwks_url"].clone(), tool["platform_token_url"].clone()),
        (
            json!("https://academy.example"),
            json!("https://academy.example/api/app/lms/lti/auth"),
            json!("https://academy.example/api/site/scorm/lms/lti/jwks"),
            json!("https://academy.example/api/site/scorm/lms/lti/token")
        )
    );
    let lesson = uid(&t
        .insert(teacher, "LmsLesson", json!({"title": "Lab", "chapter": chapter.to_string(), "lesson_type": "LTI", "lti_tool": uid(&tool).to_string()}))
        .await);
    assert_eq!(t.page(Some(asha), &format!("lessons/{lesson}")).await["lti"]["launch_url"], json!(format!("/api/app/lms/lti/{lesson}/launch")));

    // ───────────── the launch: OIDC login at the tool, then Learn's signed id_token ─────────────
    let go = t.send(asha, "GET", &format!("lti/{lesson}/launch"), b"", &[]).await.expect("login initiation");
    assert_eq!(go.status, 302);
    let login = header(&go, "location");
    assert!(login.starts_with("https://lab.test/login?"), "{login}");
    let q = query(&login);
    assert_eq!(
        (q["iss"].as_str(), q["login_hint"].clone(), q["lti_message_hint"].clone(), q["client_id"].as_str(), q["target_link_uri"].as_str()),
        ("https://academy.example", asha.user.to_string(), lesson.to_string(), client.as_str(), "https://lab.test/launch")
    );
    assert!(matches!(err(t.send(ben, "GET", &format!("lti/{lesson}/launch"), b"", &[]).await), Error::NotFound(_)), "not enrolled");
    let auth = |hint: Uuid, redirect: &str, nonce: &str| {
        format!(
            "lti/auth?scope=openid&response_type=id_token&response_mode=form_post&prompt=none&client_id={client}&redirect_uri={redirect}\
             &login_hint={hint}&lti_message_hint={lesson}&state=st-1&nonce={nonce}"
        )
    };
    let page = t.send(asha, "GET", &auth(asha.user, "https://lab.test/launch", "n-1"), b"", &[]).await.expect("the launch");
    let html = String::from_utf8(page.body.clone()).unwrap();
    assert!(header(&page, "content-security-policy").contains("form-action https://lab.test;"));
    assert!(html.contains("action=\"https://lab.test/launch\"") && html.contains("name=\"state\" value=\"st-1\""));
    let id_token = html.split("name=\"id_token\" value=\"").nth(1).and_then(|r| r.split('"').next()).expect("an id_token").to_string();
    for (hint, redirect, nonce, why) in [
        (asha.user, "https://evil.test/launch", "n-2", "a redirect the tool did not register"),
        (asha.user, "https://lab.test/launch", "", "no nonce"),
        (ben.user, "https://lab.test/launch", "n-3", "a launch started for someone else"),
    ] {
        assert!(t.send(asha, "GET", &auth(hint, redirect, nonce), b"", &[]).await.is_err(), "{why}");
    }

    // The tool checks it against Learn's published keys.
    let jwks = t.page(None, "lti/jwks").await;
    let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwks["keys"][0].clone()).unwrap();
    let mut v = Validation::new(Algorithm::RS256);
    v.set_audience(&[client.as_str()]);
    v.set_issuer(&["https://academy.example"]);
    let claims = jsonwebtoken::decode::<Value>(&id_token, &DecodingKey::from_jwk(&jwk).unwrap(), &v).expect("signed by Learn").claims;
    let lti = |k: &str| claims[format!("https://purl.imsglobal.org/spec/lti/claim/{k}")].clone();
    assert_eq!(
        (claims["sub"].clone(), claims["nonce"].clone(), lti("message_type"), lti("version")),
        (json!(asha.user.to_string()), json!("n-1"), json!("LtiResourceLinkRequest"), json!("1.3.0"))
    );
    assert_eq!(
        (lti("resource_link")["id"].clone(), lti("context")["id"].clone(), lti("deployment_id")),
        (json!(lesson.to_string()), json!(course.to_string()), tool["deployment_id"].clone())
    );
    assert_eq!(lti("roles"), json!(["http://purl.imsglobal.org/vocab/lis/v2/membership#Learner"]));
    assert_eq!(lti("custom"), json!({"level": "beginner"}));
    assert_eq!(claims.get("email"), None, "no name or email unless the tool may know them");
    let lineitem = claims["https://purl.imsglobal.org/spec/lti-ags/claim/endpoint"]["lineitem"].as_str().unwrap().to_string();
    assert_eq!(lineitem, format!("https://academy.example/api/app/lms/lti/ags/{lesson}"));

    // ───────────── grades back: a client assertion for a token, then the score ─────────────
    let now = chrono::Utc::now().timestamp();
    let token_url = "https://academy.example/api/site/scorm/lms/lti/token";
    let assertion = |jti: &str, key: &rsa::RsaPrivateKey| {
        let c = json!({"iss": client, "sub": client, "aud": token_url, "iat": now, "exp": now + 60, "jti": jti});
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &c, &EncodingKey::from_rsa_der(key.to_pkcs1_der().unwrap().as_bytes())).unwrap()
    };
    let ask = |a: String| {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
            .append_pair("client_assertion", &a)
            .append_pair("scope", "https://purl.imsglobal.org/spec/lti-ags/scope/score")
            .finish();
        let req = SiteRequest {
            tenant: t.tenant,
            user: None,
            method: "POST".into(),
            path: "/lti/token".into(),
            query: Map::new(),
            headers: Map::new(),
            body: form.into_bytes(),
            client_ip: "203.0.113.9".into(),
        };
        let (site, k) = (t.site.clone(), t.k.clone());
        async move { site.handle(&k, req).await.expect("token endpoint") }
    };
    let first = assertion("j-1", &tool_key);
    let r = ask(first.clone()).await;
    let answer: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(
        (r.status, answer["token_type"].clone(), answer["scope"].clone()),
        (200, json!("Bearer"), json!("https://purl.imsglobal.org/spec/lti-ags/scope/score"))
    );
    assert_eq!(ask(first).await.status, 400, "an assertion is used once");
    let stranger = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    assert_eq!(ask(assertion("j-2", &stranger)).await.status, 400, "signed by another key");
    // The server turns the bearer token into the tool's service user, as here.
    let (session, _) = t.k.verify_api_key(answer["access_token"].as_str().unwrap()).await.expect("an API key");
    let tool_ctx = t.k.load_ctx(t.tenant, session.user, None, "lti".into()).await.unwrap();
    let item = t.send(&tool_ctx, "GET", &format!("lti/ags/{lesson}"), b"", &[]).await.expect("the line item");
    assert_eq!(serde_json::from_slice::<Value>(&item.body).unwrap()["id"], json!(lineitem));
    let score = |user: Uuid, progress: &str| {
        serde_json::to_vec(&json!({"userId": user.to_string(), "scoreGiven": 45, "scoreMaximum": 50, "activityProgress": progress,
                                   "gradingProgress": "FullyGraded", "timestamp": "2026-11-04T10:00:00Z"}))
        .unwrap()
    };
    assert!(
        matches!(err(t.send(asha, "POST", &format!("lti/ags/{lesson}/scores"), &score(asha.user, "Completed"), &[]).await), Error::Forbidden(_)),
        "the tool's line item only"
    );
    assert!(t.send(&tool_ctx, "POST", &format!("lti/ags/{lesson}/scores"), &score(ben.user, "Completed"), &[]).await.is_err(), "not a learner of the lesson");
    let midway = t.send(&tool_ctx, "POST", &format!("lti/ags/{lesson}/scores"), &score(asha.user, "InProgress"), &[]).await.expect("a score");
    assert_eq!(serde_json::from_slice::<Value>(&midway.body).unwrap()["completed"], json!(false));
    let done = t.send(&tool_ctx, "POST", &format!("lti/ags/{lesson}/scores"), &score(asha.user, "Completed"), &[]).await.expect("the grade");
    let done: Value = serde_json::from_slice(&done.body).unwrap();
    assert_eq!((done["completed"].clone(), dec(&done["score_percent"])), (json!(true), n("90")));
    let via = t.list(&t.admin, "LmsLessonProgress", json!({"lesson": lesson.to_string()})).await.unwrap();
    assert_eq!(via[0]["completed_via"], json!("LTI"));
    assert_eq!(dec(&t.get("LmsEnrollment", enrolment).await["progress_percent"]), n("80"), "four of five lessons");
    assert!(
        err(t.try_act(asha, "LmsEnrollment", enrolment, "complete_lesson", json!({"lesson": lesson.to_string()})).await).to_string().contains("quiz lessons"),
        "a grading tool's lesson is not marked done by hand"
    );

    // A tool that sends no grades: its lessons are the learner's to mark done.
    let plain = uid(&t
        .insert(manager, "LmsLtiTool", json!({"tool_name": "Reader", "login_url": "https://reader.test/login", "launch_url": "https://reader.test/launch"}))
        .await);
    let read = uid(&t
        .insert(teacher, "LmsLesson", json!({"title": "Read", "chapter": chapter.to_string(), "lesson_type": "LTI", "lti_tool": plain.to_string()}))
        .await);
    assert!(
        matches!(err(t.send(&tool_ctx, "POST", &format!("lti/ags/{read}/scores"), &score(asha.user, "Completed"), &[]).await), Error::NotFound(_)),
        "a tool grades its own lessons only"
    );
    let marked = t.act(asha, "LmsEnrollment", enrolment, "complete_lesson", json!({"lesson": read.to_string()})).await;
    assert_eq!(dec(&marked["progress_percent"]), n("83.33"), "five of six");
}

/// LiveKit's room service as a stand-in: `ListParticipants` answers with `here`, `DeleteRoom` with `{}`.
async fn room_service(here: Arc<Mutex<Vec<Value>>>) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let here = here.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                let mut length = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).await.unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = h.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).await.unwrap();
                let answer = match path.as_str() {
                    "/twirp/livekit.RoomService/ListParticipants" => json!({"participants": here.lock().unwrap().clone()}),
                    _ => json!({}),
                };
                let text = answer.to_string();
                let mut sock = reader.into_inner();
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", text.len());
                sock.write_all(head.as_bytes()).await.unwrap();
                sock.write_all(text.as_bytes()).await.unwrap();
            });
        }
    });
    url
}

#[test]
fn live_sessions_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(live_sessions()).await.unwrap() });
}

/// A cohort's live session without a link gets a meeting (moved with the session), members get their
/// join link on the lesson page and in the reminder, attendance is read from the meeting once it is
/// over and credits the Live lesson by the session's rule, and an instructor can mark it by hand. A
/// session with a link of its own, or a workspace without Comms, keeps the typed link and manual marks.
async fn live_sessions() {
    use chrono::{Duration, Utc};
    use ferrum_app_comms::meetings;
    let url = std::env::var("FERRUM_LMS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    reset(&db).await;
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    packages.extend(ferrum_app_lms::packages().expect("app packages").into_iter().filter(|p| p.name() != "lms_payments"));
    packages.extend(ferrum_app_comms::packages().expect("Comms' packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_lms::register(&mut hooks);
    ferrum_app_comms::register(&mut hooks);
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("domain maintenance");
    let here: Arc<Mutex<Vec<Value>>> = Arc::default();
    let api = room_service(here.clone()).await;
    // SAFETY: the tests of this file run one at a time (`DB`).
    let meeting_server = |on: bool| unsafe {
        for (var, value) in [
            ("FERRUM_MEET_URL", "wss://meet.live.test"),
            ("FERRUM_MEET_KEY", "APIlive"),
            ("FERRUM_MEET_SECRET", "live-secret"),
            ("FERRUM_MEET_API_URL", api.as_str()),
        ] {
            if on {
                std::env::set_var(var, value);
            } else {
                std::env::remove_var(var);
            }
        }
    };
    meeting_server(true);
    let setup = |slug: &'static str, packages: &'static [&'static str]| {
        let k = k.clone();
        async move {
            let tenant = k
                .create_tenant(NewTenant {
                    slug: slug.into(),
                    name: slug.into(),
                    admin_email: format!("admin@{slug}.test"),
                    admin_password: PW.into(),
                    admin_name: None,
                    packages: packages.iter().map(|p| p.to_string()).collect(),
                    settings: Some(json!({"timezone": "Europe/Berlin"})),
                })
                .await
                .expect("tenant");
            let session = k.login(slug, &format!("admin@{slug}.test"), PW).await.expect("login");
            let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
            T { k: k.clone(), tenant, admin, site: k.hooks.site("lms").cloned().expect("site") }
        }
    };
    let t = setup("live", &["lms", "comms"]).await;
    let (teacher, stranger) = (t.user("teacher@live.test", "Tara Teacher", "Instructor").await, t.user("other@live.test", "Omar Other", "Instructor").await);
    let (asha, ben, cy) = (
        t.user("asha@live.test", "Asha Rao", "Learner").await,
        t.user("ben@live.test", "Ben Okafor", "Learner").await,
        t.user("cy@live.test", "Cy Cole", "Learner").await,
    );
    t.insert(&t.admin, "CommsSettings", json!({"public_url": "https://live.test"})).await;
    let course = uid(&t
        .insert(&teacher, "LmsCourse", json!({"title": "Live Basics", "visibility": "Public", "instructors": [{"instructor": "teacher@live.test"}]}))
        .await);
    let (_, lessons) = t.chapter(&teacher, course, "Week one", &[json!({"title": "Q&A", "lesson_type": "Live"}), json!({"title": "Reading"})]).await;
    let (live_lesson, reading) = (lessons[0], lessons[1]);
    t.update(&teacher, "LmsCourse", course, json!({"status": "Published"})).await;
    let today = teacher.today();
    let cohort = uid(&t
        .insert(
            &teacher,
            "LmsCohort",
            json!({"course": course, "title": "Spring", "start_date": today.to_string(), "end_date": (today + Duration::days(60)).to_string(),
                   "instructors": [{"instructor": "teacher@live.test"}]}),
        )
        .await);
    let mut enrolments = vec![];
    for who in [&asha, &ben, &cy] {
        enrolments.push(uid(&t.insert(who, "LmsEnrollment", json!({"course": course, "cohort": cohort.to_string()})).await));
    }
    // A lesson that is not Live cannot be credited by a session.
    let soon = Utc::now() + Duration::minutes(30);
    let wrong =
        t.try_update(&teacher, "LmsCohort", cohort, json!({"sessions": [{"title": "X", "starts_at": soon.to_rfc3339(), "lesson": reading.to_string()}]})).await;
    assert!(field_error(wrong, "lesson").contains("Live"));
    t.update(
        &teacher,
        "LmsCohort",
        cohort,
        json!({"sessions": [
            {"title": "Q&A", "starts_at": soon.to_rfc3339(), "duration_minutes": 60, "lesson": live_lesson.to_string()},
            {"title": "Office hours", "starts_at": (soon + Duration::days(1)).to_rfc3339(), "duration_minutes": 30, "join_url": "https://meet.live.test/office"},
        ]}),
    )
    .await;
    let sessions = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
    let (qa, office) = (vid(&sessions[0]["id"]), vid(&sessions[1]["id"]));
    let made = meetings::find(&t.k, t.tenant, "LmsCohortSession", qa).await.unwrap().expect("a meeting for the session");
    assert_eq!((made.starts_at.timestamp(), (made.ends_at - made.starts_at).num_minutes()), (soon.timestamp(), 60));
    assert!(meetings::find(&t.k, t.tenant, "LmsCohortSession", office).await.unwrap().is_none(), "a session with a link of its own keeps it");
    // Members get their own link on the lesson page, the dashboard and in the reminder.
    let room = format!("https://live.test/meet/room/{}", made.id);
    let page = t.page(Some(&asha), &format!("/lessons/{live_lesson}")).await;
    assert_eq!(
        (page["live"][0]["join_url"].clone(), page["live"][0]["has_meeting"].clone(), page["live"][0]["attended"].clone()),
        (json!(room), json!(true), Value::Null)
    );
    let on_dash = t.page(Some(&asha), "/dashboard").await["upcoming_sessions"].clone();
    let by_title = |title: &str| on_dash.as_array().unwrap().iter().find(|s| s["title"] == json!(title)).cloned().unwrap();
    assert_eq!((by_title("Q&A")["join_url"].clone(), by_title("Office hours")["join_url"].clone()), (json!(room), json!("https://meet.live.test/office")));
    ferrum_app_lms::maintenance(&t.k).await.expect("maintenance");
    let told: i64 =
        t.count("select count(*) from notification where tenant_id = $1 and user_id = $2 and body like $3", asha.user, &format!("%Join: {room}%")).await;
    assert_eq!(told, 1, "the reminder carries the meeting's link");
    // Moved with the session.
    let later = soon + Duration::minutes(15);
    let mut moved = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
    moved[0]["starts_at"] = json!(later.to_rfc3339());
    moved[0]["duration_minutes"] = json!(45);
    t.update(&teacher, "LmsCohort", cohort, json!({"sessions": moved})).await;
    let after = meetings::get(&t.k, t.tenant, made.id).await.unwrap();
    assert_eq!((after.starts_at.timestamp(), (after.ends_at - after.starts_at).num_minutes()), (later.timestamp(), 45));
    // Nothing is read before it is over.
    assert_eq!(t.count("select count(*) from lms_session_attendance where tenant_id = $1 and $2::uuid is not null and $3 = $3", Uuid::nil(), "").await, 0);

    // It is over: Asha was there the whole time, Ben for ten minutes, Cy not at all.
    let began = Utc::now() - Duration::minutes(80);
    t.sql(
        &format!(
            "update e_lms_cohort__sessions set starts_at = '{}', duration_minutes = 60 where tenant_id = $1 and id = '{qa}' and $2::uuid is not null",
            began.to_rfc3339()
        ),
        Uuid::nil(),
    )
    .await;
    t.sql(
        &format!(
            "update comms_meeting set starts_at = '{}', ends_at = '{}' where tenant_id = $1 and id = '{}' and $2::uuid is not null",
            began.to_rfc3339(),
            (began + Duration::minutes(60)).to_rfc3339(),
            made.id
        ),
        Uuid::nil(),
    )
    .await;
    let now = Utc::now().timestamp();
    let identity = |who: &Ctx, n: &str| format!("{}.{n}", who.user.simple());
    *here.lock().unwrap() = vec![
        json!({"identity": identity(&asha, "a1b2c3d4"), "name": "Asha Rao", "joined_at": (now - 3500).to_string()}),
        json!({"identity": identity(&ben, "e5f6a7b8"), "name": "Ben Okafor", "joined_at": (now - 600).to_string()}),
        json!({"identity": "guest.6578616d706c65.11223344", "name": "Outsider (guest)", "joined_at": now - 3000}),
    ];
    ferrum_app_lms::maintenance(&t.k).await.expect("maintenance");
    let report = t.act(&teacher, "LmsCohort", cohort, "session_attendance", json!({"session": qa.to_string()})).await;
    let rows: Vec<(String, bool, i64, String)> = report["learners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["learner"].as_str().unwrap().to_string(),
                r["attended"].as_bool().unwrap(),
                r["minutes"].as_i64().unwrap(),
                r["source"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(rows.len(), 3, "every learner of the cohort, and nobody else: {rows:?}");
    let row = |who: &str| rows.iter().find(|r| r.0 == who).unwrap().clone();
    let (a, b, c) = (row("asha@live.test"), row("ben@live.test"), row("cy@live.test"));
    assert!(a.1 && (57..=59).contains(&a.2) && a.3 == "Meeting", "{a:?}");
    assert!(!b.1 && (9..=11).contains(&b.2), "ten of sixty minutes is under half: {b:?}");
    assert!(!c.1 && c.2 == 0, "{c:?}");
    let credited = |enrolment: Uuid| {
        let t = &t;
        async move {
            t.count("select count(*) from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and completed_via = $3", enrolment, "Live Session")
                .await
        }
    };
    assert_eq!((credited(enrolments[0]).await, credited(enrolments[1]).await, credited(enrolments[2]).await), (1, 0, 0), "attending credits the lesson");
    assert!(meetings::get(&t.k, t.tenant, made.id).await.unwrap().ended_at.is_some(), "the meeting was closed");
    let page = t.page(Some(&asha), &format!("/lessons/{live_lesson}")).await;
    assert_eq!((page["completed"].clone(), page["live"][0]["attended"].clone()), (json!(true), json!(true)));
    ferrum_app_lms::maintenance(&t.k).await.expect("maintenance");
    assert_eq!(credited(enrolments[0]).await, 1, "read once");

    // An instructor marks Ben present by hand: it credits the lesson, and only instructors of the course may.
    assert!(matches!(
        err(t.try_act(&stranger, "LmsCohort", cohort, "mark_attendance", json!({"session": qa.to_string(), "learner": "ben@live.test"})).await),
        Error::Forbidden(_)
    ));
    assert!(matches!(
        err(t.try_act(&ben, "LmsCohort", cohort, "mark_attendance", json!({"session": qa.to_string(), "learner": "ben@live.test"})).await),
        Error::Forbidden(_) | Error::NotFound(_)
    ));
    assert!(
        field_error(
            t.try_act(&teacher, "LmsCohort", cohort, "mark_attendance", json!({"session": qa.to_string(), "learner": "nobody@live.test"})).await,
            "learner"
        )
        .contains("not a learner")
    );
    assert!(
        field_error(
            t.try_act(&teacher, "LmsCohort", cohort, "mark_attendance", json!({"session": Uuid::new_v4().to_string(), "learner": "ben@live.test"})).await,
            "session"
        )
        .contains("not a live session")
    );
    let marked = t.act(&teacher, "LmsCohort", cohort, "mark_attendance", json!({"session": qa.to_string(), "learner": "ben@live.test", "minutes": 50})).await;
    assert_eq!((marked["attended"].clone(), marked["minutes"].clone(), marked["source"].clone()), (json!(true), json!(50), json!("Instructor")));
    assert_eq!(credited(enrolments[1]).await, 0, "the lesson says who credited it");
    assert_eq!(
        t.count("select count(*) from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and completed_via = $3", enrolments[1], "Instructor")
            .await,
        1
    );

    // A session may be taken away: its meeting ends.
    let (extra,) = {
        let mut all = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
        all.push(json!({"title": "Spare", "starts_at": (Utc::now() + Duration::days(3)).to_rfc3339()}));
        t.update(&teacher, "LmsCohort", cohort, json!({"sessions": all})).await;
        let spare = vid(&t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().iter().find(|s| s["title"] == json!("Spare")).unwrap()["id"]);
        (spare,)
    };
    assert!(meetings::find(&t.k, t.tenant, "LmsCohortSession", extra).await.unwrap().is_some_and(|m| m.ended_at.is_none()));
    let mut kept = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
    kept.retain(|s| s["title"] != json!("Spare"));
    t.update(&teacher, "LmsCohort", cohort, json!({"sessions": kept})).await;
    assert!(meetings::find(&t.k, t.tenant, "LmsCohortSession", extra).await.unwrap().is_some_and(|m| m.ended_at.is_some()));

    // Without a meeting server: the typed link and the instructor's marks, as before.
    meeting_server(false);
    let mut plain = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().clone();
    plain.push(json!({"title": "No server", "starts_at": (Utc::now() + Duration::days(4)).to_rfc3339(), "join_url": "https://typed.test/class"}));
    plain.push(json!({"title": "No link", "starts_at": (Utc::now() + Duration::days(5)).to_rfc3339(), "lesson": live_lesson.to_string()}));
    t.update(&teacher, "LmsCohort", cohort, json!({"sessions": plain})).await;
    let bare = t.get("LmsCohort", cohort).await["sessions"].as_array().unwrap().iter().find(|s| s["title"] == json!("No link")).unwrap().clone();
    assert!(meetings::find(&t.k, t.tenant, "LmsCohortSession", vid(&bare["id"])).await.unwrap().is_none());
    ferrum_app_lms::maintenance(&t.k).await.expect("maintenance");
    t.act(&teacher, "LmsCohort", cohort, "mark_attendance", json!({"session": bare["id"], "learner": "cy@live.test"})).await;
    assert_eq!(
        t.count("select count(*) from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and completed_via = $3", enrolments[2], "Instructor")
            .await,
        1
    );

    // A workspace without Comms at all.
    let plain_t = setup("plainco", &["lms"]).await;
    meeting_server(true);
    let tutor = plain_t.user("tutor@plainco.test", "Tess Tutor", "Instructor").await;
    let pupil = plain_t.user("pupil@plainco.test", "Pat Pupil", "Learner").await;
    let course = uid(&plain_t
        .insert(&tutor, "LmsCourse", json!({"title": "Plain", "visibility": "Public", "instructors": [{"instructor": "tutor@plainco.test"}]}))
        .await);
    let (_, lessons) = plain_t.chapter(&tutor, course, "One", &[json!({"title": "Class", "lesson_type": "Live"})]).await;
    plain_t.update(&tutor, "LmsCourse", course, json!({"status": "Published"})).await;
    let cohort = uid(&plain_t
        .insert(
            &tutor,
            "LmsCohort",
            json!({"course": course, "title": "Only", "start_date": today.to_string(), "end_date": (today + Duration::days(9)).to_string(),
                   "instructors": [{"instructor": "tutor@plainco.test"}],
                   "sessions": [{"title": "Class", "starts_at": (Utc::now() + Duration::days(1)).to_rfc3339(), "join_url": "https://typed.test/plain", "lesson": lessons[0].to_string()}]}),
        )
        .await);
    let enrolment = uid(&plain_t.insert(&pupil, "LmsEnrollment", json!({"course": course, "cohort": cohort.to_string()})).await);
    let class = vid(&plain_t.get("LmsCohort", cohort).await["sessions"][0]["id"]);
    assert!(meetings::find(&plain_t.k, plain_t.tenant, "LmsCohortSession", class).await.unwrap().is_none());
    assert_eq!(plain_t.page(Some(&pupil), &format!("/lessons/{}", lessons[0])).await["live"][0]["join_url"], json!("https://typed.test/plain"));
    plain_t.act(&tutor, "LmsCohort", cohort, "mark_attendance", json!({"session": class.to_string(), "learner": "pupil@plainco.test"})).await;
    assert_eq!(
        plain_t
            .count("select count(*) from e_lms_lesson_progress where tenant_id = $1 and enrollment = $2 and completed_via = $3", enrolment, "Instructor")
            .await,
        1
    );
    meeting_server(false);
}
