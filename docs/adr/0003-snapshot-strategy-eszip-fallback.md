---
id: ADR-RUNTIME-0003
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
related_adrs: [ADR-RUNTIME-0002]
created_at: 2026-06-26
---

# ADR 0003: Snapshot V8 com fallback ESZIP

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (removido na Fase 1, re-introduzido na Fase 3)
**Owner:** Edge Platform
**Knowledge:** [decisions-inferred.md](../knowledge/decisions-inferred.md), `runtime/ROADMAP_SNAPSHOT.md`

## Contexto

Cold-start é crítico no edge. Parsing de ESZIP a cada boot custa latência. Snapshots V8 cacheiam
bytecode e reduzem cold-start em ~3–4x, mas ficam presos à versão do V8.

Histórico: o suporte a snapshot foi **removido** na Fase 1 (simplificação) e **re-introduzido**
na Fase 3 com um design mais robusto (base snapshot + fallback), evidenciando decisão consciente.

## Decisão

Suportar formato **Snapshot** (bytecode V8) como caminho rápido de boot, com **fallback automático
para ESZIP** em mismatch de versão V8 ou corrupção. Bundle dual: `{format: Snapshot, bundle, fallback_eszip}`.
Runtime base snapshot compilado em build-time e embutido via `include_bytes!()`.

## Alternativas consideradas

- **Apenas ESZIP:** mais simples e robusto, porém cold-start maior (motivo da remoção inicial).
- **Apenas Snapshot:** mais rápido, mas frágil a upgrades de V8 (sem fallback → falha de boot).

## Consequências

### Positivas
- Cold-start reduzido; robustez preservada pelo fallback.

### Negativas / Trade-offs
- Dois caminhos de load; checagem de compatibilidade de versão V8.
- Snapshots são V8-version-locked; upgrade exige rebuild.
- Custo de build ao computar snapshot.

## Referências
- `runtime/crates/functions/src/snapshot.rs`, `types.rs` (`BundleFormat::Snapshot`)
- `runtime/ROADMAP_SNAPSHOT.md`, bundles em `runtime/bundles/snapshot/`
