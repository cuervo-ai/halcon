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

/// OAuth login for the `claude_code` provider.
///
/// Delegates to `claude auth login` which opens the browser for claude.ai sign-in.
/// After successful login the OAuth token is stored by the Claude Code CLI itself
/// in `~/.claude.json` — halcon does not need to persist anything separately.
fn login_claude_code_oauth() -> Result<()> {
    // Find the claude binary.
    let claude_bin = find_claude_binary();
    println!("┌─ Claude Code — OAuth Login ──────────────────────────────┐");

    // 1. Check if already logged in.
    let status_out = std::process::Command::new(&claude_bin)
        .args(["auth", "status", "--json"])
        .output();

    if let Ok(out) = status_out {
        let json_str = String::from_utf8_lossy(&out.stdout);
        if json_str.contains("\"loggedIn\": true") || json_str.contains("\"loggedIn\":true") {
            // Already logged in — show status and exit.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
                let method = v["authMethod"].as_str().unwrap_or("unknown");
                let sub = v["subscriptionType"].as_str().unwrap_or("unknown");
                println!("│  ✓ Already authenticated                                  │");
                println!("│    Method: {method:<47}│");
                println!("│    Plan:   {sub:<47}│");
                println!("└──────────────────────────────────────────────────────────┘");
                println!();
                println!("Claude Code OAuth is active. Use: halcon -p claude_code chat \"...\"");
                return Ok(());
            }
        }
    }

    // 2. Not logged in — launch browser flow.
    println!("│  Opening browser for claude.ai sign-in...                │");
    println!("│  Sign in with your Anthropic account (Pro / Max / Team)  │");
    println!("└──────────────────────────────────────────────────────────┘");
    println!();

    let status = std::process::Command::new(&claude_bin)
        .args(["auth", "login"])
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run '{claude_bin} auth login': {e}"))?;

    if !status.success() {
        anyhow::bail!("Claude Code auth login failed (exit {})", status);
    }

    println!();
    println!("✓ Claude Code OAuth login complete.");
    println!("  Use: halcon -p claude_code chat \"tu pregunta\"");
    Ok(())
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
