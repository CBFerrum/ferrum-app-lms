# Changelog

## Unreleased

- Personal data: a person's request now reaches their enrollments, progress, attempts, submissions and certificates, which an erasure blanks.
- Email notifications no longer need the Mail app: without it they go through the server's own
  mailer.

## 0.1.0

First release of Learn: courses with chapters and ordered lessons (articles, allowlisted videos,
documents, quizzes, assignments; free previews and required lessons), categories, prerequisites,
learning paths and cohorts with capacity and live sessions laid out in the cohort's time zone;
question banks whose answer keys learners never reach, timed quizzes with exact scoring, attempt
limits enforced under concurrency and configurable answer reveal; graded assignments with
revision and resubmission rules; self and bulk enrolment, server-computed progress, completion and
certificates issued exactly once with public verification and a printable template; a public
catalog, learner and instructor dashboards, in-app and Mail notifications (enrolment, grading,
certificates, session reminders sent once), four reports, and an end-to-end test of it all against
PostgreSQL.
