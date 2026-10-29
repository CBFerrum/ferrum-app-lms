-- Learn: rules the database enforces so concurrent requests can never break them, and the private
-- bookkeeping of session reminders (not an entity, so it never shows in the generic API).

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
