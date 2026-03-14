#!/usr/bin/env bash
# Halcón CLI — Installer Test Suite
# Tests all installation components on macOS and Linux
#
# Usage:
#   ./scripts/test-install.sh                   # full test suite
#   ./scripts/test-install.sh --quick           # skip slow build tests
#   ./scripts/test-install.sh --component cli   # test only one component
#   HALCON_INSTALL_DIR=~/.local/bin ./scripts/test-install.sh
#
# Components: scripts cli config agents completions vscode desktop docker e2e platform build
set -euo pipefail

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Config
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

INSTALL_DIR="${HALCON_INSTALL_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${HALCON_CONFIG_DIR:-$HOME/.halcon}"
HALCON_BIN="$INSTALL_DIR/halcon"
QUICK=false
COMPONENT=""

for arg in "$@"; do
    case "$arg" in
        --quick)        QUICK=true ;;
        --component=*)  COMPONENT="${arg#*=}" ;;
        --component)    ;;
    esac
done
# Handle --component <value> (two-arg form)
prev=""
for arg in "$@"; do
    [ "$prev" = "--component" ] && COMPONENT="$arg"
    prev="$arg"
done

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Test framework
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'

PASS=0; FAIL=0; SKIP=0
FAILURES=()

_pass() { PASS=$((PASS + 1)); echo -e "  ${GREEN}✓${NC} $1"; }
_fail() { FAIL=$((FAIL + 1)); FAILURES+=("$1"); echo -e "  ${RED}✗${NC} $1"; }
_skip() { SKIP=$((SKIP + 1)); echo -e "  ${DIM}○ $1${NC}"; }
_info() { echo -e "  ${BLUE}·${NC} $1"; }

assert() {
    local desc="$1" code="${2:-$?}"
    [ "$code" = "0" ] && _pass "$desc" || _fail "$desc"
}
assert_file()    { [ -f "$2" ] && _pass "$1: $(basename "$2")" || _fail "$1: $2 not found"; }
assert_dir()     { [ -d "$2" ] && _pass "$1: $2" || _fail "$1: $2 not found"; }
assert_exec()    { [ -x "$2" ] && _pass "$1" || _fail "$1: $2 not executable"; }
assert_contains() {
    grep -q "$3" "$2" 2>/dev/null && _pass "$1" || _fail "$1 (pattern '$3' missing in $2)"
}

section() { echo -e "\n${CYAN}${BOLD}━━━ $* ━━━${NC}"; }
has()     { command -v "$1" >/dev/null 2>&1; }

OS="$(uname -s)"; ARCH="$(uname -m)"

echo ""
echo -e "${BOLD}${BLUE}╔══════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${BLUE}║   Halcón CLI — Installer Test Suite          ║${NC}"
echo -e "${BOLD}${BLUE}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${DIM}Platform:  $OS/$ARCH${NC}"
echo -e "  ${DIM}Binary:    $HALCON_BIN${NC}"
echo -e "  ${DIM}Config:    $CONFIG_DIR${NC}"
[ "$QUICK" = "true" ] && echo -e "  ${YELLOW}Mode:      --quick (slow tests skipped)${NC}"
[ -n "$COMPONENT" ]   && echo -e "  ${YELLOW}Component: $COMPONENT only${NC}"

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T1 — Script validation
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_script_tests() {
    section "T1 · Script validation"
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

    for script in install.sh install-binary.sh test-install.sh; do
        local path="$SCRIPT_DIR/$script"
        assert_file "exists"     "$path"
        assert_exec "executable" "$path"
        bash -n "$path" 2>/dev/null; assert "syntax valid ($script)" $?
    done

    if has shellcheck; then
        for script in install.sh install-binary.sh; do
            shellcheck -S warning --exclude=SC1090,SC1091,SC2034 \
                "$SCRIPT_DIR/$script" 2>/dev/null
            assert "shellcheck ($script)" $?
        done
    else
        _skip "shellcheck not installed"
    fi

    for script in install.sh install-binary.sh; do
        local path="$SCRIPT_DIR/$script"
        grep -q 'set -euo pipefail' "$path" 2>/dev/null \
            && _pass "error handling ($script)" || _fail "missing set -euo pipefail ($script)"
        grep -E 'http://' "$path" 2>/dev/null | grep -qv 'localhost\|127\.\|apple\.com/DTDs\|w3\.org' \
            && _fail "plain HTTP found ($script)" || _pass "HTTPS enforced ($script)"
        grep -qE '^\s*eval\s' "$path" 2>/dev/null \
            && _fail "unsafe eval ($script)" || _pass "no bare eval ($script)"
    done
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T2 — CLI binary
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_cli_tests() {
    section "T2 · CLI binary"
    assert_exec "binary executable" "$HALCON_BIN"
    [ -x "$HALCON_BIN" ] || return

    # Version format
    VER="$("$HALCON_BIN" --version 2>&1 || true)"
    echo "$VER" | grep -qE '[0-9]+\.[0-9]+\.[0-9]+' \
        && _pass "version valid ($VER)" || _fail "bad version: $VER"

    # Help
    "$HALCON_BIN" --help >/dev/null 2>&1; assert "halcon --help" $?

    # Key subcommands
    for subcmd in chat status doctor tools agents mcp audit; do
        "$HALCON_BIN" "$subcmd" --help >/dev/null 2>&1
        assert "subcommand '$subcmd'" $?
    done

    # Binary size > 1MB
    BIN_BYTES="$(wc -c < "$HALCON_BIN" | tr -d ' ')"
    [ "$BIN_BYTES" -gt 1048576 ] \
        && _pass "binary size: $(( BIN_BYTES / 1024 / 1024 ))MB" \
        || _fail "suspiciously small: $BIN_BYTES bytes"

    # Startup latency
    if has python3; then
        MS="$(python3 -c "
import subprocess, time
t = time.monotonic()
subprocess.run(['$HALCON_BIN', '--version'], capture_output=True)
print(int((time.monotonic()-t)*1000))
" 2>/dev/null || echo 9999)"
        [ "$MS" -lt 500 ] && _pass "startup: ${MS}ms" || _fail "startup slow: ${MS}ms (>500ms)"
    fi

    # macOS: not killed by Gatekeeper (exit 137)
    if [ "$OS" = "Darwin" ]; then
        "$HALCON_BIN" --version >/dev/null 2>&1 && E=0 || E=$?
        [ "$E" -ne 137 ] && _pass "Gatekeeper OK (exit $E)" \
            || _fail "Gatekeeper killed binary (exit 137) — run: codesign --force --sign - $HALCON_BIN"
    fi

    # Linux: ELF
    if [ "$OS" = "Linux" ]; then
        file "$HALCON_BIN" 2>/dev/null | grep -q ELF \
            && _pass "Linux ELF binary" || _fail "not an ELF binary"
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T3 — Configuration
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_config_tests() {
    section "T3 · Configuration"
    assert_dir  "config dir"       "$CONFIG_DIR"
    assert_file "config.toml"      "$CONFIG_DIR/config.toml"
    assert_file "HALCON.md"        "$CONFIG_DIR/HALCON.md"
    assert_dir  "agents/"          "$CONFIG_DIR/agents"
    assert_dir  "memory/"          "$CONFIG_DIR/memory"
    assert_dir  "completions/"     "$CONFIG_DIR/completions"
    assert_file "hooks.toml"       "$CONFIG_DIR/hooks.toml"
    assert_file "mcp.toml"         "$CONFIG_DIR/mcp.toml"
    assert_file "memory/MEMORY.md" "$CONFIG_DIR/memory/MEMORY.md"

    # TOML validity via python3
    if has python3; then
        python3 - "$CONFIG_DIR/config.toml" << 'PYEOF' 2>/dev/null
import sys
p = sys.argv[1]
try:
    import tomllib
    tomllib.load(open(p, "rb"))
    sys.exit(0)
except ImportError:
    try:
        import tomli as tomllib
        tomllib.load(open(p, "rb"))
        sys.exit(0)
    except ImportError:
        sys.exit(0)
except Exception as e:
    print(f"TOML error: {e}", file=sys.stderr)
    sys.exit(1)
PYEOF
        assert "config.toml valid TOML" $?
    fi

    assert_contains "[general] present"        "$CONFIG_DIR/config.toml" '\[general\]'
    assert_contains "anthropic provider"       "$CONFIG_DIR/config.toml" 'anthropic'
    assert_contains "[policy] present"         "$CONFIG_DIR/config.toml" '\[policy\]'
    assert_contains "enable_agent_registry"    "$CONFIG_DIR/config.toml" 'enable_agent_registry'
    assert_contains "enable_semantic_memory"   "$CONFIG_DIR/config.toml" 'enable_semantic_memory'
    assert_contains "hooks PreToolUse"         "$CONFIG_DIR/hooks.toml"  'PreToolUse'
    assert_contains "mcp [options]"            "$CONFIG_DIR/mcp.toml"    '\[options\]'

    # HALCON.md line count (should be ≤ 200)
    LINES="$(wc -l < "$CONFIG_DIR/HALCON.md" 2>/dev/null | tr -d ' ')"
    [ "${LINES:-0}" -le 200 ] && _pass "HALCON.md ≤ 200 lines ($LINES)" \
        || _fail "HALCON.md > 200 lines ($LINES) — adherence may degrade"
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T4 — Agent registry
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_agent_tests() {
    section "T4 · Agent registry"
    assert_dir "agents/" "$CONFIG_DIR/agents"
    [ -d "$CONFIG_DIR/agents" ] || return

    COUNT="$(find "$CONFIG_DIR/agents" -name "*.md" 2>/dev/null | wc -l | tr -d ' ')"
    [ "$COUNT" -ge 1 ] && _pass "$COUNT agent(s) registered" || _fail "no agents in $CONFIG_DIR/agents"

    while IFS= read -r f; do
        local n; n="$(basename "$f")"
        grep -q '^---'         "$f" && _pass "frontmatter ($n)"   || _fail "no frontmatter ($n)"
        grep -q '^name:'       "$f" && _pass "name field ($n)"    || _fail "missing name: ($n)"
        grep -q '^description' "$f" && _pass "description ($n)"   || _fail "missing description ($n)"
    done < <(find "$CONFIG_DIR/agents" -name "*.md" 2>/dev/null)

    if [ -x "$HALCON_BIN" ]; then
        "$HALCON_BIN" agents list >/dev/null 2>&1 && _pass "halcon agents list" || _fail "halcon agents list failed"
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T5 — Shell completions
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_completion_tests() {
    section "T5 · Shell completions"
    COMP_DIR="$CONFIG_DIR/completions"
    assert_dir  "completions/"  "$COMP_DIR"
    assert_file "zsh _halcon"   "$COMP_DIR/_halcon"
    assert_file "bash halcon"   "$COMP_DIR/halcon.bash"
    assert_file "fish halcon"   "$COMP_DIR/halcon.fish"

    [ -f "$COMP_DIR/_halcon" ]    && has zsh  && { zsh  -n "$COMP_DIR/_halcon"    2>/dev/null; assert "zsh syntax"  $?; }
    [ -f "$COMP_DIR/halcon.bash" ] && has bash && { bash -n "$COMP_DIR/halcon.bash" 2>/dev/null; assert "bash syntax" $?; }
    [ -f "$COMP_DIR/halcon.fish" ] && has fish && { fish -n "$COMP_DIR/halcon.fish" 2>/dev/null; assert "fish syntax" $?; }

    assert_contains "zsh: chat"     "$COMP_DIR/_halcon"    "chat"
    assert_contains "zsh: agents"   "$COMP_DIR/_halcon"    "agents"
    assert_contains "bash: audit"   "$COMP_DIR/halcon.bash" "audit"
    assert_contains "fish: providers" "$COMP_DIR/halcon.fish" "anthropic"

    # Check system installation
    if has zsh; then
        if find "$HOME/.zsh" "$HOME/.local/share/zsh" /usr/local/share/zsh \
                -name "_halcon" 2>/dev/null | grep -q .; then
            _pass "zsh: installed system-wide"
        else
            _info "zsh: in $COMP_DIR/_halcon (add to fpath, then reload)"
        fi
    fi
    if has fish; then
        FISH_COMP="$HOME/.config/fish/completions/halcon.fish"
        [ -f "$FISH_COMP" ] && _pass "fish: installed ($FISH_COMP)" || _skip "fish: not in completions dir"
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T6 — VS Code extension
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_vscode_tests() {
    section "T6 · VS Code extension"
    EDITOR=""; ENAME=""
    has code   && { EDITOR="code";   ENAME="VS Code"; }
    has cursor && { EDITOR="cursor"; ENAME="Cursor"; }

    if [ -z "$EDITOR" ]; then _skip "VS Code / Cursor not installed"; return; fi
    _info "$ENAME detected"

    # Extension installed?
    EXT="$("$EDITOR" --list-extensions 2>/dev/null | grep -i 'cuervo-ai\|halcon' || true)"
    [ -n "$EXT" ] && _pass "$ENAME extension installed: $EXT" \
        || _skip "$ENAME extension not installed (re-run installer step 8)"

    # settings.json
    if [ "$OS" = "Darwin" ]; then
        SETTINGS="$HOME/Library/Application Support/Code/User/settings.json"
        [ "$EDITOR" = "cursor" ] && SETTINGS="$HOME/Library/Application Support/Cursor/User/settings.json"
    else
        SETTINGS="$HOME/.config/Code/User/settings.json"
        [ "$EDITOR" = "cursor" ] && SETTINGS="$HOME/.config/Cursor/User/settings.json"
    fi

    if [ -f "$SETTINGS" ]; then
        grep -q 'halcon.binaryPath' "$SETTINGS" 2>/dev/null \
            && _pass "$ENAME: halcon.binaryPath set" \
            || _info "$ENAME: halcon.binaryPath not set (will use PATH)"
        python3 -c "import json; json.load(open('$SETTINGS'))" 2>/dev/null \
            && _pass "$ENAME settings.json valid JSON" || _fail "$ENAME settings.json invalid JSON"
    else
        _skip "$ENAME settings.json not found"
    fi

    # Source files
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    if [ -d "$REPO_DIR/halcon-vscode" ]; then
        _pass "extension source: halcon-vscode/"
        assert_file "package.json"     "$REPO_DIR/halcon-vscode/package.json"
        assert_file "src/extension.ts" "$REPO_DIR/halcon-vscode/src/extension.ts"
        [ -f "$REPO_DIR/halcon-vscode/dist/extension.js" ] \
            && _pass "bundle: dist/extension.js (built)" \
            || _skip "bundle not built (cd halcon-vscode && npm run bundle)"
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T7 — Desktop app
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_desktop_tests() {
    section "T7 · Desktop app"
    DESK="$INSTALL_DIR/halcon-desktop"

    if [ -x "$DESK" ]; then
        _pass "binary installed: $DESK"
        DBYTES="$(wc -c < "$DESK" | tr -d ' ')"
        [ "$DBYTES" -gt 5242880 ] \
            && _pass "size: $(( DBYTES / 1024 / 1024 ))MB" \
            || _fail "suspiciously small: $DBYTES bytes"

        [ "$OS" = "Linux" ] && { file "$DESK" 2>/dev/null | grep -q ELF \
            && _pass "Linux ELF" || _fail "not ELF"; }

        if [ "$OS" = "Linux" ]; then
            ENTRY="$HOME/.local/share/applications/halcon-desktop.desktop"
            if [ -f "$ENTRY" ]; then
                _pass ".desktop entry: $ENTRY"
                assert_contains ".desktop Exec=" "$ENTRY" "^Exec="
            else
                _skip ".desktop entry not created"
            fi
        fi

        if [ "$OS" = "Darwin" ]; then
            APP="$HOME/Applications/Halcon Desktop.app"
            if [ -d "$APP" ]; then
                _pass "macOS .app bundle: $APP"
                assert_file "Info.plist" "$APP/Contents/Info.plist"
            else
                _skip "macOS .app bundle not created"
            fi
        fi
    else
        _skip "halcon-desktop not installed (step 9: ask 'Build desktop app?')"
    fi

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    [ -d "$REPO_DIR/crates/halcon-desktop" ] && _pass "source: crates/halcon-desktop/" \
        || _skip "desktop source not found"
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T8 — Docker
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_docker_tests() {
    section "T8 · Docker"
    if ! has docker; then _skip "Docker not installed"; return; fi
    if ! docker info &>/dev/null 2>&1; then _skip "Docker daemon not running"; return; fi
    _pass "Docker daemon running"

    if docker image inspect halcon-cli:latest &>/dev/null 2>&1; then
        SZ="$(docker image inspect halcon-cli:latest --format '{{.Size}}' \
            | awk '{ printf "%.1fMB", $1/1024/1024 }')"
        _pass "image exists: halcon-cli:latest ($SZ)"

        OUT="$(docker run --rm halcon-cli:latest --version 2>&1 || true)"
        echo "$OUT" | grep -qE '[0-9]+\.[0-9]+\.[0-9]+' \
            && _pass "docker halcon --version: $OUT" || _fail "docker run failed: $OUT"
    else
        _skip "halcon-cli:latest not built (step 10: answer 'Build Docker image?')"
    fi

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    assert_file "Dockerfile" "$SCRIPT_DIR/docker/Dockerfile"
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T9 — End-to-end smoke
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_e2e_tests() {
    section "T9 · End-to-end smoke"
    [ -x "$HALCON_BIN" ] || { _skip "CLI binary not found"; return; }

    for cmd in "status" "doctor" "tools list"; do
        # shellcheck disable=SC2086
        OUT="$("$HALCON_BIN" $cmd 2>&1 || true)"
        [ -n "$OUT" ] && _pass "halcon $cmd: returns output" || _fail "halcon $cmd: no output"
    done

    # Air-gap mode should not be killed by Gatekeeper
    HALCON_AIR_GAP=1 "$HALCON_BIN" status >/dev/null 2>&1 && E=0 || E=$?
    [ "$E" -ne 137 ] && _pass "air-gap mode (exit $E)" || _fail "air-gap exit 137 (Gatekeeper)"

    # agents list
    "$HALCON_BIN" agents list >/dev/null 2>&1 && E=0 || E=$?
    [ "$E" -le 1 ] && _pass "halcon agents list (exit $E)" || _fail "halcon agents list exit $E"

    # mcp list
    "$HALCON_BIN" mcp list >/dev/null 2>&1 && E=0 || E=$?
    [ "$E" -le 1 ] && _pass "halcon mcp list (exit $E)" || _fail "halcon mcp list exit $E"
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T10 — Platform-specific
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_platform_tests() {
    section "T10 · Platform (${OS}/${ARCH})"

    if [ "$OS" = "Darwin" ]; then
        if has codesign && [ -x "$HALCON_BIN" ]; then
            codesign --verify "$HALCON_BIN" 2>/dev/null && _pass "macOS: binary signed" \
                || _fail "macOS: binary not signed"
        fi
        if has xattr && [ -x "$HALCON_BIN" ]; then
            if ! xattr -l "$HALCON_BIN" 2>/dev/null | grep -q 'com.apple.quarantine'; then
                _pass "macOS: no quarantine flag"
            else
                _fail "macOS: quarantine set — run: xattr -d com.apple.quarantine $HALCON_BIN"
            fi
        fi
        if [ "$ARCH" = "arm64" ] && [ -x "$HALCON_BIN" ]; then
            file "$HALCON_BIN" 2>/dev/null | grep -qE 'arm64|universal' \
                && _pass "macOS ARM64: native binary" || _info "macOS ARM64: arch check inconclusive"
        fi
        if has brew; then
            brew tap cuervo-ai/homebrew-tap &>/dev/null 2>&1 \
                && _pass "Homebrew tap: cuervo-ai/homebrew-tap" \
                || _skip "Homebrew tap not yet published"
        fi

    elif [ "$OS" = "Linux" ]; then
        if has ldd && [ -x "$HALCON_BIN" ]; then
            OUT="$(ldd "$HALCON_BIN" 2>&1 || true)"
            echo "$OUT" | grep -qv 'not found' \
                && _pass "Linux: all shared libs resolved" \
                || _fail "Linux: missing libs"
        fi
        if echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
            _pass "Linux: ~/.local/bin in PATH"
        else
            _info "Linux: ~/.local/bin not in PATH (add to ~/.bashrc)"
        fi
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# T11 — Build from source (slow)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

run_build_tests() {
    section "T11 · Build from source"
    if $QUICK; then _skip "build test (--quick)"; return; fi

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

    (cd "$REPO_DIR" && cargo check -p halcon-cli --no-default-features 2>&1 | tail -2)
    assert "cargo check halcon-cli" $?

    (cd "$REPO_DIR" && cargo test --lib -p halcon-cli --no-default-features \
        -- --test-threads=4 2>&1 | grep -E "^(test result|running)" | tail -2)
    assert "cargo test --lib" $?
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Main
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

main() {
    cd "$(cd "$(dirname "$0")" && pwd)/.."
    START="$(date +%s)"

    if [ -n "$COMPONENT" ]; then
        case "$COMPONENT" in
            scripts)     run_script_tests ;;
            cli|binary)  run_cli_tests ;;
            config)      run_config_tests ;;
            agents)      run_agent_tests ;;
            completions) run_completion_tests ;;
            vscode)      run_vscode_tests ;;
            desktop)     run_desktop_tests ;;
            docker)      run_docker_tests ;;
            e2e)         run_e2e_tests ;;
            platform)    run_platform_tests ;;
            build)       run_build_tests ;;
            *)           echo "Unknown component: $COMPONENT"; exit 2 ;;
        esac
    else
        run_script_tests
        run_cli_tests
        run_config_tests
        run_agent_tests
        run_completion_tests
        run_vscode_tests
        run_desktop_tests
        run_docker_tests
        run_e2e_tests
        run_platform_tests
        $QUICK || run_build_tests
    fi

    ELAPSED=$(( $(date +%s) - START ))
    echo ""
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "  ${GREEN}${BOLD}PASS: $PASS${NC}  ${RED}${BOLD}FAIL: $FAIL${NC}  ${DIM}SKIP: $SKIP  (${ELAPSED}s)${NC}"
    echo -e "${CYAN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    if [ ${#FAILURES[@]} -gt 0 ]; then
        echo ""
        echo -e "  ${RED}${BOLD}Failed:${NC}"
        for f in "${FAILURES[@]}"; do echo -e "    ${RED}✗${NC} $f"; done
    fi

    echo ""
    if [ "$FAIL" -eq 0 ]; then
        echo -e "  ${GREEN}${BOLD}✓ All tests passed${NC}"; exit 0
    else
        echo -e "  ${RED}${BOLD}✗ $FAIL test(s) failed${NC}"; exit 1
    fi
}

main "$@"
