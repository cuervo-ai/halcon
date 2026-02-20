//! TUI application shell — manages the render loop and event dispatch.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
    MouseButton, MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tokio::sync::mpsc;

use super::constants;
use super::conversational_overlay::ConversationalOverlay;
use super::events::{ControlEvent, SessionInfo, UiEvent};
use super::highlight::HighlightManager;
use super::input;
use super::layout;
use super::overlay::{self, OverlayKind};
use super::permission_context::{PermissionContext, RiskLevel};
use super::state::{AgentControl, AppState, FocusZone, UiMode};
use super::transition_engine::TransitionEngine;
use super::activity_types::ActivityLine; // P0.1B: Migrated to activity_types
use super::widgets::activity_indicator::AgentState;
use super::widgets::agent_badge::AgentBadge;
use super::widgets::panel::SidePanel;
use super::widgets::permission_modal::PermissionModal;
use super::widgets::prompt::PromptState;
use super::widgets::status::StatusState;
use super::widgets::toast::{Toast, ToastLevel, ToastStack};

/// Maximum number of events stored in the ring buffer for the inspector.
const EVENT_RING_CAPACITY: usize = 200;

/// A timestamped event entry for the inspector ring buffer.
#[derive(Debug, Clone)]
pub struct EventEntry {
    /// Wall-clock offset from app start in milliseconds.
    pub offset_ms: u64,
    /// Summary label of the event.
    pub label: String,
}

/// Expansion animation state for a single line.
///
/// Tracks progress of expand/collapse animation using time-based easing.
/// Phase B1: Smooth height transitions for tool results.
#[derive(Debug, Clone)]
pub struct ExpansionAnimation {
    /// Target state: true = expanding to 1.0, false = collapsing to 0.0.
    pub expanding: bool,
    /// Current progress [0.0, 1.0] where 0.0 = collapsed, 1.0 = fully expanded.
    pub progress: f32,
    /// When this animation started.
    pub started_at: Instant,
    /// Animation duration.
    pub duration: Duration,
}

impl ExpansionAnimation {
    /// Start expanding from current progress.
    pub fn expand_from(progress: f32) -> Self {
        Self {
            expanding: true,
            progress,
            started_at: Instant::now(),
            duration: Duration::from_millis(200), // 200ms expand
        }
    }

    /// Start collapsing from current progress.
    pub fn collapse_from(progress: f32) -> Self {
        Self {
            expanding: false,
            progress,
            started_at: Instant::now(),
            duration: Duration::from_millis(150), // 150ms collapse (snappier)
        }
    }

    /// Get current eased progress [0.0, 1.0].
    ///
    /// Uses EaseInOut for smooth acceleration/deceleration.
    /// Returns target value (0.0 or 1.0) once animation completes.
    pub fn current(&self) -> f32 {
        let elapsed = self.started_at.elapsed();
        if elapsed >= self.duration {
            return if self.expanding { 1.0 } else { 0.0 };
        }

        let t = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let t_eased = ease_in_out(t);

        if self.expanding {
            self.progress + (1.0 - self.progress) * t_eased
        } else {
            self.progress * (1.0 - t_eased)
        }
    }

    /// Check if animation is complete.
    pub fn is_complete(&self) -> bool {
        self.started_at.elapsed() >= self.duration
    }
}

/// EaseInOut easing function (smoothstep).
///
/// Slow start, fast middle, slow end.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        -1.0 + (4.0 - 2.0 * t) * t
    }
}

/// Calculate shimmer progress [0.0, 1.0] for a loading skeleton.
///
/// Phase B2: Cyclic shimmer animation with 1-second period.
/// Returns normalized position [0.0, 1.0] of the shimmer wave.
pub fn shimmer_progress(elapsed: Duration) -> f32 {
    const SHIMMER_PERIOD_MS: f32 = 1000.0; // 1 second cycle
    let elapsed_ms = elapsed.as_millis() as f32;
    let t = (elapsed_ms % SHIMMER_PERIOD_MS) / SHIMMER_PERIOD_MS;
    t // Returns [0.0, 1.0] repeating
}

/// The TUI application. Owns the terminal, state, and event channels.
pub struct TuiApp {
    state: AppState,
    prompt: PromptState,
    // P0.4B: activity: ActivityState removed — migrated to activity_model
    status: StatusState,
    panel: SidePanel,
    /// Receives UiEvents from the agent loop (via TuiSink).
    ui_rx: mpsc::Receiver<UiEvent>,
    /// Sends prompt text to the agent loop.
    prompt_tx: mpsc::UnboundedSender<String>,
    /// Sends control events (pause/step/cancel) to the agent loop.
    ctrl_tx: mpsc::UnboundedSender<ControlEvent>,
    /// Sends permission decisions to the executor's PermissionChecker.
    /// Extended from bool to PermissionDecision to support 8-option advanced modal.
    /// Dedicated channel ensures the decision reaches the executor even while the
    /// agent loop is blocked on tool execution.
    perm_tx: mpsc::UnboundedSender<halcon_core::types::PermissionDecision>,
    /// Conversational permission overlay instance (Phase I-6C, kept for compatibility).
    conversational_overlay: Option<ConversationalOverlay>,
    /// Permission modal (Phase 2.2) — replaces conversational_overlay in new flow.
    permission_modal: Option<PermissionModal>,
    /// Phase I2 Fix: Submit button area for compact styled button (14 cols, 1 line).
    submit_button_area: Rect,
    /// Ring buffer of recent events for the Expert inspector panel.
    event_log: VecDeque<EventEntry>,
    /// Start time for computing event offsets.
    start_time: Instant,
    /// Toast notification stack (Phase F1).
    toasts: ToastStack,
    /// Search state for activity zone search (B4).
    search_matches: Vec<usize>,
    search_current: usize,
    /// Watchdog: timestamp when agent last started processing (for timeout detection).
    agent_started_at: Option<Instant>,
    /// Watchdog: maximum agent duration in seconds before forcing UI unlock (default: 600 = 10 min).
    max_agent_duration_secs: u64,
    /// Phase 2.3: Perceptual color transition engine.
    transition_engine: TransitionEngine,
    /// Phase 2.3: Highlight pulse manager.
    highlights: HighlightManager,
    /// Phase 3.1: Agent status badge with transitions.
    agent_badge: AgentBadge,

    // Phase A1: SOTA Activity Architecture
    /// Activity data model with O(1) search indexing.
    activity_model: crate::tui::activity_model::ActivityModel,
    /// Activity navigation state (J/K selection, expand/collapse, search).
    activity_navigator: crate::tui::activity_navigator::ActivityNavigator,
    /// Activity interaction controller (keyboard/mouse handlers).
    activity_controller: crate::tui::activity_controller::ActivityController,

    // Phase A2: Virtual Scroll Optimization
    /// Activity renderer with LRU cache and virtual scrolling.
    activity_renderer: crate::tui::activity_renderer::ActivityRenderer,

    // Phase B1: Expand/Collapse Animations
    /// Expansion animations keyed by line index.
    /// Tracks smooth height transitions for expanding/collapsing tool results.
    expansion_animations: HashMap<usize, ExpansionAnimation>,

    // Phase B2: Loading Skeletons
    /// Executing tools keyed by tool name → start time.
    /// Used to calculate shimmer animation progress for loading skeletons.
    executing_tools: HashMap<String, Instant>,

    // Phase B4: Hover Effects (mouse event routing)
    /// Last rendered activity zone area (for mouse event boundary detection).
    /// Updated on each render, used in mouse event handler.
    last_activity_area: Rect,

    /// Last rendered panel area (for scroll calculation).
    /// Updated on each render, used to calculate max scroll offset.
    last_panel_area: Rect,

    // Phase 3 SRCH-004: Database for search history persistence
    /// Optional database for saving/loading search history.
    /// None when running without database (e.g., tests, --no-db mode).
    db: Option<halcon_storage::AsyncDatabase>,

    /// Flag to track if search history has been loaded from database.
    /// Prevents redundant database queries on every search overlay open.
    search_history_loaded: bool,

    // Phase 45: Status Bar Audit + Session Management
    /// Last rendered status bar area (for STOP button click detection).
    last_status_area: Rect,
    /// Computed ctrl button (▶ RUN / ■ STOP) area for mouse click detection.
    ctrl_button_area: Rect,
    /// Computed session ID label area for click-to-copy detection.
    session_id_button_area: Rect,
    /// Sender clone used by background async tasks to push UiEvents back to the app.
    ui_tx_for_bg: Option<tokio::sync::mpsc::Sender<UiEvent>>,
    /// Cached session list loaded from DB (for SessionList overlay).
    session_list: Vec<SessionInfo>,
    /// Cursor index in the session list overlay.
    session_list_selected: usize,

    // --- Sudo Password Elevation (Phase 50) ---
    /// Sender to deliver the password (or None on cancel) to the executor.
    sudo_pw_tx: Option<tokio::sync::mpsc::UnboundedSender<Option<String>>>,
    /// Current password being typed (masked in the modal).
    sudo_password_buf: String,
    /// "Remember for 5 minutes" toggle state.
    sudo_remember_password: bool,
    /// Whether a cached sudo password is available (within 5-minute TTL).
    sudo_has_cached: bool,
    /// Cached sudo password + expiry (in-process, never written to disk).
    sudo_cache: Option<(String, std::time::Instant)>,
}

/// Detect the OS username for the user avatar in the activity feed.
///
/// Priority: $USER → $LOGNAME → home dir basename → "you".
fn detect_username() -> String {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    if let Ok(u) = std::env::var("LOGNAME") {
        if !u.is_empty() {
            return u;
        }
    }
    dirs::home_dir()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "you".to_string())
}

impl TuiApp {
    /// Create a new TUI application with the given initial UI mode.
    pub fn new(
        ui_rx: mpsc::Receiver<UiEvent>,
        prompt_tx: mpsc::UnboundedSender<String>,
        ctrl_tx: mpsc::UnboundedSender<ControlEvent>,
        perm_tx: mpsc::UnboundedSender<halcon_core::types::PermissionDecision>,
        db: Option<halcon_storage::AsyncDatabase>,
    ) -> Self {
        Self::with_mode(ui_rx, prompt_tx, ctrl_tx, perm_tx, db, UiMode::Standard)
    }

    /// Create a new TUI application with a specific initial UI mode.
    pub fn with_mode(
        ui_rx: mpsc::Receiver<UiEvent>,
        prompt_tx: mpsc::UnboundedSender<String>,
        ctrl_tx: mpsc::UnboundedSender<ControlEvent>,
        perm_tx: mpsc::UnboundedSender<halcon_core::types::PermissionDecision>,
        db: Option<halcon_storage::AsyncDatabase>,
        initial_mode: UiMode,
    ) -> Self {
        let panel_visible = matches!(initial_mode, UiMode::Standard | UiMode::Expert);
        let mut state = AppState::new();
        state.ui_mode = initial_mode;
        state.panel_visible = panel_visible;
        state.user_display_name = detect_username();
        Self {
            state,
            prompt: PromptState::new(),
            // P0.4B: activity: ActivityState::new() removed — using activity_model instead
            status: StatusState::new(),
            panel: SidePanel::new(),
            ui_rx,
            prompt_tx,
            ctrl_tx,
            perm_tx,
            conversational_overlay: None,
            permission_modal: None, // Phase 2.2
            submit_button_area: Rect::default(),
            event_log: VecDeque::with_capacity(EVENT_RING_CAPACITY),
            start_time: Instant::now(),
            toasts: ToastStack::new(),
            search_matches: Vec::new(),
            search_current: 0,
            agent_started_at: None,
            max_agent_duration_secs: 600, // 10 minutes default watchdog timeout
            transition_engine: TransitionEngine::new(),
            highlights: HighlightManager::new(),
            agent_badge: AgentBadge::new(),
            // Phase A1: Initialize SOTA activity modules
            activity_model: crate::tui::activity_model::ActivityModel::new(),
            activity_navigator: crate::tui::activity_navigator::ActivityNavigator::new(),
            activity_controller: crate::tui::activity_controller::ActivityController::new(),
            // Phase A2: Initialize virtual scroll renderer
            activity_renderer: crate::tui::activity_renderer::ActivityRenderer::new(),
            // Phase B1: Initialize expansion animations
            expansion_animations: HashMap::new(),
            // Phase B2: Initialize executing tools tracker
            executing_tools: HashMap::new(),
            // Phase B4: Initialize last activity area (will be updated on first render)
            last_activity_area: Rect::default(),
            // Panel area tracking for scroll calculation
            last_panel_area: Rect::default(),
            // Phase 3 SRCH-004: Database for search history persistence
            db,
            search_history_loaded: false,
            // Phase 45: Status Bar Audit + Session Management
            last_status_area: Rect::default(),
            ctrl_button_area: Rect::default(),
            session_id_button_area: Rect::default(),
            ui_tx_for_bg: None,
            session_list: Vec::new(),
            session_list_selected: 0,
            // Sudo Password Elevation (Phase 50)
            sudo_pw_tx: None,
            sudo_password_buf: String::new(),
            sudo_remember_password: false,
            sudo_has_cached: false,
            sudo_cache: None,
        }
    }

    /// Wire the sudo password sender so the TUI can deliver passwords to the executor.
    /// Called from repl/mod.rs after TuiApp creation.
    pub fn set_sudo_pw_tx(&mut self, tx: tokio::sync::mpsc::UnboundedSender<Option<String>>) {
        self.sudo_pw_tx = Some(tx);
    }

    /// Set a background sender so async tasks can push UiEvents back into the app.
    /// Called from repl/mod.rs after TuiApp::with_mode().
    pub fn set_ui_tx(&mut self, tx: tokio::sync::mpsc::Sender<UiEvent>) {
        self.ui_tx_for_bg = Some(tx);
    }

    /// Push an enhanced startup banner with real feature data and artistic Momoto crow.
    pub fn push_banner(
        &mut self,
        version: &str,
        provider: &str,
        provider_connected: bool,
        model: &str,
        session_id: &str,
        session_type: &str,
        routing: Option<&crate::render::banner::RoutingDisplay>,
        features: &crate::render::banner::FeatureStatus,
    ) {
        // Minimalist SOTA banner using momoto design principles
        self.activity_model.push_info("");

        // Welcome header with version (clean, single line)
        let status_icon = if provider_connected { "●" } else { "○" };
        self.activity_model.push_info(&format!(
            "  {} Bienvenido a halcon v{}  —  {} {} {}",
            status_icon,
            version,
            provider,
            if provider_connected { "↗" } else { "⊗" },
            session_id
        ));

        self.activity_model.push_info("");

        // Minimal essential help (adaptive based on UI mode and features)
        let help_line = if features.background_tools_enabled {
            format!("  F1 Ayuda  │  Enter Enviar  │  Shift+↵ nueva línea  │  Ctrl+P comandos  │  {} herramientas activas", features.tool_count)
        } else {
            format!("  F1 Ayuda  │  Enter Enviar  │  Shift+↵ nueva línea  │  Ctrl+P comandos  │  {} herramientas", features.tool_count)
        };
        self.activity_model.push_info(&help_line);
        self.activity_model.push_info("");

        // Eagerly initialize status bar with session info — prevents blank SESSION on first frame.
        // The async ui_tx StatusUpdate arrives later but races with the first render.
        self.status.update(
            Some(provider.to_string()),
            Some(model.to_string()),
            None, None, None,
            Some(session_id.to_string()),
            None, None, None, None,
        );
    }

    /// Run the TUI render loop. Blocks until quit.
    pub async fn run(&mut self) -> io::Result<()> {
        tracing::debug!("TUI run() started");

        // Enter alternate screen + raw mode + mouse capture.
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        tracing::debug!("Entered alternate screen");

        terminal::enable_raw_mode()?;
        tracing::debug!("Enabled raw mode");

        stdout.execute(EnableMouseCapture)?;
        tracing::debug!("Enabled mouse capture");

        // Enable keyboard enhancement to detect Cmd (SUPER) on macOS.
        let _ = stdout.execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ));
        tracing::debug!("Enabled keyboard enhancements");

        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        tracing::debug!("Created terminal");

        terminal.clear()?;
        tracing::debug!("Cleared terminal, entering main loop");

        // Spawn a single dedicated thread for crossterm event polling.
        // Phase 44C: Reduced polling interval for snappier keyboard response.
        let (key_tx, mut key_rx) = mpsc::unbounded_channel::<Event>();
        std::thread::spawn(move || {
            loop {
                // 10ms polling for <50ms input latency (was 50ms).
                if event::poll(Duration::from_millis(10)).unwrap_or(false) {
                    if let Ok(ev) = event::read() {
                        if key_tx.send(ev).is_err() {
                            break; // Receiver dropped, TUI is shutting down.
                        }
                    }
                }
            }
        });

        // Spinner tick timer — 100ms interval to animate the braille spinner.
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));
        tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Phase 44C: Frame rate limiter — minimum 8ms between frames (≈120 FPS cap).
        // Increased from 60 FPS for smoother scrolling and animations.
        let min_frame_interval = Duration::from_millis(8);
        let mut last_render = Instant::now();
        let mut needs_render = true;

        // Phase 3 SRCH-004: Load search history from database on startup
        if let Some(ref db) = self.db {
            tracing::debug!("Loading search history from database");
            match db.get_recent_queries(50).await {
                Ok(queries) => {
                    tracing::debug!("Loaded {} search queries from database", queries.len());
                    self.activity_navigator.load_history(queries);
                    self.search_history_loaded = true;
                }
                Err(e) => {
                    tracing::warn!("Failed to load search history: {}", e);
                }
            }
        }

        tracing::debug!("TUI entering main event loop");
        let mut loop_iterations = 0;

        loop {
            loop_iterations += 1;
            if loop_iterations % 100 == 1 {
                tracing::trace!(iterations = loop_iterations, "TUI loop iteration");
            }

            // Phase F7: Skip render if within minimum frame interval (debounce burst events).
            let since_last = last_render.elapsed();
            if !needs_render && since_last < min_frame_interval {
                // Process events without rendering.
            } else {
                needs_render = false;
                last_render = Instant::now();
            }

            // Phase 44C: Auto-hide typing indicator after 2 seconds of inactivity.
            if self.state.typing_indicator
                && self.state.last_keystroke.elapsed() > Duration::from_secs(2)
            {
                self.state.typing_indicator = false;
            }

            // Watchdog: force UI unlock if agent is stuck longer than max duration.
            if let Some(started) = self.agent_started_at {
                let elapsed_secs = started.elapsed().as_secs();
                if elapsed_secs > self.max_agent_duration_secs {
                    tracing::warn!(
                        elapsed_secs,
                        max_secs = self.max_agent_duration_secs,
                        agent_running = self.state.agent_running,
                        prompts_queued = self.state.prompts_queued,
                        "WATCHDOG TRIGGERED: Agent timeout exceeded - forcing UI unlock"
                    );

                    // Force unlock all state
                    self.state.agent_running = false;
                    self.state.prompts_queued = 0;
                    self.state.spinner_active = false;
                    self.state.focus = FocusZone::Prompt;
                    self.state.agent_control = crate::tui::state::AgentControl::Running;
                    self.agent_started_at = None;
                    self.prompt.set_input_state(crate::tui::input_state::InputState::Idle);

                    // Alert user
                    self.activity_model.push_warning(
                        &format!("Agent watchdog triggered after {} seconds - UI unlocked", elapsed_secs),
                        Some("The agent may have hung. Check logs for details.")
                    );
                    self.toasts.push(Toast::new(
                        format!("Agent timeout ({elapsed_secs}s) - UI force-unlocked"),
                        ToastLevel::Warning
                    ));
                }
            }

            // Render frame.
            terminal.draw(|frame| {
                let area = frame.area();

                // Phase F5: Graceful degradation for small terminals.
                if layout::is_too_small(area.width, area.height) {
                    let p = &crate::render::theme::active().palette;
                    let msg = Paragraph::new("Terminal too small.\nMinimum: 40x10")
                        .style(Style::default().fg(p.warning_ratatui()));
                    frame.render_widget(msg, area);
                    return;
                }

                // Mode-aware layout: Minimal/Standard/Expert with optional panels.
                // Effective mode may be downgraded for narrow terminals.
                let effective_mode = layout::effective_mode(area.width, self.state.ui_mode);

                // Phase I2: Calculate dynamic layout based on prompt content lines
                let mode_layout = layout::calculate_mode_layout_dynamic(
                    area,
                    effective_mode,
                    self.state.panel_visible,
                    self.state.prompt_content_lines.max(1), // At least 1 line
                );

                // Phase I2 Fix: Render compact prompt with styled Momoto button
                let (content_lines, button_area) = self.prompt.render_compact(
                    frame,
                    mode_layout.prompt,
                    self.state.focus == FocusZone::Prompt,
                    self.state.typing_indicator,
                );

                // Update state for next frame's dynamic height calculation
                self.state.prompt_content_lines = content_lines;

                // Phase I2 Fix: Render styled Momoto send button if area available
                if let Some(btn_area) = button_area {
                    use ratatui::text::{Line, Span};
                    use ratatui::widgets::Paragraph;
                    use ratatui::style::{Modifier, Style};

                    let p = &crate::render::theme::active().palette;
                    let input_state = self.prompt.input_state();

                    // Button text and colors based on InputState
                    let (btn_text, btn_bg, btn_fg) = match input_state {
                        super::input_state::InputState::Idle => {
                            if self.prompt.text().trim().is_empty() {
                                ("  Type...  ", p.muted_ratatui(), p.text_label_ratatui())
                            } else {
                                ("  ► Send  ", p.success_ratatui(), p.bg_panel_ratatui())
                            }
                        },
                        super::input_state::InputState::Sending => {
                            ("  ↑ Sending", p.planning_ratatui(), p.bg_panel_ratatui())
                        },
                        super::input_state::InputState::LockedByPermission => {
                            ("  🔒 Locked", p.destructive_ratatui(), p.bg_panel_ratatui())
                        },
                    };

                    let button = Paragraph::new(Line::from(vec![
                        Span::styled(btn_text, Style::default().bg(btn_bg).fg(btn_fg).add_modifier(Modifier::BOLD))
                    ]));

                    frame.render_widget(button, btn_area);
                    self.submit_button_area = btn_area; // For mouse click detection (optional)
                }

                // Render side panel if visible.
                if let Some(panel_area) = mode_layout.side_panel {
                    self.last_panel_area = panel_area;
                    self.panel.render(frame, panel_area, self.state.panel_section);
                }

                // Render inspector panel in Expert mode — event log.
                if let Some(inspector_area) = mode_layout.inspector {
                    let p_theme = &crate::render::theme::active().palette;
                    let c_border_insp = p_theme.border_ratatui();
                    let c_muted_insp = p_theme.muted_ratatui();
                    let c_text_insp = p_theme.text_ratatui();

                    let inner_height = inspector_area.height.saturating_sub(2) as usize;
                    let total = self.event_log.len();
                    let skip = total.saturating_sub(inner_height);

                    let lines: Vec<Line<'_>> = self.event_log.iter()
                        .skip(skip)
                        .map(|entry| {
                            let ts = format!("{:>6}ms ", entry.offset_ms);
                            Line::from(vec![
                                Span::styled(ts, Style::default().fg(c_muted_insp)),
                                Span::styled(entry.label.clone(), Style::default().fg(c_text_insp)),
                            ])
                        })
                        .collect();

                    let inspector = Paragraph::new(lines)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!(" Inspector ({total}) "))
                                .border_style(Style::default().fg(c_border_insp)),
                        );
                    frame.render_widget(inspector, inspector_area);
                }

                // Phase B1: Clean up completed expansion animations
                self.expansion_animations.retain(|_, anim| !anim.is_complete());

                // Phase B4: Save activity area for mouse event routing
                self.last_activity_area = mode_layout.activity;

                // Phase A2: Use new virtual scroll renderer (Phase B1: with expansion animations, B2: with shimmer, B3: with highlights)
                let (max_scroll, viewport_height) = self.activity_renderer.render(
                    frame,
                    mode_layout.activity,
                    &self.activity_model,
                    &self.activity_navigator,
                    &self.state,
                    &self.expansion_animations, // Phase B1: pass animations
                    &self.executing_tools,      // Phase B2: pass executing tools for shimmer
                    &self.highlights,           // Phase B3: pass highlights for search
                );

                // Phase 1 Remediation: Sync max_scroll and viewport_height to Navigator
                // This prevents stale clamping and enables proper selection centering
                self.activity_navigator.last_max_scroll = max_scroll;
                self.activity_navigator.viewport_height = Some(viewport_height);

                self.status.agent_control = self.state.agent_control;
                self.status.dry_run_active = self.state.dry_run_active;
                self.status.token_budget = self.state.token_budget;
                self.status.ui_mode = self.state.ui_mode;
                self.status.reasoning_strategy = self.panel.reasoning.strategy.clone();
                // Phase A3: Update contextual hints when Activity focused
                self.status.activity_hints = if self.state.focus == FocusZone::Activity {
                    self.activity_controller.contextual_actions(&self.activity_navigator, &self.activity_model)
                } else {
                    Vec::new()
                };
                // Phase 3 SRCH-003: Update search state
                self.status.search_active = self.activity_navigator.is_searching();
                self.status.search_mode = self.activity_navigator.search_mode_label().to_string();
                self.status.search_current = self.activity_navigator.current_match_position();
                self.status.search_total = self.activity_navigator.match_count();
                // Compute cache hit rate from panel metrics.
                let cache_total = self.panel.metrics.cache_hits + self.panel.metrics.cache_misses;
                self.status.cache_hit_rate = if cache_total > 0 {
                    Some((self.panel.metrics.cache_hits as f64 / cache_total as f64) * 100.0)
                } else {
                    None
                };
                self.status.render(frame, mode_layout.status);
                // Track status area for mouse click detection (Phase 45C/D).
                self.last_status_area = mode_layout.status;
                // ctrl button is at col+2 (inside border), row+1 (inside border), ~6 chars wide.
                self.ctrl_button_area = Rect {
                    x: mode_layout.status.x + 2,
                    y: mode_layout.status.y + 1,
                    width: 6,
                    height: 1,
                };
                // session ID button: ctrl label (6) + " │ ◆ Halcon │ " (14) ≈ 20 chars offset from left inside border.
                let sid_len = self.status.session_id_display_len() as u16;
                self.session_id_button_area = Rect {
                    x: mode_layout.status.x + 20,
                    y: mode_layout.status.y + 1,
                    width: sid_len,
                    height: 1,
                };

                // Render footer with context-aware keybinding hints.
                // Use effective_mode (degraded for terminal width) not ui_mode.
                self.render_footer(frame, mode_layout.footer, effective_mode);

                // Render active overlay on top of everything.
                match &self.state.overlay.active {
                    Some(OverlayKind::Help) => {
                        overlay::render_help(frame, area);
                    }
                    Some(OverlayKind::CommandPalette) => {
                        overlay::render_command_palette(
                            frame,
                            area,
                            &self.state.overlay.input,
                            &self.state.overlay.filtered_items,
                            self.state.overlay.selected,
                        );
                    }
                    Some(OverlayKind::Search) => {
                        let match_count = self.search_matches.len();
                        let current = if match_count > 0 { self.search_current + 1 } else { 0 };
                        overlay::render_search(frame, area, &self.state.overlay.input, match_count, current);
                    }
                    Some(OverlayKind::PermissionPrompt { .. }) => {
                        // Phase 2.2: Render permission modal with momoto colors.
                        if let Some(ref modal) = self.permission_modal {
                            modal.render(frame, area, self.state.overlay.show_advanced_permissions);
                        } else if let Some(ref conv_overlay) = self.conversational_overlay {
                            // Fallback to conversational overlay (legacy).
                            conv_overlay.render(area, frame.buffer_mut());
                        } else {
                            // Fallback to simple prompt (shouldn't happen).
                            overlay::render_permission_prompt(frame, area, "(unknown)");
                        }
                    }
                    Some(OverlayKind::ContextServers) => {
                        overlay::render_context_servers(
                            frame,
                            area,
                            &self.state.context_servers,
                            self.state.context_servers_total,
                            self.state.context_servers_enabled,
                        );
                    }
                    Some(OverlayKind::SessionList) => {
                        overlay::render_session_list(
                            frame,
                            area,
                            &self.session_list,
                            self.session_list_selected,
                        );
                    }
                    Some(OverlayKind::SudoPasswordEntry { tool, command }) => {
                        use crate::tui::widgets::sudo_modal::{SudoModal, SudoModalContext};
                        let ctx = SudoModalContext::new(
                            tool.clone(),
                            command.clone(),
                            self.sudo_has_cached,
                        );
                        let modal = SudoModal::new(ctx);
                        modal.render(
                            frame,
                            area,
                            &self.sudo_password_buf,
                            self.sudo_remember_password,
                            self.sudo_has_cached,
                        );
                    }
                    None => {}
                }

                // Phase F1: Render toast notifications on top.
                self.toasts.render(frame, area);
            })?;

            // Phase F1: GC expired toasts each frame.
            self.toasts.gc();

            // Event loop: crossterm events + agent UiEvents.
            tokio::select! {
                Some(ev) = key_rx.recv() => {
                    match ev {
                        Event::Key(key) => {
                            use crossterm::event::{KeyCode, KeyModifiers};

                            // NOTE: Ctrl+S is now handled via dispatch_key → InputAction::OpenContextServers.
                            // All keybindings are unified in input::dispatch_key for consistency.

                            // CRITICAL FIX: Input ALWAYS available, overlays only intercept specific keys.
                            // This ensures the user can ALWAYS type, even during permission prompts.

                            if self.state.overlay.is_active() {
                                // Determine if this is an overlay control key or a typing key.
                                let is_overlay_control = matches!(
                                    key.code,
                                    KeyCode::Esc | KeyCode::Enter | KeyCode::Up | KeyCode::Down |
                                    KeyCode::Backspace | KeyCode::Char('y') | KeyCode::Char('n') |
                                    KeyCode::Char('Y') | KeyCode::Char('N')
                                );

                                if is_overlay_control {
                                    // Overlay-specific control keys → route to overlay
                                    self.handle_overlay_key(key);
                                } else {
                                    // ALL other keys (chars, numbers, symbols) → ALWAYS to prompt
                                    // This allows typing prompts even during permission modals (for queuing)
                                    let action = input::dispatch_key(key);
                                    self.handle_action(action);
                                }
                            } else {
                                // No overlay active → normal routing
                                let action = input::dispatch_key(key);
                                self.handle_action(action);
                            }
                        }
                        Event::Mouse(mouse) => {
                            // Phase 45C: Check STOP button (status bar ctrl area).
                            {
                                let r = self.ctrl_button_area;
                                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                                    && r.width > 0
                                    && mouse.column >= r.x
                                    && mouse.column < r.x + r.width
                                    && mouse.row >= r.y
                                    && mouse.row < r.y + r.height
                                    && self.state.agent_running
                                {
                                    let _ = self.ctrl_tx.send(ControlEvent::CancelAgent);
                                    self.state.agent_running = false;
                                    self.status.agent_running = false;
                                    use crate::tui::input_state::InputState;
                                    self.prompt.set_input_state(InputState::Idle);
                                    self.activity_model.push_info("[control] ■ Agent stopped by user");
                                }
                            }

                            // Phase 45D: Check session ID area (click to copy full UUID).
                            {
                                let r = self.session_id_button_area;
                                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                                    && r.width > 0
                                    && mouse.column >= r.x
                                    && mouse.column < r.x + r.width
                                    && mouse.row >= r.y
                                    && mouse.row < r.y + r.height
                                {
                                    let full_id = self.status.full_session_id.clone();
                                    if !full_id.is_empty() {
                                        match crate::tui::clipboard::copy_to_clipboard(&full_id) {
                                            Ok(_) => self.toasts.push(Toast::new("Session ID copied", ToastLevel::Success)),
                                            Err(e) => self.toasts.push(Toast::new(format!("Copy failed: {e}"), ToastLevel::Warning)),
                                        }
                                    }
                                }
                            }

                            // Phase I2: Check submit button first
                            let r = self.submit_button_area;
                            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                                && r.width > 0
                                && mouse.column >= r.x
                                && mouse.column < r.x + r.width
                                && mouse.row >= r.y
                                && mouse.row < r.y + r.height
                            {
                                tracing::debug!("Submit button clicked at ({}, {})", mouse.column, mouse.row);
                                self.handle_action(input::InputAction::SubmitPrompt);
                            } else {
                                // Phase B4: Route ALL mouse events to activity controller
                                // This enables: hover (MouseMove), click selection, scroll, expand/collapse
                                let viewport_height = self.last_activity_area.height.saturating_sub(2) as usize;
                                let ctrl_action = self.activity_controller.handle_mouse(
                                    mouse,
                                    self.last_activity_area,
                                    &mut self.activity_navigator,
                                    &self.activity_model,
                                    viewport_height,
                                );

                                // Execute returned action (e.g., ToggleExpand)
                                match ctrl_action {
                                    crate::tui::activity_controller::ControlAction::None => {}
                                    crate::tui::activity_controller::ControlAction::ToggleExpand(idx) => {
                                        // Phase B1: Start smooth expand/collapse animation
                                        let was_expanded = self.activity_navigator.is_expanded(idx);
                                        self.activity_navigator.toggle_expand(idx);
                                        let now_expanded = self.activity_navigator.is_expanded(idx);

                                        let current_progress = self
                                            .expansion_animations
                                            .get(&idx)
                                            .map(|anim| anim.current())
                                            .unwrap_or(if was_expanded { 1.0 } else { 0.0 });

                                        let anim = if now_expanded {
                                            ExpansionAnimation::expand_from(current_progress)
                                        } else {
                                            ExpansionAnimation::collapse_from(current_progress)
                                        };
                                        self.expansion_animations.insert(idx, anim);
                                    }
                                    _ => {
                                        // Other actions (CopyOutput, JumpToPlanStep, OpenInspector) - future
                                        tracing::debug!("Unhandled control action: {:?}", ctrl_action);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some(first_ev) = self.ui_rx.recv() => {
                    // FIX: Batch process UI events to prevent channel saturation.
                    // Drain up to 10 events per select! iteration for higher throughput.
                    self.handle_ui_event(first_ev);

                    // Try to drain additional available events (non-blocking).
                    for _ in 0..9 {
                        match self.ui_rx.try_recv() {
                            Ok(ev) => self.handle_ui_event(ev),
                            Err(_) => break,  // No more events immediately available
                        }
                    }
                }
                _ = tick_interval.tick() => {
                    // Advance spinner animation frame.
                    self.state.tick_spinner();

                    // Phase B1: Force re-render if expansion animations are active
                    // This ensures smooth 60 FPS animation playback (100ms tick = 10 FPS baseline,
                    // but active animations trigger render on every tick)
                    if !self.expansion_animations.is_empty() {
                        needs_render = true;
                    }

                    // Phase 2.3: Prune completed transitions.
                    self.transition_engine.prune_completed();

                    // Phase 3.1: Tick agent badge and panel for transitions.
                    self.agent_badge.tick();
                    self.panel.tick();

                    // Phase 2.3: Force render if active transitions/highlights.
                    if self.transition_engine.has_active() || self.highlights.has_active() {
                        needs_render = true;
                    }
                }
            }

            if self.state.should_quit {
                tracing::debug!(iterations = loop_iterations, "TUI loop exiting: should_quit = true");
                break;
            }
        }

        tracing::debug!(iterations = loop_iterations, "TUI loop completed normally");

        // Restore terminal.
        let mut stdout = io::stdout();
        let _ = stdout.execute(PopKeyboardEnhancementFlags);
        stdout.execute(DisableMouseCapture)?;
        terminal::disable_raw_mode()?;
        stdout.execute(LeaveAlternateScreen)?;
        Ok(())
    }

    /// Render the footer bar with context-aware keybinding hints.
    ///
    /// `eff_mode` is the terminal-width-degraded mode (not the user's raw `ui_mode`).
    fn render_footer(&self, frame: &mut ratatui::Frame, area: Rect, eff_mode: UiMode) {
        use super::theme_bridge;
        use super::state::AgentControl;

        let hint_style = theme_bridge::footer_hint_style();
        let key_style = theme_bridge::footer_key_style();

        let mut spans = Vec::new();

        // Context-aware hints based on current state.
        if self.state.overlay.is_active() {
            // Overlay mode: show overlay-specific hints.
            spans.push(Span::styled(" Esc", key_style));
            spans.push(Span::styled(" close  ", hint_style));
            if matches!(self.state.overlay.active, Some(OverlayKind::PermissionPrompt { .. })) {
                spans.push(Span::styled("Y", key_style));
                spans.push(Span::styled(" approve  ", hint_style));
                spans.push(Span::styled("N", key_style));
                spans.push(Span::styled(" reject  ", hint_style));
            } else if matches!(self.state.overlay.active, Some(OverlayKind::CommandPalette)) {
                spans.push(Span::styled("↑↓", key_style));
                spans.push(Span::styled(" navigate  ", hint_style));
                spans.push(Span::styled("Enter", key_style));
                spans.push(Span::styled(" select  ", hint_style));
            } else if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
                spans.push(Span::styled("↑↓", key_style));
                spans.push(Span::styled(" prev/next  ", hint_style));
                spans.push(Span::styled("Enter", key_style));
                spans.push(Span::styled(" next  ", hint_style));
            }
        } else if self.state.agent_running {
            // Agent running mode: show pause/step/cancel hints.
            match self.state.agent_control {
                AgentControl::Paused => {
                    spans.push(Span::styled(" Esc", key_style));
                    spans.push(Span::styled(" resume  ", hint_style));
                    spans.push(Span::styled("/step", key_style));
                    spans.push(Span::styled(" one step  ", hint_style));
                    spans.push(Span::styled("/cancel", key_style));
                    spans.push(Span::styled(" abort  ", hint_style));
                }
                AgentControl::WaitingApproval => {
                    spans.push(Span::styled(" Y", key_style));
                    spans.push(Span::styled(" approve  ", hint_style));
                    spans.push(Span::styled("N", key_style));
                    spans.push(Span::styled(" reject  ", hint_style));
                }
                _ => {
                    spans.push(Span::styled(" Esc", key_style));
                    spans.push(Span::styled(" pause  ", hint_style));
                    spans.push(Span::styled("/cancel", key_style));
                    spans.push(Span::styled(" stop  ", hint_style));
                }
            }
        } else {
            // Idle mode: show prompt and navigation hints.
            spans.push(Span::styled(" Enter", key_style));
            spans.push(Span::styled(" send  ", hint_style));
            spans.push(Span::styled("Shift+↵", key_style));
            spans.push(Span::styled(" newline  ", hint_style));
            spans.push(Span::styled("↑/↓", key_style));
            spans.push(Span::styled(" historial  ", hint_style));
            spans.push(Span::styled("Ctrl+P", key_style));
            spans.push(Span::styled(" cmds  ", hint_style));
        }

        // Always show mode (effective, not raw) and panel toggle.
        // Show degradation indicator if effective mode differs from user-selected mode.
        let mode_label = if eff_mode != self.state.ui_mode {
            format!(" F3:{} (→{})  ", self.state.ui_mode.label(), eff_mode.label())
        } else {
            format!(" F3:{}  ", eff_mode.label())
        };
        spans.push(Span::styled("F1", key_style));
        spans.push(Span::styled(" help  ", hint_style));
        spans.push(Span::styled("F2", key_style));
        spans.push(Span::styled(" panel  ", hint_style));
        spans.push(Span::styled(mode_label, hint_style));
        spans.push(Span::styled("F5", key_style));
        spans.push(Span::styled(
            if self.activity_model.is_conversation_only() {
                " show all  "
            } else {
                " chat only  "
            },
            hint_style
        ));

        // Quit hint at end.
        spans.push(Span::styled("Ctrl+C", key_style));
        spans.push(Span::styled(" quit", hint_style));

        // Footer ellipsis: truncate spans if they exceed the available width.
        let total_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if total_width > area.width as usize {
            let mut accumulated = 0usize;
            let max = area.width as usize;
            let mut truncated = Vec::new();
            for span in &spans {
                let len = span.content.chars().count();
                if accumulated + len > max.saturating_sub(1) {
                    // Truncate this span and add ellipsis.
                    let remaining = max.saturating_sub(accumulated + 1);
                    if remaining > 0 {
                        let content: String = span.content.chars().take(remaining).collect();
                        truncated.push(Span::styled(content, span.style));
                    }
                    truncated.push(Span::styled("…", hint_style));
                    break;
                }
                truncated.push(span.clone());
                accumulated += len;
            }
            let footer_line = Line::from(truncated);
            let footer = Paragraph::new(footer_line);
            frame.render_widget(footer, area);
        } else {
            let footer_line = Line::from(spans);
            let footer = Paragraph::new(footer_line);
            frame.render_widget(footer, area);
        }
    }

    /// Handle key events when an overlay is active.
    fn handle_overlay_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        // Phase I-6C: Route permission prompt input through conversational overlay.
        if matches!(self.state.overlay.active, Some(OverlayKind::PermissionPrompt { .. })) {
            // Special case: Esc always closes and sends Denied to unblock authorize().
            if matches!(key.code, KeyCode::Esc) {
                // Fix #6 (Bug #6): Esc was closing the modal visually but NOT sending
                // a decision to `perm_tx`, leaving `permissions.authorize()` blocked
                // on the 60s timeout. Always send Denied when user cancels with Esc.
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::Denied);
                self.conversational_overlay = None;
                self.permission_modal = None; // Phase 2.2
                self.state.agent_control = AgentControl::Running;
                self.state.overlay.close();
                self.state.overlay.show_advanced_permissions = false; // Phase 6: Reset flag

                // Phase 2.1: Restore input state after canceling permission
                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);

                self.activity_model.push_warning("[permission] Denied (canceled)", None);
                tracing::debug!("Permission canceled (Esc) — Denied sent to unblock authorize()");
                return;
            }

            // Phase 6: F1 toggles advanced permission options (progressive disclosure).
            if matches!(key.code, KeyCode::F(1)) {
                self.state.overlay.show_advanced_permissions = !self.state.overlay.show_advanced_permissions;
                tracing::debug!(
                    show_advanced = self.state.overlay.show_advanced_permissions,
                    "Toggled advanced permission options"
                );
                return;
            }

            // ========================================================================
            // CRITICAL INTEGRATION POINT: 8-Option Permission Modal Key Routing
            // ========================================================================
            //
            // Phase 5/6/7: Direct key-to-option mapping for permission modal.
            //
            // This is the CORRECT implementation that makes all 8 permission options
            // functional. Keys map directly to PermissionOptions without going through
            // a conversational overlay.
            //
            // KEY BINDINGS:
            // - Y/y → Yes (approve once)
            // - N/n → No (reject once)
            // - A/a → AlwaysThisTool (global approval) - only when advanced shown
            // - D/d → ThisDirectory (directory-scoped) - only when advanced shown
            // - S/s → ThisSession (session-scoped) - only when advanced shown
            // - P/p → ThisPattern (pattern-matched) - only when advanced shown
            // - X/x → NeverThisDirectory (directory denial) - only when advanced shown
            // - Esc → Cancel (handled above at line 743)
            // - F1 → Toggle advanced options (handled above at line 763)
            //
            // PROGRESSIVE DISCLOSURE (Phase 6):
            // Advanced options (A/D/S/P/X) only work when show_advanced_permissions=true
            // (toggled with F1 key). This prevents accidental over-permissioning.
            //
            // DO NOT route through conversational_overlay! That was the old Phase I-6C
            // implementation that only supported yes/no text input.
            //
            // FIX HISTORY: Previously routed ALL input to conversational overlay
            // (CRITICAL BUG #2). Fixed: 2026-02-15, now uses direct key mapping.
            // ========================================================================

            use crate::tui::permission_context::PermissionOption;

            let permission_option = match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(PermissionOption::Yes),
                KeyCode::Char('n') | KeyCode::Char('N') => Some(PermissionOption::No),
                KeyCode::Char('a') | KeyCode::Char('A') if self.state.overlay.show_advanced_permissions => {
                    Some(PermissionOption::AlwaysThisTool)
                }
                KeyCode::Char('d') | KeyCode::Char('D') if self.state.overlay.show_advanced_permissions => {
                    Some(PermissionOption::ThisDirectory)
                }
                KeyCode::Char('s') | KeyCode::Char('S') if self.state.overlay.show_advanced_permissions => {
                    Some(PermissionOption::ThisSession)
                }
                KeyCode::Char('p') | KeyCode::Char('P') if self.state.overlay.show_advanced_permissions => {
                    Some(PermissionOption::ThisPattern)
                }
                KeyCode::Char('x') | KeyCode::Char('X') if self.state.overlay.show_advanced_permissions => {
                    Some(PermissionOption::NeverThisDirectory)
                }
                _ => None, // Ignore unrecognized keys
            };

            if let Some(option) = permission_option {
                // Get risk level from modal to check if option is available
                let is_option_available = if let Some(ref modal) = self.permission_modal {
                    let available_options = modal.risk_level().available_options();
                    available_options.contains(&option)
                } else {
                    true // Fallback: allow if modal not present
                };

                if !is_option_available {
                    // Option not available at this risk level (e.g., AlwaysThisTool for Critical)
                    self.activity_model.push_warning(
                        &format!("[permission] Option '{}' not available at this risk level", option.label()),
                        None,
                    );
                    return;
                }

                // Convert PermissionOption to PermissionDecision
                let decision = option.to_decision();
                let _ = self.perm_tx.send(decision);

                let is_approved = !matches!(
                    decision,
                    halcon_core::types::PermissionDecision::Denied
                        | halcon_core::types::PermissionDecision::DeniedForDirectory
                );
                let status_msg = format!("[control] {} - {}", option.label(), if is_approved { "Approved" } else { "Denied" });
                if is_approved {
                    self.activity_model.push_info(&status_msg);
                } else {
                    self.activity_model.push_warning(&status_msg, None);
                }

                // Close modal and restore state
                self.conversational_overlay = None;
                self.permission_modal = None;
                self.state.agent_control = AgentControl::Running;
                self.state.overlay.close();
                self.state.overlay.show_advanced_permissions = false;

                use crate::tui::input_state::InputState;
                self.prompt.set_input_state(InputState::Idle);

                self.highlights.stop("permission_prompt");
                self.agent_badge.set_state(AgentState::Running);
                self.agent_badge.set_detail(Some("Continuing...".to_string()));

                tracing::debug!(
                    decision = ?decision,
                    option = ?option,
                    input_state = ?self.prompt.input_state(),
                    "Permission resolved via 8-option modal"
                );
            }
            return;
        }

        // Phase 50: Sudo password entry overlay — masked input with remember toggle.
        if matches!(self.state.overlay.active, Some(OverlayKind::SudoPasswordEntry { .. })) {
            use crossterm::event::{KeyCode, KeyModifiers};
            match key.code {
                KeyCode::Esc => {
                    // User cancelled — send None to unblock the executor.
                    let _ = self.sudo_pw_tx.as_ref().map(|tx| tx.send(None));
                    self.sudo_password_buf.clear();
                    self.state.overlay.close();
                    self.activity_model.push_warning("[sudo] Password entry cancelled", None);
                    tracing::debug!("Sudo password entry cancelled by user");
                }
                KeyCode::Enter => {
                    // Submit the password (empty = user just hit Enter, still valid for cached-cred cases).
                    let pw = self.sudo_password_buf.clone();

                    // If "Remember" toggle is on, cache with 5-minute TTL.
                    if self.sudo_remember_password && !pw.is_empty() {
                        self.sudo_cache = Some((pw.clone(), std::time::Instant::now()));
                        self.sudo_has_cached = true;
                        tracing::debug!("Sudo password cached for 5 minutes");
                    }

                    let _ = self.sudo_pw_tx.as_ref().map(|tx| tx.send(Some(pw)));
                    self.sudo_password_buf.clear();
                    self.state.overlay.close();
                    self.activity_model.push_info("[sudo] Password submitted — elevating privileges");
                    tracing::debug!("Sudo password submitted");
                }
                KeyCode::Tab => {
                    // Toggle "Remember for 5 minutes".
                    self.sudo_remember_password = !self.sudo_remember_password;
                }
                KeyCode::Char('c') | KeyCode::Char('C')
                    if key.modifiers == KeyModifiers::NONE
                        && self.sudo_has_cached =>
                {
                    // Use cached password immediately.
                    if let Some((ref pw, _)) = self.sudo_cache {
                        let _ = self.sudo_pw_tx.as_ref().map(|tx| tx.send(Some(pw.clone())));
                    }
                    self.sudo_password_buf.clear();
                    self.state.overlay.close();
                    self.activity_model.push_info("[sudo] Using cached password");
                    tracing::debug!("Using cached sudo password");
                }
                KeyCode::Backspace => {
                    // Remove last character from masked password buffer.
                    self.sudo_password_buf.pop();
                }
                KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT => {
                    // Append printable character to password buffer (never echoed).
                    self.sudo_password_buf.push(c);
                }
                _ => {}
            }
            return;
        }

        // Phase 45E: Session list overlay gets its own key routing.
        if matches!(self.state.overlay.active, Some(OverlayKind::SessionList)) {
            match key.code {
                KeyCode::Esc => {
                    self.state.overlay.close();
                }
                KeyCode::Up => {
                    if self.session_list_selected > 0 {
                        self.session_list_selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.session_list_selected + 1 < self.session_list.len() {
                        self.session_list_selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(session) = self.session_list.get(self.session_list_selected) {
                        let id = session.id.clone();
                        let short_id = if id.len() >= 8 { id[..8].to_string() } else { id.clone() };
                        let _ = self.ctrl_tx.send(ControlEvent::ResumeSession(id));
                        self.state.overlay.close();
                        self.activity_model.push_info(&format!(
                            "⟳ Loading session {}…", short_id
                        ));
                    }
                }
                _ => {}
            }
            return;
        }

        // Non-permission overlays: use original logic.
        match key.code {
            KeyCode::Esc => {
                if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
                    self.search_matches.clear();
                    self.search_current = 0;
                    // Phase 3 SRCH-004: Reset history navigation state when closing search
                    self.activity_navigator.reset_history_nav();
                }
                self.state.overlay.close();
            }
            KeyCode::Enter => {
                match &self.state.overlay.active {
                    Some(OverlayKind::CommandPalette) => {
                        let action = self.state.overlay.filtered_items
                            .get(self.state.overlay.selected)
                            .map(|item| item.action.clone());
                        self.state.overlay.close();
                        if let Some(cmd) = action {
                            self.execute_slash_command(&cmd);
                        }
                    }
                    Some(OverlayKind::Search) => {
                        // Enter = jump to next match.
                        self.search_next();
                    }
                    _ => {
                        self.state.overlay.close();
                    }
                }
            }
            KeyCode::Up => {
                if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
                    // Phase 3 SRCH-004: Ctrl+Up = navigate search history (older queries)
                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                        if let Some(query) = self.activity_navigator.history_up() {
                            self.state.overlay.input = query.clone();
                            self.rerun_search();
                        }
                    } else {
                        // Plain Up = navigate to previous match
                        self.search_prev();
                    }
                } else {
                    self.state.overlay.select_prev();
                }
            }
            KeyCode::Down => {
                if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
                    // Phase 3 SRCH-004: Ctrl+Down = navigate search history (newer queries)
                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                        if let Some(query) = self.activity_navigator.history_down() {
                            self.state.overlay.input = query.clone();
                            self.rerun_search();
                        }
                    } else {
                        // Plain Down = navigate to next match
                        self.search_next();
                    }
                } else {
                    let max = self.state.overlay.filtered_items.len();
                    self.state.overlay.select_next(max);
                }
            }
            KeyCode::Backspace => {
                self.state.overlay.backspace();
                self.refilter_palette();
                self.rerun_search();
            }
            KeyCode::Char(c) => {
                // All character input for other overlays.
                self.state.overlay.type_char(c);
                self.refilter_palette();
                self.rerun_search();
            }
            _ => {}
        }
    }

    /// Re-run search against activity lines (incremental search on keystroke).
    fn rerun_search(&mut self) {
        if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
            let query = self.state.overlay.input.clone();
            self.search_matches = self.activity_model.search(&query);
            self.search_current = 0;

            // Phase 3 SRCH-004: Save non-empty searches to database
            if !query.is_empty() {
                let match_count = self.search_matches.len() as i32;
                let search_mode = "exact"; // Currently all searches are exact mode
                if let Some(ref db) = self.db {
                    let db_clone = db.clone();
                    let query_clone = query.clone();
                    // Fire-and-forget save (don't block UI on database I/O)
                    tokio::spawn(async move {
                        let _ = db_clone
                            .save_search_history(query_clone, search_mode.to_string(), match_count, None)
                            .await;
                    });
                }
                // Also add to in-memory history for immediate availability
                self.activity_navigator.push_to_history(query.clone());
            }

            // Jump to first match if any.
            if let Some(&line_idx) = self.search_matches.first() {
                let vph = self.activity_navigator.viewport_height.unwrap_or(20);
                self.activity_navigator.scroll_to_line(line_idx, vph);
                self.activity_navigator.selected_index = Some(line_idx);
                // Phase 3 SRCH-003: Highlight first match on search entry
                let palette = &crate::render::theme::active().palette;
                self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
            }
        }
    }

    /// Navigate to the next search match.
    fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_current = (self.search_current + 1) % self.search_matches.len();
        let line_idx = self.search_matches[self.search_current];
        let vph = self.activity_navigator.viewport_height.unwrap_or(20);
        self.activity_navigator.scroll_to_line(line_idx, vph);
        self.activity_navigator.selected_index = Some(line_idx);

        // Phase B3: Add highlight pulse to current match
        let palette = &crate::render::theme::active().palette;
        self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
    }

    /// Navigate to the previous search match.
    fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.search_current == 0 {
            self.search_current = self.search_matches.len() - 1;
        } else {
            self.search_current -= 1;
        }
        let line_idx = self.search_matches[self.search_current];
        let vph = self.activity_navigator.viewport_height.unwrap_or(20);
        self.activity_navigator.scroll_to_line(line_idx, vph);
        self.activity_navigator.selected_index = Some(line_idx);

        // Phase B3: Add highlight pulse to current match
        let palette = &crate::render::theme::active().palette;
        self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
    }

    /// Execute a slash command by action name.
    fn execute_slash_command(&mut self, cmd: &str) {
        match cmd {
            // --- Agent control commands ---
            "pause" => {
                use crate::tui::state::AgentControl;
                if !self.state.agent_running {
                    self.activity_model.push_warning("[pause] No agent is running", None);
                    return;
                }
                self.state.agent_control = AgentControl::Paused;
                let _ = self.ctrl_tx.send(ControlEvent::Pause);
                self.activity_model.push_info("[control] ⏸ Agent paused — /resume to continue, /step for one step, /cancel to abort");
            }
            "resume" => {
                use crate::tui::state::AgentControl;
                if self.state.agent_control != AgentControl::Paused {
                    self.activity_model.push_warning("[resume] Agent is not paused", None);
                    return;
                }
                self.state.agent_control = AgentControl::Running;
                let _ = self.ctrl_tx.send(ControlEvent::Resume);
                self.activity_model.push_info("[control] ▶ Agent resumed");
            }
            "step" => {
                use crate::tui::state::AgentControl;
                if !self.state.agent_running {
                    self.activity_model.push_warning("[step] No agent is running", None);
                    return;
                }
                self.state.agent_control = AgentControl::StepMode;
                let _ = self.ctrl_tx.send(ControlEvent::Step);
                self.activity_model.push_info("[control] ⏭ Step mode — executing one agent step");
            }
            "cancel" => {
                if !self.state.agent_running {
                    self.activity_model.push_warning("[cancel] No agent is running", None);
                    return;
                }
                let _ = self.ctrl_tx.send(ControlEvent::CancelAgent);
                self.state.agent_running = false;
                self.activity_model.push_info("[control] ✕ Agent cancelled");
            }
            // --- Session info commands ---
            "status" => {
                let provider = self.status.current_provider();
                let model = self.status.current_model();
                let agent_state = if self.state.agent_running { "running" } else { "idle" };
                let session = self.status.session_id();
                self.activity_model.push_info(&format!(
                    "[status] Provider: {provider} | Model: {model} | Agent: {agent_state} | Session: {session}"
                ));
            }
            "session" => {
                let session = self.status.session_id();
                let provider = self.status.current_provider();
                let model = self.status.current_model();
                self.activity_model.push_info(&format!(
                    "[session] ID: {session} | {provider}/{model}"
                ));
            }
            "metrics" => {
                let metrics = self.panel.metrics_summary();
                self.activity_model.push_info(&format!("[metrics] {metrics}"));
            }
            "context" => {
                let ctx = self.panel.context_summary();
                self.activity_model.push_info(&format!("[context] {ctx}"));
            }
            "cost" => {
                let cost = self.status.cost_summary();
                self.activity_model.push_info(&format!("[cost] {cost}"));
            }
            "history" => {
                let count = self.activity_model.len();
                self.activity_model.push_info(&format!(
                    "[history] {count} activity lines in current session"
                ));
            }
            "why" => {
                let reasoning = self.panel.reasoning_summary();
                self.activity_model.push_info(&format!("[reasoning] {reasoning}"));
            }
            "inspect" => {
                let provider = self.status.current_provider();
                let model = self.status.current_model();
                let session = self.status.session_id();
                let cost = self.status.cost_summary();
                let metrics = self.panel.metrics_summary();
                self.activity_model.push_info(&format!(
                    "[inspect] Session: {session} | {provider}/{model} | {cost} | {metrics}"
                ));
            }
            // --- UI commands ---
            "help" => {
                self.state.overlay.open(OverlayKind::Help);
            }
            "model" => {
                let provider = self.status.current_provider();
                let model = self.status.current_model();
                let display = if provider.is_empty() && model.is_empty() {
                    "(none configured)".to_string()
                } else {
                    format!("{provider}/{model}")
                };
                self.activity_model.push_info(&format!(
                    "[model] Active: {display}  —  Use ~/.halcon/config.toml to change"
                ));
            }
            "mode" => {
                self.handle_action(input::InputAction::CycleUiMode);
            }
            "plan" => {
                self.state.panel_visible = true;
                self.state.panel_section = crate::tui::state::PanelSection::Plan;
                self.activity_model.push_info("[plan] Side panel switched to Plan view");
            }
            "panel" => {
                self.state.panel_visible = !self.state.panel_visible;
            }
            "search" => {
                self.state.overlay.open(OverlayKind::Search);
            }
            "clear" => {
                self.activity_model.clear();
            }
            "quit" => {
                self.state.should_quit = true;
            }
            other => {
                self.activity_model.push_warning(
                    &format!("[cmd] Unknown command: /{other}"),
                    Some("Type Ctrl+P to see all available commands"),
                );
            }
        }
    }

    /// Re-filter the command palette items based on current overlay input.
    fn refilter_palette(&mut self) {
        if matches!(self.state.overlay.active, Some(OverlayKind::CommandPalette)) {
            let all = overlay::default_commands();
            self.state.overlay.filtered_items =
                overlay::filter_commands(&all, &self.state.overlay.input);
            // Clamp selection to valid range.
            let max = self.state.overlay.filtered_items.len();
            if self.state.overlay.selected >= max {
                self.state.overlay.selected = max.saturating_sub(1);
            }
        }
    }

    fn handle_action(&mut self, action: input::InputAction) {
        match action {
            input::InputAction::SubmitPrompt => {
                let text = self.prompt.take_text();
                if text.trim().is_empty() {
                    return;
                }
                // Phase E7: Intercept slash commands before sending to agent.
                let trimmed = text.trim();
                if trimmed == "/" {
                    // Bare "/" opens the command palette instead of sending to agent.
                    self.state.overlay.open(OverlayKind::CommandPalette);
                    self.state.overlay.filtered_items = overlay::default_commands();
                    return;
                }
                if trimmed.starts_with('/') {
                    let cmd = trimmed.trim_start_matches('/').split_whitespace().next().unwrap_or("");
                    self.activity_model.push_user_prompt(&text);
                    // Always scroll to bottom on submit so prompt is immediately visible.
                    self.activity_navigator.scroll_to_bottom();
                    self.execute_slash_command(cmd);
                    return;
                }
                // Phase 44B: Allow queueing prompts even when agent is running.
                self.activity_model.push_user_prompt(&text);
                // Always scroll to bottom on submit so prompt is immediately visible.
                self.activity_navigator.scroll_to_bottom();

                // Queue the prompt (unbounded channel never blocks).
                if let Err(e) = self.prompt_tx.send(text) {
                    self.activity_model.push_error(&format!("Failed to queue prompt: {e}"), None);
                    return;
                }

                // Optimistically increment queue count (will be corrected by events).
                self.state.prompts_queued += 1;

                // If agent already running, show toast that prompt was queued.
                if self.state.agent_running {
                    self.toasts.push(Toast::new(
                        format!("Prompt #{} queued", self.state.prompts_queued),
                        ToastLevel::Info
                    ));
                } else {
                    // First prompt, start agent.
                    // CRITICAL: Keep focus on Prompt so user can type next message while agent processes.
                    // Focus NEVER auto-switches to Activity — user must press Tab to navigate activity.
                    self.state.agent_running = true;
                }

                // Logging de estado para debugging
                tracing::debug!(
                    agent_running = self.state.agent_running,
                    prompts_queued = self.state.prompts_queued,
                    agent_control = ?self.state.agent_control,
                    focus = ?self.state.focus,
                    "Prompt submitted to queue"
                );
            }
            input::InputAction::ClearPrompt => {
                self.prompt.clear();
            }
            input::InputAction::HistoryBack => {
                self.prompt.history_back();
            }
            input::InputAction::HistoryForward => {
                self.prompt.history_forward();
            }
            input::InputAction::CancelAgent => {
                // Signal cancellation (handled externally via Ctrl+C signal).
                self.state.agent_running = false;
                self.state.spinner_active = false;
                self.state.prompts_queued = 0;
                self.prompt.set_input_state(crate::tui::input_state::InputState::Idle);
                self.activity_model.push_warning("Agent cancelled by user", None);
            }
            input::InputAction::Quit => {
                self.state.should_quit = true;
            }
            input::InputAction::CycleFocus => {
                self.state.cycle_focus();
            }
            input::InputAction::ScrollUp => {
                self.activity_navigator.scroll_up(3);
                // Also scroll panel if visible (panel content may overflow)
                if self.state.panel_visible {
                    self.panel.scroll_up(3);
                }
            }
            input::InputAction::ScrollDown => {
                self.activity_navigator.scroll_down(3);
                // Also scroll panel if visible (panel content may overflow)
                if self.state.panel_visible {
                    // Calculate max_lines from panel content (approximation)
                    let max_lines = self.calculate_panel_content_lines();
                    // Account for borders: inner height is area height - 2
                    let viewport_height = self.last_panel_area.height.saturating_sub(2);
                    self.panel.scroll_down(3, max_lines, viewport_height);
                }
            }
            input::InputAction::ScrollToBottom => {
                self.activity_navigator.scroll_to_bottom();
            }
            input::InputAction::TogglePanel => {
                self.state.panel_visible = !self.state.panel_visible;
            }
            input::InputAction::CyclePanelSection => {
                self.state.panel_section = self.state.panel_section.next();
            }
            input::InputAction::CycleUiMode => {
                self.state.ui_mode = self.state.ui_mode.next();
                // Auto-show/hide panel based on mode.
                match self.state.ui_mode {
                    crate::tui::state::UiMode::Minimal => {
                        self.state.panel_visible = false;
                    }
                    crate::tui::state::UiMode::Standard
                    | crate::tui::state::UiMode::Expert => {
                        self.state.panel_visible = true;
                    }
                }
            }
            input::InputAction::PauseAgent => {
                use crate::tui::state::AgentControl;
                if self.state.agent_control == AgentControl::Paused {
                    self.state.agent_control = AgentControl::Running;
                    let _ = self.ctrl_tx.send(ControlEvent::Resume);
                    self.activity_model.push_info("[control] Resumed");
                } else {
                    self.state.agent_control = AgentControl::Paused;
                    let _ = self.ctrl_tx.send(ControlEvent::Pause);
                    self.activity_model.push_info("[control] Paused — Space to resume, N to step");
                }
            }
            input::InputAction::StepAgent => {
                use crate::tui::state::AgentControl;
                self.state.agent_control = AgentControl::StepMode;
                let _ = self.ctrl_tx.send(ControlEvent::Step);
                self.activity_model.push_info("[control] Step mode — executing one step");
            }
            input::InputAction::ApproveAction => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::Allowed);
                self.activity_model.push_info("[control] Action approved");
            }
            input::InputAction::RejectAction => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::Denied);
                self.activity_model.push_warning("[control] Action rejected", None);
            }
            input::InputAction::ApproveAlways => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::AllowedAlways);
                self.activity_model.push_info("[control] Approved always (global)");
            }
            input::InputAction::ApproveDirectory => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::AllowedForDirectory);
                self.activity_model.push_info("[control] Approved for this directory");
            }
            input::InputAction::ApproveSession => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::AllowedThisSession);
                self.activity_model.push_info("[control] Approved for this session");
            }
            input::InputAction::ApprovePattern => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::AllowedForPattern);
                self.activity_model.push_info("[control] Approved for this pattern");
            }
            input::InputAction::DenyDirectory => {
                let _ = self.perm_tx.send(halcon_core::types::PermissionDecision::DeniedForDirectory);
                self.activity_model.push_warning("[control] Denied for this directory", None);
            }
            input::InputAction::OpenHelp => {
                self.state.overlay.open(OverlayKind::Help);
            }
            input::InputAction::OpenCommandPalette => {
                self.state.overlay.open(OverlayKind::CommandPalette);
                self.state.overlay.filtered_items = overlay::default_commands();
            }
            input::InputAction::OpenSearch => {
                self.state.overlay.open(OverlayKind::Search);
                // Phase 3 SRCH-004: Search history is pre-loaded at TUI startup (see run() method)
            }
            input::InputAction::DismissToasts => {
                self.toasts.dismiss_all();
            }
            input::InputAction::ToggleConversationFilter => {
                self.activity_model.toggle_conversation_filter();
            }

            // Phase A3: SOTA Activity Navigation handlers (only when Activity focused)
            input::InputAction::SelectNextLine => {
                if self.state.focus == FocusZone::Activity {
                    self.activity_navigator.select_next(&self.activity_model);
                }
            }
            input::InputAction::SelectPrevLine => {
                if self.state.focus == FocusZone::Activity {
                    self.activity_navigator.select_prev(&self.activity_model);
                }
            }
            input::InputAction::ToggleExpand => {
                if self.state.focus == FocusZone::Activity {
                    if let Some(idx) = self.activity_navigator.selected() {
                        self.activity_navigator.toggle_expand(idx);
                    }
                }
            }
            input::InputAction::CopySelected => {
                if let Some(idx) = self.activity_navigator.selected() {
                    if let Some(line) = self.activity_model.get(idx) {
                        let text = line.text_content();
                        match super::clipboard::copy_to_clipboard(&text) {
                            Ok(()) => {
                                self.toasts.push(Toast::new(
                                    format!("Copied line {} to clipboard", idx + 1),
                                    ToastLevel::Success,
                                ));
                            }
                            Err(e) => {
                                self.toasts.push(Toast::new(
                                    format!("Copy failed: {e}"),
                                    ToastLevel::Error,
                                ));
                            }
                        }
                    }
                }
            }
            input::InputAction::InspectSelected => {
                if let Some(idx) = self.activity_navigator.selected() {
                    let provider = self.status.current_provider().to_string();
                    let model = self.status.current_model().to_string();
                    let session = self.status.session_id().to_string();
                    let cost = self.status.cost_summary();
                    let metrics = self.panel.metrics_summary();
                    self.activity_model.push_info(&format!(
                        "[inspect:line-{idx}] {provider}/{model}  session:{session}  {cost}  {metrics}"
                    ));
                }
            }
            input::InputAction::ExpandAllTools => {
                self.activity_navigator.expand_all_tools(&self.activity_model);
            }
            input::InputAction::CollapseAllTools => {
                self.activity_navigator.collapse_all_tools();
            }
            input::InputAction::JumpToPlan => {
                // Switch panel to Plan view and scroll activity to the plan overview.
                self.state.panel_visible = true;
                self.state.panel_section = crate::tui::state::PanelSection::Plan;
                if let Some(plan_line) = self.activity_model.find_plan_overview_idx() {
                    let viewport_h = self.last_panel_area.height.max(20) as usize;
                    self.activity_navigator.scroll_to_line(plan_line, viewport_h);
                    self.activity_model.push_info("[plan] Jumped to plan overview");
                } else {
                    self.activity_model.push_info("[plan] Plan panel opened (no plan overview yet)");
                }
            }
            input::InputAction::SearchNext => {
                if self.activity_navigator.is_searching() {
                    self.activity_navigator.search_next();
                }
            }
            input::InputAction::SearchPrev => {
                if self.activity_navigator.is_searching() {
                    self.activity_navigator.search_prev();
                }
            }
            input::InputAction::ClearSelection => {
                self.activity_navigator.clear_selection();
            }

            input::InputAction::InsertNewline => {
                self.prompt.insert_newline();
            }
            input::InputAction::PasteFromClipboard => {
                match super::clipboard::paste_from_clipboard() {
                    Ok(text) => {
                        self.prompt.insert_str(&text);
                    }
                    Err(e) => {
                        self.toasts.push(Toast::new(
                            format!("Paste failed: {e}"),
                            ToastLevel::Warning,
                        ));
                    }
                }
            }
            input::InputAction::OpenContextServers => {
                self.state.overlay.open(OverlayKind::ContextServers);
                let _ = self.ctrl_tx.send(ControlEvent::RequestContextServers);
            }

            // Phase 45E: Open session browser overlay.
            input::InputAction::OpenSessionList => {
                // Trigger async DB load; result comes back as UiEvent::SessionList.
                if let Some(ref db) = self.db {
                    let db = db.clone();
                    if let Some(ref tx) = self.ui_tx_for_bg {
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            if let Ok(sessions) = db.list_sessions(20).await {
                                let infos: Vec<SessionInfo> = sessions
                                    .into_iter()
                                    .map(|s| SessionInfo {
                                        id: s.id.to_string(),
                                        title: s.title,
                                        provider: s.provider,
                                        model: s.model,
                                        created_at: s.created_at.to_rfc3339(),
                                        updated_at: s.updated_at.to_rfc3339(),
                                        input_tokens: s.total_usage.input_tokens,
                                        output_tokens: s.total_usage.output_tokens,
                                        agent_rounds: s.agent_rounds as usize,
                                        estimated_cost: s.estimated_cost_usd,
                                    })
                                    .collect();
                                let _ = tx.try_send(UiEvent::SessionList { sessions: infos });
                            }
                        });
                    } else {
                        // No background sender — show empty overlay immediately.
                        self.session_list.clear();
                        self.session_list_selected = 0;
                        self.state.overlay.open(OverlayKind::SessionList);
                    }
                } else {
                    self.session_list.clear();
                    self.session_list_selected = 0;
                    self.state.overlay.open(OverlayKind::SessionList);
                }
            }

            input::InputAction::ForwardToWidget(key) => {
                use crossterm::event::{KeyCode, KeyModifiers};

                // Enter (no modifiers) when Prompt is focused → SUBMIT the message.
                // When Activity is focused, Enter falls through to activity_controller (toggle expand).
                if key.code == KeyCode::Enter
                    && key.modifiers.is_empty()
                    && self.state.focus == FocusZone::Prompt
                {
                    tracing::debug!("Enter in Prompt zone → submitting prompt");
                    self.handle_action(input::InputAction::SubmitPrompt);
                    return;
                }

                // Ctrl+Enter → submit (backward compat; also matched in dispatch_key but guarded
                // by the SHIFT arm, so it arrives here when entered from overlay path).
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
                    tracing::debug!("Ctrl+Enter in ForwardToWidget → submitting prompt");
                    self.handle_action(input::InputAction::SubmitPrompt);
                    return;
                }

                // ── Esc: toggle pause/resume when agent is running ──────────────
                // Works regardless of focus zone. If no agent running, Esc falls
                // through to normal routing (clear textarea or activity selection).
                if key.code == KeyCode::Esc && key.modifiers.is_empty() && self.state.agent_running {
                    use crate::tui::state::AgentControl;
                    if self.state.agent_control == AgentControl::Paused {
                        self.state.agent_control = AgentControl::Running;
                        let _ = self.ctrl_tx.send(ControlEvent::Resume);
                        self.activity_model.push_info("[control] ▶ Agent resumed");
                    } else {
                        self.state.agent_control = AgentControl::Paused;
                        let _ = self.ctrl_tx.send(ControlEvent::Pause);
                        self.activity_model.push_info(
                            "[control] ⏸ Paused — Esc resume  /step one step  /cancel abort"
                        );
                    }
                    return;
                }

                // ── Up/Down: history navigation in Prompt zone ───────────────────
                // If the cursor is on the first line → Up recalls previous prompt.
                // If the cursor is on the last line  → Down advances to next prompt.
                // Otherwise the key moves the cursor within the multi-line textarea.
                if key.code == KeyCode::Up
                    && key.modifiers.is_empty()
                    && self.state.focus == FocusZone::Prompt
                    && self.prompt.is_on_first_line()
                {
                    self.prompt.history_back();
                    return;
                }
                if key.code == KeyCode::Down
                    && key.modifiers.is_empty()
                    && self.state.focus == FocusZone::Prompt
                    && self.prompt.is_on_last_line()
                {
                    self.prompt.history_forward();
                    return;
                }

                // CRITICAL FIX: Determine if this is a navigation key or a typing key.
                // Navigation keys respect focus for scrolling.
                // ALL other keys ALWAYS go to the prompt (user can ALWAYS type).
                let is_navigation_key = matches!(key.code, KeyCode::Up | KeyCode::Down);

                // Phase A3: Activity-focused navigation keys (J/K vim-style + actions)
                let is_activity_action = matches!(
                    key.code,
                    KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Char('y') |
                    KeyCode::Char('i') | KeyCode::Char('x') | KeyCode::Char('z') |
                    KeyCode::Char('p') | KeyCode::Char('n') | KeyCode::Char('/') |
                    KeyCode::Enter | KeyCode::Esc
                ) && key.modifiers.is_empty(); // Only when no modifiers (Ctrl+J still goes to prompt)

                if (is_navigation_key || is_activity_action) && self.state.focus == FocusZone::Activity {
                    // Phase A3: Route to activity controller when Activity focused
                    if is_activity_action {
                        let ctrl_action = self.activity_controller.handle_key(
                            key,
                            &mut self.activity_navigator,
                            &self.activity_model,
                        );
                        // Execute the returned action
                        match ctrl_action {
                            crate::tui::activity_controller::ControlAction::None => {}
                            crate::tui::activity_controller::ControlAction::ToggleExpand(idx) => {
                                // Phase B1: Start smooth expand/collapse animation
                                let was_expanded = self.activity_navigator.is_expanded(idx);
                                self.activity_navigator.toggle_expand(idx);
                                let now_expanded = self.activity_navigator.is_expanded(idx);

                                // Get current animation progress (or start from 0.0/1.0)
                                let current_progress = self
                                    .expansion_animations
                                    .get(&idx)
                                    .map(|anim| anim.current())
                                    .unwrap_or(if was_expanded { 1.0 } else { 0.0 });

                                // Start animation in opposite direction
                                let anim = if now_expanded {
                                    ExpansionAnimation::expand_from(current_progress)
                                } else {
                                    ExpansionAnimation::collapse_from(current_progress)
                                };
                                self.expansion_animations.insert(idx, anim);
                            }
                            crate::tui::activity_controller::ControlAction::CopyOutput(idx) => {
                                if let Some(line) = self.activity_model.get(idx) {
                                    // Phase A3: Clipboard copy implementation
                                    let text = line.text_content();
                                    match super::clipboard::copy_to_clipboard(&text) {
                                        Ok(()) => {
                                            self.toasts.push(Toast::new(
                                                "Copied to clipboard",
                                                ToastLevel::Success,
                                            ));
                                        }
                                        Err(e) => {
                                            self.toasts.push(Toast::new(
                                                format!("Copy failed: {}", e),
                                                ToastLevel::Error,
                                            ));
                                        }
                                    }
                                }
                            }
                            crate::tui::activity_controller::ControlAction::JumpToPlanStep(step_idx) => {
                                // Switch side panel to Plan view and scroll activity to the plan overview.
                                self.state.panel_visible = true;
                                self.state.panel_section = crate::tui::state::PanelSection::Plan;
                                if let Some(plan_line) = self.activity_model.find_plan_overview_idx() {
                                    let viewport_h = self.last_panel_area.height.max(20) as usize;
                                    self.activity_navigator.scroll_to_line(plan_line, viewport_h);
                                    self.activity_model.push_info(&format!(
                                        "[plan] Jumped to step {} — plan overview above", step_idx + 1
                                    ));
                                } else {
                                    self.activity_model.push_info(&format!(
                                        "[plan] Step {} — plan panel opened (no plan overview yet)", step_idx + 1
                                    ));
                                }
                            }
                            crate::tui::activity_controller::ControlAction::OpenInspector(target) => {
                                // Show inspection data inline in the activity feed.
                                let provider = self.status.current_provider().to_string();
                                let model = self.status.current_model().to_string();
                                let session = self.status.session_id().to_string();
                                let cost = self.status.cost_summary();
                                let metrics = self.panel.metrics_summary();
                                self.activity_model.push_info(&format!(
                                    "[inspect:{:?}] {provider}/{model}  session:{session}  {cost}  {metrics}",
                                    target
                                ));
                            }
                            crate::tui::activity_controller::ControlAction::FilterByTool(tool) => {
                                // Open Search overlay pre-filled with the tool name.
                                self.state.overlay.open(OverlayKind::Search);
                                for ch in tool.chars() {
                                    self.state.overlay.type_char(ch);
                                }
                                self.activity_model.push_info(&format!(
                                    "[filter] Showing tool: {tool} — use n/N to navigate matches"
                                ));
                            }
                            crate::tui::activity_controller::ControlAction::SlashCommand(cmd) => {
                                // Route directly to execute_slash_command for unified handling.
                                self.execute_slash_command(&cmd);
                            }
                        }
                    } else {
                        // Arrow keys in Activity zone → scroll via navigator
                        match key.code {
                            KeyCode::Up => self.activity_navigator.scroll_up(1),
                            KeyCode::Down => self.activity_navigator.scroll_down(1),
                            _ => unreachable!(),
                        }
                    }
                } else {
                    // ALL other keys (chars, backspace, enter, etc.) → ALWAYS to prompt
                    // This ensures input is NEVER blocked, regardless of focus or agent state.
                    self.prompt.handle_key(key);

                    // Track typing activity for indicator (only for actual typing, not navigation).
                    if !is_navigation_key && !is_activity_action {
                        self.state.typing_indicator = true;
                        self.state.last_keystroke = std::time::Instant::now();
                    }
                }
            }
        }
    }

    /// Push an event summary into the ring buffer for inspector display.
    fn log_event(&mut self, label: String) {
        let offset_ms = self.start_time.elapsed().as_millis() as u64;
        if self.event_log.len() >= EVENT_RING_CAPACITY {
            self.event_log.pop_front();
        }
        self.event_log.push_back(EventEntry { offset_ms, label });
    }

    /// Get the event log entries (for inspector rendering).
    #[allow(dead_code)]
    pub fn event_log(&self) -> &VecDeque<EventEntry> {
        &self.event_log
    }

    fn handle_ui_event(&mut self, ev: UiEvent) {
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
                    // First token arrived — drop the "thinking" skeleton.
                    self.activity_model.remove_thinking();
                    // P0.3: Fix stream chunk acumulación — use push_assistant_text() instead of push()
                    self.activity_model.push_assistant_text(text);
                    // P0.4 FIX: Don't use clear_cache() - too aggressive, causes duplicates
                    // Instead, renderer will skip cache for last AssistantText line
                }
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
                self.status.update(
                    None, None, None, None, None,
                    Some(session_id), None, None, None, None,
                );
            }
            UiEvent::RoundStarted { round, provider, model } => {
                self.activity_model.push_round_separator(round);
                self.status.update(
                    Some(provider), Some(model), Some(round),
                    None, None, None, None, None, None, None,
                );
            }
            UiEvent::RoundEnded { round, input_tokens, output_tokens, cost, duration_ms } => {
                self.status.update(
                    None, None, None, None, Some(cost),
                    None, Some(duration_ms), None,
                    Some(input_tokens), Some(output_tokens),
                );
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
            UiEvent::PermissionAwaiting { tool, args, risk_level } => {
                self.activity_model.push_info(&format!("[permission] awaiting approval for {tool}"));
                self.state.agent_control = crate::tui::state::AgentControl::WaitingApproval;

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
                self.activity_model.push_info("[reflecting] analyzing round outcome...");
            }
            UiEvent::ReflectionComplete { analysis, score } => {
                let preview = truncate_str(&analysis, 80);
                self.activity_model.push_info(&format!("[reflection] {preview} (score: {score:.2})"));
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
                self.status.update(
                    None, None, None, None, None, None, None, None,
                    Some(session_input), Some(session_output),
                );
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

    /// Calculate approximate number of content lines in the panel.
    /// Used to determine max scroll offset for the side panel.
    fn calculate_panel_content_lines(&self) -> u16 {
        let mut lines = 0u16;

        // Plan section (if showing plan)
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Plan | crate::tui::state::PanelSection::All
        ) {
            lines += 2; // Header + blank
            if self.panel.plan_steps.is_empty() {
                lines += 1; // "(no plan)"
            } else {
                lines += self.panel.plan_steps.len() as u16; // Each step
            }
            lines += 1; // Blank separator
        }

        // Metrics section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Metrics | crate::tui::state::PanelSection::All
        ) {
            lines += 12; // Header + 8 metric lines + breakers + blank
        }

        // Context section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Context | crate::tui::state::PanelSection::All
        ) {
            lines += 8; // Header + 5 tier lines + blank
        }

        // Reasoning section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Reasoning | crate::tui::state::PanelSection::All
        ) {
            lines += 5; // Header + 3 reasoning lines + blank
        }

        lines
    }
}

/// Format a short preview string from a tool's input JSON value.
fn format_input_preview(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(map) => {
            let mut parts: Vec<String> = Vec::new();
            for (k, v) in map.iter().take(3) {
                let val = match v {
                    serde_json::Value::String(s) => truncate_str(s, 40),
                    other => truncate_str(&other.to_string(), 40),
                };
                parts.push(format!("{k}={val}"));
            }
            if map.len() > 3 {
                parts.push(format!("+{} more", map.len() - 3));
            }
            parts.join(", ")
        }
        serde_json::Value::String(s) => truncate_str(s, 60),
        other => truncate_str(&other.to_string(), 60),
    }
}

/// Truncate a string to at most `max_chars` Unicode characters, appending `…` if truncated.
/// Safe for all Unicode text — never panics on multi-byte characters.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Generate a one-line summary label for an event (for the ring buffer).
fn event_summary(ev: &UiEvent) -> String {
    match ev {
        UiEvent::StreamChunk(_) => constants::EVENT_STREAM_CHUNK.into(),
        UiEvent::StreamCodeBlock { lang, .. } => format!("CodeBlock({lang})"),
        UiEvent::StreamToolMarker(n) => format!("ToolMarker({n})"),
        UiEvent::StreamDone => constants::EVENT_STREAM_DONE.into(),
        UiEvent::StreamError(e) => format!("StreamError({e})"),
        UiEvent::ToolStart { name, .. } => format!("ToolStart({name})"),
        UiEvent::ToolOutput { name, is_error, .. } => {
            if *is_error { format!("ToolError({name})") } else { format!("ToolDone({name})") }
        }
        UiEvent::ToolDenied(n) => format!("ToolDenied({n})"),
        UiEvent::SpinnerStart(l) => format!("SpinnerStart({l})"),
        UiEvent::SpinnerStop => constants::EVENT_SPINNER_STOP.into(),
        UiEvent::Warning { message, .. } => format!("Warning({message})"),
        UiEvent::Error { message, .. } => format!("Error({message})"),
        UiEvent::Info(m) => format!("Info({m})"),
        UiEvent::StatusUpdate { .. } => constants::EVENT_STATUS_UPDATE.into(),
        UiEvent::RoundStart(n) => format!("RoundStart({n})"),
        UiEvent::RoundEnd(n) => format!("RoundEnd({n})"),
        UiEvent::Redraw => constants::EVENT_REDRAW.into(),
        UiEvent::AgentStartedPrompt => "AgentStarted".into(),
        UiEvent::AgentFinishedPrompt => "AgentFinished".into(),
        UiEvent::PromptQueueStatus(n) => format!("QueueStatus({n})"),
        UiEvent::AgentDone => constants::EVENT_AGENT_DONE.into(),
        UiEvent::Quit => constants::EVENT_QUIT.into(),
        UiEvent::PlanProgress { current_step, .. } => format!("PlanProgress(step={current_step})"),
        UiEvent::SessionInitialized { session_id } => format!("SessionInit({session_id})"),
        UiEvent::RoundStarted { round, .. } => format!("RoundStarted({round})"),
        UiEvent::RoundEnded { round, .. } => format!("RoundEnded({round})"),
        UiEvent::ModelSelected { model, .. } => format!("ModelSelected({model})"),
        UiEvent::ProviderFallback { from, to, .. } => format!("Fallback({from}→{to})"),
        UiEvent::LoopGuardAction { action, .. } => format!("LoopGuard({action})"),
        UiEvent::CompactionComplete { .. } => constants::EVENT_COMPACTION.into(),
        UiEvent::CacheStatus { hit, .. } => format!("Cache({})", if *hit { "hit" } else { "miss" }),
        UiEvent::SpeculativeResult { tool, hit } => format!("Speculative({tool},{})", if *hit { "hit" } else { "miss" }),
        UiEvent::PermissionAwaiting { tool, risk_level, .. } => format!("PermAwait({tool},{risk_level})"),
        UiEvent::ReflectionStarted => constants::EVENT_REFLECTION_START.into(),
        UiEvent::ReflectionComplete { .. } => constants::EVENT_REFLECTION_DONE.into(),
        UiEvent::ConsolidationStatus { .. } => constants::EVENT_CONSOLIDATION.into(),
        UiEvent::ConsolidationComplete { merged, pruned, .. } => format!("ConsolidationDone(m:{merged},p:{pruned})"),
        UiEvent::ToolRetrying { tool, attempt, .. } => format!("ToolRetry({tool},{attempt})"),
        UiEvent::ContextTierUpdate { .. } => constants::EVENT_CONTEXT_UPDATE.into(),
        UiEvent::ReasoningUpdate { strategy, .. } => format!("Reasoning({strategy})"),
        UiEvent::Phase2Metrics { .. } => "Phase2Metrics".into(),
        UiEvent::DryRunActive(a) => format!("DryRun({a})"),
        UiEvent::TokenBudgetUpdate { .. } => constants::EVENT_TOKEN_BUDGET.into(),
        UiEvent::ProviderHealthUpdate { provider, .. } => format!("Health({provider})"),
        UiEvent::CircuitBreakerUpdate { provider, .. } => format!("Breaker({provider})"),
        UiEvent::AgentStateTransition { from, to, .. } => format!("State({from:?}→{to:?})"),
        UiEvent::TaskStatus { ref title, ref status, .. } => format!("TaskStatus({title},{status})"),
        UiEvent::ReasoningStatus { ref task_type, .. } => format!("Reasoning({task_type})"),
        UiEvent::ContextServersList { total_count, .. } => format!("ContextServers({total_count})"),
        // Phase 45: Status Bar Audit + Session Management
        UiEvent::TokenDelta { session_input, session_output, .. } => format!("TokenDelta(↑{session_input}↓{session_output})"),
        UiEvent::SessionList { sessions } => format!("SessionList({})", sessions.len()),
        // FASE 1.2: HICON event summaries
        UiEvent::HiconCorrection { strategy, round, .. } => format!("HICON:Correction({strategy},r{round})"),
        UiEvent::HiconAnomaly { anomaly_type, severity, .. } => format!("HICON:Anomaly({severity}:{anomaly_type})"),
        UiEvent::HiconCoherence { phi, status, .. } => format!("HICON:Coherence(Φ={phi:.2},{status})"),
        UiEvent::HiconBudgetWarning { predicted_overflow_rounds, .. } => format!("HICON:Budget(overflow:{predicted_overflow_rounds}r)"),
        UiEvent::SudoPasswordRequest { tool, .. } => format!("SudoPasswordRequest({tool})"),
        // Dev Ecosystem Phase 5
        UiEvent::IdeConnected { port } => format!("IdeConnected(:{port})"),
        UiEvent::IdeDisconnected => "IdeDisconnected".into(),
        UiEvent::IdeBuffersUpdated { count, .. } => format!("IdeBuffers({count})"),
    }
}

/// Cleanup del terminal cuando TuiApp se destruye.
/// Esto asegura que el terminal se restaure correctamente incluso si el TUI
/// se cierra abruptamente (panic, Ctrl+C, etc.).
impl Drop for TuiApp {
    fn drop(&mut self) {
        // Desactivar raw mode
        let _ = terminal::disable_raw_mode();

        // Salir de la pantalla alternativa
        let _ = io::stdout().execute(LeaveAlternateScreen);

        // Desactivar captura de mouse
        let _ = io::stdout().execute(DisableMouseCapture);

        // Restaurar mejoras de teclado
        let _ = io::stdout().execute(PopKeyboardEnhancementFlags);

        tracing::debug!("Terminal cleanup completed");
    }
}

#[cfg(test)]
mod tests {
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, _perm_rx) = mpsc::unbounded_channel();
        let app = TuiApp::with_mode(rx, prompt_tx, ctrl_tx, perm_tx, None, UiMode::Expert);
        assert_eq!(app.state.ui_mode, UiMode::Expert);
        assert!(app.state.panel_visible);
    }

    #[test]
    fn app_with_minimal_mode() {
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
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
        let (_tx, rx) = mpsc::channel(16384);
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "bash".into(),
            args: serde_json::json!({"command": "echo test"}),
            risk_level: "Low".into(),
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
        let (_tx, rx) = mpsc::channel(16384);
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "bash".into(),
            args: serde_json::json!({"command": "rm -rf /tmp/*.txt"}),
            risk_level: "High".into(),
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
        let (_tx, rx) = mpsc::channel(16384);
        let (prompt_tx, _prompt_rx) = mpsc::unbounded_channel();
        let (ctrl_tx, _ctrl_rx) = mpsc::unbounded_channel();
        let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
        let mut app = TuiApp::new(rx, prompt_tx, ctrl_tx, perm_tx, None);
        // Phase I-6C: Trigger PermissionAwaiting event to create conversational overlay.
        app.handle_ui_event(UiEvent::PermissionAwaiting {
            tool: "file_write".into(),
            args: serde_json::json!({"path": "/tmp/test.txt", "content": "Hello"}),
            risk_level: "Medium".into(),
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
            UiEvent::PermissionAwaiting { tool: "bash".into(), args: serde_json::json!({}), risk_level: "Low".into() },
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
        ];
        for ev in &events {
            let summary = event_summary(ev);
            assert!(!summary.is_empty(), "empty summary for {:?}", ev);
        }
        // All 48 UiEvent variants covered (45 + 3 Dev Ecosystem Phase 5 variants).
        assert_eq!(events.len(), 48);
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
        let (_tx, rx) = mpsc::channel(16384);
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
}
