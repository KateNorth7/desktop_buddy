mod accuracy_check;
mod activity_review;
mod agent_dialogue;
mod agent_runtime;
mod ai_chat;
mod attention;
mod automatic_goals;
mod autostart;
mod branding;
mod browser;
mod buddy;
mod chat_contract;
mod claude;
pub mod coaching;
mod collector;
mod commands;
mod companion;
mod completion_signals;
mod contact;
mod continuity;
mod conversation_check;
mod core;
mod core_capture;
mod core_import;
mod credentials;
mod decisions;
mod dialogue_benchmark;
mod dialogue_context;
mod engine_observability;
mod episode_evidence;
mod episode_review;
mod focus;
mod frontend_events;
mod goal_analysis;
mod goal_matching;
mod goal_understanding;
mod goals;
mod hero_check;
mod history;
mod http;
mod insights;
mod installed_apps;
mod lane_profiles;
mod local_voice;
#[cfg(target_os = "macos")]
mod macos;
mod memory;
mod mixed_action_check;
mod models;
mod observation;
mod observation_check;
mod onboarding_goal;
mod privacy;
mod product_feedback;
mod profile;
mod recommendations;
mod relevance;
mod research;
mod research_evidence;
mod semantic_observation;
mod shared_ai;
mod signing_trust;
mod skills;
mod startup;
mod storage;
mod tracking;
mod tracking_attempts;
mod tray;
mod unplanned_work;
mod updates;
mod vision;
mod voice_cleanup;
mod web_search;
mod window_history;
mod work_context;
mod work_journal;
mod work_state;
use tauri::Manager;

#[cfg(test)]
mod threading_tests;

// IPC commands must use #[tauri::command(async)], including synchronous Rust functions.
// A worker can hold AppState::inner while waiting for a native window getter.
// Waiting for that mutex on the event thread would deadlock the getter's reply.
// Window/tray callbacks must likewise dispatch stateful work off the event thread.

pub fn run() {
    if branding::relaunch_renamed_bundle() || branding::report_migration_check() {
        return;
    }
    #[cfg(not(test))]
    shared_ai::initialize_cli();
    if std::env::args().any(|arg| arg == "--check-semantic-observation")
        || (cfg!(debug_assertions) && http::enabled("BUDDY_SEMANTIC_NATIVE_CHECK"))
    {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(semantic_observation::check::run());
        #[cfg(all(target_os = "macos", debug_assertions))]
        if http::enabled("BUDDY_SEMANTIC_NATIVE_CHECK") {
            // A signed audit bundle can be launched normally without inheriting a CLI's TCC identity.
            // This fixed debug report contains status only, never images or installed user state.
            let report = result.as_ref().cloned().unwrap_or_else(|error| {
                let failure = if error.contains("Accessibility") {
                    "accessibility_unavailable"
                } else if error.contains("Screen Recording") {
                    "screen_recording_unavailable"
                } else if error.contains("inactive or locked") {
                    "session_inactive"
                } else if error == "Screen context changed" {
                    "native_capture_context_changed"
                } else if error == "Public fixture marker or foreground changed" {
                    "fixture_text_context_changed"
                } else if error == "Public fixture foreground changed" {
                    "fixture_capture_context_changed"
                } else {
                    engine_observability::error_category(error)
                };
                serde_json::json!({"all_passed":false,"failure":failure})
            });
            let _ = std::fs::write(
                "/private/tmp/buddy-semantic-native-launchservices.json",
                report.to_string(),
            );
        }
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!(
                    "Semantic observation check unavailable ({}). No installed data was changed.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }

    if let Some(flag) = std::env::args().find(|arg| {
        matches!(
            arg.as_str(),
            "--check-goal-matching" | "--check-coaching-voice"
        )
    }) {
        let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
            if flag == "--check-goal-matching" {
                goal_matching::check::run().await
            } else {
                coaching::chat::voice_check::run().await
            }
        });
        match result {
            Ok(complete) => std::process::exit(if complete { 0 } else { 1 }),
            Err(error) => {
                eprintln!("Accuracy check failed: {error}. No installed data was changed.");
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-hero") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(hero_check::bounded_run());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["backend_passed"] == true {
                    0
                } else {
                    1
                });
            }
            Err(error) => {
                eprintln!(
                    "Hero rehearsal unavailable ({}). No personal data was changed.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }

    if std::env::args().any(|arg| arg == "--check-mixed-actions") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(mixed_action_check::bounded_run());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!(
                    "Mixed-action check failed ({}). No user data was changed.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-observation-workflow") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observation_check::run());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!(
                    "Observation check unavailable ({}). No personal data was used.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-unplanned-work") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(unplanned_work::check::run());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!(
                    "Unplanned-work check failed ({}). No user data was changed.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-coaching") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coaching::gateway::headless_check());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!(
                    "Coaching check failed ({}). No user data was changed.",
                    engine_observability::error_category(&error)
                );
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-conversation-reliability") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(conversation_check::bounded_run());
        match result {
            Ok(report) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            Err(_) => {
                eprintln!("Conversation checks could not finish. No personal data was changed.");
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-agent-background") {
        let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(180),
                agent_runtime::headless_background_check(),
            )
            .await
        });
        match result {
            Ok(Ok(report)) => {
                println!("{report}");
                std::process::exit(if report["all_passed"] == true { 0 } else { 1 });
            }
            _ => {
                eprintln!("Background checks could not finish. No personal data was changed.");
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-voice-drafts") {
        match tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(voice_cleanup::live_check())
        {
            Ok(report) => {
                println!("{}", report);
                std::process::exit(if report["passed"] == report["total"] {
                    0
                } else {
                    1
                });
            }
            Err(error) => {
                eprintln!("Voice draft check failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if std::env::args().any(|arg| arg == "--check-agent-dialogue") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(dialogue_benchmark::bounded_run());
        match result {
            Ok(report) => {
                println!("{}", report);
                std::process::exit(
                    if report["mechanical_accuracy"].as_f64().unwrap_or(0.) >= 0.95 {
                        0
                    } else {
                        1
                    },
                );
            }
            Err(_) => {
                eprintln!("Dialogue evaluation could not finish. No personal data was changed.");
                std::process::exit(1);
            }
        }
    }

    // Explicit developer CLI check: synthetic context only; no UI or production database.
    if std::env::args().any(|arg| arg == "--check-agent-actions") {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(engine_observability::headless_action_check());
        match result {
            Ok(checks) => {
                let passed =
                    checks.len() == 9 && checks.iter().all(|check| check.status == "passed");
                println!("{}", serde_json::to_string(&checks).unwrap());
                std::process::exit(if passed { 0 } else { 1 });
            }
            Err(_) => {
                eprintln!("Live agent check could not finish. No user data was changed.");
                std::process::exit(1);
            }
        }
    }
    // Development convenience only. Release uses saved credentials or environment fallback.
    #[cfg(debug_assertions)]
    {
        let _ =
            dotenvy::from_path(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env"));
    }
    #[cfg(debug_assertions)]
    let layout_rehearsal = buddy::layout_check::enabled();
    #[cfg(not(debug_assertions))]
    let layout_rehearsal = false;
    let builder = tauri::Builder::default();
    let builder = if layout_rehearsal {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let app = app.clone();
            // Opening a workspace takes application state; leave the native event thread free.
            tauri::async_runtime::spawn(async move {
                let _ = buddy::open_workspace(app);
            });
        }))
    };
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let dir = if http::enabled("DEMO_MODE") && http::enabled("AI_MOCK") {
                std::env::var_os("BUDDY_TEST_DATA_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or(dir)
            } else {
                dir
            };
            std::fs::create_dir_all(&dir)?;
            let database = if http::enabled("DEMO_MODE") {
                "buddy-demo.db"
            } else {
                "buddy.db"
            };
            let storage =
                storage::Storage::open(&dir.join(database)).map_err(std::io::Error::other)?;
            #[cfg(debug_assertions)]
            let storage = {
                let mut storage = storage;
                if buddy::layout_check::enabled() {
                    buddy::layout_check::seed(&mut storage).map_err(std::io::Error::other)?;
                }
                storage
            };
            // The production database remains initialized through the same startup path.
            startup::initialize(app, storage)?;
            #[cfg(target_os = "macos")]
            for label in ["mascot", "buddy"] {
                if let Some(window) = app.get_webview_window(label) {
                    macos::pin_window(&window);
                }
            }
            core_capture::install(app.handle())?;
            tray::install(app)?;
            #[cfg(windows)]
            companion::windows::install_selection_shortcut(app.handle().clone());
            #[cfg(target_os = "macos")]
            macos::install_shortcuts(app.handle());
            let settings = app
                .state::<commands::AppState>()
                .inner
                .lock()
                .map_err(|_| std::io::Error::other("Application state unavailable"))?
                .storage
                .user_settings()
                .map_err(std::io::Error::other)?;
            // Isolated UI validation must not rewrite the installed app's startup entry.
            let isolated_test = cfg!(debug_assertions)
                && http::enabled("DEMO_MODE")
                && http::enabled("AI_MOCK")
                && std::env::var_os("BUDDY_TEST_DATA_DIR").is_some();
            if settings.onboarding.completed && !isolated_test {
                if let Err(error) = autostart::sync(settings.autostart) {
                    if let Ok(mut inner) = app.state::<commands::AppState>().inner.lock() {
                        inner.last_error = Some(error);
                    }
                }
            }
            profile::apply_theme(app.handle(), &settings).map_err(std::io::Error::other)?;
            if std::env::args().any(|arg| arg == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
            }
            #[cfg(debug_assertions)]
            if buddy::layout_check::enabled() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(buddy::layout_check::run(handle));
                return Ok(());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(3));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticker.tick().await;
                    agent_runtime::tick(&handle).await;
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            let Some(intent) = event.id().as_ref().strip_prefix("companion-") else {
                return;
            };
            let app = app.clone();
            let intent = intent.to_string();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<commands::AppState>();
                let result = match intent.as_str() {
                    "hide" => buddy::show_desktop_buddy(false, app.clone(), state),
                    "open" => buddy::open_workspace(app.clone()),
                    "goal" => core_capture::capture_companion_goal(app.clone(), state),
                    _ => companion::open_companion_chat(intent, app.clone(), state),
                };
                if let Err(error) = result {
                    if let Ok(mut inner) = app.state::<commands::AppState>().inner.lock() {
                        inner.companion.view.notice = Some(error);
                    }
                }
            });
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::ThemeChanged(theme) = event {
                    let _ = profile::update_theme_icons(window.app_handle(), *theme);
                }
            }
            if window.label() == "main" {
                if matches!(event, tauri::WindowEvent::Focused(true)) {
                    let state = window.app_handle().state::<commands::AppState>();
                    if let Ok(mut inner) = state.inner.try_lock() {
                        inner.companion.planning_date =
                            chrono::Local::now().date_naive().to_string();
                    };
                }
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            if window.label() == "mascot" && matches!(event, tauri::WindowEvent::Moved(_)) {
                #[cfg(debug_assertions)]
                if buddy::layout_check::enabled() {
                    return;
                }
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(mut inner) = app.state::<commands::AppState>().inner.lock() {
                        let _ = buddy::sync(&app, &mut inner);
                    };
                });
            }
            if window.label() == "mascot" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = buddy::show_desktop_buddy(
                            false,
                            app.clone(),
                            app.state::<commands::AppState>(),
                        );
                    });
                }
            }
            if window.label() == "buddy" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = companion::close_companion_chat(
                            app.clone(),
                            app.state::<commands::AppState>(),
                        );
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            activity_review::get_activity_review,
            activity_review::review_activity,
            activity_review::get_activity_evidence,
            activity_review::set_activity_review_context,
            commands::get_accessibility_permission,
            tracking::set_goal_monitoring,
            vision::screenshot_permission,
            work_journal::get_work_journal,
            work_journal::undo_work_event,
            unplanned_work::get_unplanned_work_suggestion,
            unplanned_work::respond_unplanned_work_suggestion,
            commands::request_accessibility_permission,
            commands::reset_accessibility_permission,
            companion::get_companion_view,
            companion::get_companion_goal_context,
            companion::set_companion_preferences,
            companion::open_companion_chat,
            companion::close_companion_chat,
            companion::open_companion_context,
            companion::companion_voice_input,
            goal_analysis::get_ai_preferences,
            goal_analysis::set_ai_preferences,
            goal_analysis::analyze_core_goal,
            goal_analysis::complete_goal_step,
            goal_analysis::complete_goal_title,
            automatic_goals::retry_core_goal_analysis,
            automatic_goals::select_core_goal,
            goal_matching::suggestions::get_goal_match_suggestion,
            goal_matching::suggestions::respond_goal_match_suggestion,
            relevance::get_goal_progress,
            ai_chat::get_buddy_chat_history,
            ai_chat::get_buddy_conversations,
            ai_chat::search_buddy_conversations,
            ai_chat::create_buddy_conversation,
            ai_chat::select_buddy_conversation,
            ai_chat::delete_buddy_conversation,
            ai_chat::clear_buddy_chat,
            ai_chat::send_buddy_message,
            ai_chat::record_resource_view,
            ai_chat::set_goal_expected_minutes,
            ai_chat::get_buddy_coaching,
            coaching::gateway::prepare_coaching,
            coaching::gateway::get_coaching_session,
            coaching::gateway::start_coaching_session,
            coaching::gateway::reply_coaching_session,
            coaching::gateway::propose_coaching_action,
            history::get_activity_segments,
            window_history::get_window_activity,
            companion::companion_submit,
            companion::respond_companion_intervention,
            companion::update_companion_inbox,
            core::get_core_snapshot,
            core::visit_core_progress,
            core_capture::get_core_drafts,
            core_capture::resolve_core_draft,
            core::add_core_goals,
            core::create_core_goal,
            core::save_core_goal,
            core::set_core_goal_priority,
            core::set_core_goal_estimate,
            ai_chat::delete_core_goal,
            ai_chat::restore_core_goal,
            core::transition_core_goal,
            core::set_core_today,
            core::reorder_core_goals,
            core::plan_core_day,
            core::plan_core_goals,
            core::dismiss_core_carryover,
            core::set_core_timer,
            core::tick_core_timer,
            core::set_core_preferences,
            core::finish_core_setup,
            core::save_core_identity,
            core_import::propose_core_import,
            core_import::transcribe_core_voice,
            core_import::get_local_voice_languages,
            local_voice::prepare_local_voice,
            voice_cleanup::correct_voice_draft,
            updates::check_app_update,
            updates::install_app_update,
            profile::save_onboarding,
            profile::create_onboarding_goal,
            profile::finish_onboarding,
            profile::save_user_profile,
            profile::set_avatar_color,
            profile::set_user_theme,
            profile::reset_local_profile,
            installed_apps::get_installed_apps,
            contact::open_feedback_draft,
            contact::get_contact_endpoint,
            contact::set_contact_endpoint,
            contact::send_contact_feedback,
            contact::subscribe_contact_email,
            contact::share_role_profile,
            buddy::windows::mascot_frame_ready,
            buddy::windows::buddy_panel_ready,
            buddy::windows::measure_buddy_panel,
            buddy::show_desktop_buddy,
            product_feedback::save_product_feedback,
            tracking::get_tracking_settings,
            tracking::set_tracking_settings,
            history::get_goal_history,
            insights::set_app_rule,
            goals::save_goal_plan,
            goals::transition_goal,
            goals::refine_goal,
            commands::get_dashboard,
            work_context::run_work_context_action,
            memory::controls::get_memory_overview,
            memory::controls::set_memory_controls,
            memory::controls::clear_personal_memory,
            memory::controls::forget_work_context,
            memory::controls::pin_work_context,
            memory::controls::resolve_memory_proposal,
            memory::retriever::get_memory_retrieval_debug,
            memory::create_memory,
            memory::retrieve_memory,
            memory::list_memories,
            memory::update_memory,
            memory::forget_memory,
            memory::archive_memory,
            memory::pin_memory,
            memory::clear_memory_scope,
            memory::retrieve_work_context,
            memory::save_work_context,
            memory::get_working_memory,
            buddy::set_buddy_preferences,
            buddy::open_workspace,
            buddy::reset_buddy_position,
            buddy::rate_recommendation,
            buddy::quit_app,
            privacy::get_privacy_preview,
            privacy::set_retention,
            privacy::clear_local_history,
            privacy::test_provider_connection,
            engine_observability::get_personal_ai_engine,
            engine_observability::get_ai_service_status,
            engine_observability::test_claude_inference,
            engine_observability::test_agent_actions,
            commands::set_provider_key,
            commands::set_goal,
            commands::get_current_goal,
            commands::get_recent_activity,
            commands::get_activity_snapshot,
            commands::set_tracking,
            commands::set_ai_enabled,
            commands::set_dnd,
            commands::analyze_focus,
            commands::search_web,
            commands::save_feedback,
            commands::dismiss_buddy,
            commands::capture_screenshot_on_demand
        ])
        .build(tauri::generate_context!())
        .expect("sumi could not start")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                // The floating mascot can remain visible after the workspace closes.
                // Every Dock reopen must restore the workspace, regardless of that window.
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = buddy::open_workspace(app.clone()) {
                        if let Ok(mut inner) = app.state::<commands::AppState>().inner.lock() {
                            inner.last_error = Some(error);
                        }
                    }
                });
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
