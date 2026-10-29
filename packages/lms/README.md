# lms — Learn

Package `lms` (depends on `core`, logic in the `ferrum-app-lms` crate): courses with chapters and
ordered lessons, learning paths, cohorts with live sessions, question banks and timed quizzes,
graded assignments, enrolments with server-computed progress, certificates with public
verification, a public course catalog, notifications, dashboards and reports.

Everything a learner, instructor or AI agent does is a record or a document action of the generic
API (`/api/v1/...`, also over MCP). The package's own endpoints serve the public site and the
aggregated pages a UI needs.

Conventions: times are RFC 3339 UTC with milliseconds (`2026-11-02T09:00:00.000Z`), business dates
`YYYY-MM-DD` in the workspace's time zone, decimals are strings (`"85.5"`). Errors use the usual
API shape (`{"error", "message", "fields"?}`): `not_found` (404), `forbidden` (403), `invalid` /
`validation_failed` (422), `conflict` (409).

## Roles

| Role | Can |
|---|---|
| LMS Manager | everything in Learn, including Learn Settings, deleting attempts and revoking certificates |
| Instructor | create courses; edit, delete and enrol learners in the courses they teach (listed as instructor, or created) and edit their chapters, lessons, question banks, quizzes, assignments and cohorts; read every enrolment, attempt, submission and progress row; grade submissions of their courses; create learning paths (edit their own). Give the user `LmsCourse` record scopes to limit what they see to some courses |
| Learner | read the catalog (`LmsCourse`, `LmsCategory`, `LmsLearningPath`); enrol themselves; read **only their own** enrolments, lesson progress, quiz attempts, submissions and certificates; create and edit their own submissions |

Learners have **no** permission on `LmsChapter`, `LmsLesson`, `LmsQuiz`, `LmsQuestion`,
`LmsAssignment` or `LmsCohort`: course content reaches them through their enrolment's actions and
the signed-in endpoints, which check the enrolment (or a free preview). Whatever a role grants,
hooks check that writes to course content come from the course's instructors (or its creator, or
an LMS manager).

## Setup — Learn Settings (`LmsSettings`)

| Field | Meaning |
|---|---|
| `public_url` | Address the server is reached at from the internet (e.g. `https://erp.example.com`); certificates print `{public_url}/api/site/{tenant}/lms/certificates/{code}` as their verification link |
| `email_notifications` | Default on: also email people when the Mail app is installed |
| `session_reminder_minutes` | Default 60 (5–10080): how long before a live session its reminder goes out |
| `certificate_issuer` | Organization named on certificates (the workspace name when empty) |
| `certificate_signatory`, `certificate_signatory_title` | Printed under the signature line |
| `video_hosts` | Extra video hosts, one per line (`video.example.com`; subdomains included): https links on them are embedded as they are |

## Entities

### Catalog

- **LmsCategory** — `category_name` (unique), `description`. `General` is seeded.
- **LmsCourse** — `title`, `slug` (unique; lower-case letters, digits and hyphens; made from the
  title when empty, with `-2`, `-3`... when taken), `status` (`Draft` / `Published` / `Archived`),
  `visibility` (`Internal` / `Public`: public courses are on the public catalog once published),
  `category`, `level` (`Beginner` / `Intermediate` / `Advanced`), `tags` (comma-separated, stored
  normalized: `rust, safety`), `summary`, `description` (markdown), `cover_image` (upload it as a
  public file for the public catalog), `instructors` (rows `instructor` = a user's email,
  `instructor_name` filled in), `prerequisites` (rows `course`: courses a learner must have
  completed to enrol themselves; cycles are refused), `estimated_hours`, `self_enrollment`
  (default on), `issue_certificate`; read-only `published_at`, `lesson_count`, `total_minutes`.
  Publishing needs at least one lesson, and every quiz lesson a quiz with questions. Instructors
  own a course through `instructors` (owner field).
- **LmsChapter** — `course`, `title`, `position` (next free when empty), `summary`. A chapter with
  lessons cannot move to another course or be deleted.
- **LmsLesson** — `chapter` (its `course` is derived), `title`, `position` (next free in the chapter
  when empty), `lesson_type` (`Article`, `Video`, `Document`, `Quiz`, `Assignment`), `content`
  (markdown, any type), `video_url` (required for videos; https on YouTube, Vimeo, Loom, Wistia,
  Google Drive or a host from Learn Settings — anything else is refused) with the derived player
  URL `video_embed_url`, `attachment` (required for documents: a `/files/{id}` upload or an https
  link), `quiz` / `assignment` (required for those types; of the same course; each used by one
  lesson), `duration_minutes`, `is_preview` (free preview), `required` (default on). A lesson some
  learner completed cannot be deleted (make it optional instead). Changing `required`, adding or
  deleting lessons recomputes every enrolment's progress.
- **LmsLearningPath** — `title`, `status`, `summary`, `description`, `courses` (ordered rows, each
  course once, at most 50).
- **LmsCohort** — `course`, `title`, `start_date`, `end_date`, `time_zone` (IANA; the workspace's
  when empty), `capacity` (0 = unlimited; withdrawn learners do not count), `instructors`,
  `sessions` (rows `title`, `starts_at`, `duration_minutes`, `join_url` (https), `notes`). Members
  are the enrolments that name the cohort. A cohort with learners cannot move to another course.

### Assessment

- **LmsQuestion** — the question bank of a `course`, with its answer key: `question` (markdown),
  `question_type` (`Single Choice`, `Multiple Choice`, `True/False`, `Short Answer`), `points`
  (default for quizzes), `options` (rows `option`, `is_correct`: 2–20 distinct options; exactly one
  correct for single choice and true/false, at least one for multiple choice; true/false questions
  always get exactly `True` and `False`), `accepted_answers` (rows `answer`, for short answers,
  1–50), `case_sensitive`, `explanation`. Learners can never read it. A question learners have
  answered cannot change its type.
- **LmsQuiz** — `course`, `title`, `description` (instructions), `passing_percentage` (default 70),
  `max_attempts` (0 = unlimited), `time_limit_minutes` (0 = none), `questions_per_attempt` (draw this
  many at random per attempt; 0 = all), `shuffle_questions`, `shuffle_options`, `show_answers`
  (`After Each Attempt`, `After Last Attempt` — once the learner passes or uses the last attempt —
  or `Never`), `questions` (rows `question` from the course's bank, `points` defaulting to the
  question's); read-only `question_count`, `total_points`.
- **LmsAssignment** — `course`, `title`, `instructions`, `submission_type` (`Text`, `File`,
  `Text or File`), `max_score` (default 100), `passing_score` (0 = every graded submission passes),
  `due_date` **or** `due_days` (after each learner's enrolment), `accept_late` (default on),
  `allow_resubmission` (after a failing grade; default on), `max_submissions` (0 = unlimited).

### Learning

- **LmsEnrollment** (`ENR-YYYY-000001`) — `course`, `learner` (a user's email; yours when empty),
  `cohort`; read-only `learner_name`, `status` (`Active`, `Completed`, `Withdrawn`), `enrolled_at`,
  `enrolled_on`, `enrolled_by`, `progress_percent`, `completed_lessons`, `total_lessons` (required
  lessons; all lessons when none is required), `last_lesson`, `last_activity_at`, `completed_at`,
  `certificate`. One per learner and course. Rules on create:
  - the learner must be an active user; enrolling someone else takes the course's instructors or an
    LMS manager (`403` otherwise);
  - archived courses take nobody; drafts only from their instructors; learners enrolling themselves
    need a published course with `self_enrollment` (`422 enrolment ... is by invitation`) and every
    prerequisite completed (`422` on `course`: `complete these courses first: ...`);
  - the cohort must be the course's, not ended (for self-enrolment) and have a free place — the
    cohort row is locked while counting, so concurrent requests never overfill it (`409 ... is full`);
  - a second enrolment in the same course is `409`.
  Instructors may change `cohort` later (capacity checked); `course` and `learner` never change.
- **LmsLessonProgress** — one row per completed lesson of an enrolment (`enrollment`, `lesson`,
  `lesson_title`, `course`, `learner`, `completed_at`, `completed_via`: `Learner`, `Instructor`,
  `Quiz`, `Assignment`, `completed_by`). Written by Learn only.
- **LmsQuizAttempt** (`QA-YYYY-000001`) — one sitting: `quiz`, `quiz_title`, `course`,
  `enrollment`, `learner`, `attempt_no`, `status` (`In Progress`, `Submitted`, `Timed Out`),
  `started_at`, `deadline` (server time; empty without a limit), `submitted_at`,
  `time_limit_minutes`, `passing_percentage`, `max_score`, `score`, `percentage`, `passed`, and
  `answers` — rows `question`, `question_text`, `question_type`, `options` (`[{key, text}]` in the
  order shown), `points`, `response`, and after grading `is_correct`, `points_awarded`, plus
  `correct_answer` and `explanation` when the quiz reveals them. Created by `start_quiz` only.
- **LmsAssignmentSubmission** (`SUB-YYYY-000001`) — `assignment`, `answer` (markdown) and/or
  `attachment` (a `/files/{id}` the learner uploaded; it becomes the submission's file, readable by
  the course's instructors); read-only `course`, `enrollment`, `learner`, `submission_no`,
  `submitted_at`, `due_date`, `late`, `status` (`Submitted`, `Graded`, `Needs Revision`), `score`,
  `max_score`, `passed`, `feedback`, `graded_by`, `graded_at`. The learner creates it
  (`POST /api/v1/LmsAssignmentSubmission {assignment, answer?, attachment?}`) and may edit it while
  it is `Submitted`. Refused (`422`): not enrolled, a submission waiting to be graded (edit it),
  already passed, failed without `allow_resubmission`, `max_submissions` used (work returned for
  revision may always come back), past the deadline without `accept_late`. One ungraded submission
  per learner is also a database rule.
- **LmsCertificate** (`CERT-YYYY-000001`, gapless) — `enrollment` (unique), `course`,
  `course_title`, `learner`, `learner_name`, `issued_on`, `issued_at`, `verification_code` (160
  random bits, Crockford base32, `XXXX-XXXX-...-XXXX`), `verification_url`, `issuer`, `signatory`,
  `signatory_title`, `status` (`Valid`, `Revoked`), `revoked_reason`. Issued by Learn only, once per
  enrolment (under the enrolment's row lock; the unique key is the last word). The print format
  `GET /api/v1/LmsCertificate/{id}/print` is an A4-landscape certificate (print or save as PDF);
  tenants may restyle it with a `set_print_template` customization.

## Actions — `POST /api/v1/{Entity}/{id}/actions/{action}`

| Entity · action | Who | Body → result |
|---|---|---|
| `LmsCourse` · `enroll_learners` | the course's instructors, LMS managers | `{learners: [emails] \| "a@x, b@y", cohort?}` (≤ 500) → `{enrolled: [{learner, enrollment, number}], already_enrolled: [{learner, enrollment: {id, status}}], skipped: [{learner, reason}]}`. Instructors bypass prerequisites and self-enrolment; capacity still holds |
| `LmsLearningPath` · `join` | anyone who reads it (published paths) | `{}` → `{enrolled, already_enrolled, skipped: [{course, course_title, reason}]}`: self-enrolment in each course, rules applied |
| `LmsCohort` · `schedule_sessions` | the course's instructors | `{first_date, time: "HH:MM", count? (1–100, default 1), every_days? (default 7), duration_minutes? (default 60), title? ("Live session"), join_url?}` → `{time_zone, scheduled: [{title, starts_at, local_start, duration_minutes}]}`: the same wall-clock time in the cohort's zone, across daylight-saving changes (a time in a spring-forward gap moves to the first valid minute) |
| `LmsEnrollment` · `complete_lesson` | the learner (articles, videos, documents); the course's instructors (any lesson) | `{lesson}` → `{enrollment, status, progress_percent, completed_lessons, total_lessons, completed_at, certificate, newly_completed}`. Idempotent. The last required lesson completes the enrolment and issues the certificate |
| `LmsEnrollment` · `open_lesson` | the learner, the course's instructors | `{lesson}` → `{lesson: {id, title, lesson_type, chapter, course, content, video_url, video_embed_url, file_url, duration_minutes, is_preview, required}, completed, completed_at, quiz?: QuizState, assignment?: AssignmentState}`; makes it the resume point |
| `LmsEnrollment` · `start_quiz` | the learner | `{quiz}` → `{resumed, attempt}`: the attempt in progress if any (an expired one is closed first), else a new one — `422` when `max_attempts` are used. Runs under the enrolment's row lock, so concurrent starts share one attempt and the limit holds |
| `LmsEnrollment` · `outline` | the learner, the course's instructors | `{}` → `{course, progress, next_lesson, chapters: [{id, title, summary, lessons: [{id, title, lesson_type, duration_minutes, is_preview, required, quiz, assignment, completed, completed_at}]}]}` |
| `LmsEnrollment` · `withdraw` / `reactivate` | the learner, the course's instructors | `{}` → `{enrollment}`. Coming back checks self-enrolment rules (for learners) and the cohort place (`409` when full); progress is kept |
| `LmsQuizAttempt` · `save_answers` | the learner, while `In Progress` | `{answers: {question_id: response}}` → `{attempt}` (with `seconds_left`) |
| `LmsQuizAttempt` · `submit_answers` | the learner, while `In Progress` | `{answers?}` → `{attempt, status, score, max_score, percentage, passed, late_answers_ignored}` |
| `LmsQuizAttempt` · `review` | the course's instructors | `{}` → every question with the learner's answer (as option texts), the correct answer, `is_correct` and the explanation — whatever the quiz reveals to learners |
| `LmsAssignmentSubmission` · `grade` | the course's instructors | `{score, feedback?, outcome?: "Graded" \| "Needs Revision"}` → `{submission}`. `score` is required to grade (0 to `max_score`); a passing grade completes the assignment's lessons |
| `LmsCertificate` · `revoke` | LMS managers | `{reason}` → `{certificate}`; verification then shows `Revoked` |

**Responses**: a single choice or true/false answer is one option key (`"k"` or `["k"]`), multiple
choice a list of keys, short answer a string (≤ 500 characters); `null` or an empty list unanswers.
Option keys are per attempt (a keyed hash, so they reveal nothing about the options' order or
identity). **Scoring**: a question scores its points when it is exactly right — the one correct
option, exactly the set of correct options (no partial credit), or an accepted short answer
(spaces collapsed, case ignored unless the question is case-sensitive). `percentage` is
score / max × 100 rounded to 2 places; `passed` compares exactly (`score × 100 ≥ passing % × max`),
so 66.666…% fails a 66.67% mark. **Time**: the deadline is fixed when the attempt starts; answers
up to 15 seconds after it (network latency) count, later ones close the attempt as `Timed Out`
with the answers saved in time, and maintenance does the same for attempts nobody submits.

**Answer secrecy**: before an attempt is submitted nothing a learner can read — the attempt
(get, list, filters, export, print, audit trail, `_calc`), lesson pages, MCP — contains which
option is right, accepted answers or explanations; the answer key lives only in `LmsQuestion`.
After submission learners see their score and which questions they got right; correct answers and
explanations follow the quiz's `show_answers`.

**QuizState**: `{id, title, description, passing_percentage, time_limit_minutes, questions,
max_attempts, attempts_used, attempts_left, best_percentage, passed, in_progress: {attempt, deadline} | null}`.
**AssignmentState**: `{id, title, instructions, submission_type, max_score, passing_score,
due_date, overdue, accept_late, allow_resubmission, max_submissions, passed, can_submit, reason,
submissions: [{id, number, submission_no, status, score, passed, submitted_at, late, feedback}]}`.

### A learner through the generic API (or MCP)

1. `GET /api/v1/LmsCourse?filters=[["status","=","Published"]]` — the catalog.
2. `POST /api/v1/LmsEnrollment {course, cohort?}` — enrol.
3. `POST /api/v1/LmsEnrollment/{id}/actions/outline` — chapters, lessons, what is done, what is next.
4. `.../actions/open_lesson {lesson}`, then `.../actions/complete_lesson {lesson}`.
5. `.../actions/start_quiz {quiz}` → `POST /api/v1/LmsQuizAttempt/{attempt}/actions/save_answers` /
   `submit_answers`.
6. `POST /api/v1/LmsAssignmentSubmission {assignment, answer}` (upload files with `POST /api/files` first).
7. `GET /api/v1/LmsCertificate` and `GET /api/v1/LmsCertificate/{id}/print`.

## Signed-in endpoints — `/api/app/lms/...`

Bearer auth as everywhere; the caller needs the Learner, Instructor or LMS Manager role (or is an
administrator), else `403`. All are `GET`; anything else is `404`.

| Path | Query → response |
|---|---|
| `/catalog` | `q` (title, summary, tags; ≤ 100 chars), `category` (id), `level`, `tag`, `limit` (1–50, default 20), `cursor` → `{courses: [Card + my_enrollment: {id, status, progress_percent} \| null], next_cursor, learning_paths: [{id, title, summary, courses}]}` — published courses, internal and public |
| `/courses/{slug}` | → `{course: Page + can_enroll, enroll_blockers: [text], is_author, my_enrollment?, next_lesson?, upcoming_sessions?}` — published courses, or any course you author or are enrolled in (`404` otherwise). With an enrolment the chapters carry your `completed` flags and the sessions of your cohort (join links included) |
| `/lessons/{id}` | → the `open_lesson` view (content, quiz/assignment state) for enrolled learners (not withdrawn), the course's authors, and free previews of published courses; `404` otherwise. Opening it as a learner makes it the resume point |
| `/lessons/{id}/file` | → the lesson's uploaded file to the same people (PDFs and images inline, other types as downloads, `nosniff`, sandboxed) |
| `/dashboard` | → `{courses: [{enrollment, course: {id, slug, title, cover_image}, status, progress_percent, completed_lessons, total_lessons, enrolled_on, last_activity_at, completed_at, next_lesson, cohort, certificate: {id, number, verification_code} \| null}], due_assignments: [{assignment, title, course: {id, title}, lesson, due_date, status: "Not Submitted" \| "Submitted" \| "Needs Revision" \| "Failed", overdue}], upcoming_sessions: [Session], certificates: [{id, number, course_title, verification_code}], learning_paths: [{id, title, courses, completed, progress_percent}]}` — in-progress courses first |
| `/teaching` | → `{courses: [{id, slug, title, status, enrolled, active, completed, completion_rate, average_progress, submissions_to_grade}]}` — courses you author (every course for LMS managers), within your record scopes |
| `/teaching/courses/{course}/roster` | `cohort?` → `{course, enrolled, completed, completion_rate, learners: [{enrollment, number, learner, learner_name, status, progress_percent, completed_lessons, total_lessons, enrolled_on, last_activity_at, completed_at, cohort, certificate, quizzes_passed, best_quiz_percentage, submissions_to_grade}]}` — the course's authors only (`403`) |

**Session**: `{session, title, starts_at, local_start (in the cohort's zone), time_zone,
duration_minutes, join_url, cohort, cohort_title, course_title}`.

## Public endpoints — `/api/site/{tenant}/lms/...`

No sign-in; `{tenant}` is the workspace slug. Read-only (`GET`; anything else `404`). Anything not
public answers exactly like something missing (`404`). Rate limited per address by the server.

| Path | Response |
|---|---|
| `/courses` | `q`, `category`, `level`, `tag`, `limit` (1–50), `cursor` → `{courses: [Card], next_cursor}` — **published public courses only**, newest first. `Cache-Control: public, max-age=60` |
| `/categories` | → `{categories: [{id, name, courses}]}` with public published courses |
| `/courses/{slug}` | → `{course: Page}` for a published public course |
| `/courses/{slug}/lessons/{id}` | → `{lesson: {id, title, lesson_type, duration_minutes, content, video_embed_url, file_url}}` for **free-preview lessons only** |
| `/courses/{slug}/lessons/{id}/file` | → a free-preview lesson's file |
| `/certificates/{code}` | → `{certificate: {valid, status, number, learner_name, course_title, issued_on, issuer, verification_code}}`. Codes are accepted in any case, with or without dashes or spaces, `O`/`I`/`L` read as `0`/`1`. `Cache-Control: no-store` |
| `/certificates/{code}/view` | → the certificate as an HTML page (the print template; no scripts, `default-src 'none'`), to show, print or save as PDF |

**Card**: `{id, slug, title, summary, cover_image, level, tags: [..], estimated_hours, lessons,
total_minutes, certificate, published_at, visibility, category: {id, name} | null, instructors: [names]}`.
**Page**: Card fields plus `description`, `self_enrollment`, `prerequisites: [{id, slug, title}]`
(public ones only on the public site), `cohorts: [{id, title, start_date, end_date, places:
{capacity, taken, free}}]` (not ended) and `chapters: [{id, title, summary, lessons: [{id, title,
lesson_type, duration_minutes, is_preview, required}]}]` — titles, never content.

## Notifications

In-app notifications always; with the Mail app installed and `email_notifications` on, also an
email per recipient queued as a `Communication` (`channel: Email`, `reference_entity` /
`reference_id` = the record) in the same transaction — Mail delivers it after commit.

| When | Who | Title |
|---|---|---|
| enrolled (self or by someone) | the learner | `Enrolled: {course}` |
| a submission is handed in | the course's instructors | `New submission: {assignment}` |
| a submission is graded or returned | the learner | `Assignment graded: {assignment}` |
| a course is completed | the learner | `Certificate issued: {course}` (with its number and code), or `Course completed: {course}` |
| a certificate is revoked | the learner | `Certificate revoked: {course}` |
| a live session is near | cohort members (not withdrawn), cohort and course instructors | `Live session soon: {session}` — once per session start |

## Reports (`/api/reports/{name}`)

Each runs on the caller's visible rows of its entity: learners see only themselves, instructors
with course scopes only those courses.

| Name | Entity | Params | Rows |
|---|---|---|---|
| `lms_course_completion` | LmsEnrollment | `course`, `category`, `from_date` / `to_date` (enrolled on) | per course: `enrolled` (not withdrawn), `active`, `completed`, `withdrawn`, `completion_rate` (completed / enrolled %), `average_progress`, `certificates`; totals |
| `lms_quiz_scores` | LmsQuizAttempt | `course`, `quiz`, `from_date` / `to_date` (submitted) | per quiz: `attempts`, `learners`, `average`, `lowest`, `highest`, `pass_rate`, `learners_passed`, and attempts per 10-point band `pct_0_10` … `pct_90_100` |
| `lms_learner_progress` | LmsEnrollment | `course`, `cohort`, `status` | per enrolment: learner, course, cohort, status, progress, lessons done / required, dates, quizzes passed, submissions to grade, certificate |
| `lms_overdue_assignments` | LmsEnrollment | `course`, `as_of` (default today) | active learners whose work for an assignment lesson is missing, failed or returned after its due date: `due_date`, `days_overdue`, `status` |

## Maintenance (every minute, one replica at a time)

Per workspace with Learn: closes attempts past their deadline (plus 15 s) as `Timed Out` with the
answers saved in time; sends live-session reminders (each session start claimed in
`lms_session_reminder` before anyone is told, so concurrent runs cannot double-send); completes
enrolments that course changes brought to 100 % and issues certificates owed to completed
enrolments of courses that give them; purges reminder bookkeeping older than 90 days. Each step
reads only what is due through partial indexes, and a failing step does not stop the others.

## Limits

- Learners can list course metadata (`LmsCourse`) of drafts too; content stays behind enrolment.
- Instructors read every enrolment, attempt and submission unless limited by `LmsCourse` scopes.
- Certificates are HTML print formats (print or save as PDF in the browser); no server-side PDF.
- Paid courses, SCORM/xAPI packages and discussion forums are not part of this release.
