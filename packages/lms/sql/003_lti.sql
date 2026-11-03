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
