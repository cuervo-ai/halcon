import { useState, useEffect } from 'react';

interface Artifact {
  name: string;
  target: string;
  os: string;
  arch: string;
  sha256: string;
  size: number;
  url: string;
}

interface Manifest {
  version: string;
  published_at: string;
  artifacts: Artifact[];
  checksums_url: string;
  github_url: string;
}

interface PlatformsTableProps {
  releasesUrl: string;
  lang?: 'en' | 'es';
}

const KNOWN_PLATFORMS = [
  { name: 'macOS Apple Silicon', nameEs: 'macOS Apple Silicon', target: 'aarch64-apple-darwin',      ext: 'tar.gz', notes: 'M1 / M2 / M3 / M4 — Recommended', notesEs: 'M1 / M2 / M3 / M4 — Recomendado' },
  { name: 'macOS Intel',         nameEs: 'macOS Intel',         target: 'x86_64-apple-darwin',       ext: 'tar.gz', notes: 'Intel Macs (pre-2021)',           notesEs: 'Intel Macs (antes de 2021)' },
  { name: 'Linux x86_64',        nameEs: 'Linux x86_64',        target: 'x86_64-unknown-linux-musl', ext: 'tar.gz', notes: 'Static binary, no libc dep',      notesEs: 'Binario estático, sin libc' },
  { name: 'Linux ARM64',         nameEs: 'Linux ARM64',         target: 'aarch64-unknown-linux-gnu', ext: 'tar.gz', notes: 'Graviton, Raspberry Pi 4/5',       notesEs: 'Graviton, Raspberry Pi 4/5' },
  { name: 'Windows x86_64',      nameEs: 'Windows x86_64',      target: 'x86_64-pc-windows-gnu',     ext: 'zip',    notes: 'Windows 10/11',                   notesEs: 'Windows 10/11' },
];

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export default function PlatformsTable({ releasesUrl, lang = 'en' }: PlatformsTableProps) {
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [loading, setLoading]   = useState(true);

  const es = lang === 'es';

  useEffect(() => {
    fetch(`${releasesUrl}/latest/manifest.json`)
      .then(r => r.ok ? r.json() : null)
      .then((data: Manifest | null) => { setManifest(data); setLoading(false); })
      .catch(() => setLoading(false));
  }, [releasesUrl]);

  // Build lookup: target → artifact
  const artifactMap = new Map<string, Artifact>();
  if (manifest) {
    for (const a of manifest.artifacts) {
      artifactMap.set(a.target, a);
    }
  }

  const thStyle = { color: 'var(--c-t4)' };
  const headers = es
    ? ['Plataforma', 'Triple objetivo', 'Notas', 'Descargar']
    : ['Platform',   'Target triple',   'Notes', 'Download'];

  return (
    <div className="rounded-xl overflow-hidden" style={{ background: 'var(--c-code)', border: '1px solid var(--c-b)' }}>
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b" style={{ borderColor: 'var(--c-b)', background: 'var(--c-surface)' }}>
            <th className="text-left text-xs font-medium uppercase tracking-wider px-5 py-3" style={thStyle}>{headers[0]}</th>
            <th className="text-left text-xs font-medium uppercase tracking-wider px-5 py-3 hidden md:table-cell" style={thStyle}>{headers[1]}</th>
            <th className="text-left text-xs font-medium uppercase tracking-wider px-5 py-3 hidden lg:table-cell" style={thStyle}>{headers[2]}</th>
            <th className="text-right text-xs font-medium uppercase tracking-wider px-5 py-3" style={thStyle}>{headers[3]}</th>
          </tr>
        </thead>
        <tbody>
          {KNOWN_PLATFORMS.map(p => {
            const artifact  = artifactMap.get(p.target);
            const available = !!artifact;

            return (
              <tr key={p.target}
                  className="transition-colors border-t"
                  style={{ borderColor: 'rgba(200,80,0,0.08)' }}
                  onMouseOver={e  => { (e.currentTarget as HTMLElement).style.background = 'rgba(232,82,0,0.04)'; }}
                  onMouseOut={e   => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}>

                {/* Platform name */}
                <td className="px-5 py-4 font-medium" style={{ color: available ? 'var(--c-t1)' : 'var(--c-t4)' }}>
                  {es ? p.nameEs : p.name}
                </td>

                {/* Target triple */}
                <td className="px-5 py-4 font-mono text-xs hidden md:table-cell" style={{ color: 'var(--c-t3)' }}>
                  {p.target}
                </td>

                {/* Notes */}
                <td className="px-5 py-4 text-xs hidden lg:table-cell" style={{ color: 'var(--c-t4)' }}>
                  {es ? p.notesEs : p.notes}
                </td>

                {/* Download / status */}
                <td className="px-5 py-4 text-right">
                  {loading ? (
                    <span className="text-xs font-mono" style={{ color: 'var(--c-t4)' }}>…</span>
                  ) : available ? (
                    <a href={artifact!.url}
                       className="inline-flex items-center gap-1.5 text-xs font-medium transition-colors"
                       style={{ color: 'var(--c-p)' }}
                       onMouseOver={e  => { (e.currentTarget as HTMLElement).style.color = 'var(--c-gold)'; }}
                       onMouseOut={e   => { (e.currentTarget as HTMLElement).style.color = 'var(--c-p)'; }}>
                      <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                      </svg>
                      .{p.ext}
                      {artifact!.size > 0 && (
                        <span className="opacity-60">({formatBytes(artifact!.size)})</span>
                      )}
                    </a>
                  ) : (
                    <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded"
                          style={{ background: 'rgba(200,80,0,0.06)', color: 'var(--c-t4)', border: '1px solid rgba(200,80,0,0.12)' }}>
                      <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                      </svg>
                      {es ? 'Próximamente' : 'Coming soon'}
                    </span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {/* Footer */}
      <div className="px-5 py-3.5 border-t flex items-center justify-between"
           style={{ borderColor: 'var(--c-b)', background: 'var(--c-surface)' }}>
        <a href={`${releasesUrl}/latest/checksums.txt`}
           className="text-xs transition-colors flex items-center gap-1.5"
           style={{ color: 'var(--c-t4)' }}
           onMouseOver={e  => { (e.currentTarget as HTMLElement).style.color = 'var(--c-t2)'; }}
           onMouseOut={e   => { (e.currentTarget as HTMLElement).style.color = 'var(--c-t4)'; }}>
          <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
                  d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
          </svg>
          checksums.txt (SHA-256)
        </a>
        {manifest && (
          <a href={manifest.github_url}
             target="_blank" rel="noopener noreferrer"
             className="text-xs transition-colors"
             style={{ color: 'var(--c-t4)' }}
             onMouseOver={e  => { (e.currentTarget as HTMLElement).style.color = 'var(--c-gold)'; }}
             onMouseOut={e   => { (e.currentTarget as HTMLElement).style.color = 'var(--c-t4)'; }}>
            {es ? `Ver v${manifest.version} en GitHub ↗` : `View v${manifest.version} on GitHub ↗`}
          </a>
        )}
      </div>
    </div>
  );
}
