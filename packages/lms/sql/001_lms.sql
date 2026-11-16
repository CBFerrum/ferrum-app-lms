-- Learn: rules the database enforces so concurrent requests can never break them, and its engine
-- tables (not entities, so the generic API never shows them).

-- At most one attempt in progress per learner and quiz (the start_quiz action resumes it).
create unique index uq_lms_attempt_in_progress on e_lms_quiz_attempt (tenant_id, quiz, learner) where status = 'In Progress';

-- At most one ungraded submission per learner and assignment (it is edited, not duplicated).
create unique index uq_lms_submission_pending on e_lms_assignment_submission (tenant_id, assignment, learner) where status = 'Submitted';

-- Scores stay within their maximum.
alter table e_lms_quiz_attempt add constraint ck_lms_attempt_score check (score is null or (score >= 0 and score <= max_score));
alter table e_lms_assignment_submission add constraint ck_lms_submission_score check (score is null or (score >= 0 and score <= max_score));

-- Maintenance reads only what is due: open attempts by deadline, upcoming live sessions, enrolments
-- that course changes completed and completed ones still owed a certificate.
create index ix_lms_attempt_open_deadline on e_lms_quiz_attempt (tenant_id, deadline) where status = 'In Progress' and deadline is not null;
create index ix_lms_cohort_session_start on e_lms_cohort__sessions (tenant_id, starts_at);
create index ix_lms_enrollment_ready on e_lms_enrollment (tenant_id)
  where status = 'Active' and total_lessons > 0 and completed_lessons >= total_lessons;
create index ix_lms_enrollment_uncertified on e_lms_enrollment (tenant_id, course) where status = 'Completed' and certificate is null;

-- Live-session reminders already sent: one row per session start, claimed before anyone is told,
-- so every member hears about a session once (a rescheduled session is announced again).
create table lms_session_reminder (
  tenant_id uuid not null,
  session_id uuid not null,
  starts_at timestamptz not null,
  sent_at timestamptz not null default now(),
  recipients int not null default 0,
  primary key (tenant_id, session_id, starts_at),
  foreign key (tenant_id, session_id) references e_lms_cohort__sessions (tenant_id, id) on delete cascade
);
create index ix_lms_session_reminder_start on lms_session_reminder (starts_at);

alter table lms_session_reminder enable row level security;
alter table lms_session_reminder force row level security;
create policy tenant_isolation on lms_session_reminder
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Learn's xAPI endpoint (src/xapi.rs): the statements xAPI lessons send and the state documents
-- they keep (bookmarks, resume data). Engine tables, not entities: a lesson's package writes them
-- through the endpoint only, as the signed-in learner.

create table lms_xapi_statement (
  tenant_id uuid not null,
  id uuid not null,
  lesson uuid not null,
  enrollment uuid not null,
  learner text not null,
  verb text not null,
  object_id text,
  registration uuid,
  voided boolean not null default false,
  stored timestamptz not null default now(),
  statement jsonb not null,
  primary key (tenant_id, id),
  foreign key (tenant_id, lesson) references e_lms_lesson (tenant_id, id) on delete cascade,
  foreign key (tenant_id, enrollment) references e_lms_enrollment (tenant_id, id) on delete cascade
);
create index ix_lms_xapi_statement_lesson on lms_xapi_statement (tenant_id, enrollment, lesson, stored);

create table lms_xapi_state (
  tenant_id uuid not null,
  enrollment uuid not null,
  lesson uuid not null,
  activity_id text not null,
  registration text not null default '',
  state_id text not null,
  content_type text not null,
  body bytea not null,
  updated_at timestamptz not null default now(),
  primary key (tenant_id, enrollment, lesson, activity_id, registration, state_id),
  foreign key (tenant_id, lesson) references e_lms_lesson (tenant_id, id) on delete cascade,
  foreign key (tenant_id, enrollment) references e_lms_enrollment (tenant_id, id) on delete cascade
);

alter table lms_xapi_statement enable row level security;
alter table lms_xapi_statement force row level security;
create policy tenant_isolation on lms_xapi_statement
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
alter table lms_xapi_state enable row level security;
alter table lms_xapi_state force row level security;
create policy tenant_isolation on lms_xapi_state
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Learn's LTI token endpoint (src/lti.rs): the client assertions tools have spent, so that each is
-- used once (kept an hour past their expiry, then purged).

create table lms_lti_jti (
  tenant_id uuid not null,
  client_id text not null,
  jti text not null,
  expires_at timestamptz not null,
  primary key (tenant_id, client_id, jti)
);

alter table lms_lti_jti enable row level security;
alter table lms_lti_jti force row level security;
create policy tenant_isolation on lms_lti_jti
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Attendance of live sessions (src/live.rs): one row per enrolled learner and session, from the
-- meeting (Comms) once it is over or marked by an instructor, and which sessions were read already.
create table lms_session_attendance (
  tenant_id uuid not null,
  session_id uuid not null,
  learner text not null,
  attended boolean not null,
  minutes int not null default 0,
  source text not null,
  marked_by text,
  marked_at timestamptz not null default now(),
  primary key (tenant_id, session_id, learner),
  foreign key (tenant_id, session_id) references e_lms_cohort__sessions (tenant_id, id) on delete cascade
);

create table lms_session_closed (
  tenant_id uuid not null,
  session_id uuid not null,
  closed_at timestamptz not null default now(),
  primary key (tenant_id, session_id),
  foreign key (tenant_id, session_id) references e_lms_cohort__sessions (tenant_id, id) on delete cascade
);

alter table lms_session_attendance enable row level security;
alter table lms_session_attendance force row level security;
create policy tenant_isolation on lms_session_attendance
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
alter table lms_session_closed enable row level security;
alter table lms_session_closed force row level security;
create policy tenant_isolation on lms_session_closed
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Questions graded by a person (src/quiz.rs, grade_answers): with a quiz's second grader, the grades
-- that are not final yet (the first grader's, or two that differ) are kept here, apart from the
-- attempt its learner reads; the final grade moves onto the attempt and its notes go.
create table lms_grading_note (
  tenant_id uuid not null,
  attempt uuid not null,
  question uuid not null,
  grader text not null,
  points numeric(18, 2) not null,
  comment text,
  graded_at timestamptz not null default now(),
  primary key (tenant_id, attempt, question, grader),
  foreign key (tenant_id, attempt) references e_lms_quiz_attempt (tenant_id, id) on delete cascade
);

alter table lms_grading_note enable row level security;
alter table lms_grading_note force row level security;
create policy tenant_isolation on lms_grading_note
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Uploaded lesson videos (src/media.rs): the key each workspace signs its expiring video links with,
-- made on first use. Not an entity, so no API, export or listing ever shows it.
create table lms_video_key (
  tenant_id uuid primary key,
  key text not null,
  created_at timestamptz not null default now()
);

alter table lms_video_key enable row level security;
alter table lms_video_key force row level security;
create policy tenant_isolation on lms_video_key
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));
