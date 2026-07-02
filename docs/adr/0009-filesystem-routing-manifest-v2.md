---
id: ADR-RUNTIME-0009
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0009: Roteamento por filesystem (routed-app) + manifest v2

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (v1 removido em favor de v2)
**Owner:** Edge Platform
**Knowledge:** `runtime/ROADMAP_ROUTING.md`, `runtime/docs/reference/function-manifest.md`

## Contexto

Além de funções single-handler, é necessário suportar aplicações REST com múltiplas rotas e
assets estáticos, com convenções familiares a devs frontend.

## Decisão

Introduzir **manifest v2** (JSON Schema 2020-12) com suporte a routing e **dois flavors**:
`single` (um handler) e `routed-app` (rotas por arquivo `[id].get.ts` + `_middleware.ts` + `assets/`).
Remover manifest v1. Suportar **global routing manifest** (domínio → função).

## Alternativas consideradas

- **Somente single-handler + roteamento no código do usuário:** simples, mas empurra complexidade ao usuário.
- **Manter v1 + v2:** dívida de compatibilidade; optou-se por remover v1 cedo (base pequena).

## Consequências

### Positivas
- Convenções tipo Next.js/Remix; REST com dispatch por método; validação em build-time.

### Negativas / Trade-offs
- Camada de matching (estático/dinâmico/catch-all, precedência, conflitos) adiciona complexidade.
- Breaking change v1→v2 (aceitável pela maturidade inicial).

## Referências
- `runtime/schemas/function-manifest.v2.schema.json`, `routing-manifest.v1.schema.json`
- `runtime/crates/server/src/{function_route_matcher.rs,global_routing.rs}`
