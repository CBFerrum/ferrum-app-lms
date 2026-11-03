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
