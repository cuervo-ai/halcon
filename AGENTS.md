# Agent Guidelines for Halcon CLI

This document provides guidelines for agentic coding assistants (like Claude Code, Cursor, etc.) working on the Halcon CLI codebase. It includes build/lint/test commands, code style conventions, and project-specific patterns.

## Build Commands

```bash
# Build the main CLI binary (requires momoto submodule)
cargo build -p halcon-cli

# Build without color-science (CI-safe, no submodule)
cargo build -p halcon-cli --no-default-features --features tui

# Build release with color-science
cargo build --release -p halcon-cli

# Build for Linux targets (requires cross + Docker)
./scripts/build-cross.sh x86_64-unknown-linux-musl --release
./scripts/build-cross.sh aarch64-unknown-linux-gnu --release
./scripts/build-cross.sh aarch64-unknown-linux-musl --release

# Install binary to ~/.local/bin/halcon
make install
```

## Lint Commands

```bash
# Check formatting (must pass before commit)
cargo fmt --all -- --check

# Run clippy with all warnings as errors
cargo clippy --workspace --no-default-features -- -D warnings

# Type-check without codegen
cargo check --workspace --no-default-features
```

## Test Commands

```bash
# Run all workspace tests (CI-safe, no color-science)
cargo test --workspace --no-default-features

# Run color-science tests (requires momoto submodule)
cargo test -p halcon-cli --features color-science --lib

# Run delta-E palette validation
cargo test -p halcon-cli --features color-science --lib \
    tui_colors_perceptually_distinct_neon panel_sections_distinguishable \
    toast_levels_distinguishable -- --nocapture

# Run full test suite with both feature sets
make test-all

# Run a single test
cargo test -p halcon-cli --lib -- test_name

# Run tests with logging
RUST_LOG=debug cargo test -p halcon-cli --lib -- test_name

# Run integration tests
cargo test -p halcon-cli --test integration_test_name
```

## Code Style Guidelines

### Rust Style
- **Formatting**: Use `cargo fmt` before every commit. The CI enforces `--check`.
- **Linting**: `cargo clippy -- -D warnings` must pass with zero warnings.
- **Error Handling**: No `unwrap()` in production paths — use `?` or explicit error handling.
- **Async**: No `std::sync::Mutex` in async functions — use `tokio::sync::Mutex`.
- **Logging**: Prefer `tracing::` over `eprintln!`/`println!` in library code.

### Imports Order
Group imports in this order:
1. Standard library (`std`, `core`, `alloc`)
2. External crates (`tokio`, `serde`, `anyhow`)
3. Workspace crates (`halcon_core`, `halcon_tools`)
4. Current crate (`crate::` or `super::`)
5. Module-level imports (`self::`)

Use `use` statements sparingly; prefer fully qualified paths for rarely used types.

### Naming Conventions
- **Modules**: snake_case (`agent_runtime.rs`)
- **Types**: PascalCase (`HalconError`, `PermissionLevel`)
- **Functions**: snake_case (`event_bus`, `load_config`)
- **Variables**: snake_case (`session_id`, `provider_name`)
- **Constants**: SCREAMING_SNAKE_CASE (`DEFAULT_CAPACITY`)
- **Enums**: PascalCase with variant PascalCase (`EventPayload::SessionStarted`)

### Error Handling
- **Library crates**: Use `thiserror` for typed errors (`HalconError`).
- **Binary crate**: Use `anyhow` for context (`anyhow::Context`).
- **Result alias**: `pub type Result<T> = std::result::Result<T, HalconError>;`
- **Retryable errors**: Implement `is_retryable()` method on error types.

### Type Definitions
- Place shared types in `crates/halcon-core/src/types/`.
- Use `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` where appropriate.
- Include `#[serde(default)]` for backward compatibility.

### Documentation
- Use `///` doc comments for public items.
- Include examples for complex functions.
- Mark `// SAFETY:` comments for unsafe blocks.

## Security Guidelines

- **No new `unsafe` blocks** without `// SAFETY:` comment and team review.
- **Destructive tool operations** require explicit user confirmation.
- **No credentials, API keys, or tokens** in source or tests.
- **New tool implementations** must go through the FASE-2 security gate in `executor.rs`.
- **Security-sensitive paths** require `@cuervo-ai/security` review:
  - `crates/halcon-security/`
  - `crates/halcon-auth/`
  - `crates/halcon-sandbox/`
  - `crates/halcon-tools/src/bash.rs` (CATASTROPHIC_PATTERNS)
  - `crates/halcon-core/src/security.rs`

## Testing Guidelines

- **Unit tests**: Place in the same file using `#[cfg(test)] mod tests { ... }`.
- **Integration tests**: Go in `crates/halcon-cli/tests/`.
- **New features require new tests**.
- **Bug fixes require a regression test**.
- **No existing tests may be broken**.
- **Security-sensitive changes** require tests in `halcon-security/`.

## Commit Guidelines

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short description>

[optional body]

[optional footer]
```

**Types:** `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`

**Scopes:** `repl`, `tools`, `mcp`, `security`, `context`, `planning`, `cli`, `core`, `storage`, `async`

**Examples:**
```
feat(mcp): add OAuth 2.1 PKCE flow for HTTP servers
fix(async): replace std::sync::Mutex with tokio in model_selector
refactor(repl): migrate planning files to planning/ subdirectory
```

## Adding New Components

### Adding a New Tool
1. Implement `Tool` trait in `crates/halcon-tools/src/`.
2. Register in `halcon_tools::full_registry()`.
3. Add to `CATASTROPHIC_PATTERNS` check if the tool can delete/overwrite data.
4. Write tests for both allowed and blocked cases.

### Adding a New Provider
1. Implement `ModelProvider` trait in `crates/halcon-core/src/traits/`.
2. Add provider config to `crates/halcon-core/src/types/config.rs`.
3. Register in `crates/halcon-cli/src/repl/provider_normalization.rs`.
4. Add integration test in `crates/halcon-cli/tests/`.

## Project Structure

```
crates/
├── halcon-cli/          # Main CLI binary + REPL
├── halcon-core/         # Shared types and traits
├── halcon-tools/        # Tool implementations
├── halcon-context/      # Context pipeline + vector store
├── halcon-mcp/          # MCP server implementation
├── halcon-security/     # RBAC, sandboxing
├── halcon-storage/      # SQLite persistence
└── halcon-auth/         # OAuth 2.1 + PKCE
```

## Additional Resources

- [CONTRIBUTING.md](CONTRIBUTING.md) – Detailed contribution guide
- [SECURITY.md](SECURITY.md) – Security reporting and policies
- [README.md](README.md) – Project overview and usage
- CI Workflow: `.github/workflows/ci.yml`
- Makefile: `Makefile` for common tasks

---
*Last updated: March 2025*