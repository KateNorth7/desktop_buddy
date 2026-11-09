import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Cloud, RefreshCw, Search, Sparkles } from "lucide-react";
import { api, desktop } from "../api/tauri";
import {
  serviceStateLabel,
  type ServiceId,
  type ServiceState,
} from "../ai/serviceStatus";
import { useQuery } from "../data/useQuery";
import { servicesQuery } from "../data/resources";
import { useDismissiblePopover } from "./useDismissiblePopover";

const services = [
  {
    id: "anthropic",
    name: "Claude",
    icon: Cloud,
    description:
      "Anthropic's Claude API runs sumi's AI requests in the cloud with the configured Claude model.",
  },
  {
    id: "model",
    name: "AI model",
    icon: Sparkles,
    description:
      "Plans and validates goal actions and interprets background activity. The response model writes natural replies and checkpoints. Runs through Claude, not on this computer.",
  },
  {
    id: "search",
    name: "Web search",
    icon: Search,
    description:
      "Finds web sources through Claude's search tool after you approve research. Receives a short search query, not your full memory.",
  },
] as const;
export function SidebarAiServices({
  enabled = true,
  collapsed = false,
  variant = "panel",
}: {
  enabled?: boolean;
  collapsed?: boolean;
  /** "dots" is the three status squares beside the wordmark; "panel" is the full list. */
  variant?: "panel" | "dots";
}) {
  const serviceState = useQuery(servicesQuery, desktop && enabled);
  const snapshot = serviceState.data;
  const readError = !!serviceState.error;
  const [busy, setBusy] = useState(desktop && enabled && navigator.onLine);
  const [checking, setChecking] = useState<ServiceId[]>([]);
  const inFlight = useRef<Promise<void> | null>(null);
  const started = useRef(false);
  const connectionMode = useRef<string | null>(null);
  const allowed = useRef(enabled);
  allowed.current = enabled;
  const [offline, setOffline] = useState(
    () => typeof navigator !== "undefined" && !navigator.onLine,
  );
  const [active, setActive] = useState<ServiceId>("anthropic");
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const [receipt, setReceipt] = useState("");
  const mounted = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { id, open, setOpen, rootRef, surfaceRef, triggerRef } =
    useDismissiblePopover({ restoreFocusOnEscape: false });
  useEffect(() => {
    setOpen(false);
  }, [collapsed, setOpen]);
  const refresh = useCallback(async () => {
    try {
      return await servicesQuery.refresh();
    } catch {
      return undefined;
    }
  }, []);
  const check = useCallback(() => {
    if (inFlight.current) return inFlight.current;
    if (!desktop || !allowed.current || !navigator.onLine)
      return Promise.resolve();
    setBusy(true);
    setChecking(["anthropic", "model", "search"]);
    setReceipt("");
    const work = async () => {
      try {
        const initial = await refresh();
        if (!allowed.current || !mounted.current || !navigator.onLine) return;
        if (initial?.services.some((item) => item.status === "mock")) return;
        await Promise.allSettled(
          (["anthropic"] as const).map(async (provider) => {
            try {
              await api.testConnection(provider);
            } finally {
              await refresh();
              if (mounted.current) setChecking([]);
            }
          }),
        );
        if (mounted.current) setReceipt("Connection status refreshed.");
      } finally {
        inFlight.current = null;
        if (mounted.current) {
          setChecking([]);
          setBusy(false);
        }
      }
    };
    inFlight.current = work();
    return inFlight.current;
  }, [refresh]);
  useEffect(() => {
    mounted.current = true;
    if (!desktop || !enabled) {
      started.current = false;
      return () => {
        mounted.current = false;
      };
    }
    const network = () => {
      setOffline(!navigator.onLine);
      if (navigator.onLine) void check();
    };
    if (!started.current) {
      started.current = true;
      void check();
    } else void refresh();
    window.addEventListener("online", network);
    window.addEventListener("offline", network);
    return () => {
      mounted.current = false;
      window.removeEventListener("online", network);
      window.removeEventListener("offline", network);
    };
  }, [enabled, refresh, check]);
  useEffect(() => {
    if (!snapshot?.connection_mode) return;
    const previous = connectionMode.current;
    connectionMode.current = snapshot.connection_mode;
    if (previous && previous !== snapshot.connection_mode) {
      void (inFlight.current ?? Promise.resolve()).finally(() => {
        if (mounted.current && allowed.current) void check();
      });
    }
  }, [snapshot?.connection_mode, check]);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  function keepOpen() {
    if (timer.current) clearTimeout(timer.current);
  }
  function leave() {
    keepOpen();
    timer.current = setTimeout(() => setOpen(false), 150);
  }
  function show(service: ServiceId, element: HTMLButtonElement) {
    keepOpen();
    const rect = element.getBoundingClientRect();
    triggerRef.current = element;
    setPosition({
      left: Math.min(rect.right + 14, window.innerWidth - 278),
      top: Math.max(12, Math.min(rect.top, window.innerHeight - 320)),
    });
    setActive(service);
    setOpen(true);
  }
  function state(service: ServiceId): ServiceState {
    if (offline) return "offline";
    if (checking.includes(service) || (busy && !snapshot)) return "checking";
    if (readError) return "not_checked";
    return (
      snapshot?.services.find((item) => item.id === service)?.status ??
      "not_checked"
    );
  }
  function name(service: ServiceId) {
    if (service !== "model")
      return services.find((item) => item.id === service)!.name;
    return snapshot?.configured_model?.startsWith("claude-")
      ? "Claude model"
      : "AI model";
  }
  const item = services.find((service) => service.id === active)!;
  const checkedAt = snapshot?.services.find(
    (service) => service.id === active,
  )?.checked_at;
  const popover =
    open &&
    createPortal(
      <div
        ref={surfaceRef}
        className="service-tooltip"
        id={id}
        role="dialog"
        aria-label={`${name(active)} connection`}
        style={position}
        onMouseEnter={keepOpen}
        onMouseLeave={leave}
      >
        <strong>{name(active)}</strong>
        <p>{item.description}</p>
        <button
          type="button"
          className="text-button service-check"
          aria-label={`Refresh ${name(active)} connection`}
          disabled={!desktop || !enabled || busy || offline}
          onClick={() => void check()}
        >
          <RefreshCw
            size={14}
            className={busy ? "spin" : undefined}
            aria-hidden="true"
          />
          {busy ? "Checking…" : "Check connection"}
        </button>
        <div className="service-state">
          <span
            className={`service-dot service-${state(active)}`}
            aria-hidden="true"
          />
          {serviceStateLabel(active, state(active))}
        </div>
        {checkedAt && (
          <small>
            Last checked:{" "}
            {new Date(checkedAt).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </small>
        )}
        {readError && (
          <small>
            Local status is unavailable. Check the connection again.
          </small>
        )}
      </div>,
      document.body,
    );
  if (variant === "dots") {
    return (
      <div
        className="ai-dots"
        role="group"
        aria-label="AI services"
        ref={rootRef}
      >
        {services.map((service) => (
          <button
            key={service.id}
            type="button"
            className="ai-dot"
            aria-label={`${name(service.id)}: ${serviceStateLabel(service.id, state(service.id))}`}
            aria-describedby={open && active === service.id ? id : undefined}
            aria-haspopup="dialog"
            aria-expanded={open && active === service.id}
            onMouseEnter={(event) => show(service.id, event.currentTarget)}
            onMouseLeave={leave}
            onFocus={(event) => show(service.id, event.currentTarget)}
            onClick={(event) => show(service.id, event.currentTarget)}
          >
            <span
              className={`service-dot service-${state(service.id)}`}
              aria-hidden="true"
            />
          </button>
        ))}
        {popover}
      </div>
    );
  }
  return (
    <div
      className="sidebar-ai-services"
      aria-label="AI services"
      role="group"
      ref={rootRef}
    >
      <div className="sidebar-ai-heading">
        <span className="sidebar-service-label">AI services</span>
        <button
          className="text-button service-check"
          aria-label="Check AI connections"
          title="Check authentication and model access. No generation or search is sent."
          disabled={!desktop || !enabled || busy || offline}
          onClick={() => void check()}
        >
          <RefreshCw
            size={12}
            className={busy ? "spin" : undefined}
            aria-hidden="true"
          />
        </button>
      </div>
      {services.map((service) => (
        <button
          key={service.id}
          className="sidebar-service text-button"
          aria-label={`${name(service.id)}: ${serviceStateLabel(service.id, state(service.id))}`}
          aria-describedby={open && active === service.id ? id : undefined}
          aria-haspopup="dialog"
          aria-expanded={open && active === service.id}
          onMouseEnter={(event) => show(service.id, event.currentTarget)}
          onMouseLeave={leave}
          onFocus={(event) => show(service.id, event.currentTarget)}
          onClick={(event) => show(service.id, event.currentTarget)}
        >
          <service.icon size={14} aria-hidden="true" />
          <span className="sidebar-service-label">{name(service.id)}</span>
          <span
            className={`service-dot service-${state(service.id)}`}
            aria-hidden="true"
          />
        </button>
      ))}
      <span className="service-receipt" role="status">
        {receipt}
      </span>
      {popover}
    </div>
  );
}
