# Current AI architecture

Source audit: 29 October 2026, app 0.49.4, baseline `54fa6074337982bf898b79937a0a5cebb3c7cd35`. This is a code-grounded architecture reference, not a new live-agent accuracy report. Later environment/model overrides and server policy can affect runtime availability.

## Components and trust boundaries

```mermaid
flowchart LR
    subgraph Device[User device]
        UI[React and TypeScript workspace and companion]
        Native[macOS Accessibility and ScreenCaptureKit or Windows UIA and window capture]
        Speech[Local Whisper audio transcription]
        Rust[Rust and Tauri runtime, validation and skills]
        DB[(Local SQLite goals, activity, chats, scoped memory and journal)]
        Evidence[Encrypted temporary review evidence]
        Vault[OS vault for personal keys and review key]
        UI --> Rust
        Native --> Rust
        Speech --> UI
        Rust <--> DB
        Rust --> Evidence
        Vault --> Rust
    end
    Rust --> Route{Selected AI connection}
    Route -->|Built-in default| Auth[Anonymous Supabase Auth session]
    Auth --> Gateway[buddy-ai Edge Function and atomic quotas]
    Gateway --> Claude[Claude Messages API]
    Route -->|Use my API keys| Direct[Direct HTTPS provider calls]
    Direct --> Claude
    Claude --> Rust
```

Built-in provider keys never enter the frontend or the public bundle. They remain in the Edge Function environment; only public Supabase client configuration ships. Personal keys entered in Settings pass to the backend for secure storage; provider calls read the OS vault or runtime environment. The unchanged macOS credential helper accepts only the pinned signed parent. Switching modes invalidates old health receipts/pending authority and does not silently use another account after failure.

Local-first is a storage/ownership boundary, not an offline-AI claim. SQLite itself is not application-encrypted. Permitted image/interface review samples have a separate encrypted lifecycle until the corresponding Daily summary is successfully saved; missed/failed reviews can retain evidence across restart. Gateway counters and anonymous Auth sessions are separate from user goals/history. See [gateway](SUPABASE_GATEWAY.md), [review storage](DAILY_ACTIVITY_REVIEW.md) and [privacy policy](privacy.html).

## Model roles

| Role | Default | Source boundary |
| --- | --- | --- |
| General background reasoning, matching and primary auxiliary operations | `claude-opus-5-5` at low effort | `claude.rs`, `http.rs`, `goal_analysis.rs`, `goal_matching.rs`, `agent_runtime/reasoning.rs` |
| Dialogue planning, action authorization/review, answers and contextual research synthesis | `claude-opus-5-5`; deliberate planning at high effort (`ANTHROPIC_RESPONSE_MODEL_ID` override) | `http.rs`, `agent_dialogue.rs`, `ai_chat.rs`, research modules |
| Goal-aware screen interpretation | `claude-opus-5-5`; first pass can use the configured vision model | `semantic_observation.rs`, `vision.rs` |
| Independent outcome verifier | `claude-opus-5-5` by default; personal runtime override where permitted | `semantic_observation.rs` |
| Retrospective image descriptions | `claude-opus-5-5` | `vision.rs`, activity review |
| Local microphone transcription | Quantized Whisper large-v3-turbo, `whisper-rs` 0.16.0 | `local_voice.rs`, `core_import.rs` |
| Source retrieval | Claude `web_search` server tool (at most two searches per run), cited excerpts only | `web_search.rs`, `research.rs`, `research_evidence.rs` |

A configured or listed model is not proof of successful generation. The server has a fixed allowlist; Built-in AI keeps its routed defaults. Personal-provider model/endpoint overrides have their own rules. A bounded interactive proposal fallback retries once on the default primary model after a stalled attempt; it does not replay an already executed action. `claude.rs` is the only module that speaks the Claude wire format: it moves system messages into the cached `system` prompt, converts strict JSON schemas into structured outputs (unsupported constraints are removed on the wire and still validated locally), maps `stop_reason` back onto the internal `choices` envelope and enables server-side fallback for safety refusals. See the README configuration table and `http.rs`.

Whisper downloads verified public weights on first use (about 548 MiB). Metal accelerates the Mac path; Windows uses CPU. Audio stays in memory. Optional transcript correction may send bounded text and permitted goal titles before the editable draft appears; only Send enters chat. OS Dictation/Apple Speech language packs do not drive this path. [Voice details](LOCAL_VOICE.md).

## Agent cycle and authority

```mermaid
flowchart TD
    Start[Collector tick and meaningful domain events] --> Gates{Consent, permissions and permitted foreground context}
    Gates -->|Unavailable| Diagnostic[Record sanitized capability or failure state]
    Gates -->|Available| Sample[Bounded text, metadata and optional actual pixels]
    Sample --> Match[Goal attribution and stable-context matching]
    Match --> Time[Measured intervals in Progress]
    Sample --> Observe[Structured screen outcome or new-work proposal]
    Memory[(Scoped personal memory and WorkContext)] --> Observe
    Memory --> EventReason[Event-driven background reasoning]
    Start --> EventReason
    Observe --> Check[Rust IDs, evidence grounding, revisions, privacy epoch and freshness]
    EventReason --> Check
    Check --> Kind{Decision type}
    Kind -->|No supported action| Silent[Silence or clarification]
    Kind -->|New goal or step| Offer[Offer through mascot queue]
    Offer --> Accept{User accepts}
    Accept -->|Yes, still valid| Commit[Validated saved mutation and receipt]
    Accept -->|Skip or stale| Unchanged[No goal or step created]
    Kind -->|Possible completion| Verify[Independent verification against full saved criterion]
    Verify -->|Insufficient evidence| Question[Ask for confirmation or stay silent]
    Question --> Offer
    Verify -->|Verified current evidence| Auto{Automatic completion permission}
    Auto -->|Enabled| Commit
    Auto -->|Disabled| Offer
    Commit --> Journal[Durable work journal and Undo where applicable]
    Journal --> Review[Grounded daily review and future scoped context]
```

These are coordinated paths, not a single model watching a continuous video. `AgentRuntime::tick` dispatches collection, domain events, matching, semantic observation/unlisted-work specialists and due activity review. Specialists retain their gates; a shared provider lock/budget bounds background work. Explicit dialogue uses its planning/read loop, independent authorization and Skill Registry executors. The model proposes; Rust owns state.

A foreground window is necessary but not sufficient. Capture can fail, text can be unavailable, relevant evidence can disappear between samples, an inference can time out, and a stale result must be rejected. Current source permits at most one capture attempt per 20 seconds and a rolling 120 background disclosures/hour, reserving headroom for independent verification. It is not a promise of a suggestion within 20 or 30 seconds. Read [observation details](PERSONAL_AI_OBSERVATION.md) for the exact catalog/evidence limits.

New tasks and steps require acceptance. Automatic completion concerns existing saved targets only, after an independent outcome check, current authority and the saved permission. New profiles enable screenshots, screen-task assistance and verified automatic completion; saved opt-outs and OS access remain authoritative. An app name, elapsed time, draft or generated scene caption alone cannot establish completion. Arbitrary offline/global goals are not guaranteed to have an observable success criterion.

## Presentation is separate from understanding

All proactive notifications are sumi character cards, never native OS notification banners. Quiet mode/hours/days, detected calls/away/fullscreen, pause/snooze and a floating chat can prevent a card from appearing. Quiet mode can silence presentation while permitted activity accounting continues. Legacy working-hours fields are not a delivery schedule. There is no former generic three-prompts-per-day presentation cap; duplicate prevention, evidence conditions and provider budgets remain.

Deadline reminders are local scheduled logic with **I’m on it / Reschedule / Chat**. They do not complete goals. Stretch reminders use active-work timing. Daily planning and summaries use their saved schedules. These can be useful without proving an AI inference occurred. [Mascot policy](MASCOT_NOTIFICATIONS.md), [deadlines](DEADLINE_REMINDERS.md).

## Scope and validation

Only Tiny Habits is registered as an executable coaching strategy; earlier descriptions of 16 methodologies were research/planning, not 16 live workflows. Calendar integration, arbitrary desktop control, meeting transcription, a vector database and model fine-tuning are not implemented. Context is permitted scoped local memory, not a model trained on the user's activity.

Unit/mock checks establish their fixture behavior. A provider health probe establishes authentication/model access. Live synthetic backend checks establish the exercised model path. Native manual checks establish only the observed build/platform/scenario. None alone proves universal goal understanding. Use [the demo runbook](HACKATHON_DEMO_RUNBOOK.md) and preserve failed/silent stages honestly.

## Source map

| Responsibility | Source |
| --- | --- |
| Event dispatch and specialist scheduling | [agent_runtime.rs](../src-tauri/src/agent_runtime.rs) and [runtime modules](../src-tauri/src/agent_runtime) |
| Dialogue and validated app skills | [agent_dialogue.rs](../src-tauri/src/agent_dialogue.rs), [skills.rs](../src-tauri/src/skills.rs) |
| Goal attribution | [goal_matching.rs](../src-tauri/src/goal_matching.rs), [work_state.rs](../src-tauri/src/work_state.rs) |
| Goal-aware outcomes | [semantic_observation.rs](../src-tauri/src/semantic_observation.rs) |
| Capture diagnostics and adapters | [vision.rs](../src-tauri/src/vision.rs), [Windows adapter](../src-tauri/src/vision/windows.rs), [macOS bridge](../src-tauri/native/macos.m) |
| Memory and checkpoints | [memory](../src-tauri/src/memory), [work_context.rs](../src-tauri/src/work_context.rs), [continuity.rs](../src-tauri/src/continuity.rs) |
| Built-in provider boundary | [shared_ai.rs](../src-tauri/src/shared_ai.rs), [gateway handler](../server/supabase/functions/buddy-ai/handler.ts) |
| Local voice and correction | [local_voice.rs](../src-tauri/src/local_voice.rs), [voice_cleanup.rs](../src-tauri/src/voice_cleanup.rs) |
| Mascot and deadlines | [companion](../src-tauri/src/companion), [InterventionCard](../src/companion/InterventionCard.tsx) |
