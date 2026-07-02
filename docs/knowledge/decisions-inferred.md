---
id: KNOWLEDGE-RUNTIME-DECISIONS-001
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

# Decisões inferidas do histórico Git — Thunder Runtime

Reconstrução das decisões de arquitetura a partir dos 136 commits do `runtime/`
(2026-03-04 → 2026-04-07). Cada fase abaixo condensa o tema dominante dos commits.
Decisões significativas foram promovidas a **ADRs retroativos** (ver [../adr/README.md](../adr/README.md)).

## Linha do tempo por fase

### Fase 0 — Fundação (03-04 a 03-05)
- `first commit`; base do `JsRuntime` + permissões para testes de Web API.
- Biblioteca de asserção e framework de testes (`thunder:testing` / `edge://assert`).
- Comandos `watch` e `check`; typecheck TS pré-bundle.
- Inspector CDP (compatível VS Code / Chrome DevTools / nvim-dap), flags `--inspect`.
- Testes de compatibilidade com APIs Cloudflare Workers e Web Standards.
- **Perf:** dispatch de requests via API V8 em vez de `execute_script`.
- → **ADR-0001** (Deno stack), **ADR-0014** (workspace multi-crate), **ADR-0013** (CLI-first + testing lib).

### Fase 1 — Segurança & hardening de servidor (03-06)
- **TLS** com geração de cert self-signed; depois TLS dinâmico com hot-reload.
- **Dual-listener** (ingress + admin). → **ADR-0005**.
- **SSRF protection** + body size limits. → **ADR-0007**.
- CI/CD (GitHub Actions); auditoria de segurança (`AUDIT.md`).
- Rate limiting middleware; graceful shutdown; cache de métricas com TTL.
- `sanitize_internal_error`; trace context com correlation ID.
- **Global API hardening** (impede overwrite de primitivas críticas).
- **Function manifest** v1 + network allowlist.
- **Isolate pooling** + LRU eviction. → **ADR-0004**.
- **Bundle signing Ed25519**. → **ADR-0012**.
- Proxy de saída para HTTP/HTTPS/TCP.
- **Rename `deno-edge-runtime` → `thunder`** em toda a base. → **ADR-0013**.
- ⚠️ **Reversão notável:** "remove snapshot format support" (removido nesta fase). → contexto de **ADR-0003**.

### Fase 2 — Compatibilidade Node.js (03-07)
- `process` global mínimo; camada `node_compat` parcial.
- **VFS** para compat Node; `net`/`tls` subsets; **DNS-over-HTTPS**.
- `util`, `diagnostics_channel`, `async_hooks` (AsyncLocalStorage), `zlib`, `crypto`, `events`.
- Streams com **backpressure real**; `worker_threads` com erro determinístico.
- WebSocket client API; `http.createServer` stub. → **ADR-0011**.

### Fase 3 — Escala & performance (03-08 a 03-09)
- Egress: máximo de requests de saída por execução + checagem de integridade VFS. → **ADR-0008**.
- **Context pool scheduling** e métricas de routing. → **ADR-0004** (evolução).
- **Snapshot re-adicionado:** build script + criação de runtime base snapshot; métricas de snapshot. → **ADR-0003**.
- Router: capacidade de handler com pool connection e scale metrics.
- **Global routing manifest**. → **ADR-0009**.

### Fase 4 — Roteamento & manifest v2 (03-10 a 03-11)
- **Manifest v2** com routing + validação; remoção do v1. → **ADR-0009**.
- Migração de imports de testes `edge` → `thunder`.
- Node compatibility matrix docs; HTTP response helpers; request reference.
- `install.sh` (stable/unstable/tag/commit); packaging Windows no CI.
- Reserva de endereços distintos para probes de admin e ingress.

### Fase 5 — Manutenção (04-07)
- Comenta build Windows no CI/CD.

## Observabilidade transversal
- OpenTelemetry (traces/métricas/logs) evoluiu ao longo das fases 1–3. → **ADR-0010**.

## Reversões e decisões conscientes
| Decisão | Evento | ADR |
|---------|--------|-----|
| Snapshot format | Removido (Fase 1) e re-introduzido com fallback (Fase 3) | ADR-0003 |
| Manifest v1 → v2 | v1 removido em favor de v2 com routing | ADR-0009 |
| Nome do produto | `deno-edge-runtime` → `thunder` | ADR-0013 |
