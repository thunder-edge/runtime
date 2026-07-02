---
id: ADR-RUNTIME-0011
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
related_adrs: [ADR-RUNTIME-0001]
created_at: 2026-06-26
---

# ADR 0011: Camada de compatibilidade Node.js (parcial, VFS)

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/ROADMAP-NODE-COMPAT.md`, `runtime/docs/reference/NODE-COMPAT.md`, `reference/vfs.md`

## Contexto

Frameworks SSR (Next.js, Astro, Vite) e libs comuns dependem de `node:*`. Sem compat, grande
parte do ecossistema não roda.

## Decisão

Implementar **compatibilidade Node.js parcial** (~35 módulos) via shim TypeScript + ops nativas,
priorizando alto impacto (path, stream com backpressure, async_hooks/AsyncLocalStorage, dns via DoH,
zlib, util, events, crypto). Prover **VFS** (`/bundle` RO, `/tmp` RW com quota, `/dev/null`).
Módulos inviáveis no sandbox (child_process, cluster) são **stubs com erro determinístico** (`[thunder]`).

## Alternativas consideradas

- **Compat total do Node:** inviável (módulos nativos/FFI no sandbox) e alto custo.
- **Sem compat:** exclui grande parte do ecossistema e SSR.

## Consequências

### Positivas
- Habilita SSR e libs populares; falha explícita em vez de silenciosa.

### Negativas / Trade-offs
- ~8k linhas de shim TS (manutenção); compat parcial pode mascarar bugs (funciona no Node, falha no subset).

## Referências
- `runtime/crates/runtime-core/src/node_compat/` (35 módulos)
