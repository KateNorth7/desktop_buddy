# sumi 0.51.0

sumi's AI now runs on Claude.

- Every AI operation is served by Anthropic's Claude Messages API with `claude-opus-5-5`: goal matching, focus checks, dialogue planning, action authorization and review, research synthesis, goal-aware screen understanding, image descriptions and the daily review. Nebius Token Factory, NVIDIA Nemotron and Kimi K3 are no longer used.
- Requests leave the app in Claude's format. Instructions travel as a cached system prompt, strict JSON answers use Claude structured outputs, bounded background operations run at low reasoning effort and deliberate planning at high effort, and a safety refusal is retried server-side on another Claude model instead of surfacing as an empty answer. The connection check reads the configured model object directly, so a long model catalog cannot hide it.
- Settings → AI → Services shows Claude in place of Nebius. In **Use my API keys**, save an Anthropic API key in the Claude row. `ANTHROPIC_MODEL_ID`, `ANTHROPIC_RESPONSE_MODEL_ID` and `ANTHROPIC_VISION_MODEL_ID` override the model per role; `ANTHROPIC_API_URL` overrides the endpoint. A Nebius key saved earlier in this row is no longer valid and must be replaced.
- Built-in AI needs the gateway redeployed with an `ANTHROPIC_API_KEY` secret, as described in `docs/SUPABASE_GATEWAY.md`. Until that deployment the built-in connection reports its AI service as unavailable and **Use my API keys** is the working path. The gateway's model allowlist contains `claude-opus-5-5` only.
- Claude is billed per token at a higher rate than the previous models; the hourly AI budget, screenshot thresholds and other cost bounds from 0.50.0 are unchanged.

Tracking consent, exclusions, lock protection, encrypted evidence retention and the trusted signing keys are unchanged.
