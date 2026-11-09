//! Recognition timing is separate from page evidence. Gaps never preserve an assignment.
use super::*;

// Provisional two-cycle bound, measured from the first missing sample; not a measured optimum.
const METADATA_GAP_LIMIT: Duration = Duration::from_secs(6);
/// A hop to another window and back within this long keeps the first window's presence clock.
pub(crate) const EXCURSION_LIMIT: Duration = Duration::from_secs(20);
const PRESENCE_MEMORY: usize = 8;

pub(crate) struct Presence {
    key: String,
    /// Time actually spent in the window, across its hops; never the wall clock.
    dwell: Duration,
    last_seen: Instant,
}
/// Remember a window just left, so a quick return resumes its clock rather than restarting it.
pub(crate) fn remember_presence(
    presences: &mut std::collections::VecDeque<Presence>,
    key: String,
    since: Instant,
    last_seen: Instant,
) {
    presences.retain(|p| p.key != key);
    presences.push_back(Presence {
        key,
        dwell: last_seen.saturating_duration_since(since),
        last_seen,
    });
    while presences.len() > PRESENCE_MEMORY {
        presences.pop_front();
    }
}
/// The clock of a window left no longer than `EXCURSION_LIMIT` ago, consumed on return:
/// the moment that, from `now`, accounts for the time already spent there. Glances at a
/// second window therefore add up only as the seconds actually spent in it, so hopping
/// cannot make a window stable that was never looked at for long.
pub(crate) fn resume_presence(
    presences: &mut std::collections::VecDeque<Presence>,
    key: &str,
    now: Instant,
) -> Option<Instant> {
    let index = presences.iter().position(|p| p.key == key)?;
    let presence = presences.remove(index)?;
    (now.saturating_duration_since(presence.last_seen) <= EXCURSION_LIMIT)
        .then(|| now.checked_sub(presence.dwell))
        .flatten()
}

pub(super) struct MetadataGap {
    key: String,
    foreground: String,
    elapsed: Duration,
    first_missing: Instant,
}

pub(super) fn metadata_unavailable(sample: &ActivitySnapshot) -> bool {
    (sample.browser.is_some() || crate::browser::browser_name(&sample.process_name).is_some())
        && sample
            .browser
            .as_ref()
            .and_then(|b| b.domain.as_deref())
            .is_none_or(|domain| domain.trim().is_empty())
}

// A title alone cannot identify a browser window. Store only hashes, never a prior address.
fn foreground(sample: &ActivitySnapshot) -> Option<String> {
    let window = sample.window_id.filter(|id| *id != 0)?;
    let browser = sample.browser.as_ref()?;
    Some(crate::companion::engine::fingerprint(
        &json!([
            sample.process_name.to_lowercase(),
            window,
            sample.window_title,
            browser.browser,
            browser.page_title,
            sample.document_key,
        ])
        .to_string(),
    ))
}

impl Runtime {
    /// Returns whether page text must be invalidated, including any read still in flight.
    pub(super) fn observe_at(
        &mut self,
        sample: &ActivitySnapshot,
        allowed: bool,
        now: Instant,
    ) -> bool {
        if !allowed {
            self.clear();
            self.view.enabled = true;
            return true;
        }
        let stale = self
            .last_seen
            .is_some_and(|at| now.saturating_duration_since(at) > Duration::from_secs(15));
        let changed = stale
            || self
                .sample
                .as_ref()
                .is_none_or(|old| fingerprint(old) != fingerprint(sample));
        let missing = metadata_unavailable(sample);
        let page_changed = changed
            && (sample.browser.is_some()
                || self
                    .sample
                    .as_ref()
                    .is_some_and(|old| old.browser.is_some()));
        let frame = foreground(sample);
        let mut gap = self.metadata_gap.take();
        // Leaving a window for another keeps its clock for a short while; a gap does not,
        // and neither does losing a browser window's address, which the stricter
        // metadata-gap rule below governs on its own.
        if changed && !stale && !missing {
            if let (Some(old), Some(since), Some(last)) =
                (&self.sample, self.stable_since, self.last_seen)
            {
                if !metadata_unavailable(old) {
                    remember_presence(&mut self.presences, fingerprint(old), since, last);
                }
            }
        }
        let resumed = (changed && !stale && !missing)
            .then(|| resume_presence(&mut self.presences, &fingerprint(sample), now))
            .flatten();
        if !missing {
            self.unverified_since = None;
            self.unverified_asked = None;
        }
        if missing {
            // The title-only window keeps its own clock and its own AI answer, nothing else.
            if changed {
                self.unverified_since = Some(now);
                self.unverified_asked = None;
            }
            if !stale {
                if let (Some(old), Some(last), Some(since), Some(frame)) =
                    (&self.sample, self.last_seen, self.stable_since, &frame)
                {
                    if !metadata_unavailable(old) && foreground(old).as_ref() == Some(frame) {
                        gap = Some(MetadataGap {
                            key: fingerprint(old),
                            foreground: frame.clone(),
                            // Exclude the uncertain transition into the missing sample as well.
                            elapsed: last.saturating_duration_since(since),
                            first_missing: now,
                        });
                    }
                }
            }
            self.metadata_gap = gap.filter(|g| {
                !stale
                    && frame.as_ref() == Some(&g.foreground)
                    && now.saturating_duration_since(g.first_missing) <= METADATA_GAP_LIMIT
            });
            self.stable_since = None;
            self.cache.clear();
            self.pending_suggestion = None;
            self.text_fingerprint = None;
            if changed || self.view.source.as_deref() != Some("model_unverified") {
                self.view = MatchView {
                    enabled: true,
                    ..Default::default()
                };
            }
        } else if changed {
            // Only timing can resume after fresh, exact metadata confirmation. Matches,
            // page text and request revisions always belong to the new observation.
            self.stable_since = Some(
                gap.filter(|g| {
                    !stale
                        && g.key == fingerprint(sample)
                        && now.saturating_duration_since(g.first_missing) <= METADATA_GAP_LIMIT
                })
                .map(|g| now - g.elapsed)
                .or(resumed)
                .unwrap_or(now),
            );
        }
        // A new title in the same window keeps counting toward the AI-identified goal while a
        // refresh confirms or replaces it. A browser window may hold unrelated tabs, so it also
        // needs an unchanged domain. Gaps, missing metadata and other windows never carry.
        let carried = (changed
            && !stale
            && !missing
            && self.view.goal_id.is_some()
            && matches!(self.view.source.as_deref(), Some("model" | "model_carry")))
        .then_some(self.sample.as_ref())
        .flatten()
        .filter(|old| {
            let domain = |a: &ActivitySnapshot| {
                a.browser
                    .as_ref()
                    .and_then(|b| b.domain.as_deref())
                    .map(str::to_string)
            };
            old.process_name.eq_ignore_ascii_case(&sample.process_name)
                && old.window_id.is_some_and(|id| id != 0)
                && old.window_id == sample.window_id
                && !metadata_unavailable(old)
                && ((old.browser.is_none() && sample.browser.is_none())
                    || (domain(old).is_some() && domain(old) == domain(sample)))
        })
        .map(|_| MatchView {
            goal_id: self.view.goal_id,
            confidence: self.view.confidence,
            reason: Some("AI match carried over within the same window; confirming".into()),
            source: Some("model_carry".into()),
            enabled: true,
        });
        if changed {
            self.context_revision = self.context_revision.wrapping_add(1);
            self.corrected_key = None;
            // A new window, a gap or missing metadata is new evidence: the re-ask
            // debounce only holds within one uninterrupted presence in a window.
            self.asked = None;
            self.view = carried.unwrap_or(MatchView {
                enabled: true,
                ..Default::default()
            });
            self.pending_suggestion = None;
            self.text_fingerprint = None;
        }
        self.view.enabled = true;
        self.sample = Some(sample.clone());
        self.last_seen = Some(now);
        page_changed
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;
    #[test]
    fn a_quick_return_resumes_only_the_time_actually_spent() {
        let base = Instant::now();
        let mut presences = std::collections::VecDeque::new();
        // Twelve seconds in A, then away.
        remember_presence(
            &mut presences,
            "A".into(),
            base,
            base + Duration::from_secs(12),
        );
        // Back ten seconds later: the clock stands at twelve seconds of dwell, not twenty-two.
        let now = base + Duration::from_secs(22);
        let since = resume_presence(&mut presences, "A", now).unwrap();
        assert_eq!(now.duration_since(since), Duration::from_secs(12));
        assert!(presences.is_empty(), "consumed on return");
        // A window left too long ago starts over.
        remember_presence(
            &mut presences,
            "B".into(),
            base,
            base + Duration::from_secs(5),
        );
        assert!(resume_presence(&mut presences, "B", base + Duration::from_secs(40)).is_none());
        // Memory is bounded and the newest presence for a key wins.
        for n in 0..20 {
            remember_presence(
                &mut presences,
                format!("w{n}"),
                base,
                base + Duration::from_secs(n),
            );
        }
        assert!(presences.len() <= 8);
    }
}
