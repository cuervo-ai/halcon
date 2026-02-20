/**
 * FERRUM — Molten Metal / Cellular Automaton (Canvas 2D)
 * ─────────────────────────────────────────────────────────────────────────────
 * Physics model: Cellular automaton on a 2D temperature grid
 *   — Each cell stores: temperature [0, 1], phase (solid/liquid/vapor)
 *   — Thermal diffusion: Laplacian heat spread (conductivity 0.18)
 *   — Phase transitions:
 *     solid  → liquid  at T > 0.35 (melting)
 *     liquid → vapor   at T > 0.80 (vaporization)
 *     vapor  → liquid  at T < 0.70 (condensation, fast)
 *     liquid → solid   at T < 0.30 (solidification, slow)
 *   — Gravity: liquid cells flow downward (probabilistic)
 *   — Mouse: adds intense heat at cursor (forge effect)
 *   — Ambient cooling: top row radiates heat away
 *
 * Rendering: ImageData pixel-level (one pixel per cell, scaled up)
 *   — Solid:  metallic grey + specular shimmer (hue-shifted via metal_hue)
 *   — Liquid: Momoto fire ramp by temperature
 *   — Vapor:  Momoto spark/white by temperature
 *   — Specular highlight: per-cell hue variation creates metallic look
 *
 * Iteration 1: Grid CA + ImageData rendering
 * Iteration 2: Mouse heat, phase transitions, specular shimmer
 * Iteration 3: Voronoi cell cracking, anisotropic conductivity
 * ─────────────────────────────────────────────────────────────────────────────
 */
export class FerrumSimulation {
  constructor(canvas) {
    this.canvas    = canvas;
    this.ctx       = null;
    this.COLS      = 96;
    this.ROWS      = 72;
    this.grid      = null;  // Float32Array — temperature
    this.phase     = null;  // Uint8Array   — 0=solid, 1=liquid, 2=vapor
    this.metalHue  = null;  // Float32Array — per-cell hue jitter [0,1]
    this.running   = false;
    this.raf       = null;
    this.mouse     = { col: -1, row: -1, active: false };
    this.colorRamp = [];
    this.time      = 0;
    this._fps      = 60;
    this._frames   = 0;
    this._fpsTime  = 0;
    this.onStats   = null;
    this._onMouse  = this._onMouse.bind(this);
    this._onTouch  = this._onTouch.bind(this);
    this._onResize = this._onResize.bind(this);
    // Physics
    this.DIFFUSE    = 0.18;   // thermal conductivity
    this.COOL       = 0.9985; // ambient cooling factor
    this.MELT_T     = 0.35;
    this.VAPORIZE_T = 0.80;
    this.CONDENSE_T = 0.70;
    this.SOLIDIFY_T = 0.30;
  }

  init() {
    this.ctx = this.canvas.getContext('2d');
    if (!this.ctx) return false;
    this._resize();
    this._buildGrid();
    this._bindEvents();
    this.running = true;
    this._loop(0);
    return true;
  }

  setColorRamp(hexArray) { this.colorRamp = hexArray || []; }

  _buildGrid() {
    const { COLS, ROWS } = this;
    const N = COLS * ROWS;
    this.grid     = new Float32Array(N);
    this.nextGrid = new Float32Array(N);
    this.phase    = new Uint8Array(N);   // all solid initially
    this.metalHue = new Float32Array(N);

    // Initialize with strong hot bottom band — immediately visible from frame 1
    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        const idx = r * COLS + c;
        // Bottom 40%: very hot (T > MELT_T → liquid on first step)
        // Middle 20%: warm solid (visible as heated metal)
        // Top 40%: cold (dark steel)
        if (r > ROWS * 0.60) {
          this.grid[idx] = 0.65 + Math.random() * 0.30;  // 0.65–0.95: liquid/vapor
        } else if (r > ROWS * 0.40) {
          this.grid[idx] = 0.18 + Math.random() * 0.18;  // 0.18–0.36: warm solid
        } else {
          this.grid[idx] = Math.random() * 0.08;          // 0–0.08: cold steel
        }
        this.phase[idx] = 0;
        // Metallic hue jitter: small random offset for specular shimmer
        this.metalHue[idx] = Math.random() * 0.15 + (c % 3 === 0 ? 0.05 : 0);
      }
    }
  }

  // ── Physics step ──────────────────────────────────────────────────────────
  _step() {
    const { COLS, ROWS, DIFFUSE, COOL, grid, nextGrid, phase, metalHue } = this;

    // 1. Thermal diffusion (Laplacian, Neumann boundary: insulated edges)
    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        const idx = r * COLS + c;
        const T   = grid[idx];
        const n   = r > 0       ? grid[(r-1)*COLS+c] : T;
        const s   = r < ROWS-1  ? grid[(r+1)*COLS+c] : T;
        const e   = c < COLS-1  ? grid[r*COLS+c+1]   : T;
        const w   = c > 0       ? grid[r*COLS+c-1]   : T;
        // Liquid conducts faster than solid
        const conductivity = phase[idx] >= 1 ? DIFFUSE * 1.4 : DIFFUSE;
        nextGrid[idx] = T + conductivity * (n + s + e + w - 4 * T);
      }
    }

    // 2. Ambient cooling (top row radiates more — open top)
    for (let c = 0; c < COLS; c++) {
      nextGrid[c] *= COOL * 0.995;  // extra cooling at top
    }

    // 3. Gravity: liquid flows downward (swap with lower cell if lower is cooler solid)
    for (let r = ROWS - 2; r >= 0; r--) {
      for (let c = 0; c < COLS; c++) {
        const idx = r * COLS + c;
        const below = (r + 1) * COLS + c;
        if (phase[idx] === 1 && phase[below] === 0 && nextGrid[below] < this.MELT_T) {
          // Swap temperatures (liquid flows down into solid)
          if (Math.random() < 0.6) {
            [nextGrid[idx], nextGrid[below]] = [nextGrid[below], nextGrid[idx]];
            [metalHue[idx], metalHue[below]] = [metalHue[below], metalHue[idx]];
          }
        }
      }
    }

    // 3b. Persistent forge floor — keeps bottom always molten (critical for continuous activity)
    const BOTTOM = ROWS - 1;
    for (let c = 0; c < COLS; c++) {
      // Bottom row: forge floor — always white-hot
      nextGrid[BOTTOM * COLS + c] = Math.max(nextGrid[BOTTOM * COLS + c],
        0.82 + Math.random() * 0.18);
      // Second row: lava pool
      nextGrid[(BOTTOM - 1) * COLS + c] = Math.max(nextGrid[(BOTTOM - 1) * COLS + c],
        0.65 + Math.random() * 0.22);
      // Third row: transition zone
      if (Math.random() < 0.5) {
        nextGrid[(BOTTOM - 2) * COLS + c] = Math.max(nextGrid[(BOTTOM - 2) * COLS + c],
          0.40 + Math.random() * 0.20);
      }
    }

    // 4. Mouse heat source
    if (this.mouse.active && this.mouse.col >= 0) {
      const { col: mc, row: mr } = this.mouse;
      const RAD = 5;
      for (let dr = -RAD; dr <= RAD; dr++) {
        for (let dc = -RAD; dc <= RAD; dc++) {
          const r2 = mr + dr, c2 = mc + dc;
          if (r2 < 0 || r2 >= ROWS || c2 < 0 || c2 >= COLS) continue;
          const d = Math.hypot(dr, dc);
          if (d > RAD) continue;
          const idx = r2 * COLS + c2;
          nextGrid[idx] = Math.min(1.0, nextGrid[idx] + (1 - d/RAD) * 0.08);
        }
      }
    }

    // 5. Phase transitions + clamp
    for (let i = 0; i < COLS * ROWS; i++) {
      nextGrid[i] = Math.max(0, Math.min(1, nextGrid[i]));
      const T = nextGrid[i];
      if      (phase[i] === 0 && T > this.MELT_T)     phase[i] = 1;
      else if (phase[i] === 1 && T > this.VAPORIZE_T) phase[i] = 2;
      else if (phase[i] === 2 && T < this.CONDENSE_T) phase[i] = 1;
      else if (phase[i] === 1 && T < this.SOLIDIFY_T) phase[i] = 0;
    }

    // Swap buffers
    grid.set(nextGrid);
  }

  // ── Rendering (ImageData — pixel per cell) ────────────────────────────────
  _draw() {
    const { ctx, canvas, COLS, ROWS, grid, phase, metalHue, colorRamp } = this;
    const CW = canvas.width  / COLS;
    const CH = canvas.height / ROWS;
    const iw = Math.ceil(CW), ih = Math.ceil(CH);

    ctx.clearRect(0, 0, canvas.width, canvas.height);

    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        const idx = r * COLS + c;
        const T   = grid[idx];
        const ph  = phase[idx];
        const hue = metalHue[idx];

        let color;
        if (ph === 0) {
          // Solid metal: grey with hue jitter for metallic shimmer
          color = this._metalColor(T, hue, colorRamp);
        } else if (ph === 1) {
          // Liquid: fire ramp
          const rampIdx = Math.floor(T * (colorRamp.length - 1));
          color = colorRamp[Math.min(rampIdx, colorRamp.length - 1)] ?? '#c41400';
        } else {
          // Vapor: bright end of ramp
          const rampIdx = Math.floor(0.85 + T * 0.15) * (colorRamp.length - 1) | 0;
          color = colorRamp[Math.min(rampIdx, colorRamp.length - 1)] ?? '#ffd000';
        }

        // Always render metal — cold solid cells show steel grey (not near-black)
        // alpha: cold solid slightly transparent (shows layering), hot liquid/vapor opaque
        const alpha = ph === 0 ? 0.65 + T * 0.30 : 0.80 + T * 0.20;
        ctx.globalAlpha = alpha;
        ctx.fillStyle   = color;

        ctx.fillRect(
          Math.floor(c * CW), Math.floor(r * CH),
          iw + 1, ih + 1,  // +1 to avoid sub-pixel gaps
        );
        ctx.globalAlpha = 1;
      }
    }
  }

  _metalColor(T, hue, ramp) {
    // Cool metal: steel grey — brightened so cells are visible against dark bg
    // Warming metal: picks up ember/fire color from ramp
    if (T < 0.10) {
      // Cold steel: mid grey with metallic hue jitter (was too dark to see)
      const g = Math.floor(48 + hue * 60 + T * 80);
      return `rgb(${g},${g+4},${g+8})`;
    }
    if (T < 0.25) {
      // Warming steel: grey → dark ember
      const g  = Math.floor(65 + hue * 40 + T * 100);
      const r2 = Math.floor(g + T * 80);
      return `rgb(${r2},${g},${Math.floor(g * 0.85)})`;
    }
    // Hot metal near melt: fire ramp (lower half)
    const rampIdx = Math.floor(T * 0.60 * (ramp.length - 1));
    return ramp[Math.min(rampIdx, ramp.length - 1)] ?? '#c41400';
  }

  // ── Main loop ─────────────────────────────────────────────────────────────
  _loop(ts) {
    if (!this.running) return;
    this.time += 0.016;
    this._frames++;
    if (ts - this._fpsTime > 800) {
      this._fps = Math.round(this._frames * 1000 / (ts - this._fpsTime || 1));
      this._frames = 0; this._fpsTime = ts;
      let hot = 0, total = 0;
      for (let i = 0; i < this.grid.length; i++) {
        total += this.grid[i];
        if (this.phase[i] >= 1) hot++;
      }
      const avgTemp   = total / this.grid.length;
      const moltFrac  = hot / this.grid.length;
      if (this.onStats) this.onStats({ fps: this._fps, avgTemp, moltFrac });
    }
    this._step();
    this._draw();
    this.raf = requestAnimationFrame(t => this._loop(t));
  }

  _bindEvents() {
    const c = this.canvas;
    c.addEventListener('mousemove',  this._onMouse);
    c.addEventListener('mouseenter', () => { this.mouse.active = true; });
    c.addEventListener('mouseleave', () => { this.mouse.active = false; });
    c.addEventListener('touchmove',  this._onTouch, { passive: true });
    c.addEventListener('touchstart', this._onTouch, { passive: true });
    window.addEventListener('resize', this._onResize);
  }
  _onMouse(e) {
    const r = this.canvas.getBoundingClientRect();
    const dpr = devicePixelRatio || 1;
    const px = (e.clientX - r.left) * dpr;
    const py = (e.clientY - r.top)  * dpr;
    this.mouse.col = Math.floor(px / this.canvas.width  * this.COLS);
    this.mouse.row = Math.floor(py / this.canvas.height * this.ROWS);
    this.mouse.active = true;
  }
  _onTouch(e) { if (e.touches[0]) this._onMouse(e.touches[0]); }
  _onResize() { this._resize(); }
  _resize() {
    const dpr = Math.min(devicePixelRatio, 2);
    this.canvas.width  = this.canvas.clientWidth  * dpr;
    this.canvas.height = this.canvas.clientHeight * dpr;
  }

  destroy() {
    this.running = false;
    if (this.raf) cancelAnimationFrame(this.raf);
    const c = this.canvas;
    c.removeEventListener('mousemove', this._onMouse);
    c.removeEventListener('touchmove', this._onTouch);
    window.removeEventListener('resize', this._onResize);
  }
}
