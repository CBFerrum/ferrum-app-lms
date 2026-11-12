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
