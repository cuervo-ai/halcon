/* tslint:disable */
/* eslint-disable */

export class ColorOklch {
  free(): void;
  [Symbol.dispose](): void;
  /**
   * Create new OKLCH color with validation
   *
   * # Arguments
   * * `l` - Lightness [0.0, 1.0]
   * * `c` - Chroma [0.0, 0.4]
   * * `h` - Hue [0.0, 360.0]
   *
   * # Returns
   * ColorOklch or Error if values out of range
   *
   * # Example
   * ```typescript
   * import { ColorOklch } from '@momoto-ui/wasm';
   *
   * const color = ColorOklch.new(0.5, 0.1, 180.0);
   * // Mid-lightness, low chroma, cyan hue
   * ```
   */
  constructor(l: number, c: number, h: number);
  /**
   * Shift lightness by delta
   *
   * # Arguments
   * * `delta` - Lightness shift [-1.0, 1.0]
   *
   * # Returns
   * New ColorOklch with shifted lightness (clamped to valid range)
   *
   * # Example
   * ```typescript
   * const base = ColorOklch.new(0.5, 0.1, 180.0);
   * const lighter = base.shift_lightness(0.1); // l=0.6
   * const darker = base.shift_lightness(-0.2); // l=0.3
   * ```
   */
  shift_lightness(delta: number): ColorOklch;
  /**
   * Shift chroma by delta
   *
   * # Arguments
   * * `delta` - Chroma shift [-0.4, 0.4]
   *
   * # Returns
   * New ColorOklch with shifted chroma (clamped to valid range)
   */
  shift_chroma(delta: number): ColorOklch;
  /**
   * Rotate hue by degrees
   *
   * # Arguments
   * * `degrees` - Hue rotation in degrees
   *
   * # Returns
   * New ColorOklch with rotated hue (wrapped to [0, 360])
   */
  rotate_hue(degrees: number): ColorOklch;
  /**
   * Convert to hex string (via RGB)
   *
   * # Returns
   * Hex color string (e.g., "#FF5733")
   *
   * Note: This is a simplified conversion. For production, use
   * momoto-core's precise OKLCH → sRGB conversion.
   */
  to_hex(): string;
  /**
   * Create from hex string
   *
   * # Arguments
   * * `hex` - Hex color string (e.g., "#FF5733" or "FF5733")
   *
   * # Returns
   * ColorOklch or Error if invalid hex
   */
  static fromHex(hex: string): ColorOklch;
  /**
   * Lightness [0.0, 1.0]
   */
  l: number;
  /**
   * Chroma [0.0, 0.4]
   */
  c: number;
  /**
   * Hue [0.0, 360.0] degrees
   */
  h: number;
}

/**
 * WCAG conformance level
 */
export enum ContrastLevel {
  /**
   * Does not meet minimum standards
   */
  Fail = 0,
  /**
   * WCAG AA (4.5:1 normal, 3:1 large)
   */
  AA = 1,
  /**
   * WCAG AAA (7:1 normal, 4.5:1 large)
   */
  AAA = 2,
}

export class ContrastResult {
  private constructor();
  free(): void;
  [Symbol.dispose](): void;
  /**
   * Get WCAG contrast ratio
   */
  wcag_ratio(): number;
  /**
   * Get APCA contrast value
   */
  apca_contrast(): number;
  /**
   * Get WCAG level for normal text (0=Fail, 1=AA, 2=AAA)
   */
  wcag_normal_level(): number;
  /**
   * Get WCAG level for large text (0=Fail, 1=AA, 2=AAA)
   */
  wcag_large_level(): number;
  /**
   * Check if passes APCA for body text
   */
  apca_body_pass(): boolean;
  /**
   * Check if passes APCA for large text
   */
  apca_large_pass(): boolean;
}

export class StateMetadata {
  private constructor();
  free(): void;
  [Symbol.dispose](): void;
  /**
   * Lightness shift to apply [-1.0, 1.0]
   */
  lightness_shift: number;
  /**
   * Chroma shift to apply [-1.0, 1.0]
   */
  chroma_shift: number;
  /**
   * Opacity multiplier [0.0, 1.0]
   */
  opacity: number;
  /**
   * Get animation level as u8
   */
  readonly animation: number;
  /**
   * Check if focus indicator is required
   */
  readonly focus_indicator: boolean;
}

export class TokenDerivationEngine {
  free(): void;
  [Symbol.dispose](): void;
  /**
   * Create new token derivation engine
   */
  constructor();
  /**
   * Derive state tokens from base color
   *
   * Derives tokens for all common UI states:
   * - Idle (baseline)
   * - Hover (slightly lighter)
   * - Active (darker)
   * - Focus (same as idle, but with focus indicator)
   * - Disabled (much lighter, desaturated)
   * - Loading (desaturated)
   *
   * # Arguments
   * * `base_l` - Base lightness [0.0, 1.0]
   * * `base_c` - Base chroma [0.0, 0.4]
   * * `base_h` - Base hue [0.0, 360.0]
   *
   * # Returns
   * Float64Array with packed tokens: [l, c, h, state, l, c, h, state, ...]
   * Each token is 4 values: lightness, chroma, hue, state_id
   *
   * # Performance
   * - First call: ~0.2ms (cold cache)
   * - Subsequent calls: ~0.02ms (cache hit)
   *
   * # Example
   * ```typescript
   * const engine = new TokenDerivationEngine();
   * const tokens = engine.derive_states(0.5, 0.1, 180.0);
   *
   * // Unpack first token (Idle)
   * const idle_l = tokens[0];
   * const idle_c = tokens[1];
   * const idle_h = tokens[2];
   * const idle_state = tokens[3]; // 0 (Idle)
   * ```
   */
  derive_states(base_l: number, base_c: number, base_h: number): Float64Array;
  /**
   * Get cache size
   *
   * # Returns
   * Number of cached derivations
   */
  cache_size(): number;
  /**
   * Clear cache
   *
   * Useful for memory management in long-running applications.
   */
  clear_cache(): void;
  /**
   * Get cache statistics
   *
   * # Returns
   * Object with cache stats
   */
  cache_stats(): any;
}

/**
 * UI interaction states
 *
 * Priority (highest to lowest):
 * 1. Disabled (100)
 * 2. Loading (90)
 * 3. Error (80)
 * 4. Success (75)
 * 5. Active (60)
 * 6. Focus (50)
 * 7. Hover (40)
 * 8. Idle (0)
 */
export enum UIState {
  Idle = 0,
  Hover = 1,
  Active = 2,
  Focus = 3,
  Disabled = 4,
  Loading = 5,
  Error = 6,
  Success = 7,
}

/**
 * Batch derive tokens for multiple base colors
 *
 * More efficient than calling derive_states multiple times.
 *
 * # Arguments
 * * `bases` - Float64Array of base colors [l, c, h, l, c, h, ...]
 *
 * # Returns
 * Float64Array of all derived tokens
 */
export function batch_derive_tokens(bases: Float64Array): Float64Array;

/**
 * Batch validate contrast for multiple color pairs
 *
 * More efficient than calling validate_contrast multiple times.
 *
 * # Arguments
 * * `pairs` - Float64Array of color pairs [fg_l, fg_c, fg_h, bg_l, bg_c, bg_h, ...]
 *
 * # Returns
 * Array of ContrastResult objects
 */
export function batch_validate_contrast(pairs: Float64Array): Array<any>;

/**
 * Combine multiple states, returning the highest priority
 *
 * # Arguments
 * * `states` - Array of state values
 *
 * # Returns
 * Highest priority state as u8
 *
 * # Example
 * ```typescript
 * import { combine_states } from 'momoto-ui-wasm';
 * const result = combine_states(new Uint8Array([1, 3, 0])); // Hover, Focus, Idle
 * // result === 3 (Focus has higher priority than Hover)
 * ```
 */
export function combine_states(states: Uint8Array): number;

/**
 * Derive tokens for specific state (one-shot, no caching)
 *
 * Useful for one-off derivations where caching isn't needed.
 *
 * # Arguments
 * * `base_l` - Base lightness
 * * `base_c` - Base chroma
 * * `base_h` - Base hue
 * * `state` - State to derive (0-7)
 *
 * # Returns
 * Float64Array [l, c, h, state]
 */
export function derive_token_for_state(base_l: number, base_c: number, base_h: number, state: number): Float64Array;

/**
 * Determine UI state from interaction flags
 *
 * # Arguments
 * * `disabled` - Component is disabled
 * * `loading` - Component is in loading state
 * * `active` - Component is being pressed/clicked
 * * `focused` - Component has keyboard focus
 * * `hovered` - Component is being hovered
 *
 * # Returns
 * State as u8 (0=Idle, 1=Hover, 2=Active, 3=Focus, 4=Disabled, 5=Loading, 6=Error, 7=Success)
 *
 * # Example
 * ```typescript
 * import { determine_ui_state } from 'momoto-ui-wasm';
 * const state = determine_ui_state(false, false, true, false, false);
 * // state === 2 (Active)
 * ```
 */
export function determine_ui_state(disabled: boolean, loading: boolean, active: boolean, focused: boolean, hovered: boolean): number;

/**
 * Get state metadata for a given state
 *
 * # Arguments
 * * `state` - State value (0-7)
 *
 * # Returns
 * StateMetadata with perceptual shifts and animation info
 *
 * # Example
 * ```typescript
 * import { get_state_metadata } from 'momoto-ui-wasm';
 * const metadata = get_state_metadata(1); // Hover
 * console.log(metadata.lightness_shift); // 0.05
 * ```
 */
export function get_state_metadata(state: number): StateMetadata;

/**
 * Get state priority
 *
 * # Arguments
 * * `state` - State value (0-7)
 *
 * # Returns
 * Priority as u8 (higher = takes precedence)
 */
export function get_state_priority(state: number): number;

/**
 * Quick check if contrast meets WCAG AA for normal text
 *
 * # Arguments
 * * `foreground_l` - Foreground lightness [0.0, 1.0]
 * * `foreground_c` - Foreground chroma [0.0, 0.4]
 * * `foreground_h` - Foreground hue [0.0, 360.0]
 * * `background_l` - Background lightness [0.0, 1.0]
 * * `background_c` - Background chroma [0.0, 0.4]
 * * `background_h` - Background hue [0.0, 360.0]
 *
 * # Returns
 * true if contrast >= 4.5:1
 */
export function passes_wcag_aa(foreground_l: number, foreground_c: number, foreground_h: number, background_l: number, background_c: number, background_h: number): boolean;

/**
 * Validate contrast between two colors
 *
 * Calculates both WCAG 2.1 and APCA contrast metrics.
 *
 * # Arguments
 * * `foreground_l` - Foreground lightness [0.0, 1.0]
 * * `foreground_c` - Foreground chroma [0.0, 0.4]
 * * `foreground_h` - Foreground hue [0.0, 360.0]
 * * `background_l` - Background lightness [0.0, 1.0]
 * * `background_c` - Background chroma [0.0, 0.4]
 * * `background_h` - Background hue [0.0, 360.0]
 *
 * # Returns
 * ContrastResult with WCAG and APCA metrics
 *
 * # Example
 * ```typescript
 * import { validateContrast } from '@momoto-ui/wasm';
 *
 * const result = validateContrast(
 *   0.2, 0.05, 240.0,  // Dark blue text
 *   0.95, 0.02, 60.0   // Light yellow background
 * );
 *
 * console.log(result.wcag_ratio());        // e.g., 12.5
 * console.log(result.wcag_normal_level()); // 2 (AAA)
 * console.log(result.apca_contrast());     // e.g., -85.0
 * ```
 */
export function validate_contrast(foreground_l: number, foreground_c: number, foreground_h: number, background_l: number, background_c: number, background_h: number): ContrastResult;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_statemetadata_free: (a: number, b: number) => void;
  readonly statemetadata_animation: (a: number) => number;
  readonly statemetadata_focus_indicator: (a: number) => number;
  readonly determine_ui_state: (a: number, b: number, c: number, d: number, e: number) => number;
  readonly get_state_metadata: (a: number) => number;
  readonly get_state_priority: (a: number) => number;
  readonly combine_states: (a: number, b: number) => number;
  readonly __wbg_coloroklch_free: (a: number, b: number) => void;
  readonly __wbg_get_coloroklch_l: (a: number) => number;
  readonly __wbg_set_coloroklch_l: (a: number, b: number) => void;
  readonly __wbg_get_coloroklch_c: (a: number) => number;
  readonly __wbg_set_coloroklch_c: (a: number, b: number) => void;
  readonly __wbg_get_coloroklch_h: (a: number) => number;
  readonly __wbg_set_coloroklch_h: (a: number, b: number) => void;
  readonly coloroklch_new: (a: number, b: number, c: number, d: number) => void;
  readonly coloroklch_shift_lightness: (a: number, b: number) => number;
  readonly coloroklch_shift_chroma: (a: number, b: number) => number;
  readonly coloroklch_rotate_hue: (a: number, b: number) => number;
  readonly coloroklch_to_hex: (a: number, b: number) => void;
  readonly coloroklch_fromHex: (a: number, b: number, c: number) => void;
  readonly __wbg_tokenderivationengine_free: (a: number, b: number) => void;
  readonly tokenderivationengine_new: () => number;
  readonly tokenderivationengine_derive_states: (a: number, b: number, c: number, d: number, e: number) => void;
  readonly tokenderivationengine_cache_size: (a: number) => number;
  readonly tokenderivationengine_clear_cache: (a: number) => void;
  readonly tokenderivationengine_cache_stats: (a: number) => number;
  readonly derive_token_for_state: (a: number, b: number, c: number, d: number, e: number) => void;
  readonly batch_derive_tokens: (a: number, b: number, c: number) => void;
  readonly contrastresult_wcag_ratio: (a: number) => number;
  readonly contrastresult_apca_contrast: (a: number) => number;
  readonly contrastresult_wcag_normal_level: (a: number) => number;
  readonly contrastresult_wcag_large_level: (a: number) => number;
  readonly contrastresult_apca_body_pass: (a: number) => number;
  readonly contrastresult_apca_large_pass: (a: number) => number;
  readonly validate_contrast: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
  readonly batch_validate_contrast: (a: number, b: number, c: number) => void;
  readonly passes_wcag_aa: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
  readonly __wbg_set_statemetadata_chroma_shift: (a: number, b: number) => void;
  readonly __wbg_set_statemetadata_opacity: (a: number, b: number) => void;
  readonly __wbg_get_statemetadata_lightness_shift: (a: number) => number;
  readonly __wbg_get_statemetadata_opacity: (a: number) => number;
  readonly __wbg_get_statemetadata_chroma_shift: (a: number) => number;
  readonly __wbg_set_statemetadata_lightness_shift: (a: number, b: number) => void;
  readonly __wbg_contrastresult_free: (a: number, b: number) => void;
  readonly __wbindgen_export: (a: number) => void;
  readonly __wbindgen_export2: (a: number, b: number) => number;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_export3: (a: number, b: number, c: number) => void;
  readonly __wbindgen_export4: (a: number, b: number, c: number, d: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
