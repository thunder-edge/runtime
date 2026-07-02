---
id: KNOWLEDGE-RUNTIME-DATAFLOW-001
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

# Fluxos de negócio — Thunder Runtime

## 1. Deploy de função (Admin API)

```mermaid
sequenceDiagram
    participant OP as Operador/Dashboard/CLI
    participant AR as Admin Router (:9000)
    participant SIG as Bundle Signature
    participant REG as Function Registry
    participant LC as Lifecycle

    OP->>AR: POST /_internal/... (bundle + X-API-Key)
    AR->>AR: valida X-API-Key
    AR->>SIG: verifica assinatura Ed25519 (se exigida)
    SIG-->>AR: ok / 400
    AR->>REG: registra função (manifest v2 validado)
    REG->>LC: boot isolate(s) em thread(s) dedicada(s)
    LC->>LC: carrega snapshot → fallback ESZIP
    LC-->>REG: FunctionEntry pronto
    REG-->>OP: 2xx
```

## 2. Execução de request (ingress)

```mermaid
sequenceDiagram
    participant CL as Client
    participant IR as Ingress Router (:8080)
    participant REG as Registry
    participant RT as JsRuntime

    CL->>IR: HTTP /{function}/path
    IR->>IR: rate limit, body limit, request_id, traceparent
    IR->>REG: rota por nome da função
    REG->>REG: seleciona isolate (round-robin), gera execution_id
    REG->>RT: IsolateRequest
    RT->>RT: watchdog (CPU + wall-clock), executa handler
    RT-->>REG: Response (stream opcional)
    REG->>REG: clearExecutionTimers(execution_id)
    REG-->>IR: Response
    IR-->>CL: HTTP (sanitiza erros internos)
```

## 3. Egress do código de usuário

```mermaid
flowchart LR
    F["user fetch()"] --> SSRF{SSRF deny list?}
    SSRF -->|bloqueado| ERR[erro]
    SSRF -->|permitido| CM[Egress Connection Manager]
    CM --> B{FD budget / quota / token bucket}
    B -->|lease| NET[conexão externa]
    B -->|saturado| REJ[rejeita/queue]
    NET --> REAP[reaper recicla leases via TTL]
```

## 4. Timeout e limpeza de recursos

- **CPU time** (`cpu_timer.rs`) via `CLOCK_THREAD_CPUTIME_ID`; flag de excedido encerra execução.
- **Wall-clock** (`wall_clock_timeout_ms`) com watchdog + `tokio::time::timeout` (dupla camada).
- **OOM** (`mem_check.rs`): callback de limite de heap V8, extensão em estágios + terminação graciosa.
- Timeout de isolate → HTTP **504**; isolate morto → auto-restart com backoff exponencial.

## 5. Modo de desenvolvimento (`watch`)

```mermaid
flowchart LR
    FS[arquivos .ts/.js] -->|notify watcher| W[cli watch]
    W --> BND[rebundle ESZIP]
    BND --> HR[hot update no registry]
    HR --> RUN[isolate atualizado sem downtime]
```
