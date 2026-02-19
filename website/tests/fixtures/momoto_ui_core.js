let wasm;

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

function dropObject(idx) {
    if (idx < 132) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function getArrayF64FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat64ArrayMemory0().subarray(ptr / 8, ptr / 8 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedFloat64ArrayMemory0 = null;
function getFloat64ArrayMemory0() {
    if (cachedFloat64ArrayMemory0 === null || cachedFloat64ArrayMemory0.byteLength === 0) {
        cachedFloat64ArrayMemory0 = new Float64Array(wasm.memory.buffer);
    }
    return cachedFloat64ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return decodeText(ptr, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function getObject(idx) { return heap[idx]; }

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        wasm.__wbindgen_export(addHeapObject(e));
    }
}

let heap = new Array(128).fill(undefined);
heap.push(undefined, null, true, false);

let heap_next = heap.length;

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayF64ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 8, 8) >>> 0;
    getFloat64ArrayMemory0().set(arg, ptr / 8);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    }
}

let WASM_VECTOR_LEN = 0;

const ColorOklchFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_coloroklch_free(ptr >>> 0, 1));

const ContrastResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_contrastresult_free(ptr >>> 0, 1));

const StateMetadataFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_statemetadata_free(ptr >>> 0, 1));

const TokenDerivationEngineFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_tokenderivationengine_free(ptr >>> 0, 1));

/**
 * OKLCH color representation
 *
 * OKLCH is a cylindrical representation of Oklab:
 * - L (Lightness): 0.0 (black) to 1.0 (white)
 * - C (Chroma): 0.0 (gray) to ~0.4 (vivid)
 * - H (Hue): 0.0 to 360.0 (degrees)
 *
 * This color space is perceptually uniform, meaning that equal distances
 * in the color space correspond to equal perceived differences.
 */
export class ColorOklch {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(ColorOklch.prototype);
        obj.__wbg_ptr = ptr;
        ColorOklchFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ColorOklchFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_coloroklch_free(ptr, 0);
    }
    /**
     * Lightness [0.0, 1.0]
     * @returns {number}
     */
    get l() {
        const ret = wasm.__wbg_get_coloroklch_l(this.__wbg_ptr);
        return ret;
    }
    /**
     * Lightness [0.0, 1.0]
     * @param {number} arg0
     */
    set l(arg0) {
        wasm.__wbg_set_coloroklch_l(this.__wbg_ptr, arg0);
    }
    /**
     * Chroma [0.0, 0.4]
     * @returns {number}
     */
    get c() {
        const ret = wasm.__wbg_get_coloroklch_c(this.__wbg_ptr);
        return ret;
    }
    /**
     * Chroma [0.0, 0.4]
     * @param {number} arg0
     */
    set c(arg0) {
        wasm.__wbg_set_coloroklch_c(this.__wbg_ptr, arg0);
    }
    /**
     * Hue [0.0, 360.0] degrees
     * @returns {number}
     */
    get h() {
        const ret = wasm.__wbg_get_coloroklch_h(this.__wbg_ptr);
        return ret;
    }
    /**
     * Hue [0.0, 360.0] degrees
     * @param {number} arg0
     */
    set h(arg0) {
        wasm.__wbg_set_coloroklch_h(this.__wbg_ptr, arg0);
    }
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
     * @param {number} l
     * @param {number} c
     * @param {number} h
     */
    constructor(l, c, h) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.coloroklch_new(retptr, l, c, h);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            if (r2) {
                throw takeObject(r1);
            }
            this.__wbg_ptr = r0 >>> 0;
            ColorOklchFinalization.register(this, this.__wbg_ptr, this);
            return this;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
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
     * @param {number} delta
     * @returns {ColorOklch}
     */
    shift_lightness(delta) {
        const ret = wasm.coloroklch_shift_lightness(this.__wbg_ptr, delta);
        return ColorOklch.__wrap(ret);
    }
    /**
     * Shift chroma by delta
     *
     * # Arguments
     * * `delta` - Chroma shift [-0.4, 0.4]
     *
     * # Returns
     * New ColorOklch with shifted chroma (clamped to valid range)
     * @param {number} delta
     * @returns {ColorOklch}
     */
    shift_chroma(delta) {
        const ret = wasm.coloroklch_shift_chroma(this.__wbg_ptr, delta);
        return ColorOklch.__wrap(ret);
    }
    /**
     * Rotate hue by degrees
     *
     * # Arguments
     * * `degrees` - Hue rotation in degrees
     *
     * # Returns
     * New ColorOklch with rotated hue (wrapped to [0, 360])
     * @param {number} degrees
     * @returns {ColorOklch}
     */
    rotate_hue(degrees) {
        const ret = wasm.coloroklch_rotate_hue(this.__wbg_ptr, degrees);
        return ColorOklch.__wrap(ret);
    }
    /**
     * Convert to hex string (via RGB)
     *
     * # Returns
     * Hex color string (e.g., "#FF5733")
     *
     * Note: This is a simplified conversion. For production, use
     * momoto-core's precise OKLCH → sRGB conversion.
     * @returns {string}
     */
    to_hex() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.coloroklch_to_hex(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Create from hex string
     *
     * # Arguments
     * * `hex` - Hex color string (e.g., "#FF5733" or "FF5733")
     *
     * # Returns
     * ColorOklch or Error if invalid hex
     * @param {string} hex
     * @returns {ColorOklch}
     */
    static fromHex(hex) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            const ptr0 = passStringToWasm0(hex, wasm.__wbindgen_export2, wasm.__wbindgen_export4);
            const len0 = WASM_VECTOR_LEN;
            wasm.coloroklch_fromHex(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            if (r2) {
                throw takeObject(r1);
            }
            return ColorOklch.__wrap(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
}
if (Symbol.dispose) ColorOklch.prototype[Symbol.dispose] = ColorOklch.prototype.free;

/**
 * WCAG conformance level
 * @enum {0 | 1 | 2}
 */
export const ContrastLevel = Object.freeze({
    /**
     * Does not meet minimum standards
     */
    Fail: 0, "0": "Fail",
    /**
     * WCAG AA (4.5:1 normal, 3:1 large)
     */
    AA: 1, "1": "AA",
    /**
     * WCAG AAA (7:1 normal, 4.5:1 large)
     */
    AAA: 2, "2": "AAA",
});

/**
 * Contrast validation result
 */
export class ContrastResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(ContrastResult.prototype);
        obj.__wbg_ptr = ptr;
        ContrastResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ContrastResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_contrastresult_free(ptr, 0);
    }
    /**
     * Get WCAG contrast ratio
     * @returns {number}
     */
    wcag_ratio() {
        const ret = wasm.contrastresult_wcag_ratio(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get APCA contrast value
     * @returns {number}
     */
    apca_contrast() {
        const ret = wasm.contrastresult_apca_contrast(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get WCAG level for normal text (0=Fail, 1=AA, 2=AAA)
     * @returns {number}
     */
    wcag_normal_level() {
        const ret = wasm.contrastresult_wcag_normal_level(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get WCAG level for large text (0=Fail, 1=AA, 2=AAA)
     * @returns {number}
     */
    wcag_large_level() {
        const ret = wasm.contrastresult_wcag_large_level(this.__wbg_ptr);
        return ret;
    }
    /**
     * Check if passes APCA for body text
     * @returns {boolean}
     */
    apca_body_pass() {
        const ret = wasm.contrastresult_apca_body_pass(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Check if passes APCA for large text
     * @returns {boolean}
     */
    apca_large_pass() {
        const ret = wasm.contrastresult_apca_large_pass(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) ContrastResult.prototype[Symbol.dispose] = ContrastResult.prototype.free;

/**
 * Perceptual metadata for UI state
 *
 * Defines how a state affects visual appearance:
 * - Lightness/chroma shifts for color adjustments
 * - Opacity for disabled/loading states
 * - Animation level
 * - Focus indicator requirement
 */
export class StateMetadata {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(StateMetadata.prototype);
        obj.__wbg_ptr = ptr;
        StateMetadataFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        StateMetadataFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_statemetadata_free(ptr, 0);
    }
    /**
     * Lightness shift to apply [-1.0, 1.0]
     * @returns {number}
     */
    get lightness_shift() {
        const ret = wasm.__wbg_get_coloroklch_l(this.__wbg_ptr);
        return ret;
    }
    /**
     * Lightness shift to apply [-1.0, 1.0]
     * @param {number} arg0
     */
    set lightness_shift(arg0) {
        wasm.__wbg_set_coloroklch_l(this.__wbg_ptr, arg0);
    }
    /**
     * Chroma shift to apply [-1.0, 1.0]
     * @returns {number}
     */
    get chroma_shift() {
        const ret = wasm.__wbg_get_coloroklch_c(this.__wbg_ptr);
        return ret;
    }
    /**
     * Chroma shift to apply [-1.0, 1.0]
     * @param {number} arg0
     */
    set chroma_shift(arg0) {
        wasm.__wbg_set_coloroklch_c(this.__wbg_ptr, arg0);
    }
    /**
     * Opacity multiplier [0.0, 1.0]
     * @returns {number}
     */
    get opacity() {
        const ret = wasm.__wbg_get_coloroklch_h(this.__wbg_ptr);
        return ret;
    }
    /**
     * Opacity multiplier [0.0, 1.0]
     * @param {number} arg0
     */
    set opacity(arg0) {
        wasm.__wbg_set_coloroklch_h(this.__wbg_ptr, arg0);
    }
    /**
     * Get animation level as u8
     * @returns {number}
     */
    get animation() {
        const ret = wasm.statemetadata_animation(this.__wbg_ptr);
        return ret;
    }
    /**
     * Check if focus indicator is required
     * @returns {boolean}
     */
    get focus_indicator() {
        const ret = wasm.statemetadata_focus_indicator(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) StateMetadata.prototype[Symbol.dispose] = StateMetadata.prototype.free;

/**
 * Token derivation engine with memoization
 *
 * This engine derives color tokens for UI states (hover, active, etc.)
 * with intelligent caching for maximum performance.
 *
 * # Performance
 * - First call: ~0.2ms (compute + cache)
 * - Cache hit: ~0.02ms (10x faster)
 * - Cache hit rate: typically >80%
 *
 * # Example
 * ```typescript
 * import { TokenDerivationEngine, ColorOklch } from '@momoto-ui/wasm';
 *
 * const engine = new TokenDerivationEngine();
 * const base = ColorOklch.new(0.5, 0.1, 180.0);
 *
 * // Derive all state tokens
 * const tokens = engine.derive_states(base.l, base.c, base.h);
 * // Returns: Float64Array[l, c, h, state, l, c, h, state, ...]
 * ```
 */
export class TokenDerivationEngine {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        TokenDerivationEngineFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_tokenderivationengine_free(ptr, 0);
    }
    /**
     * Create new token derivation engine
     */
    constructor() {
        const ret = wasm.tokenderivationengine_new();
        this.__wbg_ptr = ret >>> 0;
        TokenDerivationEngineFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
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
     * @param {number} base_l
     * @param {number} base_c
     * @param {number} base_h
     * @returns {Float64Array}
     */
    derive_states(base_l, base_c, base_h) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.tokenderivationengine_derive_states(retptr, this.__wbg_ptr, base_l, base_c, base_h);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            if (r2) {
                throw takeObject(r1);
            }
            return takeObject(r0);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Get cache size
     *
     * # Returns
     * Number of cached derivations
     * @returns {number}
     */
    cache_size() {
        const ret = wasm.tokenderivationengine_cache_size(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Clear cache
     *
     * Useful for memory management in long-running applications.
     */
    clear_cache() {
        wasm.tokenderivationengine_clear_cache(this.__wbg_ptr);
    }
    /**
     * Get cache statistics
     *
     * # Returns
     * Object with cache stats
     * @returns {any}
     */
    cache_stats() {
        const ret = wasm.tokenderivationengine_cache_stats(this.__wbg_ptr);
        return takeObject(ret);
    }
}
if (Symbol.dispose) TokenDerivationEngine.prototype[Symbol.dispose] = TokenDerivationEngine.prototype.free;

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
 * @enum {0 | 1 | 2 | 3 | 4 | 5 | 6 | 7}
 */
export const UIState = Object.freeze({
    Idle: 0, "0": "Idle",
    Hover: 1, "1": "Hover",
    Active: 2, "2": "Active",
    Focus: 3, "3": "Focus",
    Disabled: 4, "4": "Disabled",
    Loading: 5, "5": "Loading",
    Error: 6, "6": "Error",
    Success: 7, "7": "Success",
});

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
 * @param {Float64Array} bases
 * @returns {Float64Array}
 */
export function batch_derive_tokens(bases) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passArrayF64ToWasm0(bases, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.batch_derive_tokens(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
        if (r2) {
            throw takeObject(r1);
        }
        return takeObject(r0);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

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
 * @param {Float64Array} pairs
 * @returns {Array<any>}
 */
export function batch_validate_contrast(pairs) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passArrayF64ToWasm0(pairs, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.batch_validate_contrast(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
        if (r2) {
            throw takeObject(r1);
        }
        return takeObject(r0);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

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
 * @param {Uint8Array} states
 * @returns {number}
 */
export function combine_states(states) {
    const ptr0 = passArray8ToWasm0(states, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.combine_states(ptr0, len0);
    return ret;
}

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
 * @param {number} base_l
 * @param {number} base_c
 * @param {number} base_h
 * @param {number} state
 * @returns {Float64Array}
 */
export function derive_token_for_state(base_l, base_c, base_h, state) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.derive_token_for_state(retptr, base_l, base_c, base_h, state);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
        if (r2) {
            throw takeObject(r1);
        }
        return takeObject(r0);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

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
 * @param {boolean} disabled
 * @param {boolean} loading
 * @param {boolean} active
 * @param {boolean} focused
 * @param {boolean} hovered
 * @returns {number}
 */
export function determine_ui_state(disabled, loading, active, focused, hovered) {
    const ret = wasm.determine_ui_state(disabled, loading, active, focused, hovered);
    return ret;
}

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
 * @param {number} state
 * @returns {StateMetadata}
 */
export function get_state_metadata(state) {
    const ret = wasm.get_state_metadata(state);
    return StateMetadata.__wrap(ret);
}

/**
 * Get state priority
 *
 * # Arguments
 * * `state` - State value (0-7)
 *
 * # Returns
 * Priority as u8 (higher = takes precedence)
 * @param {number} state
 * @returns {number}
 */
export function get_state_priority(state) {
    const ret = wasm.get_state_priority(state);
    return ret;
}

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
 * @param {number} foreground_l
 * @param {number} foreground_c
 * @param {number} foreground_h
 * @param {number} background_l
 * @param {number} background_c
 * @param {number} background_h
 * @returns {boolean}
 */
export function passes_wcag_aa(foreground_l, foreground_c, foreground_h, background_l, background_c, background_h) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.passes_wcag_aa(retptr, foreground_l, foreground_c, foreground_h, background_l, background_c, background_h);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
        if (r2) {
            throw takeObject(r1);
        }
        return r0 !== 0;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

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
 * @param {number} foreground_l
 * @param {number} foreground_c
 * @param {number} foreground_h
 * @param {number} background_l
 * @param {number} background_c
 * @param {number} background_h
 * @returns {ContrastResult}
 */
export function validate_contrast(foreground_l, foreground_c, foreground_h, background_l, background_c, background_h) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.validate_contrast(retptr, foreground_l, foreground_c, foreground_h, background_l, background_c, background_h);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
        if (r2) {
            throw takeObject(r1);
        }
        return ContrastResult.__wrap(r0);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

const EXPECTED_RESPONSE_TYPES = new Set(['basic', 'cors', 'default']);

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && EXPECTED_RESPONSE_TYPES.has(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else {
                    throw e;
                }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }
}

function __wbg_get_imports() {
    const imports = {};
    imports.wbg = {};
    imports.wbg.__wbg___wbindgen_throw_dd24417ed36fc46e = function(arg0, arg1) {
        throw new Error(getStringFromWasm0(arg0, arg1));
    };
    imports.wbg.__wbg_contrastresult_new = function(arg0) {
        const ret = ContrastResult.__wrap(arg0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_get_index_14b8dac5cd612a86 = function(arg0, arg1) {
        const ret = getObject(arg0)[arg1 >>> 0];
        return ret;
    };
    imports.wbg.__wbg_length_406f6daaaa453057 = function(arg0) {
        const ret = getObject(arg0).length;
        return ret;
    };
    imports.wbg.__wbg_new_1ba21ce319a06297 = function() {
        const ret = new Object();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_new_25f239778d6112b9 = function() {
        const ret = new Array();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_new_from_slice_9a48ef80d2a51f94 = function(arg0, arg1) {
        const ret = new Float64Array(getArrayF64FromWasm0(arg0, arg1));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_push_7d9be8f38fc13975 = function(arg0, arg1) {
        const ret = getObject(arg0).push(getObject(arg1));
        return ret;
    };
    imports.wbg.__wbg_set_781438a03c0c3c81 = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = Reflect.set(getObject(arg0), getObject(arg1), getObject(arg2));
        return ret;
    }, arguments) };
    imports.wbg.__wbindgen_cast_2241b6af4c4b2941 = function(arg0, arg1) {
        // Cast intrinsic for `Ref(String) -> Externref`.
        const ret = getStringFromWasm0(arg0, arg1);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_cast_d6cd19b81560fd6e = function(arg0) {
        // Cast intrinsic for `F64 -> Externref`.
        const ret = arg0;
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_object_drop_ref = function(arg0) {
        takeObject(arg0);
    };

    return imports;
}

function __wbg_finalize_init(instance, module) {
    wasm = instance.exports;
    __wbg_init.__wbindgen_wasm_module = module;
    cachedDataViewMemory0 = null;
    cachedFloat64ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;



    return wasm;
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (typeof module !== 'undefined') {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (typeof module_or_path !== 'undefined') {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (typeof module_or_path === 'undefined') {
        module_or_path = new URL('momoto_ui_core_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync };
export default __wbg_init;
