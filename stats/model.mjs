// Pure helpers for the team stats page; no DOM, so they are unit-tested.
export const STATS_URL =
  "https://qrspcvglmmrputcltgxr.supabase.co/functions/v1/buddy-stats";
export const KEY_STORAGE = "sumi-stats-key";

const integer = new Intl.NumberFormat("en-US");
export const formatNumber = (value) =>
  value === null || value === undefined || Number.isNaN(value)
    ? "–"
    : integer.format(Math.round(value));

export function formatMinutes(minutes) {
  if (minutes === null || minutes === undefined) return "–";
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const hours = minutes / 60;
  return hours < 10 ? `${hours.toFixed(1).replace(/\.0$/, "")} h` : `${Math.round(hours)} h`;
}

export function formatSeconds(seconds) {
  if (!seconds) return "–";
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return m ? `${m}m ${String(s).padStart(2, "0")}s` : `${s}s`;
}

export const percent = (part, whole) =>
  whole > 0 ? `${Math.round((part / whole) * 100)}%` : "–";

/** Funnel steps from three sources; missing sources show as unknown, never zero. */
export function funnel(body) {
  const site = body.site?.ok && body.site.data?.configured ? body.site.data : null;
  const downloads = body.downloads?.ok ? body.downloads.data.totals : null;
  const users = body.users?.ok ? body.users.data : null;
  const clicks = site?.events?.find((e) => e.name === "download")?.value ?? null;
  const steps = [
    { label: "Website visitors", value: site ? site.totals.visitors : null, note: site ? "30 days, consented" : "Connect Google Analytics" },
    { label: "Download clicks", value: clicks, note: site ? "30 days, consented" : "Connect Google Analytics" },
    {
      label: "Installers downloaded",
      value: downloads ? downloads.mac_installer + downloads.windows_installer : null,
      note: downloads ? `Mac ${formatNumber(downloads.mac_installer)} · Windows ${formatNumber(downloads.windows_installer)}` : "GitHub unavailable",
    },
    { label: "Installs opened", value: users ? users.people : null, note: "Started sumi at least once" },
    { label: "Active this week", value: users ? users.active_7d : null, note: "Opened in the last 7 days" },
  ];
  const top = steps.find((s) => s.value)?.value ?? 0;
  return steps.map((s) => ({ ...s, share: s.value !== null && top ? s.value / top : 0 }));
}

/** Cohort rows: week 0 size and the share back in each later week. */
export function cohortTable(cohorts) {
  const byCohort = new Map();
  for (const c of cohorts) {
    const row = byCohort.get(c.cohort) ?? [];
    row[c.week] = c.people;
    byCohort.set(c.cohort, row);
  }
  const width = Math.max(1, ...[...byCohort.values()].map((r) => r.length));
  return [...byCohort.entries()].map(([cohort, row]) => ({
    cohort,
    size: row[0] ?? 0,
    weeks: Array.from({ length: width }, (_, i) =>
      i === 0 ? null : row[i] === undefined ? undefined : row[i],
    ).slice(1),
  }));
}

/** A rounded scale maximum and three gridline values. */
export function niceScale(max) {
  if (!max) return { max: 4, ticks: [0, 2, 4] };
  const raw = max / 3;
  const power = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * power).find((s) => s >= raw);
  return { max: step * 3, ticks: [0, step, step * 2, step * 3] };
}

/** Status of one install from its last session refresh and last AI minute. */
export function installStatus(install, now) {
  const since = (iso) => (iso ? (now - Date.parse(iso)) / 60_000 : Infinity);
  const seen = since(install.last_seen);
  if (seen <= 75 || since(install.last_ai) <= 15) return { kind: "open", label: "Open now" };
  if (seen <= 24 * 60) return { kind: "today", label: "Last 24 h" };
  if (seen <= 7 * 24 * 60) return { kind: "week", label: "This week" };
  return { kind: "quiet", label: "Quiet" };
}

export function relativeTime(iso, now) {
  if (!iso) return "–";
  const minutes = Math.max(0, Math.round((now - Date.parse(iso)) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

/** A readable label for a clicked URL: host and the last path part, no query. */
export function shortUrl(url) {
  try {
    const u = new URL(url);
    const host = u.hostname.replace(/^www\./, "");
    const file = u.pathname.split("/").filter(Boolean).pop();
    if (host === "youtube.com" || host === "youtu.be") return "YouTube video";
    if (host === "github.com" && file) return `GitHub · ${file}`;
    return file ? `${host} · ${file}` : host;
  } catch {
    return url || "(not set)";
  }
}
