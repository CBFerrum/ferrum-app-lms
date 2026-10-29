# Ferrum Learn

A native app for [Ferrum](https://github.com/CBFerrum/Ferrum), the open-source, AI-first ERP.

- Courses with chapters and ordered lessons — articles, videos on allowlisted players (YouTube, Vimeo, Loom, Wistia, Google Drive, your own hosts), documents, quizzes and assignments — with free previews, required lessons, prerequisites, categories, levels and tags
- Learning paths, and cohorts with capacity and live sessions scheduled in the cohort's time zone (daylight-saving safe), with one reminder per session
- Question banks whose answer keys learners can never read; quizzes with single choice, multiple choice, true/false and short answers, points, pass marks, attempt limits that hold under concurrency, server-side time limits, random draws and shuffling, and answers revealed only when the quiz says so
- Assignments with text or file submissions, instructor grading, feedback, revision requests and resubmission rules, deadlines fixed or relative to enrolment
- Self-enrolment (published courses, prerequisites, cohort places) and bulk enrolment by instructors; progress computed by the server; completion when every required lesson is done and required quizzes are passed
- Certificates issued exactly once, numbered, printable as PDF from a print template, with a public verification code
- A public course catalog and course pages, learner and instructor dashboards, in-app and email notifications (through Mail when installed, otherwise the server's mailer), and reports on completion, quiz scores, learner progress and overdue work
- `packages/lms/README.md` documents every entity, action, endpoint, setting and maintenance job

## Install

Workspace administrators install it from **Apps** in Ferrum; it ships in the official Ferrum
server image. Self-hosters building their own image enable the `lms` feature of the server.
Everything it adds is also available to AI agents over Ferrum's MCP server.

## Develop

Clone it next to the platform and point the platform dependencies at your checkout (this file
is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-calc = { path = "../ferrum/crates/calc" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role — row-level security must apply:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_lms_test
```

```bash
cargo test
```

`FERRUM_LMS_TEST_DATABASE_URL` overrides the database URL. The app's metadata lives in `packages/` and is embedded
at build time; `app.yaml` is its manifest (see Ferrum's `docs/plan/apps.md`).

## License

AGPL-3.0-only, like the platform.
