/**
 * ColorSciencePanel — Momoto WASM live demo component
 * Shows: brand palette tokens, state derivations, WCAG/APCA validation
 */

import { useState, useEffect } from 'react';
import type { StateHexTokens, ContrastReport } from '../../lib/momoto/index.ts';

interface PaletteEntry {
  name:       string;
  label:      string;
  desc:       string;
  l: number; c: number; h: number;
  tokens?:    StateHexTokens;
  contrast?:  ContrastReport;
  loading:    boolean;
}

const BRAND_COLORS: Omit<PaletteEntry, 'tokens' | 'contrast' | 'loading'>[] = [
  { name: 'fire',  label: 'Brand Fire',  desc: 'Primary CTA, #e85200',  l: 0.62, c: 0.22, h: 38 },
  { name: 'gold',  label: 'Brand Gold',  desc: 'Accent, #f5a000',       l: 0.80, c: 0.19, h: 65 },
  { name: 'ember', label: 'Brand Ember', desc: 'Deep accent, #c41400',  l: 0.40, c: 0.20, h: 22 },
];

const BG = { l: 0.04, c: 0.01, h: 30 }; // #070401 warm near-black

export default function ColorSciencePanel() {
  const [ready, setReady]   = useState(false);
  const [error, setError]   = useState<string | null>(null);
  const [palette, setPalette] = useState<PaletteEntry[]>(
    BRAND_COLORS.map(c => ({ ...c, loading: true }))
  );
  const [activeIdx, setActiveIdx] = useState(0);

  // Initialize momoto WASM and derive all tokens
  useEffect(() => {
    let cancelled = false;

    async function run() {
      try {
        const { initMomoto, deriveStateColors, checkContrast } = await import('../../lib/momoto/index.ts');
        await initMomoto();

        if (cancelled) return;
        setReady(true);

        // Derive tokens for each brand color in parallel
        const results = await Promise.all(
          BRAND_COLORS.map(async (color) => {
            const tokens   = await deriveStateColors(color.l, color.c, color.h);
            const contrast = await checkContrast({ l: color.l, c: color.c, h: color.h }, BG);
            return { tokens, contrast };
          })
        );

        if (cancelled) return;
        setPalette(BRAND_COLORS.map((c, i) => ({
          ...c,
          tokens:   results[i].tokens,
          contrast: results[i].contrast,
          loading:  false,
        })));
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    }

    run();
    return () => { cancelled = true; };
  }, []);

  const active = palette[activeIdx];

  if (error) {
    return (
      <div className="rounded-xl border border-red-500/20 bg-red-500/5 p-6 text-sm text-red-400 font-mono">
        momoto WASM error: {error}
      </div>
    );
  }

  return (
    <div className="rounded-2xl border overflow-hidden"
         style={{ background: 'rgba(7,4,1,0.92)', borderColor: 'rgba(200,80,0,0.18)' }}>

      {/* Header */}
      <div className="px-6 py-4 border-b flex items-center justify-between"
           style={{ borderColor: 'rgba(200,80,0,0.12)', background: 'rgba(17,8,3,0.70)' }}>
        <div className="flex items-center gap-3">
          {/* Momoto indicator */}
          <div className="flex items-center gap-1.5">
            <span className={`w-2 h-2 rounded-full ${ready ? 'bg-emerald-400 animate-pulse' : 'bg-amber-400'}`} />
            <span className="text-xs font-mono" style={{ color: 'rgba(196,180,154,0.8)' }}>
              {ready ? 'momoto WASM ready' : 'initializing WASM…'}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="text-xs px-2 py-0.5 rounded font-mono"
                style={{ background: 'rgba(232,82,0,0.15)', color: '#ff8040', border: '1px solid rgba(232,82,0,0.30)' }}>
            OKLCH
          </span>
          <span className="text-xs px-2 py-0.5 rounded font-mono"
                style={{ background: 'rgba(245,160,0,0.10)', color: '#f5a000', border: '1px solid rgba(245,160,0,0.25)' }}>
            WCAG 2.1
          </span>
          <span className="text-xs px-2 py-0.5 rounded font-mono"
                style={{ background: 'rgba(196,20,0,0.12)', color: '#e04020', border: '1px solid rgba(196,20,0,0.25)' }}>
            APCA
          </span>
        </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3">

        {/* Color selector */}
        <div className="border-r p-4 space-y-2" style={{ borderColor: 'rgba(200,80,0,0.10)' }}>
          <p className="text-xs uppercase tracking-wider mb-3 font-semibold" style={{ color: 'rgba(245,160,0,0.7)' }}>
            Brand Palette
          </p>
          {palette.map((entry, idx) => (
            <button
              key={entry.name}
              onClick={() => setActiveIdx(idx)}
              className="w-full text-left p-3 rounded-xl transition-all duration-200 border"
              style={{
                background:   idx === activeIdx ? 'rgba(232,82,0,0.12)' : 'rgba(17,8,3,0.50)',
                borderColor:  idx === activeIdx ? 'rgba(232,82,0,0.40)' : 'rgba(200,80,0,0.15)',
              }}
            >
              <div className="flex items-center gap-3">
                {/* Color swatch — idle state */}
                <div
                  className="w-8 h-8 rounded-lg flex-shrink-0 shadow-inner"
                  style={{
                    background: entry.tokens?.idle ?? `oklch(${entry.l} ${entry.c} ${entry.h})`,
                    boxShadow: idx === activeIdx ? '0 0 12px rgba(232,82,0,0.45)' : 'none',
                  }}
                />
                <div>
                  <div className="text-sm font-semibold" style={{ color: '#e2e8f0' }}>{entry.label}</div>
                  <div className="text-xs font-mono" style={{ color: '#64748b' }}>{entry.desc}</div>
                </div>
              </div>
              {/* WCAG badge */}
              {entry.contrast && !entry.loading && (
                <div className="mt-2 flex items-center gap-2">
                  <span className={`text-xs px-1.5 py-0.5 rounded font-bold font-mono ${
                    entry.contrast.level === 'aaa' ? 'text-emerald-300' :
                    entry.contrast.level === 'aa'  ? 'text-blue-300'   : 'text-red-300'
                  }`}>
                    WCAG {entry.contrast.level.toUpperCase()}
                  </span>
                  <span className="text-xs font-mono" style={{ color: '#64748b' }}>
                    {entry.contrast.wcagRatio.toFixed(2)}:1
                  </span>
                </div>
              )}
            </button>
          ))}
        </div>

        {/* Token states grid */}
        <div className="p-4 border-r" style={{ borderColor: 'rgba(200,80,0,0.10)' }}>
          <p className="text-xs uppercase tracking-wider mb-3 font-semibold" style={{ color: 'rgba(245,160,0,0.7)' }}>
            Derived State Tokens
          </p>

          {active.loading ? (
            <div className="space-y-2">
              {['idle','hover','active','focus','disabled','loading'].map(s => (
                <div key={s} className="h-10 rounded-lg animate-pulse" style={{ background: 'rgba(71,85,105,0.2)' }} />
              ))}
            </div>
          ) : active.tokens ? (
            <div className="space-y-2">
              {(Object.entries(active.tokens) as [string, string][]).map(([state, hex]) => (
                <div key={state}
                     className="flex items-center gap-3 p-2.5 rounded-lg border"
                     style={{ background: 'rgba(17,24,39,0.5)', borderColor: 'rgba(71,85,105,0.15)' }}>
                  <div className="w-6 h-6 rounded-md flex-shrink-0 border"
                       style={{ background: hex, borderColor: 'rgba(255,255,255,0.1)' }} />
                  <div className="flex-1 flex items-center justify-between">
                    <span className="text-xs font-mono capitalize" style={{ color: '#94a3b8' }}>{state}</span>
                    <span className="text-xs font-mono" style={{ color: '#67e8f9' }}>{hex}</span>
                  </div>
                </div>
              ))}
            </div>
          ) : null}

          {/* OKLCH values */}
          <div className="mt-4 p-3 rounded-lg font-mono text-xs space-y-1"
               style={{ background: 'rgba(4,2,0,0.70)', border: '1px solid rgba(200,80,0,0.12)' }}>
            <div style={{ color: '#4a3820' }}># Base OKLCH</div>
            <div style={{ color: '#ff8040' }}>
              oklch({active.l}&nbsp; {active.c}&nbsp; {active.h})
            </div>
            <div style={{ color: '#4a3820' }}># Engine: TokenDerivationEngine</div>
            <div style={{ color: '#f5a000' }}># ~0.02ms cache hit · 0.2ms miss</div>
          </div>
        </div>

        {/* Contrast validation */}
        <div className="p-4">
          <p className="text-xs uppercase tracking-wider mb-3 font-semibold" style={{ color: 'rgba(245,160,0,0.7)' }}>
            A11y Validation
          </p>

          {active.loading ? (
            <div className="space-y-3">
              {[1,2,3,4].map(i => (
                <div key={i} className="h-12 rounded-lg animate-pulse" style={{ background: 'rgba(71,85,105,0.2)' }} />
              ))}
            </div>
          ) : active.contrast ? (
            <div className="space-y-3">

              {/* WCAG ratio */}
              <div className="p-3 rounded-xl border"
                   style={{ background: 'rgba(17,24,39,0.5)', borderColor: 'rgba(71,85,105,0.15)' }}>
                <div className="flex items-center justify-between mb-2">
                  <span className="text-xs font-semibold" style={{ color: '#94a3b8' }}>WCAG 2.1 Contrast</span>
                  <span className={`text-xs font-bold font-mono px-2 py-0.5 rounded-full border ${
                    active.contrast.level === 'aaa' ? 'text-emerald-300 border-emerald-500/30 bg-emerald-500/10' :
                    active.contrast.level === 'aa'  ? 'text-blue-300 border-blue-500/30 bg-blue-500/10' :
                                                       'text-red-300 border-red-500/30 bg-red-500/10'
                  }`}>
                    {active.contrast.level.toUpperCase()}
                  </span>
                </div>
                <div className="flex items-end gap-2">
                  <span className="text-2xl font-bold font-mono" style={{ color: '#f1f5f9' }}>
                    {active.contrast.wcagRatio.toFixed(2)}
                  </span>
                  <span className="text-sm mb-0.5" style={{ color: '#64748b' }}>:1 ratio</span>
                </div>
                {/* Progress bar */}
                <div className="mt-2 h-1.5 rounded-full overflow-hidden" style={{ background: 'rgba(71,85,105,0.3)' }}>
                  <div className="h-full rounded-full transition-all duration-700"
                       style={{
                         width: `${Math.min(100, (active.contrast.wcagRatio / 21) * 100)}%`,
                         background: active.contrast.level === 'aaa' ? '#34d399' :
                                     active.contrast.level === 'aa'  ? '#60a5fa' : '#f87171',
                       }} />
                </div>
                <div className="flex justify-between mt-1 text-xs font-mono" style={{ color: '#475569' }}>
                  <span>1:1</span><span>AA 4.5</span><span>AAA 7.0</span><span>21:1</span>
                </div>
              </div>

              {/* APCA */}
              <div className="p-3 rounded-xl border"
                   style={{ background: 'rgba(17,24,39,0.5)', borderColor: 'rgba(71,85,105,0.15)' }}>
                <div className="flex items-center justify-between mb-1">
                  <span className="text-xs font-semibold" style={{ color: '#94a3b8' }}>APCA Lc</span>
                  <span className={`text-xs font-mono px-1.5 py-0.5 rounded border ${
                    active.contrast.passesAPCABody ? 'text-emerald-300 border-emerald-500/30 bg-emerald-500/10' :
                                                      'text-amber-300 border-amber-500/30 bg-amber-500/10'
                  }`}>
                    {active.contrast.passesAPCABody ? '✓ Body' : '⚠ Large only'}
                  </span>
                </div>
                <span className="text-xl font-bold font-mono" style={{ color: '#f1f5f9' }}>
                  Lc {Math.abs(active.contrast.apcaContrast).toFixed(1)}
                </span>
                <div className="text-xs font-mono mt-1" style={{ color: '#64748b' }}>
                  min body: {60} · min large: {45}
                </div>
              </div>

              {/* Quick checks */}
              <div className="grid grid-cols-2 gap-2 text-xs">
                {[
                  { label: 'WCAG AA',       pass: active.contrast.passesAA },
                  { label: 'WCAG AAA',      pass: active.contrast.passesAAA },
                  { label: 'APCA Body',     pass: active.contrast.passesAPCABody },
                  { label: 'APCA Large',    pass: Math.abs(active.contrast.apcaContrast) >= 45 },
                ].map(({ label, pass }) => (
                  <div key={label} className="flex items-center gap-2 p-2 rounded-lg border"
                       style={{ background: 'rgba(17,24,39,0.4)', borderColor: 'rgba(71,85,105,0.15)' }}>
                    <span className={`text-sm ${pass ? 'text-emerald-400' : 'text-red-400'}`}>
                      {pass ? '✓' : '✗'}
                    </span>
                    <span style={{ color: '#94a3b8' }}>{label}</span>
                  </div>
                ))}
              </div>

            </div>
          ) : null}
        </div>
      </div>

      {/* Footer: momoto attribution */}
      <div className="px-6 py-3 border-t flex items-center justify-between"
           style={{ borderColor: 'rgba(200,80,0,0.10)', background: 'rgba(4,2,0,0.60)' }}>
        <span className="text-xs font-mono" style={{ color: '#4a3820' }}>
          Powered by <span style={{ color: '#e85200' }}>momoto-ui-core</span> · OKLCH perceptual model
        </span>
        <span className="text-xs font-mono" style={{ color: '#4a3820' }}>
          {ready ? `WASM initialized` : 'loading…'}
        </span>
      </div>
    </div>
  );
}
