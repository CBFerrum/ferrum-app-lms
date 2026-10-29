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
mod enrollment;
mod grading;
mod notice;
mod quiz;
mod reports;
mod site;
mod util;
mod video;

use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Ctx, Error, HookRegistry, Kernel, Result};
use std::sync::Arc;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/packages.rs"));

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
        .register("LmsQuiz", Arc::new(quiz::QuizHook))
        .register("LmsQuestion", Arc::new(quiz::QuestionHook))
        .register("LmsQuizAttempt", Arc::new(quiz::AttemptHook))
        .register("LmsAssignment", Arc::new(assignment::AssignmentHook))
        .register("LmsAssignmentSubmission", Arc::new(assignment::SubmissionHook))
        .register("LmsEnrollment", Arc::new(enrollment::EnrollmentHook))
        .register("LmsLessonProgress", Arc::new(enrollment::ProgressHook))
        .register("LmsCertificate", Arc::new(certificate::CertificateHook))
        .add_site(util::PACKAGE, Arc::new(site::LearnSite));
}

/// This app's reports.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(reports::LearnReports)]
}

/// Periodic upkeep: every minute, on one replica at a time (advisory lock); must be idempotent and cheap.
/// Per tenant with Learn: closes quiz attempts whose time ran out, sends live-session reminders,
/// completes enrolments that course changes finished, and purges old reminder bookkeeping. A failing
/// step is logged and does not stop the others; the first error is returned.
pub async fn maintenance(k: &Kernel) -> Result<()> {
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
            ("completions", enrollment::complete_ready(k, &ctx).await),
            ("purge", purge(k, tenant).await),
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
        all.extend(super::packages().expect("app packages"));
        let catalog = ferrum_meta::Catalog::from_packages(all).unwrap_or_else(|e| panic!("{e}"));
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
}
