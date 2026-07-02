---
id: KNOWLEDGE-RUNTIME-DOMAINS-001
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

# Bounded Contexts — Thunder Runtime

Mapa de domínios e fronteiras de responsabilidade inferidas do código e do histórico Git
(arquivos que mudam juntos tendem a pertencer ao mesmo contexto).

## Contextos

```mermaid
flowchart TB
    subgraph Execution["Execution Context (runtime-core)"]
        ISO[Isolate / JsRuntime]
        EXT[Web Extensions]
        LIM[Resource Limits]
        NODE[Node Compat + VFS]
    end
    subgraph Lifecycle["Function Lifecycle (functions)"]
        REG[Registry]
        LC[Lifecycle / boot]
        HND[Handler / dispatch]
        CM[Egress Connection Manager]
        SNAP[Snapshot loader]
    end
    subgraph Ingress["HTTP Ingress & Control (server)"]
        IR[Ingress Router]
        AR[Admin Router]
        TLS[TLS / hot-reload]
        SIG[Bundle Signature]
        GR[Global Routing]
    end
    subgraph Tooling["Developer Tooling (cli)"]
        START[start]
        BUNDLE[bundle]
        WATCH[watch]
        TEST[test]
        CHECK[check]
    end

    Tooling --> Ingress
    Ingress --> Lifecycle
    Lifecycle --> Execution
```

| Bounded Context | Crate | Responsabilidade | NÃO é responsável por |
|-----------------|-------|------------------|------------------------|
| **Execution** | `runtime-core` | Executar JS/TS com segurança, aplicar limites, prover APIs Web e Node | Roteamento HTTP, deploy, autenticação de admin |
| **Function Lifecycle** | `functions` | Deploy/update/delete, pool de isolates, routing por nome, egress leasing, métricas por função | Terminação TLS, parsing de HTTP bruto |
| **HTTP Ingress & Control** | `server` | Ingress público, Admin API, TLS, limites de corpo, assinatura de bundle, trace context | Execução de código de usuário, gestão de heap V8 |
| **Developer Tooling** | `cli` | UX de dev/CI/operação: start/bundle/watch/test/check, telemetria | Lógica de negócio de execução (delega às crates) |

## Fronteiras e contratos internos

- `server → functions`: roteia request pelo primeiro segmento do path para o `FunctionRegistry`.
- `functions → runtime-core`: dispatch de `IsolateRequest` via canal para o `JsRuntime` na thread do isolate.
- `cli → server/functions`: `start` compõe listeners + registry; `bundle` gera ESZIP/snapshot; `test` roda no isolate.
- Contrato de função de usuário: `Deno.serve()` / handler exportado (ver `runtime/docs/reference/FUNCTION_CONTRACT_README.md`).

## Sinais de acoplamento (do histórico Git)

Arquivos mais voláteis e que co-evoluem: `functions/src/lifecycle.rs`, `server/src/lib.rs`,
`server/src/router.rs`, `functions/src/registry.rs`, `functions/src/handler.rs` — indicando que
o eixo **lifecycle ↔ ingress ↔ registry** é o núcleo quente do sistema e o mais sensível a regressões.
