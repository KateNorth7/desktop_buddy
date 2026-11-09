// Operational health only: no provider secrets, inference, searches or private user data.
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const EXPECTED_MODELS = ["claude-opus-5-5"];
const LIMIT = 2_000_000;
class HealthFailure extends Error {
  constructor(stage, status = null) {
    super(stage);
    this.stage = stage;
    this.status = status;
  }
}
export function clientKeyExpiry(key) {
  if (typeof key !== "string" || key.split(".").length !== 3) return null;
  try {
    const value = JSON.parse(Buffer.from(key.split(".")[1], "base64url"));
    return Number.isFinite(value.exp) ? new Date(value.exp * 1000).toISOString() : null;
  } catch { return null; }
}
async function jsonResponse(fetcher, url, options, stage) {
  let response;
  try {
    response = await fetcher(url, {
      ...options, redirect: "error", signal: AbortSignal.timeout(25_000),
    });
  } catch { throw new HealthFailure(stage); }
  if (!response.ok) throw new HealthFailure(stage, response.status);
  if (Number(response.headers.get("content-length")) > LIMIT)
    throw new HealthFailure(stage);
  try {
    const reader = response.body.getReader();
    let size = 0;
    const chunks = [];
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > LIMIT) { await reader.cancel(); throw new Error(); }
        chunks.push(Buffer.from(value));
      }
    } finally { reader.releaseLock(); }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch { throw new HealthFailure(stage); }
}
export async function checkAvailability({ config, fetcher = fetch, now = new Date() }) {
  const report = {
    checked_at: now.toISOString(), status: "failed", stage: "configuration",
    public_client_key_expires_at: clientKeyExpiry(config?.publishableKey),
    checks: { anonymous_auth: false, claude_model: false, session_signout: false },
    models: Object.fromEntries(EXPECTED_MODELS.map(id => [id, false])),
    scope: "Authentication and model health; no generation, search or desktop observation.",
  };
  let session, base, headers;
  try {
    const url = new URL(config.url);
    if (url.protocol !== "https:" || !url.hostname.endsWith(".supabase.co") ||
        url.username || url.password || url.pathname !== "/" || url.search || url.hash ||
        typeof config.publishableKey !== "string" || !config.publishableKey)
      throw new HealthFailure("configuration");
    if (report.public_client_key_expires_at && new Date(report.public_client_key_expires_at) <= now)
      throw new HealthFailure("public_client_key_expired");
    base = url.origin;
    session = await jsonResponse(fetcher, base + "/auth/v1/signup", {
      method: "POST", headers: { apikey: config.publishableKey, "Content-Type": "application/json" },
      body: JSON.stringify({ data: { purpose: "sumi_availability_check" } }),
    }, "anonymous_auth");
    if (typeof session.access_token !== "string" || !session.access_token || !session.user?.is_anonymous)
      throw new HealthFailure("anonymous_auth");
    report.checks.anonymous_auth = true;
    headers = { apikey: config.publishableKey, Authorization: "Bearer " + session.access_token };
    // Each existing gateway health operation updates database-backed usage counters.
    // Its upstream provider credentials never return to this process. One model
    // object is read per expected model, so catalog pagination cannot hide it.
    for (const id of EXPECTED_MODELS) {
      const model = await jsonResponse(fetcher, base + "/functions/v1/buddy-ai/anthropic/v1/models/" + id, {
        method: "GET", headers,
      }, "claude_model");
      report.models[id] = model?.id === id;
    }
    if (!Object.values(report.models).every(Boolean)) throw new HealthFailure("claude_models_missing");
    report.checks.claude_model = true;
    report.status = "passed";
    report.stage = "complete";
  } catch (error) {
    // Never include exception messages, URLs, response bodies, tokens or account data.
    report.stage = error instanceof HealthFailure ? error.stage : "configuration";
    if (error instanceof HealthFailure && error.status) report.http_status = error.status;
  } finally {
    if (session?.access_token && base && headers) {
      try {
        const response = await fetcher(base + "/auth/v1/logout", {
          method: "POST", headers, redirect: "error", signal: AbortSignal.timeout(15_000),
        });
        report.checks.session_signout = response.ok;
      } catch { /* The short-lived anonymous session is never persisted. */ }
      if (!report.checks.session_signout && report.status === "passed") {
        report.status = "failed"; report.stage = "session_signout";
      }
    }
  }
  return report;
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  let report;
  try {
    const config = JSON.parse(readFileSync(new URL("./public.json", import.meta.url), "utf8"));
    report = await checkAvailability({ config });
  } catch { report = { status: "failed", stage: "configuration" }; }
  const text = JSON.stringify(report, null, 2) + "\n";
  const output = process.argv.indexOf("--report");
  if (output >= 0 && process.argv[output + 1]) writeFileSync(process.argv[output + 1], text);
  process.stdout.write(text);
  if (report.status !== "passed") process.exitCode = 1;
}
