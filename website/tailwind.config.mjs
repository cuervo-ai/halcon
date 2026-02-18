/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{astro,html,js,jsx,md,mdx,ts,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      /* ── Halcón Fire Color Palette ────────────────────────────── */
      colors: {
        fire: {
          primary:  '#e85200',   /* oklch(62% 0.22 38)  — CTA        */
          bright:   '#ff6200',   /* oklch(68% 0.23 40)  — hover      */
          deep:     '#cc4800',   /* oklch(56% 0.21 36)  — active     */
        },
        ember: {
          DEFAULT:  '#c41400',   /* oklch(40% 0.20 22)  — accent     */
          light:    '#e01800',   /* oklch(46% 0.22 24)  — hover      */
          dark:     '#8b0e00',   /* oklch(28% 0.18 20)  — deep       */
        },
        gold: {
          DEFAULT:  '#f5a000',   /* oklch(80% 0.19 65)  — highlight  */
          bright:   '#ffb800',   /* oklch(85% 0.19 72)  — hover      */
          dim:      '#c47800',   /* oklch(62% 0.16 60)  — muted      */
        },
        spark:      '#ffd000',   /* oklch(88% 0.17 78)  — hottest    */
        halcon: {
          bg:       '#070401',   /* oklch( 4% 0.01 30)  — base       */
          surface:  '#110803',   /* oklch( 8% 0.02 30)  — card       */
          surface2: '#1a0d04',   /* oklch(12% 0.03 30)  — elevated   */
          code:     '#0d0500',   /* oklch( 6% 0.02 28)  — terminal   */
          t1:       '#f0e8d8',   /* oklch(94% 0.012 50) — text 1     */
          t2:       '#c4b49a',   /* oklch(74% 0.030 50) — text 2     */
          t3:       '#8a7464',   /* oklch(52% 0.040 50) — muted      */
          t4:       '#4a3820',   /* oklch(30% 0.040 50) — dim        */
        },
      },

      /* ── Fonts ────────────────────────────────────────────────── */
      fontFamily: {
        display: ['Rajdhani', 'Montserrat', 'system-ui', 'sans-serif'],
        sans:    ['Montserrat', 'system-ui', 'sans-serif'],
        mono:    ['JetBrains Mono', 'Fira Code', 'monospace'],
      },

      /* ── Gradients ────────────────────────────────────────────── */
      backgroundImage: {
        'fire-gradient':   'linear-gradient(135deg, #e85200 0%, #c41400 100%)',
        'ember-gradient':  'linear-gradient(135deg, #c41400 0%, #8b0e00 100%)',
        'gold-gradient':   'linear-gradient(135deg, #ffd000 0%, #f5a000 100%)',
        'hero-gradient':   'linear-gradient(135deg, #ffd000 0%, #f5a000 25%, #e85200 58%, #c41400 100%)',
        'dark-gradient':   'linear-gradient(180deg, #070401 0%, #110803 100%)',
        'radial-fire':     'radial-gradient(ellipse at center, rgba(232,82,0,0.15) 0%, transparent 70%)',
        'radial-ember':    'radial-gradient(ellipse at center, rgba(196,20,0,0.12) 0%, transparent 70%)',
        'mesh-fire':       'radial-gradient(at 40% 20%, rgba(232,82,0,0.10) 0px, transparent 50%), radial-gradient(at 80% 80%, rgba(196,20,0,0.08) 0px, transparent 50%)',
      },

      /* ── Shadows — fire glows ─────────────────────────────────── */
      boxShadow: {
        'glow-fire-sm': '0 0 15px rgba(232,82,0,0.25), 0 0 30px rgba(232,82,0,0.10)',
        'glow-fire-md': '0 0 30px rgba(232,82,0,0.35), 0 0 60px rgba(232,82,0,0.15)',
        'glow-fire-lg': '0 0 60px rgba(232,82,0,0.45), 0 0 120px rgba(232,82,0,0.20)',
        'glow-gold-sm': '0 0 15px rgba(245,160,0,0.25), 0 0 30px rgba(245,160,0,0.10)',
        'glow-gold-md': '0 0 30px rgba(245,160,0,0.35), 0 0 60px rgba(245,160,0,0.15)',
        'glow-ember':   '0 0 40px rgba(196,20,0,0.30), 0 0 80px rgba(196,20,0,0.12)',
        'inner-fire':   'inset 0 0 30px rgba(232,82,0,0.12)',
        'glass':        '0 8px 32px rgba(0,0,0,0.40), 0 1px 0 rgba(255,255,255,0.04)',
      },

      /* ── Animations ───────────────────────────────────────────── */
      animation: {
        'gradient-shift': 'gradient-shift 5s ease infinite',
        'glow-pulse':     'glow-pulse 2.5s ease-in-out infinite',
        'gold-pulse':     'gold-pulse 2.5s ease-in-out infinite',
        'fade-in-up':     'fade-in-up 0.65s ease both',
        'scale-in':       'scale-in 0.5s ease both',
        'shimmer':        'shimmer 2s ease-in-out infinite',
        'float':          'float 3.2s ease-in-out infinite',
        'pulse-glow':     'pulse-glow 2s ease-in-out infinite',
        'logo-appear':    'logo-appear 1.3s cubic-bezier(0.4,0,0.2,1) both',
        'spark-rise':     'spark-rise 3.5s ease-out infinite',
      },

      keyframes: {
        'gradient-shift': {
          '0%, 100%': { backgroundPosition: '0% 50%' },
          '50%':       { backgroundPosition: '100% 50%' },
        },
        'glow-pulse': {
          '0%, 100%': { boxShadow: '0 0 20px rgba(232,82,0,0.30), 0 0 40px rgba(232,82,0,0.10)' },
          '50%':       { boxShadow: '0 0 40px rgba(232,82,0,0.55), 0 0 80px rgba(232,82,0,0.25)' },
        },
        'gold-pulse': {
          '0%, 100%': { boxShadow: '0 0 20px rgba(245,160,0,0.25)' },
          '50%':       { boxShadow: '0 0 50px rgba(245,160,0,0.50), 0 0 100px rgba(245,160,0,0.20)' },
        },
        'fade-in-up': {
          from: { opacity: '0', transform: 'translateY(28px)' },
          to:   { opacity: '1', transform: 'translateY(0)' },
        },
        'shimmer': {
          '0%':   { transform: 'translateX(-120%) skewX(-20deg)' },
          '100%': { transform: 'translateX(300%) skewX(-20deg)' },
        },
        'float': {
          '0%, 100%': { transform: 'translateY(0)' },
          '50%':       { transform: 'translateY(-10px)' },
        },
        'scale-in': {
          from: { opacity: '0', transform: 'scale(0.88)' },
          to:   { opacity: '1', transform: 'scale(1)' },
        },
        'pulse-glow': {
          '0%, 100%': { opacity: '0.45' },
          '50%':       { opacity: '1.00' },
        },
        'logo-appear': {
          '0%':   { opacity: '0', transform: 'scale(0.75) translateY(16px)', filter: 'brightness(0.3) blur(4px)' },
          '60%':  { filter: 'brightness(1.4) blur(0px)' },
          '100%': { opacity: '1', transform: 'scale(1) translateY(0)', filter: 'brightness(1) blur(0px)' },
        },
        'spark-rise': {
          '0%':   { opacity: '0', transform: 'translateY(0) scale(1)' },
          '8%':   { opacity: '0.85' },
          '85%':  { opacity: '0.40' },
          '100%': { opacity: '0', transform: 'translateY(-160px) scale(0.15)' },
        },
      },
    },
  },
  plugins: [],
};
