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
