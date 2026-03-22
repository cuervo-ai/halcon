#!/usr/bin/env python3
"""
Halcon CLI — Terminal Screenshot Generator
Captures real CLI output and renders it as styled SVG/PNG images
following the Halcon fire/ember/gold design system.

Design system:
  Fire Primary:   #e85200
  Gold:           #f5a000
  Ember:          #c41400
  Background:     #070401
  Text Primary:   #f0e8d8
  Text Secondary: #c4b49a
  Code background:#0d0500
"""

import subprocess
import os
import sys
import re
from pathlib import Path

# ── deps ─────────────────────────────────────────────────────────────────────
try:
    from rich.console import Console
    from rich.text import Text
    from rich.panel import Panel
    from rich.theme import Theme
    from rich import print as rprint
    from rich.syntax import Syntax
except ImportError:
    print("Installing rich…")
    subprocess.run([sys.executable, "-m", "pip", "install", "rich"], check=True)
    from rich.console import Console
    from rich.text import Text
    from rich.panel import Panel
    from rich.theme import Theme
    from rich.syntax import Syntax

try:
    import cairosvg
    # Test that the native cairo library is actually loadable
    cairosvg.svg2png(bytestring=b'<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>', write_to="/dev/null")
    HAS_CAIRO = True
except Exception:
    HAS_CAIRO = False

# ── paths ─────────────────────────────────────────────────────────────────────
REPO_ROOT   = Path(__file__).resolve().parent.parent
OUT_DIR     = REPO_ROOT / "img" / "screenshots"
HALCON_BIN  = Path.home() / ".local" / "bin" / "halcon"
OUT_DIR.mkdir(parents=True, exist_ok=True)

# ── design system ─────────────────────────────────────────────────────────────
FIRE    = "#e85200"
GOLD    = "#f5a000"
EMBER   = "#c41400"
BG      = "#070401"
BG_CODE = "#0d0500"
TEXT    = "#f0e8d8"
MUTED   = "#c4b49a"
GREEN   = "#57cb60"
BLUE    = "#74deff"
YELLOW  = "#fdd506"
RED     = "#ee343b"

HALCON_THEME = Theme({
    "fire":      f"bold {FIRE}",
    "gold":      f"bold {GOLD}",
    "ember":     EMBER,
    "text":      TEXT,
    "muted":     MUTED,
    "ok":        GREEN,
    "warn":      YELLOW,
    "error":     RED,
    "cyan":      BLUE,
    "prompt":    f"bold {FIRE}",
    "cmd":       f"bold {TEXT}",
})

# ── ANSI → Rich colour map ─────────────────────────────────────────────────────
# Maps the specific ANSI RGB codes halcon emits to named styles
ANSI_RGB_MAP = {
    "252;138;96":  FIRE,    # fire orange (headings)
    "116;222;255": BLUE,    # cyan (labels, tool names)
    "87;203;96":   GREEN,   # green [OK]
    "253;213;6":   YELLOW,  # yellow [DEGRADED]
    "238;52;59":   RED,     # red [D!]
    "164;148;143": MUTED,   # muted (details)
    "124;110;105": "#7c6e69", # border colour
    "245;160;0":   GOLD,    # gold
    "57;203;96":   GREEN,
    "74;222;255":  BLUE,
}

def strip_ansi(text: str) -> str:
    """Remove all ANSI escape codes."""
    ansi_escape = re.compile(r'\x1b\[[0-9;]*[mK]')
    return ansi_escape.sub('', text)

def ansi_to_rich(text: str) -> Text:
    """
    Convert ANSI-coloured terminal output to a rich Text object.
    Handles ESC[38;2;R;G;Bm (24-bit colour) and bold/reset codes.
    """
    result   = Text()
    segments = re.split(r'(\x1b\[[0-9;]*m)', text)
    current_style = ""
    bold = False

    for seg in segments:
        if seg.startswith('\x1b['):
            codes = seg[2:-1]  # strip ESC[ and m
            if codes in ('0', ''):
                current_style = ""
                bold = False
            elif codes == '1':
                bold = True
            else:
                # 24-bit colour: 38;2;R;G;B
                m = re.match(r'38;2;(\d+);(\d+);(\d+)', codes)
                if m:
                    r, g, b = m.group(1), m.group(2), m.group(3)
                    key = f"{r};{g};{b}"
                    hex_color = ANSI_RGB_MAP.get(key, f"#{int(r):02x}{int(g):02x}{int(b):02x}")
                    current_style = hex_color
        else:
            if seg:
                style = current_style
                if bold and style:
                    style = f"bold {style}"
                elif bold:
                    style = "bold"
                result.append(seg, style=style or TEXT)

    return result

def run_command(cmd: str, env_extra: dict = None, timeout: int = 15) -> str:
    """Run a shell command and return its combined stdout+stderr."""
    env = os.environ.copy()
    env["TERM"] = "xterm-256color"
    env["COLORTERM"] = "truecolor"
    env["NO_COLOR"] = ""          # let halcon emit colour
    if env_extra:
        env.update(env_extra)

    try:
        result = subprocess.run(
            cmd,
            shell=True,
            capture_output=True,
            timeout=timeout,
            env=env,
        )
        out = result.stdout.decode("utf-8", errors="replace")
        err = result.stderr.decode("utf-8", errors="replace")
        combined = out + err
        return combined.rstrip()
    except subprocess.TimeoutExpired:
        return f"[command timed out after {timeout}s]"
    except Exception as e:
        return f"[error running command: {e}]"

# ── SVG template ──────────────────────────────────────────────────────────────

def build_svg(title: str, command: str, lines: list[str], width: int = 860) -> str:
    """
    Render a terminal screenshot as SVG with the Halcon design system.
    lines: list of plain-text strings (already ANSI-stripped for sizing).
    """
    FONT_SIZE   = 13
    LINE_HEIGHT = 20
    PADDING     = 20
    HEADER_H    = 40
    CTRL_R      = 6
    CTRL_Y      = 22

    visible_lines = [l for l in lines if l is not None]
    content_h = max(len(visible_lines) * LINE_HEIGHT + PADDING * 2, 60)
    total_h   = HEADER_H + content_h + PADDING

    def esc(s):
        return (s.replace("&", "&amp;")
                 .replace("<", "&lt;")
                 .replace(">", "&gt;")
                 .replace('"', "&quot;"))

    def color_span(text: str) -> str:
        """Convert ANSI-coloured text to SVG tspan elements."""
        parts   = re.split(r'(\x1b\[[0-9;]*m)', text)
        out     = []
        cur_col = TEXT
        bold    = False

        for part in parts:
            if part.startswith('\x1b['):
                codes = part[2:-1]
                if codes in ('0', ''):
                    cur_col = TEXT
                    bold    = False
                elif codes == '1':
                    bold = True
                else:
                    m = re.match(r'38;2;(\d+);(\d+);(\d+)', codes)
                    if m:
                        r, g, b = int(m.group(1)), int(m.group(2)), int(m.group(3))
                        key = f"{r};{g};{b}"
                        cur_col = ANSI_RGB_MAP.get(key, f"#{r:02x}{g:02x}{b:02x}")
            else:
                if part:
                    weight = 'font-weight="bold" ' if bold else ''
                    out.append(
                        f'<tspan fill="{cur_col}" {weight}>{esc(part)}</tspan>'
                    )
        return "".join(out)

    rows_svg = []
    y = HEADER_H + PADDING + FONT_SIZE
    for line in lines:
        row_svg = color_span(line) if line else ""
        rows_svg.append(
            f'<text x="{PADDING}" y="{y}" '
            f'font-family="\'JetBrains Mono\',\'Fira Code\',\'Cascadia Code\',monospace" '
            f'font-size="{FONT_SIZE}px" xml:space="preserve">'
            f'{row_svg}</text>'
        )
        y += LINE_HEIGHT

    rows_block = "\n    ".join(rows_svg)

    svg = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{total_h}"
     viewBox="0 0 {width} {total_h}" role="img"
     aria-label="Halcon CLI — {esc(title)}">
  <defs>
    <linearGradient id="headerGrad" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%"   stop-color="{EMBER}" stop-opacity="0.9"/>
      <stop offset="100%" stop-color="{FIRE}"  stop-opacity="0.7"/>
    </linearGradient>
    <filter id="glow">
      <feGaussianBlur stdDeviation="2" result="blur"/>
      <feComposite in="SourceGraphic" in2="blur" operator="over"/>
    </filter>
  </defs>

  <!-- Window frame -->
  <rect width="{width}" height="{total_h}" rx="10" ry="10"
        fill="{BG}" stroke="{FIRE}" stroke-width="1.5" stroke-opacity="0.6"/>

  <!-- Title bar -->
  <rect width="{width}" height="{HEADER_H}" rx="10" ry="10" fill="url(#headerGrad)" fill-opacity="0.25"/>
  <rect y="10" width="{width}" height="{HEADER_H - 10}" fill="url(#headerGrad)" fill-opacity="0.25"/>

  <!-- Window controls -->
  <circle cx="20"  cy="{CTRL_Y}" r="{CTRL_R}" fill="{RED}"    filter="url(#glow)"/>
  <circle cx="38"  cy="{CTRL_Y}" r="{CTRL_R}" fill="{YELLOW}" filter="url(#glow)"/>
  <circle cx="56"  cy="{CTRL_Y}" r="{CTRL_R}" fill="{GREEN}"  filter="url(#glow)"/>

  <!-- Title -->
  <text x="{width // 2}" y="26" text-anchor="middle"
        font-family="'Rajdhani','Montserrat',sans-serif" font-size="13px"
        font-weight="600" fill="{TEXT}" opacity="0.85">{esc(title)}</text>

  <!-- Separator line -->
  <line x1="0" y1="{HEADER_H}" x2="{width}" y2="{HEADER_H}"
        stroke="{FIRE}" stroke-width="0.8" stroke-opacity="0.4"/>

  <!-- Prompt line -->
  <text x="{PADDING}" y="{HEADER_H + PADDING - 4}"
        font-family="'JetBrains Mono','Fira Code',monospace" font-size="{FONT_SIZE}px">
    <tspan fill="{MUTED}">~ </tspan><tspan fill="{FIRE}" font-weight="bold">❯ </tspan><tspan fill="{TEXT}">{esc(command)}</tspan>
  </text>

  <!-- Output -->
  {rows_block}

  <!-- Bottom glow accent -->
  <rect y="{total_h - 3}" width="{width}" height="3" rx="0"
        fill="url(#headerGrad)" opacity="0.5"/>
</svg>'''
    return svg


def save_screenshot(slug: str, title: str, command: str, raw_output: str,
                    max_lines: int = 55, width: int = 860) -> Path:
    """Save a terminal screenshot as SVG (and optionally PNG)."""
    lines     = raw_output.split("\n")[:max_lines]
    svg_path  = OUT_DIR / f"screenshot_{slug}.svg"
    png_path  = OUT_DIR / f"screenshot_{slug}.png"

    svg_content = build_svg(title, command, lines, width=width)
    svg_path.write_text(svg_content, encoding="utf-8")

    if HAS_CAIRO:
        try:
            cairosvg.svg2png(
                url=str(svg_path),
                write_to=str(png_path),
                output_width=width * 2,    # 2× for retina
            )
            print(f"  [PNG] {png_path.name}")
        except Exception as e:
            print(f"  [WARN] PNG conversion failed: {e}")

    print(f"  [SVG] {svg_path.name}")
    return svg_path


# ── Hero composite ─────────────────────────────────────────────────────────────

def build_hero_svg(panels: list[dict]) -> str:
    """
    Build a composite hero SVG containing multiple stacked panels.
    panels: list of { title, command, output }
    """
    PANEL_W     = 820
    PANEL_GAP   = 16
    PAD         = 32
    FONT_SIZE   = 11
    LINE_HEIGHT = 17
    HEADER_H    = 36

    def panel_height(output: str, max_lines=20) -> int:
        lines = output.split("\n")[:max_lines]
        return HEADER_H + len(lines) * LINE_HEIGHT + PAD

    total_h = PAD
    for p in panels:
        total_h += panel_height(p["output"]) + PANEL_GAP
    total_h += PAD
    total_w = PANEL_W + PAD * 2

    def esc(s):
        return (s.replace("&", "&amp;")
                 .replace("<", "&lt;")
                 .replace(">", "&gt;")
                 .replace('"', "&quot;"))

    def color_span(text: str) -> str:
        parts   = re.split(r'(\x1b\[[0-9;]*m)', text)
        out     = []
        cur_col = TEXT
        bold    = False
        for part in parts:
            if part.startswith('\x1b['):
                codes = part[2:-1]
                if codes in ('0', ''):
                    cur_col = TEXT; bold = False
                elif codes == '1':
                    bold = True
                else:
                    m = re.match(r'38;2;(\d+);(\d+);(\d+)', codes)
                    if m:
                        r, g, b = int(m.group(1)), int(m.group(2)), int(m.group(3))
                        key = f"{r};{g};{b}"
                        cur_col = ANSI_RGB_MAP.get(key, f"#{r:02x}{g:02x}{b:02x}")
            else:
                if part:
                    w = 'font-weight="bold" ' if bold else ''
                    out.append(f'<tspan fill="{cur_col}" {w}>{esc(part)}</tspan>')
        return "".join(out)

    panels_svg = []
    y_offset   = PAD

    for p in panels:
        ph     = panel_height(p["output"])
        px     = PAD
        py     = y_offset
        lines  = p["output"].split("\n")[:20]

        rows = []
        ty   = py + HEADER_H + 14
        for line in lines:
            rows.append(
                f'<text x="{px + 12}" y="{ty}" '
                f'font-family="\'JetBrains Mono\',monospace" font-size="{FONT_SIZE}px" '
                f'xml:space="preserve">{color_span(line)}</text>'
            )
            ty += LINE_HEIGHT

        panels_svg.append(f'''
  <!-- Panel: {esc(p["title"])} -->
  <rect x="{px}" y="{py}" width="{PANEL_W}" height="{ph}"
        rx="8" fill="{BG_CODE}" stroke="{FIRE}" stroke-width="1" stroke-opacity="0.5"/>
  <rect x="{px}" y="{py}" width="{PANEL_W}" height="{HEADER_H}"
        rx="8" fill="{FIRE}" fill-opacity="0.12"/>
  <rect x="{px}" y="{py + 10}" width="{PANEL_W}" height="{HEADER_H - 10}"
        fill="{FIRE}" fill-opacity="0.12"/>
  <circle cx="{px + 14}" cy="{py + 17}" r="5" fill="{RED}"/>
  <circle cx="{px + 28}" cy="{py + 17}" r="5" fill="{YELLOW}"/>
  <circle cx="{px + 42}" cy="{py + 17}" r="5" fill="{GREEN}"/>
  <text x="{px + PANEL_W // 2}" y="{py + 22}" text-anchor="middle"
        font-family="'Rajdhani',sans-serif" font-size="12px" font-weight="600"
        fill="{TEXT}" opacity="0.9">{esc(p["title"])}</text>
  <line x1="{px}" y1="{py + HEADER_H}" x2="{px + PANEL_W}" y2="{py + HEADER_H}"
        stroke="{FIRE}" stroke-width="0.6" stroke-opacity="0.4"/>
  <text x="{px + 12}" y="{py + HEADER_H + 10}"
        font-family="'JetBrains Mono',monospace" font-size="{FONT_SIZE}px">
    <tspan fill="{MUTED}">~ </tspan><tspan fill="{FIRE}" font-weight="bold">❯ </tspan><tspan fill="{TEXT}">{esc(p["command"])}</tspan>
  </text>
  {"".join(rows)}''')

        y_offset += ph + PANEL_GAP

    svg = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}"
     viewBox="0 0 {total_w} {total_h}" role="img"
     aria-label="Halcon CLI — Command Showcase">

  <!-- Background -->
  <rect width="{total_w}" height="{total_h}" fill="{BG}" rx="12"/>

  <!-- Header branding -->
  <text x="{total_w // 2}" y="26" text-anchor="middle"
        font-family="'Rajdhani','Montserrat',sans-serif" font-size="18px"
        font-weight="700" fill="{FIRE}">HALCON CLI</text>
  <text x="{total_w // 2}" y="42" text-anchor="middle"
        font-family="'Rajdhani',sans-serif" font-size="11px"
        fill="{MUTED}">AI-native terminal agent · v0.3.0</text>

  {"".join(panels_svg)}

  <!-- Footer -->
  <text x="{total_w // 2}" y="{total_h - 10}" text-anchor="middle"
        font-family="'Rajdhani',sans-serif" font-size="10px"
        fill="{MUTED}" opacity="0.6">halcon.cuervo.cloud</text>
</svg>'''
    return svg


# ── main ──────────────────────────────────────────────────────────────────────

SCREENSHOTS = [
    {
        "slug":    "version",
        "title":   "Halcon CLI — Version",
        "command": "halcon --version",
        "cmd":     f"{HALCON_BIN} --version",
        "width":   680,
    },
    {
        "slug":    "help",
        "title":   "Halcon CLI — Commands",
        "command": "halcon --help",
        "cmd":     f"{HALCON_BIN} --help",
        "width":   860,
    },
    {
        "slug":    "status",
        "title":   "Halcon CLI — Status",
        "command": "halcon status",
        "cmd":     f"{HALCON_BIN} status",
        "width":   700,
    },
    {
        "slug":    "auth_status",
        "title":   "Halcon CLI — Auth Status",
        "command": "halcon auth status",
        "cmd":     f"{HALCON_BIN} auth status",
        "width":   700,
    },
    {
        "slug":    "doctor",
        "title":   "Halcon CLI — Runtime Diagnostics",
        "command": "halcon doctor",
        "cmd":     f"{HALCON_BIN} doctor",
        "width":   900,
        "max_lines": 55,
    },
    {
        "slug":    "tools_list",
        "title":   "Halcon CLI — Tool Registry (63 tools)",
        "command": "halcon tools list",
        "cmd":     f"{HALCON_BIN} tools list 2>&1 | head -45",
        "width":   900,
        "max_lines": 48,
    },
    {
        "slug":    "agents_list",
        "title":   "Halcon CLI — Agent Registry",
        "command": "halcon agents list",
        "cmd":     f"{HALCON_BIN} agents list",
        "width":   740,
    },
    {
        "slug":    "mcp_list",
        "title":   "Halcon CLI — MCP Servers",
        "command": "halcon mcp list",
        "cmd":     f"{HALCON_BIN} mcp list",
        "width":   680,
    },
]

HERO_PANELS = [
    {
        "title":   "halcon status",
        "command": "halcon status",
        "key":     "status",
    },
    {
        "title":   "halcon doctor",
        "command": "halcon doctor",
        "key":     "doctor",
    },
    {
        "title":   "halcon tools list",
        "command": "halcon tools list",
        "key":     "tools_list",
    },
]


def main():
    print(f"\nHalcon CLI — Screenshot Generator")
    print(f"  Binary : {HALCON_BIN}")
    print(f"  Output : {OUT_DIR}")
    print(f"  Cairo  : {'yes (PNG enabled)' if HAS_CAIRO else 'no (SVG only)'}")
    print()

    outputs: dict[str, str] = {}

    # ── 1. capture all command outputs ────────────────────────────────────────
    print("── Capturing command output ──────────────────────────────")
    for s in SCREENSHOTS:
        print(f"  $ {s['command']}")
        raw = run_command(s["cmd"])
        outputs[s["slug"]] = raw

    # ── 2. render individual screenshots ──────────────────────────────────────
    print("\n── Rendering individual screenshots ──────────────────────")
    for s in SCREENSHOTS:
        raw   = outputs[s["slug"]]
        lines = raw.split("\n")[: s.get("max_lines", 55)]
        w     = s.get("width", 860)

        svg   = build_svg(s["title"], s["command"], lines, width=w)
        sp    = OUT_DIR / f"screenshot_{s['slug']}.svg"
        sp.write_text(svg, encoding="utf-8")
        print(f"  [SVG] {sp.name}")

        if HAS_CAIRO:
            pp = OUT_DIR / f"screenshot_{s['slug']}.png"
            try:
                cairosvg.svg2png(url=str(sp), write_to=str(pp), output_width=w * 2)
                print(f"  [PNG] {pp.name}")
            except Exception as e:
                print(f"  [WARN] PNG failed: {e}")

    # ── 3. render hero composite ──────────────────────────────────────────────
    print("\n── Rendering hero composite ──────────────────────────────")
    hero_panels = []
    for p in HERO_PANELS:
        hero_panels.append({
            "title":   p["title"],
            "command": p["command"],
            "output":  outputs.get(p["key"], ""),
        })

    hero_svg  = build_hero_svg(hero_panels)
    hero_path = OUT_DIR / "screenshot_hero.svg"
    hero_path.write_text(hero_svg, encoding="utf-8")
    print(f"  [SVG] {hero_path.name}")

    if HAS_CAIRO:
        hero_png = OUT_DIR / "screenshot_hero.png"
        try:
            cairosvg.svg2png(url=str(hero_path), write_to=str(hero_png), output_width=1800)
            print(f"  [PNG] {hero_png.name}")
        except Exception as e:
            print(f"  [WARN] Hero PNG failed: {e}")

    # ── 4. summary ────────────────────────────────────────────────────────────
    generated = list(OUT_DIR.glob("screenshot_*"))
    print(f"\n── Done ──────────────────────────────────────────────────")
    print(f"  Generated {len(generated)} files in {OUT_DIR}")
    for f in sorted(generated):
        size_kb = f.stat().st_size // 1024
        print(f"    {f.name}  ({size_kb} KB)")


if __name__ == "__main__":
    main()
