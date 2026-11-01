# Changelog

## 0.2.0

- Learn has an interface of its own at /lms/ (Ferrum docs/plan/apps-ui.md), in `ui/`, built into
  `ui/dist` and embedded as `UI`.
  - **Learners.** My learning: where to carry on, what is due, live sessions and certificates. The
    course player: the outline beside the lesson (a sheet on a phone) with ticks and progress,
    readings, videos in their host's own player, documents, and one button that finishes a lesson
    and moves on; a course picks up at the first lesson still to do; free previews open without
    enrolling. The quiz player: the time comes from the server's clock, answers are saved as they
    change so a closed tab loses nothing, submitting warns about unanswered questions, and the result
    shows what was right, the correct answers when the quiz shows them, and the attempts left for a
    retake. Assignments: hand in text or a file, change it until it is graded, read the grade and
    feedback. Certificates: see and print them, and share a link anyone can check.
  - **Catalog and course pages**, with filters in the address, learning paths, the exact reasons
    when enrolling is refused, and cohorts with their places. The same for everyone, without signing
    in, below /lms/c/: the public catalog, course pages with their free previews, and a certificate
    check.
  - **Instructors.** Teaching: the courses with learners and work to grade. A course's learners:
    find, sort by who has been quiet, see a learner's lessons and quiz attempts, add people by their
    addresses. A grading queue: score, feedback, or ask for changes, then on to the next. The course
    builder: drag chapters and lessons, or move them with buttons and a menu; rename in place; the
    lesson editor saves as you type; a checklist says what stops a course from publishing.
- The manifest adds the launcher entry (LMS Manager, Instructor, Learner), New Course in the bar's
  Create menu, courses opening on their page, and the public pages. It lets the five video players
  Learn embeds be framed; a host an administrator adds in Learn Settings opens in a new tab instead.
- The actions the interface runs for learners (open and complete a lesson, start a quiz, save and
  submit answers, the outline) no longer show as buttons in the enrolment and attempt forms, where
  they could not work.
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
