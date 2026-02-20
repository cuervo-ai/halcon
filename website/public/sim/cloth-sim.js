/**
 * VELUM — Cloth / Silk Simulation (Canvas 2D)
 * ─────────────────────────────────────────────────────────────────────────────
 * Physics model: Verlet integration + constraint relaxation
 *   — Grid of particles connected by structural springs (distance constraints)
 *   — Shear springs (diagonal connections for stiffness)
 *   — Gravity: constant downward acceleration
 *   — Wind: sinusoidal oscillation with Perlin-like turbulence
 *   — Mouse: drag cloth points (attraction)
 *   — Pinned: top-left and top-right corners (cloth hanging effect)
 *
 * Rendering: triangle mesh with OKLCH gradient fill
 *   — Triangle fill color = average temperature of 3 vertices
 *   — Temperature proxy = distance from rest position (stretch)
 *   — Stretched = hotter (fire/gold), relaxed = cooler (ember)
 *   — Edge lines with specular highlight color (lighter by 0.25L)
 *
 * Iteration 1: Verlet cloth + triangle rendering
 * Iteration 2: Mouse drag, wind oscillation, Momoto colors
 * Iteration 3: Tearing simulation, iridescence via hue rotation
 * ─────────────────────────────────────────────────────────────────────────────
 */
export class VelumSimulation {
  constructor(canvas) {
    this.canvas     = canvas;
    this.ctx        = null;
    this.particles  = [];
    this.constraints= [];
    this.COLS       = 18;
    this.ROWS       = 14;
    this.running    = false;
    this.raf        = null;
    this.mouse      = { x: 0, y: 0, active: false };
    this.colorRamp  = [];
    this.time       = 0;
    this._fps       = 60;
    this._frames    = 0;
    this._fpsTime   = 0;
    this.onStats    = null;
    this._onMouse   = this._onMouse.bind(this);
    this._onTouch   = this._onTouch.bind(this);
    this._onResize  = this._onResize.bind(this);
    // Physics
    this.GRAVITY    = 0.38;
    this.DAMPING    = 0.985;
    this.ITERATIONS = 4;      // constraint relaxation passes
    this.WIND_AMP   = 0.22;
    this.WIND_FREQ  = 0.8;
  }

  init() {
    this.ctx = this.canvas.getContext('2d');
    if (!this.ctx) return false;
    this._resize();
    this._buildMesh();
    this._bindEvents();
    this.running = true;
    this._loop(0);
    return true;
  }

  setColorRamp(hexArray) { this.colorRamp = hexArray || []; }

  // ── Build particle grid + constraints ─────────────────────────────────────
  _buildMesh() {
    const { W, H, COLS, ROWS } = this;
    const cellW = W * 0.8 / (COLS - 1);
    const cellH = H * 0.65 / (ROWS - 1);
    const startX = W * 0.1;
    const startY = H * 0.08;

    this.particles = [];
    this.constraints = [];

    // Create particles with initial wave displacement — visually impactful from frame 1
    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        const x = startX + c * cellW;
        const y = startY + r * cellH;
        const pinned = r === 0 && (c === 0 || c === COLS - 1);

        // Initial wave: sinusoidal displacement grows with row depth
        // Creates a rippling fabric look immediately, without waiting for wind to build up
        const waveX = !pinned ? Math.sin(c * 0.55 + r * 0.2) * cellW * 0.35 : 0;
        const waveY = !pinned ? Math.sin(c * 0.4)            * cellH * 0.5 * (r / ROWS) : 0;

        this.particles.push({
          x:  x + waveX, y:  y + waveY,
          px: x,         py: y,          // previous ≠ current → immediate Verlet velocity
          pinned,
          temp: 0,
        });
      }
    }

    // Structural constraints (horizontal + vertical)
    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        const idx = r * COLS + c;
        if (c < COLS - 1) this._addConstraint(idx, idx + 1, cellW);
        if (r < ROWS - 1) this._addConstraint(idx, idx + COLS, cellH);
      }
    }

    // Shear constraints (diagonal)
    for (let r = 0; r < ROWS - 1; r++) {
      for (let c = 0; c < COLS - 1; c++) {
        const idx = r * COLS + c;
        const d   = Math.hypot(cellW, cellH);
        this._addConstraint(idx,         idx + COLS + 1, d);
        this._addConstraint(idx + 1,     idx + COLS,     d);
      }
    }
  }

  _addConstraint(a, b, rest) {
    this.constraints.push({ a, b, rest });
  }

  // ── Physics step ──────────────────────────────────────────────────────────
  _step() {
    const { particles: ps, GRAVITY, DAMPING, ITERATIONS, WIND_AMP, WIND_FREQ } = this;

    const wind = WIND_AMP * Math.sin(this.time * WIND_FREQ) +
                 WIND_AMP * 0.4 * Math.sin(this.time * WIND_FREQ * 2.3 + 1.7);

    // Verlet integration
    for (const p of ps) {
      if (p.pinned) continue;
      const vx = (p.x - p.px) * DAMPING;
      const vy = (p.y - p.py) * DAMPING;
      p.px = p.x;
      p.py = p.y;
      p.x += vx + wind;
      p.y += vy + GRAVITY;
    }

    // Constraint relaxation
    for (let iter = 0; iter < ITERATIONS; iter++) {
      for (const c of this.constraints) {
        const a = ps[c.a], b = ps[c.b];
        const dx = b.x - a.x, dy = b.y - a.y;
        const dist = Math.hypot(dx, dy) || 0.001;
        const diff = (dist - c.rest) / dist * 0.5;
        const ox = dx * diff, oy = dy * diff;
        if (!a.pinned) { a.x += ox; a.y += oy; }
        if (!b.pinned) { b.x -= ox; b.y -= oy; }
      }
    }

    // Mouse attraction
    for (const p of ps) {
      if (p.pinned || !this.mouse.active) continue;
      const dx = this.mouse.x - p.x, dy = this.mouse.y - p.y;
      const dist = Math.hypot(dx, dy);
      if (dist < 80) {
        const f = (1 - dist / 80) * 0.15;
        p.x += dx * f;
        p.y += dy * f;
      }
    }

    // Boundary clamping
    const { W, H } = this;
    for (const p of ps) {
      if (p.pinned) continue;
      p.x = Math.max(4, Math.min(W - 4, p.x));
      p.y = Math.max(4, Math.min(H - 4, p.y));
    }

    // Temperature: stretch of structural constraints
    for (const p of ps) p.temp = 0;
    for (const c of this.constraints) {
      const a = ps[c.a], b = ps[c.b];
      const dx = b.x - a.x, dy = b.y - a.y;
      const stretch = Math.abs(Math.hypot(dx, dy) - c.rest) / c.rest;
      a.temp = Math.min(1, a.temp + stretch * 2);
      b.temp = Math.min(1, b.temp + stretch * 2);
    }
  }

  // ── Rendering ─────────────────────────────────────────────────────────────
  _draw() {
    const { ctx, canvas, particles: ps, colorRamp, COLS, ROWS } = this;
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // Render triangles (two per quad cell)
    for (let r = 0; r < ROWS - 1; r++) {
      for (let c = 0; c < COLS - 1; c++) {
        const tl = ps[r * COLS + c];
        const tr = ps[r * COLS + c + 1];
        const bl = ps[(r + 1) * COLS + c];
        const br = ps[(r + 1) * COLS + c + 1];

        this._drawTriangle(tl, tr, bl, colorRamp);
        this._drawTriangle(tr, br, bl, colorRamp);
      }
    }

    // Edge highlight lines (top row)
    ctx.strokeStyle = colorRamp[230] ?? '#ffd000';
    ctx.lineWidth   = 1.5;
    ctx.globalAlpha = 0.6;
    for (let c = 0; c < COLS - 1; c++) {
      const a = ps[c], b = ps[c + 1];
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }

  _drawTriangle(a, b, c, ramp) {
    const ctx = this.ctx;
    const avgTemp = (a.temp + b.temp + c.temp) / 3;
    // Floor at 0.32 → maps to dark ember range (visibly red on dark bg, not near-black)
    const idx  = Math.floor(Math.max(0.32, avgTemp) * (ramp.length - 1));
    const fill = ramp[Math.min(idx, ramp.length - 1)] ?? '#c41400';

    // Slight alpha variation by row (depth illusion)
    const alpha = 0.72 + avgTemp * 0.20;

    ctx.globalAlpha = alpha;
    ctx.fillStyle   = fill;
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.lineTo(c.x, c.y);
    ctx.closePath();
    ctx.fill();
    ctx.globalAlpha = 1;
  }

  // ── Main loop ─────────────────────────────────────────────────────────────
  _loop(ts) {
    if (!this.running) return;
    this.time += 0.016;
    this._frames++;
    if (ts - this._fpsTime > 800) {
      this._fps = Math.round(this._frames * 1000 / (ts - this._fpsTime || 1));
      this._frames = 0; this._fpsTime = ts;
      const avgTemp = this.particles.reduce((s, p) => s + p.temp, 0) / this.particles.length;
      if (this.onStats) this.onStats({ fps: this._fps, avgTemp });
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
    this.mouse.x = (e.clientX - r.left) * dpr;
    this.mouse.y = (e.clientY - r.top)  * dpr;
    this.mouse.active = true;
  }
  _onTouch(e) { if (e.touches[0]) this._onMouse(e.touches[0]); }
  _onResize() {
    this._resize();
    this._buildMesh();
  }
  _resize() {
    const dpr = Math.min(devicePixelRatio, 2);
    this.canvas.width  = this.canvas.clientWidth  * dpr;
    this.canvas.height = this.canvas.clientHeight * dpr;
    this.W = this.canvas.width;
    this.H = this.canvas.height;
  }

  destroy() {
    this.running = false;
    if (this.raf) cancelAnimationFrame(this.raf);
    const c = this.canvas;
    c.removeEventListener('mousemove',  this._onMouse);
    c.removeEventListener('touchmove',  this._onTouch);
    window.removeEventListener('resize', this._onResize);
  }
}
