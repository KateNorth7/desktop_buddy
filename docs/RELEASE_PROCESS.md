# Efficient validation and release process

## Workflow ownership

`ci.yml` classifies changes and calls the same platform/visual validation used by `release.yml`. Ordinary CI does not build installers. `platform-checks.yml` owns Windows x64 (pinned Windows 2022 runner) and Apple Silicon checks; `visual-checks.yml` owns the strict reviewed Windows browser matrix. The old automatic Windows/macOS installer workflows are removed.

- Ordinary Markdown/text and the separately hosted `docs/privacy.html`: workflow-policy checks, the fast privacy-source check and the aggregate gate; no Node dependency install, Rust compilation or screenshots. Unknown HTML/build inputs remain conservative. The acceptance checklist is an explicit exception.
- Frontend/application changes: both platform acceptance reports and applicable screenshots. CSS does not invalidate matching Rust test evidence.
- Backend, embedded role data, dependency, build-policy or toolchain changes: affected Rust evidence is invalidated. Unknown changes conservatively schedule validation; an unavailable diff schedules all gates.
- Rust lint runs inside each existing native validation job, with pinned Rust 1.99.1 and the same compiler cache: debug Clippy within the zero budget, then release-profile Clippy with `-D warnings`. A small Ubuntu aggregate preserves the required gate without allocating a second macOS runner. Both successful, zero-warning JSON reports are hashed and retained as a separate content/environment-bound proof; matching frontend-only/version-only work can reuse them. Failed, incomplete, stale or corrupted lint evidence reruns both profiles. Full validation and packaging proof require the embedded lint proof as well as acceptance/frontend bytes. Release does not repeat valid reports.
- Cancellation belongs to applicable platform/visual jobs. A documentation-only push no longer cancels an already-running native validation; a newer application change still supersedes obsolete application checks.
- Explicit `validate_installers=true` on Application checks builds unsigned validation packages on both systems. Do not install an unsigned Mac candidate: its credential helper cannot read saved keys.
- A real release always builds and verifies both signed update packages once. It cannot publish until both platform and visual gates succeed.

## Evidence and trust

The cache is not itself a green check. Scripts verify content/environment keys and receipt schema before using its results. Receipts identify the producing commit, run URL, date and counts; reused suites retain their provenance and ignored tests.

The keys bind tracked file paths, executable modes and bytes, the validation policy/workflows, lockfiles, actual Rust/Node versions, OS/architecture, hosted runner image, deployment/debug flags and a weekly bucket. Receipts expire after seven days. Corrupt, missing, failed, future-dated, stale or mismatching evidence falls back to running checks. No partial-prefix match is allowed for successful-test evidence.

Only trusted `main` runs save successful-check receipts. PRs can consume applicable default-branch evidence but cannot save reusable proof. GitHub's ref-scoped caches keep untrusted PR writes out of the main release path. No `pull_request_target` execution or secret sharing with PR checks is introduced.

Rust fingerprints exclude only the application's synchronized version fields. Dependency versions, all Rust/native source, configuration, scripts and embedded role JSON remain significant. Version consistency always runs separately; a new release's binary is still compiled, version-pinned, signed and verified.

Full validation also binds the exact production frontend and acceptance report bytes. Packaging may suppress Tauri's repeated frontend build only after verifying those bytes and matching inputs. It changes only `beforeBuildCommand`; updater verification, signing policy and native compilation remain intact.

Visual reuse binds the reviewed fixtures/baselines, frontend inputs and Windows environment, plus the successful Playwright JSON report. Generation of provisional candidates never certifies or overwrites approved baselines automatically.

Per-commit/per-platform concurrency serializes overlapping CI and release validation. The second run can consume the first successful result instead of simultaneously running identical checks. Caches are expendable: deletion or eviction makes the pipeline slower, not less strict.

## Rust compilation

Cargo caches have stable platform/target namespaces shared between appropriate workflows, with separate test and bundle keys so an immutable test cache cannot prevent saving release artifacts. Workspace artifacts are retained; Cargo still decides what needs recompilation. Saving compiled artifacts and proof is restricted to main. Only CI dev/test debug symbols are disabled; test assertions and release optimization/signing settings are unchanged. No new local Rust target directory is required for routine frontend work.

Clippy shares each native job's Cargo cache; it no longer has a separate runner/cache namespace. Its successful-report proof remains separate so frontend-only changes can reuse it without trusting compiler-cache presence. Both platform Rust tests, formatting, the full 122-item acceptance mapping and immutable helper/signature checks remain available. Automated acceptance does not establish the manual/live parts of that checklist.

## Prepare a complete release once

Since 0.50.0 releases publish from `KateNorth7/desktop_buddy`, the repository named by `RELEASE_REPOSITORY` in `scripts/product-branding.mjs` and `src-tauri/src/signing_trust.rs`; the Tauri updater endpoints must point at the same repository, which `scripts/check-version.mjs` enforces. The hackathon submission `ArtmarketVM/desktop_buddy` stays at 0.49.4 and keeps the retained Windows compatibility assets and the Pages privacy policy. The releasing repository needs the five signing secrets; changing the repository never changes the trusted signing keys.

Use one release owner for the combined changes. Developers finish and share their small code commits first; the release owner prepares notes, policy and synchronized metadata, then starts one signed release. Do not independently package the same version from both developer sessions.

The fast source check compares the documented capture cadence with `observation::SAMPLE_INTERVAL` and checks the current encrypted-evidence disclosure. CI planning and `release:prepare --check` run it before native builds, so a stale privacy statement stops the inexpensive preflight instead of cancelling completed installers. Source-only policy corrections do not invalidate application proof; public Pages/content verification remains separate.


1. Complete feature/fix work and targeted tests. Update relevant docs and known limitations.
2. Write English notes beginning with `# Desktop Buddy X.Y.Z`; no TODO/TBD placeholders.
3. Run `npm run release:prepare -- X.Y.Z --notes-file PATH`. It validates the notes and updates all five version manifests together. It performs no Git operations, network publication or signing.
4. Review the diff and run affected local checks once. Commit/push only within the user's authorization; preserve unrelated files.
5. Dispatch **Publish Windows and macOS release**, `release_tag=vX.Y.Z`, from main. Preflight requires a new version, completed notes and synchronized manifests. Applicable trusted proofs are reused; misses run the normal acceptance suites.
6. Run `npm run ci:watch -- RUN_ID` once. It emits only stage changes, waits one minute between requests and retries temporary monitoring failures with bounded backoff. A monitoring failure means the outcome is unknown, not that the build failed. Read detailed logs only for failures or an explicit diagnostic need. Assistants start it once as a background process and wait for it to exit; they do not re-arm short watchers or poll run status in between.
7. After success, verify the actual public version, both channels and asset/signature/checksum results once. Record source/results and stop. Documentation-only follow-ups do not require repeating application tests or installers.

Do not change Pages/protection settings, trusted signing keys or approved screenshots to make a release pass. Do not bump the app version or publish a new installer merely for CI/documentation work.

## Measured 0.48.1 delay and current optimization

The 28 October 2026 run took about 96 minutes from the first source check to public assets, and about 102 minutes including the final verification note. This was not one 100-minute installer compilation:

- Initial check [40728044765](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/40728044765): 10 minutes, failed an expiry regression already corrected upstream.
- Corrected check [40732457888](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/40732457888): 32 minutes including a cancelled/abandoned macOS lint allocation and its retry; native validation itself finished in about 5 minutes on Mac and 8 minutes on Windows.
- First packaging [40737913721](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/40737913721): cancelled after nearly 8 minutes for stale privacy wording.
- Privacy-only check [40739316924](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/40739316924): nearly 7 minutes; now handled by the fast source preflight.
- Final signed release [40742101371](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/40742101371): 8m12s. Native packaging was 173 seconds on Mac and 269 seconds on Windows, in parallel. Editing, handoff and verification gaps account for the remaining elapsed time.

The new workflow removes the extra macOS lint allocation, preserves both lint profiles with verified reuse, avoids native work for the external policy, and catches policy mismatches before packaging. The first run after this proof-format change will rebuild valid evidence; later matching runs can reuse it. These changes have local regression/syntax validation; their real hosted-runner speedup has not yet been measured. Runner availability remains outside this repository's control. Use the measured 8-minute final release as a useful warm-run baseline, not a guaranteed deadline.

## New sessions and cost measurement

Start the next independent task with `docs/SESSION_HANDOFF.md` and the concrete request. Keep that file short and link detailed evidence instead of storing chat transcripts. This reduces repeated discovery in fresh sessions; it does not remove the existing chat from Codex's context or guarantee a billing reduction. Codex manages conversation context itself.

Measure cold and warm runs separately: wall time, number of test/packaging executions, proof hits/misses and runner/cache conditions. Record token/cost figures only from actual client/provider usage; CI duration is not a token bill. Do not promise a fixed speedup or percentage without measurements.

## Validation of this infrastructure change

Local workflow syntax/expression checks use checksum-pinned actionlint 1.7.12. CI regression tests cover selective gates, input changes, version normalization, binary integrity, expiry/failure rejection, preserved ignored tests, report/frontend corruption, monitoring and release preparation. The following completed checks exercise the real GitHub runners; validation installers are test artifacts and were not published as an application release.

| Check                                     | Result                                                                                      | Evidence                                                                                 |
| ----------------------------------------- | ------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Local frontend tests and production build | 265 tests passed; build passed                                                              | Reused while application inputs remained unchanged                                       |
| Release policy tests                      | 18 passed                                                                                   | Local `npm run test:release`                                                             |
| CI/proof/acceptance regressions           | 22 passed                                                                                   | Local `node --test scripts/ci.verify.mjs scripts/acceptance.verify.mjs`                  |
| Initial cold required gates               | Passed in 9m44s, including queue time                                                       | [Run 39562112737](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39562112737) |
| Warm required gates                       | Passed in 60s, including queue time; both platform suites and visual evidence reused        | [Run 39567015380](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39567015380) |
| Both native validation packages           | Passed; matching acceptance evidence reused; no repeated Vite build in either packaging log | [Run 39565766616](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39565766616) |

These are measured validation runs, not a signed-release duration or token-cost comparison. Cold/warm runs had different cache states and runner snapshots; they are not a controlled speedup benchmark. Windows is now pinned to Windows 2022: a moving `windows-latest` alias supplied different images for the same commit, correctly invalidating the proof. One obsolete prototype run was cancelled after that diagnosis.

The final effective bundle-cache verification used the same commit (`8c54921`) and platform environments:

| Measurement                                                           | Cold bundle cache             | Warm bundle cache                        |
| --------------------------------------------------------------------- | ----------------------------- | ---------------------------------------- |
| Required checks plus both validation installers, including queue time | 17m10s                        | 5m02s                                    |
| Windows native packaging step                                         | 7m40s                         | 3m21s                                    |
| Apple Silicon native packaging step                                   | 5m32s                         | 2m35s                                    |
| Acceptance execution                                                  | Fresh, both platforms         | Verified evidence reused, both platforms |
| Cargo bundle cache                                                    | New platform namespaces saved | Exact full-key hits, both platforms      |

Evidence: [cold run 39567639225](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39567639225), [warm run 39569944975](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39569944975). Both passed the strict visual gate. These timings cover unsigned validation packages, not production signing/publication. The cold total also includes fresh acceptance; packaging rows isolate the native packaging steps. Native packaging, linking, signing and cache downloads still take time. No fixed speedup is promised for different changes or runner images.

The documentation-only follow-up uses the same classifier: only the small workflow-policy/unit checks and aggregate gate apply. Confirm the associated run from the final documentation commit; no application or installer change is included.

Signed releases since then: the 0.39.4 release (both signed installers, verification and publication) took 11m16s in [run 39590858703](https://github.com/ArtmarketVM/desktop_buddy/actions/runs/39590858703), inside the 10–15 minute target for a small change with warm caches.

## References

- GitHub cache scope and immutability: https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching
- Cargo cache behavior: https://github.com/Swatinem/rust-cache
- Codex project instructions: https://learn.chatgpt.com/docs/agent-configuration/agents-md
- Keeping material session context: https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex
