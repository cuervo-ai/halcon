/**
 * Momoto UI Core — WASM Integration Wrapper for Halcón website
 * Halcón brand palette — fire identity from logo:
 *   BRAND_FIRE  oklch(62% 0.22 38)  →  #e85200  primary CTA
 *   BRAND_EMBER oklch(40% 0.20 22)  →  #c41400  deep red accent
 *   BRAND_GOLD  oklch(80% 0.19 65)  →  #f5a000  amber highlight
 *   BRAND_BG    oklch( 4% 0.01 30)  →  #070401  warm black base
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

export const WCAG_AA_NORMAL = 4.5, WCAG_AA_LARGE  = 3.0;
export const WCAG_AAA_NORMAL = 7.0, WCAG_AAA_LARGE = 4.5;
export const APCA_MIN_BODY = 60.0,  APCA_MIN_LARGE = 45.0;

let initialized = false, initPromise: Promise<void> | null = null;

export async function initMomoto(): Promise<void> {
  if (initialized) return;
  if (initPromise) return initPromise;
  initPromise = __wbg_init('/momoto_ui_core_bg.wasm').then(() => { initialized = true; });
  return initPromise;
}

// ── Halcón fire brand constants (OKLCH from logo analysis) ─────────────────
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

export interface StateHexTokens {
  idle: string; hover: string; active: string;
  focus: string; disabled: string; loading: string;
}

export interface ContrastReport {
  wcagRatio: number; apcaContrast: number;
  passesAA: boolean; passesAAA: boolean; passesAPCABody: boolean;
  level: 'fail' | 'aa' | 'aaa';
}

export async function deriveStateColors(l: number, c: number, h: number): Promise<StateHexTokens> {
  await initMomoto();
  const engine = new TokenDerivationEngine();
  const arr    = engine.derive_states(l, c, h);
  engine.free();

  const keys = ['idle','hover','active','focus','disabled','loading'] as const;
  const out  = {} as StateHexTokens;
  for (let i = 0; i < 6; i++) {
    try {
      const color = new ColorOklch(arr[i*4], arr[i*4+1], arr[i*4+2]);
      out[keys[i]] = color.to_hex();
      color.free();
    } catch { out[keys[i]] = '#000000'; }
  }
  return out;
}

export async function checkContrast(
  fg: { l: number; c: number; h: number },
  bg: { l: number; c: number; h: number }
): Promise<ContrastReport> {
  await initMomoto();
  const result    = validate_contrast(fg.l, fg.c, fg.h, bg.l, bg.c, bg.h);
  const wcagRatio = result.wcag_ratio();
  const apca      = result.apca_contrast();
  const level     = result.wcag_normal_level();
  return {
    wcagRatio, apcaContrast: apca,
    passesAA: wcagRatio >= WCAG_AA_NORMAL, passesAAA: wcagRatio >= WCAG_AAA_NORMAL,
    passesAPCABody: Math.abs(apca) >= APCA_MIN_BODY,
    level: level === 2 ? 'aaa' : level === 1 ? 'aa' : 'fail',
  };
}

export async function isAccessible(
  fg: { l: number; c: number; h: number },
  bg: { l: number; c: number; h: number }
): Promise<boolean> {
  await initMomoto();
  return passes_wcag_aa(fg.l, fg.c, fg.h, bg.l, bg.c, bg.h);
}

/**
 * Inject Halcón fire-palette CSS custom properties onto :root.
 * Sets --m-btn-* (fire orange) and --m-accent-* (amber gold).
 */
export async function injectCssTokens(): Promise<void> {
  await initMomoto();
  const root = document.documentElement;

  const [fireTokens, goldTokens, emberTokens] = await Promise.all([
    deriveStateColors(BRAND_FIRE.l,  BRAND_FIRE.c,  BRAND_FIRE.h),
    deriveStateColors(BRAND_GOLD.l,  BRAND_GOLD.c,  BRAND_GOLD.h),
    deriveStateColors(BRAND_EMBER.l, BRAND_EMBER.c, BRAND_EMBER.h),
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

  // Ember red — secondary accents
  root.style.setProperty('--m-ember-idle',  emberTokens.idle);
  root.style.setProperty('--m-ember-hover', emberTokens.hover);

  // Raw brand hex via momoto OKLCH→sRGB conversion
  try {
    for (const [prop, {l, c, h}] of [
      ['--m-brand-fire',  BRAND_FIRE ],
      ['--m-brand-gold',  BRAND_GOLD ],
      ['--m-brand-ember', BRAND_EMBER],
      ['--m-brand-spark', BRAND_SPARK],
    ] as [string, {l: number; c: number; h: number}][]) {
      const color = new ColorOklch(l, c, h);
      root.style.setProperty(prop, color.to_hex());
      color.free();
    }
  } catch (_) { /* non-critical */ }
}
