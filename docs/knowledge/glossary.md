---
id: KNOWLEDGE-RUNTIME-GLOSSARY-001
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

# Glossário — Linguagem Ubíqua do Thunder Runtime

| Termo | Significado |
|-------|-------------|
| **Function** | Unidade de código JS/TS deployável que atende requests HTTP sob `/{function_name}/*`. |
| **Isolate** | Instância `JsRuntime` (V8 + extensões Deno) em thread dedicada; fronteira de isolamento. |
| **Context** | Contexto V8 dentro de um isolate; evolução para multi-context por isolate (ROADMAP_CONTEXT_ISOLATE). |
| **execution_id** | UUID por request usado para rastrear e limpar timers/intervals/promises/recursos. |
| **ESZIP** | Formato de bundle (grafo de módulos pré-serializado do Deno); formato canônico. |
| **Snapshot** | Cache de bytecode V8 para cold-start rápido, com fallback para ESZIP. |
| **Manifest (v2)** | Descritor da função (flavor, rotas, recursos, network allowlist) — JSON Schema 2020-12. |
| **Flavor** | Tipo de deploy: `single` (um handler) ou `routed-app` (rotas por arquivo + assets). |
| **Ingress** | Listener HTTP público (`/{function}/*`); padrão porta 8080 ou Unix socket. |
| **Admin API** | Listener de controle (`/_internal/*`), auth `X-API-Key`; padrão porta 9000. |
| **Egress** | Tráfego de saída iniciado pelo código de usuário (`fetch`, TCP, TLS). |
| **Connection Manager** | Gestor global de leases de egress (FD budget, token bucket, quotas, reaper). |
| **SSRF deny list** | Bloqueio padrão de ranges IP privados no egress; override por manifest. |
| **VFS** | Virtual File System: `/bundle` (RO), `/tmp` (RW, quota), `/dev/null`. |
| **node_compat** | Camada de compatibilidade Node.js (~35 módulos, full/partial/stub). |
| **thunder:testing** | Biblioteca de testes embutida (`edge://assert/*`) executada via `thunder test`. |
| **Bundle signing** | Verificação Ed25519 opcional da integridade do bundle no deploy. |
| **Watchdog** | Mecanismo que aplica limites de CPU/wall-clock e força término de execução. |

## Sinônimos proibidos

| Não usar | Usar |
|----------|------|
| Worker (para isolate) | Isolate |
| Lambda / serverless fn | Function |
| Endpoint admin | Admin API |
| Zip / pacote | ESZIP / bundle |
