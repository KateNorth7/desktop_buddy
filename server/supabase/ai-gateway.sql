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
-- Per-user safeguards: a daily dollar cap per user, and spike detection that blocks a
-- user until the end of the UTC day and alerts the developers.
alter table public.buddy_ai_policy
  add column if not exists user_microusd_daily bigint not null default 400000
  check (user_microusd_daily > 0);
alter table public.buddy_ai_policy
  add column if not exists spike_multiplier numeric not null default 3
  check (spike_multiplier >= 1);
alter table public.buddy_ai_policy
  add column if not exists spike_min_microusd bigint not null default 200000
  check (spike_min_microusd >= 0);
alter table public.buddy_ai_policy
  add column if not exists burst_calls_10min integer not null default 60
  check (burst_calls_10min between 1 and 10000);
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

-- Numbers only, never content: one row per user, UTC day, route and model, kept 90 days.
create table if not exists public.buddy_ai_usage_daily (
  day date not null,
  user_id uuid not null,
  route text not null,
  model text not null,
  calls integer not null default 0,
  input_tokens bigint not null default 0,
  output_tokens bigint not null default 0,
  cache_write_tokens bigint not null default 0,
  cache_read_tokens bigint not null default 0,
  searches integer not null default 0,
  microusd bigint not null default 0,
  primary key(day, user_id, route, model)
);
alter table public.buddy_ai_usage_daily enable row level security;
revoke all on public.buddy_ai_usage_daily from public, anon, authenticated;
grant all on public.buddy_ai_usage_daily to service_role;

create table if not exists public.buddy_ai_blocks (
  user_id uuid primary key,
  blocked_until timestamptz not null,
  reason text not null check (reason in ('spend_spike','call_burst','manual')),
  created_at timestamptz not null default now()
);
alter table public.buddy_ai_blocks enable row level security;
revoke all on public.buddy_ai_blocks from public, anon, authenticated;
grant all on public.buddy_ai_blocks to service_role;

-- Developers' report: spend per user per UTC day.
create or replace view public.buddy_ai_spend_by_user_day with (security_invoker = true) as
  select day, user_id, sum(calls) as calls, sum(input_tokens) as input_tokens,
    sum(output_tokens) as output_tokens, sum(cache_write_tokens) as cache_write_tokens,
    sum(cache_read_tokens) as cache_read_tokens, sum(searches) as searches,
    round(sum(microusd) / 1000000.0, 4) as usd
  from public.buddy_ai_usage_daily group by day, user_id;
revoke all on public.buddy_ai_spend_by_user_day from public, anon, authenticated;
grant select on public.buddy_ai_spend_by_user_day to service_role;
revoke all on public.buddy_ai_counters from public, anon, authenticated;
grant all on public.buddy_ai_counters to service_role;

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
  if exists(select 1 from public.buddy_ai_blocks b where b.user_id = p_user and b.blocked_until > t) then
    return false;
  end if;
  insert into public.buddy_ai_counters(scope,bucket) values
    ('global:'||p_route,d),('user:'||p_user||':'||p_route,d),
    ('minute:'||p_user,m),('budget',d),('usd:'||p_user,d) on conflict do nothing;
  select * into counter from public.buddy_ai_counters where scope='global:'||p_route and bucket=d;
  if counter.calls >= daily_limit then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='user:'||p_user||':'||p_route and bucket=d;
  if counter.calls >= per_user_limit then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='minute:'||p_user and bucket=m;
  if counter.calls >= 30 then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='budget' and bucket=d;
  if counter.units + p_units > policy.budget_units_daily then return false; end if;
  if counter.microusd + p_microusd > policy.budget_microusd_daily then return false; end if;
  select * into counter from public.buddy_ai_counters where scope='usd:'||p_user and bucket=d;
  if counter.microusd + p_microusd > policy.user_microusd_daily then return false; end if;
  update public.buddy_ai_counters set calls=calls+1 where
    (scope in ('global:'||p_route, 'user:'||p_user||':'||p_route) and bucket=d)
    or (scope='minute:'||p_user and bucket=m);
  update public.buddy_ai_counters set units=units+p_units, microusd=microusd+p_microusd where scope='budget' and bucket=d;
  update public.buddy_ai_counters set calls=calls+1, microusd=microusd+p_microusd where scope='usd:'||p_user and bucket=d;
  delete from public.buddy_ai_counters where bucket < t - interval '2 days';
  return true;
end;
$$;
revoke all on function public.buddy_reserve_ai(uuid,text,bigint,bigint) from public, anon, authenticated;
grant execute on function public.buddy_reserve_ai(uuid,text,bigint,bigint) to service_role;

-- The gateway deployed before 0.51.0 reserves without a cost estimate. Keeping this
-- signature lets the SQL be applied before that function is replaced; its requests
-- count against call and unit limits only.
create or replace function public.buddy_reserve_ai(p_user uuid, p_route text, p_units bigint)
returns boolean language sql security definer set search_path = '' as $$
  select public.buddy_reserve_ai(p_user, p_route, p_units, 0);
$$;
revoke all on function public.buddy_reserve_ai(uuid,text,bigint) from public, anon, authenticated;
grant execute on function public.buddy_reserve_ai(uuid,text,bigint) to service_role;

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

-- Records one settled response per user and checks for a spike. p_delta is actual minus
-- the reserved estimate (as for buddy_settle_ai). Returns whether the user is now
-- blocked and whether this call newly blocked them, so the gateway alerts only once.
create or replace function public.buddy_record_ai_usage(
  p_user uuid, p_route text, p_model text, p_delta bigint, p_microusd bigint,
  p_input bigint, p_output bigint, p_cache_write bigint, p_cache_read bigint, p_searches integer)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  t timestamptz := now();
  d timestamptz := date_trunc('day', t at time zone 'UTC') at time zone 'UTC';
  today date := (t at time zone 'UTC')::date;
  policy public.buddy_ai_policy%rowtype;
  spent bigint;
  average numeric;
  burst bigint;
  reason text;
  newly boolean := false;
begin
  if p_user is null or p_route not in ('ai','search') or length(coalesce(p_model,'')) not between 1 and 80
     or p_delta < -2000000 or p_delta > 20000000 or p_microusd < 0 or p_microusd > 20000000
     or least(p_input, p_output, p_cache_write, p_cache_read) < 0 or p_searches < 0 then
    return jsonb_build_object('blocked', false, 'alert', false);
  end if;
  perform pg_advisory_xact_lock(1937075561, 20261028);
  select * into policy from public.buddy_ai_policy where id = true;
  insert into public.buddy_ai_counters(scope,bucket) values ('usd:'||p_user,d) on conflict do nothing;
  update public.buddy_ai_counters set microusd = greatest(0, microusd + p_delta) where scope='usd:'||p_user and bucket=d;
  insert into public.buddy_ai_usage_daily as u(day,user_id,route,model,calls,input_tokens,output_tokens,cache_write_tokens,cache_read_tokens,searches,microusd)
    values (today,p_user,p_route,p_model,1,p_input,p_output,p_cache_write,p_cache_read,p_searches,p_microusd)
    on conflict (day,user_id,route,model) do update set calls=u.calls+1, input_tokens=u.input_tokens+excluded.input_tokens,
      output_tokens=u.output_tokens+excluded.output_tokens, cache_write_tokens=u.cache_write_tokens+excluded.cache_write_tokens,
      cache_read_tokens=u.cache_read_tokens+excluded.cache_read_tokens, searches=u.searches+excluded.searches,
      microusd=u.microusd+excluded.microusd;
  delete from public.buddy_ai_usage_daily where day < today - 90;
  select coalesce(microusd,0) into spent from public.buddy_ai_counters where scope='usd:'||p_user and bucket=d;
  select coalesce(sum(microusd),0) / 7.0 into average from public.buddy_ai_usage_daily
    where user_id=p_user and day >= today - 7 and day < today;
  select coalesce(sum(calls),0) into burst from public.buddy_ai_counters
    where scope='minute:'||p_user and bucket > t - interval '10 minutes';
  if spent > policy.spike_min_microusd and spent > policy.spike_multiplier * average then
    reason := 'spend_spike';
  elsif burst > policy.burst_calls_10min then
    reason := 'call_burst';
  end if;
  if reason is not null and not exists(select 1 from public.buddy_ai_blocks where user_id=p_user and blocked_until > t) then
    insert into public.buddy_ai_blocks(user_id,blocked_until,reason,created_at) values (p_user,d + interval '1 day',reason,t)
      on conflict (user_id) do update set blocked_until=excluded.blocked_until, reason=excluded.reason, created_at=excluded.created_at;
    newly := true;
  end if;
  return jsonb_build_object(
    'blocked', exists(select 1 from public.buddy_ai_blocks where user_id=p_user and blocked_until > t),
    'alert', newly, 'reason', reason, 'today_microusd', spent,
    'average_microusd', round(average), 'calls_10min', burst);
end;
$$;
revoke all on function public.buddy_record_ai_usage(uuid,text,text,bigint,bigint,bigint,bigint,bigint,bigint,integer) from public, anon, authenticated;
grant execute on function public.buddy_record_ai_usage(uuid,text,text,bigint,bigint,bigint,bigint,bigint,bigint,integer) to service_role;

-- Developers lift a block by hand: select public.buddy_unblock_ai('<user id>');
create or replace function public.buddy_unblock_ai(p_user uuid)
returns boolean language plpgsql security definer set search_path = '' as $$
begin
  delete from public.buddy_ai_blocks where user_id = p_user;
  return found;
end;
$$;
revoke all on function public.buddy_unblock_ai(uuid) from public, anon, authenticated;
grant execute on function public.buddy_unblock_ai(uuid) to service_role;
