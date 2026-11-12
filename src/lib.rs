//! Learn: courses, lessons, quizzes, enrolments, progress, certificates and a public course catalog.
//!
//! * Catalog (`catalog`, `video`): courses with chapters and ordered lessons (articles, videos on
//!   allowlisted hosts, documents, quizzes, assignments), learning paths, categories. Only a
//!   course's instructors and LMS managers change its content.
//! * Cohorts (`cohort`): dates, capacity, live sessions laid out in the cohort's time zone, and
//!   reminders sent once per session.
//! * Assessment (`quiz`, `grading`, `assignment`): question banks whose answer keys learners can
//!   never read, attempts with server-side timing, attempt limits that hold under concurrency and
//!   exact scores; assignments graded by instructors with resubmission rules.
//! * Learning (`enrollment`, `certificate`): enrolment rules (published, prerequisites, cohort
//!   capacity), server-computed progress, completion and certificates issued exactly once with a
//!   public verification code.
//! * Endpoints (`site`), notifications (`notice`, through Mail when installed) and reports.
//!
//! Hooks are registered per entity, so a tenant without the `lms` package never runs them.

mod assignment;
mod catalog;
mod certificate;
mod cohort;
mod discussion;
mod enrollment;
mod gradebook;
mod grading;
mod live;
mod lti;
mod notice;
mod payments;
mod quiz;
mod reports;
mod rules;
pub mod scorm;
mod signup;
mod site;
mod util;
mod video;
pub mod xapi;

use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Ctx, Error, HookRegistry, Kernel, Result};
use std::sync::Arc;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/packages.rs"));
include!(concat!(env!("OUT_DIR"), "/ui.rs"));

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

/// Registers this app's document hooks (and web endpoints).
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("LmsSettings", Arc::new(catalog::SettingsHook))
        .register("LmsCourse", Arc::new(catalog::CourseHook))
        .register("LmsChapter", Arc::new(catalog::ChapterHook))
        .register("LmsLesson", Arc::new(catalog::LessonHook))
        .register("LmsLearningPath", Arc::new(catalog::PathHook))
        .register("LmsCohort", Arc::new(cohort::CohortHook))
        .register("LmsEnrollmentRule", Arc::new(rules::RuleHook))
        .register("LmsProgramme", Arc::new(rules::ProgrammeHook))
        .register("LmsQuiz", Arc::new(quiz::QuizHook))
        .register("LmsQuestion", Arc::new(quiz::QuestionHook))
        .register("LmsQuizAttempt", Arc::new(quiz::AttemptHook))
        .register("LmsAssignment", Arc::new(assignment::AssignmentHook))
        .register("LmsRubric", Arc::new(assignment::RubricHook))
        .register("LmsGradeItem", Arc::new(gradebook::ItemHook))
        .register("LmsGradeEntry", Arc::new(gradebook::EntryHook))
        .register("LmsCourse", Arc::new(discussion::CourseHook))
        .register("LmsPost", Arc::new(discussion::PostHook))
        .register("LmsAssignmentSubmission", Arc::new(assignment::SubmissionHook))
        .register("LmsEnrollment", Arc::new(enrollment::EnrollmentHook))
        .register("LmsLessonProgress", Arc::new(enrollment::ProgressHook))
        .register("LmsPackageProgress", Arc::new(scorm::ProgressHook))
        .register("LmsLtiTool", Arc::new(lti::ToolHook))
        .register("LmsCertificate", Arc::new(certificate::CertificateHook))
        .add_site(util::PACKAGE, Arc::new(site::LearnSite))
        // People who signed themselves up on a course page are enrolled in it (or it is bought for them).
        .add_event_sink(Arc::new(signup::SignupSink));
    // The connector with Commerce: only where Paid Courses is installed.
    hooks.scoped(&[payments::PACKAGE.to_string()], |h| {
        h.register("LmsCourse", Arc::new(payments::CourseHook))
            .register("LmsEnrollment", Arc::new(payments::EnrollmentHook))
            .register("PaymentEntry", Arc::new(payments::PaidHook))
            .register("SalesInvoice", Arc::new(payments::ReturnHook))
            .register("LmsCourseBundle", Arc::new(payments::BundleHook));
    });
}

/// The manifest of the connector app with Commerce (`connectors/payments/app.yaml`): its package is
/// `lms_payments`, its code this crate's. It ships with Learn.
pub const CONNECTOR_MANIFEST: &str = include_str!("../connectors/payments/app.yaml");

/// This app's reports.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(reports::LearnReports)]
}

/// Periodic upkeep: every minute, on one replica at a time (advisory lock); must be idempotent and cheap.
/// Per tenant with Learn: closes quiz attempts whose time ran out, sends live-session reminders,
/// completes enrolments that course changes finished, reminds learners of due and overdue enrolments, and purges old reminder bookkeeping. A failing
/// step is logged and does not stop the others; the first error is returned.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    maintenance_at(k, util::now()).await
}

/// [`maintenance`] as of `at`: what is due, overdue or expiring is read against that instant, so a
/// test can step the clock. Steps that watch the database's own clock (attempt deadlines, sessions)
/// are unaffected.
pub async fn maintenance_at(k: &Kernel, at: chrono::DateTime<chrono::Utc>) -> Result<()> {
    let mut tx = k.db.system_tx().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut first_error: Option<Error> = None;
    for tenant in tenants {
        let schema = match k.registry.schema(tenant).await {
            Ok(schema) if schema.packages.iter().any(|p| p == util::PACKAGE) => schema,
            Ok(_) => continue,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };
        let ctx = Ctx::system(tenant, schema);
        let steps = [
            ("attempt deadlines", quiz::expire_attempts(k, &ctx).await),
            ("session reminders", cohort::send_reminders(k, &ctx).await),
            ("live attendance", live::close_ended(k, &ctx).await),
            ("completions", enrollment::complete_ready(k, &ctx).await),
            ("due reminders", enrollment::send_due_reminders(k, &ctx, at).await),
            ("expired certificates", certificate::expire_all(k, &ctx, at).await),
            ("recertification", enrollment::recertify(k, &ctx, at).await),
            ("enrolment rules", rules::apply(k, &ctx, at).await),
            ("lapsed purchases", payments::cancel_lapsed(k, &ctx).await),
            ("purge", purge(k, tenant).await),
            ("lti tokens", lti::purge(k, tenant).await),
        ];
        for (step, outcome) in steps {
            match outcome {
                Ok(n) if n > 0 => tracing::info!(%tenant, step, n, "learn maintenance"),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(%tenant, step, "learn maintenance failed: {e}");
                    first_error.get_or_insert(e);
                }
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Reminder bookkeeping of sessions long past.
async fn purge(k: &Kernel, tenant: Uuid) -> Result<usize> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    let n = sqlx::query("delete from lms_session_reminder where tenant_id = $1 and starts_at < now() - interval '90 days'")
        .bind(tenant)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    #[test]
    fn packages_are_valid() {
        let mut all = ferrum_domain::packages().expect("platform packages");
        all.extend(ferrum_app_erp::packages().expect("ERP's packages"));
        // Paid Courses sell through Commerce's payment requests (its Finance connector is left out here).
        all.extend(ferrum_app_commerce::packages().expect("Commerce's packages").into_iter().filter(|p| p.name() != "commerce_subscriptions"));
        all.extend(super::packages().expect("app packages"));
        let catalog = ferrum_meta::Catalog::from_packages(all).unwrap_or_else(|e| panic!("{e}"));
        let selling = ferrum_meta::effective::build(&catalog, &["lms".into(), "commerce".into(), super::payments::PACKAGE.into()], &[], 1)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(selling.warnings.is_empty(), "{:?}", selling.warnings);
        assert!(selling.entity("LmsCourse").is_some_and(|c| c.field("lpay_price").is_some() && c.def.actions.iter().any(|a| a.name == "buy")));
        assert!(selling.entity("LmsCoursePurchase").is_some(), "the connector's purchases");
        assert!(super::CONNECTOR_MANIFEST.contains("packages: [lms_payments]"));
        let schema = ferrum_meta::effective::build(&catalog, &["lms".to_string()], &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert!(schema.warnings.is_empty(), "{:?}", schema.warnings);
        assert!(schema.entity("LmsCertificate").is_some_and(|e| e.print_template.is_some()), "the certificate print template ships");
        for role in ["LMS Manager", "Instructor", "Learner"] {
            assert!(schema.roles.iter().any(|r| r.name == role), "{role}");
        }
        let learner = schema.roles.iter().find(|r| r.name == "Learner").unwrap();
        for secret in ["LmsQuestion", "LmsQuiz", "LmsLesson", "LmsChapter", "LmsAssignment", "LmsCohort"] {
            assert!(!learner.permissions.iter().any(|p| p.entity == secret || p.entity == "*"), "learners must not read {secret}");
        }
        assert!(super::MANIFEST.contains("packages: [lms]"));
    }

    #[test]
    fn the_interface_is_a_release_the_platform_takes() {
        let m = ferrum_apps::AppManifest::parse(super::MANIFEST).expect("app.yaml");
        assert_eq!(m.routes(), vec![("lms".to_string(), vec![])]);
        assert_eq!(m.public_paths(), vec!["lms/c"], "the public catalog, course pages and certificate check");
        let files = super::UI.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        let ui = ferrum_apps::ui::prepare("lms", files).expect("ui/dist is an interface release");
        assert!(super::UI.iter().any(|(p, _)| *p == "index.html") && !super::UI.iter().any(|(p, _)| p.starts_with('.')), "{}", ui.digest);
    }
}
