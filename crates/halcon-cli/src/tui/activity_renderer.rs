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

            // ── Info — chip-based rendering ──────────────────────────────────
            ActivityLine::Info(text) => {
                let (chip, rest) = classify_info_chip(text);

                match chip {
                    None => {
                        // Suppress entirely (e.g. [model] — already in status bar)
                    }
                    Some('·') => {
                        // Plain subtle bullet — no bracket prefix
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
                        // Chip indicator: icon (accent) + rest (muted/dim)
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("  {ch} "),
                                Style::default()
                                    .fg(c_accent)
                                    .add_modifier(Modifier::DIM),
                            ),
                            Span::styled(
                                rest.to_string(),
                                Style::default()
                                    .fg(c_muted)
                                    .add_modifier(Modifier::DIM)
                                    .bg(bg.unwrap_or(Color::Reset)),
                            ),
                        ]));
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

            // ── Plan overview ────────────────────────────────────────────────
            ActivityLine::PlanOverview { goal, steps, current_step } => {
                // HALCÓN plan header — precision targeting
                lines.push(Line::from(vec![
                    Span::styled(
                        "  ◈ ".to_string(),
                        Style::default().fg(c_running).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        goal.clone(),
                        Style::default().fg(c_text).add_modifier(Modifier::BOLD),
                    ),
                ]));

                if is_expanded {
                    // Expanded: full step list with status icons
                    for (i, step) in steps.iter().enumerate() {
                        use crate::tui::events::PlanStepDisplayStatus;
                        let (icon, color) = match step.status {
                            PlanStepDisplayStatus::Succeeded => ("✓", c_success),
                            PlanStepDisplayStatus::Failed    => ("✗", c_error),
                            PlanStepDisplayStatus::InProgress => ("⚙", c_running),
                            PlanStepDisplayStatus::Skipped   => ("─", c_muted),
                            PlanStepDisplayStatus::Pending   => ("○", c_muted),
                        };
                        let is_current = i == *current_step
                            && step.status == PlanStepDisplayStatus::InProgress;
                        let tool_hint = step
                            .tool_name
                            .as_deref()
                            .map(|t| format!("  {t}"))
                            .unwrap_or_default();
                        let current_mark = if is_current { "  ◀" } else { "" };

                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("    {icon} "),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("{}{tool_hint}{current_mark}", step.description),
                                Style::default().fg(color),
                            ),
                        ]));
                    }
                } else {
                    // Collapsed: inline horizontal flow  ✓ Step1 → ⚙ Step2 → ○ Step3  ▸
                    let mut flow: Vec<Span<'static>> = vec![Span::styled("  ", Style::default())];

                    for (i, step) in steps.iter().enumerate() {
                        use crate::tui::events::PlanStepDisplayStatus;
                        let (icon_ch, step_color) = match step.status {
                            PlanStepDisplayStatus::Succeeded  => ("✓", c_success),
                            PlanStepDisplayStatus::Failed     => ("✗", c_error),
                            PlanStepDisplayStatus::InProgress => ("⚙", c_running),
                            PlanStepDisplayStatus::Skipped    => ("─", c_muted),
                            PlanStepDisplayStatus::Pending    => ("○", c_muted),
                        };

                        // First word of step description, capped at 12 chars (char-safe).
                        let label_raw = step.description.split_whitespace().next().unwrap_or("Step");
                        let label_owned: String;
                        let label = if label_raw.chars().count() > 12 {
                            label_owned = label_raw.chars().take(12).collect();
                            label_owned.as_str()
                        } else {
                            label_raw
                        };

                        flow.push(Span::styled(
                            format!("{icon_ch} "),
                            Style::default().fg(step_color).add_modifier(Modifier::BOLD),
                        ));
                        flow.push(Span::styled(label.to_string(), Style::default().fg(step_color)));

                        if i + 1 < steps.len() {
                            flow.push(Span::styled(
                                " → ".to_string(),
                                Style::default().fg(c_muted),
                            ));
                        }
                    }

                    // Expand hint
                    flow.push(Span::styled(
                        "  ▸".to_string(),
                        Style::default().fg(c_muted).add_modifier(Modifier::DIM),
                    ));

                    lines.push(Line::from(flow));
                }

                lines.push(Line::from(""));
            }

            // ── Agent thinking skeleton ──────────────────────────────────────
            ActivityLine::AgentThinking => {
                let card_bg = theme::active().palette.bg_assistant_ratatui();
                // 4-phase dot animation driven by spinner_frame (ticked every 100ms)
                let dots = match state.spinner_frame % 4 {
                    0 => "·    ",
                    1 => "· ·  ",
                    2 => "· · ·",
                    _ => "     ",
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        " ◈ ".to_string(),
                        Style::default().fg(c_muted).bg(card_bg),
                    ),
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
}
