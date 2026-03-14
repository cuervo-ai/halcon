#!/usr/bin/env sh
# Halcón CLI installer
# Usage: curl -sSfL https://halcon.cuervo.cloud/install.sh | sh
# Or:    curl -sSfL https://halcon.cuervo.cloud/install.sh | sh -s -- --version v0.3.0
set -e

HALCON_VERSION="${HALCON_VERSION:-latest}"
RELEASES_URL="${HALCON_RELEASES_URL:-https://releases.cli.cuervo.cloud}"
MANIFEST_URL="${RELEASES_URL}/latest/manifest.json"
INSTALL_DIR="${HALCON_INSTALL_DIR:-}"
BINARY_NAME="halcon"

# ─── Colors ────────────────────────────────────────────────────────────────
if [ -t 1 ] && command -v tput >/dev/null 2>&1; then
    RED="$(tput setaf 1)"
    GREEN="$(tput setaf 2)"
    YELLOW="$(tput setaf 3)"
    CYAN="$(tput setaf 6)"
    BOLD="$(tput bold)"
    RESET="$(tput sgr0)"
else
    RED="" GREEN="" YELLOW="" CYAN="" BOLD="" RESET=""
fi

info()  { printf "${CYAN}  →${RESET} %s\n" "$*"; }
ok()    { printf "${GREEN}  ✓${RESET} %s\n" "$*"; }
warn()  { printf "${YELLOW}  !${RESET} %s\n" "$*" >&2; }
error() { printf "${RED}  ✗${RESET} %s\n" "$*" >&2; exit 1; }

# ─── Parse args ────────────────────────────────────────────────────────────
parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --version) HALCON_VERSION="$2"; shift 2 ;;
            --dir)     INSTALL_DIR="$2";    shift 2 ;;
            --help|-h)
                printf "Halcon CLI Installer\n\n"
                printf "Options:\n"
                printf "  --version VERSION   Install specific version (default: latest)\n"
                printf "  --dir DIR           Install directory (default: auto-detected)\n"
                printf "\nExamples:\n"
                printf "  curl -sSfL https://halcon.cuervo.cloud/install.sh | sh\n"
                printf "  curl -sSfL https://halcon.cuervo.cloud/install.sh | sh -s -- --version v0.3.0\n"
                printf "  curl -sSfL https://halcon.cuervo.cloud/install.sh | sh -s -- --dir /usr/local/bin\n"
                exit 0 ;;
            *) error "Unknown argument: $1" ;;
        esac
    done
}

# ─── Platform detection ─────────────────────────────────────────────────────
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            # Detect musl vs glibc — musl has no /lib/ld-linux* but has /lib/ld-musl*
            # or ldd reports "musl" in version output
            _IS_MUSL=0
            if ldd --version 2>&1 | grep -qi musl; then
                _IS_MUSL=1
            elif ls /lib/ld-musl* >/dev/null 2>&1; then
                _IS_MUSL=1
            fi
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-musl" ;;  # always musl for x86_64 (static binary)
                aarch64|arm64)
                    if [ "$_IS_MUSL" = "1" ]; then
                        TARGET="aarch64-unknown-linux-musl"
                    else
                        TARGET="aarch64-unknown-linux-gnu"
                    fi
                    ;;
                armv7l)  TARGET="armv7-unknown-linux-musleabihf" ;;
                *) error "Unsupported Linux architecture: $ARCH" ;;
            esac
            EXT="tar.gz"
            ;;
        Darwin)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-apple-darwin" ;;
                arm64)   TARGET="aarch64-apple-darwin" ;;
                *) error "Unsupported macOS architecture: $ARCH" ;;
            esac
            EXT="tar.gz"
            ;;
        *) error "Unsupported OS: $OS. Use install.ps1 on Windows." ;;
    esac

    info "Platform: ${OS} ${ARCH} → ${TARGET}"
}

# ─── Install directory resolution ──────────────────────────────────────────
# Finds a writable install directory, fixing ownership if needed.
# Priority: user-specified > ~/.local/bin > ~/bin > /usr/local/bin
resolve_install_dir() {
    # If user explicitly specified --dir, honor it (create + fail loudly if unwritable)
    if [ -n "$INSTALL_DIR" ]; then
        _ensure_writable_dir "$INSTALL_DIR" || \
            error "Cannot write to --dir '${INSTALL_DIR}'. Check permissions."
        return
    fi

    # Candidates in preference order
    for candidate in \
        "$HOME/.local/bin" \
        "$HOME/bin" \
        "/usr/local/bin"
    do
        if _ensure_writable_dir "$candidate" 2>/dev/null; then
            INSTALL_DIR="$candidate"
            return
        fi
    done

    error "No writable install directory found. Try: --dir \$HOME/bin"
}

# Returns 0 if dir is (or can be made) writable by the current user.
# Fixes root-owned directories under $HOME by reclaiming ownership.
_ensure_writable_dir() {
    local dir="$1"

    # Does it exist?
    if [ -d "$dir" ]; then
        # Writable already → done
        if [ -w "$dir" ]; then
            return 0
        fi

        # Under $HOME and owned by root? Reclaim it.
        case "$dir" in
            "$HOME"*)
                _dir_owner="$(ls -ld "$dir" 2>/dev/null | awk '{print $3}')"
                if [ "$_dir_owner" = "root" ]; then
                    warn "Fixing ownership of ${dir} (was owned by root from a previous sudo install)"
                    if command -v sudo >/dev/null 2>&1; then
                        sudo chown "$(id -un)" "$dir" 2>/dev/null && return 0
                    fi
                fi
                ;;
        esac
        return 1
    fi

    # Does not exist — try to create it
    if mkdir -p "$dir" 2>/dev/null; then
        return 0
    fi

    # Under $HOME it might need a sudo mkdir if a parent is root-owned
    case "$dir" in
        "$HOME"*)
            if command -v sudo >/dev/null 2>&1; then
                sudo mkdir -p "$dir" 2>/dev/null && \
                sudo chown "$(id -un)" "$dir" 2>/dev/null && \
                return 0
            fi
            ;;
    esac

    return 1
}

# ─── Download helpers ───────────────────────────────────────────────────────
download() {
    local url="$1"
    local dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL --retry 3 --retry-delay 2 -o "$dest" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --tries=3 -O "$dest" "$url"
    else
        error "Neither curl nor wget found. Install one and retry."
    fi
}

# ─── SHA-256 verification ───────────────────────────────────────────────────
verify_sha256() {
    local file="$1"
    local expected="$2"

    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$file" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$file" | cut -d' ' -f1)"
    else
        warn "Cannot verify SHA-256: sha256sum/shasum not found"
        return 0
    fi

    if [ "$actual" != "$expected" ]; then
        error "SHA-256 mismatch!\n  Expected: $expected\n  Got:      $actual"
    fi
    ok "SHA-256 verified"
}

# ─── PATH configuration ──────────────────────────────────────────────────────
configure_path() {
    local dir="$1"

    # Already in PATH
    case ":${PATH}:" in
        *":${dir}:"*)
            ok "Already in PATH — no shell config change needed"
            return
            ;;
    esac

    local export_line="export PATH=\"\$PATH:${dir}\""
    local rc_file=""

    SHELL_NAME="$(basename "${SHELL:-sh}")"
    case "$SHELL_NAME" in
        zsh)  rc_file="$HOME/.zshrc" ;;
        bash) rc_file="$HOME/.bashrc" ;;
        fish)
            rc_file="$HOME/.config/fish/config.fish"
            export_line="fish_add_path ${dir}"
            ;;
        *)    rc_file="$HOME/.profile" ;;
    esac

    # Create rc file if it doesn't exist
    if [ ! -f "$rc_file" ]; then
        mkdir -p "$(dirname "$rc_file")" 2>/dev/null || true
        touch "$rc_file" 2>/dev/null || true
    fi

    if [ -f "$rc_file" ] && grep -qF "$dir" "$rc_file" 2>/dev/null; then
        ok "PATH already in ${rc_file}"
    elif [ -w "$rc_file" ] || [ -w "$(dirname "$rc_file")" ]; then
        printf '\n# Halcon CLI\n%s\n' "$export_line" >> "$rc_file"
        ok "PATH added to ${rc_file}"
    else
        warn "Could not update ${rc_file} — add manually:"
        warn "  ${export_line}"
    fi

    printf "\n${YELLOW}  To use halcon in this terminal session:${RESET}\n"
    printf "    ${BOLD}export PATH=\"\$PATH:${dir}\"${RESET}\n"
}

# ─── Main ────────────────────────────────────────────────────────────────────
main() {
    parse_args "$@"
    detect_platform

    printf "\n${BOLD}  Halcon CLI Installer${RESET}\n"
    printf "  ──────────────────────\n\n"

    TMPDIR_WORK="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR_WORK"' EXIT

    # ─── Resolve version ────────────────────────────────────────────────────
    REQUESTED_VERSION="$(printf '%s' "$HALCON_VERSION" | sed 's/^v//')"

    if [ "$REQUESTED_VERSION" = "latest" ]; then
        info "Fetching release manifest..."
        MANIFEST_FILE="$TMPDIR_WORK/manifest.json"
        download "$MANIFEST_URL" "$MANIFEST_FILE"

        if grep -q '"error"' "$MANIFEST_FILE" 2>/dev/null; then
            ERR="$(grep -o '"error": *"[^"]*"' "$MANIFEST_FILE" | sed 's/.*"\([^"]*\)".*/\1/')"
            error "Release API error: ${ERR}. Check https://releases.cli.cuervo.cloud/health"
        fi

        VERSION="$(grep -o '"version": *"[^"]*"' "$MANIFEST_FILE" | head -1 | sed 's/.*"\([^"]*\)".*/\1/')"
        if [ -z "$VERSION" ]; then
            error "Failed to parse version from manifest"
        fi
        info "Latest version: ${VERSION}"
    else
        VERSION="$REQUESTED_VERSION"
        info "Installing version: ${VERSION}"
    fi

    # ─── Resolve install directory ───────────────────────────────────────────
    resolve_install_dir
    info "Install directory: ${INSTALL_DIR}"

    # ─── Build artifact name & URLs ──────────────────────────────────────────
    ARTIFACT_NAME="halcon-${VERSION}-${TARGET}.${EXT}"
    if [ "$REQUESTED_VERSION" = "latest" ]; then
        DOWNLOAD_URL="${RELEASES_URL}/latest/${ARTIFACT_NAME}"
        CS_URL="${RELEASES_URL}/latest/checksums.txt"
        GITHUB_URL="https://github.com/cuervo-ai/halcon-cli/releases/latest"
    else
        DOWNLOAD_URL="${RELEASES_URL}/v${VERSION}/${ARTIFACT_NAME}"
        CS_URL="${RELEASES_URL}/v${VERSION}/checksums.txt"
        GITHUB_URL="https://github.com/cuervo-ai/halcon-cli/releases/tag/v${VERSION}"
    fi

    # ─── Check artifact is listed in manifest (fast-fail before download) ───
    # We already have the manifest if REQUESTED_VERSION=latest; fetch it for
    # specific versions too so we can give a helpful error.
    if [ -z "${MANIFEST_FILE:-}" ]; then
        MANIFEST_FILE="$TMPDIR_WORK/manifest.json"
        _MURL="${RELEASES_URL}/v${VERSION}/manifest.json"
        download "$_MURL" "$MANIFEST_FILE" 2>/dev/null || true
    fi
    if [ -f "$MANIFEST_FILE" ] && ! grep -q "\"${ARTIFACT_NAME}\"" "$MANIFEST_FILE" 2>/dev/null; then
        printf "\n${RED}  ✗${RESET} No pre-built binary for ${BOLD}${OS} ${ARCH}${RESET} (${TARGET}) in v${VERSION}.\n" >&2
        printf "\n  Available artifacts in this release:\n" >&2
        grep -o '"name": *"[^"]*"' "$MANIFEST_FILE" | sed 's/.*"\([^"]*\)".*/    • \1/' >&2 || true
        printf "\n  ${YELLOW}Install via script (recommended):${RESET}\n" >&2
        printf "    — Wait for the next release which may include your platform, or\n" >&2
        printf "    — Build from source: https://github.com/cuervo-ai/halcon-cli\n" >&2
        printf "    — Check available releases: ${GITHUB_URL}\n\n" >&2
        exit 1
    fi

    # ─── Fetch SHA-256 ──────────────────────────────────────────────────────
    EXPECTED_SHA=""
    CS_FILE="$TMPDIR_WORK/checksums.txt"
    if download "$CS_URL" "$CS_FILE" 2>/dev/null; then
        EXPECTED_SHA="$(grep "${ARTIFACT_NAME}" "$CS_FILE" | awk '{print $1}' | head -1)"
    fi

    # ─── Download artifact ──────────────────────────────────────────────────
    info "Downloading ${ARTIFACT_NAME}..."
    ARCHIVE_FILE="$TMPDIR_WORK/${ARTIFACT_NAME}"
    download "$DOWNLOAD_URL" "$ARCHIVE_FILE" || \
        error "Download failed. Check available artifacts: ${GITHUB_URL}"
    ok "Downloaded ($(du -sh "$ARCHIVE_FILE" | cut -f1))"

    # ─── Verify SHA-256 ─────────────────────────────────────────────────────
    if [ -n "$EXPECTED_SHA" ]; then
        info "Verifying SHA-256..."
        verify_sha256 "$ARCHIVE_FILE" "$EXPECTED_SHA"
    else
        warn "SHA-256 not available, skipping verification"
    fi

    # ─── Extract binary ─────────────────────────────────────────────────────
    info "Extracting..."
    EXTRACT_DIR="$TMPDIR_WORK/extract"
    mkdir -p "$EXTRACT_DIR"
    tar xzf "$ARCHIVE_FILE" -C "$EXTRACT_DIR"

    BINARY_SRC=""
    for candidate in \
        "$EXTRACT_DIR/${BINARY_NAME}" \
        "$EXTRACT_DIR/halcon-${VERSION}-${TARGET}/${BINARY_NAME}" \
        "$EXTRACT_DIR/${BINARY_NAME}-${VERSION}-${TARGET}/${BINARY_NAME}"
    do
        if [ -f "$candidate" ]; then
            BINARY_SRC="$candidate"
            break
        fi
    done

    if [ -z "$BINARY_SRC" ]; then
        BINARY_SRC="$(find "$EXTRACT_DIR" -name "${BINARY_NAME}" -type f 2>/dev/null | head -1)"
    fi

    if [ -z "$BINARY_SRC" ]; then
        error "Binary '${BINARY_NAME}' not found in archive. Contents: $(ls -la "$EXTRACT_DIR" 2>/dev/null)"
    fi

    # ─── Install ─────────────────────────────────────────────────────────────
    DEST="${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "$BINARY_SRC"

    # Remove existing binary first — if it's root-owned the cp would fail,
    # but rm succeeds as long as the parent directory is user-writable.
    rm -f "$DEST" 2>/dev/null || true

    # Use cp instead of mv — avoids cross-device and permission edge cases
    if cp "$BINARY_SRC" "$DEST" 2>/dev/null; then
        ok "Installed to ${DEST}"
    else
        # Last resort: try with sudo (only for system dirs like /usr/local/bin)
        case "$INSTALL_DIR" in
            /usr/local/bin|/usr/bin|/opt/*)
                if command -v sudo >/dev/null 2>&1; then
                    warn "Using sudo to install to ${INSTALL_DIR}"
                    sudo cp "$BINARY_SRC" "$DEST" && sudo chmod +x "$DEST" || \
                        error "Installation failed — try: --dir \$HOME/bin"
                    ok "Installed to ${DEST} (via sudo)"
                else
                    error "Cannot write to ${INSTALL_DIR} and sudo not available. Try: --dir \$HOME/bin"
                fi
                ;;
            *)
                error "Cannot write to ${INSTALL_DIR}. Try: curl ... | sh -s -- --dir \$HOME/bin"
                ;;
        esac
    fi

    # ─── Verify installation ─────────────────────────────────────────────────
    if "${DEST}" --version >/dev/null 2>&1; then
        ok "Halcon CLI ${VERSION} ready"
    else
        warn "Binary installed but could not verify — may need PATH update"
    fi

    # ─── PATH configuration ──────────────────────────────────────────────────
    configure_path "$INSTALL_DIR"

    # ─── Full-capacity configuration ─────────────────────────────────────────
    configure_halcon

    printf "\n${GREEN}${BOLD}  Installation complete!${RESET}\n"
    printf "\n  ${BOLD}Next step — add your API key:${RESET}\n"
    printf "    ${BOLD}halcon auth login anthropic${RESET}   ${CYAN}# recommended${RESET}\n"
    printf "    ${BOLD}halcon auth login openai${RESET}\n"
    printf "    ${BOLD}halcon auth login deepseek${RESET}    ${CYAN}# cheapest option${RESET}\n"
    printf "\n  ${BOLD}Then start chatting:${RESET}\n"
    printf "    ${BOLD}halcon chat --tui --full --expert${RESET}\n\n"
}

# ─── Full-capacity configuration ─────────────────────────────────────────────
# Writes ~/.halcon/config.toml and ~/.halcon/.mcp.json if they don't exist.
# Skips gracefully if the user already has a config.
configure_halcon() {
    HALCON_DIR="$HOME/.halcon"
    CONFIG_FILE="$HALCON_DIR/config.toml"
    MCP_FILE="$HALCON_DIR/.mcp.json"

    printf "\n  ${BOLD}Configuring Halcón...${RESET}\n"

    mkdir -p "$HALCON_DIR" 2>/dev/null || true

    # ── config.toml ──────────────────────────────────────────────────────────
    if [ -f "$CONFIG_FILE" ]; then
        ok "Config already exists — skipping (${CONFIG_FILE})"
    else
        info "Writing full-capacity config..."
        _write_config "$CONFIG_FILE"
        ok "Config written: ${CONFIG_FILE}"
    fi

    # ── .mcp.json ────────────────────────────────────────────────────────────
    if [ -f "$MCP_FILE" ]; then
        ok "MCP config already exists — skipping"
    else
        _write_mcp_config "$MCP_FILE"
        ok "MCP config written: ${MCP_FILE}"
    fi
}

_write_config() {
    local dest="$1"
    cat > "$dest" << 'HALCON_CONFIG'
# ═══════════════════════════════════════════════════════════════════════════════
#  HALCÓN CLI — Full-Capacity Configuration
#  Generated by install.sh
#
#  Usage:
#    halcon chat --tui --full --expert            # default provider
#    halcon -p openai    chat --tui --full --expert
#    halcon -p deepseek  chat --tui --full --expert   # cheapest
#    halcon -p ollama    chat --tui --full --expert   # local / no API key
#
#  Add API keys:
#    halcon auth login anthropic
#    halcon auth login openai
#    halcon auth login deepseek
# ═══════════════════════════════════════════════════════════════════════════════

# ── General ───────────────────────────────────────────────────────────────────
[general]
default_provider = "anthropic"
default_model    = "claude-sonnet-4-6"
max_tokens       = 16000
temperature      = 0.0

# ── Display ───────────────────────────────────────────────────────────────────
[display]
show_banner         = true
animations          = true
theme               = "fire"
ui_mode             = "expert"
brand_color         = "#e85200"
terminal_background = "#1a1a1a"
compact_width       = 0

# ── Agent Limits ──────────────────────────────────────────────────────────────
[agent.limits]
max_rounds              = 40
max_total_tokens        = 0
max_duration_secs       = 1800
tool_timeout_secs       = 120
provider_timeout_secs   = 300
max_parallel_tools      = 10
max_tool_output_chars   = 100000
max_concurrent_agents   = 3
max_cost_usd            = 0.0
clarification_threshold = 0.6

# ── Agent Routing ─────────────────────────────────────────────────────────────
[agent.routing]
strategy    = "quality"
mode        = "failover"
max_retries = 1
fallback_models = [
    "claude-haiku-4-5-20251001",
    "claude-sonnet-4-6",
    "gpt-4o-mini",
]
speculation_providers = []

# ── Compaction ────────────────────────────────────────────────────────────────
[agent.compaction]
enabled            = true
threshold_fraction = 0.55
keep_recent        = 8
max_context_tokens = 180000

# ── Model Selection ───────────────────────────────────────────────────────────
[agent.model_selection]
enabled                    = true
budget_cap_usd             = 0.0
complexity_token_threshold = 2000

# ── Planning ──────────────────────────────────────────────────────────────────
[planning]
enabled              = true
adaptive             = true
max_replans          = 3
min_confidence       = 0.65
timeout_secs         = 45
auto_learn_playbooks = false

# ── Reasoning ─────────────────────────────────────────────────────────────────
[reasoning]
enabled             = true
success_threshold   = 0.6
max_retries         = 2
exploration_factor  = 1.4
learning            = true
enable_loop_critic  = true
critic_timeout_secs = 60
critic_model        = "claude-haiku-4-5-20251001"
critic_provider     = "anthropic"

# ── Reflexion ─────────────────────────────────────────────────────────────────
[reflexion]
enabled            = true
max_reflections    = 5
reflect_on_success = false

# ── Memory ────────────────────────────────────────────────────────────────────
[memory]
enabled                = true
max_entries            = 10000
auto_summarize         = true
episodic               = true
retrieval_top_k        = 5
retrieval_token_budget = 2000
decay_half_life_days   = 14.0
rrf_k                  = 60.0

# ── Orchestrator ──────────────────────────────────────────────────────────────
[orchestrator]
enabled                   = true
max_concurrent_agents     = 3
sub_agent_timeout_secs    = 270
shared_budget             = true
enable_communication      = false
min_delegation_confidence = 0.7

# ── Task Framework ────────────────────────────────────────────────────────────
[task_framework]
enabled               = true
persist_tasks         = true
default_max_retries   = 3
default_retry_base_ms = 500
resume_on_startup     = false
strict_enforcement    = false

# ── Context ───────────────────────────────────────────────────────────────────
[context]
dynamic_tool_selection = true

[context.governance]
default_max_tokens_per_source = 0
default_ttl_secs              = 0

# ── Context Servers ───────────────────────────────────────────────────────────
[context_servers]
enabled = true

[context_servers.requirements]
enabled = true
priority = 100
token_budget = 2000
cache_ttl_secs = 3600

[context_servers.architecture]
enabled = true
priority = 90
token_budget = 2000
cache_ttl_secs = 3600

[context_servers.codebase]
enabled = true
priority = 80
token_budget = 2000
cache_ttl_secs = 3600

[context_servers.workflow]
enabled = true
priority = 70
token_budget = 1500
cache_ttl_secs = 3600

[context_servers.testing]
enabled = true
priority = 60
token_budget = 1500
cache_ttl_secs = 3600

[context_servers.security]
enabled = true
priority = 40
token_budget = 1000
cache_ttl_secs = 3600

# ── Security ──────────────────────────────────────────────────────────────────
[security]
pii_detection          = false
pii_action             = "redact"
audit_enabled          = true
tbac_enabled           = true
pre_execution_critique = false
session_grant_ttl_secs = 300
scan_system_prompts    = false

[security.guardrails]
enabled  = true
builtins = true
rules    = []

[security.analysis_mode]
enabled                  = true
allow_grep_recursive     = true
allow_find_project_files = true
analysis_tool_whitelist  = [
    "grep ", "grep -", "rg ",
    "find . ", "find src", "find crates",
    "cat ", "head ", "tail ", "wc ", "ls ",
    "cargo audit", "cargo check", "cargo test --",
    "npm audit", "npm ls", "yarn audit",
    "git log ", "git diff ", "git status", "git show ",
]

# ── Tools ─────────────────────────────────────────────────────────────────────
[tools]
confirm_destructive       = true
timeout_secs              = 120
prompt_timeout_secs       = 45
auto_approve_in_ci        = false
allow_write_in_ci         = false
allow_destructive_in_ci   = false
dry_run                   = false
command_blacklist         = []
disable_builtin_blacklist = false
allowed_directories = [
    ".",
    "/tmp",
]
blocked_patterns = [
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/credentials.json",
    "**/.ssh/**",
]

[tools.sandbox]
enabled             = true
max_output_bytes    = 10485760
max_memory_mb       = 4096
max_cpu_secs        = 60
max_file_size_bytes = 104857600

[tools.retry]
max_retries   = 3
base_delay_ms = 500
max_delay_ms  = 10000

# ── Cache ─────────────────────────────────────────────────────────────────────
[cache]
enabled          = true
default_ttl_secs = 3600
max_entries      = 1000
prompt_cache     = true

# ── Search ────────────────────────────────────────────────────────────────────
[search]
enabled         = true
max_documents   = 50000
enable_semantic = true
enable_cache    = true

[search.ranking]
bm25_weight             = 0.6
semantic_weight         = 0.3
pagerank_weight         = 0.1
use_rrf                 = true
min_semantic_similarity = 0.25

[search.query]
default_results      = 10
enable_feedback_loop = true

# ── Multimodal ────────────────────────────────────────────────────────────────
[multimodal]
enabled                 = true
mode                    = "api"
max_file_size_bytes     = 20971520
local_threshold_bytes   = 2097152
strip_exif              = true
privacy_strict          = false
max_audio_duration_secs = 300
max_video_duration_secs = 120
video_sample_fps        = 2
max_video_frames        = 25
max_concurrent_analyses = 4
cache_enabled           = true
cache_ttl_secs          = 3600
api_timeout_ms          = 30000

# ── Resilience ────────────────────────────────────────────────────────────────
[resilience]
enabled = true

[resilience.circuit_breaker]
failure_threshold  = 5
window_secs        = 60
open_duration_secs = 30
half_open_probes   = 2

[resilience.health]
window_minutes      = 60
degraded_threshold  = 50
unhealthy_threshold = 30

[resilience.backpressure]
max_concurrent_per_provider = 5
queue_timeout_secs          = 30

# ── Storage ───────────────────────────────────────────────────────────────────
[storage]
max_sessions         = 1000
max_session_age_days = 90

# ── Plugins ───────────────────────────────────────────────────────────────────
[plugins]
enabled = true

# ── Logging ───────────────────────────────────────────────────────────────────
[logging]
level  = "info"
format = "pretty"

# ── MCP ───────────────────────────────────────────────────────────────────────
[mcp]
max_reconnect_attempts = 3

# ── MCP Server (Halcon as MCP server for Claude Code etc.) ────────────────────
[mcp_server]
enabled          = false
transport        = "stdio"
port             = 7777
expose_agents    = true
require_auth     = true
session_ttl_secs = 1800

# ── Providers ─────────────────────────────────────────────────────────────────
[models.providers.anthropic]
enabled       = true
api_key_env   = "ANTHROPIC_API_KEY"
api_base      = "https://api.anthropic.com"
default_model = "claude-sonnet-4-6"

[models.providers.anthropic.http]
connect_timeout_secs = 10
request_timeout_secs = 300
max_retries          = 3
retry_base_delay_ms  = 1000

[models.providers.openai]
enabled       = true
api_key_env   = "OPENAI_API_KEY"
api_base      = "https://api.openai.com/v1"
default_model = "gpt-4o"

[models.providers.openai.http]
connect_timeout_secs = 10
request_timeout_secs = 300
max_retries          = 3
retry_base_delay_ms  = 1000

[models.providers.deepseek]
enabled       = true
api_key_env   = "DEEPSEEK_API_KEY"
api_base      = "https://api.deepseek.com"
default_model = "deepseek-chat"

[models.providers.deepseek.http]
connect_timeout_secs = 15
request_timeout_secs = 300
max_retries          = 3
retry_base_delay_ms  = 1000

[models.providers.gemini]
enabled       = true
api_key_env   = "GEMINI_API_KEY"
api_base      = "https://generativelanguage.googleapis.com"
default_model = "gemini-2.5-flash"

[models.providers.gemini.http]
connect_timeout_secs = 10
request_timeout_secs = 300
max_retries          = 3
retry_base_delay_ms  = 1000

[models.providers.ollama]
enabled       = true
api_base      = "http://localhost:11434"
default_model = "llama3.2"

[models.providers.ollama.http]
connect_timeout_secs = 10
request_timeout_secs = 300
max_retries          = 3
retry_base_delay_ms  = 1000

# ── Policy ────────────────────────────────────────────────────────────────────
[policy]
use_intent_pipeline          = true
use_boundary_decision_engine = true
use_halcon_md                = true
enable_hooks                 = false
enable_auto_memory           = true
memory_importance_threshold  = 0.30
enable_agent_registry        = true
enable_semantic_memory       = false
semantic_memory_top_k        = 5
success_threshold            = 0.6
halt_confidence_threshold    = 0.8
max_round_iterations         = 12
HALCON_CONFIG
}

_write_mcp_config() {
    local dest="$1"
    # Detect filesystem MCP server location
    _MCP_SERVER=""
    for candidate in \
        "/opt/homebrew/bin/mcp-server-filesystem" \
        "/usr/local/bin/mcp-server-filesystem" \
        "$(command -v mcp-server-filesystem 2>/dev/null)"
    do
        if [ -x "$candidate" ]; then
            _MCP_SERVER="$candidate"
            break
        fi
    done

    if [ -n "$_MCP_SERVER" ]; then
        cat > "$dest" << MCPEOF
{
  "mcpServers": {
    "filesystem": {
      "command": "$_MCP_SERVER",
      "args": [
        "$HOME/Documents",
        "$HOME/Downloads",
        "$HOME/Desktop",
        "/tmp"
      ]
    }
  }
}
MCPEOF
        ok "MCP filesystem server configured: ${_MCP_SERVER}"
    else
        cat > "$dest" << 'MCPEOF'
{
  "mcpServers": {}
}
MCPEOF
        warn "mcp-server-filesystem not found — MCP left empty"
        warn "Install it: npm install -g @modelcontextprotocol/server-filesystem"
    fi
}

main "$@"
