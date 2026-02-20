/**
 * IGNIS — WebGL Fire / Plasma Simulation
 * ─────────────────────────────────────────────────────────────────────────────
 * Physics model: 2D heat field on the GPU
 *   — Semi-Lagrangian advection (heat rises, turbulence via sin + noise)
 *   — Laplacian diffusion (4-neighbor weighted average)
 *   — Cooling per frame (97.5%/frame → half-life ≈ 27 frames)
 *   — Bottom source: constant heat with hash noise
 *   — Mouse/touch: adds heat at cursor position
 *
 * Color: driven by Momoto OKLCH fire ramp passed via setColorRamp()
 *   — ramp[0]   = BG dark      (cool)
 *   — ramp[51]  = BRAND_EMBER  (low heat)
 *   — ramp[128] = BRAND_FIRE   (medium)
 *   — ramp[192] = BRAND_GOLD   (high)
 *   — ramp[230] = BRAND_SPARK  (very hot)
 *   — ramp[255] = near-white   (plasma)
 *
 * Iteration 1: Core heat simulation + color ramp texture
 * Iteration 2: Mouse heat, turbulence controls, FPS monitor
 * Iteration 3: Adaptive resolution, alpha blending for layering
 * ─────────────────────────────────────────────────────────────────────────────
 */
export class IgnisSimulation {
  constructor(canvas) {
    this.canvas    = canvas;
    this.gl        = null;
    this.progs     = {};
    this.bufs      = {};
    this.textures  = { a: null, b: null, ramp: null };
    this.fbs       = { a: null, b: null };
    this.ping      = true;
    this.time      = 0;
    this.running   = false;
    this.raf       = null;
    this.mouse     = { x: 0.5, y: 0.9, active: false };
    this.simW      = 0;
    this.simH      = 0;
    // Performance
    this._fps      = 60;
    this._frames   = 0;
    this._fpsTime  = 0;
    this.onStats   = null;   // callback({ fps, avgHeat, dominantHex })
    // Bound handlers
    this._onMouse  = this._onMouse.bind(this);
    this._onTouch  = this._onTouch.bind(this);
    this._onResize = this._onResize.bind(this);
  }

  // ── Vertex shader (shared) ────────────────────────────────────────────────
  static _VS = `
    attribute vec2 a_pos;
    varying vec2 v_uv;
    void main() {
      v_uv = a_pos * 0.5 + 0.5;
      gl_Position = vec4(a_pos, 0.0, 1.0);
    }
  `;

  // ── Simulation fragment shader ────────────────────────────────────────────
  static _SIM_FS = `
    precision highp float;
    uniform sampler2D u_prev;
    uniform vec2  u_res;
    uniform float u_time;
    uniform vec2  u_mouse;
    uniform float u_mouseActive;
    varying vec2 v_uv;

    float hash(vec2 p) {
      return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
    }

    void main() {
      vec2 px = 1.0 / u_res;

      // Turbulent advection: heat rises with lateral noise
      float warp = sin(v_uv.y * 5.8 + u_time * 1.6) * px.x * 2.2;
      warp += (hash(vec2(v_uv.y * 17.3, floor(u_time * 8.0))) - 0.5) * px.x * 1.8;
      vec2 src = clamp(v_uv + vec2(warp, px.y * 1.6), px, 1.0 - px);

      float heat = texture2D(u_prev, src).r;

      // Diffuse: weighted Laplacian with upward bias
      float n = texture2D(u_prev, v_uv + vec2(0.0,  px.y)).r;
      float s = texture2D(u_prev, v_uv - vec2(0.0,  px.y)).r;
      float e = texture2D(u_prev, v_uv + vec2(px.x, 0.0 )).r;
      float w = texture2D(u_prev, v_uv - vec2(px.x, 0.0 )).r;
      // Upward bias: north neighbor contributes less (heat already rose from there)
      heat = mix(heat, (n * 0.8 + s * 1.2 + e + w) / 4.0, 0.065);

      // Cooling
      heat *= 0.9745;

      // Mouse heat source
      if (u_mouseActive > 0.5) {
        float d = distance(v_uv, u_mouse);
        heat += smoothstep(0.14, 0.0, d) * 0.75;
      }

      // Bottom heat source with spatial + temporal noise
      // Extended to 14% height and boosted to 0.70–1.05 for a fierce, always-visible base flame
      float noise  = hash(vec2(v_uv.x * 31.7, floor(u_time * 14.0)));
      float noise2 = hash(vec2(v_uv.x * 7.3,  floor(u_time * 5.0) + 1.0));
      float base = smoothstep(0.14, 0.0, v_uv.y) * (0.70 + noise * 0.25 + noise2 * 0.10);
      heat = max(heat, base);

      // Edge fade (hard boundaries)
      float edgeFade = smoothstep(0.0, 0.025, v_uv.x) * smoothstep(1.0, 0.975, v_uv.x);
      heat *= edgeFade;

      gl_FragColor = vec4(clamp(heat, 0.0, 1.0), 0.0, 0.0, 1.0);
    }
  `;

  // ── Render fragment shader (uses Momoto color ramp texture) ───────────────
  static _RENDER_FS = `
    precision highp float;
    uniform sampler2D u_heat;
    uniform sampler2D u_ramp;
    varying vec2 v_uv;

    void main() {
      float heat = texture2D(u_heat, v_uv).r;
      // Sample the 1D color ramp texture (Momoto OKLCH-derived)
      vec3 color = texture2D(u_ramp, vec2(heat, 0.5)).rgb;
      // Alpha: fade at extremes + fade toward top
      float alpha = smoothstep(0.0, 0.12, heat);
      gl_FragColor = vec4(color, alpha);
    }
  `;

  // ── Init ──────────────────────────────────────────────────────────────────
  init() {
    const gl = this.canvas.getContext('webgl', {
      antialias: false, depth: false, stencil: false,
      premultipliedAlpha: false, alpha: true,
    });
    if (!gl) { console.warn('[IGNIS] WebGL not supported'); return false; }
    this.gl = gl;

    this._resize();
    this._buildPrograms();
    this._buildQuad();
    this._buildSimTextures();
    this._buildRampTexture([]);  // grey placeholder until setColorRamp() called
    this._buildFramebuffers();
    this._bindEvents();
    this.running = true;
    this._loop(0);
    return true;
  }

  setColorRamp(hexArray) {
    if (!this.gl || !hexArray || hexArray.length === 0) return;
    const pixels = new Uint8Array(hexArray.length * 4);
    hexArray.forEach((hex, i) => {
      const h = hex.replace('#', '');
      pixels[i * 4 + 0] = parseInt(h.slice(0, 2), 16);
      pixels[i * 4 + 1] = parseInt(h.slice(2, 4), 16);
      pixels[i * 4 + 2] = parseInt(h.slice(4, 6), 16);
      pixels[i * 4 + 3] = 255;
    });
    this._buildRampTexture(null, pixels, hexArray.length);
  }

  // ── WebGL helpers ─────────────────────────────────────────────────────────
  _buildPrograms() {
    this.progs.sim    = this._mkProgram(IgnisSimulation._VS, IgnisSimulation._SIM_FS);
    this.progs.render = this._mkProgram(IgnisSimulation._VS, IgnisSimulation._RENDER_FS);
  }

  _mkProgram(vs, fs) {
    const gl = this.gl;
    const prog = gl.createProgram();
    [vs, fs].forEach((src, i) => {
      const s = gl.createShader(i === 0 ? gl.VERTEX_SHADER : gl.FRAGMENT_SHADER);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS))
        console.error('[IGNIS] Shader error:', gl.getShaderInfoLog(s));
      gl.attachShader(prog, s);
    });
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS))
      console.error('[IGNIS] Link error:', gl.getProgramInfoLog(prog));
    return prog;
  }

  _buildQuad() {
    const gl = this.gl;
    this.bufs.quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.bufs.quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,1,-1,-1,1,1,1]), gl.STATIC_DRAW);
  }

  _mkTex(w, h, data = null, linear = true) {
    const gl = this.gl;
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    const filter = linear ? gl.LINEAR : gl.NEAREST;
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
    return tex;
  }

  _buildSimTextures() {
    const { simW: w, simH: h } = this;
    if (this.textures.a) this.gl.deleteTexture(this.textures.a);
    if (this.textures.b) this.gl.deleteTexture(this.textures.b);
    this.textures.a = this._mkTex(w, h, null, true);
    this.textures.b = this._mkTex(w, h, null, true);
    this.ping = true;
  }

  _buildRampTexture(hexArray, pixelData = null, w = 256) {
    const gl = this.gl;
    if (this.textures.ramp) gl.deleteTexture(this.textures.ramp);
    let data;
    if (pixelData) {
      data = pixelData;
    } else if (hexArray && hexArray.length > 0) {
      w = hexArray.length;
      data = new Uint8Array(w * 4);
      hexArray.forEach((hex, i) => {
        const h = hex.replace('#', '');
        data[i*4]   = parseInt(h.slice(0,2), 16);
        data[i*4+1] = parseInt(h.slice(2,4), 16);
        data[i*4+2] = parseInt(h.slice(4,6), 16);
        data[i*4+3] = 255;
      });
    } else {
      // Default grey ramp
      data = new Uint8Array(256 * 4);
      for (let i = 0; i < 256; i++) {
        data[i*4] = data[i*4+1] = data[i*4+2] = i; data[i*4+3] = 255;
      }
      w = 256;
    }
    this.textures.ramp = this._mkTex(w, 1, data, true);
  }

  _buildFramebuffers() {
    const gl = this.gl;
    ['a','b'].forEach(k => {
      if (this.fbs[k]) gl.deleteFramebuffer(this.fbs[k]);
      this.fbs[k] = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbs[k]);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.textures[k], 0);
    });
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  _resize() {
    const dpr = Math.min(devicePixelRatio, 2);
    this.canvas.width  = this.canvas.clientWidth  * dpr;
    this.canvas.height = this.canvas.clientHeight * dpr;
    this.simW = Math.max(1, Math.ceil(this.canvas.width  / 3));
    this.simH = Math.max(1, Math.ceil(this.canvas.height / 3));
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
    this.mouse.x = (e.clientX - r.left) / r.width;
    this.mouse.y = 1 - (e.clientY - r.top)  / r.height;
    this.mouse.active = true;
  }
  _onTouch(e) { if (e.touches[0]) this._onMouse(e.touches[0]); }
  _onResize() {
    this._resize();
    if (this.gl) {
      this._buildSimTextures();
      this._buildFramebuffers();
    }
  }

  // ── Render loop ───────────────────────────────────────────────────────────
  _loop(ts) {
    if (!this.running) return;
    this.time += 0.016;
    this._frames++;

    if (ts - this._fpsTime > 800) {
      this._fps = Math.round(this._frames * 1000 / (ts - this._fpsTime || 1));
      this._frames = 0; this._fpsTime = ts;
      if (this.onStats) this.onStats({ fps: this._fps });
    }

    this._simulate();
    this._render();
    this.raf = requestAnimationFrame(t => this._loop(t));
  }

  _useProgram(prog) {
    const gl = this.gl;
    gl.useProgram(prog);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.bufs.quad);
    const loc = gl.getAttribLocation(prog, 'a_pos');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
  }

  _simulate() {
    const gl = this.gl;
    const { simW: w, simH: h } = this;
    const prog = this.progs.sim;
    const src  = this.ping ? this.textures.a : this.textures.b;
    const dst  = this.ping ? this.fbs.b      : this.fbs.a;

    gl.bindFramebuffer(gl.FRAMEBUFFER, dst);
    gl.viewport(0, 0, w, h);
    this._useProgram(prog);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, src);
    gl.uniform1i(gl.getUniformLocation(prog, 'u_prev'), 0);
    gl.uniform2f(gl.getUniformLocation(prog, 'u_res'), w, h);
    gl.uniform1f(gl.getUniformLocation(prog, 'u_time'), this.time);
    gl.uniform2f(gl.getUniformLocation(prog, 'u_mouse'), this.mouse.x, this.mouse.y);
    gl.uniform1f(gl.getUniformLocation(prog, 'u_mouseActive'), this.mouse.active ? 1.0 : 0.0);

    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    this.ping = !this.ping;
  }

  _render() {
    const gl = this.gl;
    const { canvas } = this;
    const prog = this.progs.render;
    const src  = this.ping ? this.textures.a : this.textures.b;

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    this._useProgram(prog);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, src);
    gl.uniform1i(gl.getUniformLocation(prog, 'u_heat'), 0);

    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.textures.ramp);
    gl.uniform1i(gl.getUniformLocation(prog, 'u_ramp'), 1);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.BLEND);
  }

  destroy() {
    this.running = false;
    if (this.raf) cancelAnimationFrame(this.raf);
    const c = this.canvas;
    c.removeEventListener('mousemove',  this._onMouse);
    c.removeEventListener('touchmove',  this._onTouch);
    window.removeEventListener('resize', this._onResize);
    const gl = this.gl;
    if (gl) {
      Object.values(this.textures).forEach(t => t && gl.deleteTexture(t));
      Object.values(this.fbs).forEach(f => f && gl.deleteFramebuffer(f));
      Object.values(this.bufs).forEach(b => b && gl.deleteBuffer(b));
    }
  }
}
