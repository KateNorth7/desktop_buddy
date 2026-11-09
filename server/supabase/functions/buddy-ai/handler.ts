// Runtime secrets never leave this function. No content or credentials are logged.
type Environment = (name: string) => string | undefined;
type Dependencies = { env: Environment; fetch: typeof fetch };
// Built-in AI routes one Claude model; personal connections keep their own controls.
const MODELS = new Set(["claude-haiku-5-5"]);
const ANTHROPIC_VERSION = "2026-10-21";
// Server-side fallback re-runs a safety refusal on another Claude model in the same call.
const ANTHROPIC_BETA = "server-side-fallback-2026-07-01";
const BASE = "/buddy-ai";
// Match the desktop's existing four-image / 12 MB decoded attachment budget.
const MAX_BODY = 17000000;
const MAX_RESPONSE = 2000000;
const EFFORTS = new Set(["low", "medium", "high", "xhigh", "max"]);
// Claude list prices in micro-dollars per token ($ per million tokens), plus
// $10 per 1,000 web searches. Update together with MODELS.
const PRICE: Record<
  string,
  { input: number; output: number; cacheWrite: number; cacheRead: number }
> = {
  "claude-haiku-5-5": { input: 0.1, output: 0.5, cacheWrite: 0.125, cacheRead: 0.01 },
};
const SEARCH_MICROUSD = 10000;
// Reserved before a request: a conservative estimate settled to actual usage after it.
const RESERVED_OUTPUT_TOKENS = 1536;
const reply = (status: number, message: string) =>
  Response.json(
    { error: message },
    {
      status,
      headers: { "Cache-Control": "no-store" },
    },
  );
async function bounded(
  response: Response | Request,
  limit: number,
): Promise<string> {
  if (Number(response.headers.get("content-length")) > limit)
    throw Error("size");
  const reader = response.body?.getReader();
  if (!reader) return "";
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) {
        await reader.cancel();
        throw Error("size");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const joined = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.length;
  }
  return new TextDecoder("utf-8", { fatal: true }).decode(joined);
}
const BASE64 = /^[A-Za-z0-9+/=]+$/;
function isTextBlock(part: unknown): part is { type: "text"; text: string } {
  return (
    !!part &&
    typeof part === "object" &&
    (part as { type?: unknown }).type === "text" &&
    typeof (part as { text?: unknown }).text === "string" &&
    Object.keys(part).every((k) =>
      ["type", "text", "cache_control"].includes(k),
    )
  );
}
export type Validated = {
  /** Conservative reservation units, not a currency price. */
  units: number;
  /** Estimated cost in micro-dollars, reserved before the call and settled after it. */
  microusd: number;
  /** Quota route: `search` when the request runs Claude's web search tool. */
  route: "ai" | "search";
};
/** Validates a Claude Messages request and derives its reservation. */
export function validateMessages(body: Record<string, unknown>): Validated {
  const fields = new Set([
    "model",
    "max_tokens",
    "messages",
    "system",
    "output_config",
    "fallbacks",
    "tools",
  ]);
  if (
    Object.keys(body).some((k) => !fields.has(k)) ||
    !MODELS.has(String(body.model)) ||
    !Array.isArray(body.messages) ||
    !body.messages.length ||
    body.messages.length > 128 ||
    !Number.isInteger(body.max_tokens) ||
    Number(body.max_tokens) < 1 ||
    Number(body.max_tokens) > 8192 ||
    (body.fallbacks !== undefined && body.fallbacks !== "default")
  )
    throw Error("payload");
  if (body.output_config !== undefined) {
    const config = body.output_config as Record<string, unknown>;
    if (
      !config ||
      typeof config !== "object" ||
      Array.isArray(config) ||
      Object.keys(config).some((k) => !["effort", "format"].includes(k)) ||
      (config.effort !== undefined && !EFFORTS.has(String(config.effort)))
    )
      throw Error("output_config");
    if (config.format !== undefined) {
      const format = config.format as Record<string, unknown>;
      if (
        !format ||
        typeof format !== "object" ||
        format.type !== "json_schema" ||
        !format.schema ||
        typeof format.schema !== "object" ||
        Array.isArray(format.schema) ||
        Object.keys(format).some((k) => !["type", "schema"].includes(k))
      )
        throw Error("format");
    }
  }
  let searches = 0;
  if (body.tools !== undefined) {
    // The only server tool allowed is one bounded web search; every search is billed.
    if (!Array.isArray(body.tools) || body.tools.length !== 1) throw Error("tools");
    const tool = body.tools[0] as Record<string, unknown>;
    if (
      !tool ||
      typeof tool !== "object" ||
      tool.type !== "web_search_20260209" ||
      tool.name !== "web_search" ||
      !Number.isInteger(tool.max_uses) ||
      Number(tool.max_uses) < 1 ||
      Number(tool.max_uses) > 3 ||
      (tool.allowed_callers !== undefined &&
        JSON.stringify(tool.allowed_callers) !== '["direct"]') ||
      Object.keys(tool).some(
        (k) => !["type", "name", "max_uses", "allowed_callers"].includes(k),
      )
    )
      throw Error("tools");
    searches = Number(tool.max_uses);
  }
  let images = 0,
    imageBytes = 0,
    textBytes = 0;
  const encoder = new TextEncoder();
  if (typeof body.system === "string")
    textBytes += encoder.encode(body.system).length;
  else if (Array.isArray(body.system)) {
    for (const part of body.system) {
      if (!isTextBlock(part)) throw Error("system");
      textBytes += encoder.encode(part.text).length;
    }
  } else if (body.system !== undefined) throw Error("system");
  for (const message of body.messages) {
    if (
      !message ||
      !["user", "assistant"].includes(message.role) ||
      Object.keys(message).some((k) => !["role", "content"].includes(k))
    )
      throw Error("message");
    if (typeof message.content === "string")
      textBytes += encoder.encode(message.content).length;
    else if (Array.isArray(message.content)) {
      for (const part of message.content) {
        if (isTextBlock(part)) textBytes += encoder.encode(part.text).length;
        else if (
          part?.type === "image" &&
          Object.keys(part).every((k) => ["type", "source"].includes(k)) &&
          part.source?.type === "base64" &&
          ["image/jpeg", "image/png", "image/webp"].includes(
            part.source.media_type,
          ) &&
          typeof part.source.data === "string" &&
          part.source.data.length <= 6000050 &&
          BASE64.test(part.source.data) &&
          Object.keys(part.source).every((k) =>
            ["type", "media_type", "data"].includes(k),
          )
        ) {
          images++;
          imageBytes += part.source.data.length;
        } else throw Error("content");
      }
    } else throw Error("content");
  }
  if (images > 4 || imageBytes > 16000200 || textBytes > 160000)
    throw Error("content budget");
  const price = PRICE[String(body.model)];
  const inputTokens = Math.ceil(textBytes / 3) + images * 1600 + 400;
  const outputTokens = Math.min(Number(body.max_tokens), RESERVED_OUTPUT_TOKENS);
  return {
    // Conservative reservation units, not a quoted currency price. Timeouts stay charged.
    units: textBytes + Number(body.max_tokens) * 8 + images * 16384,
    microusd: Math.ceil(
      inputTokens * price.input +
        outputTokens * price.output +
        searches * SEARCH_MICROUSD,
    ),
    route: searches ? "search" : "ai",
  };
}
/** Cost of one completed response from the provider's reported usage, in micro-dollars. */
export function actualMicrousd(
  model: string,
  usage: Record<string, unknown> | undefined,
): number {
  const price = PRICE[model] ?? PRICE["claude-haiku-5-5"];
  const count = (value: unknown) =>
    Number.isFinite(Number(value)) && Number(value) > 0 ? Number(value) : 0;
  const tools = (usage?.server_tool_use ?? {}) as Record<string, unknown>;
  return Math.ceil(
    count(usage?.input_tokens) * price.input +
      count(usage?.cache_creation_input_tokens) * price.cacheWrite +
      count(usage?.cache_read_input_tokens) * price.cacheRead +
      count(usage?.output_tokens) * price.output +
      count(tools.web_search_requests) * SEARCH_MICROUSD,
  );
}
type Route = { method: string; url: string; kind: string };
function resolveRoute(path: string): Route | undefined {
  if (path === `${BASE}/anthropic/v1/messages`)
    return {
      method: "POST",
      url: "https://api.anthropic.com/v1/messages",
      kind: "ai",
    };
  // One model object per health check: a paginated catalog cannot hide the model.
  const model = path.match(
    /^\/buddy-ai\/anthropic\/v1\/models\/([A-Za-z0-9._-]{1,80})$/,
  )?.[1];
  if (model && MODELS.has(model))
    return {
      method: "GET",
      url: `https://api.anthropic.com/v1/models/${model}`,
      kind: "health",
    };
  return undefined;
}
export function createGateway(dependencies: Dependencies) {
  const { env, fetch: request } = dependencies;
  return async (req: Request): Promise<Response> => {
    // The hosted gateway strips /functions/v1 before invoking the runtime.
    const path = new URL(req.url).pathname.replace(
      /^\/functions\/v1(?=\/buddy-ai\/)/,
      "",
    );
    const route = resolveRoute(path);
    if (!route || req.method !== route.method || new URL(req.url).search)
      return reply(404, "Unknown service operation");
    const authorization = req.headers.get("authorization") ?? "";
    if (
      !/^Bearer [A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(
        authorization,
      ) ||
      authorization.length > 8192
    )
      return reply(401, "Authentication required");
    const supabase = env("SUPABASE_URL"),
      service = env("SUPABASE_SERVICE_ROLE_KEY"),
      upstream = env("ANTHROPIC_API_KEY");
    if (!supabase || !service || !upstream)
      return reply(503, "Built-in AI is not configured");
    const rpc = (name: string, body: Record<string, unknown>) =>
      request(`${supabase}/rest/v1/rpc/${name}`, {
        method: "POST",
        headers: {
          apikey: service,
          Authorization: `Bearer ${service}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(body),
        redirect: "error",
        signal: AbortSignal.timeout(10000),
      });
    let stage = "auth";
    try {
      const userResponse = await request(`${supabase}/auth/v1/user`, {
        headers: {
          Authorization: authorization,
          apikey: env("SUPABASE_ANON_KEY") ?? service,
        },
        redirect: "error",
        signal: AbortSignal.timeout(10000),
      });
      if (!userResponse.ok)
        return reply(401, "Session expired; reconnect built-in AI");
      const user = JSON.parse(await bounded(userResponse, 32000));
      if (
        !/^[a-f0-9-]{36}$/.test(user.id ?? "") ||
        user.role !== "authenticated"
      )
        return reply(401, "Invalid user session");
      let payload: Record<string, unknown> | undefined;
      let reservation: Validated = { units: 0, microusd: 0, route: "ai" };
      let quotaRoute: string = route.kind;
      if (req.method === "POST") {
        if (
          !(req.headers.get("content-type") ?? "").startsWith(
            "application/json",
          )
        )
          return reply(415, "JSON required");
        try {
          payload = JSON.parse(await bounded(req, MAX_BODY));
          if (!payload || Array.isArray(payload) || typeof payload !== "object")
            throw Error("payload");
          reservation = validateMessages(payload!);
          quotaRoute = reservation.route;
        } catch {
          return reply(
            400,
            "Request exceeds the supported operation or context budget",
          );
        }
      }
      stage = "quota";
      const reserved = await rpc("buddy_reserve_ai", {
        p_user: user.id,
        p_route: quotaRoute,
        p_units: reservation.units,
        p_microusd: reservation.microusd,
      });
      if (!reserved.ok)
        return reply(503, "Built-in AI is temporarily unavailable");
      if (JSON.parse(await bounded(reserved, 1000)) !== true)
        return reply(
          429,
          "Built-in AI allowance reached. Try later or use your own API keys.",
        );
      stage = "provider";
      const response = await request(route.url, {
        method: route.method,
        headers: {
          "x-api-key": upstream,
          "anthropic-version": ANTHROPIC_VERSION,
          "anthropic-beta": ANTHROPIC_BETA,
          "Content-Type": "application/json",
        },
        body: payload ? JSON.stringify(payload) : undefined,
        redirect: "error",
        signal: AbortSignal.any([req.signal, AbortSignal.timeout(60000)]),
      });
      if (!response.ok) {
        console.warn(
          JSON.stringify({
            event: "gateway_upstream_failure",
            service: quotaRoute,
            status: response.status,
          }),
        );
        return reply(
          [401, 403].includes(response.status)
            ? 502
            : response.status >= 500
              ? 503
              : response.status,
          "The selected AI service could not complete this request",
        );
      }
      stage = "response";
      const data = JSON.parse(await bounded(response, MAX_RESPONSE));
      if (route.method === "GET") {
        if (typeof data.id !== "string" || !MODELS.has(data.id))
          return reply(502, "Invalid service model response");
        return Response.json(
          { id: data.id },
          { headers: { "Cache-Control": "no-store" } },
        );
      }
      // Replace the estimate with the provider's reported usage so the daily
      // budget tracks real spend. A failed settlement keeps the estimate.
      stage = "settle";
      const actual = actualMicrousd(String(payload?.model), data?.usage);
      try {
        const settled = await rpc("buddy_settle_ai", {
          p_user: user.id,
          p_delta: actual - reservation.microusd,
        });
        if (!settled.ok)
          console.warn(JSON.stringify({ event: "gateway_settle_failure" }));
      } catch {
        console.warn(JSON.stringify({ event: "gateway_settle_failure" }));
      }
      return Response.json(data, { headers: { "Cache-Control": "no-store" } });
    } catch (error) {
      const kind =
        error instanceof Error &&
        ["AbortError", "TimeoutError"].includes(error.name)
          ? "timeout"
          : error instanceof SyntaxError
            ? "invalid_json"
            : "unavailable";
      console.warn(JSON.stringify({ event: "gateway_failure", stage, kind }));
      return reply(
        503,
        "Built-in AI could not reach the service; no automatic retry was sent",
      );
    }
  };
}
