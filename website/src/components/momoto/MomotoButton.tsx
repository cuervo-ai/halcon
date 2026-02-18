/**
 * MomotoButton — CTA button with momoto WASM state derivation
 * States (Idle/Hover/Active/Focus/Loading) driven by TokenDerivationEngine
 */

import { useState, useEffect, useRef } from 'react';
import type { StateHexTokens } from '../../lib/momoto/index.ts';

interface Props {
  href:      string;
  children:  React.ReactNode;
  variant?:  'primary' | 'secondary';
  icon?:     React.ReactNode;
  className?: string;
}

// OKLCH brand blue for primary CTA
const BASE_L = 0.44, BASE_C = 0.205, BASE_H = 264;

export default function MomotoButton({ href, children, variant = 'primary', icon, className = '' }: Props) {
  const [tokens, setTokens]       = useState<StateHexTokens | null>(null);
  const [uiState, setUiState]     = useState<'idle' | 'hover' | 'active' | 'focus' | 'loading'>('idle');
  const [isLoading, setIsLoading] = useState(false);
  const btnRef = useRef<HTMLAnchorElement>(null);

  useEffect(() => {
    let active = true;
    import('../../lib/momoto/index.ts').then(async ({ initMomoto, deriveStateColors }) => {
      await initMomoto();
      if (!active) return;
      const t = await deriveStateColors(BASE_L, BASE_C, BASE_H);
      if (active) setTokens(t);
    });
    return () => { active = false; };
  }, []);

  type TokenKey = keyof StateHexTokens;
  const currentBg = tokens
    ? (tokens[uiState as TokenKey] ?? tokens.idle)
    : (uiState === 'hover' ? '#2563eb' : uiState === 'active' ? '#1d4ed8' : '#2563eb');

  const handleClick = (_e: React.MouseEvent) => {
    setIsLoading(true);
    setUiState('loading');
    // reset after navigation (won't fire if page changes)
    setTimeout(() => { setIsLoading(false); setUiState('idle'); }, 2000);
  };

  return (
    <a
      ref={btnRef}
      href={href}
      onClick={handleClick}
      className={`inline-flex items-center gap-2 font-semibold rounded-full transition-all duration-200 relative overflow-hidden select-none ${className}`}
      style={{
        padding:     '0.75rem 2rem',
        background:  variant === 'primary' ? currentBg : 'transparent',
        color:       '#ffffff',
        border:      variant === 'secondary' ? `2px solid ${tokens?.idle ?? '#22d3ee'}` : 'none',
        transform:   uiState === 'active' ? 'scale(0.97)' : uiState === 'hover' ? 'translateY(-2px)' : 'none',
        boxShadow:   uiState === 'hover'
                       ? `0 0 24px ${tokens?.idle ?? '#1d4ed8'}55, 0 8px 32px rgba(0,0,0,0.3)`
                       : uiState === 'focus'
                         ? `0 0 0 3px ${tokens?.focus ?? '#22d3ee'}66`
                         : 'none',
        textDecoration: 'none',
      }}
      onMouseEnter={() => !isLoading && setUiState('hover')}
      onMouseLeave={() => !isLoading && setUiState('idle')}
      onMouseDown={() =>  !isLoading && setUiState('active')}
      onMouseUp={() =>    !isLoading && setUiState('hover')}
      onFocus={() =>      !isLoading && setUiState('focus')}
      onBlur={() =>       !isLoading && setUiState('idle')}
    >
      {/* Shimmer overlay on hover */}
      {uiState === 'hover' && (
        <span
          className="absolute inset-0 pointer-events-none"
          style={{
            background: 'linear-gradient(105deg, transparent 40%, rgba(255,255,255,0.08) 50%, transparent 60%)',
            animation:  'shimmer 1.5s ease infinite',
          }}
        />
      )}

      {/* Loading spinner */}
      {isLoading ? (
        <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none">
          <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" strokeOpacity="0.3" />
          <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
        </svg>
      ) : icon ? (
        <span className="flex-shrink-0">{icon}</span>
      ) : null}

      {children}

      {/* Momoto state indicator (dev hint — remove in prod if desired) */}
      {tokens && (
        <span
          className="absolute -top-1 -right-1 w-2 h-2 rounded-full opacity-60"
          style={{ background: tokens[uiState as TokenKey] ?? tokens.idle }}
          title={`momoto state: ${uiState}`}
        />
      )}
    </a>
  );
}
