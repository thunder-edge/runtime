---
id: ADR-RUNTIME-0002
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
related_adrs: [ADR-RUNTIME-0001, ADR-RUNTIME-0003]
created_at: 2026-06-26
---

# ADR 0002: ESZIP como formato canônico de bundle

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (vigente desde a fundação)
**Owner:** Edge Platform

## Contexto

Funções precisam de um formato de empacotamento estável para deploy, que preserve o grafo de
módulos, sources e source maps, e permita cold-start rápido.

## Decisão

Usar **ESZIP** (formato serializado de grafo de módulos do Deno) como formato canônico de bundle,
carregado pelo `module_loader.rs` com caminhos síncrono (`eszip`) e assíncrono.

## Alternativas consideradas

- **Bundle de source TS/JS cru:** simples, mas exige parse/transpile a cada boot.
- **Tarball custom:** controle total, porém reinventa resolução de módulos e tooling.

## Consequências

### Positivas
- Grafo pré-serializado → boot mais rápido; inclui source maps opcionais.
- Ferramentas maduras (`deno_ast`, `deno_graph`).

### Negativas / Trade-offs
- Formato específico do Deno (não portável a outros runtimes).
- Requer tooling de build-time para gerar o ESZIP.

## Referências
- `runtime/crates/functions/src/types.rs` (`BundleFormat::Eszip`), `lifecycle.rs` (`parse_eszip_bundle`)
