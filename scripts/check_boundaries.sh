#!/usr/bin/env bash
#
# scripts/check_boundaries.sh
#
# Halcon frontier architecture boundary guards — enforcement de Ω-01..Ω-20.
#
# Spec autoritativa:     docs/architecture/halcon-v3-correction.md
# ADR principal:         docs/architecture/adr/ADR-HALCON-001-capabilities-scope.md
# Formal specs:          spec/halcon_plan_lifecycle.tla + model/halcon_ownership.als
#
# Falla con exit 1 si encuentra:
#   [1] Dependencias globalmente prohibidas en production (LLM SDKs directos)
#   [2] Dependencias de crates internos de Paloma (deben usar paloma-boundary)
#   [3] Símbolos prohibidos en production paths (routing/retry/budget local)
#   [4] Wire DTOs sin schema_version obligatorio
#   [5] Feature flags cfg(feature="X") sin feature declarada en Cargo.toml
#   [6] Endpoints HTTP que duplican rutas de tordo-api /v1/jobs
#   [7] URLs de LLM upstream hardcoded en binary production
#
# Diseñado para correr local y en CI. Sin deps externas más allá de bash y grep.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

red()    { printf '\033[31m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
blue()   { printf '\033[34m%s\033[0m\n' "$*"; }

violations=0

# Flag --strict exige cumplimiento total (TODO gates); por default permite
# warnings para checks progresivos durante la transición Ciclo 0 → 6.
STRICT=0
SKIP_SDK_CHECK=0
if [[ "${1:-}" == "--strict" ]]; then
  STRICT=1
  blue "Modo STRICT: todas las violaciones fallan"
fi

# ---------------------------------------------------------------------------
# [1] Dependencias globalmente prohibidas en PRODUCTION path
#     (toleradas en feature dev-providers tras Ciclo 2)
# ---------------------------------------------------------------------------
# Estas son SDKs que sólo deben vivir tras #[cfg(feature = "dev-providers")].
# Por ahora sólo emitimos warning; tras Ciclo 2 serán hard fail.
GLOBAL_BANNED_LLM_SDKS=(
  "^[[:space:]]*async-openai[[:space:]]*="
  "^[[:space:]]*anthropic-sdk[[:space:]]*="
  "^[[:space:]]*clust[[:space:]]*="
  "^[[:space:]]*cohere[[:space:]]*="
  "^[[:space:]]*cohere-rs[[:space:]]*="
  "^[[:space:]]*ollama-rs[[:space:]]*="
)

yellow "==> [1/7] Dependencias LLM SDK directas (warn hasta Ciclo 2, fail en strict)"
for pattern in "${GLOBAL_BANNED_LLM_SDKS[@]}"; do
  # shellcheck disable=SC2086
  hits=$(grep -RIn --include='Cargo.toml' -E "$pattern" crates/ 2>/dev/null || true)
  if [[ -n "$hits" ]]; then
    if [[ "$STRICT" -eq 1 ]]; then
      red "  prohibida: ${pattern}"
      echo "$hits" | sed 's/^/    /'
      violations=$((violations + 1))
    else
      yellow "  (warn) SDK directo detectado — OK bajo dev-providers feature tras Ciclo 2:"
      echo "$hits" | sed 's/^/    /'
    fi
  fi
done

# ---------------------------------------------------------------------------
# [2] Dependencias en crates INTERNOS de Paloma (prohibidas — usar paloma-boundary)
# ---------------------------------------------------------------------------
# ADR-HALCON-001: Halcon consume Paloma SÓLO vía contracts/DTOs stable.
# Los crates internos de Paloma (pipeline, scoring, planner, registry, budget,
# health, trace, types) llevan lógica que no debe existir en Halcon.
yellow "==> [2/7] Crates internos de Paloma prohibidos en Halcon"
PALOMA_INTERNAL_CRATES=(
  "paloma-pipeline"
  "paloma-scoring"
  "paloma-planner"
  "paloma-registry"
  "paloma-budget"
  "paloma-health"
  "paloma-trace"
  "paloma-ledger"
  "paloma-eval"
  "paloma-server"
  "paloma-store-pg"
  "paloma-store-redis"
  "paloma-events"
  "paloma-metrics"
  "paloma-policy"
)
for crate in "${PALOMA_INTERNAL_CRATES[@]}"; do
  hits=$(grep -RIn --include='Cargo.toml' -E "^[[:space:]]*${crate}[[:space:]]*=" crates/ 2>/dev/null || true)
  if [[ -n "$hits" ]]; then
    if [[ "$STRICT" -eq 1 ]]; then
      red "  crate interno Paloma en Halcon: ${crate}"
      echo "$hits" | sed 's/^/    /'
      violations=$((violations + 1))
    else
      yellow "  (warn) interno Paloma — migrar a paloma-boundary tras Ciclo 2:"
      echo "$hits" | sed 's/^/    /'
    fi
  fi
done

# ---------------------------------------------------------------------------
# [3] Símbolos prohibidos en production paths
# ---------------------------------------------------------------------------
# Halcon production NO debe contener código que:
#   - decida routing localmente (IntelligentRouter::route, PalomaRouter::route)
#   - mantenga retry loops (RetryPolicy en hot path)
#   - instancie Paloma internals (Pipeline::new)
yellow "==> [3/7] Símbolos de routing/retry local en production paths"
PROHIBITED_SYMBOLS_IN_PRODUCTION=(
  "Pipeline::new(PipelineConfig"
  "BudgetStore::new()"
  "HealthTracker::new(&candidate_ids)"
  "RegistrySnapshot::new("
)
for symbol in "${PROHIBITED_SYMBOLS_IN_PRODUCTION[@]}"; do
  # Exclude test code and dev-providers gated code
  hits=$(grep -RIn --include='*.rs' -F "$symbol" crates/ 2>/dev/null \
         | grep -v '#\[cfg(test)\]' \
         | grep -v 'tests/' \
         | grep -v '#\[cfg(feature = "dev-providers")\]' \
         || true)
  if [[ -n "$hits" ]]; then
    if [[ "$STRICT" -eq 1 ]]; then
      red "  símbolo Paloma in-process: ${symbol}"
      echo "$hits" | sed 's/^/    /'
      violations=$((violations + 1))
    else
      yellow "  (warn) Paloma in-process detectado — mover a dev-providers en Ciclo 2:"
      echo "$hits" | head -5 | sed 's/^/    /'
    fi
  fi
done

# ---------------------------------------------------------------------------
# [4] Wire DTOs sin schema_version
# ---------------------------------------------------------------------------
# Todo DTO que cruza frontera HTTP/MCP debe tener schema_version: String.
# Por ahora sólo detectamos en crates explícitos de wire types.
yellow "==> [4/7] schema_version en wire DTOs (informativo)"
# Crates que contienen wire types (cross-boundary DTOs)
WIRE_CRATES=(
  "halcon-api/src/types"
)
for crate_path in "${WIRE_CRATES[@]}"; do
  full_path="crates/${crate_path}"
  if [[ ! -d "$full_path" ]]; then
    continue
  fi
  # Cuenta structs con Serialize/Deserialize que NO tengan schema_version
  missing=$(grep -RIl '#\[derive(.*Serialize.*Deserialize\|Deserialize.*Serialize' "$full_path" 2>/dev/null \
            | while read -r file; do
                if ! grep -q 'schema_version' "$file" 2>/dev/null; then
                  echo "$file"
                fi
              done)
  if [[ -n "$missing" ]]; then
    yellow "  (info) wire types sin schema_version — obligatorio post Ciclo 1:"
    echo "$missing" | head -10 | sed 's/^/    /'
  fi
done

# ---------------------------------------------------------------------------
# [5] Feature flags cfg(feature = "X") sin X declarada en Cargo.toml
# ---------------------------------------------------------------------------
# I-H13: cfg sobre features inexistentes == código silenciosamente muerto.
# Optimizado: 1 grep global + comparación por crate via parsing paths.
yellow "==> [5/7] Integridad de feature flags (cfg sin feature declarada)"

# Set de features built-in de cargo que no requieren declaración
BUILTIN_FEATURES="test bench"

# 1 pasada global: extraer {crate_dir, feature_name} de todos los cfg(feature="X")
# Usa awk para parsing + dedup en memoria.
feature_usages=$(
  # Encuentra todos los archivos .rs bajo crates/*/src/
  find crates -type f -name '*.rs' -path '*/src/*' -not -path '*/target/*' \
    -exec grep -HE 'cfg\s*\(\s*feature\s*=\s*"[a-z0-9_-]+"' {} + 2>/dev/null \
  | awk -F: '
      {
        # path es $1; resto es contenido
        # Extrae crate dir (primera parte después de crates/)
        match($1, /crates\/[^\/]+/)
        crate = substr($1, RSTART, RLENGTH)
        # Extrae cada feature name en la línea
        content = $0
        while (match(content, /feature[[:space:]]*=[[:space:]]*"[a-z0-9_-]+"/)) {
          feat_token = substr(content, RSTART, RLENGTH)
          if (match(feat_token, /"[a-z0-9_-]+"/)) {
            feat = substr(feat_token, RSTART+1, RLENGTH-2)
            print crate "\t" feat
          }
          content = substr(content, RSTART+RLENGTH)
        }
      }' \
  | sort -u
)

# Por cada (crate, feature) único, chequear si está declarada en su Cargo.toml
prev_crate=""
declared=""
missing_count=0
while IFS=$'\t' read -r crate feat; do
  [[ -z "$crate" || -z "$feat" ]] && continue
  # Built-in skip
  if [[ " $BUILTIN_FEATURES " == *" $feat "* ]]; then continue; fi
  if [[ "$crate" != "$prev_crate" ]]; then
    cargo_toml="$crate/Cargo.toml"
    if [[ -f "$cargo_toml" ]]; then
      declared=$(awk '/^\[features\]/{flag=1; next} /^\[/{flag=0} flag && /^[a-z][a-z0-9_-]*[[:space:]]*=/{gsub(/[[:space:]]*=.*/,""); print}' "$cargo_toml" 2>/dev/null)
    else
      declared=""
    fi
    prev_crate="$crate"
  fi
  if ! echo "$declared" | grep -qx "$feat"; then
    missing_count=$((missing_count + 1))
    if [[ "$STRICT" -eq 1 ]]; then
      red "  feature '${feat}' usada en ${crate} pero NO declarada"
      violations=$((violations + 1))
    elif [[ "$missing_count" -le 5 ]]; then
      yellow "  (warn) '${feat}' en ${crate} sin declarar"
    fi
  fi
done <<< "$feature_usages"
if [[ "$missing_count" -gt 5 && "$STRICT" -eq 0 ]]; then
  yellow "  ... y ${missing_count} más (total features no declaradas)"
fi

# ---------------------------------------------------------------------------
# [6] halcon-api no debe exponer endpoints conceptualmente de Tordo
# ---------------------------------------------------------------------------
# Ω-08: halcon-api no debe tener POST /tasks que dispare DAGs durables.
# Production path: cliente va directo a tordo-api.
yellow "==> [6/7] halcon-api no duplica endpoints durables de tordo-api"
# Esta regla es informativa hasta Ciclo 2; en strict falla.
duplicate_endpoints=$(grep -rn 'route.*tasks.*post\|route.*tasks.*submit' crates/halcon-api/src/server/router.rs 2>/dev/null || true)
if [[ -n "$duplicate_endpoints" ]]; then
  if [[ "$STRICT" -eq 1 ]]; then
    red "  halcon-api expone tasks submit (debe delegar a tordo-api):"
    echo "$duplicate_endpoints" | sed 's/^/    /'
    violations=$((violations + 1))
  else
    yellow "  (warn) /tasks endpoint — eliminar en Ciclo 2 a favor de tordo-api:"
    echo "$duplicate_endpoints" | sed 's/^/    /'
  fi
fi

# ---------------------------------------------------------------------------
# [7] URLs LLM upstream en binary release
# ---------------------------------------------------------------------------
# Tras Ciclo 2, el binary release sin --features dev-providers NO debe
# contener URLs de OpenAI/Anthropic/etc.  Este check requiere binary compilado.
yellow "==> [7/7] URLs LLM upstream en binary (requiere build release)"
BINARY_PATH="target/release/halcon"
if [[ -x "$BINARY_PATH" ]]; then
  FORBIDDEN_URLS=(
    "api.anthropic.com"
    "api.openai.com"
    "api.deepseek.com"
    "generativelanguage.googleapis.com"
    "bedrock-runtime."
    "aiplatform.googleapis.com"
  )
  for url in "${FORBIDDEN_URLS[@]}"; do
    if strings "$BINARY_PATH" 2>/dev/null | grep -qF "$url"; then
      if [[ "$STRICT" -eq 1 ]]; then
        red "  URL LLM upstream en binary: ${url}"
        violations=$((violations + 1))
      else
        yellow "  (warn) URL ${url} en binary — OK pre-Ciclo 2, FAIL después"
      fi
    fi
  done
else
  blue "  (skip) ${BINARY_PATH} no existe — construye con 'cargo build --release' para ejecutar este check"
fi

# ---------------------------------------------------------------------------
# Resumen
# ---------------------------------------------------------------------------
echo ""
if [[ "$violations" -eq 0 ]]; then
  green "==> check_boundaries.sh PASS (0 violaciones estrictas)"
  exit 0
else
  red "==> check_boundaries.sh FAIL (${violations} violaciones)"
  echo ""
  echo "Para ver el plan de corrección y obligaciones Ω-NN:"
  echo "  docs/architecture/halcon-v3-correction.md"
  echo "  docs/architecture/adr/ADR-HALCON-001-capabilities-scope.md"
  echo ""
  echo "Para ejecutar sólo warnings (no fail):"
  echo "  scripts/check_boundaries.sh"
  echo ""
  echo "Para modo strict (CI post Ciclo 2):"
  echo "  scripts/check_boundaries.sh --strict"
  exit 1
fi
