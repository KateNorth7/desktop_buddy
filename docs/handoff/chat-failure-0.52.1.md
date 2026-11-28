# Chat failure diagnosis — 27 November 2026

Source: clean `main` at `0511310`, identical to freshly fetched `fork/main`. `origin/main` is the older upstream; this checkout contains 45 additional commits. The installed Windows executable reports 0.52.1. No application source, installation, user settings or user records were changed by this investigation.

## Confirmed findings

- The affected chat contains six persisted user messages and no assistant messages. SQLite `PRAGMA quick_check` returned `ok`. The displayed saving error does not establish a database failure.
- The installed executable's `--check-agent-actions` reproduces failure on the first synthetic create-goal request, before an action executes: `buddy_skill_plan`, `schema_or_provider_error`, no provider model/token metadata and no goal created. The check uses an in-memory database. Capture Windows GUI-subsystem CLI output with `subprocess.run(..., capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW)`; a PowerShell GUI executable invocation can return without usable output.
- Built-in AI authentication and model health pass. A minimal structured generation returns HTTP 200 and a valid Claude message.
- A source-derived reproduction of the empty-workspace planner response schema (37 skill variants, 9 read-only skills, 41,154 serialized bytes) returns HTTP 400. Retaining only `create_goal` and `inspect_conversation` in the action alternatives, with the same model, high effort, token limit and synthetic request, returns HTTP 200. The full planner schema is rejected; the gateway conceals the provider's detailed rejection, so the exact provider schema limit is not established. Do not infer an authentication, language or disk failure from this result.
- `Conversation.tsx::chatError` replaces the backend's actionable AI errors with `QueryClient.recordError`'s generic saving message. Normal chat saves the question before requesting the planner, explaining why retries leave repeated messages without answers.
- The click-through warning is independent. `startBuddyPointer` stops permanently after any adapter/listener error and displays a restart message. Both catches discard the original error. The necessary permission exists in `src-tauri/capabilities/buddy-pointer.json`; missing permission is **not** established. The exact failing adapter operation in the reported session cannot be recovered after app exit.

## Evidence and next work

Ignored local reports: `.tools/chat-diagnostic-20261127-{health,probe,planner-probe,compact}.json` and `.tools/chat-diagnostic-20261127-actions-captured.log`. Requests used synthetic data and disposable anonymous sessions; sessions were signed out. No user messages, tasks, saved sessions or personal provider credentials were sent by these checks.

The schema reproduction and probe scripts are ignored diagnostic tools. The exact click-through trigger still needs a live reproduction. The initial investigation did not change product code or prepare a release.

## Unreleased fix prepared

The following local changes are based on `0511310`; versions remain 0.52.1 because no installer has been delivered:

- `agent_dialogue.rs` retains all registered skills and the existing scoped input contracts, but represents action inputs as JSON-encoded strings in the provider grammar. The complete input contracts remain in schema descriptions. Inputs are decoded with a size bound before the existing local argument, scope, authorization, state and confirmation checks; malformed/non-object JSON is rejected. Read mode still exposes read-only skill names. Search-page target restrictions are preserved. No extra inference call or dependency is introduced.
- Claude documents a maximum of 16 union parameters across strict schemas: https://platform.claude.com/docs/en/build-with-claude/structured-outputs#schema-complexity-limits . The old planner exceeds this limit; the new grammar fits it. Added regression checks cover that budget, encoded inputs, local validation, read-only safety and two separate goal creations.
- Chat now displays only explicitly allowlisted, safe backend AI errors; unknown errors receive a chat-specific fallback. HTTP 400 chat failures no longer direct users to replace their model configuration. Raw provider errors are not displayed.
- Pointer failures immediately restore mouse input and retry after 250, 500, 1000 and 2000 ms. A successful tick resets the failure count; five consecutive failures stop sampling and report the operation name without private error contents. Disposal cancels recovery. Listener registration failures remain fail-interactive.

Validation on this local source state:

- `npx vitest run --exclude '**/.tools/**' src/companion/useBuddyPointer.test.ts src/companion/Conversation.runtime.test.tsx`: 39 passed. The exclusion avoids accidentally running a separate ignored worktree's old tests with duplicate React installations.
- `npm test -- --exclude '**/.tools/**'`: 353 passed, one unchanged `src/core/DeadlinePicker.test.tsx` test failed because it hardcodes `Oct 22` while this Windows environment formats dates as `22 окт.` (`ru-RU`). The picker and its tests are unchanged by this patch.
- `npm run build`: passed, with the existing chunk-size advisory. `npm run lint`: passed with zero errors/warnings. `cargo fmt --check --manifest-path src-tauri/Cargo.toml`: passed.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml`: blocked before application compilation by missing MSVC `link.exe`, including outside the sandbox after attempting Visual Studio discovery. Rust regression tests are added but **not executed**. No Windows native binary or macOS build has validated this patch yet.
- A real built-in Claude request using the source-derived new schema and a synthetic Russian two-parcel request returned HTTP 200, `decision=change`, the unchanged state version, and two separate `create_goal` actions with valid JSON-decoded inputs. These actions were inspected, not executed. Report: ignored `.tools/chat-diagnostic-20261127-fixed-live.json`; the disposable session was signed out. This validates provider acceptance and planning, not installed-app execution.

The owner authorized committing and pushing this patch on 28 November. That Git-only follow-up reuses the validation above: application source, dependencies and configuration have not changed since those checks. No installer, signing change or publication is included. Complete native tests and both platform builds before delivering a patch release; the installed app still runs the previous code.
