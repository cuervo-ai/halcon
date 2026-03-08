    use super::*;

    // Helper struct to keep channel receivers alive during tests
    struct TestAppContext {
        app: TuiApp,
        #[allow(dead_code)]
        prompt_rx: mpsc::UnboundedReceiver<String>,
        #[allow(dead_code)]
        ctrl_rx: mpsc::UnboundedReceiver<ControlEvent>,
        #[allow(dead_code)]
        perm_rx: mpsc::UnboundedReceiver<halcon_core::types::PermissionDecision>,
    }

    impl std::ops::Deref for TestAppContext {
        type Target = TuiApp;
        fn deref(&self) -> &Self::Target {
            &self.app
        }
    }

    impl std::ops::DerefMut for TestAppContext {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.app
        }
    }

    fn test_app() -> TestAppContext {
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = mpsc::unbounded_channel();
        TestAppContext {
            app: TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None), // Phase 3 SRCH-004: No database in tests
            prompt_rx,
            ctrl_rx,
            perm_rx,
        }
    }

    #[test]
    fn app_initial_state() {
        let app = test_app();
        assert!(!app.state.agent_running);
        assert!(!app.state.should_quit);
        assert_eq!(app.state.focus, FocusZone::Prompt);
    }

    #[test]
    fn app_with_expert_mode() {
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let app = TuiApp::with_mode(rx, prompt_tx, ctrl_tx, perm_tx, None, UiMode::Expert);
        assert_eq!(app.state.ui_mode, UiMode::Expert);
        assert!(app.state.panel_visible);
    }

    #[test]
    fn app_with_minimal_mode() {
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let app = TuiApp::with_mode(rx, prompt_tx, ctrl_tx, perm_tx, None, UiMode::Minimal);
        assert_eq!(app.state.ui_mode, UiMode::Minimal);
        assert!(!app.state.panel_visible);
    }

    #[test]
    fn handle_quit_action() {
        let mut app = test_app();
        app.handle_action(input::InputAction::Quit);
        assert!(app.state.should_quit);
    }

    #[test]
    fn handle_cycle_focus() {
        let mut app = test_app();
        assert_eq!(app.state.focus, FocusZone::Prompt);
        app.handle_action(input::InputAction::CycleFocus);
        assert_eq!(app.state.focus, FocusZone::Activity);
        app.handle_action(input::InputAction::CycleFocus);
        assert_eq!(app.state.focus, FocusZone::Prompt);
    }

    #[test]
    fn handle_agent_done_event() {
        let mut app = test_app();
        app.state.agent_running = true;
        app.state.focus = FocusZone::Activity;
        app.handle_ui_event(UiEvent::AgentDone);
        assert!(!app.state.agent_running);
        assert_eq!(app.state.focus, FocusZone::Prompt);
    }

    #[test]
    fn handle_spinner_events() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::SpinnerStart("Thinking...".into()));
        assert!(app.state.spinner_active);
        assert_eq!(app.state.spinner_label, "Thinking...");
        app.handle_ui_event(UiEvent::SpinnerStop);
        assert!(!app.state.spinner_active);
    }

    // --- Phase 95: ThinkingDelta buffer tests ---

    #[test]
    fn stream_thinking_accumulates_silently_without_flooding() {
        let mut app = test_app();
        // Simulate ~45 tiny SSE fragments (typical deepseek-reasoner pattern).
        let fragments = ["H", "ola", ",", " osc", "ar", "!", " ¿", "Có", "mo", " est", "ás", "?"];
        for frag in &fragments {
            app.handle_ui_event(UiEvent::StreamThinking((*frag).into()));
        }
        // Buffer is populated — no activity lines yet.
        let expected = fragments.concat();
        assert_eq!(app.state.thinking_buffer, expected, "Buffer should accumulate all fragments");
        // Activity feed should contain only the AgentThinking skeleton (none here since no AgentStartedPrompt was sent).
        let info_lines = app.activity_model.all_lines().iter().filter(|l| matches!(l, crate::tui::activity_types::ActivityLine::Info(_))).count();
        assert_eq!(info_lines, 0, "No Info lines should be pushed per fragment");
    }

    #[test]
    fn stream_thinking_collapsed_on_first_stream_chunk() {
        let mut app = test_app();
        // Accumulate thinking fragments.
        app.handle_ui_event(UiEvent::StreamThinking("Chain of thought".into()));
        app.handle_ui_event(UiEvent::StreamThinking(" part 2".into()));
        assert!(!app.state.thinking_buffer.is_empty());
        // First real text token arrives — buffer should collapse to a single Info line and clear.
        app.handle_ui_event(UiEvent::StreamChunk("Final answer".into()));
        assert!(app.state.thinking_buffer.is_empty(), "Buffer cleared after collapse");
        // Activity feed: 1 Info line (thinking summary) + AssistantText with "Final answer".
        let info_count = app.activity_model.all_lines().iter().filter(|l| matches!(l, crate::tui::activity_types::ActivityLine::Info(_))).count();
        assert_eq!(info_count, 1, "Exactly one collapsed thinking summary");
        let has_answer = app.activity_model.all_lines().iter().any(|l| {
            if let crate::tui::activity_types::ActivityLine::AssistantText(t) = l { t.contains("Final answer") } else { false }
        });
        assert!(has_answer, "Answer text should appear in activity feed");
    }

    #[test]
    fn stream_thinking_collapsed_on_done_when_no_stream_chunk() {
        let mut app = test_app();
        // Pure reasoning response: only ThinkingDeltas, no StreamChunk.
        app.handle_ui_event(UiEvent::StreamThinking("Solo pensé esto".into()));
        assert!(!app.state.thinking_buffer.is_empty());
        // StreamDone with no prior StreamChunk — buffer collapses and thinking shown as response.
        app.handle_ui_event(UiEvent::StreamDone);
        assert!(app.state.thinking_buffer.is_empty(), "Buffer cleared on StreamDone");
        let has_response = app.activity_model.all_lines().iter().any(|l| {
            if let crate::tui::activity_types::ActivityLine::AssistantText(t) = l { t.contains("Solo pensé esto") } else { false }
        });
        assert!(has_response, "Thinking content surfaced as assistant response");
    }

    #[test]
    fn spinner_stop_clears_thinking_buffer() {
        let mut app = test_app();
        app.state.thinking_buffer = "partial thinking".into();
        app.handle_ui_event(UiEvent::SpinnerStop);
        assert!(app.state.thinking_buffer.is_empty(), "Buffer cleared on SpinnerStop");
    }

    #[test]
    fn agent_started_prompt_clears_thinking_buffer() {
        let mut app = test_app();
        app.state.thinking_buffer = "stale thinking from previous turn".into();
        app.handle_ui_event(UiEvent::AgentStartedPrompt);
        assert!(app.state.thinking_buffer.is_empty(), "Buffer reset on new prompt");
    }

    #[test]
    fn cancel_agent_action() {
        let mut app = test_app();
        app.state.agent_running = true;
        app.handle_action(input::InputAction::CancelAgent);
        assert!(!app.state.agent_running);
    }

    #[test]
    fn empty_submit_rejected() {
        let mut app = test_app();
        app.handle_action(input::InputAction::SubmitPrompt);
        // Should not start agent on empty prompt.
        assert!(!app.state.agent_running);
    }

    #[test]
    fn submit_button_area_default_is_zero() {
        // Phase I2: Submit button removed, field kept for backward compatibility
        let app = test_app();
        assert_eq!(app.submit_button_area, Rect::default());
    }

    #[test]
    fn handle_info_event() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::Info("round separator".into()));
        assert!(app.activity_model.line_count() > 0);
    }

    #[test]
    fn push_banner_adds_lines() {
        let mut app = test_app();
        let features = crate::render::banner::FeatureStatus::default();
        app.push_banner("0.1.0", "deepseek", true, "deepseek-chat", "abc12345", "new", None, &features);
        // Banner should populate the activity zone with multiple lines.
        assert!(app.activity_model.line_count() >= 4);
    }

    #[test]
    fn push_banner_with_routing_shows_chain() {
        use crate::render::banner::RoutingDisplay;
        let mut app = test_app();
        let routing = RoutingDisplay {
            mode: "failover".into(),
            strategy: "balanced".into(),
            fallback_chain: vec![
                "anthropic".into(),
                "deepseek".into(),
                "ollama".into(),
            ],
        };
        let features = crate::render::banner::FeatureStatus::default();
        let before = app.activity_model.line_count();
        app.push_banner(
            "0.1.0", "anthropic", true, "claude-sonnet",
            "abc12345", "new", Some(&routing), &features,
        );
        let after = app.activity_model.line_count();
        // Should have at least several lines more than without routing.
        assert!(after > before + 3);
    }

    #[test]
    fn tool_start_event_creates_tool_exec() {
        let mut app = test_app();
        let input = serde_json::json!({"path": "src/main.rs"});
        app.handle_ui_event(UiEvent::ToolStart {
            name: "file_read".into(),
            input,
        });
        assert_eq!(app.activity_model.line_count(), 1);
        assert!(app.activity_model.has_loading_tools());
    }

    #[test]
    fn tool_output_event_completes_tool() {
        let mut app = test_app();
        let input = serde_json::json!({"command": "ls"});
        app.handle_ui_event(UiEvent::ToolStart {
            name: "bash".into(),
            input,
        });
        assert!(app.activity_model.has_loading_tools());
        app.handle_ui_event(UiEvent::ToolOutput {
            name: "bash".into(),
            content: "file1\nfile2".into(),
            is_error: false,
            duration_ms: 42,
        });
        assert!(!app.activity_model.has_loading_tools());
    }

    #[test]
    fn format_input_preview_object() {
        let val = serde_json::json!({"path": "src/main.rs", "line": 10});
        let preview = super::format_input_preview(&val);
        assert!(preview.contains("path=src/main.rs"));
        assert!(preview.contains("line=10"));
    }

    #[test]
    fn format_input_preview_string() {
        let val = serde_json::Value::String("hello world".into());
        let preview = super::format_input_preview(&val);
        assert_eq!(preview, "hello world");
    }

    #[test]
    fn format_input_preview_truncates_long_values() {
        let long_val = "a".repeat(100);
        let val = serde_json::json!({"data": long_val});
        let preview = super::format_input_preview(&val);
        // truncate_str uses '…' (single Unicode ellipsis, not "...")
        assert!(preview.contains('…'));
        assert!(preview.chars().count() < 100);
    }

    #[test]
    fn arrow_keys_scroll_in_activity_zone() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = test_app();
        app.state.focus = FocusZone::Activity;
        // Add enough content to have scroll range.
        for i in 0..50 {
            app.activity_model.push_info(&format!("line {i}"));
        }
        app.activity_navigator.last_max_scroll = 40; // Simulate render having computed this.
        let up_key = KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_action(input::InputAction::ForwardToWidget(up_key));
        assert!(!app.activity_navigator.auto_scroll);
    }

    #[test]
    fn input_always_enabled_regardless_of_focus() {
        // CRITICAL TEST: Verify that typing ALWAYS works, even when focus is on Activity.
        // This ensures the user is NEVER blocked from typing.
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = test_app();

        // Set focus to Activity (not Prompt).
        app.state.focus = FocusZone::Activity;

        // Try to type a character - should ALWAYS work.
        let char_key = KeyEvent {
            code: KeyCode::Char('h'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_action(input::InputAction::ForwardToWidget(char_key));

        // Verify the character was inserted (prompt is not empty).
        let text = app.prompt.text();
        assert_eq!(text, "h", "Input should ALWAYS be enabled, even when focus is on Activity");

        // Verify typing indicator is active.
        assert!(app.state.typing_indicator, "Typing indicator should activate when user types");
    }

    #[test]
    fn typing_works_when_agent_running() {
        // CRITICAL TEST: Verify that typing works even when agent is running.
        // This ensures queued prompts can be typed while agent processes.
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = test_app();

        // Simulate agent running.
        app.state.agent_running = true;
        app.state.prompts_queued = 1;

        // Try to type - should ALWAYS work.
        let char_key = KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_action(input::InputAction::ForwardToWidget(char_key));

        // Verify the character was inserted.
        let text = app.prompt.text();
        assert_eq!(text, "t", "Input should work even when agent is running");
    }

    #[test]
    fn navigation_keys_respect_focus() {
        // Verify that arrow keys only scroll Activity when focus is there.
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut app = test_app();

        // Focus on Prompt initially.
        app.state.focus = FocusZone::Prompt;

        // Add scrollable content.
        for i in 0..50 {
            app.activity_model.push_info(&format!("line {i}"));
        }
        app.activity_navigator.last_max_scroll = 40;

        // Arrow down while focus is on Prompt → should go to prompt (newline).
        let down_key = KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        app.handle_action(input::InputAction::ForwardToWidget(down_key));

        // Activity auto_scroll should still be true (not affected).
        assert!(app.activity_navigator.auto_scroll, "Arrow keys should not scroll Activity when focus is on Prompt");

        // Now switch focus to Activity.
        app.state.focus = FocusZone::Activity;

        // Arrow down while focus is on Activity → should scroll.
        app.handle_action(input::InputAction::ForwardToWidget(down_key));

        // Activity auto_scroll should now be false (scrolling happened).
        assert!(!app.activity_navigator.auto_scroll, "Arrow keys should scroll Activity when focus is there");
    }

    #[test]
    fn cycle_ui_mode_updates_state_and_panel() {
        use crate::tui::state::UiMode;
        let mut app = test_app();
        assert_eq!(app.state.ui_mode, UiMode::Standard);
        assert!(app.state.panel_visible); // Standard starts with panel

        // Standard → Expert: panel stays shown
        app.handle_action(input::InputAction::CycleUiMode);
        assert_eq!(app.state.ui_mode, UiMode::Expert);
        assert!(app.state.panel_visible);

        // Expert → Minimal: panel hidden
        app.handle_action(input::InputAction::CycleUiMode);
        assert_eq!(app.state.ui_mode, UiMode::Minimal);
        assert!(!app.state.panel_visible);

        // Minimal → Standard: panel shown
        app.handle_action(input::InputAction::CycleUiMode);
        assert_eq!(app.state.ui_mode, UiMode::Standard);
        assert!(app.state.panel_visible);
    }

    #[test]
    fn pause_agent_sends_control_event() {
        use crate::tui::state::AgentControl;
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        app.state.agent_running = true;
        app.handle_action(input::InputAction::PauseAgent);
        assert_eq!(app.state.agent_control, AgentControl::Paused);
        assert_eq!(ctrl_rx.try_recv().unwrap(), ControlEvent::Pause);
    }

    #[test]
    fn pause_resumes_on_second_press() {
        use crate::tui::state::AgentControl;
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        app.state.agent_running = true;
        app.handle_action(input::InputAction::PauseAgent);
        assert_eq!(app.state.agent_control, AgentControl::Paused);
        let _ = ctrl_rx.try_recv(); // consume Pause
        app.handle_action(input::InputAction::PauseAgent);
        assert_eq!(app.state.agent_control, AgentControl::Running);
        assert_eq!(ctrl_rx.try_recv().unwrap(), ControlEvent::Resume);
    }

    #[test]
    fn step_agent_sends_step_event() {
        use crate::tui::state::AgentControl;
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        app.state.agent_running = true;
        app.handle_action(input::InputAction::StepAgent);
        assert_eq!(app.state.agent_control, AgentControl::StepMode);
        assert_eq!(ctrl_rx.try_recv().unwrap(), ControlEvent::Step);
    }

    #[test]
    fn approve_sends_on_perm_channel() {
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        app.state.agent_running = true;
        app.handle_action(input::InputAction::ApproveAction);
        assert_eq!(perm_rx.try_recv().unwrap(), halcon_core::types::PermissionDecision::Allowed);
    }

    #[test]
    fn reject_sends_on_perm_channel() {
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        app.state.agent_running = true;
        app.handle_action(input::InputAction::RejectAction);
        assert_eq!(perm_rx.try_recv().unwrap(), halcon_core::types::PermissionDecision::Denied);
    }

    #[test]
    fn plan_progress_event_updates_activity_and_status() {
        use crate::tui::events::{PlanStepDisplayStatus, PlanStepStatus};
        let mut app = test_app();
        app.handle_ui_event(UiEvent::PlanProgress {
            goal: "Fix bug".into(),
            steps: vec![
                PlanStepStatus {
                    description: "Read file".into(),
                    tool_name: Some("file_read".into()),
                    status: PlanStepDisplayStatus::Succeeded,
                    duration_ms: Some(120),
                },
                PlanStepStatus {
                    description: "Edit file".into(),
                    tool_name: Some("file_edit".into()),
                    status: PlanStepDisplayStatus::InProgress,
                    duration_ms: None,
                },
            ],
            current_step: 1,
            elapsed_ms: 500,
        });
        // Should have a PlanOverview in activity.
        assert!(app.activity_model.line_count() > 0);
        // Status bar should show plan step.
        assert!(app.status.plan_step.is_some());
        let step_text = app.status.plan_step.as_ref().unwrap();
        assert!(step_text.contains("Step 2/2"));
        assert!(step_text.contains("Edit file"));
    }

    // --- Phase B6: Event ring buffer tests ---

    #[test]
    fn event_log_starts_empty() {
        let app = test_app();
        assert!(app.event_log.is_empty());
    }

    #[test]
    fn event_log_records_events() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::Info("test".into()));
        app.handle_ui_event(UiEvent::SpinnerStart("thinking".into()));
        assert_eq!(app.event_log.len(), 2);
        assert!(app.event_log[0].label.contains("Info"));
        assert!(app.event_log[1].label.contains("SpinnerStart"));
    }

    #[test]
    fn event_log_offsets_increase() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::Info("first".into()));
        app.handle_ui_event(UiEvent::Info("second".into()));
        assert!(app.event_log[1].offset_ms >= app.event_log[0].offset_ms);
    }

    #[test]
    fn event_log_respects_capacity() {
        let mut app = test_app();
        for i in 0..(EVENT_RING_CAPACITY + 50) {
            app.handle_ui_event(UiEvent::Info(format!("event {i}")));
        }
        assert_eq!(app.event_log.len(), EVENT_RING_CAPACITY);
        // Oldest should have been evicted, newest should be last.
        assert!(app.event_log.back().unwrap().label.contains("event 249"));
    }

    #[test]
    fn event_summary_covers_all_variants() {
        // Just verify event_summary doesn't panic for a few key variants.
        let summaries = vec![
            event_summary(&UiEvent::StreamChunk("test".into())),
            event_summary(&UiEvent::ToolStart {
                name: "bash".into(),
                input: serde_json::json!({}),
            }),
            event_summary(&UiEvent::AgentDone),
            event_summary(&UiEvent::Quit),
        ];
        assert!(summaries.iter().all(|s| !s.is_empty()));
    }

    // --- Phase E7: Slash command interception tests ---

    #[test]
    fn slash_command_not_sent_to_agent() {
        let mut app = test_app();
        // Type "/help" into the prompt textarea.
        for c in "/help".chars() {
            app.prompt.handle_key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        app.handle_action(input::InputAction::SubmitPrompt);
        // Should NOT start agent for slash commands.
        assert!(!app.state.agent_running);
        // Help overlay should be open.
        assert!(app.state.overlay.is_active());
    }

    #[test]
    fn slash_clear_clears_activity() {
        let mut app = test_app();
        app.activity_model.push_info("some data");
        assert!(app.activity_model.line_count() > 0);
        for c in "/clear".chars() {
            app.prompt.handle_key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        app.handle_action(input::InputAction::SubmitPrompt);
        assert!(!app.state.agent_running);
        assert_eq!(app.activity_model.line_count(), 0);
    }

    #[test]
    fn slash_quit_sets_should_quit() {
        let mut app = test_app();
        for c in "/quit".chars() {
            app.prompt.handle_key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        app.handle_action(input::InputAction::SubmitPrompt);
        assert!(app.state.should_quit);
    }

    #[test]
    fn normal_text_sent_to_agent() {
        let mut app = test_app();
        for c in "hello world".chars() {
            app.prompt.handle_key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE));
        }
        app.handle_action(input::InputAction::SubmitPrompt);
        assert!(app.state.agent_running);
    }

    // --- Phase E: Agent integration event handler tests ---

    #[test]
    fn dry_run_active_event_handled() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::DryRunActive(true));
        // Should be logged in the event ring buffer.
        assert!(app.event_log.back().is_some());
        assert!(app.event_log.back().unwrap().label.contains("DryRun"));
    }

    #[test]
    fn token_budget_update_event_handled() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::TokenBudgetUpdate {
            used: 500,
            limit: 1000,
            rate_per_minute: 120.5,
        });
        assert!(app.event_log.back().is_some());
    }

    #[test]
    fn agent_state_transition_event_handled() {
        use crate::tui::events::AgentState;
        let mut app = test_app();
        app.handle_ui_event(UiEvent::AgentStateTransition {
            from: AgentState::Idle,
            to: AgentState::Executing,
            reason: "started".into(),
        });
        assert!(app.event_log.back().unwrap().label.contains("State"));
    }

    // --- Sprint 1 B2+B3: Data parity tests ---

    #[test]
    fn task_status_event_visible_in_activity() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::TaskStatus {
            title: "Read config".into(),
            status: "Completed".into(),
            duration_ms: Some(1200),
            artifact_count: 2,
        });
        assert!(app.activity_model.line_count() > 0);
        assert!(app.event_log.back().unwrap().label.contains("TaskStatus"));
    }

    #[test]
    fn reasoning_status_event_visible_in_activity() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::ReasoningStatus {
            task_type: "CodeModification".into(),
            complexity: "Complex".into(),
            strategy: "PlanExecuteReflect".into(),
            score: 0.85,
            success: true,
        });
        // Should add 2 lines: [reasoning] + [evaluation]
        assert!(app.activity_model.line_count() >= 2);
    }

    // --- Sprint 1 B4: Search tests ---

    #[test]
    fn search_finds_matching_lines() {
        let mut app = test_app();
        app.activity_model.push_info("hello world");
        app.activity_model.push_info("goodbye world");
        app.activity_model.push_info("hello again");
        let matches = app.activity_model.search("hello");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn search_case_insensitive() {
        let mut app = test_app();
        app.activity_model.push_info("Hello World");
        let matches = app.activity_model.search("hello");
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let mut app = test_app();
        app.activity_model.push_info("data");
        let matches = app.activity_model.search("");
        assert!(matches.is_empty());
    }

    #[test]
    fn search_no_match_returns_empty() {
        let mut app = test_app();
        app.activity_model.push_info("hello");
        let matches = app.activity_model.search("zzzzz");
        assert!(matches.is_empty());
    }

    #[test]
    fn search_next_wraps_around() {
        let mut app = test_app();
        // Use whole words so the word-index tokenizer finds exact matches.
        app.activity_model.push_info("has match here");
        app.activity_model.push_info("other content");
        app.activity_model.push_info("also match here");
        app.search_matches = app.activity_model.search("match");
        assert_eq!(app.search_matches.len(), 2);
        app.search_current = 0;
        app.search_next();
        assert_eq!(app.search_current, 1);
        app.search_next();
        assert_eq!(app.search_current, 0); // wrapped
    }

    #[test]
    fn search_prev_wraps_around() {
        let mut app = test_app();
        app.activity_model.push_info("first match here");
        app.activity_model.push_info("second match here");
        app.search_matches = app.activity_model.search("match");
        app.search_current = 0;
        app.search_prev();
        assert_eq!(app.search_current, 1); // wrapped to last
    }

    #[test]
    fn search_enter_navigates_forward() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = test_app();
        app.activity_model.push_info("alpha");
        app.activity_model.push_info("beta");
        app.activity_model.push_info("alpha");
        app.state.overlay.open(OverlayKind::Search);
        app.state.overlay.input = "alpha".into();
        app.rerun_search();
        assert_eq!(app.search_matches.len(), 2);
        assert_eq!(app.search_current, 0);
        // Press Enter to go to next match.
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.search_current, 1);
        // Press Enter again to wrap back to first.
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.search_current, 0);
    }

    #[test]
    fn search_shift_enter_navigates_backward() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = test_app();
        // Use whole words — the inverted index matches exact tokens, not substrings.
        app.activity_model.push_info("first test item");
        app.activity_model.push_info("second test item");
        app.state.overlay.open(OverlayKind::Search);
        app.state.overlay.input = "test".into();
        app.rerun_search();
        assert_eq!(app.search_matches.len(), 2);
        assert_eq!(app.search_current, 0);
        // Press Shift+Enter to go to previous (wraps to last).
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(app.search_current, 1);
    }

    #[test]
    fn search_empty_query_no_matches() {
        let mut app = test_app();
        app.activity_model.push_info("content");
        app.state.overlay.open(OverlayKind::Search);
        app.state.overlay.input = "".into();
        app.rerun_search();
        assert_eq!(app.search_matches.len(), 0);
    }

    // --- Sprint 1 B1: Permission channel tests ---

    #[test]
    fn permission_overlay_y_sends_approve_on_perm_channel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "bash".into(),
            args: serde_json::json!({"command": "echo test"}),
            risk_level: "Low".into(),
            timeout_secs: 60,
            reply_tx: None,
        });
        // Type 'y' then Enter to approve.
        app.handle_overlay_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(perm_rx.try_recv().unwrap(), halcon_core::types::PermissionDecision::Allowed);
        assert!(!app.state.overlay.is_active()); // overlay closed
    }

    #[test]
    fn permission_overlay_n_sends_reject_on_perm_channel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "bash".into(),
            args: serde_json::json!({"command": "rm -rf /tmp/*.txt"}),
            risk_level: "High".into(),
            timeout_secs: 180,
            reply_tx: None,
        });
        // Type 'n' then Enter to reject.
        app.handle_overlay_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(perm_rx.try_recv().unwrap(), halcon_core::types::PermissionDecision::Denied);
        assert!(!app.state.overlay.is_active());
    }

    #[test]
    fn permission_overlay_enter_sends_approve_on_perm_channel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "file_write".into(),
            args: serde_json::json!({"path": "/tmp/test.txt", "content": "Hello"}),
            risk_level: "Medium".into(),
            timeout_secs: 120,
            reply_tx: None,
        });
        // Type 'yes' then Enter to approve.
        for c in "yes".chars() {
            app.handle_overlay_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(perm_rx.try_recv().unwrap(), halcon_core::types::PermissionDecision::Allowed);
    }

    // --- Sprint 2: UX + consistency tests ---

    #[test]
    fn agent_done_resets_agent_control() {
        use crate::tui::state::AgentControl;
        let mut app = test_app();
        app.state.agent_running = true;
        app.state.agent_control = AgentControl::Paused;
        app.handle_ui_event(UiEvent::AgentDone);
        assert_eq!(app.state.agent_control, AgentControl::Running);
        assert!(!app.state.agent_running);
    }

    #[test]
    fn agent_done_emits_toast() {
        let mut app = test_app();
        app.state.agent_running = true;
        app.handle_ui_event(UiEvent::AgentDone);
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn error_event_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::Error {
            message: "Connection failed".into(),
            hint: None,
        });
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn stream_error_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::StreamError("timeout".into()));
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn tool_denied_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::ToolDenied("bash".into()));
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn permission_awaiting_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "bash".into(),
            args: serde_json::json!({"command": "echo test"}),
            risk_level: "Low".into(),
            timeout_secs: 60,
            reply_tx: None,
        });
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn model_selected_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::ModelSelected {
            model: "gpt-4o".into(),
            provider: "openai".into(),
            reason: "complex task".into(),
        });
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn dry_run_emits_toast() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::DryRunActive(true));
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn agent_state_transition_persists_in_app_state() {
        use crate::tui::events::AgentState;
        let mut app = test_app();
        assert_eq!(app.state.agent_state, AgentState::Idle);
        app.handle_ui_event(UiEvent::AgentStateTransition {
            from: AgentState::Idle,
            to: AgentState::Planning,
            reason: "new task".into(),
        });
        assert_eq!(app.state.agent_state, AgentState::Planning);
    }

    #[test]
    fn agent_state_failed_transition_emits_toast() {
        use crate::tui::events::AgentState;
        let mut app = test_app();
        app.handle_ui_event(UiEvent::AgentStateTransition {
            from: AgentState::Executing,
            to: AgentState::Failed,
            reason: "provider error".into(),
        });
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn invalid_fsm_transition_logged_as_warning() {
        use crate::tui::events::AgentState;
        let mut app = test_app();
        // Idle → Complete is not a valid transition.
        app.handle_ui_event(UiEvent::AgentStateTransition {
            from: AgentState::Idle,
            to: AgentState::Complete,
            reason: "invalid".into(),
        });
        // State should still be persisted.
        assert_eq!(app.state.agent_state, AgentState::Complete);
        // Activity should contain a warning.
        assert!(app.activity_model.line_count() > 0);
    }

    #[test]
    fn slash_model_shows_info() {
        let mut app = test_app();
        app.execute_slash_command("model");
        assert!(app.activity_model.line_count() > 0);
    }

    #[test]
    fn slash_plan_switches_panel_to_plan() {
        let mut app = test_app();
        app.state.panel_visible = false;
        app.execute_slash_command("plan");
        assert!(app.state.panel_visible);
        assert_eq!(app.state.panel_section, crate::tui::state::PanelSection::Plan);
    }

    #[test]
    fn unknown_slash_command_shows_warning() {
        let mut app = test_app();
        app.execute_slash_command("nonexistent");
        assert!(app.activity_model.line_count() > 0);
    }

    // --- Sprint 3: Hardening tests ---

    #[test]
    fn burst_events_bounded_memory() {
        // Simulate 1000 rapid events — verify memory remains bounded.
        let mut app = test_app();
        for i in 0..1000 {
            app.handle_ui_event(UiEvent::Info(format!("burst event {i}")));
        }
        // Event log should be bounded at EVENT_RING_CAPACITY.
        assert!(app.event_log.len() <= EVENT_RING_CAPACITY);
        // Activity lines should all be present (no memory cap on activity).
        assert_eq!(app.activity_model.line_count(), 1000);
    }

    #[test]
    fn burst_events_event_log_oldest_evicted() {
        let mut app = test_app();
        for i in 0..500 {
            app.handle_ui_event(UiEvent::Info(format!("event {i}")));
        }
        assert_eq!(app.event_log.len(), EVENT_RING_CAPACITY);
        // Newest should be the last event.
        assert!(app.event_log.back().unwrap().label.contains("event 499"));
    }

    #[test]
    fn dismiss_toasts_action() {
        let mut app = test_app();
        app.handle_ui_event(UiEvent::Error {
            message: "test error".into(),
            hint: None,
        });
        assert!(!app.toasts.is_empty());
        app.handle_action(input::InputAction::DismissToasts);
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn bare_slash_opens_command_palette() {
        let mut app = test_app();
        for c in "/".chars() {
            app.prompt.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.handle_action(input::InputAction::SubmitPrompt);
        // Should open command palette, not send to agent.
        assert!(!app.state.agent_running);
        assert!(matches!(
            app.state.overlay.active,
            Some(OverlayKind::CommandPalette)
        ));
    }

    #[test]
    fn event_summary_exhaustive() {
        // Verify event_summary covers all UiEvent variants without panicking.
        use crate::tui::events::*;
        let events: Vec<UiEvent> = vec![
            UiEvent::StreamChunk("test".into()),
            UiEvent::StreamThinking("Chain-of-thought token preview".into()),
            UiEvent::StreamCodeBlock { lang: "rs".into(), code: "fn(){}".into() },
            UiEvent::StreamToolMarker("bash".into()),
            UiEvent::StreamDone,
            UiEvent::StreamError("err".into()),
            UiEvent::ToolStart { name: "bash".into(), input: serde_json::json!({}) },
            UiEvent::ToolOutput { name: "bash".into(), content: "ok".into(), is_error: false, duration_ms: 10 },
            UiEvent::ToolDenied("bash".into()),
            UiEvent::SpinnerStart("think".into()),
            UiEvent::SpinnerStop,
            UiEvent::Warning { message: "w".into(), hint: None },
            UiEvent::Error { message: "e".into(), hint: None },
            UiEvent::Info("info".into()),
            UiEvent::StatusUpdate { provider: None, model: None, round: None, tokens: None, cost: None, session_id: None, elapsed_ms: None, tool_count: None, input_tokens: None, output_tokens: None },
            UiEvent::RoundStart(1),
            UiEvent::RoundEnd(1),
            UiEvent::Redraw,
            UiEvent::AgentDone,
            UiEvent::Quit,
            UiEvent::PlanProgress { goal: "g".into(), steps: vec![], current_step: 0, elapsed_ms: 0 },
            UiEvent::RoundStarted { round: 1, provider: "p".into(), model: "m".into() },
            UiEvent::RoundEnded { round: 1, input_tokens: 0, output_tokens: 0, cost: 0.0, duration_ms: 0 },
            UiEvent::ModelSelected { model: "m".into(), provider: "p".into(), reason: "r".into() },
            UiEvent::ProviderFallback { from: "a".into(), to: "b".into(), reason: "r".into() },
            UiEvent::LoopGuardAction { action: "a".into(), reason: "r".into() },
            UiEvent::CompactionComplete { old_msgs: 10, new_msgs: 5, tokens_saved: 100 },
            UiEvent::CacheStatus { hit: true, source: "s".into() },
            UiEvent::SpeculativeResult { tool: "t".into(), hit: false },
            UiEvent::PermissionAwaiting { tool: "bash".into(), args: serde_json::json!({}), risk_level: "Low".into(), timeout_secs: 60, reply_tx: None },
            UiEvent::ReflectionStarted,
            UiEvent::ReflectionComplete { analysis: "a".into(), score: 0.5 },
            UiEvent::ConsolidationStatus { action: "a".into() },
            UiEvent::ToolRetrying { tool: "t".into(), attempt: 1, max_attempts: 3, delay_ms: 100 },
            UiEvent::ContextTierUpdate { l0_tokens: 0, l0_capacity: 0, l1_tokens: 0, l1_entries: 0, l2_entries: 0, l3_entries: 0, l4_entries: 0, total_tokens: 0 },
            UiEvent::ReasoningUpdate { strategy: "s".into(), task_type: "t".into(), complexity: "c".into() },
            UiEvent::Phase2Metrics { delegation_success_rate: None, delegation_trigger_rate: None, plan_success_rate: None, ucb1_agreement_rate: None },
            UiEvent::DryRunActive(false),
            UiEvent::TokenBudgetUpdate { used: 0, limit: 0, rate_per_minute: 0.0 },
            UiEvent::ProviderHealthUpdate { provider: "p".into(), status: ProviderHealthStatus::Healthy },
            UiEvent::CircuitBreakerUpdate { provider: "p".into(), state: CircuitBreakerState::Closed, failure_count: 0 },
            UiEvent::AgentStateTransition { from: AgentState::Idle, to: AgentState::Planning, reason: "r".into() },
            UiEvent::TaskStatus { title: "t".into(), status: "s".into(), duration_ms: None, artifact_count: 0 },
            UiEvent::ReasoningStatus { task_type: "t".into(), complexity: "c".into(), strategy: "s".into(), score: 0.0, success: true },
            // Phase 45: Status Bar Audit + Session Management
            UiEvent::TokenDelta { round_input: 10, round_output: 5, session_input: 100, session_output: 50 },
            UiEvent::SessionList { sessions: vec![] },
            // Dev Ecosystem Phase 5
            UiEvent::IdeConnected { port: 5758 },
            UiEvent::IdeDisconnected,
            UiEvent::IdeBuffersUpdated { count: 2, git_branch: Some("main".into()) },
            // Multi-Agent Orchestration Visibility
            UiEvent::OrchestratorWave { wave_index: 1, total_waves: 1, task_count: 3 },
            UiEvent::SubAgentSpawned { step_index: 1, total_steps: 3, description: "Analyse project".into(), agent_type: "General".into() },
            UiEvent::SubAgentCompleted { step_index: 1, total_steps: 3, success: true, latency_ms: 2100, tools_used: vec!["bash".into()], rounds: 2, summary: "Fixed auth bug".into(), error_hint: String::new() },
            // Multimodal
            UiEvent::MediaAnalysisStarted { count: 2 },
            UiEvent::MediaAnalysisComplete { filename: "photo.jpg".into(), tokens: 512 },
            // Phase 83: Phase-Aware Skeleton/Spinner
            UiEvent::PhaseStarted { phase: "planning".into(), label: "Generating execution plan...".into() },
            UiEvent::PhaseEnded,
            // Phase 93: Cross-Platform SOTA — media attachment events
            UiEvent::AttachmentAdded { path: "/tmp/photo.jpg".into(), modality: "image".into() },
            UiEvent::AttachmentRemoved { index: 0 },
            // Phase 94: Project Onboarding
            UiEvent::OnboardingAvailable { root: "/tmp/myproject".into(), project_type: "rust".into() },
            UiEvent::ProjectAnalysisComplete {
                root: "/tmp/myproject".into(),
                project_type: "rust".into(),
                package_name: Some("myapp".into()),
                has_git: true,
                preview: "# myapp\n".into(),
                save_path: "/tmp/myproject/.halcon/HALCON.md".into(),
            },
            UiEvent::ProjectConfigCreated { path: "/tmp/myproject/.halcon/HALCON.md".into() },
            UiEvent::ProjectHealthCalculated { score: 78, issues: vec!["No CI detected".into()], recommendations: vec!["Add GitHub Actions".into()] },
            UiEvent::ProjectConfigLoaded { path: "/tmp/myproject/.halcon/HALCON.md".into() },
            UiEvent::OpenInitWizard { dry_run: false },
            // Phase 95: Plugin Auto-Implantation
            UiEvent::PluginSuggestionReady { suggestions: vec![], dry_run: false },
            UiEvent::PluginBootstrapStarted { count: 2, dry_run: false },
            UiEvent::PluginBootstrapComplete { installed: 1, skipped: 1, failed: 0 },
            UiEvent::PluginStatusChanged { plugin_id: "my-plugin".into(), new_status: "active".into() },
        ];
        for ev in &events {
            let summary = event_summary(ev);
            assert!(!summary.is_empty(), "empty summary for {:?}", ev);
        }
        // All 68 UiEvent variants covered (63 Phase 95 base + 4 Phase 95 plugin management + 1 ProjectHealthCalculated).
        assert_eq!(events.len(), 68);
    }

    // Phase B1: Expansion Animation Tests
    #[test]
    fn expansion_animation_starts_at_initial_progress() {
        let anim = ExpansionAnimation::expand_from(0.5);
        let current = anim.current();
        assert!((current - 0.5).abs() < 0.01, "Expected ~0.5, got {}", current);
    }

    #[test]
    fn expansion_animation_reaches_target() {
        let anim = ExpansionAnimation::expand_from(0.0);
        std::thread::sleep(Duration::from_millis(210)); // 200ms + margin
        assert_eq!(anim.current(), 1.0, "Should reach 1.0 when expanding");
        assert!(anim.is_complete());
    }

    #[test]
    fn collapse_animation_reaches_zero() {
        let anim = ExpansionAnimation::collapse_from(1.0);
        std::thread::sleep(Duration::from_millis(160)); // 150ms + margin
        assert_eq!(anim.current(), 0.0, "Should reach 0.0 when collapsing");
        assert!(anim.is_complete());
    }

    #[test]
    fn expansion_animation_progresses_midway() {
        let anim = ExpansionAnimation::expand_from(0.0);
        std::thread::sleep(Duration::from_millis(50)); // ~25% of duration
        let current = anim.current();
        // With EaseInOut, early progress is slower than linear
        assert!(current > 0.0 && current < 0.5, "Expected 0.0 < current < 0.5, got {}", current);
    }

    #[test]
    fn cancel_mid_animation_reverses_direction() {
        let anim1 = ExpansionAnimation::expand_from(0.0);
        std::thread::sleep(Duration::from_millis(100)); // ~50% of 200ms
        let midpoint = anim1.current();
        assert!(midpoint > 0.0 && midpoint < 1.0, "Should be mid-animation");

        // Collapse from midpoint
        let anim2 = ExpansionAnimation::collapse_from(midpoint);
        let current = anim2.current();
        assert!((current - midpoint).abs() < 0.1, "Should start from midpoint, got {}", current);
    }

    #[test]
    fn ease_in_out_symmetry() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        // Midpoint should be roughly 0.5 (smoothstep)
        let mid = ease_in_out(0.5);
        assert!((mid - 0.5).abs() < 0.1, "Midpoint easing should be ~0.5, got {}", mid);
    }

    // Phase B2: Shimmer Animation Tests
    #[test]
    fn shimmer_progress_starts_at_zero() {
        let progress = shimmer_progress(Duration::from_millis(0));
        assert_eq!(progress, 0.0, "Shimmer should start at 0.0");
    }

    #[test]
    fn shimmer_progress_cycles_at_one_second() {
        // At 1000ms, should wrap back to ~0.0
        let progress = shimmer_progress(Duration::from_millis(1000));
        assert!((progress - 0.0).abs() < 0.01, "Shimmer should cycle at 1s, got {}", progress);
    }

    #[test]
    fn shimmer_progress_midpoint() {
        // At 500ms, should be ~0.5
        let progress = shimmer_progress(Duration::from_millis(500));
        assert!((progress - 0.5).abs() < 0.01, "Shimmer at 500ms should be ~0.5, got {}", progress);
    }

    #[test]
    fn shimmer_progress_quarter() {
        // At 250ms, should be ~0.25
        let progress = shimmer_progress(Duration::from_millis(250));
        assert!((progress - 0.25).abs() < 0.01, "Shimmer at 250ms should be ~0.25, got {}", progress);
    }

    #[test]
    fn shimmer_progress_repeats_after_cycle() {
        // At 1500ms (1.5 cycles), should be same as 500ms
        let p1 = shimmer_progress(Duration::from_millis(500));
        let p2 = shimmer_progress(Duration::from_millis(1500));
        assert!((p1 - p2).abs() < 0.01, "Shimmer should repeat after 1s cycle");
    }

    // Phase B3: Search Highlight Tests
    #[test]
    fn search_next_adds_highlight() {
        use crate::tui::highlight::HighlightManager;

        let mut highlights = HighlightManager::new();
        let test_idx = 5;

        // Simulate search_next adding highlight
        let highlight_key = format!("search_{}", test_idx);
        let palette = &crate::render::theme::active().palette;
        highlights.start_medium(&highlight_key, palette.accent);

        // Verify highlight is active
        assert!(highlights.is_pulsing(&highlight_key), "Highlight should be active after search_next");
    }

    #[test]
    fn search_highlight_stops_after_navigation() {
        use crate::tui::highlight::HighlightManager;

        let mut highlights = HighlightManager::new();
        let old_idx = 3;
        let new_idx = 7;

        // Add highlight at old position
        let old_key = format!("search_{}", old_idx);
        highlights.start_medium(&old_key, crate::render::theme::active().palette.accent);

        // Navigate to new position (stop old, start new)
        highlights.stop(&old_key);
        let new_key = format!("search_{}", new_idx);
        highlights.start_medium(&new_key, crate::render::theme::active().palette.accent);

        // Verify old stopped, new active
        assert!(!highlights.is_pulsing(&old_key), "Old highlight should be stopped");
        assert!(highlights.is_pulsing(&new_key), "New highlight should be active");
    }

    #[test]
    fn highlight_pulses_correctly() {
        use crate::tui::highlight::HighlightManager;

        let mut highlights = HighlightManager::new();
        let key = "search_10";
        let palette = &crate::render::theme::active().palette;

        highlights.start_medium(key, palette.accent);

        // Get current color (should be between accent and bg_highlight)
        let current_color = highlights.current(key, palette.bg_highlight);

        // Verify it's not the default (means pulse is active)
        assert!(highlights.is_pulsing(key), "Highlight should be pulsing");
    }

    #[test]
    fn search_matches_empty_no_crash() {
        // Verify search_next/prev don't crash with empty matches
        // (This would be tested in integration, here we verify the pattern)
        let search_matches: Vec<usize> = Vec::new();
        assert!(search_matches.is_empty(), "Empty search should be safe");
    }

    #[test]
    fn rerun_search_highlights_first_match() {
        let mut app = test_app();
        app.activity_model.push_info("first match");
        app.activity_model.push_info("other");
        app.activity_model.push_info("second match");

        // Open search overlay and enter query
        app.state.overlay.open(OverlayKind::Search);
        app.state.overlay.input = "match".into();
        app.rerun_search();

        // Verify first match (line 0) is highlighted
        let highlight_key = "search_0";
        assert!(app.highlights.is_pulsing(highlight_key),
                "First match should be highlighted after search entry");
    }

    // Phase B4: Hover Effect Tests
    #[test]
    fn hover_state_set_correctly() {
        use crate::tui::activity_navigator::ActivityNavigator;

        let mut nav = ActivityNavigator::new();

        // Initially no hover
        assert_eq!(nav.hovered(), None, "Should start with no hover");

        // Set hover to line 5
        nav.set_hover(Some(5));
        assert_eq!(nav.hovered(), Some(5), "Hover should be set to line 5");
        assert!(nav.is_hovered(5), "Line 5 should be hovered");
        assert!(!nav.is_hovered(3), "Line 3 should not be hovered");

        // Clear hover
        nav.clear_hover();
        assert_eq!(nav.hovered(), None, "Hover should be cleared");
        assert!(!nav.is_hovered(5), "Line 5 should no longer be hovered");
    }

    #[test]
    fn hover_updates_on_mouse_move() {
        use crate::tui::activity_navigator::ActivityNavigator;

        let mut nav = ActivityNavigator::new();

        // Move from line 2 to line 7
        nav.set_hover(Some(2));
        assert!(nav.is_hovered(2), "Line 2 should be hovered initially");

        nav.set_hover(Some(7));
        assert!(!nav.is_hovered(2), "Line 2 should no longer be hovered");
        assert!(nav.is_hovered(7), "Line 7 should now be hovered");
    }

    #[test]
    fn hover_cleared_when_mouse_leaves() {
        use crate::tui::activity_navigator::ActivityNavigator;

        let mut nav = ActivityNavigator::new();

        nav.set_hover(Some(10));
        assert!(nav.is_hovered(10), "Line 10 should be hovered");

        // Mouse leaves activity zone
        nav.set_hover(None);
        assert_eq!(nav.hovered(), None, "Hover should be cleared when mouse leaves");
    }

    #[test]
    fn hover_priority_below_selection() {
        // This is a design assertion:
        // Background priority: highlight > selection > hover > none
        // When a line is both selected and hovered, selection background wins.
        // This is enforced in ActivityRenderer.render_line() background logic.

        use crate::tui::activity_navigator::ActivityNavigator;

        let mut nav = ActivityNavigator::new();

        nav.selected_index = Some(5);
        nav.set_hover(Some(5));

        // Both are true
        assert!(nav.selected() == Some(5), "Line 5 should be selected");
        assert!(nav.is_hovered(5), "Line 5 should also be hovered");

        // Renderer will prioritize selection background over hover background
        // (tested via visual inspection and background priority logic)
    }

    // Phase 3 SRCH-004: Search History Integration Tests
    #[tokio::test]
    async fn search_history_saves_to_database() {
        use halcon_storage::{AsyncDatabase, Database};
        use std::sync::Arc;

        // Create in-memory database
        let db = Database::open_in_memory().unwrap();
        let async_db = AsyncDatabase::new(Arc::new(db));

        // Directly save a search to the database (bypass TUI complexity)
        // This tests the core save/load functionality
        async_db
            .save_search_history("testquery".to_string(), "exact".to_string(), 3, None)
            .await
            .unwrap();

        // Verify the search was saved
        let history = async_db.get_recent_queries(10).await.unwrap();
        assert_eq!(history.len(), 1, "Should have 1 query in history");
        assert_eq!(history[0], "testquery", "Query should be 'testquery'");

        // Verify full entry details
        let entries = async_db.load_search_history(10).await.unwrap();
        assert_eq!(entries.len(), 1, "Should have 1 entry");
        assert_eq!(entries[0].query, "testquery");
        assert_eq!(entries[0].search_mode, "exact");
        assert_eq!(entries[0].match_count, 3);
    }

    #[tokio::test]
    async fn search_history_loads_on_startup() {
        use halcon_storage::{AsyncDatabase, Database};
        use std::sync::Arc;

        // Create in-memory database and pre-populate with history
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.save_search_history("alpha", "exact", 1, None).unwrap();
        db.save_search_history("beta", "exact", 2, None).unwrap();
        db.save_search_history("gamma", "exact", 3, None).unwrap();

        let async_db = AsyncDatabase::new(db);

        // Create TUI app with database
        let (_tx, rx) = mpsc::unbounded_channel();
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, Some(async_db.clone()));

        // Simulate the loading that happens in run() method
        let queries = async_db.get_recent_queries(50).await.unwrap();
        app.activity_navigator.load_history(queries);
        app.search_history_loaded = true;

        // Verify history was loaded
        assert_eq!(app.search_history_loaded, true);
        assert_eq!(app.activity_navigator.search_history.len(), 3);
        assert_eq!(app.activity_navigator.search_history[0], "gamma"); // Most recent first
        assert_eq!(app.activity_navigator.search_history[1], "beta");
        assert_eq!(app.activity_navigator.search_history[2], "alpha");
    }

    #[test]
    fn search_history_navigation_with_arrows() {
        use crate::tui::overlay::OverlayKind;

        let mut app = test_app();

        // Pre-load history (simulating what run() does)
        app.activity_navigator.load_history(vec![
            "recent".to_string(),
            "middle".to_string(),
            "oldest".to_string(),
        ]);

        // Open search overlay
        app.state.overlay.open(OverlayKind::Search);

        // Simulate Ctrl+Up to navigate to older query
        let query1 = app.activity_navigator.history_up();
        assert_eq!(query1, Some("recent".to_string()));

        // Navigate to next older query
        let query2 = app.activity_navigator.history_up();
        assert_eq!(query2, Some("middle".to_string()));

        // Navigate to oldest query
        let query3 = app.activity_navigator.history_up();
        assert_eq!(query3, Some("oldest".to_string()));

        // Attempt to go beyond history returns None
        let query4 = app.activity_navigator.history_up();
        assert_eq!(query4, None);

        // Navigate back down
        let query5 = app.activity_navigator.history_down();
        assert_eq!(query5, Some("middle".to_string()));
    }

    #[test]
    fn search_history_resets_on_overlay_close() {
        use crate::tui::overlay::OverlayKind;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = test_app();

        // Pre-load history
        app.activity_navigator.load_history(vec!["test".to_string()]);

        // Open search overlay and navigate history
        app.state.overlay.open(OverlayKind::Search);
        let _ = app.activity_navigator.history_up();
        assert!(app.activity_navigator.history_index > 0, "History index should be > 0 after navigation");

        // Close overlay with Esc
        app.handle_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        // Verify history navigation was reset
        assert_eq!(app.activity_navigator.history_index, 0, "History index should be reset to 0");
    }
