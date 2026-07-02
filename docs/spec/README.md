---
id: SPEC-RUNTIME-INDEX
type: spec-index
domain: Platform
hardness: H3
owner: Thunder Runtime
team: Edge Platform
status: ACTIVE
created_at: 2026-06-26
---

# Specs — Thunder Runtime

Nenhuma spec formal criada ainda (projeto mapeado retroativamente via Knowledge + ADRs).

Para criar uma nova spec, siga `CLAUDE.md` §6 e §7 e salve como `runtime/docs/spec/<componente>.md`.
Candidatos prioritários (derivados da Knowledge Base):

| Componente candidato | Base | Hardness |
|----------------------|------|----------|
| Function lifecycle & pool | knowledge/architecture.md §4, ADR-0004, ADR-0006 | H3 |
| Ingress & Admin API | ADR-0005, docs/reference/cli.md | H3 |
| Egress connection manager | ADR-0008, docs/design/egress-connection-manager.md | H3 |
| SSRF & network allowlist | ADR-0007, AUDIT.md | H3 |
| Function manifest v2 & routing | ADR-0009, docs/reference/function-manifest.md | H3 |
| Node compat & VFS | ADR-0011, docs/reference/NODE-COMPAT.md, reference/vfs.md | H3 |
| thunder:testing library | docs/guides/testing-library.md | H2 |
