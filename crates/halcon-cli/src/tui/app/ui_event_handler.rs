//! UiEvent dispatch handler for TuiApp.
use super::*;

impl TuiApp {
    pub(super) fn handle_ui_event(&mut self, ev: UiEvent) {
        // Log every event to the ring buffer for inspector.
        self.log_event(event_summary(&ev));

        match ev {
            UiEvent::StreamChunk(text) => {
                // Filter DeepSeek DSML tool-call XML that leaks when tools are removed.
                // <｜DSML｜function_calls> blocks are internal protocol artifacts that
                // appear when DeepSeek uses its XML fallback format (no tools in request).
                // The loop guard threshold increase (6/10) prevents this in most cases;
                // this filter handles edge cases where it still leaks through.
                if text.contains("\u{FF5C}DSML\u{FF5C}") {
                    tracing::debug!("Suppressing DSML function_call block from activity feed ({} bytes)", text.len());
                } else {
                    // First real answer token arrived — drop the "thinking" skeleton.
                    self.activity_model.remove_thinking();

                    // Collapse the ThinkingDelta buffer into a single compact summary line.
                    // All fragments were silently accumulated in state.thinking_buffer to prevent
                    // ~45 individual "🧠 X" info lines from flooding the activity feed.
                    if !self.state.thinking_buffer.is_empty() {
                        let tb = std::mem::take(&mut self.state.thinking_buffer);
                        const MAX_PREVIEW: usize = 80;
                        let summary = if tb.len() > MAX_PREVIEW {
                            format!("🧠 {}… ({} chars)", &tb[..MAX_PREVIEW], tb.len())
                        } else {
                            format!("🧠 {tb}")
                        };
                        self.activity_model.push_info(&summary);
                    }

                    // P0.3: Fix stream chunk acumulación — use push_assistant_text() instead of push()
                    self.activity_model.push_assistant_text(text);
                    // P0.4 FIX: Don't use clear_cache() - too aggressive, causes duplicates
                    // Instead, renderer will skip cache for last AssistantText line
                }
            }
            UiEvent::StreamThinking(text) => {
                // Chain-of-thought tokens from reasoning models (deepseek-reasoner, o1, o3-mini).
                //
                // SSE streams emit ThinkingDelta as tiny 1-10 char fragments. Calling push_info()
                // per fragment floods the activity feed with ~45 single-character lines.
                //
                // Fix: accumulate all fragments silently in `state.thinking_buffer`.
                // The buffer is collapsed into a SINGLE summary line when:
                //   - `StreamChunk` arrives (real answer token), or
                //   - `StreamDone` arrives with no StreamChunk (pure reasoning response)
                //
                // ThinkingProgress events (from TuiSink) will update the PhaseIndicator live.
                // ThinkingComplete (from TuiSink) will push a persistent ThinkingBubble.
                self.state.thinking_buffer.push_str(&text);
            }
            UiEvent::ThinkingProgress { chars } => {
                // Update AgentThinking → PhaseIndicator(Reasoning) with live char count label.
                let kchars = if chars >= 1000 {
                    format!("{:.1}K", chars as f64 / 1000.0)
                } else {
                    chars.to_string()
                };
                let label = format!("Razonando... {kchars} chars");
                use crate::tui::activity_types::AgentPhase;
                self.activity_model.remove_thinking();
                self.activity_model.push_phase_indicator(AgentPhase::Reasoning, &label);
            }
            UiEvent::ThinkingComplete { preview, char_count } => {
                // PhaseIndicator → ThinkingBubble persistente.
                self.activity_model.push_thinking_bubble(char_count, preview);
                // Clear thinking_buffer so StreamChunk's fallback 🧠 line is skipped.
                self.state.thinking_buffer.clear();
            }
            UiEvent::StreamCodeBlock { lang, code } => {
                self.activity_model.push_code_block(&lang, &code);
            }
            UiEvent::StreamToolMarker(_name) => {
                // Suppress: ToolStart already creates a ToolExec card — no redundant Info line
            }
            UiEvent::StreamDone => {
                // P0.4 FIX: Clear cache to prevent stale renders after streaming completes
                // When streaming ends, the AssistantText line is no longer "last" (Info lines added after)
                // so renderer would use cache, but cache might have partial content from streaming
                self.activity_renderer.clear_cache();

                // Collapse any remaining ThinkingDelta buffer.
                // This fires when a reasoning model produced ONLY chain-of-thought tokens (no TextDelta).
                // Example: deepseek-reasoner responds to "hola" with only reasoning_content, empty content.
                // In this case, we surface the thinking content as the assistant response.
                if !self.state.thinking_buffer.is_empty() {
                    let tb = std::mem::take(&mut self.state.thinking_buffer);
                    self.activity_model.remove_thinking();
                    const MAX_PREVIEW: usize = 80;
                    let summary = if tb.len() > MAX_PREVIEW {
                        format!("🧠 {}… ({} chars)", &tb[..MAX_PREVIEW], tb.len())
                    } else {
                        format!("🧠 {tb}")
                    };
                    self.activity_model.push_info(&summary);
                    // Surface the full thinking content as the assistant response so the user
                    // sees the model's actual output rather than a blank screen.
                    self.activity_model.push_assistant_text(tb);
                }

                tracing::trace!("StreamDone received");
            }
            UiEvent::StreamError(msg) => {
                self.activity_model.push_error(&msg, None);
                self.toasts.push(Toast::new("Stream error", ToastLevel::Error));
            }
            UiEvent::ToolStart { name, input } => {
                // Phase B2: Track tool start time for shimmer animation
                self.executing_tools.insert(name.clone(), Instant::now());

                // Build a short input preview from the JSON value.
                let input_preview = format_input_preview(&input);
                self.activity_model.push_tool_start(&name, &input_preview);
                self.panel.metrics.tool_count += 1;
                // Phase 100: Also update status bar tool_count in real-time (Fix #2).
                // Previously status bar only updated at end-of-loop via StatusUpdate,
                // causing "Tools: 0" during execution. Now reflects live execution count.
                self.status.increment_tool_count();

                // Phase 2.3: Set agent state to ToolExecution + highlight
                self.agent_badge.set_state(AgentState::ToolExecution);
                self.agent_badge.set_detail(Some(format!("Running {}...", name)));

                // Start subtle highlight pulse on tool execution
                let p = &crate::render::theme::active().palette;
                self.highlights.start_subtle("tool_execution", p.delegated);
            }
            UiEvent::ToolOutput { name, content, is_error, duration_ms } => {
                // Phase B2: Remove from executing tools (shimmer animation complete)
                self.executing_tools.remove(&name);

                self.activity_model.complete_tool(&name, content.clone(), is_error, duration_ms);
            }
            UiEvent::ToolDenied(name) => {
                let msg = format!("Tool denied: {name}");
                self.activity_model.push_warning(&msg, None);
                self.toasts.push(Toast::new(format!("Denied: {name}"), ToastLevel::Warning));
            }
            UiEvent::SpinnerStart(label) => {
                self.state.spinner_active = true;
                self.state.spinner_label = label;
            }
            UiEvent::SpinnerStop => {
                self.state.spinner_active = false;
                self.activity_model.remove_thinking();
                // Safety net: discard any leftover thinking buffer when the spinner stops.
                // Prevents stale thinking content from appearing in the next response.
                self.state.thinking_buffer.clear();
            }
            UiEvent::Warning { message, hint } => {
                self.activity_model.push_warning(&message, hint.as_deref());
            }
            UiEvent::Error { message, hint } => {
                self.activity_model.push_error(&message, hint.as_deref());
                self.toasts.push(Toast::new(
                    truncate_str(&message, 40),
                    ToastLevel::Error,
                ));

                // Phase 4C: CRITICAL FIX - Force unlock input on ANY error to prevent stuck UI
                // When provider errors occur (auth, quota, etc.), we MUST guarantee input remains accessible.
                self.state.agent_running = false;
                self.state.focus = FocusZone::Prompt;
                self.state.spinner_active = false;
                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);
            }
            UiEvent::Info(msg) => {
                self.activity_model.push_info(&msg);
            }
            UiEvent::StatusUpdate {
                provider, model, round, tokens, cost,
                session_id, elapsed_ms, tool_count, input_tokens, output_tokens,
            } => {
                self.status.update(
                    provider, model, round, tokens, cost,
                    session_id, elapsed_ms, tool_count, input_tokens, output_tokens,
                );
            }
            UiEvent::RoundStart(n) => {
                self.activity_model.push_round_separator(n);
            }
            UiEvent::RoundEnd(_n) => {
                // Legacy round end — superseded by RoundEnded with metrics.
                tracing::trace!(round = _n, "RoundEnd (legacy) received");
            }
            UiEvent::Redraw => {
                // Force redraw — the next frame will pick up any pending changes.
                tracing::trace!("Redraw requested");
            }
            // Phase 44B: Continuous interaction events
            UiEvent::AgentStartedPrompt => {
                // Agent dequeued a prompt and started processing.
                // Decrement queue count (will be corrected by PromptQueueStatus).
                self.state.prompts_queued = self.state.prompts_queued.saturating_sub(1);
                // Reset thinking buffer for the new prompt — ensures stale reasoning from a
                // previous turn doesn't bleed into the current turn's display.
                self.state.thinking_buffer.clear();
                // Phase 100 Fix #2: Reset real-time tool_count so each turn starts at 0.
                self.status.apply_patch(StatusPatch { tool_count: Some(0), ..Default::default() });
                self.panel.metrics.tool_count = 0;
                self.state.agent_running = true;
                // Phase 45C: Sync status bar agent_running for STOP button display.
                self.status.agent_running = true;

                // Input remains idle/ready — user can type next message while agent processes.
                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);

                // Phase 2.3: Set agent state to Running
                self.agent_badge.set_state(AgentState::Running);
                self.agent_badge.set_detail(Some("Processing prompt...".to_string()));

                // Start watchdog timer to prevent permanent UI freeze
                self.agent_started_at = Some(Instant::now());

                // Show "thinking" skeleton while waiting for first model token.
                self.activity_model.push_thinking();
                self.activity_navigator.scroll_to_bottom();

                tracing::debug!(
                    agent_running = self.state.agent_running,
                    prompts_queued = self.state.prompts_queued,
                    watchdog_started = true,
                    input_state = ?self.prompt.input_state(),
                    "Agent dequeued and started processing prompt"
                );
            }
            UiEvent::AgentFinishedPrompt => {
                // Agent finished processing one prompt.
                // Decrementar inmediatamente si la cola está vacía para evitar desincronización.
                // PromptQueueStatus proporcionará la cuenta autoritativa después.
                if self.state.prompts_queued > 0 {
                    self.state.prompts_queued -= 1;
                }

                // Safety net: ensure thinking skeleton is gone even if StreamChunk never fired.
                self.activity_model.remove_thinking();

                // Input stays idle — user can always type.
                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);

                tracing::debug!(
                    prompts_queued = self.state.prompts_queued,
                    input_state = ?self.prompt.input_state(),
                    "Agent finished processing prompt"
                );
            }
            UiEvent::PromptQueueStatus(count) => {
                // Authoritative queue count from the agent loop.
                self.state.prompts_queued = count;

                // Phase 4B-Lite: Update status bar with queue info
                let agents_active = if self.state.agent_running { 1 } else { 0 };
                self.status.update_queue_status(count, agents_active);

                tracing::debug!(
                    queued = count,
                    agents_active,
                    "Prompt queue status updated"
                );
            }
            UiEvent::AgentDone => {
                // Capture state BEFORE changes for debugging
                let before_agent_running = self.state.agent_running;
                let before_prompts_queued = self.state.prompts_queued;
                let watchdog_elapsed = self.agent_started_at.map(|t| t.elapsed().as_secs());

                tracing::debug!(
                    before_agent_running,
                    before_prompts_queued,
                    watchdog_elapsed_secs = ?watchdog_elapsed,
                    "AgentDone event received - transitioning to idle state"
                );

                // Apply state transitions
                self.state.agent_running = false;
                self.state.spinner_active = false;
                self.state.focus = FocusZone::Prompt;
                self.state.agent_control = crate::tui::state::AgentControl::Running;
                // Phase 45C: Sync status bar agent_running for STOP button display.
                self.status.agent_running = false;

                // Clear watchdog timer
                self.agent_started_at = None;

                // Reset FSM state to Idle + sync agent badge + clear highlights.
                self.state.agent_state = crate::tui::events::AgentState::Idle;
                self.agent_badge.set_state(AgentState::Idle); // indicator::AgentState::Idle
                self.agent_badge.set_detail(None);
                self.status.plan_step = None; // Clear plan step indicator when agent finishes.
                self.highlights.clear();

                // ALWAYS restore InputState to Idle — prompt is never stuck after agent done.
                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);

                // Validation: warn if prompts still queued (expected if user queued during processing)
                if self.state.prompts_queued > 0 {
                    tracing::info!(
                        prompts_queued = self.state.prompts_queued,
                        "AgentDone: prompts still queued - agent will process next prompt"
                    );
                } else {
                    // Only show completion toast if queue is empty
                    self.toasts.push(Toast::new("Agent completed", ToastLevel::Success));
                }

                // Log final state AFTER changes
                tracing::debug!(
                    after_agent_running = self.state.agent_running,
                    after_prompts_queued = self.state.prompts_queued,
                    agent_control = ?self.state.agent_control,
                    focus = ?self.state.focus,
                    watchdog_cleared = true,
                    "AgentDone: state transition complete - UI ready for input"
                );
            }
            UiEvent::Quit => {
                self.state.should_quit = true;
            }
            UiEvent::PlanProgress { goal, steps, current_step, .. } => {
                self.activity_model.set_plan_overview(goal.clone(), steps.clone(), current_step);
                self.panel.update_plan(steps.clone(), current_step);

                // Update status bar plan step indicator.
                if current_step < steps.len() {
                    let desc = &steps[current_step].description;
                    let truncated = truncate_str(desc, 30);
                    self.status.plan_step = Some(format!(
                        "Step {}/{}: {truncated}",
                        current_step + 1,
                        steps.len()
                    ));
                } else {
                    self.status.plan_step = Some("Plan complete".into());
                }
            }

            // --- Phase 42B: Cockpit feedback event handlers ---
            UiEvent::SessionInitialized { session_id } => {
                self.status.apply_patch(StatusPatch { session_id: Some(session_id), ..Default::default() });
            }
            UiEvent::RoundStarted { round, provider, model } => {
                self.activity_model.push_round_separator(round);
                self.status.apply_patch(StatusPatch {
                    provider: Some(provider),
                    model: Some(model),
                    round: Some(round),
                    ..Default::default()
                });
            }
            UiEvent::RoundEnded { round, input_tokens, output_tokens, cost, duration_ms } => {
                self.status.apply_patch(StatusPatch {
                    cost: Some(cost),
                    elapsed_ms: Some(duration_ms),
                    input_tokens: Some(input_tokens),
                    output_tokens: Some(output_tokens),
                    ..Default::default()
                });
                self.panel.update_metrics(round, input_tokens, output_tokens, cost, duration_ms);
            }
            UiEvent::ModelSelected { model, provider, reason: _ } => {
                // [model] info already visible in status bar — suppress from activity feed
                self.toasts.push(Toast::new(
                    format!("Model: {provider}/{model}"),
                    ToastLevel::Info,
                ));
            }
            UiEvent::ProviderFallback { from, to, reason } => {
                // Single push using chip-aware prefix (⇄ rendered by Warning chip classifier)
                self.activity_model.push_warning(&format!("⇄ {from} → {to}  {reason}"), None);
                self.toasts.push(Toast::new(format!("{from} → {to}"), ToastLevel::Warning));
            }
            UiEvent::LoopGuardAction { action, reason } => {
                self.activity_model.push_warning(&format!("[guard] {action}: {reason}"), None);
            }
            UiEvent::CompactionComplete { old_msgs, new_msgs, tokens_saved } => {
                // Single push, no duplicate
                self.activity_model.push_info(&format!(
                    "[compaction] {old_msgs} → {new_msgs} messages ({tokens_saved} tokens saved)"
                ));
            }
            UiEvent::CacheStatus { hit, source: _ } => {
                // Cache status tracked in panel metrics only — not noisy in activity feed
                self.panel.record_cache(hit);
            }
            UiEvent::SpeculativeResult { tool: _, hit: _ } => {
                // Speculative execution results: panel-only visibility
            }
            UiEvent::PermissionAwaiting { tool, args, risk_level, reply_tx } => {
                self.activity_model.push_info(&format!("[permission] awaiting approval for {tool}"));
                self.state.agent_control = crate::tui::state::AgentControl::WaitingApproval;

                // Store sub-agent reply channel (if present) so decisions are routed correctly.
                // None = main agent, decisions go via self.perm_tx as before.
                self.pending_perm_reply_tx = reply_tx;

                // Phase 2.1: Keep input available during permission prompt (for queuing).
                // Input state stays Queued or Idle - user can still type.
                // NOTE: InputState::LockedByPermission is NOT used anymore - input is ALWAYS available.

                // Phase 2.2 & 5/6/7: Create permission modal with momoto colors (8-option system).
                let risk = PermissionContext::parse_risk(&risk_level);
                let context = PermissionContext::new(tool.clone(), args.clone(), risk);
                self.permission_modal = Some(PermissionModal::new(context));

                // Phase 5/6/7: Conversational overlay removed - using direct 8-option modal instead.
                // All permission keys (Y/N/A/D/S/P/X) now route directly to PermissionOptions.

                self.state.overlay.open(OverlayKind::PermissionPrompt { tool: tool.clone() });
                self.toasts.push(Toast::new(
                    format!("Approval needed: {tool} ({} risk)", risk.label()),
                    ToastLevel::Warning,
                ));

                // Phase 2.3: Set agent state to WaitingPermission + strong pulse
                self.agent_badge.set_state(AgentState::WaitingPermission);
                self.agent_badge.set_detail(Some(format!("Awaiting approval: {}", tool)));

                // Start strong pulse on permission prompt (high urgency)
                let risk_color = risk.color(&crate::render::theme::active().palette);
                self.highlights.start_strong("permission_prompt", risk_color);

                tracing::debug!(
                    tool = tool,
                    risk_level = ?risk,
                    input_state = ?self.prompt.input_state(),
                    "Permission required, input locked (Phase 2.2 modal)"
                );
            }
            // Phase 43C: Feedback completeness events.
            UiEvent::ReflectionStarted => {
                use crate::tui::activity_types::AgentPhase;
                self.activity_model.push_phase_indicator(AgentPhase::Reflecting, "Analyzing conversation quality...");
            }
            UiEvent::ReflectionComplete { analysis, score } => {
                self.activity_model.remove_phase_indicator();
                let preview = truncate_str(&analysis, 80);
                self.activity_model.push_info(&format!("[reflection] {preview} (score: {score:.2})"));
            }

            // Phase 83: Phase-Aware Skeleton/Spinner.
            UiEvent::PhaseStarted { phase, label } => {
                use crate::tui::activity_types::AgentPhase;
                let ap = match phase.as_str() {
                    "planning"   => AgentPhase::Planning,
                    "reasoning"  => AgentPhase::Reasoning,
                    "reflecting" => AgentPhase::Reflecting,
                    "searching"  => AgentPhase::Searching,
                    _            => AgentPhase::Reasoning,
                };
                self.activity_model.push_phase_indicator(ap, label.as_str());
            }
            UiEvent::PhaseEnded => {
                self.activity_model.remove_phase_indicator();
            }
            // Phase 93: Media attachment events — update state for chip rendering.
            UiEvent::AttachmentAdded { path, modality } => {
                self.activity_model.push_info(&format!("[media] Attached {modality}: {path}"));
            }
            UiEvent::AttachmentRemoved { index } => {
                self.activity_model.push_info(&format!("[media] Removed attachment at index {index}"));
            }

            // Phase 94: Project Onboarding events
            UiEvent::OnboardingAvailable { root, project_type } => {
                self.toasts.push(Toast::new(
                    format!("No project config ({project_type}) — /init to configure"),
                    ToastLevel::Info,
                ));
                self.activity_model.push_info(
                    &format!("[onboarding] Sin HALCON.md en {root} → /init para configurar el proyecto"),
                );
            }
            UiEvent::ProjectAnalysisComplete { root: _, project_type: _, package_name: _, has_git: _, preview, save_path } => {
                if let Some(crate::tui::overlay::OverlayKind::InitWizard { ref mut step, preview: ref mut p, save_path: ref mut sp, .. }) =
                    self.state.overlay.active
                {
                    *step = 1;
                    *p = preview;
                    *sp = save_path;
                }
            }
            UiEvent::ProjectConfigCreated { path } => {
                if let Some(crate::tui::overlay::OverlayKind::InitWizard { ref mut step, .. }) = self.state.overlay.active {
                    *step = 4;
                }
                self.toasts.push(Toast::new(
                    format!("Guardado: {path}"),
                    ToastLevel::Success,
                ));
            }
            UiEvent::ProjectConfigLoaded { .. } => {
                // Silent — no toast. Banner already shows ◆ project cfg.
            }
            UiEvent::OpenInitWizard { dry_run } => {
                self.state.overlay.open(crate::tui::overlay::OverlayKind::InitWizard {
                    step: 0,
                    preview: String::new(),
                    save_path: String::new(),
                    dry_run,
                });
                // Kick off background analysis identical to the /init command path.
                if let Some(ref tx) = self.ui_tx_for_bg {
                    let tx = tx.clone();
                    let cwd = std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from("."));
                    tokio::spawn(async move {
                        super::super::project_analyzer::analyze_and_emit(tx, cwd).await;
                    });
                }
            }

            // Phase 95: Plugin Auto-Implantation events
            UiEvent::PluginSuggestionReady { suggestions, dry_run } => {
                self.state.overlay.open(crate::tui::overlay::OverlayKind::PluginSuggest {
                    suggestions,
                    selected: 0,
                    dry_run,
                });
            }
            UiEvent::PluginBootstrapStarted { count, dry_run } => {
                let label = if dry_run { "dry-run" } else { "installing" };
                self.activity_model.push_info(&format!(
                    "[plugins] Bootstrap {label} — {count} plugins"
                ));
            }
            UiEvent::PluginBootstrapComplete { installed, skipped, failed } => {
                self.toasts.push(Toast::new(
                    format!("Plugins: ✓{installed} installed, {skipped} skipped, {failed} failed"),
                    if failed > 0 { ToastLevel::Warning } else { ToastLevel::Info },
                ));
            }
            UiEvent::PluginStatusChanged { plugin_id, new_status } => {
                self.toasts.push(Toast::new(
                    format!("[plugin] {plugin_id} → {new_status}"),
                    ToastLevel::Info,
                ));
            }

            UiEvent::ProjectHealthCalculated { score, issues, recommendations } => {
                let icon = if score >= 80 { "◈" } else if score >= 60 { "◇" } else { "⚐" };
                self.activity_model.push_info(&format!(
                    "[init] {icon} Health score: {score}/100 ({} issues, {} recs)",
                    issues.len(),
                    recommendations.len()
                ));
                if !issues.is_empty() {
                    for issue in &issues {
                        self.activity_model.push_warning(
                            &format!("[init] ⚐ {issue}"),
                            None,
                        );
                    }
                }
                if !recommendations.is_empty() {
                    for rec in recommendations.iter().take(3) {
                        self.activity_model.push_info(&format!("[init] ↳ {rec}"));
                    }
                }
                self.toasts.push(Toast::new(
                    format!("Project health: {score}/100"),
                    if score >= 80 { ToastLevel::Info } else { ToastLevel::Warning },
                ));
            }

            UiEvent::ConsolidationStatus { action } => {
                self.activity_model.push_info(&format!("[memory] {action}"));
            }
            UiEvent::ConsolidationComplete { merged, pruned, duration_ms } => {
                let duration_s = duration_ms as f64 / 1000.0;
                self.activity_model.push_info(&format!(
                    "[memory] consolidation complete: merged={merged}, pruned={pruned}, {duration_s:.2}s"
                ));
                tracing::debug!(
                    merged,
                    pruned,
                    duration_ms,
                    "Memory consolidation completed successfully"
                );
            }
            UiEvent::ToolRetrying { tool, attempt, max_attempts, delay_ms } => {
                // Single push — no duplicate
                self.activity_model.push_warning(
                    &format!("[retry] {tool} attempt {attempt}/{max_attempts} in {delay_ms}ms"),
                    None,
                );
                self.toasts.push(Toast::new(
                    format!("Retrying {tool} ({attempt}/{max_attempts})"),
                    ToastLevel::Warning,
                ));
            }

            // Phase 43D: Live panel data
            UiEvent::ContextTierUpdate {
                l0_tokens, l0_capacity, l1_tokens, l1_entries,
                l2_entries, l3_entries, l4_entries, total_tokens,
            } => {
                self.panel.update_context(
                    l0_tokens, l0_capacity, l1_tokens, l1_entries,
                    l2_entries, l3_entries, l4_entries, total_tokens,
                );
            }
            UiEvent::ReasoningUpdate { strategy, task_type, complexity } => {
                self.panel.update_reasoning(strategy, task_type, complexity);
            }

            // Phase 2: Metrics update
            UiEvent::Phase2Metrics {
                delegation_success_rate,
                delegation_trigger_rate,
                plan_success_rate,
                ucb1_agreement_rate,
            } => {
                self.panel.update_phase2_metrics(
                    delegation_success_rate,
                    delegation_trigger_rate,
                    plan_success_rate,
                    ucb1_agreement_rate,
                );
            }

            // Phase 50: Sudo password elevation — open modal for password entry.
            UiEvent::SudoPasswordRequest { tool, command, has_cached } => {
                // Check in-process 5-minute sudo cache before showing modal.
                let use_cached = has_cached && self.sudo_cache
                    .as_ref()
                    .map(|(_, ts)| ts.elapsed().as_secs() < 300)
                    .unwrap_or(false);

                self.sudo_has_cached = use_cached;
                self.sudo_password_buf.clear();
                self.sudo_remember_password = false;

                if use_cached {
                    // We have a fresh cached password — send it immediately.
                    if let Some((ref pw, _)) = self.sudo_cache {
                        let _ = self.sudo_pw_tx.as_ref().map(|tx| tx.send(Some(pw.clone())));
                        tracing::debug!("Used cached sudo password (within 5-minute TTL)");
                    }
                } else {
                    // Open the sudo password overlay.
                    self.state.overlay.open(
                        crate::tui::overlay::OverlayKind::SudoPasswordEntry {
                            tool: tool.clone(),
                            command: command.clone(),
                        }
                    );
                    self.toasts.push(Toast::new(
                        format!("Sudo elevation required for {tool}"),
                        ToastLevel::Warning,
                    ));
                    tracing::debug!(tool = %tool, "Sudo password modal opened");
                }
            }

            // Phase 44A: Observability events
            UiEvent::DryRunActive(active) => {
                self.state.dry_run_active = active;
                if active {
                    self.activity_model.push_warning(
                        constants::DRY_RUN_WARNING,
                        Some(constants::DRY_RUN_HINT),
                    );
                    self.toasts.push(Toast::new(constants::DRY_RUN_TOAST, ToastLevel::Warning));
                }
            }
            UiEvent::TokenBudgetUpdate { used, limit, rate_per_minute } => {
                self.state.token_budget.used = used;
                self.state.token_budget.limit = limit;
                self.state.token_budget.rate_per_minute = rate_per_minute;
            }
            UiEvent::ProviderHealthUpdate { provider, status } => {
                let label = match &status {
                    crate::tui::events::ProviderHealthStatus::Healthy => "healthy".to_string(),
                    crate::tui::events::ProviderHealthStatus::Degraded { failure_rate, .. } => {
                        format!("degraded (fail:{:.0}%)", failure_rate * 100.0)
                    }
                    crate::tui::events::ProviderHealthStatus::Unhealthy { reason } => {
                        format!("unhealthy: {reason}")
                    }
                };
                self.activity_model.push_info(&format!("[health] {provider}: {label}"));
                // Update status bar health indicator for the active provider.
                if provider == self.status.current_provider() {
                    self.status.provider_health = status;
                }
            }

            // Phase B4: Circuit breaker state
            UiEvent::CircuitBreakerUpdate { provider, state, failure_count } => {
                let label = match &state {
                    crate::tui::events::CircuitBreakerState::Closed => "closed",
                    crate::tui::events::CircuitBreakerState::Open => "OPEN",
                    crate::tui::events::CircuitBreakerState::HalfOpen => "half-open",
                };
                self.activity_model.push_info(&format!(
                    "[breaker] {provider}: {label} (failures: {failure_count})"
                ));
                self.panel.update_breaker(provider.clone(), state.clone(), failure_count);
                if matches!(state, crate::tui::events::CircuitBreakerState::Open) {
                    self.toasts.push(Toast::new(
                        format!("Breaker OPEN: {provider}"),
                        ToastLevel::Error,
                    ));
                }
            }

            // Phase B5: Agent state transition
            UiEvent::AgentStateTransition { from, to, reason } => {
                // FSM transition validation.
                if !from.can_transition_to(&to) {
                    self.activity_model.push_warning(
                        &format!("[state] INVALID: {:?} → {:?}: {reason}", from, to),
                        Some("This transition is not expected by the FSM"),
                    );
                    tracing::warn!(
                        from = ?from, to = ?to, reason = %reason,
                        "Invalid agent state transition"
                    );
                } else {
                    self.activity_model.push_info(&format!(
                        "[state] {:?} → {:?}: {reason}", from, to
                    ));
                }
                // Persist FSM state in AppState.
                self.state.agent_state = to.clone();

                // Sync agent badge visual state (events::AgentState → indicator::AgentState).
                use crate::tui::events::AgentState as FsmState;
                use crate::tui::widgets::activity_indicator::AgentState as BadgeState;
                let badge_state = match &to {
                    FsmState::Idle      => BadgeState::Idle,
                    FsmState::Planning  => BadgeState::Planning,
                    FsmState::Executing => BadgeState::Running,
                    FsmState::ToolWait  => BadgeState::ToolExecution,
                    FsmState::Reflecting => BadgeState::Running,
                    FsmState::Paused    => BadgeState::WaitingPermission,
                    FsmState::Complete  => BadgeState::Idle,
                    FsmState::Failed    => BadgeState::Error,
                };
                self.agent_badge.set_state(badge_state);
                // Update badge detail label.
                let detail = match &to {
                    FsmState::Planning   => Some("Planning…".to_string()),
                    FsmState::Executing  => Some("Running".to_string()),
                    FsmState::ToolWait   => Some("Tools…".to_string()),
                    FsmState::Reflecting => Some("Reflecting…".to_string()),
                    FsmState::Paused     => Some("Paused".to_string()),
                    FsmState::Complete   => Some("Done".to_string()),
                    FsmState::Failed     => Some(format!("Failed: {reason}")),
                    FsmState::Idle       => None,
                };
                self.agent_badge.set_detail(detail);
                // Toast for failure transitions.
                if matches!(to, FsmState::Failed) {
                    self.toasts.push(Toast::new(
                        format!("Agent failed: {reason}"),
                        ToastLevel::Error,
                    ));
                }
            }

            // Sprint 1 B2: Task status (parity with ClassicSink)
            UiEvent::TaskStatus { title, status, duration_ms, artifact_count } => {
                let timing = duration_ms
                    .map(|ms| format!(" ({:.1}s", ms as f64 / 1000.0))
                    .unwrap_or_default();
                let artifacts = if artifact_count > 0 {
                    format!(", {} artifact{}", artifact_count, if artifact_count == 1 { "" } else { "s" })
                } else {
                    String::new()
                };
                let suffix = if !timing.is_empty() {
                    format!("{timing}{artifacts})")
                } else if !artifacts.is_empty() {
                    format!("({artifacts})")
                } else {
                    String::new()
                };
                self.activity_model.push_info(&format!("[task] {title} — {status}{suffix}"));
            }

            // Sprint 1 B3: Reasoning status (parity with ClassicSink)
            UiEvent::ReasoningStatus { task_type, complexity, strategy, score, success } => {
                let outcome = if success { "Success" } else { "Below threshold" };
                self.activity_model.push_info(&format!("[reasoning] {task_type} ({complexity}) → {strategy}"));
                self.activity_model.push_info(&format!("[evaluation] Score: {score:.2} — {outcome}"));
            }

            // FASE 1.2: HICON Metrics Visibility
            UiEvent::HiconCorrection { strategy, reason, round } => {
                self.activity_model.push_info(&format!(
                    "[hicon:correction] Round {round}: Applied {strategy} — {reason}"
                ));
            }
            UiEvent::HiconAnomaly { anomaly_type, severity, details, confidence } => {
                let message = format!(
                    "[hicon:anomaly] {severity} {anomaly_type} detected (conf: {:.2}) — {details}",
                    confidence
                );
                if severity == "high" || severity == "critical" {
                    self.activity_model.push_warning(&message, None);
                } else {
                    self.activity_model.push_info(&message);
                }
            }
            UiEvent::HiconCoherence { phi, round, status } => {
                let message = format!("[hicon:coherence] Round {round}: Φ = {:.3} ({status})", phi);
                if status == "degraded" || status == "critical" {
                    self.activity_model.push_warning(&message, Some("Agent coherence below target threshold"));
                } else {
                    self.activity_model.push_info(&message);
                }
            }
            UiEvent::HiconBudgetWarning { predicted_overflow_rounds, current_tokens, projected_tokens } => {
                self.activity_model.push_warning(
                    &format!(
                        "[hicon:budget] Token overflow predicted in {predicted_overflow_rounds} rounds (current: {current_tokens}, projected: {projected_tokens})"
                    ),
                    Some("Consider reducing context tier budgets or increasing compaction frequency"),
                );
                self.toasts.push(Toast::new(
                    format!("Budget overflow in {predicted_overflow_rounds} rounds"),
                    ToastLevel::Warning,
                ));
            }

            // Context Servers Integration: Receive real server data from Repl
            UiEvent::ContextServersList { servers, total_count, enabled_count } => {
                self.state.context_servers = servers;
                self.state.context_servers_total = total_count;
                self.state.context_servers_enabled = enabled_count;
                self.status.context_servers_count = total_count;
            }

            // Phase 45B: Real-time token delta from streaming.
            UiEvent::TokenDelta { session_input, session_output, .. } => {
                self.status.apply_patch(StatusPatch {
                    input_tokens: Some(session_input),
                    output_tokens: Some(session_output),
                    ..Default::default()
                });
            }

            // Phase 45E: Session list loaded from DB.
            UiEvent::SessionList { sessions } => {
                self.session_list = sessions;
                self.session_list_selected = 0;
                self.state.overlay.open(OverlayKind::SessionList);
            }

            // --- Dev Ecosystem Phase 5: IDE/Editor connection events ---

            // LSP server started — show ○ LSP:<port> indicator in status bar.
            UiEvent::IdeConnected { port } => {
                self.status.dev_gateway_port = Some(port);
                self.status.ide_connected = false; // no buffers yet
                self.activity_model.push_info(
                    &format!("[dev] LSP server listening on localhost:{port} — connect your IDE extension"),
                );
            }

            // LSP server stopped (session teardown).
            UiEvent::IdeDisconnected => {
                self.status.dev_gateway_port = None;
                self.status.ide_connected = false;
                self.status.open_buffers = 0;
            }

            // --- Multi-Agent Orchestration Visibility ---

            UiEvent::OrchestratorWave { wave_index: _, total_waves, task_count } => {
                self.activity_model.push_orchestrator_header(task_count, total_waves);
            }

            UiEvent::SubAgentSpawned { step_index, total_steps, description, agent_type } => {
                self.activity_model.push_sub_agent_spawn(step_index, total_steps, &description, &agent_type);
            }

            UiEvent::SubAgentCompleted { step_index, total_steps: _, success, latency_ms, tools_used, rounds, summary } => {
                self.activity_model.update_sub_agent_complete(step_index, success, latency_ms, tools_used, rounds, summary);
            }

            UiEvent::MediaAnalysisStarted { count } => {
                self.activity_model.push_info(
                    &format!("[media] Analyzing {count} file{}…", if count == 1 { "" } else { "s" }),
                );
            }

            UiEvent::MediaAnalysisComplete { filename, tokens } => {
                self.activity_model.push_info(&format!("[media]   {filename}: {tokens} tokens"));
            }

            // IDE buffer count changed — update ⚡ IDE:N indicator.
            UiEvent::IdeBuffersUpdated { count, git_branch } => {
                self.status.open_buffers = count;
                self.status.ide_connected = count > 0;
                if count > 0 {
                    let branch_str = git_branch
                        .as_deref()
                        .map(|b| format!(" on {b}"))
                        .unwrap_or_default();
                    self.activity_model.push_info(
                        &format!("[dev] IDE: {count} open buffer{}{branch_str}",
                            if count == 1 { "" } else { "s" }),
                    );
                }
            }
        }
    }
}
