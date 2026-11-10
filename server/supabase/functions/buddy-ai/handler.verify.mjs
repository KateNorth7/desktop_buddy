import { test } from "node:test";
import assert from "node:assert/strict";
import { actualMicrousd, createGateway, validateMessages } from "./handler.ts";

const jwt = "aaa.bbb.ccc";
const user = "11111111-1111-4111-8111-111111111111";
const chat = () => ({
  model: "claude-haiku-5-5",
  max_tokens: 1024,
  system: [
    {
      type: "text",
      text: "Synthetic instructions",
      cache_control: { type: "ephemeral" },
    },
  ],
  messages: [{ role: "user", content: "Summarize the fictional checklist" }],
  output_config: {
    effort: "low",
    format: {
      type: "json_schema",
      schema: { type: "object", additionalProperties: false },
    },
  },
  fallbacks: "default",
});
const search = () => ({
  model: "claude-haiku-5-5",
  max_tokens: 2048,
  messages: [{ role: "user", content: "official application workflow" }],
  tools: [
    {
      type: "web_search_20260209",
      name: "web_search",
      max_uses: 2,
      allowed_callers: ["direct"],
    },
  ],
  output_config: { effort: "low" },
  fallbacks: "default",
});
const usage = {
  input_tokens: 1000,
  cache_creation_input_tokens: 200,
  cache_read_input_tokens: 500,
  output_tokens: 300,
};
function fixture({
  authorized = true,
  reserved = true,
  providerStatus = 200,
  settleStatus = 200,
  recordStatus = 200,
  recorded = { blocked: false, alert: false },
  telegram = false,
} = {}) {
  const calls = [];
  const env = {
    SUPABASE_URL: "https://project.supabase.co",
    SUPABASE_SERVICE_ROLE_KEY: "server-role-fixture",
    SUPABASE_ANON_KEY: "public-anon-fixture",
    ANTHROPIC_API_KEY: "private-anthropic-fixture",
    ...(telegram
      ? {
          TELEGRAM_BOT_TOKEN: "123456:fixture-token",
          TELEGRAM_ALERT_CHAT_ID: "-100200",
        }
      : {}),
  };
  const fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith("/auth/v1/user"))
      return Response.json(
        { id: user, role: "authenticated" },
        { status: authorized ? 200 : 401 },
      );
    if (url.endsWith("/rpc/buddy_reserve_ai")) return Response.json(reserved);
    if (url.endsWith("/rpc/buddy_settle_ai"))
      return Response.json(true, { status: settleStatus });
    if (url.endsWith("/rpc/buddy_record_ai_usage"))
      return Response.json(recorded, { status: recordStatus });
    if (url.startsWith("https://api.telegram.org/"))
      return Response.json({ ok: true });
    if (url.includes("/v1/models/"))
      return Response.json({
        id: "claude-haiku-5-5",
        display_name: "Claude Opus 5.5",
        private: "not returned",
      });
    return Response.json(
      {
        id: "msg_fixture",
        type: "message",
        role: "assistant",
        model: "claude-haiku-5-5",
        content: [{ type: "text", text: "Synthetic answer" }],
        stop_reason: "end_turn",
        usage,
      },
      { status: providerStatus },
    );
  };
  return { calls, handler: createGateway({ env: (name) => env[name], fetch }) };
}
function request(path = "/anthropic/v1/messages", body = chat(), token = jwt) {
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
test("a verified user is billed atomically before a fixed upstream request and settled afterwards; provider keys stay server-side", async () => {
  const { handler, calls } = fixture();
  const response = await handler(request());
  assert.equal(response.status, 200);
  assert.equal(calls.length, 5);
  assert.equal(
    calls[1].url,
    "https://project.supabase.co/rest/v1/rpc/buddy_reserve_ai",
  );
  const reservation = JSON.parse(calls[1].options.body);
  assert.equal(reservation.p_user, user);
  assert.equal(reservation.p_route, "ai");
  assert.ok(reservation.p_units > 0 && reservation.p_microusd > 0);
  assert.equal(calls[2].url, "https://api.anthropic.com/v1/messages");
  assert.equal(
    calls[2].options.headers["x-api-key"],
    "private-anthropic-fixture",
  );
  assert.equal(calls[2].options.headers["anthropic-version"], "2026-10-21");
  assert.equal(calls[2].options.headers.Authorization, undefined);
  assert.equal(
    calls[3].url,
    "https://project.supabase.co/rest/v1/rpc/buddy_settle_ai",
  );
  const settlement = JSON.parse(calls[3].options.body);
  assert.equal(settlement.p_user, user);
  assert.equal(
    settlement.p_delta,
    actualMicrousd("claude-haiku-5-5", usage) - reservation.p_microusd,
  );
  const raw = await response.text();
  assert.ok(!raw.includes("private-anthropic-fixture"));
  assert.ok(raw.includes("Synthetic answer"));
  assert.equal(response.headers.get("cache-control"), "no-store");
});
test("costs are estimated from the request and settled from reported usage in micro-dollars", () => {
  // 1000 input × $0.10/M + 200 cache writes × $0.125/M + 500 cache reads × $0.01/M + 300 output × $0.50/M
  assert.equal(actualMicrousd("claude-haiku-5-5", usage), 100 + 25 + 5 + 150);
  assert.equal(
    actualMicrousd("claude-haiku-5-5", {
      ...usage,
      server_tool_use: { web_search_requests: 2 },
    }),
    280 + 20000,
  );
  assert.equal(actualMicrousd("claude-haiku-5-5", undefined), 0);
  const plain = validateMessages(chat());
  assert.equal(plain.route, "ai");
  assert.ok(plain.microusd >= 1024 * 0.5 && plain.microusd < 100000);
  const searching = validateMessages(search());
  assert.equal(searching.route, "search");
  assert.ok(searching.microusd >= plain.microusd + 20000);
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
test("web search runs through the same route with a bounded tool and the search quota", async () => {
  const f = fixture();
  assert.equal((await f.handler(request(undefined, search()))).status, 200);
  assert.equal(JSON.parse(f.calls[1].options.body).p_route, "search");
  for (const tools of [
    [{ type: "web_search_20260209", name: "web_search", max_uses: 10 }],
    [{ type: "web_fetch_20260209", name: "web_fetch" }],
    [
      { type: "web_search_20260209", name: "web_search", max_uses: 1 },
      { type: "web_search_20260209", name: "web_search", max_uses: 1 },
    ],
    [
      {
        type: "web_search_20260209",
        name: "web_search",
        max_uses: 1,
        allowed_domains: ["example.com"],
      },
    ],
    [{ name: "custom", description: "x", input_schema: { type: "object" } }],
  ]) {
    const bad = fixture();
    assert.equal(
      (await bad.handler(request(undefined, { ...search(), tools }))).status,
      400,
    );
    assert.equal(bad.calls.length, 1);
  }
});
test("arbitrary URLs, unlisted models and expensive unsupported operations are rejected", async () => {
  for (const body of [
    { ...chat(), model: "arbitrary-model" },
    { ...chat(), max_tokens: 999999 },
    { ...chat(), stream: true },
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
  for (const path of [
    "/https://attacker.invalid",
    "/tavily/search",
    "/anthropic/v1/models/not-allowed",
  ]) {
    assert.equal((await f.handler(request(path))).status, 404);
  }
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
              source: {
                type: "base64",
                media_type: "image/jpeg",
                data: "YQ==",
              },
            },
          ],
        },
      ],
    }).units > 0,
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
test("health checks reach the provider once, filter private fields and are not settled", async () => {
  const f = fixture();
  const response = await f.handler(
    new Request(
      "https://project.supabase.co/functions/v1/buddy-ai/anthropic/v1/models/claude-haiku-5-5",
      { headers: { Authorization: "Bearer " + jwt } },
    ),
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { id: "claude-haiku-5-5" });
  assert.equal(f.calls.length, 3);
  assert.equal(
    f.calls[2].url,
    "https://api.anthropic.com/v1/models/claude-haiku-5-5",
  );
  assert.equal(JSON.parse(f.calls[1].options.body).p_microusd, 0);
});
test("provider failures are sanitized, never replayed and keep the reserved estimate", async () => {
  const f = fixture({ providerStatus: 503 });
  const response = await f.handler(request());
  assert.equal(response.status, 503);
  assert.equal(f.calls.length, 3);
  assert.ok(!(await response.text()).includes("private"));
});
test("a failed settlement does not fail the delivered response", async () => {
  const f = fixture({ settleStatus: 500 });
  const response = await f.handler(request());
  assert.equal(response.status, 200);
  // Settlement, then the per-user usage record, both after the provider answered.
  assert.equal(f.calls.length, 5);
});

test("each response is recorded per user with tokens and cost, and content never leaves", async () => {
  const { handler, calls } = fixture();
  const response = await handler(request());
  assert.equal(response.status, 200);
  const record = calls.find((c) =>
    c.url.endsWith("/rpc/buddy_record_ai_usage"),
  );
  const body = JSON.parse(record.options.body);
  assert.deepEqual(
    {
      user: body.p_user,
      route: body.p_route,
      model: body.p_model,
      input: body.p_input,
      output: body.p_output,
      write: body.p_cache_write,
      read: body.p_cache_read,
      searches: body.p_searches,
      cost: body.p_microusd,
    },
    {
      user,
      route: "ai",
      model: "claude-haiku-5-5",
      input: 1000,
      output: 300,
      write: 200,
      read: 500,
      searches: 0,
      cost: actualMicrousd("claude-haiku-5-5", usage),
    },
  );
  assert.ok(!record.options.body.includes("fictional checklist"));
  assert.ok(!calls.some((c) => c.url.startsWith("https://api.telegram.org/")));
});
test("a newly blocked user alerts the developers once, with counters and no content", async () => {
  const blocked = {
    blocked: true,
    alert: true,
    reason: "spend_spike",
    today_microusd: 450000,
    average_microusd: 20000,
    calls_10min: 12,
  };
  const { handler, calls } = fixture({ recorded: blocked, telegram: true });
  const response = await handler(request());
  assert.equal(response.status, 200, "the delivered answer is not withdrawn");
  const alerts = calls.filter((c) =>
    c.url.startsWith("https://api.telegram.org/"),
  );
  assert.equal(alerts.length, 1);
  assert.equal(
    alerts[0].url,
    "https://api.telegram.org/bot123456:fixture-token/sendMessage",
  );
  const message = JSON.parse(alerts[0].options.body);
  assert.equal(message.chat_id, "-100200");
  assert.ok(
    message.text.includes(user) && message.text.includes("spend_spike"),
  );
  assert.ok(
    message.text.includes("USD 0.45") && message.text.includes("USD 0.02"),
  );
  assert.ok(!message.text.includes("fictional checklist"));
  // Without a configured bot nothing is sent, and an already blocked user is not re-alerted.
  const quiet = fixture({ recorded: blocked });
  await quiet.handler(request());
  assert.ok(
    !quiet.calls.some((c) => c.url.startsWith("https://api.telegram.org/")),
  );
  const again = fixture({
    recorded: { ...blocked, alert: false },
    telegram: true,
  });
  await again.handler(request());
  assert.ok(
    !again.calls.some((c) => c.url.startsWith("https://api.telegram.org/")),
  );
});
test("a failed usage record does not fail the delivered response", async () => {
  const { handler } = fixture({ recordStatus: 500 });
  const response = await handler(request());
  assert.equal(response.status, 200);
  assert.ok((await response.text()).includes("Synthetic answer"));
});
