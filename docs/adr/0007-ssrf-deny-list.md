---
id: ADR-RUNTIME-0007
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Security
created_at: 2026-06-26
---

# ADR 0007: Proteção SSRF por deny list + allowlist por manifest

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/AUDIT.md`

## Contexto

Código de usuário com `fetch()` pode ser usado para SSRF contra serviços internos (metadata de
cloud, redes privadas).

## Decisão

Bloquear por padrão ranges IP privados no egress (127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12,
192.168.0.0/16, 169.254.0.0/16, 0.0.0.0/8, `[::1]`), com **allowlist por função no manifest**
(`network.allow`). Enforcement no `permissions.rs`/`ssrf.rs` antes de qualquer conexão.

## Alternativas consideradas

- **Allowlist total (default-deny egress):** mais seguro, porém quebra casos comuns de uso.
- **Sem proteção, confiando na rede:** inaceitável para multi-tenant.

## Consequências

### Positivas
- Mitiga SSRF por padrão; override explícito e auditável por função.

### Negativas / Trade-offs
- Suporte a CIDR IPv6 incompleto → bypass conhecido via IPv4-mapped IPv6 (ver `AUDIT.md` NEW-C1).
- Allowlist adiciona metadados de deploy.

### Riscos
- Gap IPv6 deve ser fechado; rastreado na auditoria.

## Referências
- `runtime/crates/runtime-core/src/{ssrf.rs,permissions.rs}`
- `runtime/schemas/base/network.schema.json`, `runtime/AUDIT.md`
