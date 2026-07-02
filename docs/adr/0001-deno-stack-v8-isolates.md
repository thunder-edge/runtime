---
id: ADR-RUNTIME-0001
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
related_specs: []
related_plans: []
created_at: 2026-06-26
---

# ADR 0001: Adotar Deno stack (deno_core + V8) para execução JS/TS

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (decisão vigente desde 2026-03-04)
**Owner:** Edge Platform
**Knowledge:** [architecture.md](../knowledge/architecture.md)

## Contexto

Thunder precisa executar funções JavaScript/TypeScript de usuários no edge, com isolamento
forte, APIs Web modernas e cold-start baixo, sem construir um motor JS próprio.

## Decisão

Construir o runtime sobre a **stack Deno** (`deno_core` e extensões `deno_*`) em cima do **V8**,
usando isolates V8 como fronteira de isolamento e ops nativas (Rust → JS) para capacidades do host.

## Alternativas consideradas

- **Node.js embutido / N-API:** ecossistema maior, mas modelo de sandbox fraco e legado CommonJS.
- **QuickJS / motor próprio:** menor footprint, porém sem APIs Web nem ecossistema de módulos.
- **WASM-only:** isolamento excelente, mas ergonomia ruim para JS/TS e compat de libs limitada.

## Consequências

### Positivas
- APIs Web nativas (fetch, Streams, WebCrypto, URL) prontas.
- ESM-first, resolução de módulos tipada, sandbox por isolate.
- Extensível via ops; base sólida para compat Node parcial.

### Negativas / Trade-offs
- ~35 dependências `deno_*` acopladas a versões específicas do V8.
- Compat Node é necessariamente parcial (ver ADR-0011).
- Upgrades de V8/Deno exigem coordenação (snapshots, APIs).

### Riscos
- Acompanhar cadência de releases do ecossistema Deno.

## Referências
- `runtime/crates/runtime-core/src/extensions.rs`, `isolate.rs`
- `runtime/Cargo.toml` (deno_core 0.390, deno_web 0.268, ...)
