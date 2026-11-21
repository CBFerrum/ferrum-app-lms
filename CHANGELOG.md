# Changelog

## 0.13.0

New entity (LmsAnnouncement) and action (Announce); package lms 0.7.0. Interface rebuilt.

- **lms-open-doors.** A signed-in person with no Learn role is served the public pages as anyone; a person outside the workspace authors nothing.
- **lms-1.** The quiz player answers and reviews all ten question types.
- **lms-2.** The gradebook on screen, for instructors and for learners.
- **lms-3.** Discussion pages for a course and its lessons, with moderation.
- **lms-4.** An uploaded video plays with captions and completes by watching.
- **lms-5.** Grading on screen: essay and file answers, rubrics, second and third grader; a file answer is tied to its attempt so instructors read it.
- **lms-6.** An instructor announces something to everyone in a course.

## 0.12.0

No database or interface change. Needs Ferrum 0.4.0 (apps' upkeep runs one workspace at a time, on whichever replica is free).

- **Upkeep is one workspace's.** `maintenance(k, tenant)` and `maintenance_at(k, tenant, at)` run the same steps, in the same order, for
  the one workspace they are given; the first failure is returned after the rest ran, as before. They no longer list workspaces or check for Learn.

## 0.11.1

Needs the same platform as 0.11.0. No database change. Interface only, `ui/dist` rebuilt with Ferrum-Frontend `06bd1a2`.

- The two routes to the screens apps no longer have (`apps/:app/settings`, `x/:app/:page`) are gone.
- The page starts through the frontend's `mountApp`; the interface lint has no baseline file any more. The pages guests open
  stay outside the sign-in, as before.

## 0.11.0

Paid Courses (package `lms_payments` 0.3.0, connector 0.3.0) needs Commerce 0.13 (`requires: {commerce: ">=0.13"}`) and
the platform that lets an app call another's operation (Ferrum main 44520a63). No database change, no interface change.

* **Payments and refunds go through Commerce's operations, not its code.** Paid Courses declares what it uses of
  Commerce (`uses` in its package, each at `^0.6`): `commerce.sales_invoice.request_payment` and
  `commerce.payment_request.refund`. Learn calls them through the platform's gate (`Kernel::call_tx`) with its own vault
  handle, in the learner's transaction as before. The install screen lists both uses; disabling Paid Courses withdraws
  them.
* **Keys.** Asking for a purchase's invoice is keyed `course:{invoice}:after:{request}`, the request it replaces (`none`
  for the first): buying again after a request lapsed or failed asks anew, while the same ask again (a double click in one
  transaction, a retried call) is answered with the request already made. A refund is keyed `course-refund:{purchase}`:
  a purchase is refunded once. A bundle's courses share their invoice, and so its key.
* **Receipted as Learn's.** Each call's receipt names Learn and the learner it acted for.
* **What changed that a person may see.** A refund a learner's withdrawal asks for names the learner as `requested_by`
  (it named the system), and the payment request is made on the learner's behalf. A store payment method that does not
  exist refuses the purchase on `gateway` (it was `payment_method`). An invoice owed nothing still refuses with "nothing
  is outstanding on this purchase's invoice".
* `ferrum-app-commerce` is a development dependency only: the suite registers Commerce to answer the calls.

Tests: `paid_courses_end_to_end` keeps its outcomes and adds: one receipt for Asha's purchase, by Learn, for her, keyed
by the invoice; the same ask again is answered `repeated` with the same request and no new receipt; her withdrawal's
refund is one receipt keyed by the purchase; Chen's second ask after his request lapsed has a key of its own; a payment
method the store lacks refuses a purchase with nothing kept; with Paid Courses disabled, a learner enrols in the course
and a call as Learn is `forbidden`, writing nothing. `live_sessions_end_to_end` gives Comms LiveKit's key and secret
through Comms' vault, as Comms 0.25 reads them.

## 0.10.0

* **A learner's customer is the one ERP finds or makes for a person** (`ferrum_app_erp::portal::customer_for`, ERP 0.28.3), in place of Paid Courses' own copy. A customer a web shop made at a guest checkout under the learner's address is no longer theirs, since anyone may type anyone's address there: they get a customer of their own, found again next time. Two purchases at once for one address (two tabs, a course and a bundle) wait for each other on ERP's lock for the address and make one customer, where they made two. ERP is a dependency of the app now, not of its tests only.
* **The package's SQL is its final schema, in one file.** Package `lms` 0.6.0 has `sql/001_lms.sql` only: the engine tables of `002` to `004`, `006` and `007` are in it, and `005_recertification.sql` is gone; it dropped the unique keys enrolments, attempts and submissions had before recertification and filled in `cycle` on older enrolments, so on a new database it did nothing. A database that applied the old files refuses the changed one: create it again. No interface change.

## 0.9.3

Learn reads its LTI signing key (Learn Settings' sealed `lti_private_key`) with its own handle on the vault, in the same transaction as before, instead of the platform's reader that any app could use, which is deprecated. It registers as itself (`hooks.app("lms", …)`) to be given that handle. Needs a platform with the vault's field reader (0.3.8 on the platform's main from cdec881). Launches and their keys work as before; while the vault is closed a launch refuses plainly. No database or interface change.

## 0.9.2

* **Addresses without a public URL of Learn's.** With Learn Settings' public URL empty, a certificate had no verification link and an LTI tool could not be registered. Each address now comes from the platform, by what calls it. A certificate's verification link, which people follow, and LTI's authorization endpoint, which the learner's browser is sent to by GET with the session it signed in with, go through the operator's link host when there is one (`FERRUM_LINK_HOST`, `https://{it}/go/{workspace}/p/…`), so they keep working when the workspace's address changes, and lead to the workspace's portal host when it has one (where Learn's learners sign in), else its main host. LTI's issuer, keys (`jwks`), token endpoint and grade service, which a tool's server keeps or calls (the token endpoint by POST, which the link host does not take), start where other services call back (`FERRUM_HOOKS_HOST`, else the server's public address), which never moves with a workspace's address. With the public URL set, nothing changes; with no address at all, there is none, as before. A tool registered before keeps the addresses it was given. No database change, no interface change.

## 0.9.1

* **Erasure reaches the discussions.** Following a thread, reporting a post and removing one are declared personal data like the rest: erasing a person empties their follower rows on every thread, blanks their address as the reporter or remover of a post, and, with the posts they wrote, the thread titles they gave. Nothing else of the app held a discussion address.
* **A learner is told when manual grading of a quiz attempt is finished.** The message template `lms.grading_done` (in-app and email, editable by the workspace) goes to the learner once, when the last answer a person grades has its final grade (with a second grader, after the third settles a disagreement), never while an answer still waits. Assignment submissions already told the learner when graded and still do.

## 0.9.0, and Paid Courses 0.2.0

Learn sells to people outside the workspace, assesses like an academic LMS, keeps a gradebook, holds discussions and plays its own videos (LM-60 to LM-66, LM-30, LM-31, LM-37, LM-45, LM-80, LM-01). Package `lms` 0.5.0, `lms_payments` 0.2.0. Migrations `006` (grading notes) and `007` (video link keys).

**Signing up on a course page (LM-60).** A new portal role, **Online Learner**: their own enrolments, progress, attempts, submissions, certificates and purchases, the published public catalog, nothing of other learners or of course administration (lessons, quizzes and assignments reach them only through Learn's pages, which check the enrolment). An administrator turns public sign-up on for it, passing `course` (`PUT /api/admin/roles/Online Learner/signup {"passes": ["course"]}`); the public course page then says `signup: {role, passes: {course}}`. When the mailed code is confirmed, the platform's `ferrum.portal.signed_up` enrols them in that course, or, for a paid one, makes its order (the invoice and payment request, as Buy does), and the payment enrols them. The course must be published, public and open to self-enrolment; otherwise the person is made and enrolled in nothing. A returning learner signs in with an e-mailed code. Outside people see the public catalog only and cannot enrol in or buy internal courses. The app declares `shell.portal` (My learning).

**Selling (LM-61 to LM-66).** Buy takes `codes`: the invoice is on the **Learn** channel with the codes on it, and ERP's promotions price it (a code that cannot be used refuses the purchase; uses are counted by ERP when the invoice is submitted). **Course Bundle**: courses sold at one price, the price being an ERP promotion giving a Bundle Price over the courses' items (a promotion, not an ERP product bundle); Buy invoices every course on one invoice, each course gets its purchase with its share of the price, and the one payment enrols the learner in each. A learner holding one of the courses buys the others one by one. A credit note against a course's invoice made in the store (ERP's Create Credit Note) withdraws the learner's enrolment, progress kept; Learn's own refund now uses ERP's Create Credit Note, so the credit is what was paid after promotions. A bundle is refunded whole by the store, not one course at a time. **Subscriptions are not built**: Commerce's Store Subscriptions sells a Finance plan through the web shop; Learn would need from it (or ERP) only (1) a subscription's customer e-mail, plan and state (active, past due, ended) readable by Learn, (2) an event when that state changes, and (3) a plan named on a Learn record that lists the courses or "the catalogue" it opens. With those, Learn would enrol on active and withdraw (progress kept) on lapse.

**Questions and grading (LM-30, LM-31, LM-37).** New types: Numeric (with a tolerance), Ordering, Matching, Fill in the Blanks (`[[1]]` in the text, accepted answers per blank), Essay and File Upload (graded by a person); short answers and blanks take patterns (`*`, `?`). A quiz has **Partial Credit**, **Negative Marking (%)** (an attempt never scores below 0) and **Second Grader**. Questions have a **Question Category** and tags; a quiz's **draws** take N questions of a category (and tag) at random per attempt, kept with the attempt. An attempt with answers a person grades is **Awaiting Grading** until each is graded with **Grade Answers** (points or a rubric, and a comment); with a second grader, two grades that agree are final and two that differ are settled by a third person, the grades not yet final kept apart from the attempt. Teaching's grading queue: `GET /api/app/lms/teaching/grading`. **Rubrics**: criteria with levels and points, on an assignment or an essay or file question; the grader picks a level per criterion and the score is the rubric's share of the points; the learner sees the filled rubric with the released result. No answer key reaches a learner in any new type (tested by searching every answer she gets).

**Gradebook (LM-45).** A course has weighted **Grade Categories** (with lowest items dropped), a **Grading Scale** of bands, and **Minimum Total to Complete (%)** (empty: as before). **Grade Items**: a quiz (best final attempt), an assignment (latest graded submission, less a late penalty per day up to a cap), a live session (attended), or marks entered; released or hidden. **Grade Entries** give marks, override, or excuse. `GET teaching/courses/{id}/gradebook` (`?format=csv` exports) for the course's instructors; `GET courses/{slug}/grades` for a learner's own released marks.

**Discussions (LM-80).** The platform's comments are not shared with portal people, so posts are Learn's own **Discussion Post** records: a thread per course or lesson, replies, written through the course's **discuss** action by enrolled learners and instructors; **follow**, **report_post**; instructors **mark_answer**, **pin**, **close**, **remove** (kept with who and why; learners no longer read its text). Followers are told by the template `lms.discussion_reply`, instructors of a report by `lms.post_reported`. `GET courses/{slug}/discussion?lesson=&thread=`.

**Uploaded videos (LM-01).** A Video lesson may hold a **Video File** (up to **Largest Video (MB)** in Learn Settings, 2048 by default), **Captions** (WebVTT), **Video Length (Seconds)** and **Watched to Complete (%)**. It plays from `lessons/{id}/video` by a link signed for the person, expiring in four hours, with range requests; never a public address. **video_progress** records the seconds watched (growing only, by no more than the time since the last report) and completes the lesson at the share asked. Not here: transcoding, adaptive streaming, thumbnails, length read from the file, and the player's own heartbeats; a media component would add them.

Not in this release: the interface's screens for any of it (see the README), subscriptions, discussion followers reached by personal-data erasure (their addresses sit in the thread's follower rows), and new notices for graded attempts.

## 0.8.1

The interface gains the screens for Live sessions and compliance training, rebuilt with Ferrum-Frontend 1381367. A Live lesson shows its session's time on the person's own clock, the way in from ten minutes before it starts (the learner's own meeting link, else the typed one), whether it is not open yet, live now or ended, and after it ended whether the learner was credited or why not. On a cohort's page an instructor sees each session's learners with joined or not and minutes and marks them present or absent (Mark Attendance); Live is a kind of lesson in the builder. My learning, the catalog and the course page show an enrolment's due date, Overdue, and Recertification where it renews an expiring certificate (read from the learner's own enrolment records). The public check says Expired, with its last day, or replaced by a newer one, instead of revoked. An enrolment rule's page has a Preview that says how many people it would enrol. Teaching links the Training status, Programme status and My team's training reports, and My learning links the last (they open in the records frame, with its filters and CSV download). Words and names in Arabic and Hindi (53 words and 58 names more; translated by the model, to be reviewed by native speakers).

Not in this release: the public check does not link to the newer certificate (its data does not say which it is), My learning's certificate list does not say Expired or Superseded, the cohort session form is the records frame's own (it already offers Host, Lesson Credited and Attendance Needed), and the new screens were not run against a server.

## 0.8.0

Compliance training can be run and proved (LM-50 to LM-53, LM-90, LM-110).

**Due dates.** An enrolment has a **Due** date: set by an instructor or LMS manager, or worked out from the course's **Due (Days After Enrolment)** when it is made (a learner cannot choose their own). The **Training Status** report gives each enrolment's state on a day: Not Started, In Progress, Completed or Overdue (not completed and past due), with totals. The upkeep reminds a learner once before the due date and once after it, at **Remind Before Due** and **Remind After Due** days on the course or, when empty, in Learn Settings (3 and 1 by default; 0 is never), and never after completion; a new due date earns the reminders again.

**Expiry and recertification.** A course states **Certificate Valid (Days)** (empty: for ever, as before) and **Re-enrol Before Expiry (Days)** (empty: Learn Settings, 30 by default; 0: never). A certificate now has an **Expires On** date and reads **Expired** once its last day has passed, in lists, on the printed copy and on the public verification page. Within the re-enrolment days of its expiry the learner is enrolled again: a new enrolment, the next **cycle** of the old one (which stays as it was), due on the expiry date, once for each expiry. Completing it issues a new certificate; the old one becomes **Superseded** and stays verifiable as such. Quiz attempts and assignment submissions are now counted per enrolment, not per learner, so a recertification starts fresh; migration `005` drops the earlier per-learner unique keys.

**Enrolment rules.** An **Enrolment Rule** names a course or a programme and who matches: the workspace's active users by role or group, or, with the HR app, its active employees who sign in by Employee fields (department, designation, branch, employment type and the like, by name or id; fields in permission groups are refused), read in place. A switched-on rule enrols at the next upkeep the people who match and have no enrolment in the course (so never twice, never someone who enrolled themselves), due **N days after matching** or as the course says, and, if it says **Withdraw If Not Started**, withdraws its unstarted enrolments of people who stop matching (the default keeps them). **Preview** says how many it would enrol before it is switched on. Enrolments it makes are not paid for: rules assign training.

**Training programmes.** A **Training Programme** lists courses, each with its own due days and whether it repeats (is enrolled again as its certificate nears expiry). A rule can name it. The **Programme Status** report shows each person and course with state and dates and the completion rate and overdue count; LMS managers also see people the rules match who are not enrolled yet.

**Teams.** **My Team's Training** lists the latest enrolment of each person who reports to the viewer in HR (one level), cut in the query to the viewer's own reports whatever parameters are sent; LMS managers see everyone or, with HR, one manager's team; without HR only they run it. With HR installed, a completed course goes on the employee's HR training record (the course's name as the HR training program, valid until the certificate's last day).

Not in this release: reminders to managers, escalation, the interface's own screens for any of it (the new records use the generic forms; the public verification page still says "revoked" for an expired or superseded certificate), and people outside the workspace signing up and buying.

## 0.7.0

Live classes run in the product, with attendance (LM-20, LM-21). A cohort's live session without a link of its own gets a meeting in the Mail, Chat and Calendar app (Comms 0.15.0 and a meeting server) when it is saved, hosted by the session's new Host or the cohort's first instructor; moving or shortening the session moves the meeting, removing it ends it. Members get their own join link where they see the session: the lesson page's data (`live`), the dashboard and the reminder. A session with a typed link, or a workspace without Comms or a meeting server, is as before.

A new lesson type, **Live**, is credited by a session that names it (**Lesson Credited**). Ten minutes after a session was due to end, Learn reads its meeting, records for every learner of the cohort whether they attended and for how many minutes, and marks the lesson done for those who were in the meeting for at least the session's **Attendance Needed (%)** (50 by default). Instructors mark attendance by hand with **Mark Attendance** (which also corrects the meeting's record and credits the lesson) and read it with **Session Attendance**; this is all a workspace without a meeting server has. Nothing is taken back once credited. The time in the meeting is accurate to about a minute (see Comms 0.15.0).
## 0.6.16

Rebuilt with Ferrum-Frontend 9e7190c, the design system as tuned after the landed build was reviewed. The left menu's icons are chosen so that none repeats on more than three rows and each fits its row; their drawings load beside the first page instead of inside it, so the page starts lighter; a row that scrolls sideways fades at the edge where there is more, in every browser and with reduced motion; a printed record leaves out empty fields and keeps its totals with the table; menus and popovers sit inside the page's landmarks for assistive technology; a selection bar folds what does not fit into More on a narrow screen. Nothing else changes.

## 0.6.15

The interface follows the upgraded design system, rebuilt with Ferrum-Frontend 76c4fad. Text uses the kit's type roles (nothing under 12px) and controls its height tokens; loading shows skeletons shaped like what is coming; empty, no matches, denied, offline and failed are told apart; status colours come from stored state ids. The interface check now runs the UI lint against an empty baseline. Progress lines, class bands and the teaching figures use the kit's Progress and Stat; the learners page has the kit's page head.

## 0.6.14

The reports it defines in code (their titles and parameters) have names in Arabic and Hindi too, with its models' and manifest's (412 each in ui/src/locales/names). Translated by the model, to be reviewed by native speakers.

## 0.6.13

Rebuilt with Ferrum-Frontend 43d7779: the names of its models, fields and manifest in Arabic and Hindi (411 each in ui/src/locales/names, which the platform serves to every page of the workspace, so a record's labels, the launcher and the bar read in the person's language), and the sentences its pages still showed in English (split around a link or a figure, built in code from a template or word by word) whole and translated (504 words each in ui/src/locales). Translated by the model, to be reviewed by native speakers.

## 0.6.12

Rebuilt with Ferrum-Frontend 046a3a2: the sentences its pages built in code (counts, dates, what a toast or a screen reader says) go through t() too, as whole sentences where code chose a word, and carry built-in Arabic and Hindi (486 words each in ui/src/locales, translated by the model and to be reviewed by native speakers).

## 0.6.11

The assistant knows what its records mean in its pages (contributes.shell.assistant): their states, what counts and where figures come from, with questions it suggests asking there.

## 0.6.10

Rebuilt with Ferrum-Frontend 389b2af: its own words carry built-in Arabic and Hindi (459 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.6.9

Rebuilt with Ferrum-Frontend 1f5981d: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (412 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.6.8

Rebuilt with Ferrum-Frontend 8a1c2e7: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else. Its own screens' margins, paddings, borders, alignment and insets go by the line's start and end, so they mirror right to left as the kit's do; what it places by coordinates (boards, timelines, charts, handles) stays as it was.

## 0.6.7

Rebuilt with Ferrum-Frontend 9d80f4f: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.6.6

Rebuilt with Ferrum-Frontend 204b713: a new record's box whose default is 1 shows ticked, as it is saved. An action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once.

## 0.6.5

Rebuilt with Ferrum-Frontend b39fffb: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.6.4

Rebuilt with Ferrum-Frontend d350579: its pages show times on the workspace's clock, naming the zone when it is not the browser's, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.6.3

Rebuilt with Ferrum-Frontend 20d08e5: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.6.2

The record pages ask for what their actions read: a certificate's Revoke needs a reason, a course's Enrol Learners their emails, a cohort's Schedule Sessions the first date and time, and a submission's Grade a score, and none asked, so from those pages they could not work. They now ask; Learn's own screens send them as before.

## 0.6.1

A lesson's New quiz and New assignment make one where the lesson is, named after it, and pick it, so a quiz or assignment lesson saves at once; Edit quiz and Edit assignment then open it for its questions or instructions. They used to leave for a blank form, and the lesson lost its kind, which saves only with its quiz or assignment. Rebuilt with Ferrum-Frontend 22d49b9: a settings page is headed by what it is.

## 0.6.0

Lesson files stream: a lesson's file is sent by the server straight from storage, whatever its size, instead of being read whole into memory first (Ferrum's SiteResponse::file, FERRUM-OPS-006).

## 0.5.3

Rebuilt with Ferrum-Frontend ae13c69: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.5.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.5.1

Rebuilt with Ferrum-Frontend 89b7cc7: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

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
