// Website numbers from the Google Analytics 4 Data API, read with a service account
// that has Viewer access to the property. Only consenting visitors are in GA4.
type Env = (name: string) => string | undefined;

const b64url = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
const text = (value: string) => b64url(new TextEncoder().encode(value));

async function accessToken(
  account: { client_email: string; private_key: string },
  fetcher: typeof fetch,
) {
  const now = Math.floor(Date.now() / 1000);
  const unsigned = `${text(JSON.stringify({ alg: "RS256", typ: "JWT" }))}.${text(
    JSON.stringify({
      iss: account.client_email,
      scope: "https://www.googleapis.com/auth/analytics.readonly",
      aud: "https://oauth2.googleapis.com/token",
      iat: now,
      exp: now + 3600,
    }),
  )}`;
  const pem = account.private_key
    .replace(/-----[^-]+-----/g, "")
    .replace(/\s+/g, "");
  const key = await crypto.subtle.importKey(
    "pkcs8",
    Uint8Array.from(atob(pem), (c) => c.charCodeAt(0)),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = new Uint8Array(
    await crypto.subtle.sign(
      "RSASSA-PKCS1-v1_5",
      key,
      new TextEncoder().encode(unsigned),
    ),
  );
  const response = await fetcher("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer",
      assertion: `${unsigned}.${b64url(signature)}`,
    }),
  });
  if (!response.ok) throw new Error(`ga4_token_${response.status}`);
  return (await response.json()).access_token as string;
}

type Row = {
  dimensionValues?: { value: string }[];
  metricValues?: { value: string }[];
};

export async function siteStats(env: Env, fetcher: typeof fetch) {
  const property = env("GA4_PROPERTY_ID");
  const credentials = env("GA4_SERVICE_ACCOUNT");
  if (!property || !credentials) return { configured: false };
  const token = await accessToken(JSON.parse(credentials), fetcher);
  const call = async (
    method: "runReport" | "runRealtimeReport",
    body: unknown,
  ) => {
    const response = await fetcher(
      `https://analyticsdata.googleapis.com/v1beta/properties/${property}:${method}`,
      {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          "content-type": "application/json",
        },
        body: JSON.stringify(body),
      },
    );
    if (!response.ok) throw new Error(`ga4_${method}_${response.status}`);
    return ((await response.json()).rows ?? []) as Row[];
  };
  const range = [{ startDate: "29daysAgo", endDate: "today" }];
  const top = (dimension: string, metric = "activeUsers", limit = 8) =>
    call("runReport", {
      dateRanges: range,
      dimensions: [{ name: dimension }],
      metrics: [{ name: metric }],
      orderBys: [{ metric: { metricName: metric }, desc: true }],
      limit,
    }).then((rows) =>
      rows.map((r) => ({
        name: r.dimensionValues?.[0]?.value ?? "",
        value: Number(r.metricValues?.[0]?.value ?? 0),
      })),
    );
  const [
    daily,
    totals,
    events,
    sources,
    countries,
    devices,
    systems,
    realtime,
    links,
  ] = await Promise.all([
    call("runReport", {
      dateRanges: range,
      dimensions: [{ name: "date" }],
      metrics: [
        { name: "activeUsers" },
        { name: "sessions" },
        { name: "screenPageViews" },
      ],
      orderBys: [{ dimension: { dimensionName: "date" } }],
    }),
    call("runReport", {
      dateRanges: range,
      metrics: [
        { name: "activeUsers" },
        { name: "newUsers" },
        { name: "sessions" },
        { name: "screenPageViews" },
        { name: "averageSessionDuration" },
        { name: "engagementRate" },
      ],
    }),
    top("eventName", "eventCount", 20),
    top("sessionSource"),
    top("country"),
    top("deviceCategory"),
    top("operatingSystem"),
    call("runRealtimeReport", { metrics: [{ name: "activeUsers" }] }),
    call("runReport", {
      dateRanges: range,
      dimensions: [{ name: "linkUrl" }],
      metrics: [{ name: "eventCount" }],
      dimensionFilter: {
        filter: { fieldName: "eventName", stringFilter: { value: "click" } },
      },
      orderBys: [{ metric: { metricName: "eventCount" }, desc: true }],
      limit: 10,
    }),
  ]);
  const t = totals[0]?.metricValues?.map((m) => Number(m.value)) ?? [];
  return {
    configured: true,
    totals: {
      visitors: t[0] ?? 0,
      new_visitors: t[1] ?? 0,
      sessions: t[2] ?? 0,
      page_views: t[3] ?? 0,
      avg_session_seconds: t[4] ?? 0,
      engagement_rate: t[5] ?? 0,
    },
    realtime_visitors: Number(realtime[0]?.metricValues?.[0]?.value ?? 0),
    daily: daily.map((r) => {
      const d = r.dimensionValues?.[0]?.value ?? "";
      const [users, sessions, views] = (r.metricValues ?? []).map((m) =>
        Number(m.value),
      );
      return {
        day: `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`,
        visitors: users,
        sessions,
        views,
      };
    }),
    events: events.filter((e) =>
      [
        "download",
        "play_video",
        "watch_on_youtube",
        "open_github",
        "open_privacy",
        "companion_reply",
        "page_view",
        "first_visit",
        "scroll",
        "click",
      ].includes(e.name),
    ),
    links: links.map((r) => ({
      name: r.dimensionValues?.[0]?.value ?? "",
      value: Number(r.metricValues?.[0]?.value ?? 0),
    })),
    sources,
    countries,
    devices,
    systems,
  };
}
