// Evidence covers deterministic contracts, not installed-app or live-account acceptance.
export const coverage = {};
function map(ids, scope, evidence, remaining = "") {
  for (const id of ids.split(" ")) {
    if (coverage[id]) throw new Error(`Duplicate acceptance mapping: ${id}`);
    coverage[id] = { scope, evidence, remaining };
  }
}
const rust = (...names) => names.map((name) => `rust:${name}`);
const ui = (file, title) => `frontend:${file}::${title}`;
const manual = (ids, reason) => map(ids, "manual", [], reason);
const partial = (ids, evidence, reason) =>
  map(ids, "partial", evidence, reason);
const contract = (ids, evidence) => map(ids, "contract", evidence);

manual(
  "PRE-01 PRE-04 PRE-05",
  "Record the installed version, real OS permissions/languages, and pre-update user-data baseline on each device.",
);
partial(
  "PRE-02",
  rust(
    "hackathon_hero_two_lanes_voice_park_receipt_resume_research_summary_restart",
  ),
  "Prepare and record the full four-step manual fixture on each installed app.",
);
partial(
  "PRE-03",
  rust("mock_provider_is_separate_from_simulated_collection_and_sends_no_http"),
  "Configure and verify real accounts and permission settings without exposing keys.",
);

partial(
  "P01-01",
  rust(
    "create_goal_through_structured_plan_and_result",
    "read_skills_today_and_focus_share_the_same_registry",
    "structured_skill_orchestration_uses_real_backend_result_for_reply",
  ),
  "Repeat the natural-language create/list/details flow with the live model.",
);
partial(
  "P01-02",
  rust(
    "rename_goal_uses_id_even_when_names_are_identical",
    "delete_restore_preserves_step_ids_time_and_completion_events",
    "deleted_goal_backups_follow_history_retention",
  ),
  "Verify chat deletion and restoration on the installed app. Delete permanently in Goals has no chat undo backup.",
);
partial(
  "P01-03",
  rust(
    "read_skills_today_and_focus_share_the_same_registry",
    "completing_a_goal_completes_its_steps_and_reopening_starts_them_over",
  ),
  "Check chat routing and corresponding installed UI state for each transition.",
);
partial(
  "P01-04",
  rust(
    "multi_action_command_can_reference_new_objects_without_invented_ids",
    "add_there_uses_last_referenced_goal_across_messages",
    "delete_second_then_complete_first_requires_exact_confirmation",
  ),
  "Repeat the four-step conversational sequence with a live model.",
);
partial(
  "P01-05",
  rust(
    "create_step_returns_actual_id_and_records_conversation_reference",
    "complete_and_reopen_step_reuse_goal_completion_rules",
    "delete_second_then_complete_first_requires_exact_confirmation",
  ),
  "Exercise renaming and all step actions through the installed chat UI.",
);
partial(
  "P01-06",
  rust("acceptance_ordinals_include_completed_steps_in_the_full_order"),
  "Repeat the ordinal scenario with live-model phrasing in the installed app.",
);
partial(
  "P01-07",
  rust(
    "ambiguous_reference_requests_clarification_without_mutation",
    "structured_skill_orchestration_returns_clarification_without_executing",
  ),
  "Verify clarification with the live model and ambiguous user phrasing.",
);
contract(
  "P01-08",
  rust(
    "stale_state_rejects_entire_plan",
    "state_revision_rejects_change_then_change_back",
    "old_conversation_reference_never_targets_reused_sqlite_goal_id",
  ),
);
contract(
  "P01-09",
  rust(
    "invalid_goal_id_never_falls_back_to_current_goal",
    "invalid_step_id_is_rejected_without_text_matching",
    "step_from_another_goal_is_rejected",
    "goal_sharing_off_blocks_existing_objects_and_sends_no_reference_state",
  ),
);
contract(
  "P01-10",
  rust("batch_rolls_back_goal_creation_and_references_on_later_failure"),
);
partial(
  "P01-11",
  rust(
    "continuity_read_skill_validates_goal_view_and_result_contract",
    "inspect_does_not_resume_or_change_focus_and_resume_can_leave_focus_unchanged",
    "feedback_and_attachment_are_idempotent_and_use_shared_skill_validation",
  ),
  "Check natural-language routing for every read-only family with the live model.",
);

contract(
  "P02-01",
  rust(
    "work_context_is_persistent_and_preserves_creation_timestamp",
    "restart_and_new_date_preserve_independent_lanes",
  ),
);
contract(
  "P02-02",
  rust(
    "start_park_resume_and_waiting_events_reuse_existing_lifecycle",
    "confirmed_completions_create_static_episodes_and_update_checkpoint",
  ),
);
contract(
  "P02-03",
  rust("activity_ticks_do_not_create_memory_or_copy_window_text"),
);
partial(
  "P02-04",
  rust(
    "explicit_preference_is_saved_without_training_or_inference",
    "memory_survives_database_reopen",
  ),
  "Inspect and confirm the preference in the installed UI after restart.",
);
contract(
  "P02-05",
  rust("inference_requires_three_independent_events_across_two_days"),
);
contract(
  "P02-06",
  rust(
    "edit_forget_and_clear_memory_never_delete_goals_steps_chat_or_activity",
  ),
);

contract(
  "P03-01",
  rust(
    "retrieval_excludes_unrelated_goals_chats_facts_and_activity",
    "pinned_memories_survive_recency_but_never_override_scope",
  ),
);
contract(
  "P03-02",
  rust(
    "memory_and_utf8_context_budgets_include_goal_and_checkpoint",
    "acceptance_default_memory_limits_bound_entries_and_utf8_bytes",
  ),
);
partial(
  "P03-03",
  [
    ui(
      "src/components/MemorySettings.test.tsx",
      "shows five separate switches, categories and privacy explanation",
    ),
  ],
  "Inspect all categories on the installed Windows and Mac UI.",
);
partial(
  "P03-04",
  [
    ...rust(
      "controls_and_checkpoint_pin_survive_database_reopen",
      "edit_forget_and_clear_memory_never_delete_goals_steps_chat_or_activity",
    ),
    ui(
      "src/components/MemorySettings.test.tsx",
      "edits with the original version and confirms the corrected content",
    ),
    ui(
      "src/components/MemorySettings.test.tsx",
      "pins an item without a destructive confirmation",
    ),
    ui(
      "src/components/MemorySettings.test.tsx",
      "requires confirmation to forget an item and supports cancel",
    ),
  ],
  "Repeat edit, pin, forget, and full app restart on each device.",
);
contract("P03-05", rust("forget_goal_is_isolated_and_clears_its_checkpoint"));
contract(
  "P03-06",
  rust(
    "edit_forget_and_clear_memory_never_delete_goals_steps_chat_or_activity",
  ),
);
contract(
  "P03-07",
  rust(
    "memory_off_blocks_capture_retrieval_and_new_writes_but_preserves_management",
  ),
);
partial(
  "P03-08",
  [
    ...rust(
      "individual_toggles_gate_capture_and_recall",
      "controls_and_checkpoint_pin_survive_database_reopen",
    ),
    ui(
      "src/components/MemorySettings.test.tsx",
      "shows five separate switches, categories and privacy explanation",
    ),
  ],
  "Check all switches persist after a full installed-app restart.",
);
contract(
  "P03-09",
  rust(
    "suggestions_require_explicit_verbatim_evidence_and_user_confirmation",
    "malformed_model_proposals_and_malformed_controls_are_rejected",
  ),
);

partial(
  "P04-01",
  rust(
    "park_active_goal_saves_step_and_stops_its_timer_without_claiming_completion",
    "last_result_uses_only_confirmed_completion_and_waiting_survives_resume",
  ),
  "Verify the Today entry point and native timer/UI state.",
);
partial(
  "P04-02",
  rust("six_parallel_contexts_are_isolated"),
  "Verify Park from Goals and both expandable context records in the installed app.",
);
partial(
  "P04-03",
  rust("resume_restores_context_focus_and_step_without_starting_a_timer"),
  "Verify installed UI focus, Today, and the stopped manual timer.",
);
partial(
  "P04-04",
  rust(
    "restart_and_day_change_preserve_next_action_waiting_and_independent_lanes",
  ),
  "Close and relaunch the actual Windows and Mac apps.",
);
partial(
  "P04-05",
  rust(
    "park_chat_commands_generate_then_validate_then_save_checkpoint",
    "resume_and_inspect_chat_use_exact_context_and_never_other_lanes",
  ),
  "Repeat commands with live-model phrasing.",
);
contract(
  "P04-06",
  rust(
    "inspect_does_not_resume_or_change_focus_and_resume_can_leave_focus_unchanged",
  ),
);
partial(
  "P04-07",
  rust(
    "resume_restores_context_focus_and_step_without_starting_a_timer",
    "research_preserves_waiting_dependency_and_does_not_confirm_completion",
  ),
  "Observe that Resume launches no external native applications.",
);
partial(
  "P04-08",
  rust(
    "six_parallel_contexts_are_isolated",
    "acceptance_seventh_goal_has_independent_park_and_resume_context",
  ),
  "Verify native UI access to all seven independent goal contexts.",
);
contract(
  "P04-09",
  rust(
    "missing_checkpoint_completed_goal_and_memory_off_do_not_mutate",
    "stale_state_invalidates_context_and_memory_permissions_changes",
  ),
);
partial(
  "P04-10",
  rust("invalid_or_stale_generated_checkpoint_never_parks_the_goal"),
  "Check missing real account configuration and recovery guidance on each device.",
);
contract(
  "P04-11",
  rust("current_window_text_never_enters_checkpoint_source_or_storage"),
);

contract(
  "P05-01",
  rust(
    "meaningful_event_calls_ai_and_stay_silent_has_no_action_or_memory_effect",
    "stable_context_and_domain_changes_emit_only_significant_events",
  ),
);
contract(
  "P05-02",
  rust(
    "debounce_cooldown_and_bounded_queue_coalesce_events",
    "insignificant_collector_ticks_do_not_call_ai",
  ),
);
contract(
  "P05-03",
  rust(
    "stale_state_or_privacy_response_is_rejected",
    "cancellation_drops_an_in_flight_http_request_and_explicit_input_preempts_it",
  ),
);
partial(
  "P05-04",
  rust(
    "dnd_and_each_privacy_gate_suppress_ai",
    "delivery_respects_only_quiet_meeting_idle_pause_and_the_single_fullscreen_setting",
  ),
  "Verify native suppression during actual quiet hours, meetings and idle periods.",
);
partial(
  "P05-05",
  rust(
    "explicit_user_action_uses_the_registry_and_background_cannot_execute_it",
    "single_fullscreen_setting_hides_both_mascot_and_chat",
  ),
  "Send an explicit command while installed-app background restrictions are active.",
);
partial(
  "P05-06",
  rust("excluded_private_idle_and_tracking_off_clear_evidence_and_suppress_ai"),
  "Revoke actual OS observation permissions and exercise real excluded/private apps.",
);
contract(
  "P05-07",
  rust("hackathon_idle_stress_has_no_db_writes_inference_or_queue_growth"),
);

partial(
  "P06-01",
  [
    ...rust("mocked_stt_create_goal_and_text_use_the_same_runtime"),
    ui(
      "src/companion/Conversation.runtime.test.tsx",
      "routes a locally transcribed message as voice, then an ordinary message as text",
    ),
    ui(
      "src/companion/Conversation.windows-voice.test.tsx",
      "uses Windows local WAV transcription and the same agent command after review",
    ),
  ],
  "Speak through real microphones in every supported installed local language on both platforms.",
);
partial(
  "P06-02",
  rust("spoken_goal_delete_shows_transcript_then_needs_separate_confirmation"),
  "Verify transcript review and confirm/cancel with actual speech.",
);
partial(
  "P06-03",
  rust(
    "spoken_step_delete_cancel_and_stale_confirmation_are_safe",
    "delete_second_then_complete_first_requires_exact_confirmation",
  ),
  "Verify actual microphone recognition before step deletion.",
);
partial(
  "P06-04",
  rust(
    "mocked_stt_create_complete_step_rename_today_and_focus",
    "voice_and_text_reject_the_same_invalid_step_without_mutating",
  ),
  "Compare a spoken ordinal and a typed ordinal in the installed app.",
);
partial(
  "P06-05",
  [
    ui(
      "src/companion/Conversation.runtime.test.tsx",
      "preserves the typed draft after a local STT failure",
    ),
    ui(
      "src/companion/Conversation.runtime.test.tsx",
      "explains microphone permission failure and permits typed commands",
    ),
    ui(
      "src/companion/Conversation.runtime.test.tsx",
      "does not start a microphone or cloud fallback if local STT is unsupported",
    ),
  ],
  "Deny real microphone permission and test unavailable local language support.",
);
partial(
  "P06-06",
  [
    ui(
      "src/companion/Conversation.runtime.test.tsx",
      "routes raw audio only to local STT and sends only the transcript to the agent",
    ),
    ...rust(
      "hackathon_hero_two_lanes_voice_park_receipt_resume_research_summary_restart",
    ),
  ],
  "Inspect synthetic traffic through the real speech adapters and live accounts.",
);

contract(
  "P07-01",
  rust("explicit_focus_wins_without_inflating_other_goal_activity"),
);
contract(
  "P07-02",
  rust("unique_user_app_rule_precedes_keywords_and_presets_do_not"),
);
partial(
  "P07-03",
  rust("six_lanes_strong_local_match_is_ready_at_thirty_seconds"),
  "Measure actual stable-window matching time on each device.",
);
partial(
  "P07-04",
  rust(
    "ambiguity_reuses_a_match_within_the_evidence_window_then_requires_a_refresh",
    "dnd_and_sharing_block_calls_even_for_sustained_ambiguity",
  ),
  "Measure actual roughly 30-second eligibility and live-provider latency.",
);
contract(
  "P07-05",
  rust(
    "short_switches_do_not_change_lane_but_sustained_switch_does",
    "similar_goals_and_unknown_context_are_never_guessed",
    "dnd_and_sharing_block_calls_even_for_sustained_ambiguity",
  ),
);
contract(
  "P07-06",
  rust("work_context_disambiguates_similar_goals_and_respects_memory_off"),
);
partial(
  "P07-07",
  rust(
    "work_state_reports_stuck_and_distraction_without_mutating_memory",
    "waiting_context_and_live_step_survive_a_lane_switch_without_tick_memories",
    "excluded_private_idle_and_tracking_off_clear_evidence_and_suppress_ai",
  ),
  "Observe all five states with actual platform activity adapters.",
);
contract(
  "P07-08",
  rust(
    "work_state_reports_stuck_and_distraction_without_mutating_memory",
    "activity_ticks_do_not_create_memory_or_copy_window_text",
  ),
);
contract(
  "P07-09",
  rust(
    "explicit_focus_wins_without_inflating_other_goal_activity",
    "relevant_time_is_attributed_to_the_current_open_step",
  ),
);

contract(
  "P08-01 P08-02",
  rust("stuck_without_consent_never_calls_tavily_or_enqueues_network_work"),
);
partial(
  "P08-03",
  rust(
    "confirmation_to_tavily_to_work_context_mocked_end_to_end",
    "correct_goal_step_and_only_relevant_memory_reach_claude",
  ),
  "Accept research using actual Claude/Tavily accounts and assess usefulness for the current step.",
);
contract(
  "P08-04",
  rust(
    "confirmation_to_tavily_to_work_context_mocked_end_to_end",
    "correct_goal_step_and_only_relevant_memory_reach_claude",
  ),
);
contract(
  "P08-05",
  rust(
    "source_validation_and_duplicate_resources_preserve_real_destinations",
    "duplicate_only_results_do_not_call_synthesis_or_attach",
  ),
);
contract(
  "P08-06",
  rust(
    "tavily_unavailable_has_no_synthesis_or_attached_result",
    "malformed_intent_and_hallucinated_source_indices_never_attach",
    "stale_goal_step_checkpoint_consent_and_replay_are_rejected",
  ),
);
contract(
  "P08-07",
  rust(
    "privacy_revoked_during_intent_cancels_before_tavily",
    "stale_goal_step_checkpoint_consent_and_replay_are_rejected",
    "consent_is_bound_to_context_before_the_worker_starts",
  ),
);
contract(
  "P08-08",
  rust(
    "research_survives_reopen_and_pending_network_requests_are_not_replayed",
  ),
);
contract(
  "P08-09",
  rust(
    "feedback_and_attachment_are_idempotent_and_use_shared_skill_validation",
    "research_survives_reopen_and_pending_network_requests_are_not_replayed",
  ),
);
partial(
  "P08-10",
  rust("chat_sources_cannot_be_invented_and_no_goal_is_changed_by_a_reply"),
  "Run existing chat research and goal analysis against live providers.",
);
contract(
  "P08-11",
  rust(
    "no_fabricated_work_from_time_or_checkpoint_or_future_data",
    "research_preserves_waiting_dependency_and_does_not_confirm_completion",
  ),
);
partial(
  "P08-12 P08-13",
  rust(
    "hackathon_hero_two_lanes_voice_park_receipt_resume_research_summary_restart",
    "unverified_or_mismatched_episode_never_writes_memory",
  ),
  "Exercise actual platform result cues, visible offers, and both accept/reject paths.",
);
partial(
  "P08-14",
  rust(
    "confirmation_is_invalidated_by_state_change_and_cannot_be_replayed",
    "unverified_or_mismatched_episode_never_writes_memory",
  ),
  "Test a real completion offer after changing focus and plan revision.",
);
contract(
  "P08-15",
  rust(
    "only_grounded_completion_with_provenance_and_reopen_handling",
    "migration_backfills_only_known_day_and_keeps_old_completion_unclassified",
    "voice_today_question_uses_confirmed_backend_history_not_elapsed_time",
  ),
);

partial(
  "P09-01",
  rust(
    "pinned_preferences_and_explicit_priority_from_stored_facts",
    "continuity_chat_uses_grounded_backend_summary_without_freeform_regeneration",
  ),
  "Check live-model routing of the priority question.",
);
contract(
  "P09-02",
  rust(
    "yesterday_uses_historical_checkpoint_even_after_today_edit",
    "invalid_checkpoint_timestamp_never_creates_fabricated_daily_history",
  ),
);
contract(
  "P09-03",
  rust("waiting_items_are_scoped_and_completed_lanes_excluded"),
);
partial(
  "P09-04",
  rust("continuity_read_skill_validates_goal_view_and_result_contract"),
  "Check live-model resolution and the actual remaining-work list for the named goal.",
);
partial(
  "P09-05",
  rust(
    "only_grounded_completion_with_provenance_and_reopen_handling",
    "no_fabricated_work_from_time_or_checkpoint_or_future_data",
    "continuity_chat_uses_grounded_backend_summary_without_freeform_regeneration",
  ),
  "Repeat the two-goal day report in the installed app with live-model phrasing.",
);
contract(
  "P09-06",
  rust(
    "restart_and_new_date_preserve_independent_lanes",
    "new_day_remembers_context_without_inventing_completion",
    "day_start_includes_recent_confirmed_results_with_actual_dates",
  ),
);
contract(
  "P09-07",
  rust(
    "yesterday_uses_historical_checkpoint_even_after_today_edit",
    "no_fabricated_work_from_time_or_checkpoint_or_future_data",
  ),
);
contract(
  "P09-08",
  rust("rejected_interventions_reduce_frequency_only_after_repeated_evidence"),
);
contract(
  "P09-09",
  rust(
    "explicit_preference_wins_over_feedback_and_other_lane_isolated",
    "explicit_global_preference_precedes_goal_inference_in_retrieval",
    "acceptance_positive_feedback_reduces_cooldown_without_cross_goal_leaks",
    "dnd_and_each_privacy_gate_suppress_ai",
  ),
);
contract("P09-10", rust("stale_inferred_memory_archived_and_pinned_preserved"));
contract(
  "P09-11",
  rust(
    "exact_duplicate_merging_preserves_pinned_and_scopes_and_evidence_counts",
  ),
);

partial(
  "P10-01",
  [
    ui(
      "src/ai/PersonalAiEngine.test.tsx",
      "stays collapsed without requests and opening reads local metadata only",
    ),
    ...rust(
      "diagnostics_are_read_only_redacted_and_hide_stale_or_disabled_memory",
    ),
  ],
  "Inspect every diagnostic field in the installed app.",
);
partial(
  "P10-02",
  [
    ui(
      "src/ai/PersonalAiEngine.test.tsx",
      "runs one manually requested real inference and distinguishes provider-reported model",
    ),
    ...rust(
      "every_skill_exposes_validator_executor_and_strict_result_contract",
    ),
  ],
  "Inspect action grouping, review labels, and model claims in the installed UI.",
);
partial(
  "P10-03",
  rust("manual_health_is_one_real_bounded_schema_request_without_user_context"),
  "Run the manual inference health check against the user's actual Claude account; the automated test uses an isolated HTTP server.",
);
contract(
  "P10-04",
  rust(
    "health_handles_timeout_invalid_envelope_and_size_limit",
    "health_single_flight_cooldown_cancellation_and_window_boundary",
    "health_sanitizes_http_errors_and_never_retries",
  ),
);
partial(
  "P10-05",
  rust(
    "health_rejects_malformed_json_schema_refusal_and_truncation",
    "health_sanitizes_http_errors_and_never_retries",
  ),
  "Validate account-specific balance, model access, and schema support against the actual provider.",
);
contract(
  "P10-06",
  rust(
    "manual_health_is_one_real_bounded_schema_request_without_user_context",
    "diagnostics_are_read_only_redacted_and_hide_stale_or_disabled_memory",
  ),
);
partial(
  "P10-07",
  rust(
    "hackathon_hero_two_lanes_voice_park_receipt_resume_research_summary_restart",
  ),
  "Repeat the complete scenario through native apps, actual microphones, and live providers on both platforms.",
);
partial(
  "P10-08",
  ["suite:frontend", "suite:rust"],
  "Walk through the installed app's existing goals, Today, chats, tracking, timers, and Progress.",
);

contract(
  "SEC-01",
  rust(
    "dnd_and_each_privacy_gate_suppress_ai",
    "individual_toggles_gate_capture_and_recall",
    "tracking_requires_both_persistent_request_and_consent_and_keeps_pause_after_goal_switch",
  ),
);
contract(
  "SEC-02",
  rust(
    "privacy_sharing_off_sends_no_personal_memory_or_checkpoint",
    "current_window_text_never_enters_checkpoint_source_or_storage",
  ),
);
contract(
  "SEC-03",
  rust(
    "expired_consent_and_mock_ai_cannot_search",
    "claude_unavailable_prevents_tavily",
  ),
);
partial(
  "SEC-04",
  rust(
    "working_memory_is_transient_and_does_not_store_window_text",
    "activity_ticks_do_not_create_memory_or_copy_window_text",
    "current_window_text_never_enters_checkpoint_source_or_storage",
  ),
  "Inspect synthetic platform tracking metadata and confirm database storage/privacy wording.",
);
contract(
  "SEC-05",
  rust(
    "scoped_retrieval_precedes_reasoning_and_excludes_other_projects_and_screen_text",
    "correct_goal_step_and_only_relevant_memory_reach_claude",
    "memory_and_utf8_context_budgets_include_goal_and_checkpoint",
  ),
);
partial(
  "SEC-06",
  rust(
    "malformed_inputs_and_secrets_are_rejected_without_storing_them",
    "health_sanitizes_http_errors_and_never_retries",
  ),
  "Inspect synthetic inputs and logs through actual platform adapters; do not claim perfect redaction.",
);
partial(
  "SEC-07",
  rust(
    "request_cancellation_newest_request_and_privacy_staleness_are_observable",
    "stale_state_or_privacy_response_is_rejected",
  ),
  "Inspect user-facing wording about data already transmitted to a provider.",
);
partial(
  "SEC-08",
  [
    ...rust(
      "edit_forget_and_clear_memory_never_delete_goals_steps_chat_or_activity",
    ),
    ui(
      "src/components/MemorySettings.test.tsx",
      "requires confirmation to forget an item and supports cancel",
    ),
  ],
  "Review wording about backups and external provider data on the installed app.",
);
manual(
  "SEC-09",
  "Review source and generated frontend assets for credentials, runtime-only key storage, and logs; do not print or collect real secrets.",
);

partial(
  "REL-01",
  ["suite:versions", "suite:release"],
  "Verify the installed version, intended source tag, real release metadata, and immutability of delivered artifacts.",
);
partial(
  "REL-02",
  ["suite:release"],
  "Inspect actual CI results and Windows/Apple Silicon release assets for the same version.",
);
manual(
  "REL-03 REL-04 REL-05 REL-07 REL-08 REL-09",
  "Use disposable native installations on Windows and Apple Silicon macOS to verify installation, updater/restart, compatibility bridges, first-launch trust guidance, failures, and user acceptance. No installer is built or installed by this runner.",
);
partial(
  "REL-06",
  ["suite:release"],
  "Verify hashes and signatures of actual downloaded release artifacts; fixture signing tests do not authenticate public downloads.",
);

contract("AUTO-01", ["suite:frontend"]);
contract("AUTO-02", ["suite:versions", "suite:typescript", "suite:bundle"]);
contract("AUTO-03", ["suite:rust"]);
contract("AUTO-04", ["suite:release"]);
manual(
  "AUTO-05",
  "Attach matching native installer-build evidence from Windows and Apple Silicon CI. A frontend bundle and local tests are not two installer builds.",
);
contract(
  "AUTO-06",
  rust(
    "batch_rolls_back_goal_creation_and_references_on_later_failure",
    "memory_and_utf8_context_budgets_include_goal_and_checkpoint",
    "stale_state_rejects_entire_plan",
    "six_parallel_contexts_are_isolated",
    "rejected_interventions_reduce_frequency_only_after_repeated_evidence",
    "restart_and_new_date_preserve_independent_lanes",
    "hackathon_idle_stress_has_no_db_writes_inference_or_queue_growth",
  ),
);
