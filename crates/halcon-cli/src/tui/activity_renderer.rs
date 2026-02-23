//! Virtual scroll renderer for activity zone.
//!
//! **Phase A2: Virtual Scroll — Performance Optimization**
//!
//! Renders only visible lines in viewport instead of all lines.
//! Uses LRU cache for parsed markdown spans to avoid re-parsing per frame.
//!
//! Target: <2ms rendering time for 500 lines (vs ~6ms without virtual scroll).
//!
//! **Visual Redesign (Phase 44B):**
//! - Chip-based Info/Warning indicators replacing bracket notation
//! - Compact round badges instead of verbose separators
//! - Inline horizontal plan flow (collapsed PlanOverview)
//! - Cleaner ToolExec cards (⟳ loading, compact completed)
//! - Accurate line counting for scroll/scrollbar
//! - Removed TOP/BOTTOM scroll indicators (position shown in title)

use std::collections::HashMap;
use std::time::Instant;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Frame;

use crate::render::theme;
use crate::tui::state::AppState;
use super::activity_model::ActivityModel;
use super::activity_navigator::ActivityNavigator;
use super::app::{ExpansionAnimation, shimmer_progress}; // Phase B1, B2
use super::activity_types::ActivityLine;

// ── Chip classifiers (module-level so count_rendered_lines can use them) ──────

/// Classify an Info line for chip-style rendering.
///
/// Returns `(chip_char, rest)`:
/// - `None` → suppress this line entirely (e.g. `[model]`, redundant in status bar)
/// - `Some('·')` → plain subtle text (no prefix)
/// - `Some(ch)` → render as `  {ch} {rest}` with accent chip + muted text
fn classify_info_chip(text: &str) -> (Option<char>, &str) {
    // Suppress model selection — already visible in status bar
    if text.starts_with("[model]") {
        return (None, "");
    }

    // Named chip patterns (prefix → icon)
    const CHIPS: &[(&str, char)] = &[
        // Agent lifecycle / state
        ("[state] ",             '→'),
        ("[health] ",            '◉'),
        ("[control] ",           '⊞'),
        ("[planning] ",          '⊡'),
        ("[evaluation] ",        '◈'),
        ("[strategy] ",          '◉'),
        ("[step] ",              '▸'),
        ("[delegation] ",        '⇢'),
        ("[replan] ",            '↺'),
        ("[dry-run] ",           '⊘'),
        // Tools / execution
        ("[tool] ",              '⊟'),
        ("[tool_call] ",         '⊟'),
        // Infrastructure
        ("[guard] ",             '⊕'),
        ("[compaction] ",        '⊙'),
        ("[memory] ",            '◈'),
        ("[reflecting] ",        '◎'),
        ("[reflection] ",        '◎'),
        ("[cache hit] ",         '≋'),
        ("[cache miss] ",        '≋'),
        ("[speculative hit] ",   '◇'),
        ("[speculative miss] ",  '◇'),
        ("[permission] ",        '⚐'),
        // HICON subsystem
        ("[hicon:correct] ",     '⬡'),
        ("[hicon:φ] ",           'Φ'),
        ("[hicon:budget] ",      '◈'),
        ("[hicon:anomaly] ",     '⬡'),
        ("[hicon:",              '⬡'),
        // Higher-level
        ("[reasoning] ",         '◉'),
        ("[task] ",              '▣'),
        ("[context] ",           '⊟'),
        ("[round] ",             '○'),
        // Planning V3 convergence signals
        ("[convergence] ",       '⊙'),
        // Multi-agent orchestration
        ("[orchestrator] ",      '⬡'),   // hexagonal cluster — orchestration
        ("[sub-agent] ",         '◎'),   // concentric circles — sub-agent instance
        // Multimodal analysis
        ("[media] ",             '◫'),   // film frame — multimodal image/audio analysis
    ];

    for (prefix, chip) in CHIPS {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (Some(*chip), rest);
        }
    }

    // Default: plain subtle bullet
    (Some('·'), text)
}

/// Classify a Warning line for chip-style rendering.
///
/// Returns `(chip_char, display_text)` where chip replaces the `⚠` prefix.
fn classify_warning_chip(message: &str) -> (char, &str) {
    const CHIPS: &[(&str, char)] = &[
        ("[retry] ",         '↻'),
        ("[guard] ",         '⊕'),
        ("[hicon:anomaly] ", '⬡'),
        ("[hicon:budget] ",  '◈'),
        ("Tool denied: ",    '✕'),
    ];

    for (prefix, chip) in CHIPS {
        if let Some(rest) = message.strip_prefix(prefix) {
            return (*chip, rest);
        }
    }

    // Provider fallback (emitted with ⇄ prefix from app.rs already)
    if message.starts_with("⇄ ") {
        return ('⇄', message);
    }

    ('⚠', message)
}

// ── Macro step line parser ──────────────────────────────────────────────────────

/// Parse a Planning V3 macro-step feedback line: `[N/M] {rest}`.
///
/// Matches lines emitted by `MacroPlanView::format_start()` and `MacroStep::done_line()`.
///
/// # Returns
/// `Some((n, m, rest))` where `rest` is the text after `[N/M] `, e.g. `"✓ Description"`.
/// `None` if the text does not start with a valid `[N/M]` counter.
fn parse_macro_step_prefix(text: &str) -> Option<(usize, usize, &str)> {
    let inner = text.strip_prefix('[')?;
    let slash = inner.find('/')?;
    let n: usize = inner.get(..slash)?.parse().ok()?;
    let after_slash = inner.get(slash + 1..)?;
    let bracket = after_slash.find(']')?;
    let m: usize = after_slash.get(..bracket)?.parse().ok()?;
    // Sanity: n in [1, m+1] and m >= 1
    if n == 0 || m == 0 || n > m + 1 {
        return None;
    }
    let rest = after_slash.get(bracket + 1..)?.trim_start_matches(' ');
    Some((n, m, rest))
}

// ── LRU span cache ─────────────────────────────────────────────────────────────

/// LRU cache for parsed markdown spans.
///
/// Key: line index, Value: Vec<Span> (cached parsed markdown).
/// Evicts least-recently-used entries when capacity exceeded.
pub struct SpanCache {
    cache: HashMap<usize, Vec<Span<'static>>>,
    access_order: Vec<usize>,
    max_capacity: usize,
}

impl SpanCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(capacity),
            access_order: Vec::with_capacity(capacity),
            max_capacity: capacity,
        }
    }

    /// Get cached spans for a line index.
    /// Updates LRU access order on cache hit.
    pub fn get(&mut self, line_idx: usize) -> Option<&Vec<Span<'static>>> {
        if self.cache.contains_key(&line_idx) {
            // Update access order (move to end = most recently used)
            self.access_order.retain(|&idx| idx != line_idx);
            self.access_order.push(line_idx);
            self.cache.get(&line_idx)
        } else {
            None
        }
    }

    /// Insert spans into cache for a line index.
    /// Evicts LRU entry if capacity exceeded.
    pub fn insert(&mut self, line_idx: usize, spans: Vec<Span<'static>>) {
        // Evict LRU if at capacity
        if self.cache.len() >= self.max_capacity && !self.cache.contains_key(&line_idx) {
            if let Some(&lru_idx) = self.access_order.first() {
                self.cache.remove(&lru_idx);
                self.access_order.remove(0);
            }
        }

        // Insert new entry
        self.cache.insert(line_idx, spans);

        // Update access order
        self.access_order.retain(|&idx| idx != line_idx);
        self.access_order.push(line_idx);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.access_order.clear();
    }

    /// Get cache hit rate (for diagnostics).
    #[allow(dead_code)]
    pub fn hit_rate(&self) -> f64 {
        // Would need hit/miss counters for accurate rate
        // For now, return cache fill ratio as proxy
        self.cache.len() as f64 / self.max_capacity as f64
    }
}

// ── ActivityRenderer ───────────────────────────────────────────────────────────

/// Virtual scroll renderer for activity zone.
///
/// Renders only visible lines to optimize performance.
/// Uses LRU cache to avoid re-parsing markdown per frame.
pub struct ActivityRenderer {
    /// LRU cache for parsed markdown spans (capacity: 200 lines).
    span_cache: SpanCache,
}

impl ActivityRenderer {
    /// Create a new renderer with default cache capacity (200 lines).
    pub fn new() -> Self {
        Self {
            span_cache: SpanCache::new(200),
        }
    }

    /// Create a renderer with custom cache capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            span_cache: SpanCache::new(capacity),
        }
    }

    /// Render the activity zone with virtual scrolling.
    ///
    /// Only renders lines visible in the viewport (scroll_offset to scroll_offset + viewport_height).
    /// Uses cached spans when available to avoid markdown re-parsing.
    ///
    /// Phase B1: Accepts expansion_animations for smooth expand/collapse height transitions.
    /// Phase B2: Accepts executing_tools for dynamic shimmer loading skeletons.
    /// Phase B3: Accepts highlights for search match fade-in/fade-out animations.
    ///
    /// **Phase 1 Remediation**: Returns (max_scroll, viewport_height) for Navigator sync.
    /// Caller must update `navigator.last_max_scroll` to prevent stale clamping.
    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        model: &ActivityModel,
        nav: &ActivityNavigator,
        state: &AppState,
        expansion_animations: &HashMap<usize, ExpansionAnimation>, // Phase B1
        executing_tools: &HashMap<String, Instant>,                // Phase B2
        highlights: &crate::tui::highlight::HighlightManager,      // Phase B3
    ) -> (usize, usize) {
        let p = &theme::active().palette;
        // Cache ratatui colors (eliminates OKLCH→sRGB conversions per line)
        let c_success = p.success_ratatui();
        let c_accent = p.accent_ratatui();
        let c_warning = p.warning_ratatui();
        let c_error = p.error_ratatui();
        let c_running = p.running_ratatui();
        let c_text = p.text_ratatui();
        let c_muted = p.muted_ratatui();
        let c_border = p.border_ratatui();
        let c_spinner = p.spinner_color_ratatui();

        let border_color = if state.focus == super::state::FocusZone::Activity {
            c_accent
        } else {
            c_border
        };

        // Calculate viewport bounds
        let viewport_height = area.height.saturating_sub(2) as usize; // -2 for borders
        let total_lines = self.count_rendered_lines(model, nav, state);
        let max_scroll = total_lines.saturating_sub(viewport_height);

        // Determine scroll offset (auto-scroll or manual)
        let scroll = if nav.auto_scroll {
            max_scroll
        } else {
            nav.scroll_offset.min(max_scroll)
        };

        // Virtual scroll: only render visible lines (Phase B1: with animations, B2: with shimmer, B3: with highlights)
        let visible_lines = self.viewport_lines(
            model,
            nav,
            state,
            scroll,
            viewport_height,
            expansion_animations, // Phase B1
            executing_tools,      // Phase B2
            highlights,           // Phase B3
            c_success,
            c_accent,
            c_warning,
            c_error,
            c_running,
            c_text,
            c_muted,
            c_spinner,
        );

        // Phase 2 VIZ-001: Dynamic title showing scroll position when content overflows
        let title = if total_lines > viewport_height {
            let start = scroll + 1; // 1-indexed for UX
            let end = (scroll + viewport_height).min(total_lines);
            format!(" Activity ({}-{} / {}) ", start, end, total_lines)
        } else {
            " Activity ".to_string()
        };

        let paragraph = Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(border_color)),
            )
            .wrap(Wrap { trim: true }); // Word-wrap intelligently (by words, not chars) and trim whitespace

        frame.render_widget(paragraph, area);

        // Render scrollbar if content exceeds viewport
        if total_lines > viewport_height {
            let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }

        // Phase 1 Remediation: Return (max_scroll, viewport_height) for Navigator sync
        (max_scroll, viewport_height)
    }

    /// Get lines visible in the current viewport.
    ///
    /// Virtual scroll optimization: only processes lines in [scroll_offset, scroll_offset + viewport_height].
    /// Uses LRU cache for parsed markdown spans.
    /// Phase B1: Uses expansion_animations for smooth height transitions.
    /// Phase B2: Uses executing_tools for dynamic shimmer loading skeletons.
    /// Phase B3: Uses highlights for search match fade-in/fade-out animations.
    fn viewport_lines(
        &mut self,
        model: &ActivityModel,
        nav: &ActivityNavigator,
        state: &AppState,
        scroll_offset: usize,
        viewport_height: usize,
        expansion_animations: &HashMap<usize, ExpansionAnimation>, // Phase B1
        executing_tools: &HashMap<String, Instant>,                // Phase B2
        highlights: &crate::tui::highlight::HighlightManager,      // Phase B3
        c_success: Color,
        c_accent: Color,
        c_warning: Color,
        c_error: Color,
        c_running: Color,
        c_text: Color,
        c_muted: Color,
        c_spinner: Color,
    ) -> Vec<Line<'static>> {
        let mut result: Vec<Line<'static>> = Vec::new();
        let mut row_cursor: usize = 0;

        // Collect all filtered model lines
        let filtered: Vec<(usize, &ActivityLine)> = model.filter_active().collect();

        for (idx, line) in &filtered {
            // Early exit: already past the visible viewport
            if row_cursor >= scroll_offset.saturating_add(viewport_height) {
                break;
            }

            let is_selected = nav.selected() == Some(*idx);
            let is_expanded = nav.is_expanded(*idx);
            let is_hovered = nav.is_hovered(*idx); // Phase B4

            // Phase B1: Get expansion animation progress (if any)
            let expansion_progress = expansion_animations
                .get(idx)
                .map(|anim| anim.current())
                .unwrap_or(if is_expanded { 1.0 } else { 0.0 });

            // Render the model line into rendered rows
            let rendered = self.render_line(
                line,
                *idx,
                model.len(),
                is_selected,
                is_expanded,
                is_hovered,
                expansion_progress,
                executing_tools,
                highlights,
                state,
                c_success,
                c_accent,
                c_warning,
                c_error,
                c_running,
                c_text,
                c_muted,
                c_spinner,
            );

            let n = rendered.len();
            let line_end = row_cursor + n;

            if line_end <= scroll_offset {
                // Entirely before viewport — skip without adding to result
                row_cursor = line_end;
                continue;
            }

            // This model line overlaps with [scroll_offset, scroll_offset + viewport_height)
            // Skip rows that fall before the viewport, take only what fits
            let skip = scroll_offset.saturating_sub(row_cursor);
            let remaining = viewport_height.saturating_sub(result.len());
            for row in rendered.into_iter().skip(skip).take(remaining) {
                result.push(row);
            }
            row_cursor = line_end;

            if result.len() >= viewport_height {
                break;
            }
        }

        // Spinner: render only if active and falls within the visible viewport range
        if state.spinner_active {
            let spinner_row = row_cursor; // spinner immediately follows the last model line
            if spinner_row >= scroll_offset && result.len() < viewport_height {
                // HALCÓN spinner: braille frames — precise, rhythmic, minimal
                let frames = ['⠁', '⠃', '⠇', '⠧', '⠷', '⠿', '⠾', '⠼', '⠸', '⠰'];
                let ch = frames[state.spinner_frame % frames.len()];
                result.push(Line::from(vec![
                    Span::styled(
                        format!("  {ch}  "),
                        Style::default().fg(c_spinner).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        state.spinner_label.clone(),
                        Style::default().fg(c_spinner),
                    ),
                ]));
            }
        }

        result
    }

    /// Render a single activity line.
    ///
    /// Returns a vector of Line<'static> (some ActivityLine types expand to multiple rendered lines).
    #[allow(clippy::too_many_arguments)]
    fn render_line(
        &mut self,
        line: &ActivityLine,
        line_idx: usize,
        total_lines: usize,                                // P0.4: total lines for last-line detection
        is_selected: bool,
        is_expanded: bool,
        is_hovered: bool,                                  // Phase B4: hover state
        expansion_progress: f32,                           // Phase B1: [0.0, 1.0] animation progress
        executing_tools: &HashMap<String, Instant>,        // Phase B2: tool_name → start_time
        highlights: &crate::tui::highlight::HighlightManager, // Phase B3: search highlight pulses
        state: &AppState,
        c_success: Color,
        c_accent: Color,
        c_warning: Color,
        c_error: Color,
        c_running: Color,
        c_text: Color,
        c_muted: Color,
        c_spinner: Color,
    ) -> Vec<Line<'static>> {
        let _ = (total_lines, c_spinner); // suppress unused warnings
        let mut lines = Vec::new();

        // Phase B3: Check for search highlight pulse
        let highlight_key = format!("search_{}", line_idx);

        // Background priority: highlight > selection > hover > none
        let bg = if highlights.is_pulsing(&highlight_key) {
            // Phase B3: Fade-in/fade-out search highlight background
            let pulse_color = highlights.current(&highlight_key, theme::active().palette.bg_highlight);
            Some(pulse_color.to_ratatui_color())
        } else if is_selected {
            // Selection highlight background
            Some(theme::active().palette.bg_highlight_ratatui())
        } else if is_hovered {
            // Phase B4: Hover effect background (subtle, muted color)
            Some(theme::active().palette.muted_ratatui())
        } else {
            None
        };

        match line {
            // ── User prompt ─────────────────────────────────────────────────
            ActivityLine::UserPrompt(text) => {
                let card_bg = bg.unwrap_or_else(|| theme::active().palette.bg_user_ratatui());

                // HALCÓN avatar: › (directional, understated) + OS username
                let display_name = if state.user_display_name.is_empty() {
                    "you".to_string()
                } else {
                    state.user_display_name.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled(" › ".to_string(), Style::default().fg(c_muted).bg(card_bg)),
                    Span::styled(display_name, Style::default().fg(c_muted).add_modifier(Modifier::BOLD).bg(card_bg)),
                ]));

                // Message content
                for content_line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("   ".to_string(), Style::default().bg(card_bg)),
                        Span::styled(content_line.to_string(), Style::default().fg(c_text).bg(card_bg)),
                    ]));
                }

                // Blank separator
                lines.push(Line::from("".to_string()));
            }

            // ── Assistant text ───────────────────────────────────────────────
            ActivityLine::AssistantText(text) => {
                let card_bg = if bg.is_some() {
                    bg
                } else {
                    Some(theme::active().palette.bg_assistant_ratatui())
                };

                // HALCÓN avatar: ◈ (precision targeting reticle) + brand name
                lines.push(Line::from(vec![
                    Span::styled(" ◈ ".to_string(), Style::default().fg(c_accent).add_modifier(Modifier::BOLD).bg(card_bg.unwrap_or(Color::Reset))),
                    Span::styled("halcon".to_string(), Style::default().fg(c_accent).add_modifier(Modifier::BOLD).bg(card_bg.unwrap_or(Color::Reset))),
                ]));

                // Message content with markdown rendering
                for l in text.lines() {
                    let parsed = super::activity_types::parse_md_spans_with_bg(l, c_warning, card_bg);
                    let mut indented = vec![Span::styled("   ".to_string(), Style::default().bg(card_bg.unwrap_or(Color::Reset)))];
                    indented.extend(parsed);
                    lines.push(Line::from(indented));
                }

                // Blank separator
                lines.push(Line::from("".to_string()));
            }

            // ── Code block ──────────────────────────────────────────────────
            ActivityLine::CodeBlock { lang, code } => {
                // Header
                lines.push(Line::from(vec![
                    Span::styled("  ┌─ ", Style::default().fg(c_muted)),
                    Span::styled(lang.clone(), Style::default().fg(c_accent).add_modifier(Modifier::BOLD)),
                    Span::styled(" ─", Style::default().fg(c_muted)),
                ]));

                if is_expanded {
                    // Phase B1: Smooth expansion animation
                    let all_lines: Vec<&str> = code.lines().collect();
                    let total = all_lines.len();
                    let lines_to_show = if expansion_progress < 1.0 {
                        ((total as f32 * expansion_progress).ceil() as usize).max(1)
                    } else {
                        total
                    };

                    for l in all_lines.iter().take(lines_to_show) {
                        lines.push(Line::from(vec![
                            Span::styled("  │ ", Style::default().fg(c_muted)),
                            Span::styled(l.to_string(), Style::default().fg(c_warning)),
                        ]));
                    }
                } else {
                    // Collapsed: show 2-line preview
                    for l in code.lines().take(2) {
                        lines.push(Line::from(vec![
                            Span::styled("  │ ", Style::default().fg(c_muted)),
                            Span::styled(l.to_string(), Style::default().fg(c_warning)),
                        ]));
                    }
                    if code.lines().count() > 2 {
                        lines.push(Line::from(Span::styled(
                            format!("  │ … {} more lines  ▸", code.lines().count() - 2),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        )));
                    }
                }

                // Footer
                lines.push(Line::from(Span::styled("  └───", Style::default().fg(c_muted))));
            }

            // ── Info — hierarchical + chip-based rendering ───────────────────
            ActivityLine::Info(text) => {
                // ── [N/M] macro step feedback — tree-connector style ──────────
                if let Some((n, m, rest)) = parse_macro_step_prefix(text) {
                    let is_last = n == m;
                    let connector = if is_last { "  └─ " } else { "  ├─ " };
                    let counter = format!(" [{n}/{m}] ");

                    let (icon, icon_color, desc, dim_desc) =
                        if let Some(d) = rest.strip_prefix("✓ ") {
                            ('✓', c_success, d, true)
                        } else if let Some(d) = rest.strip_prefix("✗ ") {
                            ('✗', c_error, d, false)
                        } else {
                            // Starting / in-progress
                            ('▸', c_running, rest, false)
                        };

                    let desc_style = if dim_desc {
                        Style::default().fg(c_muted).add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(c_text)
                    };

                    lines.push(Line::from(vec![
                        Span::styled(
                            connector.to_string(),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                        Span::styled(
                            format!("{icon}"),
                            Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            counter,
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                        Span::styled(desc.to_string(), desc_style),
                    ]));

                // ── "Plan: A → B → C" summary — planning color ───────────────
                } else if let Some(rest) = text.strip_prefix("Plan: ") {
                    let c_planning = theme::active().palette.planning_ratatui();
                    lines.push(Line::from(vec![
                        Span::styled(
                            "  ◈ ".to_string(),
                            Style::default().fg(c_planning).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            rest.to_string(),
                            Style::default().fg(c_muted),
                        ),
                    ]));

                // ── Standard chip rendering ───────────────────────────────────
                } else {
                    let (chip, rest) = classify_info_chip(text);

                    // Convergence signals get accent color (not dim) — they are
                    // important routing decisions from the ConvergenceDetector.
                    let is_convergence = text.starts_with("[convergence]");

                    match chip {
                        None => {
                            // Suppress entirely (e.g. [model] — already in status bar)
                        }
                        Some('·') => {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  · {rest}"),
                                    Style::default()
                                        .fg(c_muted)
                                        .add_modifier(Modifier::DIM)
                                        .bg(bg.unwrap_or(Color::Reset)),
                                ),
                            ]));
                        }
                        Some(ch) => {
                            let chip_color = if is_convergence { c_accent } else { c_accent };
                            let chip_modifier = if is_convergence {
                                Modifier::BOLD
                            } else {
                                Modifier::DIM
                            };
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  {ch} "),
                                    Style::default().fg(chip_color).add_modifier(chip_modifier),
                                ),
                                Span::styled(
                                    rest.to_string(),
                                    Style::default()
                                        .fg(if is_convergence { c_text } else { c_muted })
                                        .add_modifier(if is_convergence {
                                            Modifier::empty()
                                        } else {
                                            Modifier::DIM
                                        })
                                        .bg(bg.unwrap_or(Color::Reset)),
                                ),
                            ]));
                        }
                    }
                }
            }

            // ── Warning — chip-based rendering ──────────────────────────────
            ActivityLine::Warning { message, hint } => {
                let (chip, display_msg) = classify_warning_chip(message);

                let mut spans = vec![
                    Span::styled(
                        format!("  {chip} "),
                        Style::default().fg(c_warning).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(display_msg.to_string(), Style::default().fg(c_warning)),
                ];

                if let Some(h) = hint {
                    spans.push(Span::styled(
                        format!("  {h}"),
                        Style::default().fg(c_muted),
                    ));
                }

                lines.push(Line::from(spans));
            }

            // ── Error ────────────────────────────────────────────────────────
            ActivityLine::Error { message, hint } => {
                let mut spans = vec![
                    Span::styled(
                        "  ✖ ",
                        Style::default().fg(c_error).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(message.clone(), Style::default().fg(c_error)),
                ];
                if let Some(h) = hint {
                    spans.push(Span::styled(
                        format!("  {h}"),
                        Style::default().fg(c_muted),
                    ));
                }
                lines.push(Line::from(spans));
            }

            // ── Round separator — HALCÓN minimal rule ────────────────────────
            ActivityLine::RoundSeparator(n) => {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  ─ ".to_string(),
                        Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                    ),
                    Span::styled(
                        format!("{n}"),
                        Style::default().fg(c_muted),
                    ),
                ]));
            }

            // ── Plan overview — HALCÓN hierarchical tree ────────────────────
            ActivityLine::PlanOverview { goal, steps, current_step } => {
                use crate::tui::events::PlanStepDisplayStatus;
                // momoto planning color: deep violet for plan structure
                let c_planning = theme::active().palette.planning_ratatui();
                let n_steps = steps.len();

                if is_expanded {
                    // ── Expanded: tree with connectors ────────────────────────
                    // Header: "  ◈ Plan  Goal description" in planning color
                    lines.push(Line::from(vec![
                        Span::styled(
                            "  ◈ ".to_string(),
                            Style::default().fg(c_planning).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "Plan  ".to_string(),
                            Style::default()
                                .fg(c_planning)
                                .add_modifier(Modifier::BOLD | Modifier::DIM),
                        ),
                        Span::styled(
                            goal.clone(),
                            Style::default().fg(c_text).add_modifier(Modifier::BOLD),
                        ),
                    ]));

                    // Steps with tree connectors: ├─ or └─ (last step)
                    for (i, step) in steps.iter().enumerate() {
                        let (icon, icon_color) = match step.status {
                            PlanStepDisplayStatus::Succeeded  => ("✓", c_success),
                            PlanStepDisplayStatus::Failed     => ("✗", c_error),
                            PlanStepDisplayStatus::InProgress => ("⚙", c_running),
                            PlanStepDisplayStatus::Skipped    => ("─", c_muted),
                            PlanStepDisplayStatus::Pending    => ("○", c_muted),
                        };

                        let is_current = i == *current_step
                            && step.status == PlanStepDisplayStatus::InProgress;
                        let connector = if i + 1 == n_steps { "  └─ " } else { "  ├─ " };
                        let counter = format!("[{}/{}] ", i + 1, n_steps);
                        let tool_hint = step
                            .tool_name
                            .as_deref()
                            .map(|t| format!("  {t}"))
                            .unwrap_or_default();
                        let current_mark = if is_current { "  ◀" } else { "" };

                        // Active step: full brightness; done: dim; pending: muted
                        let (desc_style, meta_style) = if is_current {
                            (
                                Style::default().fg(c_running),
                                Style::default().fg(c_muted),
                            )
                        } else if matches!(step.status, PlanStepDisplayStatus::Succeeded) {
                            (
                                Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                                Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                            )
                        } else {
                            (
                                Style::default().fg(c_muted),
                                Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                            )
                        };

                        lines.push(Line::from(vec![
                            Span::styled(
                                connector.to_string(),
                                Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                            ),
                            Span::styled(
                                format!("{icon} "),
                                Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                counter,
                                Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                            ),
                            Span::styled(step.description.clone(), desc_style),
                            Span::styled(tool_hint, meta_style),
                            Span::styled(
                                current_mark.to_string(),
                                Style::default().fg(c_running).add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    }
                } else {
                    // ── Collapsed: goal + live progress badge ─────────────────
                    // "  ◈ Goal text  ·  ⚙ [2/4]  ▸"
                    let (progress_icon, progress_color) = steps
                        .get(*current_step)
                        .map(|s| match s.status {
                            PlanStepDisplayStatus::Succeeded  => ("✓", c_success),
                            PlanStepDisplayStatus::Failed     => ("✗", c_error),
                            PlanStepDisplayStatus::InProgress => ("⚙", c_running),
                            PlanStepDisplayStatus::Skipped    => ("─", c_muted),
                            PlanStepDisplayStatus::Pending    => ("○", c_muted),
                        })
                        .unwrap_or(("◈", c_muted));

                    let n = (*current_step + 1).min(n_steps);

                    // Truncate goal if long to leave room for progress badge
                    let goal_display: String = if goal.chars().count() > 34 {
                        format!("{}…", goal.chars().take(33).collect::<String>())
                    } else {
                        goal.clone()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(
                            "  ◈ ".to_string(),
                            Style::default().fg(c_planning).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            goal_display,
                            Style::default().fg(c_text).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "  ·  ".to_string(),
                            Style::default().fg(c_muted),
                        ),
                        Span::styled(
                            format!("{progress_icon} "),
                            Style::default().fg(progress_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("[{n}/{n_steps}]"),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                        Span::styled(
                            "  ▸".to_string(),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                    ]));
                }

                lines.push(Line::from(""));
            }

            // ── Agent thinking skeleton ──────────────────────────────────────
            ActivityLine::AgentThinking => {
                let card_bg = theme::active().palette.bg_assistant_ratatui();
                let c_primary = theme::active().palette.primary_ratatui();
                // Falcon eye cycling: ○ ◎ ◉ ● ◉ ◎  (6-frame loop)
                let eye_frames = ["○", "◎", "◉", "●", "◉", "◎"];
                let eye = eye_frames[state.spinner_frame % 6];
                // Breathing dots (slowed: advance every 2 eye frames)
                let dots = match (state.spinner_frame / 2) % 4 {
                    0 => "·    ",
                    1 => "· ·  ",
                    2 => "· · ·",
                    _ => "     ",
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        eye.to_string(),
                        Style::default().fg(c_primary).bg(card_bg).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        dots.to_string(),
                        Style::default()
                            .fg(c_muted)
                            .add_modifier(Modifier::DIM)
                            .bg(card_bg),
                    ),
                ]));
                lines.push(Line::from("".to_string()));
            }

            // ── Thinking bubble (persistent CoT summary) ─────────────────────
            ActivityLine::ThinkingBubble { char_count, preview } => {
                let pal = &theme::active().palette;
                let c_dim = pal.muted_ratatui();
                let kchars = if *char_count >= 1000 {
                    format!("{:.1}K", *char_count as f64 / 1000.0)
                } else {
                    char_count.to_string()
                };
                let snippet = if preview.len() > 80 {
                    format!("{}...", &preview[..80])
                } else {
                    preview.clone()
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("⟨ razonando · {kchars} chars ⟩  \"{snippet}\""),
                        Style::default().fg(c_dim).add_modifier(Modifier::DIM | Modifier::ITALIC),
                    ),
                ]));
            }

            // ── Phase indicator skeleton (planning / reasoning / reflecting) ─
            ActivityLine::PhaseIndicator { phase, label } => {
                use super::activity_types::AgentPhase;
                let pal = &theme::active().palette;
                let (icon, phase_color) = match phase {
                    AgentPhase::Planning    => ('⊡', pal.planning_ratatui()),
                    AgentPhase::Reasoning   => ('◉', pal.reasoning_ratatui()),
                    AgentPhase::Reflecting  => ('◎', pal.reasoning_ratatui()),
                    AgentPhase::Searching   => ('≋', pal.accent_ratatui()),
                    AgentPhase::Delegating { .. } => ('⬡', pal.delegated_ratatui()),
                };
                // Line 1: icon + label
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        icon.to_string(),
                        Style::default().fg(phase_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(label.clone(), Style::default().fg(phase_color)),
                ]));
                // Line 2: shimmer bar sweeping left→right driven by spinner_frame
                let pos = state.spinner_frame % 12;
                let mut bar = vec![Span::raw("     ")];
                for i in 0usize..12 {
                    let (ch, modifier) = match i.abs_diff(pos) {
                        0 => ('▓', Modifier::BOLD),
                        1 => ('▒', Modifier::empty()),
                        _ => ('░', Modifier::DIM),
                    };
                    bar.push(Span::styled(
                        ch.to_string(),
                        Style::default().fg(phase_color).add_modifier(modifier),
                    ));
                }
                lines.push(Line::from(bar));
            }

            // ── Tool execution ───────────────────────────────────────────────
            ActivityLine::ToolExec { name, input_preview, result, .. } => {
                match result {
                    None => {
                        // HALCÓN tool executing — precision pulse bar
                        let shimmer_pos = if let Some(start_time) = executing_tools.get(name) {
                            shimmer_progress(start_time.elapsed())
                        } else {
                            0.0
                        };

                        const SHIMMER_WIDTH: usize = 8;
                        const WAVE_WIDTH: usize = 2;

                        // Thinner, sharper pulse — blade cutting through
                        let shimmer_bar: String = (0..SHIMMER_WIDTH)
                            .map(|i| {
                                let pos = (shimmer_pos * SHIMMER_WIDTH as f32) as usize;
                                let distance = if i >= pos { i - pos } else { pos - i };
                                if distance < WAVE_WIDTH { '▪' } else { '·' }
                            })
                            .collect();

                        // ⟳ tool_name  arg_preview
                        lines.push(Line::from(vec![
                            Span::styled(
                                "  ⟳ ".to_string(),
                                Style::default().fg(c_running),
                            ),
                            Span::styled(
                                name.clone(),
                                Style::default().fg(c_text).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("  {input_preview}"),
                                Style::default().fg(c_muted),
                            ),
                        ]));

                        // Precision pulse bar
                        lines.push(Line::from(vec![
                            Span::styled("    ".to_string(), Style::default()),
                            Span::styled(shimmer_bar, Style::default().fg(c_running).add_modifier(Modifier::DIM)),
                        ]));
                    }

                    Some(res) => {
                        // Completed: ✓/✗ name  duration  ▸/▾
                        let (icon_char, icon_color) = if res.is_error {
                            ("✗", c_error)
                        } else {
                            ("✓", c_success)
                        };
                        let duration_str = if res.duration_ms < 1000 {
                            format!("{}ms", res.duration_ms)
                        } else {
                            format!("{:.1}s", res.duration_ms as f64 / 1000.0)
                        };
                        let expand_hint = if is_expanded { " ▾" } else { " ▸" };

                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("  {icon_char} "),
                                Style::default()
                                    .fg(icon_color)
                                    .bg(bg.unwrap_or(Color::Reset)),
                            ),
                            Span::styled(
                                name.clone(),
                                Style::default()
                                    .fg(c_text)
                                    .add_modifier(Modifier::BOLD)
                                    .bg(bg.unwrap_or(Color::Reset)),
                            ),
                            Span::styled(
                                format!("  {duration_str}{expand_hint}"),
                                Style::default()
                                    .fg(c_muted)
                                    .bg(bg.unwrap_or(Color::Reset)),
                            ),
                        ]));

                        // Content: expanded or collapsed preview
                        if !res.content.is_empty() {
                            let content_color = if res.is_error { c_error } else { c_muted };

                            if is_expanded {
                                // Phase B1: Smooth expansion animation
                                let all_lines: Vec<&str> = res.content.lines().collect();
                                let total = all_lines.len();
                                let lines_to_show = if expansion_progress < 1.0 {
                                    ((total as f32 * expansion_progress).ceil() as usize).max(1)
                                } else {
                                    total
                                };

                                for pline in all_lines.iter().take(lines_to_show) {
                                    lines.push(Line::from(vec![
                                        Span::styled("    ", Style::default()),
                                        Span::styled(pline.to_string(), Style::default().fg(content_color)),
                                    ]));
                                }
                            } else {
                                // Collapsed preview: first 200 chars, char-safe, 3 lines max.
                                let preview: String = res.content.chars().take(200).collect();
                                for pline in preview.lines().take(3) {
                                    lines.push(Line::from(vec![
                                        Span::styled("    ", Style::default()),
                                        Span::styled(pline.to_string(), Style::default().fg(content_color)),
                                    ]));
                                }

                                let total_content_lines = res.content.lines().count();
                                if total_content_lines > 3 {
                                    lines.push(Line::from(Span::styled(
                                        format!("    … {} more lines  ▸", total_content_lines - 3),
                                        Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                                    )));
                                }
                            }
                        }
                    }
                }
            }

            // ── Orchestrator header ──────────────────────────────────────────
            ActivityLine::OrchestratorHeader { task_count, wave_count } => {
                let pal = &theme::active().palette;
                let c_delegated = pal.delegated_ratatui();
                lines.push(Line::from(vec![
                    Span::styled(
                        "  ● ".to_string(),
                        Style::default().fg(c_delegated).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "Orchestrator".to_string(),
                        Style::default().fg(c_delegated).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ·  ".to_string(), Style::default().fg(c_muted)),
                    Span::styled(
                        format!("{task_count} tasks"),
                        Style::default().fg(c_text),
                    ),
                    Span::styled("  ·  ".to_string(), Style::default().fg(c_muted)),
                    Span::styled(
                        format!("Wave 1/{wave_count}"),
                        Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                    ),
                ]));
            }

            // ── Sub-agent task pill ──────────────────────────────────────────
            ActivityLine::SubAgentTask {
                step_index,
                total_steps,
                description,
                status,
                rounds: _,
                tools_used,
                summary,
                ..
            } => {
                use super::activity_types::SubAgentStatus;

                let (icon, icon_color) = match status {
                    SubAgentStatus::Running => ("⟳", c_running),
                    SubAgentStatus::Success { .. } => ("✓", c_success),
                    SubAgentStatus::Failed { .. } => ("✗", c_error),
                };

                let duration_str = match status {
                    SubAgentStatus::Running => String::new(),
                    SubAgentStatus::Success { latency_ms } | SubAgentStatus::Failed { latency_ms } => {
                        format!("  ·  {:.1}s", *latency_ms as f64 / 1000.0)
                    }
                };

                if !state.show_sub_agent_detail {
                    // ── Collapsed pill ────────────────────────────────────────
                    let tools_pill = if tools_used.is_empty() {
                        String::new()
                    } else {
                        format!("  ·  {}", tools_used.join(" "))
                    };

                    let desc_short: String = description.chars().take(48).collect();

                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {icon} "),
                            Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("[{step_index}/{total_steps}]"),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                        Span::raw("  "),
                        Span::styled(desc_short, Style::default().fg(c_text)),
                        Span::styled(tools_pill, Style::default().fg(c_muted)),
                        Span::styled(duration_str, Style::default().fg(c_muted).add_modifier(Modifier::DIM)),
                    ]));
                } else {
                    // ── Expanded detail ────────────────────────────────────────
                    // Header line
                    let desc_short: String = description.chars().take(48).collect();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {icon} "),
                            Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("[{step_index}/{total_steps}]"),
                            Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                        ),
                        Span::raw("  "),
                        Span::styled(desc_short, Style::default().fg(c_text).add_modifier(Modifier::BOLD)),
                        Span::styled(duration_str, Style::default().fg(c_muted).add_modifier(Modifier::DIM)),
                    ]));

                    // Tool rows
                    for tool in tools_used.iter() {
                        lines.push(Line::from(vec![
                            Span::styled("      └ ".to_string(), Style::default().fg(c_muted)),
                            Span::styled(tool.clone(), Style::default().fg(c_accent)),
                        ]));
                    }

                    // Summary row
                    if !summary.is_empty() {
                        let summary_display: String = summary.chars().take(100).collect();
                        lines.push(Line::from(vec![
                            Span::styled("      └ ".to_string(), Style::default().fg(c_muted)),
                            Span::styled(
                                format!("\"{summary_display}\""),
                                Style::default().fg(c_muted).add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                }
            }
        }

        lines
    }

    /// Count total rendered lines for scrollbar + max_scroll calculation.
    ///
    /// Must mirror the actual line output of `render_line` exactly.
    /// **Fixed in Phase 44B**: UserPrompt and AssistantText now account for avatar + blank lines.
    fn count_rendered_lines(
        &self,
        model: &ActivityModel,
        nav: &ActivityNavigator,
        state: &AppState,
    ) -> usize {
        let mut count = 0;

        for (idx, line) in model.filter_active() {
            let is_expanded = nav.is_expanded(idx);

            count += match line {
                // avatar (1) + content lines (N) + blank (1) = N+2
                ActivityLine::UserPrompt(text) => text.lines().count() + 2,

                // avatar (1) + content lines (N) + blank (1) = N+2
                ActivityLine::AssistantText(text) => text.lines().count() + 2,

                ActivityLine::CodeBlock { code, .. } => {
                    if is_expanded {
                        2 + code.lines().count() // header + content + footer
                    } else {
                        let extra = if code.lines().count() > 2 { 1 } else { 0 };
                        2 + 2 + extra // header + 2 preview + maybe "…more" + footer
                    }
                }

                ActivityLine::Info(text) => {
                    // Suppressed [model] lines count as 0
                    let (chip, _) = classify_info_chip(text);
                    if chip.is_none() { 0 } else { 1 }
                }

                ActivityLine::Warning { .. } => 1,
                ActivityLine::Error { .. }   => 1,
                ActivityLine::RoundSeparator(_) => 1,

                ActivityLine::PlanOverview { steps, .. } => {
                    if is_expanded {
                        2 + steps.len() // header + steps + blank
                    } else {
                        2 // header + inline flow + blank
                    }
                }

                ActivityLine::AgentThinking => 2, // indicator line + blank
                ActivityLine::PhaseIndicator { .. } => 2, // icon+label line + shimmer bar
                ActivityLine::ThinkingBubble { .. } => 1, // single dim bubble line
                ActivityLine::OrchestratorHeader { .. } => 1, // single header line
                ActivityLine::SubAgentTask { tools_used, summary, .. } => {
                    if state.show_sub_agent_detail {
                        let summary_line = if summary.is_empty() { 0 } else { 1 };
                        1 + tools_used.len() + summary_line // header + tool rows + summary
                    } else {
                        1 // collapsed pill
                    }
                }

                ActivityLine::ToolExec { result, .. } => {
                    match result {
                        None => 2, // name + shimmer bar
                        Some(res) => {
                            let content_lines = if is_expanded {
                                res.content.lines().count()
                            } else {
                                let preview_lines = res.content.lines().take(3).count();
                                let extra = if res.content.lines().count() > 3 { 1 } else { 0 };
                                preview_lines + extra
                            };
                            1 + content_lines // header + content
                        }
                    }
                }
            };
        }

        // Spinner (if active) adds 1 rendered line
        if state.spinner_active {
            count += 1;
        }

        count
    }

    /// Clear the span cache (useful when theme changes).
    pub fn clear_cache(&mut self) {
        self.span_cache.clear();
    }
}

impl Default for ActivityRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_cache_insert_and_get() {
        let mut cache = SpanCache::new(3);
        let spans = vec![Span::raw("test".to_string())];

        cache.insert(0, spans.clone());
        assert!(cache.get(0).is_some());
        assert_eq!(cache.get(0).unwrap().len(), 1);
    }

    #[test]
    fn span_cache_lru_eviction() {
        let mut cache = SpanCache::new(2);

        cache.insert(0, vec![Span::raw("line 0".to_string())]);
        cache.insert(1, vec![Span::raw("line 1".to_string())]);
        cache.insert(2, vec![Span::raw("line 2".to_string())]); // Should evict 0

        assert!(cache.get(0).is_none()); // Evicted (LRU)
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn span_cache_access_updates_lru() {
        let mut cache = SpanCache::new(2);

        cache.insert(0, vec![Span::raw("line 0".to_string())]);
        cache.insert(1, vec![Span::raw("line 1".to_string())]);

        // Access 0 → makes it most recently used
        let _ = cache.get(0);

        // Insert 2 → should evict 1 (not 0)
        cache.insert(2, vec![Span::raw("line 2".to_string())]);

        assert!(cache.get(0).is_some()); // Still cached
        assert!(cache.get(1).is_none()); // Evicted
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn span_cache_clear() {
        let mut cache = SpanCache::new(3);
        cache.insert(0, vec![Span::raw("test".to_string())]);
        cache.insert(1, vec![Span::raw("test".to_string())]);

        cache.clear();

        assert!(cache.get(0).is_none());
        assert!(cache.get(1).is_none());
        assert_eq!(cache.cache.len(), 0);
    }

    #[test]
    fn renderer_creates_with_default_capacity() {
        let renderer = ActivityRenderer::new();
        assert_eq!(renderer.span_cache.max_capacity, 200);
    }

    #[test]
    fn renderer_creates_with_custom_capacity() {
        let renderer = ActivityRenderer::with_capacity(50);
        assert_eq!(renderer.span_cache.max_capacity, 50);
    }

    // ── count_rendered_lines tests (updated for Phase 44B accurate counting) ──

    #[test]
    fn count_rendered_lines_user_prompt_single_line() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::UserPrompt("hello".into()));

        // avatar(1) + content(1) + blank(1) = 3
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 3);
    }

    #[test]
    fn count_rendered_lines_user_prompt_multiline() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::UserPrompt("line1\nline2\nline3".into()));

        // avatar(1) + content(3) + blank(1) = 5
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 5);
    }

    #[test]
    fn count_rendered_lines_assistant_multiline() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::AssistantText("line 1\nline 2\nline 3".into()));

        // avatar(1) + content(3) + blank(1) = 5
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 5);
    }

    #[test]
    fn count_rendered_lines_info_suppressed() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        // [model] lines are suppressed → count = 0
        model.push(ActivityLine::Info("[model] provider/model — reason".into()));
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 0);
    }

    #[test]
    fn count_rendered_lines_info_chip() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::Info("[compaction] 10 → 5 messages".into()));
        model.push(ActivityLine::Info("[memory] merging...".into()));

        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 2); // both render as 1 chip line each
    }

    #[test]
    fn count_rendered_lines_code_block_collapsed() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::CodeBlock {
            lang: "rust".into(),
            code: "fn main() {}\nfn test() {}\nfn other() {}".into(),
        });

        // header(1) + 2 preview lines(2) + "…N more lines"(1) + footer(1) = 5
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 5);
    }

    #[test]
    fn count_rendered_lines_code_block_expanded() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let mut nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::CodeBlock {
            lang: "rust".into(),
            code: "fn main() {}\nfn test() {}".into(),
        });

        nav.toggle_expand(0);

        // header(1) + 2 code lines(2) + footer(1) = 4
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 4);
    }

    #[test]
    fn count_rendered_lines_with_spinner() {
        let renderer = ActivityRenderer::new();
        let model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let mut state = AppState::new();

        state.spinner_active = true;

        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 1); // Just spinner
    }

    #[test]
    fn count_rendered_lines_round_separator() {
        let renderer = ActivityRenderer::new();
        let mut model = ActivityModel::new();
        let nav = ActivityNavigator::new();
        let state = AppState::new();

        model.push(ActivityLine::RoundSeparator(3));
        let count = renderer.count_rendered_lines(&model, &nav, &state);
        assert_eq!(count, 1);
    }

    // ── Chip classifier tests ──────────────────────────────────────────────────

    #[test]
    fn classify_info_chip_suppresses_model() {
        let (chip, _rest) = classify_info_chip("[model] deepseek/chat — auto");
        assert!(chip.is_none(), "model lines must be suppressed");
    }

    #[test]
    fn classify_info_chip_compaction() {
        let (chip, rest) = classify_info_chip("[compaction] 10 → 5 messages");
        assert_eq!(chip, Some('⊙'));
        assert_eq!(rest, "10 → 5 messages");
    }

    #[test]
    fn classify_info_chip_memory() {
        let (chip, rest) = classify_info_chip("[memory] merging 3 entries");
        assert_eq!(chip, Some('◈'));
        assert_eq!(rest, "merging 3 entries");
    }

    #[test]
    fn classify_info_chip_reflecting() {
        let (chip, _) = classify_info_chip("[reflecting] analyzing round outcome...");
        assert_eq!(chip, Some('◎'));
    }

    #[test]
    fn classify_info_chip_reflection() {
        let (chip, _) = classify_info_chip("[reflection] good progress (score: 0.85)");
        assert_eq!(chip, Some('◎'));
    }

    #[test]
    fn classify_info_chip_plain() {
        let (chip, rest) = classify_info_chip("some plain info message");
        assert_eq!(chip, Some('·'));
        assert_eq!(rest, "some plain info message");
    }

    #[test]
    fn classify_warning_chip_retry() {
        let (chip, rest) = classify_warning_chip("[retry] bash attempt 2/3 in 500ms");
        assert_eq!(chip, '↻');
        assert_eq!(rest, "bash attempt 2/3 in 500ms");
    }

    #[test]
    fn classify_warning_chip_guard() {
        let (chip, rest) = classify_warning_chip("[guard] force_synthesis: too many rounds");
        assert_eq!(chip, '⊕');
        assert_eq!(rest, "force_synthesis: too many rounds");
    }

    #[test]
    fn classify_warning_chip_fallback() {
        let (chip, _) = classify_warning_chip("⇄ anthropic → deepseek  credits exhausted");
        assert_eq!(chip, '⇄');
    }

    #[test]
    fn classify_warning_chip_default() {
        let (chip, _) = classify_warning_chip("something unexpected happened");
        assert_eq!(chip, '⚠');
    }

    #[test]
    fn classify_info_chip_guard() {
        let (chip, rest) = classify_info_chip("[guard] inject_synthesis: convergence");
        assert_eq!(chip, Some('⊕'));
        assert_eq!(rest, "inject_synthesis: convergence");
    }

    #[test]
    fn classify_info_chip_hicon() {
        let (chip, _) = classify_info_chip("[hicon:φ] coherence 0.73");
        assert_eq!(chip, Some('Φ'));
    }

    #[test]
    fn classify_info_chip_cache_hit() {
        let (chip, rest) = classify_info_chip("[cache hit] l3_semantic");
        assert_eq!(chip, Some('≋'));
        assert_eq!(rest, "l3_semantic");
    }

    // ── parse_macro_step_prefix tests ──────────────────────────────────────

    #[test]
    fn parse_macro_step_prefix_start_line() {
        let result = parse_macro_step_prefix("[1/3] Analysing project");
        assert_eq!(result, Some((1, 3, "Analysing project")));
    }

    #[test]
    fn parse_macro_step_prefix_done_line() {
        // done_line() produces "[N/M] ✓ Description"
        let result = parse_macro_step_prefix("[2/3] ✓ Apply fixes");
        assert_eq!(result, Some((2, 3, "✓ Apply fixes")));
    }

    #[test]
    fn parse_macro_step_prefix_failed_line() {
        let result = parse_macro_step_prefix("[1/2] ✗ Build failed");
        assert_eq!(result, Some((1, 2, "✗ Build failed")));
    }

    #[test]
    fn parse_macro_step_prefix_last_step() {
        let result = parse_macro_step_prefix("[3/3] Synthesise report");
        assert_eq!(result, Some((3, 3, "Synthesise report")));
    }

    #[test]
    fn parse_macro_step_prefix_single_step() {
        let result = parse_macro_step_prefix("[1/1] Only step");
        assert_eq!(result, Some((1, 1, "Only step")));
    }

    #[test]
    fn parse_macro_step_prefix_leading_space_stripped() {
        // rest must have leading space stripped
        let result = parse_macro_step_prefix("[1/5]   spaced text");
        assert_eq!(result, Some((1, 5, "spaced text")));
    }

    #[test]
    fn parse_macro_step_prefix_rejects_plain_text() {
        assert!(parse_macro_step_prefix("plain text").is_none());
    }

    #[test]
    fn parse_macro_step_prefix_rejects_convergence_brackets() {
        // "[convergence]" must NOT be parsed as a macro step
        assert!(parse_macro_step_prefix("[convergence] something").is_none());
    }

    #[test]
    fn parse_macro_step_prefix_rejects_planning_bracket() {
        assert!(parse_macro_step_prefix("[planning] ...").is_none());
    }

    #[test]
    fn parse_macro_step_prefix_rejects_zero_n() {
        // n=0 is invalid
        assert!(parse_macro_step_prefix("[0/3] Zero step").is_none());
    }

    #[test]
    fn parse_macro_step_prefix_rejects_zero_m() {
        // m=0 is invalid
        assert!(parse_macro_step_prefix("[1/0] No steps").is_none());
    }

    #[test]
    fn parse_macro_step_prefix_rejects_n_too_large() {
        // n > m+1 is invalid
        assert!(parse_macro_step_prefix("[5/2] Overflow").is_none());
    }

    // ── convergence chip classification ────────────────────────────────────

    #[test]
    fn classify_info_chip_convergence() {
        let (chip, rest) = classify_info_chip("[convergence] EvidenceThreshold: 80%");
        assert_eq!(chip, Some('⊙'));
        assert_eq!(rest, "EvidenceThreshold: 80%");
    }

    #[test]
    fn classify_info_chip_convergence_diminishing() {
        let (chip, _) = classify_info_chip("[convergence] DiminishingReturns after 2 rounds");
        assert_eq!(chip, Some('⊙'));
    }
}
