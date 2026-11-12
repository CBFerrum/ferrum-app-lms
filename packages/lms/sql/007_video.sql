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
