# Changelog

## 0.6.10

Rebuilt with Ferrum-Frontend 5b22e7b: its own words carry built-in Arabic and Hindi (459 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.6.9

Rebuilt with Ferrum-Frontend 6537d65: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (412 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.6.8

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else. Its own screens' margins, paddings, borders, alignment and insets go by the line's start and end, so they mirror right to left as the kit's do; what it places by coordinates (boards, timelines, charts, handles) stays as it was.

## 0.6.7

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.6.6

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved. An action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once.

## 0.6.5

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.6.4

Rebuilt with Ferrum-Frontend 24c3951: its pages show times on the workspace's clock, naming the zone when it is not the browser's, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.6.3

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.6.2

The record pages ask for what their actions read: a certificate's Revoke needs a reason, a course's Enrol Learners their emails, a cohort's Schedule Sessions the first date and time, and a submission's Grade a score, and none asked, so from those pages they could not work. They now ask; Learn's own screens send them as before.

## 0.6.1

A lesson's New quiz and New assignment make one where the lesson is, named after it, and pick it, so a quiz or assignment lesson saves at once; Edit quiz and Edit assignment then open it for its questions or instructions. They used to leave for a blank form, and the lesson lost its kind, which saves only with its quiz or assignment. Rebuilt with Ferrum-Frontend 0f1a544: a settings page is headed by what it is.

## 0.6.0

Lesson files stream: a lesson's file is sent by the server straight from storage, whatever its size, instead of being read whole into memory first (Ferrum's SiteResponse::file, FERRUM-OPS-006).

## 0.5.3

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.5.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.5.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.5.0

LTI 1.3 tools as lessons, Learn as the platform (the rest of FERRUM-LMS-002).

- An LMS manager registers a tool (LTI Tool): its login and launch addresses, the parameters it
  gets, whether it learns names and emails, and whether it sends grades. Learn gives it a client id
  and a deployment id, and shows the addresses to enter in the tool: the issuer (Learn Settings'
  public URL), the authorization endpoint, Learn's keys and its token endpoint.
- An LTI lesson opens its tool in a new window. Learn starts the OIDC login at the tool, and
  answers the tool's authorization request for the signed-in learner with an id_token signed with
  Learn's own RSA key (made on first use, kept as a secret). It is posted only to an address the
  tool registered, and names the learner by an opaque id, with the course, lesson, role and custom
  parameters.
- Tools that send grades get an access token for an hour for a client assertion signed with their
  key (each assertion used once) and post scores to the lesson's line item (Assignment and Grade
  Services). A completed activity completes the lesson, and the score is kept in Package Progress.
  A tool that sends no grades leaves its lessons for the learner to mark done.

Not yet: Deep Linking (choosing content in the tool), Names and Role Provisioning, line items
other than the lesson's own, and authorization requests sent by form post.

## 0.4.0

SCORM and xAPI lessons (part of FERRUM-LMS-002).

- An LMS manager allows packages in Learn Settings (off by default: a package's scripts run with the
  learner's session). Instructors then make lessons of SCORM 1.2, SCORM 2004 or xAPI (Tin Can)
  packages: they upload the zip, and Learn reads what it launches from its `imsmanifest.xml` or
  `tincan.xml`, refusing what cannot play or reaches outside the package.
- **SCORM** plays in Learn's player through scorm-again (MIT, vendored with its provenance). The
  package knows the learner and resumes where it stopped, and what it records is saved as it
  commits and when it finishes. Completed or passed, the lesson is completed; its status, score and
  time are kept in the learner's Package Progress.
- **xAPI** packages are launched with Learn's own xAPI endpoint, which speaks only for the signed-in
  learner about that lesson. It keeps statements (stored once at their id, voided, read back) and
  state documents (bookmarks and resume data, merged and guarded by their tags). A statement that
  the lesson's activity was completed or passed completes the lesson, and once done it stays done.
- An instructor's preview plays the package and keeps nothing. A new package is a new address, so
  the old one stops being served.

Not yet: SCORM 2004 sequencing between SCOs, cmi5, xAPI profiles and attachments, and a separate
origin for package content.

## 0.3.0, and Paid Courses 0.1.0

Paid courses (part of FERRUM-LMS-002), in a connector app with Commerce, Paid Courses, offered where
both are installed.

- A course can have a price and the item it is sold as. Its page shows the price, in the catalog,
  on the course page and on the public pages, and a Buy button where Enrol was. Nobody enrols
  themselves in a paid course they have not paid for, also not through a learning path; its
  instructors and LMS managers still enrol anyone, and try it themselves without paying.
- Buying invoices the learner (their customer is found by email, or made) for the selling company
  in Learn Settings, and asks for the invoice through the store's payment method named there. The
  course page waits for the payment and opens up when it arrives: the payment enrols the learner,
  or brings back the enrolment they left, progress kept. Buying again while a payment is due asks
  for the same invoice again instead of invoicing twice.
- Withdrawing within the refund period (14 days by default, from paying) credits the whole invoice
  and asks for the payment to go back, through the gateway once a person approves the refund. Later,
  the money is kept and enrolling again needs no new purchase.
- A purchase whose payment request lapsed, failed or was cancelled is cancelled with its invoice
  after a day, unless the learner asks to pay again first.
- Each purchase is a Course Purchase record: who bought what, for how much, its invoice, payment
  request, enrolment, credit note and refund. LMS managers and instructors read them; learners read
  their own.

Not yet: coupons and discounts, prices in more than one currency, refunds of part of the price,
and buying a course for someone else.

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
