---
id: ADR-RUNTIME-0008
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
related_adrs: [ADR-RUNTIME-0007]
created_at: 2026-06-26
---

# ADR 0008: Egress Connection Manager global (FD budget)

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/ROADMAP_FD.md`, `runtime/docs/design/egress-connection-manager.md`

## Contexto

Sob alta concorrência, `fetch()`/TCP/TLS de usuário podem esgotar file descriptors (EMFILE),
derrubando o processo, e um "noisy neighbor" pode starving os demais.

## Decisão

Rotear todo egress por um **gestor global de leases**: alocador de FD budget adaptativo (baseado
em `RLIMIT_NOFILE`), **token bucket** para bursts, **quotas por função** e **reaper** de leases
obsoletas (TTL). Métricas: `active_leases`, `queued_waiters`, `total_rejected`, `total_timeouts`.

## Alternativas consideradas

- **Sem limite (confiar no OS):** leva a EMFILE e crash.
- **Limite por isolate apenas:** não protege o processo globalmente.

## Consequências

### Positivas
- Previne exaustão de FD; isolamento de vizinhança; observabilidade de pressão.

### Negativas / Trade-offs
- Latência extra por fetch (acquire/release); quotas exigem tuning.

## Referências
- `runtime/crates/functions/src/connection_manager.rs`
- `runtime/docs/design/high-load-capacity-fd-saturation.md`
