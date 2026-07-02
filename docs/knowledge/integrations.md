---
id: KNOWLEDGE-RUNTIME-INTEGRATIONS-001
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

# Integrações externas — Thunder Runtime

Integrações e contratos observados no código e docs internas.

## 1. Dependências de plataforma (Deno stack)

Ecossistema `deno_*` sobre V8 (versões do `runtime/Cargo.toml`):

| Crate | Versão | Uso |
|-------|--------|-----|
| `deno_core` | 0.390 | Core JS runtime + bindings V8 |
| `deno_web` | 0.268 | APIs Web (fetch, streams, crypto...) |
| `deno_fetch` | 0.261 | HTTP fetch + TLS |
| `deno_websocket` | 0.242 | WebSocket cliente/servidor |
| `deno_crypto` | 0.251 | WebCrypto |
| `deno_net` / `deno_tls` | 0.229 / 0.224 | Sockets TCP/UDP e TLS |
| `deno_permissions` | 0.96 | Sistema de permissões (base da SSRF) |
| `deno_node` | 0.175 | Shims Node.js |
| `deno_fs` | 0.147 | VFS / filesystem |
| `deno_telemetry` | 0.59 | Ops de telemetria (OTEL) |

Runtime assíncrono: **Tokio**. HTTP: **Hyper + Tower**. TLS: **Rustls**.

## 2. Egress (saída de rede do código de usuário)

- Todo `fetch()`/TCP/TLS do usuário passa pelo **Egress Connection Manager** (`functions/src/connection_manager.rs`).
- **DNS-over-HTTPS** (DoH) configurável (padrão Cloudflare 1.1.1.1) para `node:dns`.
- **Proxy de saída** opcional para HTTP/HTTPS/TCP.
- **SSRF deny list** aplicada antes de qualquer conexão (ranges privados bloqueados por padrão).

## 3. Observabilidade (OTLP)

- Exportador OpenTelemetry via OTLP HTTP (padrão `http://127.0.0.1:4318`).
- Coletor encaminha: traces → Tempo, logs → Loki, métricas → Prometheus/Victoria Metrics.
- Endpoints internos: `/_internal/metrics` e `/metrics` (ver `runtime/docs/reference/metrics-endpoint-reference.md`).

## 4. Ferramentas externas opcionais

| Ferramenta | Uso | Obrigatória? |
|-----------|-----|--------------|
| `deno` CLI | typecheck semântico TS em `bundle`/`check` | Opcional |
| `k6` | load testing (`runtime/docs/guides/load_testing.md`) | Opcional |
| GitHub Releases | distribuição de binário via `install.sh` | Runtime |

## 5. Deploy / Admin API (contrato de controle)

- **Admin API** em `/_internal/*` (auth `X-API-Key`): deploy/update/delete de funções, reload, metrics, health.
- **Bundle signing**: header `x-bundle-signature-ed25519` (base64) validado contra chave pública configurada.
- Consumidores previstos: **dashboard/** (control plane), CLI, CI/CD.

## 6. Consumidores conhecidos

| Consumidor | Como integra |
|-----------|--------------|
| `dashboard/` | Control plane que gerencia funções via Admin API (ver `dashboard/OPENAPI.md`) |
| `tsuru/` | Executa o binário `thunder` no PaaS da Globo (perfil de pooling "throughput 1k") |
| `docs/` (público) | Documenta uso final do runtime (consolidação) |
