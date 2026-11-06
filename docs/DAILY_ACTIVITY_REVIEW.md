# Daily activity review

Current review lifecycle at release 0.49.4. The configured **Daily summary** time is the review boundary; the earlier Working hours requirement is superseded. See [current observation](PERSONAL_AI_OBSERVATION.md) for later attribution/trail changes and [mascot delivery](MASCOT_NOTIFICATIONS.md) for presentation timing.

## Measured time and goals

The existing collector measures intervals even without a selected goal, using
its existing idle, exclusion, pause and privacy guards. Observed activity does
not account for every minute of elapsed work. A selected goal alone cannot
explain a generic ChatGPT/browser window: uncertain work stays **Pending review**.

Context belongs to a contiguous segment within a five-minute bucket. A later
image cannot explain an earlier visit to the same generic window. Changed
protected interface text invalidates a cached AI assignment. Planning in sumi
uses explicit short-lived UI context, without a separate timer or self-capture.

Recognized work outside the goal list proposes an additional open goal. The user
confirms its title before adding it to Today. Time transfers with that goal;
recognition and duration never prove completion. Manual assignment and correction
transfer the existing measured intervals and daily aggregates atomically,
including across midnight. Retries cannot duplicate time and background AI
cannot overwrite a manual decision.

## Temporary evidence

Permitted bounded interface text and opt-in active-window screenshots are
encrypted with AES-256-GCM. A random key lives separately in Windows Credential
Manager or a dedicated macOS Keychain item. Provider credentials and the immutable
macOS credential helper are unchanged. Tests use isolated keys.

Evidence survives restart and assignments until the summary is durably saved.
It is not written to Personal Memory or logs. Explicit Clear local history also
clears pending evidence. Automatic history retention preserves unfinished review
periods and their accounting sources. The encrypted store is limited to 64 MB;
a full store reports an error and stops saving new evidence instead of deleting
unfinished evidence silently.

Capture retains its opt-in, foreground/exclusion/private-window checks, bounded
image size and 20-second cooldown. It is conditional on recognition needs,
not continuous recording or a promise of an image for every activity.

## Daily review lifecycle

1. Freeze the next Daily summary boundary in local time when the first interval
   is observed. A changed time applies to a subsequent period. DST boundaries
   handle repeated hours and gaps explicitly.
2. Use the existing dispatcher/provider lane and rolling observation budget.
   Recover missed periods after restart or sleep.
3. Retry retained images whose initial recognition failed, using only historical
   evidence and current screenshot consent. Describe one bounded image per job,
   then review up to eight unresolved blocks per job.
4. Match bounded redacted text/descriptions to open goals, propose unlisted work,
   and preserve uncertainty. Automatic transfers to existing goals require
   Automatic goal matching. New goals require user confirmation.
5. Save the summary and delete temporary images/raw samples in one transaction.
   Provider failure retries after five minutes; the failure category is kept on
   the period, and after twelve failures the period is closed with its remaining
   blocks pending so its evidence is released rather than held indefinitely. A
   period describes at most 24 retained images before reviewing the remaining
   blocks on text, and one image is retained per context segment. Failed saves
   and interrupted jobs preserve evidence and their durable completion markers.

When AI is disabled, sharing is revoked, no provider is configured, or mock
responses are enabled, save an honest local summary with unresolved blocks.
Evidence is removed only with the successful saved summary. Disabling capture
alone does not delete existing evidence.

The summary preserves goal time, proposed goals, outside-goal activity,
unresolved blocks and recorded results. Manual assignments can update it after
screenshots are deleted. Today shows current review blocks; My progress also
shows the latest fourteen summaries.

Notification delivery uses 0.41.0's shared mascot queue and its Daily summary
schedule, quiet mode and delivery rules. Suppressing a message cannot prevent
saving or trigger evidence cleanup.

## Validation

See `SESSION_HANDOFF.md` for commands, results and platform limits. Windows x64
and Apple Silicon macOS 14.5+ are supported. Both native builds must pass before
publishing a complete update. No installer is part of this development change.
