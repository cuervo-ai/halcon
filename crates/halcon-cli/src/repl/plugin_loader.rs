//! Plugin Loader — discovers and loads plugin manifests from disk.
//!
//! Scans `~/.halcon/plugins/*.plugin.toml`, validates each manifest,
//! auto-prefixes capability names (e.g. `run` → `plugin_<id>_run`),
//! registers `PluginProxyTool` instances in the session `ToolRegistry`,
//! and registers manifests in the `PluginRegistry`.
//!
//! All I/O is synchronous (no async) so callers can invoke on any thread.

use std::path::PathBuf;
use std::sync::Arc;

use super::plugin_manifest::{PluginManifest, PluginTransport};
use super::plugin_registry::PluginRegistry;
use super::plugin_transport_runtime::{PluginTransportRuntime, TransportHandle};

// ─── Result ───────────────────────────────────────────────────────────────────

/// Outcome of a `PluginLoader::load_into()` call.
#[derive(Debug, Clone, Default)]
pub struct PluginLoaderResult {
    /// Number of plugins successfully loaded and registered.
    pub loaded: usize,
    /// Plugins skipped because SHA-256 checksum did not match.
    pub skipped_checksum: usize,
    /// Plugins skipped due to manifest parse/validation errors.
    pub skipped_invalid: usize,
}

// ─── Loader ───────────────────────────────────────────────────────────────────

/// Discovers and validates plugin manifests from configured search paths.
///
/// Default search path: `~/.halcon/plugins/`.
/// Files must match the glob `*.plugin.toml`.
pub struct PluginLoader {
    search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    /// Create a loader with explicit search paths.
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        Self { search_paths }
    }

    /// Create a loader using the default plugin directory (`~/.halcon/plugins/`).
    pub fn default() -> Self {
        let default_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".halcon")
            .join("plugins");
        Self { search_paths: vec![default_dir] }
    }

    /// Discover all manifest files in the configured search paths.
    ///
    /// Returns `(path, raw_toml)` pairs for files ending in `.plugin.toml`.
    pub fn discover_raw(&self) -> Vec<(PathBuf, String)> {
        let mut found = Vec::new();
        for dir in &self.search_paths {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml")
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(".plugin.toml"))
                        .unwrap_or(false)
                {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        found.push((path, content));
                    }
                }
            }
        }
        found
    }

    /// Discover, parse and validate all plugin manifests.
    pub fn discover(&self) -> Vec<PluginManifest> {
        self.discover_raw()
            .into_iter()
            .filter_map(|(path, raw)| {
                match toml::from_str::<PluginManifest>(&raw) {
                    Ok(manifest) => {
                        tracing::debug!(
                            "Discovered plugin '{}' from {}",
                            manifest.meta.id,
                            path.display()
                        );
                        Some(manifest)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse plugin manifest at {}: {e}",
                            path.display()
                        );
                        None
                    }
                }
            })
            .collect()
    }

    /// Load all discovered plugins into the registries.
    ///
    /// For each valid manifest:
    /// 1. Optionally validates SHA-256 checksum.
    /// 2. Auto-prefixes capability names (`plugin_<id>_<cap>`).
    /// 3. Registers the manifest in `plugin_registry`.
    /// 4. Creates a `TransportHandle` and registers it in `runtime`.
    ///
    /// Returns a `PluginLoaderResult` with counts.
    pub fn load_into(
        &self,
        plugin_registry: &mut PluginRegistry,
        runtime: &mut PluginTransportRuntime,
    ) -> PluginLoaderResult {
        let raw_files = self.discover_raw();
        let mut result = PluginLoaderResult::default();

        for (path, raw) in raw_files {
            let mut manifest: PluginManifest = match toml::from_str(&raw) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        "Plugin manifest parse error at {}: {e}",
                        path.display()
                    );
                    result.skipped_invalid += 1;
                    continue;
                }
            };

            // Validate SHA-256 checksum when declared in manifest.
            if let Some(expected) = &manifest.meta.checksum {
                let actual = sha256_of(&raw);
                if actual != *expected {
                    tracing::warn!(
                        "Plugin '{}' checksum mismatch (expected {}, got {}) — skipping",
                        manifest.meta.id,
                        expected,
                        actual
                    );
                    result.skipped_checksum += 1;
                    continue;
                }
            }

            // Auto-prefix capability names.
            let id_underscored = manifest.meta.id.replace('-', "_");
            for cap in &mut manifest.capabilities {
                let prefix = format!("plugin_{id_underscored}_");
                if !cap.name.starts_with(&prefix) {
                    cap.name = format!("{prefix}{}", cap.name);
                }
            }

            // Build transport handle from manifest transport type.
            let handle = match &manifest.meta.transport {
                PluginTransport::Stdio { command, args } => TransportHandle::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                },
                PluginTransport::Http { base_url } => TransportHandle::Http {
                    client: Arc::new(reqwest::Client::new()),
                    base_url: base_url.clone(),
                },
                PluginTransport::Local | PluginTransport::InProcess => TransportHandle::Local,
            };

            runtime.register(manifest.meta.id.clone(), handle);
            plugin_registry.register(manifest);
            result.loaded += 1;
        }

        result
    }
}

/// Compute the lowercase hex SHA-256 of a UTF-8 string using the sha2 crate.
fn sha256_of(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(content.as_bytes());
    format!("{hash:x}")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::plugin_manifest::{
        PluginCategory, PluginMeta, PluginPermissions, RiskTier, SandboxContract,
        SupervisorPolicy, ToolCapabilityDescriptor,
    };
    use std::io::Write;
    use tempfile::TempDir;

    fn make_toml(id: &str) -> String {
        format!(
            r#"
[meta]
id = "{id}"
name = "{id}-plugin"
version = "1.0.0"

[meta.transport]
type = "local"

[[capabilities]]
name = "run"
description = "Run a task"
risk_tier = "low"
permission_level = "read_only"
budget_tokens_per_call = 100
"#
        )
    }

    fn write_plugin(dir: &TempDir, id: &str) -> PathBuf {
        let path = dir.path().join(format!("{id}.plugin.toml"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(make_toml(id).as_bytes()).unwrap();
        path
    }

    fn make_registry() -> PluginRegistry {
        PluginRegistry::new()
    }

    #[test]
    fn discover_raw_finds_plugin_tomls() {
        let dir = TempDir::new().unwrap();
        write_plugin(&dir, "my-plugin");
        // non-plugin toml should not be found
        std::fs::write(dir.path().join("ignored.toml"), "irrelevant").unwrap();

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let found = loader.discover_raw();
        assert_eq!(found.len(), 1);
        assert!(found[0].0.to_str().unwrap().contains("my-plugin"));
    }

    #[test]
    fn discover_empty_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        assert!(loader.discover().is_empty());
    }

    #[test]
    fn discover_nonexistent_dir_returns_empty() {
        let loader = PluginLoader::new(vec![PathBuf::from("/nonexistent_xyz_12345")]);
        assert!(loader.discover().is_empty());
    }

    #[test]
    fn load_into_registers_plugin() {
        let dir = TempDir::new().unwrap();
        write_plugin(&dir, "test-plugin");

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let mut registry = make_registry();
        let mut runtime = PluginTransportRuntime::new();

        let result = loader.load_into(&mut registry, &mut runtime);
        assert_eq!(result.loaded, 1);
        assert_eq!(result.skipped_invalid, 0);
        assert_eq!(result.skipped_checksum, 0);
        assert_eq!(registry.active_plugin_count(), 1);
    }

    #[test]
    fn load_into_autoprefixes_capability_names() {
        let dir = TempDir::new().unwrap();
        write_plugin(&dir, "my-plugin");

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let mut registry = make_registry();
        let mut runtime = PluginTransportRuntime::new();

        loader.load_into(&mut registry, &mut runtime);

        // The capability "run" should become "plugin_my_plugin_run"
        let tool_id = registry.plugin_id_for_tool("plugin_my_plugin_run");
        assert_eq!(tool_id, Some("my-plugin"), "auto-prefixed tool name must resolve");
    }

    #[test]
    fn load_into_skips_invalid_toml() {
        let dir = TempDir::new().unwrap();
        let bad_path = dir.path().join("bad.plugin.toml");
        std::fs::write(&bad_path, "not valid toml [[[").unwrap();

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let mut registry = make_registry();
        let mut runtime = PluginTransportRuntime::new();

        let result = loader.load_into(&mut registry, &mut runtime);
        assert_eq!(result.loaded, 0);
        assert_eq!(result.skipped_invalid, 1);
    }

    #[test]
    fn load_into_skips_checksum_mismatch() {
        let dir = TempDir::new().unwrap();
        let toml_with_checksum = format!(
            r#"
[meta]
id = "checksum-plugin"
name = "Checksum Plugin"
version = "1.0.0"
checksum = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"

[meta.transport]
type = "local"

[[capabilities]]
name = "run"
description = "Run"
"#
        );
        let path = dir.path().join("checksum-plugin.plugin.toml");
        std::fs::write(&path, &toml_with_checksum).unwrap();

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let mut registry = make_registry();
        let mut runtime = PluginTransportRuntime::new();

        let result = loader.load_into(&mut registry, &mut runtime);
        assert_eq!(result.loaded, 0);
        assert_eq!(result.skipped_checksum, 1);
    }

    #[test]
    fn load_multiple_plugins() {
        let dir = TempDir::new().unwrap();
        write_plugin(&dir, "plugin-alpha");
        write_plugin(&dir, "plugin-beta");

        let loader = PluginLoader::new(vec![dir.path().to_path_buf()]);
        let mut registry = make_registry();
        let mut runtime = PluginTransportRuntime::new();

        let result = loader.load_into(&mut registry, &mut runtime);
        assert_eq!(result.loaded, 2);
        assert_eq!(registry.active_plugin_count(), 2);
    }

    #[test]
    fn sha256_of_is_deterministic() {
        let h1 = sha256_of("hello world");
        let h2 = sha256_of("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn sha256_of_differs_for_different_input() {
        let h1 = sha256_of("hello");
        let h2 = sha256_of("world");
        assert_ne!(h1, h2);
    }
}
