---
id: KNOWLEDGE-RUNTIME-INDEX
type: knowledge
domain: Platform
hardness: H3
owner: Thunder Runtime
team: Edge Platform
status: ACTIVE
confidence: HIGH
created_at: 2026-06-26
updated_at: 2026-06-26
---

# Knowledge Base — Thunder Runtime

> **Escopo:** documenta o estado **atual** (as-is) do `runtime/`. Não é fonte de verdade
> comportamental — Specs e ADRs prevalecem. Ver hierarquia em `CLAUDE.md` §2 e §18.
>
> Ordem de precedência: **ADR > SPEC > KNOWLEDGE > CÓDIGO**.

Base construída retroativamente (Legacy Discovery Mode) a partir de:

- Código-fonte das crates (`runtime/crates/*`)
- Documentação técnica interna (`runtime/docs/*.md`)
- Roadmaps temáticos (`runtime/ROADMAP*.md`)
- Auditoria de segurança (`runtime/AUDIT.md`) e análise de arquitetura (`runtime/CURRENT_ARCHITECTURE_ANALYSIS.md`)
- Histórico Git (136 commits, 2026-03-04 → 2026-04-07)

## Documentos

| Documento | Conteúdo |
|-----------|----------|
| [architecture.md](./architecture.md) | Visão geral da arquitetura, crates, fluxo de request, topologia de listeners |
| [domains.md](./domains.md) | Bounded contexts e fronteiras de responsabilidade |
| [integrations.md](./integrations.md) | Integrações externas e contratos observados |
| [data-flow.md](./data-flow.md) | Fluxos de negócio (deploy, request, egress, timeout) |
| [decisions-inferred.md](./decisions-inferred.md) | Decisões inferidas do histórico Git → índice de ADRs retroativos |
| [glossary.md](./glossary.md) | Linguagem ubíqua do domínio de edge runtime |

## Artefatos relacionados

- ADRs retroativos: [../adr/README.md](../adr/README.md)
- Docs técnicas internas: [../](../) (cli.md, function-manifest.md, vfs.md, etc.)
- Roadmaps: `runtime/ROADMAP*.md`
