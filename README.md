<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Learn: courses, quizzes, certificates and compliance training" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.9.1-4f46e5?style=flat-square)](CHANGELOG.md)
[![CI](https://github.com/CBFerrum/ferrum-app-lms/actions/workflows/ci.yml/badge.svg)](https://github.com/CBFerrum/ferrum-app-lms/actions/workflows/ci.yml)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server-b7410e?style=flat-square&logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![React](https://img.shields.io/badge/React%20%2B%20TypeScript-interface-087ea4?style=flat-square&logo=react&logoColor=white)
![Tests](https://img.shields.io/badge/tests-49%20Rust%20%2B%20155%20UI-16a34a?style=flat-square)
![MCP](https://img.shields.io/badge/AI%20agents-MCP-8b5cf6?style=flat-square)

### Courses, quizzes and certificates for your team, your customers or your students, on the same data as the rest of the business.

Ferrum Learn is a learning management system. Instructors build courses with chapters and lessons. Learners take them in a
course player, sit timed quizzes, hand in assignments and earn certificates that anyone can verify. Managers assign
compliance training with due dates, and courses can be sold to people outside the workspace through Ferrum's invoices.

[💡 Why](#-why-learn) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

## 💡 Why Learn

- 🔒 **Quizzes that cannot be cheated from the browser.** The server keeps the answer keys, holds the clock and decides every
  score. A learner can never read which option is right before submitting, and attempt limits hold under concurrent requests.
- 🎓 **Certificates issued exactly once,** numbered, printable and verifiable by a public code. They can expire, and the learner
  is enrolled again before they do.
- 🧾 **Sales run through the ledger.** A paid course is an ERP sales invoice, a payment request and, when someone withdraws in time, a
  credit note and a refund. There is no separate shop to reconcile.
- 🏢 **Compliance without a second system.** Due dates, reminders, overdue states, enrolment rules by role, group or HR employee
  fields, training programmes and a manager's team report.
- 📦 **Open standards.** SCORM 1.2 and 2004 packages, xAPI with an endpoint of its own, and LTI 1.3 tools with grades sent back.
- 🤖 **Every record is open to AI agents** over Ferrum's REST and MCP APIs, under each person's permissions.
- 🔏 **Privacy built in.** Personal-data erasure is declared record by record, including discussions, and row-level security
  keeps learners to their own records.
- 🔓 **Open source and self-hosted.** AGPL-3.0, on your own PostgreSQL, with no price per learner.
- 🌍 **Arabic and Hindi** words and names, with right-to-left layout.

## 🎯 At a glance

| | |
|---|---|
| **Version** | 0.9.1 (Paid Courses connector 0.2.0) |
| **Kind** | Its own interface at `/lms/`: course player, quiz player, catalog, teaching dashboards and a course builder; public catalog and certificate check; "My learning" on the portal. Other records use Ferrum's lists and forms |
| **What it adds** | 45 record definitions: 27 records and 18 line tables, in 2 packages (Learn 42, Paid Courses 3); 4 roles (one for the portal); 7 reports; 1 print format (the certificate); 3 message templates; 1 menu |
| **Depends on** | Ferrum platform `>=0.3.2, <0.4`. Learn needs nothing else. Paid Courses needs Commerce `>=0.7` |
| **Works with** | Commerce and ERP (sales), Comms (live meetings), HR (enrolment rules, team reports, training records), Forms and Studio |
| **Languages** | English, Arabic, Hindi (words and record names); right-to-left layout |
| **Tests** | 49 Rust tests (16 of them walk the app against PostgreSQL) and 155 interface tests |
| **Licence** | AGPL-3.0-only; the vendored SCORM runtime is MIT |

## 📸 Screenshots

<div align="center">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-my-learning-dark.webp"><img alt="My learning" src="docs/assets/screens/lms-my-learning-light.webp" width="100%"></picture>
<p><b>🎓 My learning</b><br><sub>Where to carry on with a course, what is due, the live classes coming up and the certificates earned.</sub></p>
</div>

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-catalog-dark.webp"><img alt="The course catalog" src="docs/assets/screens/lms-catalog-light.webp"></picture>
<p><b>🧭 The course catalog</b><br><sub>Browse published courses and learning paths, filtered by category and level, and see where you stand in each.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-course-dark.webp"><img alt="A course page" src="docs/assets/screens/lms-course-light.webp"></picture>
<p><b>📖 A course page</b><br><sub>A course&#x27;s outline, who teaches it and the one thing to do next: enrol, continue or see the certificate.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-teaching-dark.webp"><img alt="Teaching" src="docs/assets/screens/lms-teaching-light.webp"></picture>
<p><b>🧑‍🏫 Teaching</b><br><sub>The courses you teach, how many are learning or finished, and how much work waits to be graded.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-lms-course-list-dark.webp"><img alt="Courses" src="docs/assets/screens/lms-lms-course-list-light.webp"></picture>
<p><b>📚 Courses</b><br><sub>Courses with their category, level, status and visibility.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-lms-enrollment-list-dark.webp"><img alt="Enrolments" src="docs/assets/screens/lms-lms-enrollment-list-light.webp"></picture>
<p><b>🙋 Enrolments</b><br><sub>Every enrolment with its learner, course, status, due date and progress.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-report-lms-course-completion-dark.webp"><img alt="Course completion report" src="docs/assets/screens/lms-report-lms-course-completion-light.webp"></picture>
<p><b>📊 Course completion report</b><br><sub>How many learners enrolled in each course and how many finished it.</sub></p>
</td>
</tr>
</table>

### 📱 On a phone

<table>
<tr>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-my-learning-phone-dark.webp"><img alt="My learning on a phone" src="docs/assets/screens/lms-my-learning-phone-light.webp" width="260"></picture>
<p><b>🎓 My learning on a phone</b><br><sub>Continue a course from a phone.</sub></p>
</td>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/lms-lms-course-list-phone-dark.webp"><img alt="Courses on a phone" src="docs/assets/screens/lms-lms-course-list-phone-light.webp" width="260"></picture>
<p><b>📚 Courses on a phone</b><br><sub>The course list on a narrow screen.</sub></p>
</td>
</tr>
</table>


## ✨ Features

> [!NOTE]
> Learn 0.7 to 0.9 added a lot behind the server first: live classes, compliance, selling, more question types, grading, a
> gradebook, discussions and uploaded video. Where a feature has no screen of its own in the learner's player yet, this
> page says so. It is available through the records screens and the REST and MCP APIs.

### 📚 Courses and lessons

- **Courses.** A course has a title, a slug used in links, a status (*Draft, Published, Archived*), a visibility (*Internal* or
  *Public*), a category, a level (*Beginner, Intermediate, Advanced*), tags, a summary, a description, a cover image and estimated hours.
  Learners enrol in published courses; archived courses take no new enrolments.
- **Instructors.** A course lists its instructors by the email they sign in with. An instructor edits only the courses they own, and an LMS
  manager edits all.
- **Chapters and ordered lessons.** Chapters and lessons have positions and move by drag and drop in the builder, or by buttons for
  keyboards and touch screens.
- **Nine lesson types.** *Article* (Markdown), *Video*, *Document*, *Quiz*, *Assignment*, *SCORM*, *xAPI*, *LTI* and *Live*.
- **Previews and required lessons.** A lesson can be a free preview, visible on the public catalog, and can be required or optional for completion.
- **Prerequisites.** A course can require other courses first. Instructors can enrol learners past a prerequisite.
- **Self-enrolment** can be switched on or off for each course.
- **Learning paths.** A path bundles courses. Joining a path enrols the learner in every course, with the rules applied.
- **Categories.** Course categories organise the catalog.
- **Import and export.** Managers can import and export courses, chapters, lessons and questions through the records screens.

### 🎬 Video

- **Hosted video in its own player.** YouTube, Vimeo, Loom and Wistia links, Google Drive, and any host an administrator adds in
  Learn Settings are framed in the host's embedded player. Other links get a plain "open in a new tab" link.
- **Uploaded video (API).** A video lesson can hold a video file (up to *Largest Video (MB)* in the settings, 2,048 by default), WebVTT
  captions, its length and a "watched to complete" percentage. The file plays from a link signed for the person that expires in four
  hours, with range requests; it is never a public address. Reporting the seconds watched grows only, and completes the lesson at the
  share asked. The learner's page does not draw a player for uploaded files yet, and there is no transcoding or adaptive streaming.

### 📦 SCORM, xAPI and LTI

- **SCORM 1.2 and 2004.** Packages are played by the vendored scorm-again runtime inside Learn, resume where the learner stopped,
  and complete the lesson from what the package records (status, score, time). Package files are read from the zip on the server.
- **xAPI.** Learn runs its own xAPI endpoint for statements and state. A lesson completes when its package is completed or passed.
- **LTI 1.3 tools as lessons.** Tools are registered with their keys, launched by OIDC with an id\_token signed by Learn's key, and
  send grades back through Assignment and Grade Services, which complete the lesson.
- **Switched off by default.** *Allow SCORM and xAPI Packages* is a setting, because a package's scripts run in Learn with the
  learner's session.
- **Not yet.** SCORM 2004 sequencing between SCOs, cmi5, LTI Deep Linking and Names and Role Provisioning, and forwarding statements to an outside learning record store.

### ❓ Quizzes and question banks

- **Timed quizzes.** A passing percentage, a time limit on the server's clock, a maximum number of attempts, a number of questions per
  attempt, and shuffled questions and options.
- **Answer keys stay on the server.** Nothing a learner can read before submitting says which option is right. After submitting, a
  quiz's *Show Correct Answers* rule decides (after each attempt, after the last attempt, or never).
- **Attempts.** An attempt is *In Progress*, *Awaiting Grading*, *Submitted* or *Timed Out*. Answers are saved a moment after
  every change, so a closed tab loses nothing. The database keeps one open attempt per learner and bounded scores.
- **Ten question types in the model.** *Single Choice, Multiple Choice, True/False, Short Answer, Numeric* (with a tolerance),
  *Ordering, Matching, Fill in the Blanks* (`[[1]]` in the text, accepted answers for each blank), *Essay* and *File Upload*. Short
  answers and blanks accept patterns (`*`, `?`). The learner's quiz player draws single choice, multiple choice, true/false and short
  answer today; the other types are accepted by the server and the API, and their player screens are not built yet.
- **Partial credit and negative marking.** A quiz can give partial credit for choice questions and subtract a percentage for wrong
  answers; an attempt never scores below zero.
- **Question banks.** Questions have a category and tags. A quiz can draw N questions of a category (and tag) at random for each
  attempt, kept with the attempt.
- **Grading by people.** An attempt with essay or file answers waits as *Awaiting Grading* until each is graded with *Grade
  Answers* (points or a rubric, and a comment). With a *Second Grader*, two grades that agree are final and two that differ are
  settled by a third person. The learner is told once, when the last answer has its final grade.
- **Rubrics.** A rubric has criteria with levels and points. It can be used on an assignment or on an essay or file question; the
  score is the rubric's share of the points and the learner sees the filled rubric with the released result.
- **Reviewing.** Instructors review any attempt with the learner's answers, the correct answers and explanations.

### 📝 Assignments

- **Text, file, or both.** An assignment has instructions, a submission type, a maximum score and a passing score.
- **Deadlines.** A fixed due date or a number of days after enrolment; late work is accepted or refused as the assignment says.
- **Resubmission.** Resubmission can be allowed with a limit; submissions are numbered.
- **Grading screen.** The instructor's grading queue opens one piece at a time with the learner's answer or file, a score, feedback and
  an outcome (*Graded* or *Needs Revision*); saving moves to the next. A passing grade completes the assignment's lessons.
- **Statuses.** *Submitted, Graded, Needs Revision*, with a late flag.

### 🧑‍🤝‍🧑 Cohorts and live classes

- **Cohorts.** A cohort belongs to a course and has start and end dates, a time zone, a capacity and its instructors. Capacity is
  enforced when learners join or come back.
- **Sessions.** A cohort lists live sessions with a title, start time, length, host, a join link and the lesson it credits. *Schedule
  Sessions* makes up to 100 of them at once (weekly by default) at a wall-clock time in the cohort's time zone, across daylight-saving changes.
- **One reminder per session** is sent to members and instructors, at a time set in Learn Settings (60 minutes by default).
- **Meetings in the product.** Where the Comms app and a meeting server are installed, a session without a link of its own gets a
  meeting, hosted by the session's host or the cohort's first instructor. Moving or shortening the session moves the meeting; removing it
  ends it. Each member gets their own join link.
- **Attendance.** Ten minutes after a session was due to end, Learn reads the meeting and records who attended and for how many
  minutes. The Live lesson is marked done for those who were there for at least *Attendance Needed (%)* (50 by default). Instructors can
  mark attendance by hand, which is all a workspace without a meeting server has. Nothing is taken back once credited.
- **Screens.** The Live panel of a lesson shows the session time on the person's own clock, the way in from ten minutes before, and
  whether the learner was credited. A cohort's page lists each session's learners with joined or not and minutes, and lets an
  instructor mark them present or absent.

### 🏅 Gradebook

- **Weighted categories** with the lowest items dropped, a **grading scale** of bands, and a *Minimum Total to Complete (%)* for a course.
- **Grade items** from a quiz (best final attempt), an assignment (latest graded submission, less a late penalty per day up to a cap), a
  live session (attended) or marks entered by hand. Items can be released or hidden.
- **Grade entries** give marks, override them, or excuse a learner.
- **Exports.** An instructor reads a course's gradebook and downloads it as CSV; a learner reads their own released marks.
- **Where it lives today.** The server and the API, with the records screens. The interface has no gradebook screen yet.

### 🎓 Certificates

- **Issued once.** Completing a course (every required lesson done, required quizzes passed, and the minimum total met where one is set)
  issues exactly one certificate, numbered, with an issuer, a signatory and a verification code.
- **Printed and shared.** The certificate has a print format, and a public page at `/lms/c/{workspace}/certificate/{code}` checks that it is real.
- **Four states.** *Valid, Expired, Revoked, Superseded.* A manager revokes with a reason. The public check says *Expired*, with its last day, or
  that a newer one replaced it.
- **Expiry and recertification.** A course can say how many days a certificate is valid and how many days before expiry the learner
  is enrolled again. The new enrolment is the next *cycle* of the old one, due on the expiry date, once for each expiry. Completing it
  issues a new certificate and the old one becomes *Superseded*. The earlier enrolment stays as it was.

### 🏢 Compliance training

- **Due dates.** An enrolment has a due date, set by an instructor or manager, or worked out from the course's *Due (Days After Enrolment)*.
  A learner cannot choose their own.
- **States.** The Training Status report gives each enrolment's state on a day: *Not Started, In Progress, Completed* or *Overdue*, with totals.
- **Reminders.** Upkeep reminds a learner once before the due date and once after it (3 and 1 days by default, 0 for never), never after
  completion. A new due date earns the reminders again.
- **Enrolment rules.** A rule names a course or a programme and who matches: the workspace's active users by role or group, or, with the
  HR app, its active employees by Employee fields (department, designation, branch, employment type and the like). A switched-on rule enrols at
  the next upkeep those who match and have no enrolment, never twice and never someone who enrolled themselves. *Withdraw If Not Started* can
  take unstarted enrolments back when people stop matching. *Preview* says how many people a rule would enrol before it is switched on.
- **Training programmes.** A programme lists courses, each with its own due days and whether it repeats as its certificate nears expiry.
  The Programme Status report shows each person and course with state and dates, the completion rate and the overdue count.
- **Teams.** *My Team's Training* lists the latest enrolment of each person who reports to the viewer in HR (one level). LMS managers see everyone or,
  with HR, one manager's team; without HR only they run it.
- **HR link.** With HR installed, a completed course goes on the employee's HR training record, valid until the certificate's last day.
- **Not yet.** Reminders to managers and escalation.

### 🛒 Selling courses (Paid Courses)

- **A connector with Commerce.** Paid Courses adds a *Price* and a *Sold As* item to a course, and a selling company, a store
  payment method and a refund period (14 days by default) to Learn Settings.
- **Buy.** A learner's Buy makes a course purchase, a submitted sales invoice and a payment request. The gateway's payment enrols them.
  A purchase is *Awaiting Payment, Paid, Cancelled* or *Refund Requested*.
- **Promotion codes.** Buy takes codes; ERP's promotions price the invoice, which is on the *Learn* channel with the codes on it. A code
  that cannot be used refuses the purchase.
- **Bundles.** A course bundle sells courses at one price, a promotion in ERP. Buy invoices every course on one invoice and the one
  payment enrols the learner in each. A learner who holds one of the courses buys the others one by one.
- **Refunds.** A learner who withdraws within the refund period, before completing, gets the whole price back: Learn credits the invoice
  and asks for the refund, which someone approves in the store. A credit note made in the store withdraws the learner's enrolment, with
  progress kept. A bundle is refunded whole.
- **Not built.** Subscriptions, instalments, prices in several currencies, buying for others and team seats.

### 🌐 People outside the workspace

- **Online Learner.** A portal role for someone who signed themselves up on a course page: their own enrolments, progress, attempts,
  submissions, certificates and purchases, and the published public catalog. Lessons, quizzes and assignments reach them only through Learn's
  pages, which check their enrolment.
- **Sign-up.** An administrator turns on public sign-up for the role. The public course page then offers it; when the mailed code is confirmed
  the person is enrolled in that course, or, for a paid one, their order is made and the payment enrols them. The course must be published,
  public and open to self-enrolment. A returning learner signs in with an emailed code.
- **My learning** appears on the workspace's portal.
- **The public catalog.** Published public courses at `/lms/c/{workspace}`, with free-preview lessons and the certificate check, with no sign-in.

### 💬 Discussions

- **Threads** for a course or a lesson, with replies, written by enrolled learners and instructors.
- **Follow and report.** Followers are told of replies. A reader can report a post and instructors are told.
- **Moderation.** Instructors mark an answer, pin, close or remove a post; a removed post is kept with who and why, and learners no longer read its text.
- **Where it lives today.** The server and the API, with the records screens. The player has no discussion screen yet.
- **Erasure.** Following a thread, reporting a post and removing one are declared personal data: erasing a person empties their follower rows, their addresses on posts, and
  the posts and thread titles they wrote.

### 🖥️ Learner, instructor and manager screens

- **My learning.** Where to carry on with each course, what is due, the live classes coming up, and the certificates earned. Due dates, *Overdue*
  and *Recertification* are marked.
- **The course player.** The lesson with the course's outline beside it (a sheet on a phone), a tick on each finished lesson, and one obvious next
  step at the foot of every lesson.
- **The quiz player.** Rules, one question at a time, autosave, a countdown from the server's clock, a result and a review.
- **The catalog and course pages,** filtered by category and level, with the filters kept in the address.
- **Teaching.** The courses a person teaches (every course for a manager) with how many are learning, how many finished, average progress and
  how much work waits. Each row leads to the learners and the builder.
- **Course dashboard.** How many are learning and how far, who has gone quiet, each learner's lessons and attempts, people to add by address,
  and the work waiting for a grade.
- **The course builder.** The outline on one side and the lesson editor on the other, saved a moment after typing stops or at once with
  Ctrl+S, with the server's reasons shown under the fields they concern. Publishing asks the server's rules in advance.
- **Training reports** (Training Status, Programme Status, My Team's Training) are linked from Teaching and My learning.
- **Assistant.** It knows the states and suggests "Which of my courses have I not finished?", "How many people completed each course this month?" and
  "Which assignments wait to be graded?".

### 📊 Reports and notifications

| Report | What it shows |
|---|---|
| Lms Course Completion | Enrolled and completed learners for each course |
| Lms Quiz Scores | Scores of quiz attempts |
| Lms Learner Progress | Each learner's progress |
| Lms Overdue Assignments | Assignments past their due date |
| Lms Training Status | Each enrolment's state on a day, with totals |
| Lms Programme Status | People and courses of a programme, completion rate and overdue count |
| Lms Team Training | A manager's team's latest enrolments |

Learners and instructors are notified in the bar and by email (when Mail is installed) of enrolment, grading, certificates, sessions and
reminders. Three message templates can be edited per workspace: *A reply in a discussion you follow*, *A discussion post was reported* and *Your quiz attempt was graded*.

### 🔐 Roles and permissions

| Who | Can |
|---|---|
| 🗝️ LMS Manager | Runs the learning programme: every course, question bank, cohort, enrolment, grade and certificate; enrolment rules, programmes, LTI tools, categories, grading scales, and Learn Settings; imports and exports |
| 🧑‍🏫 Instructor | Builds the courses they teach (chapters, lessons, banks, quizzes, assignments, cohorts), enrols learners, grades, follows progress, moderates discussions. LmsCourse scopes limit an instructor to some courses |
| 🙋 Learner | Enrols in published courses, works through lessons, attempts quizzes, submits assignments, earns certificates, and sees only their own records |
| 🌐 Online Learner (portal) | An outside person who signed up: their own records and the public catalog, nothing of administration |

### 🔏 Personal data

- **Erasure by rule.** Enrolments, lesson and package progress, quiz attempts, assignment submissions, certificates, grade entries,
  discussion posts and follower rows, and course purchases are declared with the fields a request erases.

### 🌍 Interface

- **Languages.** English, Arabic and Hindi words, and Arabic and Hindi names for every record, field and report. A workspace can add its
  own words in Admin.
- **Right to left.** Pages mirror as they read.
- **Accessibility basics.** The outline is a list of links that works with a keyboard and a screen reader, and controls have labels.
- **Phone.** The outline becomes a sheet and pages fit a narrow screen. There is no native app and no offline use yet.

## 🧭 How it works

```mermaid
flowchart LR
  E["Enrolment: self, bulk, rule or purchase"] --> L["Lessons"]
  L --> Q["Quizzes"]
  L --> AS["Assignments"]
  Q --> P{"Required lessons done and quizzes passed?"}
  AS --> P
  P -->|yes| C["Certificate, once, verifiable"]
  C -->|expires| R["Recertification: next cycle"]
  R --> L
```

An enrolment is made by the learner, by an instructor, by a rule or by a purchase. Progress is computed by the server. When every
required lesson is done, the certificate is issued exactly once. If it expires, the learner is enrolled again.

```mermaid
flowchart TD
  B["Learner presses Buy"] --> PU["Course purchase"]
  PU --> INV["Sales invoice in ERP"]
  PU --> PR["Payment request in Commerce"]
  PR -->|gateway pays| EN["Learner enrolled"]
  EN -->|withdraws in time| CN["Credit note and refund request"]
```

Paid Courses uses Commerce for payment and ERP for the invoice, so a sale is in the books from the first click.

```mermaid
flowchart LR
  UI["Learn interface at /lms/"] --> API["Ferrum REST and MCP"]
  API --> K["Kernel: permissions, row-level security, audit"]
  K --> APP["Learn: rules in Rust, records as YAML"]
  K --> DB[("PostgreSQL")]
  K --> OB["Outbox: notifications, webhooks, automations"]
```

Everything Learn adds is ordinary Ferrum metadata and Rust, so its records get lists, forms, search, permissions, an audit trail, exports
and the APIs, and AI agents work with them under the same rules as people.

## 🧱 Data model

```mermaid
erDiagram
  LmsCourse ||--o{ LmsChapter : has
  LmsChapter ||--o{ LmsLesson : has
  LmsCourse ||--o{ LmsEnrollment : "taken by"
  LmsLesson ||--o| LmsQuiz : "can be"
  LmsLesson ||--o| LmsAssignment : "can be"
  LmsQuiz ||--o{ LmsQuizAttempt : "attempted as"
  LmsAssignment ||--o{ LmsAssignmentSubmission : "answered by"
  LmsEnrollment ||--o{ LmsLessonProgress : keeps
  LmsEnrollment ||--o| LmsCertificate : earns
  LmsCourse ||--o{ LmsCohort : "taught in"
```

| Entity | What it is | Key fields or states |
|---|---|---|
| Course, Chapter, Lesson | The content | Status, visibility, level; position; lesson type, quiz, assignment, package, video |
| Course Category | A catalog category | Name |
| Learning Path | Courses taken together | *Draft, Published, Archived* |
| Cohort | A group with live sessions | Dates, time zone, capacity, instructors, sessions |
| Quiz, Question, Question Category | Assessments and the bank | Passing %, attempts, limit, shuffle, partial credit; question type, points, answer key |
| Rubric | Criteria with levels and points | Criteria, levels, maximum points |
| Assignment | Work to hand in | Submission type, scores, due date or days, resubmission |
| Enrolment | A learner in a course | *Active, Completed, Withdrawn*; due date, cycle, progress, rule, programme |
| Lesson Progress, SCORM Progress | What a learner has done | Completed via; status, score, time, data |
| Quiz Attempt | One sitting | *In Progress, Awaiting Grading, Submitted, Timed Out*; score, percentage, passed |
| Assignment Submission | One piece of work | *Submitted, Graded, Needs Revision*; score, late |
| Certificate | Proof of completion | *Valid, Expired, Revoked, Superseded*; code, issued and expiry dates |
| Enrolment Rule, Programme | Compliance assignment | Audience, criteria, due days; courses with due days and repeats |
| Grading Scale, Grade Item, Grade Entry | The gradebook | Bands; source, category, late penalty; points, excused |
| Discussion Post | A thread or a reply | Course, lesson, pinned, closed, reported, removed |
| LTI Tool | An outside tool | Launch and login URLs, keys, whether it sends grades |
| Course Purchase, Course Bundle | Selling (Paid Courses) | *Awaiting Payment, Paid, Cancelled, Refund Requested*; courses and price |
| Learn Settings | Defaults | Public URL, email, reminders, certificate issuer and signatory, video hosts, video size, packages |

**Roles:** LMS Manager, Instructor, Learner; Online Learner on the portal.

**Reports:** seven, listed above. **Print format:** the certificate.

**Scheduled upkeep:** every minute, one replica at a time. It closes timed-out attempts, sends session and due-date reminders,
credits live attendance, enrols by rule and enrols again before certificates expire.

## 🔌 Integrations

| With | How | Needs |
|---|---|---|
| **Commerce and ERP** | Paid Courses: purchases, invoices, payment requests, promotions, credit notes and refunds | The Paid Courses connector, Commerce `>=0.7`, a selling company and a payment method |
| **Comms** | Meetings for live sessions and attendance read from them | Comms with a meeting server (otherwise a typed link and hand-marked attendance) |
| **HR** | Enrolment rules by employee fields; manager team reports; completed courses on the employee's training record | The HR app |
| **Mail** | Email notices | Mail installed and *Email Notifications* on |
| **SCORM, xAPI, LTI 1.3** | Packages played in Learn; Learn's xAPI endpoint; outside tools launched with grades back | *Allow SCORM and xAPI Packages* for packages; a tool's keys for LTI |
| **Video hosts** | YouTube, Vimeo, Loom, Wistia, Google Drive and hosts an administrator adds | Nothing |
| **REST and MCP** | Every record and action | A person's permissions |
| **Studio and Forms** | Custom reports and fields; a form linked from a lesson | Nothing |

## 📊 How it compares

Learn is compared with **Moodle** and **Canvas** (both open source), and with **TalentLMS**, **Docebo** and **Cornerstone**
(corporate training suites). The academic systems are built for classes and the corporate ones for compliance and sales; Learn sits
between them with the strengths of a sales ledger and a platform behind it.

| Capability | Ferrum Learn | Moodle | Canvas | TalentLMS | Docebo | Cornerstone |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| Courses with chapters, ordered lessons, previews and prerequisites | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Articles, documents, quizzes and assignments as lessons | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Uploaded video with captions and watched-to-complete | 🟡 | ✅ | ✅ | ✅ | ✅ | 🟡 |
| SCORM 1.2 and 2004 packages | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ |
| xAPI packages | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| LTI 1.3 tools as lessons, with grades back | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 |
| Learning paths and programmes | 🟡 | ✅ | ✅ | ✅ | ✅ | ✅ |
| Timed quizzes with attempt limits and shuffling | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Question types beyond choice (numeric, matching, essay and others) | 🟡 | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Question banks with categories, tags and random draws | ✅ | ✅ | ✅ | 🟡 | 🟡 | 🟡 |
| Partial credit and negative marking | ✅ | ✅ | 🟡 | 🟡 | 🟡 | 🟡 |
| Manual grading with a second grader and rubrics | 🟡 | ✅ | ✅ | 🟡 | 🟡 | 🟡 |
| Assignments with files, deadlines and resubmission | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Peer review and group assignments | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ |
| Gradebook with weighted categories and CSV export | 🟡 | ✅ | ✅ | 🟡 | 🟡 | 🟡 |
| Live sessions with capacity, reminders and attendance | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| Classroom training with venues and waitlists | 🟡 | 🟡 | ❌ | ✅ | ✅ | ✅ |
| Certificates with a public verification code | ✅ | 🟡 | ❌ | ✅ | ✅ | ✅ |
| Certificate expiry and automatic recertification | ✅ | 🟡 | ❌ | ✅ | ✅ | ✅ |
| Due dates, reminders and overdue states | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| Enrolment rules by role, group or HR fields | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| Training programmes with status reports | ✅ | 🟡 | ❌ | 🟡 | ✅ | ✅ |
| Manager's team training report | ✅ | 🟡 | ❌ | 🟡 | ✅ | ✅ |
| HR link: training record on the employee | ✅ | ❌ | ❌ | 🟡 | 🟡 | ✅ |
| Open badges and skills credentials | ❌ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Outside people sign up on a course page | ✅ | ✅ | 🟡 | ✅ | ✅ | 🟡 |
| Sell courses with payment and refunds | ✅ | 🟡 | ❌ | ✅ | ✅ | 🟡 |
| Sales posted in the same ledger as the business accounts | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Promotion codes and course bundles | ✅ | 🟡 | ❌ | ✅ | ✅ | 🟡 |
| Subscriptions, instalments, team seats | ❌ | ❌ | ❌ | ✅ | ✅ | 🟡 |
| Public course catalog with free previews | ✅ | ✅ | 🟡 | ✅ | ✅ | 🟡 |
| Branded academies for each customer or partner | ❌ | 🟡 | ✅ | ✅ | ✅ | ✅ |
| Discussions with follow, report and moderation | 🟡 | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Announcements to everyone in a course | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Reports on completion, scores, progress and overdue work | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ |
| Time on task and engagement analytics, scheduled reports | ❌ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| Native mobile app with offline lessons | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| AI help to draft courses and tutor learners | ❌ | 🟡 | 🟡 | ✅ | ✅ | ✅ |
| Self-hosted on your own servers | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| Open source | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| No licence fee per learner | ✅ | ✅ | 🟡 | ❌ | ❌ | ❌ |

Notes. **Ferrum 🟡:** uploaded video, the extra question types, manual grading screens, the gradebook and discussions are built on the server and
reachable through the records screens and the API, and the learner's and instructor's interface does not yet have screens for them; learning
paths enrol in every course but have no order, locking or path certificate; classroom training has capacity but no venue or waitlist.
Live meetings, HR rules and team reports need the Comms and HR apps. **Leaders:** 🟡 means a plugin, an add-on or a different edition, a lighter
feature, or a feature the vendor reaches through an integration. Moodle's and Canvas's open-source cores differ from their hosted editions.

### 🏆 Where Learn stands out

- 🧾 **Selling through the ledger.** Invoices, payment requests, promotions, credit notes and refunds are ERP and Commerce records.
- 🔒 **Assessment integrity in the database.** One open attempt, bounded scores, answer keys never readable, and the clock on the server.
- 📦 **Standards and compliance in one product.** SCORM, xAPI and LTI 1.3 beside due dates, recertification and rules by HR fields.
- 🤖 **Open to AI agents** through MCP, under each person's permissions.
- 🔏 **Privacy built in.** Declared erasure, row-level security, and discussions covered.
- 🔓 **Open and owned.** AGPL, self-hosted, no per-learner fees.

### 🚧 Where the leaders are still ahead

- 📱 **Mobile and offline.** Native apps and offline lessons.
- 🧩 **Authoring.** Drip release, forced order, course copy and templates, H5P, cmi5 and sandboxed package origins.
- 🧪 **Assessment.** Peer review, proctoring, plagiarism checks, and quiz windows and per-learner extensions.
- 🏷️ **Credentials.** Open badges, skills and several certificate templates.
- 🏫 **Scale.** Branches and sub-accounts, branded academies, subscriptions and team seats.
- 📈 **Analytics.** Time on task, drop-off and scheduled reports.

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change. Product names belong to
> their owners.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.9.1** Erasure reaches discussions; a learner is told when manual grading of an attempt is finished.
- [x] **0.9.0** Outside people sign up and buy (Online Learner); promotion codes, bundles and store refunds; six more question types, partial credit,
  negative marking, bank draws, grading by people with a second grader, rubrics; a gradebook; discussions; uploaded video.
- [x] **0.8.1** Screens for live sessions and compliance training.
- [x] **0.8.0** Compliance training: due dates, reminders, expiring certificates and recertification, enrolment rules, programmes and team reports.
- [x] **0.7.0** Live classes in the product: meetings per session, join links and attendance.
- [x] **0.5.0** LTI 1.3 tools as lessons, with grades back.

### 🔜 Next up

**Screens for what the server already does**
- [ ] Quiz player screens for numeric, ordering, matching, fill-in-the-blank, essay and file-upload questions.
- [ ] A screen to grade quiz answers by hand, with rubrics and a second grader.
- [ ] A gradebook screen for instructors and a grades page for learners.
- [ ] Discussion screens in the course player.
- [ ] A player for uploaded video with captions and progress.

**Authoring**
- [ ] Drip release by days after enrolment or a date, and lessons in forced order.
- [ ] Copy a course, and course templates.
- [ ] Learning paths with order, locking, path progress and a path certificate.
- [ ] Announcements to everyone in a course or cohort.

**Reach**
- [ ] Installable use with offline lessons and queued progress.
- [ ] Reminders to managers and escalation for overdue training.
- [ ] Open badges and several certificate templates.

### 🔭 Later

**Content**
- [ ] H5P content and cmi5 packages.
- [ ] Package content on its own origin, and SCORM 2004 sequencing.
- [ ] LTI Deep Linking and Names and Role Provisioning.
- [ ] Several downloads and audio lessons; course content in several languages.
- [ ] Accessibility audit of the player, with transcripts.

**Assessment**
- [ ] Quiz windows, extra time or attempts for one learner, and a choice of grading method.
- [ ] Peer review and group assignments.
- [ ] Question import from QTI and GIFT.
- [ ] Proctoring and plagiarism checks through LTI tools.

**Teaching in person**
- [ ] Classroom training with venues, seats and waitlists.
- [ ] Recordings kept with the course; sessions in the learner's calendar.

**Selling**
- [ ] Subscriptions and instalments; prices in several currencies; team seats and purchase orders.
- [ ] Branded academies with their own logo, colours and catalog; sales pages; ratings and reviews.
- [ ] Content libraries from vendors, through LTI or SCORM.

**People and data**
- [ ] Groups and branches with scoped administrators.
- [ ] Time on task, drop-off by lesson, and quiz item analysis.
- [ ] Forwarding xAPI statements to an outside record store; OneRoster for school rosters.
- [ ] Points, levels and leaderboards.
- [ ] AI help to draft outlines and quizzes and to tutor learners.

## 🚀 Install

Workspace administrators install Learn from **Apps** (`/store/`) in Ferrum. It ships in the official Ferrum server image, and a
self-hoster building their own image enables the `lms` feature of the server. It needs platform `>=0.3.2, <0.4` and nothing else.

After installing:

1. Give people the roles **LMS Manager**, **Instructor** and **Learner**. Limit an instructor to some courses with LmsCourse scopes.
2. Open **Learn Settings**: set the **public URL** (used in certificate links and emails), the certificate **issuer** and **signatory**, reminder
   days, and any extra video hosts. Turn on **Allow SCORM and xAPI Packages** only if you will use them.
3. Create a **course**, add chapters and lessons in the builder, and publish it.
4. For compliance training, add **enrolment rules** and **programmes** and run the *Preview* before switching a rule on.
5. For live classes with meetings, install Comms and a meeting server; otherwise type a join link and mark attendance by hand.
6. To sell courses, install **Commerce** and the **Paid Courses** connector, then set the selling company, a store payment method and the
   refund period in Learn Settings, and a price and an item on each course.
7. For outside learners, an administrator turns on public sign-up for the **Online Learner** role, passing `course`.

## 🔧 Develop

Clone it beside the platform:

```text
work/
├── ferrum/            https://github.com/CBFerrum/Ferrum
└── ferrum-app-lms/
```

To build against your checkout, point the platform dependencies at it (this file is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-calc = { path = "../ferrum/crates/calc" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-apps = { path = "../ferrum/crates/apps" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security applies:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_lms_test
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
FERRUM_LMS_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_lms_test cargo test
```

Learn also depends on the Commerce and Comms app crates (for Paid Courses and live meetings), and its tests use ERP and HR as development
dependencies.

The interface lives in `ui/` (React, built with Ferrum-Frontend's packages from a checkout beside this one, `../ferrum/web`). Its built files in `ui/dist` are
committed and embedded in the crate, so a change to `ui/src` is not done until `dist` is rebuilt:

```bash
cd ui
npm test            # logic, and every screen drawn to markup from data in the query cache
npm run typecheck
npm run build       # writes dist/
npm run check       # fails if dist/ is not built from these sources, or is over the size budget
```

CI (`.github/workflows/ci.yml`) runs the Rust job on PostgreSQL 17 and the interface job.

## 📂 Project layout

```text
app.yaml                    manifest: id, roles, shell entries, public catalog, portal page, assistant notes, message templates
connectors/payments/        the Paid Courses connector's manifest
packages/lms/               entities, roles, reports, the certificate print format and SQL migrations
packages/lms_payments/      Paid Courses: purchases and bundles
src/                        hooks, actions, endpoints and upkeep: enrollment, quiz, grading, gradebook, certificate, cohort, live,
                            rules, scorm, xapi, lti, payments, discussion, video, site
tests/lms_it.rs             integration tests against PostgreSQL
ui/                         the interface: src/ (React) and dist/ (built from it); vendor/scorm-again
CHANGELOG.md                what changed in each version
```

## 📜 Changelog

Every release is described in [CHANGELOG.md](CHANGELOG.md).

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-lms](https://github.com/CBFerrum/ferrum-app-lms/issues). Keep the
checks above green, and rebuild `ui/dist` with any change to `ui/src`.

`ui/vendor/scorm-again/` holds the SCORM 1.2 and 2004 runtimes of [scorm-again](https://github.com/jcputney/scorm-again) 3.4.6, unchanged,
under its MIT licence; its `PROVENANCE.md` says where they came from and their checksums.

## 📄 License

[AGPL-3.0-only](LICENSE), like the platform.

<div align="center">

<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>

</div>
