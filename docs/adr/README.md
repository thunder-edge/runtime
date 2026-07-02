---
id: ADR-RUNTIME-INDEX
type: adr-index
domain: Platform
hardness: H3
owner: Thunder Runtime
team: Edge Platform
status: ACTIVE
created_at: 2026-06-26
---

# ADRs — Thunder Runtime

Architecture Decision Records **retroativos**, reconstruídos do código, docs internas e
histórico Git (Legacy Discovery Mode — ver `CLAUDE.md` §17). Status `ACCEPTED (retroativo)`
significa: decisão já vigente no código, documentada a posteriori.

| ADR | Título | Status | Hardness |
|-----|--------|--------|----------|
| [0001](./0001-deno-stack-v8-isolates.md) | Adotar Deno stack (deno_core + V8) para execução JS/TS | ACCEPTED (retroativo) | H3 |
| [0002](./0002-eszip-bundle-format.md) | ESZIP como formato canônico de bundle | ACCEPTED (retroativo) | H3 |
| [0003](./0003-snapshot-strategy-eszip-fallback.md) | Snapshot V8 com fallback ESZIP | ACCEPTED (retroativo) | H3 |
| [0004](./0004-isolate-pool-multi-context.md) | Pool de isolates por função → multi-context por isolate | ACCEPTED (retroativo) | H3 |
| [0005](./0005-dual-listener-architecture.md) | Arquitetura dual-listener (ingress + admin) | ACCEPTED (retroativo) | H3 |
| [0006](./0006-execution-id-watchdog-cleanup.md) | execution_id + watchdog para limpeza de recursos | ACCEPTED (retroativo) | H3 |
| [0007](./0007-ssrf-deny-list.md) | Proteção SSRF por deny list + allowlist por manifest | ACCEPTED (retroativo) | H3 |
| [0008](./0008-egress-connection-manager.md) | Egress Connection Manager global (FD budget) | ACCEPTED (retroativo) | H3 |
| [0009](./0009-filesystem-routing-manifest-v2.md) | Roteamento por filesystem (routed-app) + manifest v2 | ACCEPTED (retroativo) | H3 |
| [0010](./0010-opentelemetry-observability.md) | Observabilidade via OpenTelemetry (OTLP) | ACCEPTED (retroativo) | H3 |
| [0011](./0011-node-compat-layer.md) | Camada de compatibilidade Node.js (parcial, VFS) | ACCEPTED (retroativo) | H3 |
| [0012](./0012-bundle-signing-ed25519.md) | Bundle signing com Ed25519 | ACCEPTED (retroativo) | H3 |
| [0013](./0013-rename-thunder-cli-first.md) | Rename para `thunder` e postura CLI-first | ACCEPTED (retroativo) | H2 |
| [0014](./0014-rust-workspace-multi-crate.md) | Workspace Rust multi-crate | ACCEPTED (retroativo) | H3 |

## Convenção

- Novos ADRs: copiar o formato de um existente, numerar sequencialmente, status `PROPOSED`.
- Mudanças que quebram contrato H3: exigem ADR + plano de depreciação (ver `CLAUDE.md` Hardness Governance).
