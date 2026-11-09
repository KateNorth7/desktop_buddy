import { invoke } from "../data/ipc";
export type ServiceId = "anthropic" | "model" | "tavily";
export type ServiceState =
  | "verified"
  | "unavailable"
  | "not_checked"
  | "expired"
  | "not_configured"
  | "checking"
  | "mock"
  | "offline";
export type ServiceSnapshot = {
  connection_mode?: "built_in" | "personal";
  configured_model: string | null;
  configured_response_model?: string | null;
  configured_vision_model?: string | null;
  configured_image_description_model?: string | null;
  services: {
    id: ServiceId;
    status: ServiceState;
    checked_at: string | null;
  }[];
};
export const serviceStatusApi = {
  read: () => invoke<ServiceSnapshot>("get_ai_service_status"),
};
export function serviceStateLabel(id: ServiceId, state: ServiceState): string {
  switch (state) {
    case "verified":
      return id === "model" ? "Model available" : "Connected";
    case "unavailable":
      return "Could not verify access";
    case "not_configured":
      return "Not connected";
    case "checking":
      return "Checking…";
    case "expired":
      return "Check again";
    case "mock":
      return "Simulated provider";
    case "offline":
      return "Offline";
    default:
      return "Not checked";
  }
}
