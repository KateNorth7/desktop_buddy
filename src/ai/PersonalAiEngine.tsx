import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { desktop } from "../api/tauri";
import {
  engineApi,
  type InferenceHealth,
  type PersonalAiEngine,
} from "./engineApi";
import "./engine.css";
import { queryClient } from "../data/client";
import { useQuery } from "../data/useQuery";
const engineQuery = queryClient.query(
  "engine",
  ["engine"],
  null as PersonalAiEngine | null,
  () => engineApi.status(),
  5000,
);

const readable = (value: string) => value.replaceAll("_", " ");
const latency = (value: number | null | undefined) =>
  value == null ? "Not measured" : `${value} ms`;

export function PersonalAiEnginePanel({
  initiallyOpen = false,
  active = true,
}: {
  initiallyOpen?: boolean;
  active?: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const [showDataDiagnostics, setShowDataDiagnostics] = useState(false);
  const [live, setLive] = useState(true);
  const mounted = useRef(true);
  const engineState = useQuery(engineQuery, desktop && active && open && live);
  const data = engineState.data;
  const diagnostics = useSyncExternalStore(
    queryClient.subscribeDiagnostics,
    queryClient.getDiagnostics,
    queryClient.getDiagnostics,
  );
  const [health, setHealth] = useState<InferenceHealth | null>(null);
  const loading = engineState.loading;
  const [testing, setTesting] = useState(false);
  const [actionChecks, setActionChecks] = useState<
    Array<{ name: string; status: string; latency_ms: number }>
  >([]);
  const [checkingActions, setCheckingActions] = useState(false);
  const [error, setError] = useState("");
  async function refresh() {
    if (!desktop || !mounted.current) return;
    setError("");
    try {
      await engineQuery.refresh();
    } catch {
      /* Shared query exposes a safe error. */
    }
  }
  useEffect(() => {
    if (data) setHealth(data.health);
  }, [data]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function check() {
    setTesting(true);
    setError("");
    setHealth(null);
    try {
      setHealth(await engineApi.health());
    } catch {
      setError(
        "Inference check could not start. Wait 60 seconds and try again.",
      );
    } finally {
      setTesting(false);
    }
  }
  async function checkActions() {
    setCheckingActions(true);
    setError("");
    setActionChecks([]);
    try {
      setActionChecks(await engineApi.actions());
    } catch {
      setError(
        "Agent check could not finish. Enable AI, check the connection and wait 60 seconds before retrying. No user goals were changed.",
      );
    } finally {
      setCheckingActions(false);
    }
  }
  const reported =
    health?.provider_reported_model ??
    data?.last_inference?.provider_reported_model;
  return (
    <details
      className="inline-details personal-ai-engine"
      open={open}
      onToggle={(event) => {
        setOpen(event.currentTarget.open);
      }}
    >
      <summary>Personal AI Engine</summary>
      <p className="helper">
        Developer and demo diagnostics. Shows events, decisions and verified
        actions; no private reasoning, prompts, keys or screen text. Live
        refresh reads local state only.
      </p>
      <label className="engine-live-switch">
        <input
          type="checkbox"
          checked={live}
          onChange={(event) => setLive(event.target.checked)}
        />
        Live monitor · refresh every 5 seconds while open
      </label>
      <button
        type="button"
        className="secondary"
        disabled={!desktop || loading || testing}
        onClick={() => void refresh()}
      >
        {loading ? "Refreshing…" : "Refresh engine status"}
      </button>
      {!desktop && (
        <p className="helper">Available in the desktop application.</p>
      )}
      {(error || engineState.error) && (
        <p role="alert">{error || engineState.error}</p>
      )}
      {diagnostics.length > 0 && (
        <details
          onToggle={(event) => setShowDataDiagnostics(event.currentTarget.open)}
        >
          <summary>Frontend data diagnostics</summary>
          {showDataDiagnostics && (
            <pre>
              {diagnostics
                .map((item) => `${item.at} ${item.operation}: ${item.detail}`)
                .join("\n")}
            </pre>
          )}
        </details>
      )}
      {data && (
        <>
          <dl className="engine-facts">
            <dt>Provider</dt>
            <dd>{data.provider}</dd>
            <dt>Configured model</dt>
            <dd>{data.configured_model ?? "Not configured"}</dd>
            <dt>Configured response model</dt>
            <dd>
              {data.configured_reasoning_model &&
              data.configured_reasoning_model !== data.configured_model
                ? data.configured_reasoning_model
                : "Same as configured model"}
            </dd>
            <dt>Last inference mode</dt>
            <dd>
              {data.inference_mode
                ? readable(data.inference_mode)
                : "No inference yet"}
            </dd>
            {reported && (
              <>
                <dt>Provider-reported model</dt>
                <dd>{reported}</dd>
              </>
            )}
            <dt>Active goal</dt>
            <dd>{data.active_goal?.label ?? "None or sharing disabled"}</dd>
            <dt>Active step</dt>
            <dd>{data.active_step?.label ?? "Not selected"}</dd>
            <dt>Current work state</dt>
            <dd>{readable(data.work_state)}</dd>
            <dt>Relevant memories retrieved</dt>
            <dd>
              {data.retrieval
                ? `${data.retrieval.passed} passed / ${data.retrieval.found} found · ${data.retrieval.context_bytes} context bytes`
                : "No retrieval or memory disabled"}
            </dd>
            <dt>Last input / background event</dt>
            <dd>
              {readable(data.runtime.diagnostics.last_input ?? "none")} /{" "}
              {readable(data.runtime.diagnostics.last_event ?? "none")}
            </dd>
            <dt>Enabled features and quiet mode</dt>
            <dd>
              {Object.entries(data.configuration ?? {})
                .map(
                  ([name, enabled]) =>
                    `${readable(name)}: ${enabled ? "on" : "off"}`,
                )
                .join(" · ") || "Not reported"}
            </dd>
            <dt>Last decision</dt>
            <dd>
              {data.last_decision
                ? readable(data.last_decision)
                : "None this session"}
            </dd>
            <dt>Last skill</dt>
            <dd>{data.last_skill ?? "None"}</dd>
            <dt>Last agent inference latency</dt>
            <dd>
              {latency(data.last_inference?.latency_ms)}
              {data.last_inference &&
                ` · ${data.last_inference.operation} · ${data.last_inference.status}`}
            </dd>
            <dt>Last response diagnostics</dt>
            <dd>
              {data.last_inference?.finish_reason ??
                "No finish reason reported"}{" "}
              · {data.last_inference?.prompt_tokens ?? "?"} input /{" "}
              {data.last_inference?.completion_tokens ?? "?"} output tokens
              {data.last_inference?.validation_error
                ? ` · ${readable(data.last_inference.validation_error)}`
                : ""}
            </dd>
            {data.observation && (
              <>
                <dt>Interface text</dt>
                <dd>{readable(data.observation.interface_text)}</dd>
                <dt>Screenshots</dt>
                <dd>
                  {data.observation.screenshots_enabled
                    ? readable(data.observation.screenshot_status) +
                      (data.observation.screenshot_detail
                        ? ` · ${readable(data.observation.screenshot_detail)}`
                        : "")
                    : "Off"}
                </dd>
                <dt>Configured vision model</dt>
                <dd>{data.observation.configured_vision_model}</dd>
                <dt>Automatic completion</dt>
                <dd>
                  {data.observation.automatic_completion ? "Opted in" : "Off"}
                </dd>
                <dt>Observation requests this hour</dt>
                <dd>
                  {data.observation.hourly_disclosures} /{" "}
                  {data.observation.hourly_limit}
                </dd>
              </>
            )}
            <dt>Web search status</dt>
            <dd>{readable(data.web_search_status)}</dd>
            <dt>AgentRuntime status</dt>
            <dd>
              {readable(data.runtime.status)} · {data.runtime.pending_events}{" "}
              pending events
            </dd>
            <dt>Runtime counters this session</dt>
            <dd>
              {data.runtime.diagnostics.ai_calls} reasoning calls ·{" "}
              {data.runtime.diagnostics.suppressed} suppressed ·{" "}
              {data.runtime.diagnostics.cancelled} cancelled ·{" "}
              {data.runtime.diagnostics.verified} verified
            </dd>
            <dt>Suppression reason</dt>
            <dd>{data.runtime.diagnostics.suppression_reason ?? "None"}</dd>
            <dt>Last background failure</dt>
            <dd>
              {data.runtime.diagnostics.last_failure
                ? readable(data.runtime.diagnostics.last_failure)
                : "None"}
            </dd>
            <dt>Collector simulation / provider mock</dt>
            <dd>
              {data.demo_mode ? "DEMO_MODE on" : "DEMO_MODE off"} ·{" "}
              {data.ai_mock ? "AI_MOCK on" : "AI_MOCK off"}
            </dd>
          </dl>
          <p className="helper">
            Configured model is a setting. A provider-reported model is response
            metadata, not independent verification of the model running at the
            provider. Latency covers instrumented agent, chat, checkpoint and
            research requests, including their retries.
          </p>
          <section aria-label="Claude inference health">
            <h4>Safe inference health check</h4>
            <p className="helper">
              Sends one small synthetic request to Claude with a strict JSON
              Schema. No goals, memory, audio or window contents are sent.
              Provider charges may apply. Manual only, with a 20-second timeout
              and at least 60 seconds between checks.
            </p>
            <button
              type="button"
              className="secondary"
              disabled={!desktop || testing || loading}
              onClick={() => void check()}
            >
              {testing ? "Checking inference…" : "Run inference health check"}
            </button>
            <p role="status">
              {testing
                ? "Checking real structured inference…"
                : health
                  ? `${readable(health.status)} · ${latency(health.latency_ms)}`
                  : "Not checked"}
            </p>
            {health?.error && <p role="alert">{health.error}</p>}
            {health?.checked_at && (
              <p className="helper">Last check: {health.checked_at}</p>
            )}
          </section>
          <section aria-label="Live agent action check">
            <h4>Test the complete action pipeline</h4>
            <p className="helper">
              Runs nine real AI checks on disposable goals in an isolated
              in-memory database: create, steps, confirmation, completion,
              rename, Park, Resume and remaining work. No personal goals, chats
              or window text are sent. Provider charges may apply. Stops at the
              first failure; maximum 6 minutes.
            </p>
            <button
              className="secondary"
              disabled={!desktop || checkingActions || testing}
              onClick={() => void checkActions()}
            >
              {checkingActions
                ? "Checking agent actions…"
                : "Run isolated agent check"}
            </button>
            <ul aria-live="polite">
              {actionChecks.map((check) => (
                <li key={check.name}>
                  {readable(check.name)} · {check.status} · {check.latency_ms}{" "}
                  ms
                </li>
              ))}
            </ul>
          </section>
          <section aria-label="Agent event journal">
            <h4>Agent event journal</h4>
            <p className="helper">
              Latest 200 events this session. Observe → reason → decide → skill
              → verify. Waiting or staying silent can be the correct decision.
            </p>
            {(data.events ?? []).length === 0 ? (
              <p className="helper">No instrumented events yet this session.</p>
            ) : (
              <ol className="engine-events">
                {(data.events ?? []).map((event, index) => (
                  <li key={`${event.at}-${index}`}>
                    <time dateTime={event.at}>
                      {new Date(event.at).toLocaleTimeString()}
                    </time>
                    <div>
                      <strong>{readable(event.operation)}</strong>
                      <span>
                        {event.stage} · {readable(event.status)}
                        {event.goal_id ? ` · goal #${event.goal_id}` : ""}
                        {event.latency_ms != null
                          ? ` · ${event.latency_ms} ms`
                          : ""}
                      </span>
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </section>
          <h4>Available skills</h4>
          <p className="helper">
            The configured chat model proposes actions; Rust validates
            permissions, IDs, scope and state before execution. Goal and step
            completion still requires an explicit action or confirmation.
          </p>
          {data.skills.map((group) => (
            <details className="inline-details" key={group.group}>
              <summary>{group.group}</summary>
              {group.note && <p className="helper">{group.note}</p>}
              <ul>
                {group.skills.map((skill) => (
                  <li key={skill.name}>
                    <code>{skill.name}</code> · {readable(skill.permission)}
                  </li>
                ))}
              </ul>
            </details>
          ))}
        </>
      )}
    </details>
  );
}
