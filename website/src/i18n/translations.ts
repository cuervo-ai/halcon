export type Lang = 'en' | 'es';

export const ui = {
  en: {
    /* ── Meta ─────────────────────────────────────────────────────── */
    'meta.site_name':       'Halcón CLI',
    'meta.index.title':     'Halcón CLI — AI coding assistant for your terminal',
    'meta.index.desc':      'Connect Claude, GPT-4o, DeepSeek, Gemini, or any local Ollama model to your terminal. 21 built-in tools, full permission control, MCP server built in.',
    'meta.download.title':  'Download Halcón CLI',
    'meta.download.desc':   'Download Halcón CLI v0.3.0 for macOS, Linux, and Windows.',
    'meta.docs.title':      'Documentation — Halcón CLI',
    'meta.docs.desc':       'Get started with Halcón CLI. Configuration, commands, tools, and provider setup.',

    /* ── Nav ──────────────────────────────────────────────────────── */
    'nav.features':  'Features',
    'nav.providers': 'Providers',
    'nav.download':  'Download',
    'nav.docs':      'Docs',
    'nav.lang':      'ES',
    'nav.lang_href': '/es',
    'nav.lang_label':'Cambiar a Español',

    /* ── Hero ─────────────────────────────────────────────────────── */
    'hero.headline':   'The terminal AI agent\nthat runs on any model.',
    'hero.sub':        'Halcón connects your terminal to Claude, GPT-4o, DeepSeek, Gemini, or any local Ollama model — with 21 built-in tools, layered permissions, and a native MCP server for your IDE.',
    'hero.cta_dl':     'Download v0.3.0',
    'hero.cta_docs':   'Read the docs',
    'hero.platforms':  'macOS · Linux · Windows',
    'hero.install_label': 'One-line install',

    /* ── Providers ────────────────────────────────────────────────── */
    'providers.heading':  'Works with every major AI provider',
    'providers.sub':      'Switch providers and models with a single flag. No reconfiguration, no friction.',
    'providers.local_badge': 'Local · Free',
    'providers.local_desc':  'Run any Ollama-supported model on your hardware. Zero API cost, full privacy.',
    'providers.models':  'Models',

    /* ── Features ─────────────────────────────────────────────────── */
    'features.heading': 'Everything you need to automate your workflow',
    'features.sub':     'Production-grade with 2,200+ test coverage. Multi-model. Zero native dependencies.',
    'f1.title': '21 built-in tools',
    'f1.desc':  'File read/write/edit, bash execution, git operations, web search, regex grep, code symbol extraction, HTTP requests, and background jobs — all wired into the agent loop.',
    'f2.title': 'Permission-first by design',
    'f2.desc':  'Every tool is classified: ReadOnly tools execute silently, ReadWrite and Destructive tools require your explicit confirmation. The agent never runs rm or git commit without you.',
    'f3.title': 'TUI Cockpit',
    'f3.desc':  'A 3-zone terminal UI with live token counters, agent FSM state, plan progress, side panel metrics, and a real-time activity stream. Pause, step, or cancel the agent mid-execution.',
    'f4.title': 'Episodic memory',
    'f4.desc':  'The agent remembers decisions, file paths, and learnings across sessions using BM25 semantic search, temporal decay scoring, and automatic consolidation. No setup required.',
    'f5.title': 'MCP Server built in',
    'f5.desc':  'Run `halcon mcp-server` to expose all 21 tools via JSON-RPC over stdio. Wire it into VS Code, Cursor, or any IDE that supports the Model Context Protocol.',
    'f6.title': 'Multi-provider routing',
    'f6.desc':  'Automatic fallback between providers, latency-aware model selection, and cost tracking per session. Set `balanced`, `fast`, or `cheap` routing strategies in config.',

    /* ── Quick start ──────────────────────────────────────────────── */
    'qs.heading': 'Up and running in 60 seconds',
    'qs.step1':   'Install Halcón CLI',
    'qs.step2':   'Configure your API key',
    'qs.step3':   'Start coding with AI',
    'qs.step1_note': 'Installs to ~/.local/bin/halcon. SHA-256 verified.',
    'qs.step2_note': 'Stores your key securely in the OS keychain.',
    'qs.step3_note': 'Or launch the full TUI cockpit with --tui',
    'qs.all_providers': 'Supports Anthropic, OpenAI, DeepSeek, Gemini, and local Ollama.',

    /* ── Download page ────────────────────────────────────────────── */
    'dl.heading':       'Download Halcón CLI',
    'dl.badge':         'Latest release · v0.3.0',
    'dl.sub':           'AI-powered coding agent for your terminal.\nMulti-model. Native performance.',
    'dl.smart_heading': 'Recommended for your system',
    'dl.methods':       'Install methods',
    'dl.all_platforms': 'All platforms',
    'dl.platform':      'Platform',
    'dl.target':        'Target triple',
    'dl.notes':         'Notes',
    'dl.download':      'Download',
    'dl.checksums':     'Download checksums.txt (SHA-256)',
    'dl.verify_heading':'Verify your download',
    'dl.verify_sub':    'All releases include SHA-256 checksums. The installer verifies automatically. To verify manually:',
    'dl.after_heading': 'After installing',
    'dl.step1_cmd':     'halcon auth login anthropic',
    'dl.step1_note':    'Add your API key — stored securely in OS keychain',
    'dl.step2_cmd':     'halcon chat "Hello, Halcón!"',
    'dl.step2_note':    'Test the connection',
    'dl.step3_cmd':     'halcon chat --tui',
    'dl.step3_note':    'Launch the full TUI cockpit',
    'dl.docs_link':     'Read the full documentation →',

    /* ── Docs page ────────────────────────────────────────────────── */
    'docs.heading':       'Documentation',
    'docs.sub':           'Get started in minutes. Everything you need to configure and use Halcón CLI.',
    'docs.nav.quickstart':  'Quick Start',
    'docs.nav.config':      'Configuration',
    'docs.nav.commands':    'Commands',
    'docs.nav.chat':        'Chat & TUI',
    'docs.nav.providers':   'Providers',
    'docs.nav.tools':       'Tools',
    'docs.nav.memory':      'Memory',
    'docs.nav.mcp':         'MCP Server',

    /* ── Footer ───────────────────────────────────────────────────── */
    'footer.tagline':    'AI coding agent for your terminal. Multi-model. Native performance.',
    'footer.by':         'By Cuervo AI',
    'footer.product':    'Product',
    'footer.download':   'Download',
    'footer.changelog':  'Changelog',
    'footer.roadmap':    'Roadmap',
    'footer.developers': 'Developers',
    'footer.quickstart': 'Quick Start',
    'footer.cli_ref':    'CLI Reference',
    'footer.config':     'Configuration',
    'footer.mcp':        'MCP Server',
    'footer.install':    'Install',
    'footer.or':         'Or',
    'footer.manual_dl':  'download manually →',
    'footer.copyright':  '© 2026 Cuervo AI. All rights reserved.',
    'footer.support':    'Support',
  },

  es: {
    'meta.site_name':       'Halcón CLI',
    'meta.index.title':     'Halcón CLI — Agente IA de desarrollo para tu terminal',
    'meta.index.desc':      'Conecta Claude, GPT-4o, DeepSeek, Gemini o cualquier modelo Ollama local a tu terminal. 21 herramientas integradas, permisos completos, servidor MCP incluido.',
    'meta.download.title':  'Descargar Halcón CLI',
    'meta.download.desc':   'Descarga Halcón CLI v0.3.0 para macOS, Linux y Windows.',
    'meta.docs.title':      'Documentación — Halcón CLI',
    'meta.docs.desc':       'Primeros pasos con Halcón CLI. Configuración, comandos, herramientas y proveedores.',

    'nav.features':  'Características',
    'nav.providers': 'Proveedores',
    'nav.download':  'Descargar',
    'nav.docs':      'Docs',
    'nav.lang':      'EN',
    'nav.lang_href': '/',
    'nav.lang_label':'Switch to English',

    'hero.headline':   'El agente IA para tu terminal\nque funciona con cualquier modelo.',
    'hero.sub':        'Halcón conecta tu terminal a Claude, GPT-4o, DeepSeek, Gemini o cualquier modelo Ollama local — con 21 herramientas integradas, permisos por capas y un servidor MCP nativo para tu IDE.',
    'hero.cta_dl':     'Descargar v0.3.0',
    'hero.cta_docs':   'Leer documentación',
    'hero.platforms':  'macOS · Linux · Windows',
    'hero.install_label': 'Instalación en una línea',

    'providers.heading':  'Compatible con todos los proveedores de IA',
    'providers.sub':      'Cambia de proveedor y modelo con un solo flag. Sin reconfiguración, sin fricción.',
    'providers.local_badge': 'Local · Gratis',
    'providers.local_desc':  'Ejecuta cualquier modelo Ollama en tu hardware. Sin costo de API, privacidad total.',
    'providers.models':  'Modelos',

    'features.heading': 'Todo lo que necesitas para automatizar tu flujo de trabajo',
    'features.sub':     'Motor de alto rendimiento con más de 2.200 pruebas. Multi-modelo. Sin dependencias nativas.',
    'f1.title': '21 herramientas integradas',
    'f1.desc':  'Lectura/escritura/edición de archivos, ejecución bash, operaciones git, búsqueda web, grep regex, extracción de símbolos de código, peticiones HTTP y tareas en segundo plano — todo conectado al bucle del agente.',
    'f2.title': 'Permisos en el primer diseño',
    'f2.desc':  'Cada herramienta está clasificada: las herramientas ReadOnly se ejecutan silenciosamente, las ReadWrite y Destructive requieren tu confirmación explícita. El agente nunca ejecuta rm ni git commit sin ti.',
    'f3.title': 'Cockpit TUI',
    'f3.desc':  'Interfaz de terminal con 3 zonas: contadores de tokens en vivo, estado FSM del agente, progreso del plan, métricas en panel lateral y flujo de actividad en tiempo real. Pausa, avanza paso a paso o cancela el agente a mitad de ejecución.',
    'f4.title': 'Memoria episódica',
    'f4.desc':  'El agente recuerda decisiones, rutas de archivos y aprendizajes entre sesiones usando búsqueda semántica BM25, puntuación con decaimiento temporal y consolidación automática. Sin configuración.',
    'f5.title': 'Servidor MCP integrado',
    'f5.desc':  'Ejecuta `halcon mcp-server` para exponer las 21 herramientas vía JSON-RPC por stdio. Conéctalo a VS Code, Cursor o cualquier IDE compatible con Model Context Protocol.',
    'f6.title': 'Enrutamiento multi-proveedor',
    'f6.desc':  'Fallback automático entre proveedores, selección de modelos por latencia y seguimiento de costos por sesión. Configura estrategias `balanced`, `fast` o `cheap` en el archivo de configuración.',

    'qs.heading': 'En marcha en 60 segundos',
    'qs.step1':   'Instala Halcón CLI',
    'qs.step2':   'Configura tu clave API',
    'qs.step3':   'Empieza a programar con IA',
    'qs.step1_note': 'Se instala en ~/.local/bin/halcon. Verificado con SHA-256.',
    'qs.step2_note': 'Almacena tu clave de forma segura en el keychain del sistema.',
    'qs.step3_note': 'O lanza el cockpit TUI completo con --tui',
    'qs.all_providers': 'Compatible con Anthropic, OpenAI, DeepSeek, Gemini y Ollama local.',

    'dl.heading':       'Descargar Halcón CLI',
    'dl.badge':         'Última versión · v0.3.0',
    'dl.sub':           'Agente IA de desarrollo para tu terminal.\nMulti-modelo. Rendimiento nativo.',
    'dl.smart_heading': 'Recomendado para tu sistema',
    'dl.methods':       'Métodos de instalación',
    'dl.all_platforms': 'Todas las plataformas',
    'dl.platform':      'Plataforma',
    'dl.target':        'Triple objetivo',
    'dl.notes':         'Notas',
    'dl.download':      'Descargar',
    'dl.checksums':     'Descargar checksums.txt (SHA-256)',
    'dl.verify_heading':'Verifica tu descarga',
    'dl.verify_sub':    'Todas las versiones incluyen checksums SHA-256. El instalador verifica automáticamente. Para verificar manualmente:',
    'dl.after_heading': 'Después de instalar',
    'dl.step1_cmd':     'halcon auth login anthropic',
    'dl.step1_note':    'Agrega tu clave API — guardada de forma segura en el keychain del sistema',
    'dl.step2_cmd':     'halcon chat "¡Hola, Halcón!"',
    'dl.step2_note':    'Prueba la conexión',
    'dl.step3_cmd':     'halcon chat --tui',
    'dl.step3_note':    'Lanza el cockpit TUI completo',
    'dl.docs_link':     'Leer la documentación completa →',

    'docs.heading':       'Documentación',
    'docs.sub':           'Empieza en minutos. Todo lo que necesitas para configurar y usar Halcón CLI.',
    'docs.nav.quickstart':  'Inicio Rápido',
    'docs.nav.config':      'Configuración',
    'docs.nav.commands':    'Comandos',
    'docs.nav.chat':        'Chat y TUI',
    'docs.nav.providers':   'Proveedores',
    'docs.nav.tools':       'Herramientas',
    'docs.nav.memory':      'Memoria',
    'docs.nav.mcp':         'Servidor MCP',

    'footer.tagline':    'Agente IA de desarrollo para tu terminal. Multi-modelo. Rendimiento nativo.',
    'footer.by':         'Por Cuervo AI',
    'footer.product':    'Producto',
    'footer.download':   'Descargar',
    'footer.changelog':  'Historial de cambios',
    'footer.roadmap':    'Hoja de ruta',
    'footer.developers': 'Desarrolladores',
    'footer.quickstart': 'Inicio rápido',
    'footer.cli_ref':    'Referencia CLI',
    'footer.config':     'Configuración',
    'footer.mcp':        'Servidor MCP',
    'footer.install':    'Instalar',
    'footer.or':         'O',
    'footer.manual_dl':  'descarga manual →',
    'footer.copyright':  '© 2026 Cuervo AI. Todos los derechos reservados.',
    'footer.support':    'Soporte',
  },
} as const;

export function t(lang: Lang, key: string): string {
  const dict = ui[lang] as Record<string, string>;
  return dict[key] ?? (ui.en as Record<string, string>)[key] ?? key;
}

export function getLang(pathname: string): Lang {
  return pathname.startsWith('/es') ? 'es' : 'en';
}

export function getAlternatePath(lang: Lang, currentPath: string): string {
  if (lang === 'en') {
    // Going to ES: prepend /es
    return '/es' + (currentPath === '/' ? '' : currentPath);
  } else {
    // Going to EN: remove /es prefix
    return currentPath.replace(/^\/es/, '') || '/';
  }
}
