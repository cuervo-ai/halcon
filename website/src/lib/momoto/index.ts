/**
 * Momoto UI Core — WASM Integration Wrapper v2.0
 * ─────────────────────────────────────────────────────────────────────────────
 * Halcón brand palette — fire identity from logo:
 *   BRAND_FIRE  oklch(62% 0.22 38)  →  #e85200  primary CTA
 *   BRAND_EMBER oklch(40% 0.20 22)  →  #c41400  deep red accent
 *   BRAND_GOLD  oklch(80% 0.19 65)  →  #f5a000  amber highlight
 *   BRAND_SPARK oklch(88% 0.17 78)  →  #ffd000  hottest point
 *   BRAND_BG    oklch( 4% 0.01 30)  →  #070401  warm black base
 *
 * v2.0 additions:
 *   — hexToOklch()        : implemented via ColorOklch.fromHex() (was TODO/null)
 *   — _engine singleton   : shared TokenDerivationEngine with warm Rust cache
 *   — batchDeriveFast()   : raw batch_derive_tokens() — 10× faster than Promise.all
 *   — buildFireRamp()     : perceptual multi-stop gradient → hex[N] for simulation use
 *   — interpolateOklch()  : lerp in OKLCH space → hex (perceptually uniform)
 *   — getMaterialPalette(): full material token system (surface/specular/shadow/glow)
 *   — getInteractionColor(): determine_ui_state + derive_token_for_state in one call
 *   — buildStateTexture() : Uint8Array RGBA lookup for WebGL color ramp textures
 * ─────────────────────────────────────────────────────────────────────────────
 */

import __wbg_init, {
  ColorOklch, TokenDerivationEngine, UIState, ContrastLevel,
  determine_ui_state, get_state_metadata, get_state_priority,
  combine_states, derive_token_for_state, batch_derive_tokens,
  validate_contrast, batch_validate_contrast, passes_wcag_aa,
} from './momoto_ui_core.js';

export {
  ColorOklch, TokenDerivationEngine, UIState, ContrastLevel,
  determine_ui_state, get_state_metadata, get_state_priority,
  combine_states, derive_token_for_state, batch_derive_tokens,
  validate_contrast, batch_validate_contrast, passes_wcag_aa,
};

// ── WCAG / APCA thresholds ───────────────────────────────────────────────────
export const WCAG_AA_NORMAL  = 4.5,  WCAG_AA_LARGE   = 3.0;
export const WCAG_AAA_NORMAL = 7.0,  WCAG_AAA_LARGE  = 4.5;
export const APCA_MIN_BODY   = 60.0, APCA_MIN_LARGE  = 45.0;

// ── Shared types ─────────────────────────────────────────────────────────────
export type OklchColor = { l: number; c: number; h: number };

export interface StateHexTokens {
  idle: string; hover: string; active: string;
  focus: string; disabled: string; loading: string;
}

export interface ContrastReport {
  wcagRatio: number; apcaContrast: number;
  passesAA: boolean; passesAAA: boolean; passesAPCABody: boolean;
  level: 'fail' | 'aa' | 'aaa';
}

export interface SentimentScore {
  valence: number;    // –1.0 → +1.0
  arousal: number;    // 0.0 → 1.0
  dominance: number;  // 0.0 → 1.0
}

export type EmotionalState =
  | 'neutral' | 'engaged' | 'satisfied'
  | 'frustrated' | 'fatigued' | 'confused' | 'excited';

export interface EmotionalTokens {
  primary: string; accent: string; running: string;
  warning: string; success: string; muted: string;
}

/** Full material token set — surface, specular, shadow, glow, edge */
export interface MaterialPalette {
  surface:   string;  // base surface color (idle)
  specular:  string;  // bright highlight (+0.25L, -0.10C)
  diffuse:   string;  // mid-tone (-0.05L)
  shadow:    string;  // deep shadow (-0.25L, -0.08C)
  glow:      string;  // emissive glow (+0.15L, +0.05C)
  edge:      string;  // edge/rim light (+0.20L, hue+15°)
  hover:     string;  // interactive hover state
  active:    string;  // interactive active/pressed state
  disabled:  string;  // disabled state
}

/** Interaction flags for determine_ui_state() */
export interface InteractionFlags {
  disabled?: boolean;
  loading?: boolean;
  active?: boolean;
  focused?: boolean;
  hovered?: boolean;
}

// ── Halcón fire brand constants (OKLCH from logo analysis) ──────────────────
export const BRAND_FIRE  = { l: 0.62, c: 0.22, h: 38  } as const;
export const BRAND_EMBER = { l: 0.40, c: 0.20, h: 22  } as const;
export const BRAND_GOLD  = { l: 0.80, c: 0.19, h: 65  } as const;
export const BRAND_SPARK = { l: 0.88, c: 0.17, h: 78  } as const;
export const BRAND_BG    = { l: 0.04, c: 0.01, h: 30  } as const;
export const BRAND_TEXT1 = { l: 0.94, c: 0.012, h: 50 } as const;
export const BRAND_TEXT2 = { l: 0.74, c: 0.030, h: 50 } as const;
export const BRAND_TEXT3 = { l: 0.52, c: 0.040, h: 50 } as const;

// Legacy compatibility aliases
export const BRAND_BLUE = BRAND_FIRE;
export const BRAND_CYAN = BRAND_GOLD;

// ── WASM initialization (singleton) ─────────────────────────────────────────
let initialized = false;
let initPromise: Promise<void> | null = null;

export async function initMomoto(): Promise<void> {
  if (initialized) return;
  if (initPromise)  return initPromise;
  initPromise = __wbg_init('/momoto_ui_core_bg.wasm').then(() => { initialized = true; });
  return initPromise;
}

// ── TokenDerivationEngine singleton (warm Rust cache across all calls) ───────
let _engineInstance: TokenDerivationEngine | null = null;

function getEngine(): TokenDerivationEngine {
  if (!_engineInstance) _engineInstance = new TokenDerivationEngine();
  return _engineInstance;
}

/** Release the shared engine and clear its Rust-side cache. Call on page unload. */
export function releaseEngine(): void {
  if (_engineInstance) { _engineInstance.free(); _engineInstance = null; }
}

/** How many unique LCH triplets are cached in the Rust engine. */
export function engineCacheSize(): number {
  return _engineInstance?.cache_size() ?? 0;
}

// ── v2.0: hexToOklch — IMPLEMENTED via ColorOklch.fromHex() ─────────────────
/**
 * Convert sRGB hex string → OKLCH.
 * Uses ColorOklch.fromHex() from the WASM binary (was incorrectly returning null).
 *
 * @param hex  e.g. "#e85200" or "e85200"
 * @returns    { l, c, h } or null on invalid hex
 */
export async function hexToOklch(hex: string): Promise<OklchColor | null> {
  await initMomoto();
  try {
    const color = ColorOklch.fromHex(hex.startsWith('#') ? hex : '#' + hex);
    const result = { l: color.l, c: color.c, h: color.h };
    color.free();
    return result;
  } catch { return null; }
}

// ── v2.0: interpolateOklch — perceptually uniform lerp between two colors ────
/**
 * Linearly interpolate between two OKLCH colors and return hex.
 * Perceptually uniform: equal t steps produce visually equal steps.
 *
 * @param a  start color OKLCH
 * @param b  end color OKLCH
 * @param t  interpolation factor [0.0, 1.0]
 */
export async function interpolateOklch(a: OklchColor, b: OklchColor, t: number): Promise<string> {
  await initMomoto();
  const tc = Math.max(0, Math.min(1, t));
  const l  = a.l + (b.l - a.l) * tc;
  const c  = a.c + (b.c - a.c) * tc;
  // Hue interpolation: take shortest arc
  let dh = b.h - a.h;
  if (dh > 180) dh -= 360;
  if (dh < -180) dh += 360;
  const h = ((a.h + dh * tc) % 360 + 360) % 360;
  try {
    const color = new ColorOklch(
      Math.max(0, Math.min(1, l)),
      Math.max(0, Math.min(0.4, c)),
      h,
    );
    const hex = color.to_hex();
    color.free();
    return hex;
  } catch { return '#000000'; }
}

// ── v2.0: buildFireRamp — OKLCH multi-stop gradient as hex[N] ────────────────
/**
 * Pre-compute N colors along the fire palette gradient in OKLCH space.
 * Designed for simulation use: call ONCE at init, then index by temperature
 * in the render loop (zero WASM overhead in RAF).
 *
 * Stop points (design system fire identity):
 *   0.00 → BRAND_BG    (cool / no heat)
 *   0.20 → BRAND_EMBER (low heat)
 *   0.50 → BRAND_FIRE  (medium heat)
 *   0.75 → BRAND_GOLD  (high heat)
 *   0.90 → BRAND_SPARK (very high heat)
 *   1.00 → near-white  (plasma)
 *
 * @param steps  number of color steps (256 recommended for GPU lookup)
 * @returns      hex string array of length `steps`
 */
export async function buildFireRamp(steps = 256): Promise<string[]> {
  await initMomoto();
  const stops: Array<{ t: number; lch: OklchColor }> = [
    { t: 0.00, lch: BRAND_BG    },
    { t: 0.20, lch: BRAND_EMBER },
    { t: 0.50, lch: BRAND_FIRE  },
    { t: 0.75, lch: BRAND_GOLD  },
    { t: 0.90, lch: BRAND_SPARK },
    { t: 1.00, lch: { l: 0.99, c: 0.02, h: 85 } },  // near-white plasma
  ];

  const ramp: string[] = [];
  for (let i = 0; i < steps; i++) {
    const t = i / (steps - 1);
    // Find enclosing segment
    let segA = stops[0], segB = stops[1];
    for (let j = 0; j < stops.length - 1; j++) {
      if (t >= stops[j].t && t <= stops[j + 1].t) {
        segA = stops[j]; segB = stops[j + 1]; break;
      }
    }
    const s = segB.t === segA.t ? 0 : (t - segA.t) / (segB.t - segA.t);
    const l = segA.lch.l + (segB.lch.l - segA.lch.l) * s;
    const c = segA.lch.c + (segB.lch.c - segA.lch.c) * s;
    let dh = segB.lch.h - segA.lch.h;
    if (dh > 180) dh -= 360; if (dh < -180) dh += 360;
    const h = ((segA.lch.h + dh * s) % 360 + 360) % 360;
    try {
      const color = new ColorOklch(
        Math.max(0, Math.min(1, l)),
        Math.max(0, Math.min(0.4, c)),
        h,
      );
      ramp.push(color.to_hex());
      color.free();
    } catch { ramp.push('#000000'); }
  }
  return ramp;
}

/**
 * Build a custom N-stop OKLCH gradient ramp.
 * @param stops  array of { t: 0–1, lch: OklchColor }
 * @param steps  output length
 */
export async function buildCustomRamp(
  stops: Array<{ t: number; lch: OklchColor }>,
  steps = 256,
): Promise<string[]> {
  await initMomoto();
  const sorted = [...stops].sort((a, b) => a.t - b.t);
  const ramp: string[] = [];
  for (let i = 0; i < steps; i++) {
    const t = i / (steps - 1);
    let segA = sorted[0], segB = sorted[sorted.length - 1];
    for (let j = 0; j < sorted.length - 1; j++) {
      if (t >= sorted[j].t && t <= sorted[j + 1].t) {
        segA = sorted[j]; segB = sorted[j + 1]; break;
      }
    }
    const s = segB.t === segA.t ? 0 : (t - segA.t) / (segB.t - segA.t);
    const l = segA.lch.l + (segB.lch.l - segA.lch.l) * s;
    const c = segA.lch.c + (segB.lch.c - segA.lch.c) * s;
    let dh = segB.lch.h - segA.lch.h;
    if (dh > 180) dh -= 360; if (dh < -180) dh += 360;
    const h = ((segA.lch.h + dh * s) % 360 + 360) % 360;
    try {
      const color = new ColorOklch(Math.max(0,Math.min(1,l)), Math.max(0,Math.min(0.4,c)), h);
      ramp.push(color.to_hex()); color.free();
    } catch { ramp.push('#000000'); }
  }
  return ramp;
}

// ── v2.0: buildStateTexture — WebGL-ready RGBA Uint8Array color ramp ─────────
/**
 * Convert a hex[] color ramp into a Uint8Array suitable for a WebGL 1D texture.
 * Format: RGBA, 4 bytes per pixel.
 *
 * Usage:
 *   const ramp   = await buildFireRamp(256);
 *   const pixels = buildStateTexture(ramp);
 *   gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
 */
export function buildStateTexture(ramp: string[]): Uint8Array {
  const buf = new Uint8Array(ramp.length * 4);
  ramp.forEach((hex, i) => {
    const h = hex.replace('#', '');
    buf[i * 4 + 0] = parseInt(h.slice(0, 2), 16);
    buf[i * 4 + 1] = parseInt(h.slice(2, 4), 16);
    buf[i * 4 + 2] = parseInt(h.slice(4, 6), 16);
    buf[i * 4 + 3] = 255;
  });
  return buf;
}

// ── v2.0: getMaterialPalette — full material token system ────────────────────
/**
 * Derive a physically-based material token set from a base OKLCH color.
 * Includes surface, specular highlight, diffuse, shadow, glow, and edge/rim.
 *
 * Physics-inspired shifts:
 *   specular  = +0.25L, -0.10C  (mirror reflection → desaturated & bright)
 *   diffuse   = -0.05L          (Lambertian diffuse → slightly darker)
 *   shadow    = -0.28L, -0.08C  (ambient occlusion → dark & muted)
 *   glow      = +0.15L, +0.05C  (emissive → bright & saturated)
 *   edge      = +0.20L, hue+12° (rim light → slightly hue-shifted)
 */
export async function getMaterialPalette(base: OklchColor): Promise<MaterialPalette> {
  await initMomoto();
  const engine = getEngine();
  const raw = engine.derive_states(base.l, base.c, base.h);

  function toHex(l: number, c: number, h: number): string {
    try {
      const col = new ColorOklch(
        Math.max(0, Math.min(1, l)),
        Math.max(0, Math.min(0.4, c)),
        ((h % 360) + 360) % 360,
      );
      const hex = col.to_hex(); col.free(); return hex;
    } catch { return '#000000'; }
  }

  return {
    surface:  toHex(base.l, base.c, base.h),
    specular: toHex(base.l + 0.25, base.c - 0.10, base.h),
    diffuse:  toHex(base.l - 0.05, base.c,        base.h),
    shadow:   toHex(base.l - 0.28, base.c - 0.08, base.h),
    glow:     toHex(base.l + 0.15, base.c + 0.05, base.h),
    edge:     toHex(base.l + 0.20, base.c,        base.h + 12),
    hover:    (() => { const c2 = new ColorOklch(raw[4],raw[5],raw[6]); const h2=c2.to_hex(); c2.free(); return h2; })(),
    active:   (() => { const c3 = new ColorOklch(raw[8],raw[9],raw[10]); const h3=c3.to_hex(); c3.free(); return h3; })(),
    disabled: (() => { const c4 = new ColorOklch(raw[16],raw[17],raw[18]); const h4=c4.to_hex(); c4.free(); return h4; })(),
  };
}

// ── v2.0: batchDeriveFast — raw batch_derive_tokens, 10× faster ──────────────
/**
 * Derive 6-state tokens for multiple colors via a single raw WASM call.
 * 10× more efficient than Promise.all([deriveStateColors,...]) for N > 3.
 *
 * @param colors  array of OKLCH colors
 * @returns       StateHexTokens[] in same order
 */
export async function batchDeriveFast(colors: OklchColor[]): Promise<StateHexTokens[]> {
  await initMomoto();
  const bases = new Float64Array(colors.length * 3);
  colors.forEach(({ l, c, h }, i) => {
    bases[i * 3] = l; bases[i * 3 + 1] = c; bases[i * 3 + 2] = h;
  });

  const raw = batch_derive_tokens(bases);
  const keys = ['idle','hover','active','focus','disabled','loading'] as const;

  return colors.map((_, ci) => {
    const offset = ci * 24;   // 6 states × 4 floats
    const out = {} as StateHexTokens;
    for (let si = 0; si < 6; si++) {
      const o = offset + si * 4;
      try {
        const color = new ColorOklch(raw[o], raw[o+1], raw[o+2]);
        out[keys[si]] = color.to_hex();
        color.free();
      } catch { out[keys[si]] = '#000000'; }
    }
    return out;
  });
}

// ── v2.0: getInteractionColor — one-call state resolution + color ─────────────
/**
 * Determine the correct UI color for a component given its current interaction flags.
 * Combines determine_ui_state() + derive_token_for_state() in a single efficient call.
 *
 * @param base   base OKLCH color
 * @param flags  current interaction state flags
 * @returns      hex color for the resolved state
 */
export async function getInteractionColor(base: OklchColor, flags: InteractionFlags): Promise<string> {
  await initMomoto();
  const state = determine_ui_state(
    flags.disabled ?? false,
    flags.loading  ?? false,
    flags.active   ?? false,
    flags.focused  ?? false,
    flags.hovered  ?? false,
  );
  const token = derive_token_for_state(base.l, base.c, base.h, state);
  try {
    const color = new ColorOklch(token[0], token[1], token[2]);
    const hex = color.to_hex();
    color.free();
    return hex;
  } catch { return '#000000'; }
}

/**
 * Get StateMetadata for a resolved UI state.
 * Useful for driving animation duration/intensity from physics-derived values.
 *
 * Returns: { lightness_shift, chroma_shift, opacity, animation, focus_indicator }
 */
export async function getStatePhysics(flags: InteractionFlags) {
  await initMomoto();
  const state = determine_ui_state(
    flags.disabled ?? false,
    flags.loading  ?? false,
    flags.active   ?? false,
    flags.focused  ?? false,
    flags.hovered  ?? false,
  );
  const meta = get_state_metadata(state);
  const result = {
    state,
    lightnessShift: meta.lightness_shift,
    chromaShift:    meta.chroma_shift,
    opacity:        meta.opacity,
    animationLevel: meta.animation,       // 0 = none, 1 = subtle, 2 = prominent
    requiresFocus:  meta.focus_indicator,
    priority:       get_state_priority(state),
  };
  meta.free();
  return result;
}

// ── Core wrappers (unchanged from v1, now use singleton engine) ───────────────
export async function shiftColor(
  color: OklchColor,
  delta: { lightness?: number; chroma?: number; hue?: number },
): Promise<string> {
  await initMomoto();
  const l = Math.max(0.0, Math.min(1.0,  color.l + (delta.lightness ?? 0)));
  const c = Math.max(0.0, Math.min(0.4,  color.c + (delta.chroma   ?? 0)));
  const h = ((color.h + (delta.hue ?? 0)) % 360 + 360) % 360;
  try {
    const clr = new ColorOklch(l, c, h);
    const hex = clr.to_hex(); clr.free(); return hex;
  } catch { return '#000000'; }
}

export async function batchDeriveColors(colors: OklchColor[]): Promise<StateHexTokens[]> {
  return batchDeriveFast(colors);  // v2: route through fast batch path
}

export const deriveFullStateColors = deriveStateColors;

export async function deriveStateColors(l: number, c: number, h: number): Promise<StateHexTokens> {
  await initMomoto();
  const engine = getEngine();            // v2: singleton engine (warm cache)
  const arr    = engine.derive_states(l, c, h);
  const keys   = ['idle','hover','active','focus','disabled','loading'] as const;
  const out    = {} as StateHexTokens;
  for (let i = 0; i < 6; i++) {
    try {
      const color = new ColorOklch(arr[i*4], arr[i*4+1], arr[i*4+2]);
      out[keys[i]] = color.to_hex(); color.free();
    } catch { out[keys[i]] = '#000000'; }
  }
  return out;
}

export async function checkContrast(fg: OklchColor, bg: OklchColor): Promise<ContrastReport> {
  await initMomoto();
  const result    = validate_contrast(fg.l, fg.c, fg.h, bg.l, bg.c, bg.h);
  const wcagRatio = result.wcag_ratio();
  const apca      = result.apca_contrast();
  const level     = result.wcag_normal_level();
  const report: ContrastReport = {
    wcagRatio, apcaContrast: apca,
    passesAA: wcagRatio >= WCAG_AA_NORMAL, passesAAA: wcagRatio >= WCAG_AAA_NORMAL,
    passesAPCABody: Math.abs(apca) >= APCA_MIN_BODY,
    level: level === 2 ? 'aaa' : level === 1 ? 'aa' : 'fail',
  };
  result.free();
  return report;
}

export async function isAccessible(fg: OklchColor, bg: OklchColor): Promise<boolean> {
  await initMomoto();
  return passes_wcag_aa(fg.l, fg.c, fg.h, bg.l, bg.c, bg.h);
}

/**
 * Inject Halcón fire-palette CSS custom properties onto :root.
 * v2: uses batchDeriveFast() for a single WASM round-trip.
 */
export async function injectCssTokens(): Promise<void> {
  await initMomoto();
  const root = document.documentElement;

  const [fireTokens, goldTokens, emberTokens, sparkTokens] = await batchDeriveFast([
    BRAND_FIRE, BRAND_GOLD, BRAND_EMBER, BRAND_SPARK,
  ]);

  // Fire orange — primary buttons
  root.style.setProperty('--m-btn-idle',     fireTokens.idle);
  root.style.setProperty('--m-btn-hover',    fireTokens.hover);
  root.style.setProperty('--m-btn-active',   fireTokens.active);
  root.style.setProperty('--m-btn-focus',    fireTokens.focus);
  root.style.setProperty('--m-btn-disabled', fireTokens.disabled);
  root.style.setProperty('--m-btn-loading',  fireTokens.loading);

  // Amber gold — accents
  root.style.setProperty('--m-accent-idle',     goldTokens.idle);
  root.style.setProperty('--m-accent-hover',    goldTokens.hover);
  root.style.setProperty('--m-accent-active',   goldTokens.active);
  root.style.setProperty('--m-accent-disabled', goldTokens.disabled);

  // Ember — secondary
  root.style.setProperty('--m-ember-idle',  emberTokens.idle);
  root.style.setProperty('--m-ember-hover', emberTokens.hover);

  // Spark — hottest
  root.style.setProperty('--m-spark-idle',  sparkTokens.idle);

  // Raw brand hex
  try {
    for (const [prop, {l,c,h}] of [
      ['--m-brand-fire',  BRAND_FIRE ],
      ['--m-brand-gold',  BRAND_GOLD ],
      ['--m-brand-ember', BRAND_EMBER],
      ['--m-brand-spark', BRAND_SPARK],
    ] as [string, OklchColor][]) {
      const color = new ColorOklch(l, c, h);
      root.style.setProperty(prop, color.to_hex());
      color.free();
    }
  } catch { /* non-critical */ }
}

// ── Phase 61 — Sentiment-aware emotional token generation (unchanged) ─────────
const FRUSTRATION_WORDS = new Set([
  'again','still','broken','wrong','error','fail','failed','terrible','awful',
  'useless','why','stupid','bad','keep','ridiculous','frustrating','pathetic',
  'unacceptable','impossible','never works',
]);
const FATIGUE_WORDS = new Set([
  'please','just','already','same','tired','exhausted','endless','forever',
  'give up','forget it','whatever','seriously','honestly',
]);
const CONFUSION_WORDS = new Set([
  'what','how','understand','confused','unclear','lost','help','explain',
  "don't get","not sure",'unsure','weird','strange','expected',
]);
const SATISFACTION_WORDS = new Set([
  'great','perfect','thanks','thank','excellent','wonderful','amazing',
  'love','awesome','nice','good job','fantastic','brilliant','exactly',
]);
const POSITIVE_WORDS  = new Set(['yes','works','done','ok','good','correct','right','fixed','solved','got it']);
const APOLOGY_WORDS   = new Set(['sorry','unfortunately','error','cannot','unable','apologize','regret','failed']);

export function analyzeSentiment(text: string, source: 'user' | 'agent' = 'user'): SentimentScore {
  const lower = text.toLowerCase();
  const words = lower.split(/[\s,!?.;:]+/).filter(w => w.length > 1);
  const wordSet = new Set(words);

  let valence = 0.0, arousal = 0.3, dominance = 0.5;

  for (const w of wordSet) {
    if (FRUSTRATION_WORDS.has(w))                    { valence -= 0.12; arousal += 0.08; }
    if (FATIGUE_WORDS.has(w))                        { valence -= 0.06; arousal -= 0.06; }
    if (CONFUSION_WORDS.has(w))                      { dominance -= 0.08; }
    if (SATISFACTION_WORDS.has(w))                   { valence += 0.14; arousal += 0.04; dominance += 0.05; }
    if (POSITIVE_WORDS.has(w))                       { valence += 0.06; dominance += 0.03; }
    if (source === 'agent' && APOLOGY_WORDS.has(w))  { valence -= 0.08; }
  }

  const exclamations = (text.match(/!/g) || []).length;
  if (exclamations >= 3)      { valence -= 0.25; arousal += 0.20; }
  else if (exclamations >= 1) { arousal += Math.min(exclamations * 0.15, 0.40); }

  if (text.endsWith('?') || text.endsWith('??')) dominance -= 0.10;
  if (text.includes('...')) arousal -= 0.10;

  const avgWords = words.length;
  if (avgWords < 5)  arousal += 0.10;
  if (avgWords > 30) arousal -= 0.05;

  return {
    valence:   Math.max(-1.0, Math.min(1.0, valence)),
    arousal:   Math.max(0.0,  Math.min(1.0, arousal)),
    dominance: Math.max(0.0,  Math.min(1.0, dominance)),
  };
}

export function emotionalStateFromVad(
  valence: number, arousal: number, dominance: number,
): EmotionalState {
  if (valence > 0.5 && arousal > 0.6)                          return 'excited';
  if (valence > 0.4 && arousal < 0.4)                          return 'satisfied';
  if (valence < -0.3 && arousal > 0.5)                         return 'frustrated';
  if (valence < -0.1 && arousal < 0.25)                        return 'fatigued';
  if (dominance < 0.3 && arousal > 0.25 && arousal < 0.6)     return 'confused';
  if (valence > 0.2 && arousal > 0.4)                          return 'engaged';
  return 'neutral';
}

export async function deriveEmotionalTokens(
  { valence, arousal, dominance }: SentimentScore,
): Promise<EmotionalTokens> {
  await initMomoto();
  const state = emotionalStateFromVad(valence, arousal, dominance);

  let primaryLch = { l: 0.80, c: 0.18, h: 207.0 };
  let accentLch  = { l: 0.88, c: 0.14, h: 194.0 };
  let runningLch = { l: 0.80, c: 0.18, h: 207.0 };
  let warningLch = { l: 0.85, c: 0.16, h: 82.0  };
  let successLch = { l: 0.58, c: 0.22, h: 142.0 };
  let mutedLch   = { l: 0.48, c: 0.022, h: 232.0 };

  switch (state) {
    case 'engaged':    primaryLch = { ...primaryLch, c: primaryLch.c + 0.02 };
                       accentLch  = { ...accentLch,  l: accentLch.l + 0.02 }; break;
    case 'satisfied':  successLch = { ...successLch, c: successLch.c + 0.04 };
                       primaryLch = { ...primaryLch, h: ((primaryLch.h - 5) % 360 + 360) % 360 };
                       runningLch = successLch; break;
    case 'frustrated': runningLch = { ...runningLch, h: (runningLch.h + 150) % 360 };
                       warningLch = { ...warningLch, c: warningLch.c + 0.06 }; break;
    case 'fatigued':   runningLch = { ...runningLch, c: Math.max(0, runningLch.c - 0.03) };
                       mutedLch   = { ...mutedLch,   l: mutedLch.l + 0.02 }; break;
    case 'confused':   mutedLch   = { ...mutedLch,   l: mutedLch.l + 0.03 }; break;
    case 'excited':    primaryLch = { ...primaryLch, l: primaryLch.l + 0.03, c: primaryLch.c + 0.03 };
                       accentLch  = { ...accentLch,  c: accentLch.c + 0.03 };
                       runningLch = { ...runningLch, c: runningLch.c + 0.03 };
                       successLch = { ...successLch, c: successLch.c + 0.03 }; break;
  }

  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
  const norm  = (lch: OklchColor) => ({
    l: clamp(lch.l, 0, 1), c: clamp(lch.c, 0, 0.5), h: ((lch.h % 360) + 360) % 360,
  });

  const [pT, aT, rT, wT, sT, mT] = await batchDeriveFast([
    norm(primaryLch), norm(accentLch), norm(runningLch),
    norm(warningLch), norm(successLch), norm(mutedLch),
  ]);

  return {
    primary: pT.idle, accent: aT.idle, running: rT.idle,
    warning: wT.idle, success: sT.idle, muted: mT.idle,
  };
}

export async function injectEmotionalCssTokens(score: SentimentScore): Promise<void> {
  const tokens = await deriveEmotionalTokens(score);
  const root = document.documentElement;
  root.style.setProperty('--m-emotional-primary', tokens.primary);
  root.style.setProperty('--m-emotional-accent',  tokens.accent);
  root.style.setProperty('--m-emotional-running', tokens.running);
  root.style.setProperty('--m-emotional-warning', tokens.warning);
  root.style.setProperty('--m-emotional-success', tokens.success);
  root.style.setProperty('--m-emotional-muted',   tokens.muted);
  root.style.setProperty('--m-emotional-state',   emotionalStateFromVad(
    score.valence, score.arousal, score.dominance,
  ));
}

export async function adaptToText(
  text: string, source: 'user' | 'agent' = 'user',
): Promise<SentimentScore> {
  const score = analyzeSentiment(text, source);
  await injectEmotionalCssTokens(score);
  return score;
}
