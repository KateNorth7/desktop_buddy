-- Shared AI stores counters only. Prompts, screenshots, goals and responses are not stored.
create table if not exists public.buddy_ai_policy (
  id boolean primary key default true check (id),
  enabled boolean not null default false,
  ai_daily integer not null default 3000 check (ai_daily between 1 and 100000),
  search_daily integer not null default 300 check (search_daily between 1 and 10000),
  budget_units_daily bigint not null default 30000000 check (budget_units_daily > 0)
);
insert into public.buddy_ai_policy(id) values (true) on conflict do nothing;
-- Daily spend ceiling for everyone together, in micro-dollars (1,000,000 = $1.00).
-- Estimates are reserved before each provider call and settled to the provider's
-- reported usage afterwards; failed or timed-out requests keep their estimate.
alter table public.buddy_ai_policy
  add column if not exists budget_microusd_daily bigint not null default 1000000
  check (budget_microusd_daily > 0);
alter table public.buddy_ai_policy enable row level security;
revoke all on public.buddy_ai_policy from public, anon, authenticated;
grant all on public.buddy_ai_policy to service_role;

create table if not exists public.buddy_ai_counters (
  scope text not null,
  bucket timestamptz not null,
  calls integer not null default 0,
  units bigint not null default 0,
  primary key(scope, bucket)
);
alter table public.buddy_ai_counters add column if not exists microusd bigint not null default 0;
alter table public.buddy_ai_counters enable row level security;
revoke all on public.buddy_ai_counters from public, anon, authenticated;
grant all on public.buddy_ai_counters to service_role;

drop function if exists public.buddy_reserve_ai(uuid, text, bigint);
create or replace function public.buddy_reserve_ai(p_user uuid, p_route text, p_units bigint, p_microusd bigint)
returns boolean language plpgsql security definer set search_path = '' as $$
declare
  t timestamptz := now();
  d timestamptz := date_trunc('day', t at time zone 'UTC') at time zone 'UTC';
  m timestamptz := date_trunc('minute', t);
  policy public.buddy_ai_policy%rowtype;
  counter public.buddy_ai_counters%rowtype;
  daily_limit integer;
  per_user_limit integer;
begin
  if p_user is null or p_route not in ('ai','search','health') or p_units < 0 or p_units > 400000
     or p_microusd < 0 or p_microusd > 2000000 then return false; end if;
  -- One short transaction serializes all reservations, including across Edge isolates.
  perform pg_advisory_xact_lock(1937075561, 20261028);
  select * into policy from public.buddy_ai_policy where id = true;
  if not found or not policy.enabled then return false; end if;
  daily_limit := case p_route when 'ai' then policy.ai_daily when 'search' then policy.search_daily else 10000 end;
  per_user_limit := case p_route when 'ai' then 1000 when 'search' then 100 else 200 end;
  insert into public.buddy_ai_counters(scope,bucket) values
    ('global:'||p_route,d),('user:'||p_user||':'||p_route,d),
    ('minute:'||p_user,m),('budget',d) on conflict do nothing;
  select * into counter from public.buddy_ai_counters where scope='global:'||p_route and bucket=d;
  if counter.calls >= daily_limit then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='user:'||p_user||':'||p_route and bucket=d;
  if counter.calls >= per_user_limit then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='minute:'||p_user and bucket=m;
  if counter.calls >= 30 then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='budget' and bucket=d;
  if counter.units + p_units > policy.budget_units_daily then return false; end if;
  if counter.microusd + p_microusd > policy.budget_microusd_daily then return false; end if;
  update public.buddy_ai_counters set calls=calls+1 where
    (scope in ('global:'||p_route, 'user:'||p_user||':'||p_route) and bucket=d)
    or (scope='minute:'||p_user and bucket=m);
  update public.buddy_ai_counters set units=units+p_units, microusd=microusd+p_microusd where scope='budget' and bucket=d;
  delete from public.buddy_ai_counters where bucket < t - interval '2 days';
  return true;
end;
$$;
revoke all on function public.buddy_reserve_ai(uuid,text,bigint,bigint) from public, anon, authenticated;
grant execute on function public.buddy_reserve_ai(uuid,text,bigint,bigint) to service_role;

-- Replaces a request's reserved estimate with the provider's reported usage.
-- p_delta is actual minus estimate in micro-dollars; today's spend never drops below zero.
create or replace function public.buddy_settle_ai(p_user uuid, p_delta bigint)
returns boolean language plpgsql security definer set search_path = '' as $$
declare
  t timestamptz := now();
  d timestamptz := date_trunc('day', t at time zone 'UTC') at time zone 'UTC';
begin
  if p_user is null or p_delta < -2000000 or p_delta > 20000000 then return false; end if;
  perform pg_advisory_xact_lock(1937075561, 20261028);
  insert into public.buddy_ai_counters(scope,bucket) values ('budget',d) on conflict do nothing;
  update public.buddy_ai_counters set microusd = greatest(0, microusd + p_delta) where scope='budget' and bucket=d;
  return true;
end;
$$;
revoke all on function public.buddy_settle_ai(uuid,bigint) from public, anon, authenticated;
grant execute on function public.buddy_settle_ai(uuid,bigint) to service_role;
