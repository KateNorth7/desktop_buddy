// Team statistics for the sumi stats page. Aggregates only: no user IDs, content or
// per-person rows ever leave this function. The app itself sends nothing new; every
// number comes from records the built-in AI gateway already keeps, GitHub release
// download counters and, when configured, Google Analytics 4 for the website.

export type Sql = (query: string) => Promise<Record<string, unknown>[]>;
type Env = (name: string) => string | undefined;

const REPOSITORY = "KateNorth7/desktop_buddy";
const CACHE_MS = 5 * 60_000;
// A gap longer than this between AI calls ends a session.
export const SESSION_GAP_MINUTES = 15;
const ALLOWED_ORIGINS = [
  "https://katenorth7.github.io",
  "http://localhost:4173",
  "http://127.0.0.1:4173",
];

const hex = async (value: string) =>
  Array.from(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value)),
    ),
    (b) => b.toString(16).padStart(2, "0"),
  ).join("");

/** Constant-time comparison of the presented team key with the configured one. */
export async function keyMatches(presented: string | null, expected: string) {
  if (!presented || expected.length < 24) return false;
  const [a, b] = await Promise.all([hex(presented), hex(expected)]);
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/**
 * Sessions from per-user active minutes: consecutive minutes with AI calls, split by
 * gaps longer than `gap` minutes. Returns durations in minutes; a lone minute
 * counts as one minute. Users stay anonymous: only durations come back.
 */
export function sessionDurations(
  minutesByUser: Map<string, number[]>,
  gap = SESSION_GAP_MINUTES,
) {
  const durations: number[] = [];
  for (const minutes of minutesByUser.values()) {
    const sorted = [...minutes].sort((a, b) => a - b);
    let start = sorted[0];
    let last = sorted[0];
    for (const minute of sorted.slice(1)) {
      if (minute - last > gap) {
        durations.push(last - start + 1);
        start = minute;
      }
      last = minute;
    }
    if (start !== undefined) durations.push(last - start + 1);
  }
  return durations;
}

export function percentile(values: number[], p: number) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[
    Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))
  ];
}

type Asset = { name: string; download_count: number };
type Release = {
  tag_name: string;
  published_at: string;
  draft: boolean;
  assets: Asset[];
};

/** Installers versus updates per release. Windows updates reuse the installer file. */
export function summarizeReleases(releases: Release[]) {
  const rows = releases
    .filter((r) => !r.draft)
    .map((r) => {
      const count = (pattern: RegExp) =>
        r.assets
          .filter((a) => pattern.test(a.name))
          .reduce((n, a) => n + a.download_count, 0);
      return {
        version: r.tag_name.replace(/^v/, ""),
        published: r.published_at,
        mac_installer: count(/\.dmg$/),
        mac_update: count(/\.app\.tar\.gz$/),
        windows_installer: count(/setup\.exe$/),
      };
    });
  const total = (key: "mac_installer" | "mac_update" | "windows_installer") =>
    rows.reduce((n, r) => n + r[key], 0);
  return {
    releases: rows,
    totals: {
      mac_installer: total("mac_installer"),
      mac_update: total("mac_update"),
      windows_installer: total("windows_installer"),
    },
  };
}

const num = (value: unknown) => Number(value ?? 0);
const iso = (value: unknown) =>
  value === null || value === undefined
    ? null
    : new Date(value as string | Date).toISOString();

// Activity comes from records Supabase Auth and the gateway already keep:
// - every refresh of the app's anonymous session (about hourly while sumi runs);
// - the gateway's per-minute AI counters, which it keeps for two days.
// People = anonymous sessions that refreshed at least once. The daily availability
// check signs out (deleting its session and tokens) and test tooling identifies
// itself as "node", so neither counts.
const ACTIVITY = `
  with tok as (
    select r.user_id::text as user_id, r.created_at
    from auth.refresh_tokens r join auth.sessions s on s.id = r.session_id
    where coalesce(s.user_agent, '') <> 'node'
  ), people as (
    select t.user_id, least(min(t.created_at), min(a.created_at)) as first_seen, max(t.created_at) as last_seen
    from tok t left join auth.users a on a.id::text = t.user_id group by t.user_id
  )`;
// Consecutive refreshes further apart than this end a session.
export const REFRESH_GAP_MINUTES = 90;

async function supabaseStats(sql: Sql) {
  const [summary] = await sql(`${ACTIVITY}
    select
      (select count(*) from people) as people,
      (select count(*) from people where first_seen > now() - interval '7 days') as new_7d,
      (select count(distinct user_id) from tok where created_at >= current_date) as active_today,
      (select count(distinct user_id) from tok where created_at > now() - interval '7 days') as active_7d,
      (select count(distinct user_id) from tok where created_at > now() - interval '30 days') as active_30d,
      (select count(*) from people where last_seen > now() - interval '24 hours') as seen_24h,
      (select min(first_seen)::date::text from people) as tracked_since
  `);
  const daily = await sql(`${ACTIVITY},
    days as (select generate_series(current_date - 29, current_date, interval '1 day')::date as day)
    select d.day::text as day,
      (select count(distinct user_id) from tok where created_at::date = d.day) as active,
      (select count(*) from people where first_seen::date = d.day) as new_people,
      (select count(distinct (user_id, date_trunc('hour', created_at))) from tok where created_at::date = d.day) as app_hours
    from days d order by d.day
  `);
  const cohorts = await sql(`${ACTIVITY},
    weeks as (select distinct user_id, date_trunc('week', created_at)::date as week from tok)
    select date_trunc('week', p.first_seen)::date::text as cohort,
      ((w.week - date_trunc('week', p.first_seen)::date) / 7) as week_offset,
      count(distinct w.user_id) as people
    from people p join weeks w using (user_id)
    group by 1, 2 order by 1, 2
  `);
  const refreshes = await sql(`${ACTIVITY}
    select user_id, (extract(epoch from created_at) / 60)::bigint as minute
    from tok where created_at > now() - interval '30 days'
  `);
  const minutes = await sql(`${ACTIVITY}
    select substring(c.scope from 8) as user_key,
      (extract(epoch from c.bucket) / 60)::bigint as minute, c.calls
    from public.buddy_ai_counters c
    where c.scope like 'minute:%' and c.calls > 0 and c.bucket > now() - interval '48 hours'
      and substring(c.scope from 8) in (select user_id from people)
  `);
  const [ai] = await sql(`
    select
      coalesce(sum(calls) filter (where scope = 'global:ai' and bucket = current_date), 0) as ai_today,
      coalesce(sum(calls) filter (where scope = 'global:search' and bucket = current_date), 0) as search_today,
      coalesce(sum(calls) filter (where scope = 'global:ai' and bucket = current_date - 1), 0) as ai_yesterday,
      coalesce(sum(calls) filter (where scope = 'global:search' and bucket = current_date - 1), 0) as search_yesterday
    from public.buddy_ai_counters
  `);
  // One row per install, numbered by first appearance. The anonymous ID never leaves:
  // the page sees only "#n" and timings.
  const installs = await sql(`${ACTIVITY}
    select row_number() over (order by p.first_seen) as n,
      p.first_seen, p.last_seen,
      (select max(c.bucket) from public.buddy_ai_counters c
        where c.scope = 'minute:' || p.user_id and c.calls > 0) as last_ai,
      (select count(distinct created_at::date) from tok t
        where t.user_id = p.user_id and t.created_at > now() - interval '30 days') as days_30d,
      (select count(distinct date_trunc('hour', created_at)) from tok t
        where t.user_id = p.user_id and t.created_at > now() - interval '7 days') as hours_7d,
      (select count(*) from tok t
        where t.user_id = p.user_id and t.created_at > now() - interval '7 days') as refreshes_7d,
      (select count(*) from tok t where t.user_id = p.user_id) as refreshes_total
    from people p
    order by p.last_seen desc
  `);

  // App sessions over 30 days from refresh chains: each refresh stands for about an hour.
  const chains = new Map<string, number[]>();
  for (const row of refreshes) {
    const user = String(row.user_id);
    chains.set(user, [...(chains.get(user) ?? []), num(row.minute)]);
  }
  const appSessions = sessionDurations(chains, REFRESH_GAP_MINUTES).map(
    (m) => m + 59,
  );

  const now = Math.floor(Date.now() / 60_000);
  const byUser = new Map<string, number[]>();
  const hourly = new Map<number, { people: Set<string>; calls: number }>();
  const recent = { m15: new Set<string>(), m60: new Set<string>(), calls60: 0 };
  for (const row of minutes) {
    const user = String(row.user_key);
    const minute = num(row.minute);
    const calls = num(row.calls);
    byUser.set(user, [...(byUser.get(user) ?? []), minute]);
    const hour = Math.floor(minute / 60) * 60;
    const slot = hourly.get(hour) ?? { people: new Set<string>(), calls: 0 };
    slot.people.add(user);
    slot.calls += calls;
    hourly.set(hour, slot);
    if (now - minute < 15) recent.m15.add(user);
    if (now - minute < 60) {
      recent.m60.add(user);
      recent.calls60 += calls;
    }
  }
  const aiSessions = sessionDurations(byUser);
  const firstHour = Math.floor((now - 47 * 60) / 60) * 60;
  const hours = Array.from({ length: 48 }, (_, i) => {
    const hour = firstHour + i * 60;
    const slot = hourly.get(hour);
    return {
      hour: new Date(hour * 60_000).toISOString(),
      people: slot?.people.size ?? 0,
      calls: slot?.calls ?? 0,
    };
  });
  const describe = (durations: number[]) => ({
    count: durations.length,
    median_minutes: percentile(durations, 50),
    p75_minutes: percentile(durations, 75),
    longest_minutes: durations.length ? Math.max(...durations) : null,
  });
  return {
    people: num(summary.people),
    new_7d: num(summary.new_7d),
    active_today: num(summary.active_today),
    active_7d: num(summary.active_7d),
    active_30d: num(summary.active_30d),
    seen_24h: num(summary.seen_24h),
    tracked_since: summary.tracked_since
      ? String(summary.tracked_since).slice(0, 10)
      : null,
    recent: {
      people_15m: recent.m15.size,
      people_60m: recent.m60.size,
      calls_60m: recent.calls60,
    },
    app_sessions_30d: {
      ...describe(appSessions),
      gap_minutes: REFRESH_GAP_MINUTES,
    },
    ai_sessions_48h: {
      ...describe(aiSessions),
      gap_minutes: SESSION_GAP_MINUTES,
    },
    hourly_48h: hours,
    daily: daily.map((d) => ({
      day: String(d.day),
      active: num(d.active),
      new_people: num(d.new_people),
      app_hours: num(d.app_hours),
    })),
    cohorts: cohorts.map((c) => ({
      cohort: String(c.cohort),
      week: num(c.week_offset),
      people: num(c.people),
    })),
    installs: installs.map((i) => ({
      label: `#${num(i.n)}`,
      first_seen: iso(i.first_seen),
      last_seen: iso(i.last_seen),
      last_ai: iso(i.last_ai),
      days_30d: num(i.days_30d),
      hours_7d: num(i.hours_7d),
      refreshes_7d: num(i.refreshes_7d),
      refreshes_total: num(i.refreshes_total),
    })),
    ai: {
      calls_today: num(ai.ai_today),
      searches_today: num(ai.search_today),
      calls_yesterday: num(ai.ai_yesterday),
      searches_yesterday: num(ai.search_yesterday),
    },
  };
}

async function githubStats(env: Env, fetcher: typeof fetch) {
  const headers: Record<string, string> = {
    accept: "application/vnd.github+json",
    "user-agent": "sumi-stats",
  };
  const token = env("GITHUB_TOKEN");
  if (token) headers.authorization = `Bearer ${token}`;
  const response = await fetcher(
    `https://api.github.com/repos/${REPOSITORY}/releases?per_page=100`,
    { headers },
  );
  if (!response.ok) throw new Error(`github_${response.status}`);
  return summarizeReleases(await response.json());
}

export type SiteStats = (env: Env, fetcher: typeof fetch) => Promise<unknown>;

export function createStats(deps: {
  env: Env;
  fetch: typeof fetch;
  sql: Sql;
  site?: SiteStats;
}) {
  let cache: { at: number; body: unknown } | null = null;
  return async (request: Request) => {
    const origin = request.headers.get("origin") ?? "";
    const cors: Record<string, string> = {
      "access-control-allow-headers": "x-stats-key",
      "access-control-allow-methods": "GET, OPTIONS",
      vary: "origin",
    };
    if (ALLOWED_ORIGINS.includes(origin))
      cors["access-control-allow-origin"] = origin;
    if (request.method === "OPTIONS")
      return new Response(null, { status: 204, headers: cors });
    if (request.method !== "GET")
      return Response.json({ error: "method" }, { status: 405, headers: cors });
    const expected = deps.env("STATS_KEY");
    if (
      !expected ||
      !(await keyMatches(request.headers.get("x-stats-key"), expected))
    ) {
      return Response.json(
        { error: "unauthorized" },
        { status: 401, headers: cors },
      );
    }
    if (!cache || Date.now() - cache.at > CACHE_MS) {
      const settle = async <T>(work: () => Promise<T>) => {
        try {
          return { ok: true, data: await work() };
        } catch (error) {
          const reason = String(error).slice(0, 160);
          console.log(JSON.stringify({ event: "stats_source_failed", reason }));
          return { ok: false, reason };
        }
      };
      const [users, downloads, site] = await Promise.all([
        settle(() => supabaseStats(deps.sql)),
        settle(() => githubStats(deps.env, deps.fetch)),
        deps.site
          ? settle(() => deps.site!(deps.env, deps.fetch))
          : Promise.resolve({ ok: false, configured: false }),
      ]);
      cache = {
        at: Date.now(),
        body: {
          generated_at: new Date().toISOString(),
          users,
          downloads,
          site,
        },
      };
    }
    return Response.json(cache.body, {
      headers: { ...cors, "cache-control": "private, no-store" },
    });
  };
}
