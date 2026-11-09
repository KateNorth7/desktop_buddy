import { test } from "node:test";
import assert from "node:assert/strict";
import { checkAvailability, clientKeyExpiry, EXPECTED_MODELS } from "./availability.mjs";
const config = { url: "https://example.supabase.co", publishableKey: "public-client-key" };
const secret = "PRIVATE_RESPONSE_MUST_NOT_BE_LOGGED";
function mock({ fail, missing = false, payload } = {}) {
  const calls = [];
  return { calls, fetcher: async (url, options) => {
    const path = new URL(url).pathname; calls.push({ path, options });
    if (path === fail) return Response.json({ secret }, { status: 401 });
    if (path.endsWith("/signup")) return Response.json({ access_token: secret, refresh_token: secret, user: { is_anonymous: true } });
    if (path.includes("/v1/models/")) return Response.json(payload ?? { id: missing ? "other-model" : path.split("/").pop(), display_name: "Fixture" });
    if (path.endsWith("/logout")) return new Response(null, { status: 204 });
    throw new Error(secret);
  } };
}
test("uses only anonymous auth, catalog and usage; never generates, searches or logs credentials", async () => {
  const m = mock(); const report = await checkAvailability({ config, fetcher: m.fetcher });
  assert.equal(report.status, "passed");
  assert.deepEqual(m.calls.map(c => c.path), ["/auth/v1/signup", "/functions/v1/buddy-ai/anthropic/v1/models/claude-opus-5-5", "/auth/v1/logout"]);
  assert.ok(!JSON.stringify(report).includes(secret));
  assert.ok(Object.values(report.checks).every(Boolean));
  assert.ok(m.calls.every(c => c.options.redirect === "error"));
});
test("invalid or expired public configuration performs no network request", async () => {
  let calls = 0;
  for (const url of ["http://example.supabase.co", "https://other.test", "https://u:p@example.supabase.co", "https://example.supabase.co/?x=y"]) {
    const report = await checkAvailability({ config: { ...config, url }, fetcher: async () => { calls++; } });
    assert.equal(report.stage, "configuration");
  }
  const key = "x." + Buffer.from(JSON.stringify({ exp: 1 })).toString("base64url") + ".x";
  const report = await checkAvailability({ config: { ...config, publishableKey: key }, fetcher: async () => { calls++; } });
  assert.equal(report.stage, "public_client_key_expired"); assert.equal(calls, 0);
  assert.equal(clientKeyExpiry(key), "1970-01-01T00:00:01.000Z");
});
test("provider failure reports a sanitized stage and still signs out", async () => {
  const m = mock({ fail: "/functions/v1/buddy-ai/anthropic/v1/models/claude-opus-5-5" });
  const report = await checkAvailability({ config, fetcher: m.fetcher });
  assert.equal(report.status, "failed"); assert.equal(report.stage, "claude_model");
  assert.equal(report.http_status, 401); assert.equal(report.checks.session_signout, true);
  assert.ok(!JSON.stringify(report).includes(secret));
});
test("an available model is required and malformed provider JSON cannot pass", async () => {
  for (const options of [{ missing: true }, { payload: { text: secret } }]) {
    const m = mock(options); const report = await checkAvailability({ config, fetcher: m.fetcher });
    assert.equal(report.status, "failed"); assert.equal(report.checks.session_signout, true);
    assert.ok(!JSON.stringify(report).includes(secret));
  }
});
test("authentication failure has no provider calls or raw error output", async () => {
  const m = mock({ fail: "/auth/v1/signup" }); const report = await checkAvailability({ config, fetcher: m.fetcher });
  assert.equal(report.stage, "anonymous_auth"); assert.equal(m.calls.length, 1);
  assert.ok(!JSON.stringify(report).includes(secret));
});
test("network exceptions and logout failures cannot be mistaken for success", async () => {
  const report = await checkAvailability({ config, fetcher: async () => { throw new Error(secret); } });
  assert.equal(report.status, "failed"); assert.ok(!JSON.stringify(report).includes(secret));
  const m = mock({ fail: "/auth/v1/logout" }); const logout = await checkAvailability({ config, fetcher: m.fetcher });
  assert.equal(logout.stage, "session_signout"); assert.equal(logout.status, "failed");
});

test("oversized health responses are rejected without logging their contents", async () => {
  const m = mock();
  const report = await checkAvailability({ config, fetcher: async (url, options) => {
    if (url.includes("/v1/models/")) return new Response(secret.repeat(80_000), { status: 200 });
    return m.fetcher(url, options);
  } });
  assert.equal(report.status, "failed"); assert.equal(report.stage, "claude_model");
  assert.equal(report.checks.session_signout, true); assert.ok(!JSON.stringify(report).includes(secret));
});
