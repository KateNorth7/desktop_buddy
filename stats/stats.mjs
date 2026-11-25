import {
  STATS_URL,
  KEY_STORAGE,
  cohortTable,
  formatMinutes,
  formatNumber,
  formatSeconds,
  funnel,
  niceScale,
  percent,
} from "./model.mjs?v=1";

const $ = (id) => document.getElementById(id);
const SVG = "http://www.w3.org/2000/svg";
const el = (tag, attrs = {}, ns) => {
  const node = ns ? document.createElementNS(ns, tag) : document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  return node;
};
const text = (tag, value, attrs) => {
  const node = el(tag, attrs);
  node.textContent = value;
  return node;
};

const store = {
  get() {
    try {
      return localStorage.getItem(KEY_STORAGE);
    } catch {
      return null;
    }
  },
  set(value) {
    try {
      localStorage.setItem(KEY_STORAGE, value);
    } catch {
      /* Private mode: the key lasts for this visit only. */
    }
  },
  clear() {
    try {
      localStorage.removeItem(KEY_STORAGE);
    } catch {
      /* nothing stored */
    }
  },
};
let sessionKey = store.get();

// --- Tooltip -----------------------------------------------------------------
const tip = $("tooltip");
function showTip(event, value, label, color) {
  tip.replaceChildren(text("b", value));
  const row = el("span", { class: "row" });
  if (color) {
    const key = el("i", { class: "key" });
    key.style.background = color;
    row.append(key);
  }
  row.append(document.createTextNode(label));
  tip.append(row);
  tip.hidden = false;
  const rect = event.target.getBoundingClientRect?.();
  const x = event.clientX ?? (rect ? rect.left + rect.width / 2 : 0);
  const y = event.clientY ?? (rect ? rect.top : 0);
  const width = tip.offsetWidth;
  tip.style.left = `${Math.min(window.innerWidth - width - 8, Math.max(8, x - width / 2))}px`;
  tip.style.top = `${Math.max(8, y - tip.offsetHeight - 12)}px`;
}
const hideTip = () => (tip.hidden = true);

// --- Bar chart (single series or stacked) --------------------------------------
function barChart(container, rows, { series, label, height = 180 }) {
  container.replaceChildren();
  const total = (r) => series.reduce((n, s) => n + (r[s.key] ?? 0), 0);
  if (!rows.length || rows.every((r) => total(r) === 0)) {
    container.append(text("p", "No activity yet.", { class: "empty" }));
    return;
  }
  const width = Math.max(300, Math.round(container.clientWidth || 640));
  const pad = { top: 8, right: 4, bottom: 22, left: 28 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const scale = niceScale(Math.max(...rows.map(total)));
  const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, height, role: "img" }, SVG);
  const title = el("title", {}, SVG);
  title.textContent = label;
  svg.append(title);
  for (const tick of scale.ticks) {
    const y = pad.top + plotH - (tick / scale.max) * plotH;
    svg.append(el("line", { class: "gridline", x1: pad.left, x2: width - pad.right, y1: y, y2: y }, SVG));
    const t = el("text", { class: "axis", x: pad.left - 6, y: y + 3, "text-anchor": "end" }, SVG);
    t.textContent = formatNumber(tick);
    svg.append(t);
  }
  const slot = plotW / rows.length;
  const barW = Math.max(3, Math.min(28, slot - 2));
  const labelEvery = Math.ceil(rows.length / Math.max(2, Math.floor(plotW / 64)));
  rows.forEach((row, i) => {
    const x = pad.left + i * slot + (slot - barW) / 2;
    let base = pad.top + plotH;
    const group = el("g", {}, SVG);
    const hit = el("rect", { class: "hit", x: pad.left + i * slot, y: pad.top, width: slot, height: plotH, tabindex: "0" }, SVG);
    const describe = series
      .map((s) => `${s.name}: ${formatNumber(row[s.key] ?? 0)}`)
      .join(" · ");
    const enter = (event) => showTip(event, series.length > 1 ? formatNumber(total(row)) : formatNumber(row[series[0].key] ?? 0), series.length > 1 ? `${row.label} — ${describe}` : `${row.label} · ${series[0].name}`, series.length > 1 ? null : series[0].color);
    hit.addEventListener("pointermove", enter);
    hit.addEventListener("focus", enter);
    hit.addEventListener("pointerleave", hideTip);
    hit.addEventListener("blur", hideTip);
    const bars = el("g", { class: "bar" }, SVG);
    series.forEach((s, si) => {
      const value = row[s.key] ?? 0;
      if (!value) return;
      const h = Math.max(1, (value / scale.max) * plotH - (si ? 2 : 0));
      const isTop = series.slice(si + 1).every((n) => !(row[n.key] ?? 0));
      const r = Math.min(4, barW / 2, h);
      const y = base - h;
      const path = isTop
        ? `M${x},${base} V${y + r} Q${x},${y} ${x + r},${y} H${x + barW - r} Q${x + barW},${y} ${x + barW},${y + r} V${base} Z`
        : `M${x},${base} V${y} H${x + barW} V${base} Z`;
      bars.append(el("path", { d: path, fill: s.color }, SVG));
      base = y - 2;
    });
    group.append(bars, hit);
    svg.append(group);
    if (i % labelEvery === 0) {
      const t = el("text", { class: "axis", x: pad.left + i * slot + slot / 2, y: height - 6, "text-anchor": "middle" }, SVG);
      t.textContent = row.tick ?? row.label;
      svg.append(t);
    }
  });
  container.append(svg);
}

// --- Rendering ---------------------------------------------------------------
const dayLabel = (iso) =>
  new Date(`${iso}T12:00:00Z`).toLocaleDateString("en-GB", { day: "numeric", month: "short" });
const ACCENT = "#4388a8";

function fact(list, label, value, small) {
  const div = el("div");
  div.append(text("dt", label));
  const dd = text("dd", value);
  if (small) dd.append(" ", text("small", small));
  div.append(dd);
  list.append(div);
}

function ranked(title, rows) {
  const box = el("div", { class: "ranked" });
  box.append(text("h3", title));
  const list = el("ol");
  const top = Math.max(1, ...rows.map((r) => r.value));
  for (const r of rows.slice(0, 6)) {
    const li = el("li");
    li.append(text("span", r.name || "(not set)"), text("span", formatNumber(r.value)));
    const bar = el("i");
    bar.style.width = `${(r.value / top) * 100}%`;
    li.append(bar);
    list.append(li);
  }
  if (!rows.length) list.append(text("li", "No data yet."));
  box.append(list);
  return box;
}

function render(body) {
  $("updated").textContent = `Updated ${new Date(body.generated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  const users = body.users?.ok ? body.users.data : null;
  const downloads = body.downloads?.ok ? body.downloads.data : null;

  // Tiles
  if (users) {
    $("k-recent").textContent = formatNumber(users.recent.people_60m);
    $("k-recent-sub").textContent = `last hour · ${formatNumber(users.recent.calls_60m)} AI calls · ${formatNumber(users.recent.people_15m)} in 15 min`;
    $("k-today").textContent = formatNumber(users.active_today);
    $("k-today-sub").textContent = `${formatNumber(users.seen_24h)} in the last 24 h`;
    $("k-week").textContent = formatNumber(users.active_7d);
    $("k-week-sub").textContent = `${percent(users.active_7d, users.people)} of installs · ${formatNumber(users.active_30d)} in 30 days`;
    $("k-people").textContent = formatNumber(users.people);
    $("k-people-sub").textContent = `${formatNumber(users.new_7d)} new this week · since ${users.tracked_since ? dayLabel(users.tracked_since) : "–"}`;
    $("k-session").textContent = formatMinutes(users.app_sessions_30d.median_minutes);
    $("k-session-sub").textContent = `app open, median · active AI ${formatMinutes(users.ai_sessions_48h.median_minutes)}`;
  }
  if (downloads) {
    const t = downloads.totals;
    $("k-downloads").textContent = formatNumber(t.mac_installer + t.windows_installer);
    $("k-downloads-sub").textContent = `installers · ${formatNumber(t.mac_update)} Mac updates`;
  }

  // Funnel
  const steps = funnel(body);
  $("funnel").replaceChildren(
    ...steps.map((s) => {
      const li = el("li");
      li.append(text("span", formatNumber(s.value), { class: "value" }), text("span", s.label, { class: "label" }), text("span", s.note, { class: "note" }));
      const meter = el("div", { class: "meter" });
      const fill = el("span");
      fill.style.width = `${Math.round(s.share * 100)}%`;
      meter.append(fill);
      li.append(meter);
      return li;
    }),
  );

  // Daily charts
  if (users) {
    const daily = users.daily.map((d) => ({ ...d, label: dayLabel(d.day), tick: dayLabel(d.day) }));
    barChart($("chart-active"), daily, { series: [{ key: "active", name: "People", color: ACCENT }], label: "People using sumi per day" });
    barChart($("chart-hours"), daily, { series: [{ key: "app_hours", name: "Hours open", color: ACCENT }], label: "Hours sumi was open per day" });
    const hourly = users.hourly_48h.map((h) => {
      const d = new Date(h.hour);
      const hh = d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
      return { ...h, label: `${d.toLocaleDateString("en-GB", { weekday: "short" })} ${hh}`, tick: d.getHours() === 0 ? d.toLocaleDateString("en-GB", { weekday: "short" }) : hh };
    });
    barChart($("chart-hourly"), hourly, { series: [{ key: "people", name: "People", color: ACCENT }], label: "People calling AI per hour" });

    const s = $("sessions");
    s.replaceChildren();
    const app = users.app_sessions_30d;
    const ai = users.ai_sessions_48h;
    fact(s, "App open · median", formatMinutes(app.median_minutes), `${formatNumber(app.count)} sessions, 30 days`);
    fact(s, "App open · longer quarter", formatMinutes(app.p75_minutes), `longest ${formatMinutes(app.longest_minutes)}`);
    fact(s, "Active AI work · median", formatMinutes(ai.median_minutes), `${formatNumber(ai.count)} bursts, 48 h`);
    fact(s, "Active AI work · longer quarter", formatMinutes(ai.p75_minutes), `longest ${formatMinutes(ai.longest_minutes)}`);

    // Retention
    const table = $("cohorts");
    table.replaceChildren();
    const rows = cohortTable(users.cohorts);
    const weeks = Math.max(0, ...rows.map((r) => r.weeks.length));
    const head = el("tr");
    head.append(text("th", "Started"), text("th", "People"));
    for (let w = 1; w <= weeks; w++) head.append(text("th", `Week ${w}`));
    table.append(head);
    for (const r of rows) {
      const tr = el("tr");
      tr.append(text("th", `Week of ${dayLabel(r.cohort)}`), text("td", formatNumber(r.size)));
      r.weeks.forEach((v) => {
        const share = v === undefined || !r.size ? null : v / r.size;
        const td = text("td", share === null ? "" : `${Math.round(share * 100)}%`);
        if (share !== null) {
          td.style.background = `rgb(67 136 168 / ${0.08 + share * 0.72})`;
          td.style.color = share > 0.55 ? "#fff" : "var(--text)";
          td.title = `${v} of ${r.size} people`;
        }
        tr.append(td);
      });
      table.append(tr);
    }
    const ai2 = $("ai");
    ai2.replaceChildren();
    fact(ai2, "AI requests today", formatNumber(users.ai.calls_today), `yesterday ${formatNumber(users.ai.calls_yesterday)}`);
    fact(ai2, "Web searches today", formatNumber(users.ai.searches_today), `yesterday ${formatNumber(users.ai.searches_yesterday)}`);
  } else {
    for (const id of ["chart-active", "chart-hours", "chart-hourly"])
      $(id).replaceChildren(text("p", `Supabase unavailable: ${body.users?.reason ?? "unknown"}`, { class: "empty" }));
  }

  // Downloads
  if (downloads) {
    const series = [
      { key: "mac_installer", name: "Mac installer", color: "#2a78d6" },
      { key: "mac_update", name: "Mac update", color: "#eb6834" },
      { key: "windows_installer", name: "Windows installer + updates", color: "#1baf7a" },
    ];
    $("downloads-legend").replaceChildren(
      ...series.map((s) => {
        const span = text("span", s.name);
        span.style.setProperty("--swatch", s.color);
        return span;
      }),
    );
    const rows = [...downloads.releases].reverse().map((r) => ({ ...r, label: `v${r.version}`, tick: r.version }));
    barChart($("chart-downloads"), rows.slice(-16), { series, label: "Downloads by release", height: 200 });
  }

  // Website
  const site = body.site?.ok ? body.site.data : null;
  const box = $("site");
  box.replaceChildren();
  if (!site?.configured) {
    $("site-note").textContent = "Google Analytics 4 for katenorth7.github.io/desktop_buddy.";
    box.append(text("div", "Not connected yet. Once the GA4 property grants Viewer access to sumi's stats service, visitors, sources, countries, devices and clicks appear here.", { class: "connect" }));
  } else {
    const list = el("dl", { class: "facts" });
    list.style.gridTemplateColumns = "repeat(4, minmax(0, 1fr))";
    fact(list, "Visitors", formatNumber(site.totals.visitors), `${formatNumber(site.realtime_visitors)} now`);
    fact(list, "Sessions", formatNumber(site.totals.sessions), `${formatNumber(site.totals.page_views)} page views`);
    fact(list, "Avg. session", formatSeconds(site.totals.avg_session_seconds));
    fact(list, "Engaged", percent(site.totals.engagement_rate, 1));
    box.append(list);
    const chart = el("div", { class: "chart" });
    chart.style.marginTop = "1.25rem";
    box.append(chart);
    barChart(chart, site.daily.map((d) => ({ ...d, label: dayLabel(d.day), tick: dayLabel(d.day) })), { series: [{ key: "visitors", name: "Visitors", color: ACCENT }], label: "Website visitors per day", height: 150 });
    const grid = el("div", { class: "site-grid" });
    grid.append(ranked("Clicks and events", site.events), ranked("Sources", site.sources), ranked("Countries", site.countries), ranked("Devices and systems", [...site.devices, ...site.systems]));
    box.append(grid);
  }
}

// --- Loading -----------------------------------------------------------------
let lastBody = null;
let resizeTimer = 0;
window.addEventListener("resize", () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => lastBody && !$("content").hidden && render(lastBody), 150);
});
async function load(key) {
  $("gate").hidden = true;
  $("loading").hidden = false;
  try {
    const response = await fetch(STATS_URL, { headers: { "x-stats-key": key } });
    if (response.status === 401) throw Object.assign(new Error("That key is not valid."), { gate: true });
    if (!response.ok) throw new Error(`The stats service answered ${response.status}.`);
    const body = await response.json();
    sessionKey = key;
    $("loading").hidden = true;
    $("content").hidden = false;
    $("refresh").hidden = false;
    $("signout").hidden = false;
    lastBody = body;
    render(body);
  } catch (error) {
    $("loading").hidden = true;
    if (error.gate) store.clear();
    $("gate").hidden = false;
    $("gate-error").textContent = error.message || "Could not reach the stats service.";
    $("gate-error").hidden = false;
  }
}

$("gate-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const key = $("gate-key").value.trim();
  if (!key) return;
  store.set(key);
  load(key);
});
$("refresh").addEventListener("click", () => sessionKey && load(sessionKey));
$("signout").addEventListener("click", () => {
  store.clear();
  sessionKey = null;
  $("content").hidden = true;
  $("refresh").hidden = true;
  $("signout").hidden = true;
  $("gate-error").hidden = true;
  $("gate").hidden = false;
});

if (sessionKey) load(sessionKey);
else $("gate").hidden = false;
