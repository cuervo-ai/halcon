use std::io::{BufRead, BufReader};

use anyhow::Result;
use halcon_auth::KeyStore;
use serde_json;

const SERVICE_NAME: &str = "halcon-cli";

/// Known provider key names for the keychain.
fn provider_key(provider: &str) -> String {
    format!("{provider}_api_key")
}

/// Run `halcon auth login <provider>` — prompt for API key and store in keychain.
///
/// For `claude_code`, launches the Claude Code CLI OAuth browser flow instead of
/// prompting for an API key.
pub fn login(provider: &str) -> Result<()> {
    if provider == "claude_code" {
        return login_claude_code_oauth();
    }
    login_api_key(provider)
}

/// OAuth login for the `claude_code` provider — OpenCode-style browser flow.
///
/// Pattern (mirrors OpenCode's `McpOAuthCallback` + browser-open approach):
/// 1. Check if already authenticated via `claude auth status --json`
/// 2. If not: spawn `claude auth login` with piped stdout
/// 3. Read output line-by-line; when a https:// URL is detected → open browser +
///    print it as a manual fallback (for SSH / headless sessions)
/// 4. Confirm success via another `claude auth status` call
fn login_claude_code_oauth() -> Result<()> {
    let claude_bin = find_claude_binary();

    println!();
    println!("  ╔══════════════════════════════════════════════════════╗");
    println!("  ║        Claude Code — OAuth Login (claude.ai)         ║");
    println!("  ╚══════════════════════════════════════════════════════╝");
    println!();

    // ── 1. Already logged in? ────────────────────────────────────────────────
    if let Some((method, sub, email)) = check_claude_auth_status(&claude_bin) {
        let email_display = if email.is_empty() { "—".to_string() } else { email };
        println!("  ✓  Ya autenticado");
        println!("     Cuenta:      {email_display}");
        println!("     Método:      {method}");
        println!("     Plan:        {sub}");
        println!();
        println!("  Listo. Usa: halcon -p claude_code chat \"tu pregunta\"");
        println!();
        return Ok(());
    }

    // ── 2. Spawn claude auth login, pipe stdout to detect the auth URL ───────
    println!("  Iniciando flujo OAuth con claude.ai...");
    println!("  Se abrirá el navegador automáticamente.");
    println!("  Si no se abre, copia la URL que aparece abajo.");
    println!();

    let mut child = std::process::Command::new(&claude_bin)
        .args(["auth", "login"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit()) // show errors directly
        .spawn()
        .map_err(|e| anyhow::anyhow!("No se pudo lanzar '{claude_bin} auth login': {e}"))?;

    // ── 3. Read stdout line by line, open browser on first URL seen ──────────
    let mut browser_opened = false;
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            // Echo every line from the claude CLI so the user sees everything.
            println!("  {line}");

            // Detect a https:// URL in the output (the authorization URL).
            if let Some(url) = extract_https_url(&line) {
                if !browser_opened {
                    println!();
                    println!("  ┌─ URL de autorización ──────────────────────────────────┐");
                    println!("  │  {url}");
                    println!("  └────────────────────────────────────────────────────────┘");
                    println!();

                    // Try to open the default browser (OpenCode pattern).
                    match open_browser(&url) {
                        Ok(_) => println!("  ✓ Navegador abierto"),
                        Err(_) => println!("  ⚠ No se pudo abrir el navegador — copia la URL de arriba"),
                    }
                    println!();
                    browser_opened = true;
                }
            }
        }
    }

    let exit_status = child.wait()?;
    if !exit_status.success() {
        anyhow::bail!("claude auth login falló (código {})", exit_status);
    }

    // ── 4. Confirm final auth state ──────────────────────────────────────────
    println!();
    if let Some((method, sub, email)) = check_claude_auth_status(&claude_bin) {
        let email_display = if email.is_empty() { "—".to_string() } else { email };
        println!("  ✓  Login exitoso");
        println!("     Cuenta:      {email_display}");
        println!("     Método:      {method}");
        println!("     Plan:        {sub}");
        println!();
        println!("  Listo. Usa: halcon -p claude_code chat \"tu pregunta\"");
    } else {
        println!("  ✓  Proceso completado.");
        println!("     Ejecuta `halcon auth status` para verificar.");
    }
    println!();
    Ok(())
}

/// Run `claude auth status --json` and return (method, subscriptionType, email)
/// if logged in, or None otherwise.
fn check_claude_auth_status(claude_bin: &str) -> Option<(String, String, String)> {
    let out = std::process::Command::new(claude_bin)
        .args(["auth", "status", "--json"])
        .output()
        .ok()?;

    let json_str = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    if v["loggedIn"].as_bool() != Some(true) {
        return None;
    }

    let method = v["authMethod"].as_str().unwrap_or("unknown").to_string();
    let sub = v["subscriptionType"].as_str().unwrap_or("unknown").to_string();
    let email = v["email"].as_str().unwrap_or("").to_string();
    Some((method, sub, email))
}

/// Extract the first `https://` URL from a line of text.
fn extract_https_url(line: &str) -> Option<String> {
    // Find "https://" in the line and extract until whitespace or end.
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')')
        .unwrap_or(rest.len());
    let url = &rest[..end];
    if url.len() > 8 {
        Some(url.to_string())
    } else {
        None
    }
}

/// Open the default browser for the given URL.
///
/// - macOS: `open <url>`
/// - Linux: `xdg-open <url>`
/// - Fallback: silently fail (user sees the printed URL)
fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("open failed: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("xdg-open failed: {e}"))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("browser open not supported on this platform"))
}

/// Locate the `claude` binary: prefer the native install location, then PATH.
fn find_claude_binary() -> String {
    // Native install location (installed via `claude` installer script).
    if let Ok(home) = std::env::var("HOME") {
        let native = format!("{home}/.local/bin/claude");
        if std::path::Path::new(&native).exists() {
            return native;
        }
    }
    // Fall back to whatever is in PATH.
    "claude".to_string()
}

/// Manual API key entry — prompt and store in keychain.
fn login_api_key(provider: &str) -> Result<()> {
    let keystore = KeyStore::new(SERVICE_NAME);
    let key_name = provider_key(provider);

    // Read API key from stdin (hidden input).
    eprint!("Enter API key for {provider}: ");
    let api_key = read_hidden_line()?;
    let api_key = api_key.trim();

    if api_key.is_empty() {
        eprintln!("No key entered, aborting.");
        return Ok(());
    }

    keystore
        .set_secret(&key_name, api_key)
        .map_err(|e| anyhow::anyhow!("Failed to store API key: {e}"))?;

    println!("API key for {provider} stored in OS keychain.");
    Ok(())
}

/// Run `halcon auth logout <provider>` — remove API key from keychain.
pub fn logout(provider: &str) -> Result<()> {
    let keystore = KeyStore::new(SERVICE_NAME);
    let key_name = provider_key(provider);

    keystore
        .delete_secret(&key_name)
        .map_err(|e| anyhow::anyhow!("Failed to remove API key: {e}"))?;

    println!("API key for {provider} removed from OS keychain.");
    Ok(())
}

/// All known providers that may have API keys.
const KNOWN_PROVIDERS: &[&str] = &["anthropic", "openai", "deepseek", "gemini", "ollama"];

/// Run `halcon auth status` — show which providers have keys stored.
pub fn status() -> Result<()> {
    let keystore = KeyStore::new(SERVICE_NAME);

    println!("API key status:");

    // Claude Code uses OAuth via claude.ai — check via `claude auth status`.
    let claude_bin = find_claude_binary();
    let claude_status_str = std::process::Command::new(&claude_bin)
        .args(["auth", "status", "--json"])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            if s.trim().is_empty() { None } else { Some(s) }
        });

    let claude_code_status = match &claude_status_str {
        Some(json_str) => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                let logged_in = v["loggedIn"].as_bool().unwrap_or(false);
                if logged_in {
                    let method = v["authMethod"].as_str().unwrap_or("unknown");
                    let sub = v["subscriptionType"].as_str().unwrap_or("unknown");
                    format!("logged in (OAuth · {method} · {sub})  -> halcon -p claude_code chat")
                } else {
                    "not logged in  -> run `halcon auth login claude_code`".into()
                }
            } else {
                "unknown (run `claude auth status`)".into()
            }
        }
        None => "not installed or not found".into(),
    };
    println!("  claude_code: {claude_code_status}");

    // Cenzontle uses SSO tokens, not API keys — check its dedicated keychain entries.
    let cenzontle_token = keystore.get_secret("cenzontle:access_token").ok().flatten().is_some();
    let cenzontle_env = std::env::var("CENZONTLE_ACCESS_TOKEN")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let cenzontle_status: String = match (cenzontle_token, cenzontle_env) {
        (true, true) => "logged in (SSO keychain + $CENZONTLE_ACCESS_TOKEN)".into(),
        (true, false) => "logged in (SSO keychain)".into(),
        (false, true) => "set ($CENZONTLE_ACCESS_TOKEN)".into(),
        (false, false) => "not logged in  -> run `halcon auth login cenzontle`".into(),
    };
    println!("  cenzontle: {cenzontle_status}");

    for provider in KNOWN_PROVIDERS {
        let key_name = provider_key(provider);

        // Check keychain.
        let in_keychain = keystore.get_secret(&key_name).ok().flatten().is_some();

        // Check env var.
        let env_var = format!("{}_API_KEY", provider.to_uppercase());
        let in_env = std::env::var(&env_var)
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        let status = match (in_keychain, in_env) {
            (true, true) => format!("set (keychain + ${env_var})"),
            (true, false) => "set (keychain)".into(),
            (false, true) => format!("set (${env_var})"),
            (false, false) => "not set".into(),
        };

        println!("  {provider}: {status}");
    }
    Ok(())
}

/// Resolve the API key for a provider, checking keychain then env var.
pub fn resolve_api_key(provider: &str, env_var: Option<&str>) -> Option<String> {
    // 1. Check env var first (takes precedence).
    if let Some(var) = env_var {
        if let Ok(key) = std::env::var(var) {
            if !key.is_empty() {
                return Some(key);
            }
        }
    }

    // 2. Fall back to OS keychain.
    let keystore = KeyStore::new(SERVICE_NAME);
    let key_name = provider_key(provider);
    keystore.get_secret(&key_name).ok().flatten()
}

/// Read a line from stdin with echo disabled (for API key input).
fn read_hidden_line() -> Result<String> {
    // Try crossterm raw mode for hidden input.
    use std::io::{self, Read};
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| anyhow::anyhow!("Failed to enable raw mode: {e}"))?;

    let stdin = io::stdin();
    let mut line = String::new();
    // Read bytes until newline.
    for byte_result in stdin.lock().bytes() {
        match byte_result {
            Ok(b'\n') | Ok(b'\r') => break,
            Ok(3) => {
                // Ctrl+C
                crossterm::terminal::disable_raw_mode().ok();
                eprintln!();
                return Ok(String::new());
            }
            Ok(127) | Ok(8) => {
                // Backspace
                line.pop();
            }
            Ok(b) if b >= 32 => {
                line.push(b as char);
            }
            Ok(_) => {}
            Err(e) => {
                crossterm::terminal::disable_raw_mode().ok();
                return Err(anyhow::anyhow!("Read error: {e}"));
            }
        }
    }
    crossterm::terminal::disable_raw_mode().ok();
    eprintln!(); // newline after hidden input
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_format() {
        assert_eq!(provider_key("anthropic"), "anthropic_api_key");
        assert_eq!(provider_key("openai"), "openai_api_key");
    }

    #[test]
    fn resolve_api_key_from_env() {
        let var_name = "HALCON_TEST_KEY_12345";
        std::env::set_var(var_name, "sk-test-12345");
        let result = resolve_api_key("test", Some(var_name));
        assert_eq!(result, Some("sk-test-12345".into()));
        std::env::remove_var(var_name);
    }

    #[test]
    fn resolve_api_key_empty_env_returns_none() {
        let var_name = "HALCON_TEST_KEY_EMPTY";
        std::env::set_var(var_name, "");
        // Empty env var should fall through (keychain likely has nothing for "test").
        let result = resolve_api_key("test_nonexistent", Some(var_name));
        // Result is None since there's no keychain entry either.
        assert!(result.is_none());
        std::env::remove_var(var_name);
    }

    #[test]
    fn resolve_api_key_no_env_var() {
        // No env var set, no keychain entry.
        let result = resolve_api_key("nonexistent_provider_xyz", None);
        assert!(result.is_none());
    }
}
