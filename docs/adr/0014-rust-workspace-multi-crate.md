---
id: ADR-RUNTIME-0014
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Platform
created_at: 2026-06-26
---

# ADR 0014: Workspace Rust multi-crate

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26 (vigente desde a fundação)
**Owner:** Edge Platform

## Contexto

O runtime combina execução V8, ciclo de vida de funções, ingress HTTP e tooling. Manter tudo em
uma única crate dificultaria fronteiras, testes e tempos de compilação.

## Decisão

Organizar como **workspace Cargo com 4 crates**: `runtime-core` (execução/sandbox), `functions`
(lifecycle/registry/egress), `server` (ingress/admin/TLS) e `cli` (binário `thunder`). Dependências
compartilhadas no `[workspace.dependencies]` do `runtime/Cargo.toml`; `Cargo.lock` commitado.

## Alternativas consideradas

- **Crate única:** simples no início, mas acopla domínios e piora build incremental.
- **Repos separados por crate:** overhead de versionamento cross-repo prematuro.

## Consequências

### Positivas
- Fronteiras de domínio explícitas (ver knowledge/domains.md); build incremental; testes por crate.

### Negativas / Trade-offs
- Coordenação de versões internas; algumas mudanças cruzam múltiplas crates (lifecycle ↔ server).

## Referências
- `runtime/Cargo.toml` (`[workspace] members = [...]`)
- `runtime/crates/{runtime-core,functions,server,cli}`
