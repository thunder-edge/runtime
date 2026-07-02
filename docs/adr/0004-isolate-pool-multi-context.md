---
id: ADR-RUNTIME-0004
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0004: Pool de isolates por função → multi-context por isolate

**Status:** `ACCEPTED (retroativo)` — evolução em andamento
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/ROADMAP_CONTEXT_ISOLATE.md`

## Contexto

Cada função precisa de isolamento e capacidade de escalar sob carga. O modelo inicial usa uma
thread OS + `JsRuntime` por réplica de isolate, garantindo isolamento pela fronteira de thread,
mas com alto custo de threads (~8 MiB de stack cada) e pressão de FD.

## Decisão

- **Atual:** cada função → `FunctionEntry` com N isolates replicados (round-robin), cada um em
  thread dedicada com loop de canal de requests. Pooling com LRU eviction.
- **Alvo:** N contextos por isolate compartilhando um `JsRuntime`; auto-scale de contexto → novo
  isolate ao atingir `max_contexts_per_isolate` → shedding ao atingir o máximo global.

## Alternativas consideradas

- **Um isolate global multi-tenant:** máximo aproveitamento, isolamento fraco (inaceitável).
- **Isolate por request:** isolamento máximo, cold-start proibitivo.

## Consequências

### Positivas
- Atual: isolamento simples e forte. Alvo: eficiência de recursos, alívio de FD.

### Negativas / Trade-offs
- Atual: alto número de threads. Alvo: escalonamento complexo, coordenação de limpeza cross-context (via execution_id, ver ADR-0006).

## Referências
- `runtime/crates/functions/src/registry.rs`, `lifecycle.rs`
- `runtime/ROADMAP_CONTEXT_ISOLATE.md`
