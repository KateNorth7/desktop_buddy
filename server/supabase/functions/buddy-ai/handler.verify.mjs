import { test } from "node:test";
import assert from "node:assert/strict";
import { createGateway, validateMessages } from "./handler.ts";

const jwt = "aaa.bbb.ccc";
const user = "11111111-1111-4111-8111-111111111111";
const chat = () => ({
  model: "claude-opus-5-5",
  max_tokens: 1024,
  system: [{ type: "text", text: "Synthetic instructions", cache_control: { type: "ephemeral" } }],
  messages: [{ role: "user", content: "Summarize the fictional checklist" }],
  output_config: {
    effort: "low",
    format: { type: "json_schema", schema: { type: "object", additionalProperties: false } },
  },
  fallbacks: "default",
});
function fixture({
  authorized = true,
  reserved = true,
  providerStatus = 200,
} = {}) {
  const calls = [];
  const env = {
    SUPABASE_URL: "https://project.supabase.co",
    SUPABASE_SERVICE_ROLE_KEY: "server-role-fixture",
    SUPABASE_ANON_KEY: "public-anon-fixture",
    ANTHROPIC_API_KEY: "private-anthropic-fixture",
    TAVILY_API_KEY: "private-tavily-fixture",
  };
  const fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith("/auth/v1/user"))
      return Response.json(
        { id: user, role: "authenticated" },
        { status: authorized ? 200 : 401 },
      );
    if (url.includes("/rpc/")) return Response.json(reserved);
    if (url.includes("/v1/models/"))
      return Response.json({
        id: "claude-opus-5-5",
        display_name: "Claude Opus 5.5",
        private: "not returned",
      });
    if (url.endsWith("/usage"))
      return Response.json({
        key: { api_key: "must-never-leave-server", usage: 999 },
      });
    return Response.json(
      {
        id: "msg_fixture",
        type: "message",
        role: "assistant",
        model: "claude-opus-5-5",
        content: [{ type: "text", text: "Synthetic answer" }],
        stop_reason: "end_turn",
      },
      { status: providerStatus },
    );
  };
  return { calls, handler: createGateway({ env: (name) => env[name], fetch }) };
}
function request(
  path = "/anthropic/v1/messages",
  body = chat(),
  token = jwt,
) {
  return new Request(
    "https://project.supabase.co/functions/v1/buddy-ai" + path,
    {
      method: "POST",
      headers: {
        Authorization: "Bearer " + token,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    },
  );
}
test("a verified user is billed atomically before a fixed upstream request; provider keys stay server-side", async () => {
  const { handler, calls } = fixture();
  const response = await handler(request());
  assert.equal(response.status, 200);
  assert.equal(calls.length, 3);
  assert.equal(
    calls[1].url,
    "https://project.supabase.co/rest/v1/rpc/buddy_reserve_ai",
  );
  assert.equal(JSON.parse(calls[1].options.body).p_user, user);
  assert.equal(calls[2].url, "https://api.anthropic.com/v1/messages");
  assert.equal(
    calls[2].options.headers["x-api-key"],
    "private-anthropic-fixture",
  );
  assert.equal(calls[2].options.headers["anthropic-version"], "2026-10-21");
  assert.equal(calls[2].options.headers.Authorization, undefined);
  const raw = await response.text();
  assert.ok(!raw.includes("private-anthropic-fixture"));
  assert.ok(raw.includes("Synthetic answer"));
  assert.equal(response.headers.get("cache-control"), "no-store");
});
test("missing or invalid auth never reaches quota or provider", async () => {
  for (const token of ["", "public-anon-fixture"]) {
    const f = fixture();
    assert.equal(
      (await f.handler(request(undefined, chat(), token))).status,
      401,
    );
    assert.equal(f.calls.length, 0);
  }
  const f = fixture({ authorized: false });
  assert.equal((await f.handler(request())).status, 401);
  assert.equal(f.calls.length, 1);
});
test("exhausted budgets fail closed without upstream calls", async () => {
  const f = fixture({ reserved: false });
  assert.equal((await f.handler(request())).status, 429);
  assert.equal(f.calls.length, 2);
});
test("arbitrary URLs, unlisted models and expensive unsupported operations are rejected", async () => {
  for (const body of [
    { ...chat(), model: "arbitrary-model" },
    { ...chat(), max_tokens: 999999 },
    { ...chat(), stream: true },
    { ...chat(), tools: [] },
    { ...chat(), fallbacks: [{ model: "arbitrary-model" }] },
    { ...chat(), output_config: { effort: "ultra" } },
    { ...chat(), url: "https://attacker.invalid" },
    { ...chat(), messages: [{ role: "system", content: "x" }] },
    { ...chat(), messages: [{ role: "tool", content: "x" }] },
  ]) {
    const f = fixture();
    assert.equal((await f.handler(request(undefined, body))).status, 400);
    assert.equal(f.calls.length, 1);
  }
  const f = fixture();
  assert.equal(
    (await f.handler(request("/https://attacker.invalid"))).status,
    404,
  );
  assert.equal(f.calls.length, 0);
  assert.equal(
    (
      await f.handler(
        new Request(
          "https://project.supabase.co/functions/v1/buddy-ai/anthropic/v1/models/not-allowed",
          { headers: { Authorization: "Bearer " + jwt } },
        ),
      )
    ).status,
    404,
  );
  assert.equal(f.calls.length, 0);
});
test("image input permits bounded base64 images, never provider-side remote image fetches", () => {
  assert.throws(() =>
    validateMessages({
      ...chat(),
      messages: [
        {
          role: "user",
          content: [
            {
              type: "image",
              source: { type: "url", url: "https://internal.invalid/private" },
            },
          ],
        },
      ],
    }),
  );
  assert.ok(
    validateMessages({
      ...chat(),
      messages: [
        {
          role: "user",
          content: [
            { type: "text", text: "What is visible?" },
            {
              type: "image",
              source: { type: "base64", media_type: "image/jpeg", data: "YQ==" },
            },
          ],
        },
      ],
    }) > 0,
  );
  assert.throws(() =>
    validateMessages({
      ...chat(),
      messages: [{ role: "user", content: "x".repeat(160001) }],
    }),
  );
  assert.throws(() =>
    validateMessages({ ...chat(), system: [{ type: "image", text: "x" }] }),
  );
});
test("health checks actually reach the selected provider but filter account/private fields", async () => {
  for (const [path, expected] of [
    ["/anthropic/v1/models/claude-opus-5-5", { id: "claude-opus-5-5" }],
    ["/tavily/usage", { key: { configured: true } }],
  ]) {
    const f = fixture();
    const response = await f.handler(
      new Request("https://project.supabase.co/functions/v1/buddy-ai" + path, {
        headers: { Authorization: "Bearer " + jwt },
      }),
    );
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), expected);
    assert.equal(f.calls.length, 3);
  }
  const f = fixture();
  await f.handler(
    new Request(
      "https://project.supabase.co/functions/v1/buddy-ai/anthropic/v1/models/claude-opus-5-5",
      { headers: { Authorization: "Bearer " + jwt } },
    ),
  );
  assert.equal(f.calls[2].url, "https://api.anthropic.com/v1/models/claude-opus-5-5");
});
test("provider failures are sanitized and are never automatically replayed", async () => {
  const f = fixture({ providerStatus: 503 });
  const response = await f.handler(request());
  assert.equal(response.status, 503);
  assert.equal(f.calls.length, 3);
  assert.ok(!(await response.text()).includes("private"));
});
test("Tavily uses the fixed search endpoint with bounded supported search options", async () => {
  const f = fixture();
  assert.equal(
    (
      await f.handler(
        request("/tavily/search", {
          query: "public docs",
          search_depth: "advanced",
          max_results: 5,
          topic: "general",
          include_raw_content: false,
        }),
      )
    ).status,
    200,
  );
  assert.equal(f.calls[2].url, "https://api.tavily.com/search");
  assert.equal(
    f.calls[2].options.headers.Authorization,
    "Bearer private-tavily-fixture",
  );
  const bad = fixture();
  assert.equal(
    (
      await bad.handler(
        request("/tavily/search", {
          query: "public docs",
          search_depth: "advanced",
          max_results: 100,
        }),
      )
    ).status,
    400,
  );
});
