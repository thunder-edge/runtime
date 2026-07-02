---
id: ADR-RUNTIME-0006
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0006: execution_id + watchdog para limpeza de recursos

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform

## Contexto

Contextos de longa duração podem vazar timers, intervals e promises entre requests. Timeouts
precisam ser aplicados de forma determinística.

## Decisão

Atribuir um **`execution_id` (UUID) por request**; marcar todos os timers/intervals/promises/recursos
por ele; limpar automaticamente no término ou timeout. Timeout com **dupla camada**: watchdog
(CPU via `CLOCK_THREAD_CPUTIME_ID` + wall-clock) e `tokio::time::timeout`.

## Alternativas consideradas

- **GC/limpeza implícita:** insuficiente para timers/sockets; não garante timeout determinístico.

## Consequências

### Positivas
- Sem vazamento entre requests; timeout previsível; isolate morto → auto-restart com backoff.

### Negativas / Trade-offs
- Overhead por request (UUID + lookups na limpeza); user code não deve cachear recursos entre requests.

## Referências
- `runtime/crates/functions/src/{lifecycle.rs,handler.rs}`
- `runtime/crates/runtime-core/src/{cpu_timer.rs,mem_check.rs}`
- `runtime/docs/design/timeout-and-resource-tracking.md`
