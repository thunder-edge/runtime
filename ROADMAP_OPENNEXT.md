# ROADMAP_OPENNEXT

> Status: Planejamento de Integracao e Convergencia
>
> Data: 10/03/2026
>
> Objetivo: habilitar uso futuro de Next.js via wrapper/adapter OpenNext sobre o runtime atual, sem quebrar isolamento, seguranca e contrato operacional.
>
> Referencias: `ROADMAP.md` (Fase 6), `ROADMAP_ROUTING.md`, `docs/reference/function-manifest.md`, `docs/reference/NODE-COMPAT.md`

---

## 1. Premissas

- Runtime aceita apenas `manifestVersion: 2`.
- Prefixo canonico de deploy `/{function_id}/...` deve ser preservado.
- Sem co-hosting inseguro entre funcoes distintas no mesmo contexto.
- Sem acesso ao host fisico fora das politicas de sandbox.

---

## 2. Escopo da Trilha OpenNext

Esta trilha cobre:

- Adapter de build/deploy para converter saida OpenNext em artefato compativel com runtime.
- Contrato de roteamento multi-rota (`flavor: routed-app`, `routes[]`, assets).
- Garantia de semantica HTTP para SSR (cookies, headers, streaming, abort/cancelamento).
- Matriz de compatibilidade por feature de Next.js/OpenNext.

Nao cobre nesta fase:

- Features cloud proprietarias fora do perfil runtime-only.
- Emulacao irrestrita de APIs Node fora da politica de sandbox.

---

## 3. Fases de Entrega

### Fase A - Adapter Contract e PoC

- Definir contrato de entrada do adapter OpenNext:
  - metadados de build,
  - tabela de rotas,
  - mapa de assets,
  - regras de rewrite/redirect.
- Definir contrato de saida para runtime:
  - `manifest v2` com `flavor: routed-app`,
  - `routes[]` com `kind: function` e `kind: asset`,
  - bundle deployavel via `POST /_internal/functions`.
- Entregar PoC com app Next.js de referencia:
  - rota estatica,
  - rota dinamica,
  - SSR basico.

Criterio de aceite:

- build OpenNext gera artefato valido para deploy no runtime;
- app referencia responde em `/{function_id}/...`.

### Fase B - Semantica HTTP e Assets

- Fechar mapeamento de headers/cookies para evitar regressao em SSR.
- Garantir preservacao de `Set-Cookie` multiplo, `content-encoding` e corpo streamado.
- Implementar politica de assets:
  - short-circuit de assets sem entrar no isolate quando aplicavel,
  - fallback deterministico para rotas de app.
- Garantir comportamento consistente de cancelamento/timeout em streaming.

Criterio de aceite:

- SSR streaming estavel e sem corrupcao de body;
- rotas e assets respeitam prioridade deterministica.

### Fase C - Rewrites, Redirects e Roteamento Avancado

- Implementar traducao de rewrites/redirects OpenNext para regras do ingress.
- Validar conflitos de rota em build time (estatica vs dinamica vs catch-all).
- Integrar com manifesto global de dominios quando habilitado.

Criterio de aceite:

- rewrites/redirects de referencia funcionam sem ambiguidade;
- conflitos falham em build/deploy com erro claro.

### Fase D - Compatibilidade e Operacao

- Publicar matriz OpenNext por feature critica:
  - SSR,
  - RSC,
  - server actions,
  - headers/cookies,
  - middleware,
  - image.
- Definir niveis: `Full`, `Partial`, `None`.
- Publicar playbook operacional:
  - tuning de pool/context,
  - limites conhecidos,
  - troubleshooting,
  - rollback.

Criterio de aceite:

- decisao de adocao possivel sem ler codigo-fonte;
- regressoes criticas cobertas por E2E e gate em CI.

---

## 4. Backlog Executavel por PR

## PR1 - Contract e Manifest Mapping

Escopo:

- Adapter contract versionado.
- Conversao para `manifest v2` + `routes[]`.
- Validacao de schema e semantica no pipeline.

Dependencias:

- `ROADMAP.md` 6.2 (scan/metadata/deploy routed-app).

## PR2 - Assets e Streaming Basico

Escopo:

- Mapeamento de `public/` e assets de build.
- Curto-circuito de assets no ingress quando possivel.
- E2E de SSR streaming com app de referencia.

Dependencias:

- `ROADMAP.md` 6.2 e 6.3.

## PR3 - Rewrites/Redirects e Priorizacao

Escopo:

- Traducao de rewrites/redirects para runtime.
- Deteccao de colisoes de rota e prioridade deterministica.

Dependencias:

- `ROADMAP.md` 6.3.

## PR4 - Matriz de Compatibilidade e Operacao

Escopo:

- Matriz OpenNext (Full/Partial/None).
- Testes E2E de regressao com gate em CI.
- Playbook de rollout/rollback.

Dependencias:

- `ROADMAP.md` 6.4, 6.5, 6.6.

---

## 5. Mapeamento com ROADMAP Principal

- `ROADMAP.md` 6.2:
  - scan de `functions/`, metadata de rotas, deploy `routed-app`, assets.
- `ROADMAP.md` 6.3:
  - ingress em dois estagios, matching deterministico e prefixo canonico.
- `ROADMAP.md` 6.4:
  - contrato RESTful `export default` e `405/Allow`.
- `ROADMAP.md` 6.5:
  - E2E, migracao e observabilidade por rota.
- `ROADMAP.md` 6.6:
  - dominio global host/path -> function.

---

## 6. Riscos e Mitigacoes

Risco: divergencia de semantica HTTP em SSR.
Mitigacao: suite E2E dedicada para cookies/headers/streaming com app OpenNext.

Risco: incompatibilidade Node em features especificas do ecossistema Next.
Mitigacao: matriz de compatibilidade e falha deterministica para `None`.

Risco: regressao operacional por carga/concorrencia.
Mitigacao: testes de carga e tuning de pool/context com limites documentados.

---

## 7. Gate de Pronto para Producao

- App Next.js de referencia sobe no runtime com adapter OpenNext.
- SSR streaming, rotas dinamicas e cookies funcionam com semantica estavel.
- Matriz OpenNext publicada e versionada.
- CI com gate de regressao para cenarios criticos.
- Playbook de operacao e rollback publicado.
