import { invoke } from "../data/ipc";

export interface InferenceHealth {
  status: string;
  latency_ms: number | null;
  provider_reported_model: string | null;
  error: string | null;
  checked_at: string | null;
}
export interface PersonalAiEngine {
  events?: {
    at: string;
    stage: string;
    operation: string;
    status: string;
    latency_ms: number | null;
    goal_id: number | null;
  }[];
  configuration?: Record<string, boolean>;
  provider: string;
  configured_model: string | null;
  configured_reasoning_model?: string | null;
  inference_mode?: "fast" | "deliberate" | null;
  active_goal: { id: number; label: string } | null;
  active_step: { id: string; label: string } | null;
  work_state: string;
  retrieval: { found: number; passed: number; context_bytes: number } | null;
  last_decision: string | null;
  last_skill: string | null;
  last_inference: {
    finish_reason?: string | null;
    prompt_tokens?: number | null;
    completion_tokens?: number | null;
    validation_error?: string | null;
    operation: string;
    status: string;
    latency_ms: number;
    provider_reported_model: string | null;
  } | null;
  observation?: {
    interface_text: string;
    screenshots_enabled: boolean;
    screenshot_status: string;
    screenshot_detail?: string;
    configured_vision_model: string;
    automatic_completion: boolean;
    hourly_disclosures: number;
    hourly_limit: number;
  };
  tavily_status: string;
  runtime: {
    status: string;
    pending_events: number;
    diagnostics: {
      last_input?: string | null;
      last_event?: string | null;
      ai_calls: number;
      suppressed: number;
      cancelled: number;
      verified: number;
      suppression_reason: string | null;
      last_failure?: string | null;
    };
  };
  health: InferenceHealth;
  skills: {
    group: string;
    skills: { name: string; permission: string }[];
    note: string;
  }[];
  demo_mode: boolean;
  ai_mock: boolean;
}
export const engineApi = {
  status: () => invoke<PersonalAiEngine>("get_personal_ai_engine"),
  actions: () =>
    invoke<Array<{ name: string; status: string; latency_ms: number }>>(
      "test_agent_actions",
    ),
  health: () => invoke<InferenceHealth>("test_claude_inference"),
};
