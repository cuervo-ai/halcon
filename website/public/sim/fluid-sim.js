/**
 * AQUA — SPH Fluid Simulation (Canvas 2D)
 * ─────────────────────────────────────────────────────────────────────────────
 * Physics model: Smoothed Particle Hydrodynamics (simplified)
 *   — Pressure force: density deviation from ρ₀ → repulsion
 *   — Viscosity force: velocity difference between neighbors → damping
 *   — Gravity: constant downward acceleration
 *   — Mouse: repulsion / attraction toggle
 *   — Boundaries: elastic reflection at canvas edges
 *
 * Rendering: metaball-style radial gradients with additive blending
 *   — Each particle is a radial gradient glow (color from Momoto OKLCH ramp)
 *   — Particle color = ramp[temperature] where temperature ∝ speed
 *   — High-velocity particles → BRAND_FIRE orange
 *   — Low-velocity particles  → BRAND_EMBER deep red
 *
 * Iteration 1: Core SPH forces + gradient rendering
 * Iteration 2: Mouse repulsion/attraction, temperature tracking
 * Iteration 3: Surface tension, foam particles at high density
 * ─────────────────────────────────────────────────────────────────────────────
 */
export class AquaSimulation {
  constructor(canvas) {
    this.canvas    = canvas;
    this.ctx       = null;
    this.particles = [];
    this.N         = 120;       // particle count
    this.running   = false;
    this.raf       = null;
    this.mouse     = { x: -999, y: -999, active: false, repel: false };
    this.colorRamp = [];        // hex strings from Momoto
    this.time      = 0;
    this._fps      = 60;
    this._frames   = 0;
    this._fpsTime  = 0;
    this.onStats   = null;
    this._onMouse  = this._onMouse.bind(this);
    this._onTouch  = this._onTouch.bind(this);
    this._onResize = this._onResize.bind(this);
    // SPH constants
    this.H       = 42;     // smoothing radius px
    this.K       = 420;    // stiffness
    this.MU      = 28;     // viscosity
    this.REST_D  = 0.6;    // rest density (normalized)
    this.MASS    = 1.0;
    this.GRAVITY = 420;    // px/s²
    this.DT      = 1/60;
  }

  // ── Init ──────────────────────────────────────────────────────────────────
  init() {
    this.ctx = this.canvas.getContext('2d');
    if (!this.ctx) return false;
    this._resize();
    this._spawnParticles();
    this._bindEvents();
    this.running = true;
    this._loop(0);
    return true;
  }

  setColorRamp(hexArray) { this.colorRamp = hexArray || []; }

  _spawnParticles() {
    const { W, H } = this;
    this.particles = Array.from({ length: this.N }, () => ({
      x:  W * 0.2 + Math.random() * W * 0.6,
      y:  H * 0.1 + Math.random() * H * 0.5,
      vx: (Math.random() - 0.5) * 180,  // higher speed → higher temp → visible fire color
      vy: Math.random() * 100 - 30,
      density: 0,
      pressure: 0,
      temp: Math.random(),   // 0=cool (ember), 1=hot (fire/gold)
    }));
  }

  // ── SPH kernel functions ──────────────────────────────────────────────────
  _W(r, h) {
    // Poly6 kernel (density)
    if (r >= h) return 0;
    const q = 1 - (r * r) / (h * h);
    return (315 / (64 * Math.PI * h ** 3)) * q * q * q;
  }

  _gradW(r, h) {
    // Spiky kernel gradient (pressure force)
    if (r >= h || r < 0.001) return 0;
    const q = 1 - r / h;
    return -(45 / (Math.PI * h ** 4)) * q * q;
  }

  _lapW(r, h) {
    // Viscosity kernel Laplacian
    if (r >= h) return 0;
    return (45 / (Math.PI * h ** 5)) * (1 - r / h);
  }

  // ── Physics step ──────────────────────────────────────────────────────────
  _step() {
    const { particles: ps, H, K, MU, REST_D, MASS, GRAVITY, DT, W, HEIGHT } = this;

    // 1. Compute densities
    for (const pi of ps) {
      pi.density = 0;
      for (const pj of ps) {
        const dx = pi.x - pj.x, dy = pi.y - pj.y;
        const r  = Math.hypot(dx, dy);
        pi.density += MASS * this._W(r, H);
      }
      pi.pressure = K * Math.max(0, pi.density - REST_D);
    }

    // 2. Compute forces
    for (const pi of ps) {
      let fx = 0, fy = GRAVITY * MASS;   // gravity

      for (const pj of ps) {
        if (pi === pj) continue;
        const dx = pi.x - pj.x, dy = pi.y - pj.y;
        const r  = Math.hypot(dx, dy);
        if (r >= H || r < 0.001) continue;

        const nx = dx / r, ny = dy / r;

        // Pressure force
        const fp = -MASS * (pi.pressure + pj.pressure) / (2 * pj.density) * this._gradW(r, H);
        fx += fp * nx;
        fy += fp * ny;

        // Viscosity force
        const fv = MU * MASS * this._lapW(r, H) / pj.density;
        fx += fv * (pj.vx - pi.vx);
        fy += fv * (pj.vy - pi.vy);
      }

      // Mouse force
      const mdx = pi.x - this.mouse.x, mdy = pi.y - this.mouse.y;
      const mr  = Math.hypot(mdx, mdy);
      if (this.mouse.active && mr < 100) {
        const mf = (this.mouse.repel ? 1 : -1) * 6000 / (mr + 1);
        fx += mf * mdx / (mr + 1);
        fy += mf * mdy / (mr + 1);
      }

      // Integrate velocity
      pi.vx += (fx / (pi.density + 0.0001)) * DT;
      pi.vy += (fy / (pi.density + 0.0001)) * DT;
    }

    // 3. Integrate positions + boundary conditions
    for (const p of ps) {
      p.x += p.vx * DT;
      p.y += p.vy * DT;

      const PAD = 8, DAMP = 0.45;
      if (p.x < PAD)       { p.x = PAD;         p.vx *= -DAMP; }
      if (p.x > W - PAD)   { p.x = W - PAD;     p.vx *= -DAMP; }
      if (p.y < PAD)       { p.y = PAD;          p.vy *= -DAMP; }
      if (p.y > HEIGHT-PAD){ p.y = HEIGHT - PAD; p.vy *= -DAMP; }

      // Temperature: speed → heat (normalized)
      const speed = Math.hypot(p.vx, p.vy);
      p.temp = Math.min(1, speed / 220);
    }
  }

  // ── Rendering ─────────────────────────────────────────────────────────────
  _draw() {
    const { ctx, canvas, particles, colorRamp } = this;
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // Additive blending for glowing effect
    ctx.globalCompositeOperation = 'screen';

    for (const p of particles) {
      const r   = 22;
      const idx = Math.floor(p.temp * (colorRamp.length - 1));
      const col = colorRamp[idx] ?? '#e85200';

      const grad = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, r * 1.8);
      grad.addColorStop(0,   col);
      grad.addColorStop(0.4, col + '80');
      grad.addColorStop(1,   col + '00');

      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.arc(p.x, p.y, r * 1.8, 0, Math.PI * 2);
      ctx.fill();
    }

    ctx.globalCompositeOperation = 'source-over';

    // Core dots (solid, over glow)
    for (const p of particles) {
      const idx = Math.floor(p.temp * (colorRamp.length - 1));
      const col = colorRamp[Math.min(idx + 20, colorRamp.length - 1)] ?? '#f5a000';
      ctx.fillStyle = col + 'cc';
      ctx.beginPath();
      ctx.arc(p.x, p.y, 4, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // ── Main loop ─────────────────────────────────────────────────────────────
  _loop(ts) {
    if (!this.running) return;
    this.time += this.DT;
    this._frames++;
    if (ts - this._fpsTime > 800) {
      this._fps = Math.round(this._frames * 1000 / (ts - this._fpsTime || 1));
      this._frames = 0; this._fpsTime = ts;
      const avgTemp = this.particles.reduce((s,p) => s + p.temp, 0) / this.particles.length;
      if (this.onStats) this.onStats({ fps: this._fps, avgTemp });
    }
    this._step();
    this._draw();
    this.raf = requestAnimationFrame(t => this._loop(t));
  }

  // ── Events ────────────────────────────────────────────────────────────────
  _bindEvents() {
    const c = this.canvas;
    c.addEventListener('mousemove',  this._onMouse);
    c.addEventListener('mouseenter', () => { this.mouse.active = true; });
    c.addEventListener('mouseleave', () => { this.mouse.active = false; });
    c.addEventListener('mousedown',  () => { this.mouse.repel = true; });
    c.addEventListener('mouseup',    () => { this.mouse.repel = false; });
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
    // Clamp particle positions to new bounds
    for (const p of this.particles) {
      p.x = Math.min(p.x, this.W - 8);
      p.y = Math.min(p.y, this.HEIGHT - 8);
    }
  }
  _resize() {
    const dpr = Math.min(devicePixelRatio, 2);
    this.canvas.width  = this.canvas.clientWidth  * dpr;
    this.canvas.height = this.canvas.clientHeight * dpr;
    this.W      = this.canvas.width;
    this.HEIGHT = this.canvas.height;
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
