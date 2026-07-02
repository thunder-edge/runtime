---
id: ADR-RUNTIME-0005
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0005: Arquitetura dual-listener (ingress + admin)

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (vigente desde Fase 1)
**Owner:** Edge Platform

## Contexto

Operações privilegiadas (deploy, reload, métricas, health) não podem ser expostas ao tráfego
público sem risco de redeploy malicioso/acidental.

## Decisão

Rodar **dois listeners HTTP separados**:
- **Admin** (padrão :9000, localhost): rotas `/_internal/*`, auth por header `X-API-Key`, inclui `/metrics` e `/health`.
- **Ingress** (padrão :8080 ou Unix socket): público, rotas `/{function_name}/*`, TLS opcional, rate limiting e body limits.

## Alternativas consideradas

- **Listener único com auth por rota:** menos superfície operacional, porém maior risco de exposição acidental.

## Consequências

### Positivas
- Isolamento de superfície privilegiada; políticas de segurança distintas por listener.

### Negativas / Trade-offs
- Duas máquinas de estado de conexão; dobro de configuração/monitoração.

## Referências
- `runtime/crates/server/src/{lib.rs,admin_router.rs,ingress_router.rs}`
