//! Frontier auth gate — shown when `halcon chat` starts with no authenticated provider.
//!
//! Detects the auth state of every supported provider (cenzontle, anthropic, openai,
//! deepseek, gemini, claude_code, ollama) and, if none have valid credentials, renders
//! an interactive crossterm UI that lets the user configure one before the session begins.
//!
//! Supports two authentication methods, selected automatically per provider:
//!   - **Browser / OAuth** — cenzontle (Cuervo SSO) and claude_code (claude.ai)
//!   - **API key**         — anthropic, openai, deepseek, gemini
//!
//! Works identically for classic (REPL) and TUI modes because the gate runs before
//! either the REPL or ratatui TUI is initialized.
//!
//! ## Cross-platform rendering
//!
//! The box-drawing UI uses `unicode-width` for display-width-aware padding, detects
//! terminal Unicode support via `$LANG`, and falls back to ASCII box chars on limited
//! terminals. All lines are guaranteed to have exactly the same display width.

use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use halcon_auth::KeyStore;
use halcon_core::types::AppConfig;
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

const SERVICE_NAME: &str = "halcon-cli";

// ── Auth method classification ───────────────────────────────────────────────

/// How a provider authenticates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthFlow {
    Browser,
    ApiKey,
    NoAuth,
}

/// Authentication state as observed right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    Authenticated,
    Missing,
    NoAuthRequired,
}

/// Single row in the provider list.
#[derive(Clone)]
pub struct ProviderEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub flow: AuthFlow,
    pub status: AuthStatus,
    /// Env var that carries the credential (API key providers).
    pub env_var: Option<&'static str>,
    /// OS keystore key name.
    pub keystore_key: Option<&'static str>,
    /// Short user-facing hint shown in the setup screen.
    pub hint: &'static str,
}

/// Outcome returned to `chat::run` after the gate finishes.
pub struct AuthGateOutcome {
    /// True if at least one credential was successfully saved during this session.
    pub credentials_added: bool,
}

// ── Credential probing ────────────────────────────────────────────────────────

fn has_env_or_keystore(env_var: &str, keystore_key: &str) -> bool {
    if std::env::var(env_var)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    KeyStore::new(SERVICE_NAME)
        .get_secret(keystore_key)
        .ok()
        .flatten()
        .is_some()
}

fn cenzontle_authenticated() -> bool {
    std::env::var("CENZONTLE_ACCESS_TOKEN")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
        || KeyStore::new(SERVICE_NAME)
            .get_secret("cenzontle:access_token")
            .ok()
            .flatten()
            .is_some()
}

fn claude_code_authenticated() -> bool {
    let bin = locate_claude_binary();
    let result = std::process::Command::new(&bin)
        .args(["auth", "status", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();

    match result {
        Ok(out) => serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|v| v["loggedIn"].as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Locate the `claude` binary by searching `$PATH` first, then checking
/// well-known installation paths, and finally falling back to bare name.
fn locate_claude_binary() -> String {
    // First: try to resolve via $PATH using `command -v` (POSIX) or `where` (Windows)
    #[cfg(unix)]
    {
        if let Ok(out) = std::process::Command::new("sh")
            .args(["-c", "command -v claude 2>/dev/null"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return path;
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(out) = std::process::Command::new("where")
            .arg("claude")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            let path = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return path;
            }
        }
    }

    // Second: check well-known user-local install paths
    if let Ok(home) = std::env::var("HOME") {
        let candidates: &[&str] = &[
            ".local/bin/claude",
            ".npm-global/bin/claude",
            ".nvm/current/bin/claude",
        ];
        for suffix in candidates {
            let p = format!("{home}/{suffix}");
            if std::path::Path::new(&p).exists() {
                return p;
            }
        }
    }

    // Bare name — std::process::Command will search $PATH as last resort
    "claude".to_string()
}

/// Build the ordered list of providers with current auth status.
pub fn probe_providers(_config: &AppConfig) -> Vec<ProviderEntry> {
    vec![
        ProviderEntry {
            id: "cenzontle",
            label: "Cenzontle",
            subtitle: "Cuervo Cloud · SSO",
            flow: AuthFlow::Browser,
            status: if cenzontle_authenticated() {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: Some("CENZONTLE_ACCESS_TOKEN"),
            keystore_key: Some("cenzontle:access_token"),
            hint: "Abre el navegador para el flujo SSO de Cuervo",
        },
        ProviderEntry {
            id: "anthropic",
            label: "Anthropic",
            subtitle: "Claude API · api key",
            flow: AuthFlow::ApiKey,
            status: if has_env_or_keystore("ANTHROPIC_API_KEY", "anthropic_api_key") {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: Some("ANTHROPIC_API_KEY"),
            keystore_key: Some("anthropic_api_key"),
            hint: "console.anthropic.com → API keys",
        },
        ProviderEntry {
            id: "openai",
            label: "OpenAI",
            subtitle: "GPT API · api key",
            flow: AuthFlow::ApiKey,
            status: if has_env_or_keystore("OPENAI_API_KEY", "openai_api_key") {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: Some("OPENAI_API_KEY"),
            keystore_key: Some("openai_api_key"),
            hint: "platform.openai.com → API keys",
        },
        ProviderEntry {
            id: "deepseek",
            label: "DeepSeek",
            subtitle: "DeepSeek API · api key",
            flow: AuthFlow::ApiKey,
            status: if has_env_or_keystore("DEEPSEEK_API_KEY", "deepseek_api_key") {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: Some("DEEPSEEK_API_KEY"),
            keystore_key: Some("deepseek_api_key"),
            hint: "platform.deepseek.com → API keys",
        },
        ProviderEntry {
            id: "gemini",
            label: "Google Gemini",
            subtitle: "Gemini API · api key",
            flow: AuthFlow::ApiKey,
            status: if has_env_or_keystore("GEMINI_API_KEY", "gemini_api_key") {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: Some("GEMINI_API_KEY"),
            keystore_key: Some("gemini_api_key"),
            hint: "aistudio.google.com → Get API key",
        },
        ProviderEntry {
            id: "claude_code",
            label: "Claude Code",
            subtitle: "claude.ai OAuth · browser",
            flow: AuthFlow::Browser,
            status: if claude_code_authenticated() {
                AuthStatus::Authenticated
            } else {
                AuthStatus::Missing
            },
            env_var: None,
            keystore_key: None,
            hint: "Requiere el binario `claude` instalado",
        },
        ProviderEntry {
            id: "ollama",
            label: "Ollama",
            subtitle: "servidor local · sin auth",
            flow: AuthFlow::NoAuth,
            status: AuthStatus::NoAuthRequired,
            env_var: None,
            keystore_key: None,
            hint: "Inicia con: ollama serve",
        },
    ]
}

/// True if at least one provider that requires credentials has them.
pub fn any_authenticated(entries: &[ProviderEntry]) -> bool {
    entries
        .iter()
        .any(|p| p.status == AuthStatus::Authenticated)
}

// ── Main entry point ──────────────────────────────────────────────────────────

/// Check auth state and, if needed, run the interactive gate.
///
/// `registry_has_no_real_providers` — true when every API-requiring provider
/// is missing from the registry (only echo/ollama at most).
pub async fn run_if_needed(
    config: &AppConfig,
    registry_has_no_real_providers: bool,
) -> Result<AuthGateOutcome> {
    if !registry_has_no_real_providers {
        return Ok(AuthGateOutcome {
            credentials_added: false,
        });
    }

    let providers = probe_providers(config);

    // If tokens exist somewhere (keystore, env) but registry is empty, the issue is
    // a config mismatch — let precheck_providers_explicit handle it with its own message.
    if any_authenticated(&providers) {
        return Ok(AuthGateOutcome {
            credentials_added: false,
        });
    }

    // Don't show the interactive gate in non-TTY environments (CI, pipes).
    if !crossterm::tty::IsTty::is_tty(&io::stdin()) {
        return Ok(AuthGateOutcome {
            credentials_added: false,
        });
    }

    run_gate(config, providers).await
}

// ── Box renderer — deterministic, width-safe, cross-platform ─────────────────

/// Characters used for box drawing.  Automatically selected based on terminal
/// Unicode capability — standard box-drawing on UTF-8 terminals, ASCII on others.
///
/// Design decision: we use STANDARD corners (┌┐└┘) not ROUNDED (╭╮╰╯) because
/// rounded corners are missing from many Linux monospace fonts (DejaVu Sans Mono,
/// Liberation Mono, Ubuntu Mono) and render with incorrect width or as replacement
/// glyphs, destroying the entire layout.
struct BoxChars {
    top_left: &'static str,
    top_right: &'static str,
    bottom_left: &'static str,
    bottom_right: &'static str,
    horizontal: &'static str,
    vertical: &'static str,
    tee_right: &'static str,
    tee_left: &'static str,
}

impl BoxChars {
    fn detect() -> Self {
        if detect_unicode_support() {
            Self::unicode()
        } else {
            Self::ascii()
        }
    }

    fn unicode() -> Self {
        Self {
            top_left: "\u{250C}",     // ┌
            top_right: "\u{2510}",    // ┐
            bottom_left: "\u{2514}",  // └
            bottom_right: "\u{2518}", // ┘
            horizontal: "\u{2500}",   // ─
            vertical: "\u{2502}",     // │
            tee_right: "\u{251C}",    // ├
            tee_left: "\u{2524}",     // ┤
        }
    }

    fn ascii() -> Self {
        Self {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            horizontal: "-",
            vertical: "|",
            tee_right: "+",
            tee_left: "+",
        }
    }
}

/// Detect Unicode support from the environment.
fn detect_unicode_support() -> bool {
    // Check LANG/LC_ALL/LC_CTYPE for UTF-8 indicators
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.contains("utf-8") || lower.contains("utf8") {
                return true;
            }
            // Explicit non-UTF locale (e.g. "C", "POSIX", "en_US.ISO-8859-1")
            if !val.is_empty() {
                return false;
            }
        }
    }
    // No locale set — default to true on macOS/Windows, false on Linux
    #[cfg(target_os = "linux")]
    {
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Determine the inner width for the box based on actual terminal size.
///
/// Returns the number of content columns between the left and right borders.
/// Guarantees: `inner >= MIN_INNER` and `inner + 2 <= term_width`.
fn effective_inner_width() -> usize {
    const PREFERRED_INNER: usize = 66;
    const MIN_INNER: usize = 48;

    let (term_width, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let tw = term_width as usize;

    // Need at least inner + 2 (for │ borders) to fit
    if tw < MIN_INNER + 2 {
        // Terminal is extremely narrow — use everything we can
        tw.saturating_sub(2).max(20)
    } else {
        PREFERRED_INNER.min(tw.saturating_sub(2))
    }
}

/// Pad or truncate `text` to exactly `target_width` display columns.
///
/// Uses `unicode-width` for correct measurement of all Unicode characters
/// including CJK, emoji, combining marks, and box-drawing.
fn pad_to_display_width(text: &str, target_width: usize) -> String {
    let current_width = UnicodeWidthStr::width(text);
    if current_width >= target_width {
        // Truncate from the right until we fit
        truncate_to_display_width(text, target_width)
    } else {
        // Pad with spaces
        let padding = target_width - current_width;
        let mut result = String::with_capacity(text.len() + padding);
        result.push_str(text);
        for _ in 0..padding {
            result.push(' ');
        }
        result
    }
}

/// Truncate `text` to at most `max_width` display columns.
/// Ensures we never split a multi-column character.
fn truncate_to_display_width(text: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > max_width {
            break;
        }
        result.push(ch);
        width += cw;
    }
    // Fill remaining columns with spaces if we stopped mid-width
    while width < max_width {
        result.push(' ');
        width += 1;
    }
    result
}

/// Render the top border: ┌──...──┐
fn render_box_top(stdout: &mut impl Write, bc: &BoxChars, inner: usize) -> Result<()> {
    let horiz: String = bc.horizontal.repeat(inner);
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.top_left),
        Print(&horiz),
        Print(bc.top_right),
        Print("\n"),
        ResetColor,
    )?;
    Ok(())
}

/// Render the bottom border: └──...──┘
fn render_box_bottom(stdout: &mut impl Write, bc: &BoxChars, inner: usize) -> Result<()> {
    let horiz: String = bc.horizontal.repeat(inner);
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.bottom_left),
        Print(&horiz),
        Print(bc.bottom_right),
        Print("\n"),
        ResetColor,
    )?;
    Ok(())
}

/// Render a divider: ├──...──┤
fn render_box_divider(stdout: &mut impl Write, bc: &BoxChars, inner: usize) -> Result<()> {
    let horiz: String = bc.horizontal.repeat(inner);
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.tee_right),
        Print(&horiz),
        Print(bc.tee_left),
        Print("\n"),
        ResetColor,
    )?;
    Ok(())
}

/// Render a content line: │<content padded to inner>│
///
/// The content is measured with `unicode-width` and padded/truncated to exactly
/// `inner` display columns, guaranteeing alignment with all borders.
fn render_box_line(
    stdout: &mut impl Write,
    bc: &BoxChars,
    inner: usize,
    text: &str,
    color: Color,
    bold: bool,
) -> Result<()> {
    let safe = pad_to_display_width(text, inner);

    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.vertical)
    )?;
    if bold {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }
    queue!(stdout, SetForegroundColor(color), Print(&safe),)?;
    if bold {
        queue!(stdout, SetAttribute(Attribute::NoBold))?;
    }
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.vertical),
        Print("\n"),
        ResetColor,
    )?;
    Ok(())
}

/// Render a provider row with proper column alignment.
///
/// Layout inside the box (all display-width-measured):
///   [prefix 4][icon 1][space 1][label col][subtitle col][tag col]
///
/// The columns are dynamically sized to fill exactly `inner` display columns.
fn render_provider_row(
    stdout: &mut impl Write,
    bc: &BoxChars,
    inner: usize,
    entry: &ProviderEntry,
    is_selected: bool,
    use_unicode: bool,
) -> Result<()> {
    // Selection prefix (4 display cols)
    let prefix = if is_selected { "  > " } else { "    " };

    // Status icon (1 display col) — use ASCII-safe alternatives
    let (status_icon, status_color) = if use_unicode {
        match entry.status {
            AuthStatus::Authenticated => ("*", Color::Green),
            AuthStatus::NoAuthRequired => ("o", Color::DarkGrey),
            AuthStatus::Missing => ("o", Color::DarkGrey),
        }
    } else {
        match entry.status {
            AuthStatus::Authenticated => ("*", Color::Green),
            AuthStatus::NoAuthRequired => ("o", Color::DarkGrey),
            AuthStatus::Missing => ("o", Color::DarkGrey),
        }
    };

    let flow_tag = match entry.flow {
        AuthFlow::Browser => "[browser]",
        AuthFlow::ApiKey => "[api key]",
        AuthFlow::NoAuth => "[no auth]",
    };

    // Fixed overhead: prefix(4) + icon(1) + space(1) + space_before_tag(1) + tag(9) = 16
    let fixed_overhead = 16usize;
    let available = inner.saturating_sub(fixed_overhead);

    // Split remaining space: ~40% label, ~60% subtitle
    let label_col = (available * 2 / 5).max(8);
    let sub_col = available.saturating_sub(label_col);

    let label_padded = pad_to_display_width(entry.label, label_col);
    let sub_padded = pad_to_display_width(entry.subtitle, sub_col);

    // Assemble: we control every column's display width, so total == inner
    let fg_color = if is_selected {
        Color::White
    } else {
        Color::DarkGrey
    };
    let label_color = if is_selected {
        Color::Cyan
    } else {
        Color::DarkGrey
    };
    let tag_color = match entry.flow {
        AuthFlow::Browser => {
            if is_selected {
                Color::Yellow
            } else {
                Color::DarkGrey
            }
        }
        AuthFlow::ApiKey => {
            if is_selected {
                Color::Blue
            } else {
                Color::DarkGrey
            }
        }
        AuthFlow::NoAuth => Color::DarkGrey,
    };

    // Left border
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.vertical)
    )?;

    // Prefix
    if is_selected {
        queue!(
            stdout,
            SetForegroundColor(Color::Cyan),
            SetAttribute(Attribute::Bold)
        )?;
    } else {
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
    }
    queue!(stdout, Print(prefix))?;

    // Status icon
    queue!(stdout, SetForegroundColor(status_color), Print(status_icon))?;

    // Space + Label
    queue!(stdout, SetForegroundColor(label_color),)?;
    if is_selected {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }
    queue!(stdout, Print(" "), Print(&label_padded))?;
    if is_selected {
        queue!(stdout, SetAttribute(Attribute::NoBold))?;
    }

    // Subtitle
    queue!(stdout, SetForegroundColor(fg_color), Print(&sub_padded),)?;

    // Tag (with leading space)
    queue!(
        stdout,
        SetForegroundColor(tag_color),
        Print(" "),
        Print(flow_tag),
    )?;

    // Right border
    queue!(
        stdout,
        SetForegroundColor(Color::DarkCyan),
        Print(bc.vertical),
        Print("\n"),
        ResetColor,
    )?;

    Ok(())
}

// ── Interactive gate ──────────────────────────────────────────────────────────

async fn run_gate(config: &AppConfig, providers: Vec<ProviderEntry>) -> Result<AuthGateOutcome> {
    let _ = config; // reserved for future use

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, Hide)?;

    let mut selected: usize = 0;
    let mut status_line = String::new();
    let mut status_ok = false;
    let mut credentials_added = false;

    'outer: loop {
        render_selector(&mut stdout, &providers, selected, &status_line, status_ok)?;

        match event::read()? {
            Event::Key(key) => {
                match key.code {
                    // Navigation
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                        status_line.clear();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected + 1 < providers.len() {
                            selected += 1;
                        }
                        status_line.clear();
                    }

                    // Confirm
                    KeyCode::Enter => {
                        let entry = &providers[selected];
                        match entry.flow {
                            AuthFlow::ApiKey => match run_api_key_input(&mut stdout, entry).await {
                                Ok(true) => {
                                    credentials_added = true;
                                    break 'outer;
                                }
                                Ok(false) => {
                                    status_line = "Configuracion cancelada.".into();
                                    status_ok = false;
                                }
                                Err(e) => {
                                    status_line = format!("Error: {e}");
                                    status_ok = false;
                                }
                            },
                            AuthFlow::Browser => {
                                terminal::disable_raw_mode()?;
                                execute!(stdout, Show, MoveTo(0, 0), Clear(ClearType::All))?;

                                match run_browser_flow(entry).await {
                                    Ok(true) => {
                                        credentials_added = true;
                                        break 'outer;
                                    }
                                    Ok(false) => {
                                        terminal::enable_raw_mode()?;
                                        execute!(stdout, Hide)?;
                                        status_line =
                                            "Login no completado. Intenta de nuevo.".into();
                                        status_ok = false;
                                    }
                                    Err(e) => {
                                        terminal::enable_raw_mode()?;
                                        execute!(stdout, Hide)?;
                                        status_line = format!("Error: {e}");
                                        status_ok = false;
                                    }
                                }
                            }
                            AuthFlow::NoAuth => {
                                terminal::disable_raw_mode()?;
                                execute!(stdout, Show, MoveTo(0, 0), Clear(ClearType::All))?;
                                show_noauth_instructions(entry)?;
                                break 'outer;
                            }
                        }
                    }

                    // Skip
                    KeyCode::Esc | KeyCode::Char('s') | KeyCode::Char('S') => break 'outer,

                    // Ctrl+C / Ctrl+D
                    KeyCode::Char('c') | KeyCode::Char('d')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        break 'outer
                    }

                    _ => {}
                }
            }
            Event::Resize(_, _) => {} // just redraw next iteration
            _ => {}
        }
    }

    let _ = terminal::disable_raw_mode();
    let _ = execute!(stdout, Show);

    if credentials_added {
        println!();
        print_styled(
            &mut stdout,
            Color::Green,
            "  Proveedor configurado. Iniciando sesion...\n",
        )?;
        stdout.flush()?;
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    Ok(AuthGateOutcome { credentials_added })
}

// ── Selector screen ───────────────────────────────────────────────────────────

fn render_selector(
    stdout: &mut impl Write,
    providers: &[ProviderEntry],
    selected: usize,
    status: &str,
    status_ok: bool,
) -> Result<()> {
    let bc = BoxChars::detect();
    let use_unicode = detect_unicode_support();
    let inner = effective_inner_width();

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    // Header
    render_box_top(stdout, &bc, inner)?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_line(
        stdout,
        &bc,
        inner,
        "  halcon -- configuracion de proveedor",
        Color::Cyan,
        true,
    )?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_line(
        stdout,
        &bc,
        inner,
        "  No hay ningun proveedor de IA autenticado.",
        Color::White,
        false,
    )?;
    render_box_line(
        stdout,
        &bc,
        inner,
        "  Selecciona uno para comenzar:",
        Color::DarkGrey,
        false,
    )?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_divider(stdout, &bc, inner)?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;

    // Provider rows
    for (i, entry) in providers.iter().enumerate() {
        render_provider_row(stdout, &bc, inner, entry, i == selected, use_unicode)?;
    }

    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_divider(stdout, &bc, inner)?;

    // Status line
    if !status.is_empty() {
        let color = if status_ok { Color::Green } else { Color::Red };
        render_box_line(stdout, &bc, inner, &format!("  {status}"), color, false)?;
    } else {
        render_box_line(
            stdout,
            &bc,
            inner,
            "  [Up/Down] navegar  [Enter] configurar  [S] omitir",
            Color::DarkGrey,
            false,
        )?;
    }

    render_box_bottom(stdout, &bc, inner)?;
    stdout.flush()?;
    Ok(())
}

// ── API key input screen ──────────────────────────────────────────────────────

async fn run_api_key_input(stdout: &mut impl Write, entry: &ProviderEntry) -> Result<bool> {
    let mut input = String::new();
    let mut err_msg = String::new();

    loop {
        render_api_key_screen(stdout, entry, &input, &err_msg)?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Enter => {
                    let key_val = input.trim().to_string();
                    if key_val.is_empty() {
                        err_msg = "No ingresaste ninguna clave.".into();
                        continue;
                    }
                    match save_api_key(entry, &key_val) {
                        Ok(()) => {
                            // Zero-out the input buffer before dropping
                            clear_string(&mut input);
                            return Ok(true);
                        }
                        Err(e) => {
                            err_msg = format!("Error al guardar: {e}");
                        }
                    }
                }
                KeyCode::Esc => {
                    clear_string(&mut input);
                    return Ok(false);
                }
                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if c == 'c' || c == 'd' {
                        clear_string(&mut input);
                        return Ok(false);
                    }
                    if c == 'u' {
                        clear_string(&mut input);
                    }
                    if c == 'w' {
                        // Delete last word
                        let trimmed = input.trim_end().to_string();
                        let word_end = trimmed
                            .rfind(|c: char| c.is_whitespace())
                            .map(|i| i + 1)
                            .unwrap_or(0);
                        input = trimmed[..word_end].to_string();
                    }
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    err_msg.clear();
                }
                KeyCode::Backspace => {
                    input.pop();
                    err_msg.clear();
                }
                _ => {}
            }
        }
    }
}

fn render_api_key_screen(
    stdout: &mut impl Write,
    entry: &ProviderEntry,
    input: &str,
    err: &str,
) -> Result<()> {
    let bc = BoxChars::detect();
    let inner = effective_inner_width();

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    render_box_top(stdout, &bc, inner)?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_line(
        stdout,
        &bc,
        inner,
        &format!("  {} -- API Key", entry.label),
        Color::Cyan,
        true,
    )?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_line(
        stdout,
        &bc,
        inner,
        &format!("  Obten tu clave en: {}", entry.hint),
        Color::DarkGrey,
        false,
    )?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;

    // Input field — mask with bullets (ASCII-safe)
    let max_bullets = inner.saturating_sub(12); // "  Clave: " prefix
    let masked: String = "*".repeat(input.len().min(max_bullets));
    let cursor = if input.is_empty() { "_" } else { "" };
    let field = format!("  Clave: {masked}{cursor}");
    render_box_line(stdout, &bc, inner, &field, Color::White, false)?;

    render_box_line(stdout, &bc, inner, "", Color::White, false)?;

    // Env var hint
    if let Some(env) = entry.env_var {
        render_box_line(
            stdout,
            &bc,
            inner,
            &format!("  Tambien puedes exportar: {env}=<tu_clave>"),
            Color::DarkGrey,
            false,
        )?;
    }

    render_box_line(
        stdout,
        &bc,
        inner,
        "  La clave se guarda de forma segura en el OS keystore.",
        Color::DarkGrey,
        false,
    )?;
    render_box_line(stdout, &bc, inner, "", Color::White, false)?;
    render_box_divider(stdout, &bc, inner)?;

    if !err.is_empty() {
        render_box_line(stdout, &bc, inner, &format!("  {err}"), Color::Red, false)?;
    } else {
        render_box_line(
            stdout,
            &bc,
            inner,
            "  [Enter] guardar  [Ctrl+U] limpiar  [Esc] volver",
            Color::DarkGrey,
            false,
        )?;
    }

    render_box_bottom(stdout, &bc, inner)?;
    stdout.flush()?;
    Ok(())
}

fn save_api_key(entry: &ProviderEntry, api_key: &str) -> Result<()> {
    // Save to OS keystore
    if let Some(ks_key) = entry.keystore_key {
        KeyStore::new(SERVICE_NAME)
            .set_secret(ks_key, api_key)
            .map_err(|e| anyhow::anyhow!("keystore error: {e}"))?;
    }

    // Set the env var for the current process so the rebuilt registry picks it up
    // immediately without needing a restart.
    //
    // SAFETY NOTE: This is called from the single-threaded auth gate, before the
    // async runtime spawns worker threads.  At this point the auth_gate holds raw
    // mode and is the only active execution context.  We use the std function
    // directly because there is no multi-thread concern at this call site.
    if let Some(env_var) = entry.env_var {
        // In Rust >= 1.80 set_var is unsafe.  The call is sound here because the
        // auth gate runs synchronously on the main thread before any tokio workers
        // have been spawned (raw-mode blocks the event loop).
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var(env_var, api_key);
        }
    }

    Ok(())
}

/// Overwrite a String's buffer with zeros before clearing it.
/// Best-effort defense against API keys lingering in heap memory.
fn clear_string(s: &mut String) {
    // SAFETY: we overwrite the valid UTF-8 bytes with 0x00, then clear the
    // string.  The bytes are never read as str while in a zero'd state.
    let bytes = unsafe { s.as_mut_vec() };
    for b in bytes.iter_mut() {
        // Use write_volatile to prevent the compiler from optimizing this out.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    s.clear();
}

// ── Browser OAuth flow ────────────────────────────────────────────────────────

/// Returns `true` if the browser flow completed successfully.
async fn run_browser_flow(entry: &ProviderEntry) -> Result<bool> {
    match entry.id {
        "cenzontle" => match super::sso::login().await {
            Ok(()) => Ok(true),
            Err(e) => {
                eprintln!("\n  Error durante el login: {e}");
                Ok(false)
            }
        },
        "claude_code" => match super::auth::login("claude_code") {
            Ok(()) => Ok(true),
            Err(e) => {
                eprintln!("\n  Error durante el login: {e}");
                Ok(false)
            }
        },
        _ => Ok(false),
    }
}

// ── Ollama instructions ───────────────────────────────────────────────────────

fn show_noauth_instructions(entry: &ProviderEntry) -> Result<()> {
    let mut stdout = io::stdout();
    println!();
    print_styled(&mut stdout, Color::Cyan, "  Ollama -- servidor local\n")?;
    println!();
    println!("  Ollama no requiere autenticacion, pero el servidor debe estar");
    println!("  corriendo localmente antes de iniciar halcon.");
    println!();
    print_styled(&mut stdout, Color::Yellow, &format!("  {}\n", entry.hint))?;
    println!();
    println!("  Despues de iniciar Ollama, ejecuta `halcon chat` de nuevo.");
    println!();
    stdout.flush()?;
    Ok(())
}

// ── Styling helper ───────────────────────────────────────────────────────────

fn print_styled(stdout: &mut impl Write, color: Color, text: &str) -> Result<()> {
    queue!(stdout, SetForegroundColor(color), Print(text), ResetColor)?;
    stdout.flush()?;
    Ok(())
}

// ── Registry empty check ──────────────────────────────────────────────────────

/// Returns true when no real AI provider is registered (only echo / ollama counts as "no real provider").
pub fn registry_has_no_real_providers(list: &[&str]) -> bool {
    list.iter().all(|n| *n == "echo" || *n == "ollama")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Existing tests (preserved) ──────────────────────────────────────────

    #[test]
    fn registry_empty_check_true_for_echo_only() {
        assert!(registry_has_no_real_providers(&["echo"]));
    }

    #[test]
    fn registry_empty_check_false_when_anthropic_present() {
        assert!(!registry_has_no_real_providers(&["echo", "anthropic"]));
    }

    #[test]
    fn registry_empty_check_true_for_empty() {
        assert!(registry_has_no_real_providers(&[]));
    }

    #[test]
    fn registry_empty_check_false_for_cenzontle() {
        assert!(!registry_has_no_real_providers(&["cenzontle", "echo"]));
    }

    #[test]
    fn registry_empty_check_ollama_alone_counts_as_empty() {
        assert!(registry_has_no_real_providers(&["ollama", "echo"]));
    }

    #[test]
    fn any_authenticated_false_when_all_missing() {
        let providers = vec![
            ProviderEntry {
                id: "anthropic",
                label: "Anthropic",
                subtitle: "",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Missing,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
            ProviderEntry {
                id: "openai",
                label: "OpenAI",
                subtitle: "",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Missing,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
        ];
        assert!(!any_authenticated(&providers));
    }

    #[test]
    fn any_authenticated_true_when_one_authenticated() {
        let providers = vec![
            ProviderEntry {
                id: "anthropic",
                label: "Anthropic",
                subtitle: "",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Authenticated,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
            ProviderEntry {
                id: "openai",
                label: "OpenAI",
                subtitle: "",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Missing,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
        ];
        assert!(any_authenticated(&providers));
    }

    #[test]
    fn provider_list_has_expected_entries() {
        use halcon_core::types::AppConfig;
        let config = AppConfig::default();
        let providers = probe_providers(&config);
        let ids: Vec<&str> = providers.iter().map(|p| p.id).collect();
        assert!(ids.contains(&"cenzontle"));
        assert!(ids.contains(&"anthropic"));
        assert!(ids.contains(&"openai"));
        assert!(ids.contains(&"deepseek"));
        assert!(ids.contains(&"gemini"));
        assert!(ids.contains(&"claude_code"));
        assert!(ids.contains(&"ollama"));
    }

    #[test]
    fn ollama_is_always_no_auth_required() {
        use halcon_core::types::AppConfig;
        let config = AppConfig::default();
        let providers = probe_providers(&config);
        let ollama = providers.iter().find(|p| p.id == "ollama").unwrap();
        assert_eq!(ollama.status, AuthStatus::NoAuthRequired);
        assert_eq!(ollama.flow, AuthFlow::NoAuth);
    }

    // ── New rendering correctness tests ─────────────────────────────────────

    #[test]
    fn pad_to_display_width_ascii() {
        let result = pad_to_display_width("hello", 10);
        assert_eq!(UnicodeWidthStr::width(result.as_str()), 10);
        assert_eq!(result, "hello     ");
    }

    #[test]
    fn pad_to_display_width_truncates_long() {
        let result = pad_to_display_width("this is a very long string", 10);
        assert_eq!(UnicodeWidthStr::width(result.as_str()), 10);
        assert_eq!(result, "this is a ");
    }

    #[test]
    fn pad_to_display_width_empty() {
        let result = pad_to_display_width("", 5);
        assert_eq!(UnicodeWidthStr::width(result.as_str()), 5);
        assert_eq!(result, "     ");
    }

    #[test]
    fn pad_to_display_width_exact() {
        let result = pad_to_display_width("exact", 5);
        assert_eq!(UnicodeWidthStr::width(result.as_str()), 5);
        assert_eq!(result, "exact");
    }

    #[test]
    fn truncate_preserves_exact_width() {
        let result = truncate_to_display_width("abcdefghij", 5);
        assert_eq!(UnicodeWidthStr::width(result.as_str()), 5);
    }

    #[test]
    fn box_top_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();
        render_box_top(&mut buf, &bc, inner).unwrap();
        let line = String::from_utf8(buf).unwrap();
        // Strip ANSI escape codes for width check
        let clean = strip_ansi(&line);
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "box_top width mismatch: got '{}' (width {})",
            clean_trimmed,
            UnicodeWidthStr::width(clean_trimmed)
        );
    }

    #[test]
    fn box_bottom_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();
        render_box_bottom(&mut buf, &bc, inner).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(UnicodeWidthStr::width(clean_trimmed), inner + 2);
    }

    #[test]
    fn box_divider_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();
        render_box_divider(&mut buf, &bc, inner).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(UnicodeWidthStr::width(clean_trimmed), inner + 2);
    }

    #[test]
    fn box_line_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();
        render_box_line(&mut buf, &bc, inner, "  test content", Color::White, false).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "box_line width mismatch: '{}'",
            clean_trimmed
        );
    }

    #[test]
    fn box_line_long_text_truncated() {
        let inner = 20;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "this is a very long text that exceeds the box width",
            Color::White,
            false,
        )
        .unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "long text was not truncated correctly: '{}'",
            clean_trimmed
        );
    }

    #[test]
    fn provider_row_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let entry = ProviderEntry {
            id: "anthropic",
            label: "Anthropic",
            subtitle: "Claude API - api key",
            flow: AuthFlow::ApiKey,
            status: AuthStatus::Missing,
            env_var: None,
            keystore_key: None,
            hint: "",
        };

        let mut buf = Vec::new();
        render_provider_row(&mut buf, &bc, inner, &entry, false, true).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "provider row width mismatch: '{}' (width {})",
            clean_trimmed,
            UnicodeWidthStr::width(clean_trimmed)
        );
    }

    #[test]
    fn provider_row_selected_has_correct_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let entry = ProviderEntry {
            id: "cenzontle",
            label: "Cenzontle",
            subtitle: "Cuervo Cloud - SSO",
            flow: AuthFlow::Browser,
            status: AuthStatus::Authenticated,
            env_var: None,
            keystore_key: None,
            hint: "",
        };

        let mut buf = Vec::new();
        render_provider_row(&mut buf, &bc, inner, &entry, true, true).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "selected provider row width mismatch: '{}' (width {})",
            clean_trimmed,
            UnicodeWidthStr::width(clean_trimmed)
        );
    }

    #[test]
    fn all_providers_rows_same_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        use halcon_core::types::AppConfig;
        let config = AppConfig::default();
        let providers = probe_providers(&config);

        for (i, entry) in providers.iter().enumerate() {
            for &selected in &[true, false] {
                let mut buf = Vec::new();
                render_provider_row(&mut buf, &bc, inner, entry, selected, true).unwrap();
                let clean = strip_ansi(&String::from_utf8(buf).unwrap());
                let clean_trimmed = clean.trim_end_matches('\n');
                let width = UnicodeWidthStr::width(clean_trimmed);
                assert_eq!(
                    width,
                    inner + 2,
                    "provider[{}] '{}' (selected={}) has width {} (expected {}): '{}'",
                    i,
                    entry.id,
                    selected,
                    width,
                    inner + 2,
                    clean_trimmed
                );
            }
        }
    }

    #[test]
    fn narrow_terminal_provider_rows_still_aligned() {
        let inner = 48; // Narrow terminal
        let bc = BoxChars::unicode();
        let entry = ProviderEntry {
            id: "anthropic",
            label: "Anthropic",
            subtitle: "Claude API - api key",
            flow: AuthFlow::ApiKey,
            status: AuthStatus::Missing,
            env_var: None,
            keystore_key: None,
            hint: "",
        };

        let mut buf = Vec::new();
        render_provider_row(&mut buf, &bc, inner, &entry, false, true).unwrap();
        let clean = strip_ansi(&String::from_utf8(buf).unwrap());
        let clean_trimmed = clean.trim_end_matches('\n');
        assert_eq!(
            UnicodeWidthStr::width(clean_trimmed),
            inner + 2,
            "narrow terminal row: '{}' (width {})",
            clean_trimmed,
            UnicodeWidthStr::width(clean_trimmed)
        );
    }

    #[test]
    fn ascii_box_chars_single_width() {
        let bc = BoxChars::ascii();
        assert_eq!(UnicodeWidthStr::width(bc.top_left), 1);
        assert_eq!(UnicodeWidthStr::width(bc.top_right), 1);
        assert_eq!(UnicodeWidthStr::width(bc.bottom_left), 1);
        assert_eq!(UnicodeWidthStr::width(bc.bottom_right), 1);
        assert_eq!(UnicodeWidthStr::width(bc.horizontal), 1);
        assert_eq!(UnicodeWidthStr::width(bc.vertical), 1);
    }

    #[test]
    fn unicode_box_chars_single_width() {
        let bc = BoxChars::unicode();
        assert_eq!(
            UnicodeWidthStr::width(bc.top_left),
            1,
            "┌ should be width 1"
        );
        assert_eq!(
            UnicodeWidthStr::width(bc.top_right),
            1,
            "┐ should be width 1"
        );
        assert_eq!(
            UnicodeWidthStr::width(bc.bottom_left),
            1,
            "└ should be width 1"
        );
        assert_eq!(
            UnicodeWidthStr::width(bc.bottom_right),
            1,
            "┘ should be width 1"
        );
        assert_eq!(
            UnicodeWidthStr::width(bc.horizontal),
            1,
            "─ should be width 1"
        );
        assert_eq!(
            UnicodeWidthStr::width(bc.vertical),
            1,
            "│ should be width 1"
        );
    }

    #[test]
    fn selector_full_render_all_lines_same_width() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let mut buf = Vec::new();

        // Render a complete selector to a buffer
        render_box_top(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_line(&mut buf, &bc, inner, "  halcon -- test", Color::Cyan, true).unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();

        let entry = ProviderEntry {
            id: "test",
            label: "Test Provider",
            subtitle: "test subtitle here",
            flow: AuthFlow::ApiKey,
            status: AuthStatus::Missing,
            env_var: None,
            keystore_key: None,
            hint: "",
        };
        render_provider_row(&mut buf, &bc, inner, &entry, false, true).unwrap();
        render_provider_row(&mut buf, &bc, inner, &entry, true, true).unwrap();

        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  footer text",
            Color::DarkGrey,
            false,
        )
        .unwrap();
        render_box_bottom(&mut buf, &bc, inner).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let expected_width = inner + 2;

        for (line_num, line) in output.lines().enumerate() {
            if line.is_empty() {
                continue; // trailing newline
            }
            let clean = strip_ansi(line);
            if clean.is_empty() {
                continue;
            }
            let width = UnicodeWidthStr::width(clean.as_str());
            assert_eq!(
                width,
                expected_width,
                "line {} has width {} (expected {}): '{}'",
                line_num + 1,
                width,
                expected_width,
                clean
            );
        }
    }

    #[test]
    fn clear_string_zeroes_buffer() {
        let mut s = String::from("secret_api_key_12345");
        let ptr = s.as_ptr();
        let len = s.len();
        clear_string(&mut s);
        assert!(s.is_empty());
        // Verify the original memory region is zeroed
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert!(bytes.iter().all(|&b| b == 0), "buffer was not zeroed");
    }

    // ── Test helper ────────────────────────────────────────────────────────

    /// Strip ANSI escape sequences from a string for width measurement.
    fn strip_ansi(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // Skip CSI sequence: ESC [ ... final_byte
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                                  // Skip parameter bytes (0x30-0x3F), intermediate bytes (0x20-0x2F),
                                  // stop at final byte (0x40-0x7E)
                    loop {
                        match chars.next() {
                            Some(c) if ('\x40'..='\x7e').contains(&c) => break,
                            Some(_) => continue,
                            None => break,
                        }
                    }
                }
            } else {
                result.push(ch);
            }
        }
        result
    }

    // ── Snapshot (visual regression) tests ──────────────────────────────

    /// Render the full selector screen to a buffer and snapshot it.
    /// Any change to the visual output will cause `cargo insta test` to fail,
    /// requiring explicit review via `cargo insta review`.
    #[test]
    fn snapshot_selector_unicode() {
        let inner = 66;
        let bc = BoxChars::unicode();
        let providers = make_test_providers();
        let mut buf = Vec::new();

        render_box_top(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  halcon -- configuracion de proveedor",
            Color::Cyan,
            true,
        )
        .unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  No hay ningun proveedor de IA autenticado.",
            Color::White,
            false,
        )
        .unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  Selecciona uno para comenzar:",
            Color::DarkGrey,
            false,
        )
        .unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();

        for (i, entry) in providers.iter().enumerate() {
            render_provider_row(&mut buf, &bc, inner, entry, i == 0, true).unwrap();
        }

        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  [Up/Down] navegar  [Enter] configurar  [S] omitir",
            Color::DarkGrey,
            false,
        )
        .unwrap();
        render_box_bottom(&mut buf, &bc, inner).unwrap();

        let raw_output = String::from_utf8(buf).unwrap();
        let clean = strip_ansi(&raw_output);
        insta::assert_snapshot!("selector_unicode_66", clean);
    }

    #[test]
    fn snapshot_selector_ascii() {
        let inner = 66;
        let bc = BoxChars::ascii();
        let providers = make_test_providers();
        let mut buf = Vec::new();

        render_box_top(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  halcon -- configuracion de proveedor",
            Color::Cyan,
            true,
        )
        .unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();

        for (i, entry) in providers.iter().enumerate() {
            render_provider_row(&mut buf, &bc, inner, entry, i == 0, false).unwrap();
        }

        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  [Up/Down] navegar  [Enter] configurar  [S] omitir",
            Color::DarkGrey,
            false,
        )
        .unwrap();
        render_box_bottom(&mut buf, &bc, inner).unwrap();

        let raw_output = String::from_utf8(buf).unwrap();
        let clean = strip_ansi(&raw_output);
        insta::assert_snapshot!("selector_ascii_66", clean);
    }

    #[test]
    fn snapshot_selector_narrow() {
        let inner = 50;
        let bc = BoxChars::unicode();
        let providers = make_test_providers();
        let mut buf = Vec::new();

        render_box_top(&mut buf, &bc, inner).unwrap();
        render_box_line(&mut buf, &bc, inner, "", Color::White, false).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  halcon -- config proveedor",
            Color::Cyan,
            true,
        )
        .unwrap();
        render_box_divider(&mut buf, &bc, inner).unwrap();

        for (i, entry) in providers.iter().enumerate() {
            render_provider_row(&mut buf, &bc, inner, entry, i == 2, true).unwrap();
        }

        render_box_divider(&mut buf, &bc, inner).unwrap();
        render_box_line(
            &mut buf,
            &bc,
            inner,
            "  [Up/Down] nav  [Enter] config  [S] skip",
            Color::DarkGrey,
            false,
        )
        .unwrap();
        render_box_bottom(&mut buf, &bc, inner).unwrap();

        let raw_output = String::from_utf8(buf).unwrap();
        let clean = strip_ansi(&raw_output);
        insta::assert_snapshot!("selector_narrow_50", clean);
    }

    /// Build a minimal set of providers for snapshot tests (deterministic).
    fn make_test_providers() -> Vec<ProviderEntry> {
        vec![
            ProviderEntry {
                id: "cenzontle",
                label: "Cenzontle",
                subtitle: "Cuervo Cloud - SSO",
                flow: AuthFlow::Browser,
                status: AuthStatus::Missing,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
            ProviderEntry {
                id: "anthropic",
                label: "Anthropic",
                subtitle: "Claude API - api key",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Authenticated,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
            ProviderEntry {
                id: "openai",
                label: "OpenAI",
                subtitle: "GPT API - api key",
                flow: AuthFlow::ApiKey,
                status: AuthStatus::Missing,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
            ProviderEntry {
                id: "ollama",
                label: "Ollama",
                subtitle: "servidor local - sin auth",
                flow: AuthFlow::NoAuth,
                status: AuthStatus::NoAuthRequired,
                env_var: None,
                keystore_key: None,
                hint: "",
            },
        ]
    }
}
