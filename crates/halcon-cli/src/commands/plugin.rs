//! `halcon plugin` — manage V3 plugins.
//!
//! Sub-commands:
//! - `list`    — show all discovered plugins in ~/.halcon/plugins/
//! - `install` — copy a .plugin.toml manifest to the plugins directory
//! - `remove`  — delete a plugin manifest and its metrics from the plugins directory
//! - `status`  — show runtime plugin counts from the database

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use halcon_core::types::AppConfig;

/// Default plugin directory.
fn plugin_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".halcon")
        .join("plugins")
}

// ─── list ──────────────────────────────────────────────────────────────────────

/// List all plugin manifests discovered in ~/.halcon/plugins/.
pub fn list(_config: &AppConfig) -> Result<()> {
    let dir = plugin_dir();
    let mut out = io::stdout().lock();

    if !dir.exists() {
        let _ = writeln!(out, "No plugins directory found at {}", dir.display());
        let _ = writeln!(out, "Create it with: mkdir -p ~/.halcon/plugins");
        return Ok(());
    }

    let entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("read plugin dir {}", dir.display()))?
        .flatten()
        .filter(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".plugin.toml"))
                .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        let _ = writeln!(
            out,
            "No plugins installed. Use 'halcon plugin install <path>' to add one."
        );
        return Ok(());
    }

    let _ = writeln!(out, "\nInstalled plugins ({}):\n", entries.len());
    for entry in &entries {
        let path = entry.path();
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        // Extract id from toml quickly (no full parse needed for display).
        let id = extract_toml_field(&raw, "id").unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .trim_end_matches(".plugin")
                .to_string()
        });
        let version = extract_toml_field(&raw, "version").unwrap_or_else(|| "?".to_string());
        let name = extract_toml_field(&raw, "name").unwrap_or_else(|| id.clone());
        let _ = writeln!(out, "  {id} ({name}) v{version}");
    }
    let _ = writeln!(out);
    Ok(())
}

// ─── install ───────────────────────────────────────────────────────────────────

/// Install a plugin by copying its manifest to ~/.halcon/plugins/.
///
/// The source must be a `.plugin.toml` file. On conflict the existing manifest
/// is overwritten (the user has explicitly requested installation).
pub fn install(_config: &AppConfig, source: &str) -> Result<()> {
    let src = PathBuf::from(source);
    if !src.exists() {
        bail!("Plugin manifest not found: {}", src.display());
    }
    if src.extension().and_then(|e| e.to_str()) != Some("toml") {
        bail!("Plugin manifest must be a .toml file: {}", src.display());
    }

    let dest_dir = plugin_dir();
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("create plugin dir {}", dest_dir.display()))?;

    // Use the source filename; validate it ends with .plugin.toml.
    let filename = src
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| "Cannot determine source filename")?;
    if !filename.ends_with(".plugin.toml") {
        bail!("Manifest filename must end with '.plugin.toml', got: {filename}");
    }

    let dest = dest_dir.join(filename);
    std::fs::copy(&src, &dest)
        .with_context(|| format!("copy {} → {}", src.display(), dest.display()))?;

    let mut out = io::stdout().lock();
    let _ = writeln!(out, "Plugin installed: {}", dest.display());
    let _ = writeln!(out, "Restart halcon for the plugin to take effect.");
    Ok(())
}

// ─── remove ────────────────────────────────────────────────────────────────────

/// Remove a plugin by ID or filename.
///
/// Deletes `~/.halcon/plugins/<id>.plugin.toml`. Pass `force = true` to skip
/// confirmation prompt.
pub fn remove(_config: &AppConfig, id: &str, force: bool) -> Result<()> {
    let dir = plugin_dir();
    // Try both "id.plugin.toml" and "id" (user might omit the extension).
    let filename = if id.ends_with(".plugin.toml") {
        id.to_string()
    } else {
        format!("{id}.plugin.toml")
    };
    let path = dir.join(&filename);

    if !path.exists() {
        bail!("Plugin not found: {} (looked at {})", id, path.display());
    }

    if !force {
        let mut out = io::stdout().lock();
        let _ = write!(out, "Remove plugin '{id}'? [y/N] ");
        drop(out);
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    std::fs::remove_file(&path).with_context(|| format!("delete {}", path.display()))?;

    println!("Plugin '{id}' removed.");
    Ok(())
}

// ─── status ────────────────────────────────────────────────────────────────────

/// Show plugin system status: count of manifests on disk and directory path.
pub fn status(_config: &AppConfig) -> Result<()> {
    let dir = plugin_dir();
    let mut out = io::stdout().lock();

    let _ = writeln!(out, "\nPlugin system status:");
    let _ = writeln!(out, "  Directory: {}", dir.display());

    if !dir.exists() {
        let _ = writeln!(out, "  Status: directory not found (no plugins configured)");
        return Ok(());
    }

    let count = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(".plugin.toml"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);

    let _ = writeln!(out, "  Manifests on disk: {count}");
    if count == 0 {
        let _ = writeln!(
            out,
            "  Hint: use 'halcon plugin install <path>' to add a plugin."
        );
    } else {
        let _ = writeln!(out, "  Run 'halcon plugin list' to see details.");
    }
    Ok(())
}

// ─── helpers ───────────────────────────────────────────────────────────────────

/// Extract a quoted string field value from a TOML string without full parsing.
///
/// Matches the first occurrence of `key = "value"` or `key = 'value'`.
fn extract_toml_field(toml: &str, key: &str) -> Option<String> {
    let pattern = format!(" {key} =");
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let after_eq = trimmed.split_once('=')?.1.trim();
            let value = after_eq.trim_matches(|c| c == '"' || c == '\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    let _ = pattern;
    None
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::AppConfig;
    use std::io::Write;
    use tempfile::TempDir;

    fn config() -> AppConfig {
        AppConfig::default()
    }

    fn write_manifest(dir: &TempDir, id: &str) -> PathBuf {
        let path = dir.path().join(format!("{id}.plugin.toml"));
        let content = format!(
            r#"
[meta]
id = "{id}"
name = "Plugin {id}"
version = "1.0.0"
"#
        );
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn list_empty_dir_does_not_panic() {
        // list() reads from plugin_dir() (~/.halcon/plugins) — just assert no panic
        let cfg = config();
        // We can't easily override plugin_dir; just ensure it doesn't panic
        let _ = list(&cfg);
    }

    #[test]
    fn install_rejects_missing_file() {
        let cfg = config();
        let result = install(&cfg, "/nonexistent/plugin.plugin.toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn install_rejects_wrong_extension() {
        let dir = TempDir::new().unwrap();
        let bad = dir.path().join("plugin.json");
        std::fs::write(&bad, "{}").unwrap();
        let cfg = config();
        let result = install(&cfg, bad.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn install_rejects_bad_filename() {
        let dir = TempDir::new().unwrap();
        // Valid extension but doesn't end with .plugin.toml
        let bad = dir.path().join("myplugin.toml");
        std::fs::write(&bad, "[meta]\nid = \"x\"").unwrap();
        let cfg = config();
        let result = install(&cfg, bad.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("plugin.toml"));
    }

    #[test]
    fn remove_missing_plugin_returns_err() {
        let cfg = config();
        let result = remove(&cfg, "totally-nonexistent-plugin-xyz", true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn extract_toml_field_finds_quoted_value() {
        let toml = r#"
[meta]
id = "my-plugin"
name = "My Plugin"
version = "2.3.1"
"#;
        assert_eq!(
            extract_toml_field(toml, "id"),
            Some("my-plugin".to_string())
        );
        assert_eq!(
            extract_toml_field(toml, "version"),
            Some("2.3.1".to_string())
        );
        assert_eq!(extract_toml_field(toml, "missing"), None);
    }
}
