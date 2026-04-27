# Halcon Architecture Decision Records (ADRs)

Este directorio contiene los ADRs normativos de Halcon. Cada ADR documenta una decisión arquitectónica significativa con contexto, consecuencias y verificabilidad.

## Convenciones

- Nombres: `ADR-HALCON-NNN-short-title.md`.
- Status: `Draft` → `Proposed` → `Accepted` → `Superseded` / `Deprecated`.
- Aceptación requiere sign-off de ≥2 principal architects.
- Cambios post-aceptación: nuevo ADR con `Supersedes: ADR-HALCON-NNN`.

## Índice

| # | Título | Status | Date |
|---|--------|--------|------|
| 001 | [Halcon capabilities scope](./ADR-HALCON-001-capabilities-scope.md) | Draft | 2026-04-17 |
| 002 | Boundary contracts and versioning | Planned (Ciclo 1) | — |
| 003 | No local retry; no local budget; no local routing | Planned (Ciclo 2) | — |
| 004 | Durable execution via Tordo | Planned (Ciclo 3) | — |
| 005 | Audit replication to paloma-ledger | Planned (Ciclo 3) | — |

## Cómo proponer un ADR nuevo

1. Crear rama `adr/halcon-NNN-short-title`.
2. Copiar template de un ADR existente (ADR-001 es el más completo).
3. Rellenar: Context, Decision, Consequences, Alternatives, Implementation.
4. PR con label `architecture`.
5. Notificar a arquitectos para review.
6. Merge tras ≥2 sign-offs.

## Documentos relacionados

- [`halcon-v3-correction.md`](../halcon-v3-correction.md) — spec principal (obligaciones Ω-01..Ω-20, invariantes I-H1..I-H14, ciclos 0-6).
- [`../new_design.md`](../new_design.md) — documento de diseño legacy (pre-v3).
