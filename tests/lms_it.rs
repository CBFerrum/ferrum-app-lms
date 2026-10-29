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

#[test]
fn learn_end_to_end() {
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_LMS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_lms::packages().expect("app packages"));
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
    let nobody = t.user("nobody@learn.test", "No Body", "Sales User").await;
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
    let g1 = t.act(&teacher, "LmsAssignmentSubmission", sub1, "grade", json!({"score": 40, "feedback": "Explain lifetimes too."})).await;
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
    let file = t.call(Some(&chen), "GET", &format!("/lessons/{l3}/file")).await.expect("lesson file");
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
    let public_file = t.call(None, "GET", &format!("/courses/rust-basics/lessons/{l3}/file")).await.expect("preview file");
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
