---
id: ADR-RUNTIME-0013
type: adr
status: ACCEPTED
hardness: H2
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0013: Rename para `thunder` e postura CLI-first

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (rename executado na Fase 1)
**Owner:** Edge Platform

## Contexto

O projeto nasceu como `deno-edge-runtime`, nome que amarra a identidade à dependência (Deno) e
não comunica o produto. A experiência primária é via linha de comando (dev/CI/operação).

## Decisão

Renomear o produto e o binário para **`thunder`** em toda a base (CLI, telemetria, docs, schemas,
mensagens de log) e consolidar a **postura CLI-first**: `start`, `bundle`, `watch`, `test`, `check`,
com framework de testes embutido (`thunder:testing`, migrado de `edge://`).

## Alternativas consideradas

- **Manter `deno-edge-runtime`:** acopla marca à dependência; confuso para usuários.

## Consequências

### Positivas
- Identidade de produto própria; DX coesa via CLI única.

### Negativas / Trade-offs
- Breaking change de nomes/imports (`edge` → `thunder`); exigiu migração de testes e URIs de schema.

## Referências
- Commit "Rename deno-edge-runtime to thunder across the codebase"
- `runtime/crates/cli/src/main.rs`, `runtime/docs/reference/cli.md`
