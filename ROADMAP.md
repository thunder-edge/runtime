# Roadmap de Correções — Deno Edge Runtime

> Baseado na auditoria de segurança e arquitetura realizada em 05/03/2026.
> Cada item referencia o finding correspondente no `AUDIT.md`.
>
> Última atualização: 06/03/2026 (TLS 0.1 concluída + base em `git log` + `git diff`).
> Commits de referência: `92aa473`, `6607a2b`, `4933dda`.
> Inclui também mudanças locais ainda não commitadas em `functions/runtime-core`.

---

## Fase 0 — Crítico (Pré-Produção)

> Itens que **bloqueiam** qualquer uso em produção. Devem ser resolvidos antes de expor o runtime a tráfego externo.

### 0.1 Implementar TLS de Verdade

**Ref:** AUDIT §1.1
**Crate:** `server`
**Arquivo:** `crates/server/src/lib.rs`

- [x] Usar o `tls_acceptor` retornado por `build_tls_acceptor()` para envolver o TCP stream
- [x] Chamar `tls_acceptor.accept(stream).await` antes de passar para hyper
- [x] Servir plain HTTP apenas se TLS config não for fornecida
- [x] Adicionar teste E2E com conexão TLS real (self-signed cert)
- [x] Logar warning se servidor iniciar sem TLS

**Status:** ✅ Concluído

**Detalhes de implementação:**
```rust
// No accept loop (com fallback para HTTP plain):
let maybe_stream = if let Some(acceptor) = tls_acceptor {
    let tls_stream = acceptor.accept(stream).await?;
    tls::MaybeHttpsStream::TcpTls(tls_stream)
} else {
    tls::MaybeHttpsStream::TcpPlain(stream)
};

let io = TokioIo::new(maybe_stream);
```

**Validação adicionada:**
- Teste E2E `tests::e2e_tls_accepts_https_connection` em `crates/server/src/lib.rs`
- Certificado self-signed gerado em runtime de teste, handshake TLS real e request HTTP sobre canal criptografado
- Warnings explícitos quando listeners iniciam sem TLS (admin, ingress TCP e legado)

---

### 0.2 Autenticação nos Endpoints `/_internal`

**Ref:** AUDIT §1.2
**Crate:** `server`
**Arquivo:** `crates/server/src/router.rs`

- [x] Definir variável de ambiente `EDGE_RUNTIME_API_KEY` (ou flag CLI `--api-key`)
- [x] Extrair campo `api_key: Option<String>` no `ServerConfig`
- [x] No `handle_internal()`, verificar header `X-API-Key` contra o valor configurado
- [x] Retornar `401 Unauthorized` se key ausente/incorreta
- [x] Se nenhuma key configurada, logar warning e aceitar (modo dev)
- [x] Adicionar testes unitários para auth success/failure/missing

**Status:** ✅ Concluído

**Implementação:**
- Arquitetura de dual-listener separando admin (porta 9000) e ingress (porta 8080 ou Unix socket)
- Admin router com autenticação via header `X-API-Key`
- Ingress router rejeita `/_internal/*` com 404
- Suporte a Unix socket para ingress
- Novos arquivos: `admin_router.rs`, `ingress_router.rs`

---

### 0.3 Bloquear SSRF (IPs Privados no `fetch`)

**Ref:** AUDIT §1.3
**Crate:** `runtime-core`
**Arquivo:** `crates/runtime-core/src/permissions.rs`

- [x] Implementar bloqueio de IPs privados (equivalente ao `is_private_ip`) via denylist de ranges
  - `127.0.0.0/8` (loopback)
  - `10.0.0.0/8` (RFC 1918)
  - `172.16.0.0/12` (RFC 1918)
  - `192.168.0.0/16` (RFC 1918)
  - `169.254.0.0/16` (link-local / metadata de cloud)
  - `0.0.0.0/8`
    - `[::1]` (IPv6 loopback)
    - `fc00::/7`, `fe80::/10` (TODO: pendente por limitação do parser `deno_permissions` para CIDR IPv6 nesta versão)
- [x] Adicionar `deny_net` com esses ranges na `create_permissions_with_ssrf_protection()`
- [x] Manter `allow_net: Some(vec![])` para hosts públicos
- [x] Adicionar testes que confirmem bloqueio de `fetch("http://169.254.169.254/...")`
- [x] Adicionar testes que confirmem que `fetch("https://api.github.com/")` funciona

**Status:** ✅ Concluído (com ressalva IPv6 CIDR)

---

### 0.4 Limitar Tamanho de Request/Response Body

**Ref:** AUDIT §1.4
**Crate:** `server`
**Arquivo:** `crates/server/src/router.rs`

- [x] Definir limites default para request/response (5 MiB / 10 MiB), configuráveis via CLI/env
- [x] Antes de coletar body, verificar `Content-Length` header
- [x] Se `Content-Length > MAX`, retornar `413 Payload Too Large` imediatamente
- [x] Após iniciar coleta, impor limite de leitura também sem `Content-Length` (`http_body_util::Limited`)
- [x] Definir `MAX_RESPONSE_BODY_BYTES` (default: 10 MiB) no handler
- [x] Truncar error messages em logs para max 1 KiB
- [x] Adicionar testes com payloads oversized

**Status:** ✅ Concluído

---

### 0.5 Limitar Conexões Simultâneas

**Ref:** AUDIT §2.1
**Crate:** `server`
**Arquivo:** `crates/server/src/lib.rs`

- [x] Adicionar `max_connections: usize` ao `ServerConfig` (default: 10.000)
- [x] Criar `tokio::sync::Semaphore` com o limite configurado
- [x] Adquirir permit antes de `tokio::spawn` no accept loop
- [x] Se sem permits disponíveis, dropar a conexão com log warning
- [x] Adicionar flag CLI `--max-connections`

**Status:** ✅ Concluído

```rust
let semaphore = Arc::new(Semaphore::new(config.max_connections));

// No accept loop:
let permit = semaphore.clone().try_acquire_owned();
match permit {
    Ok(permit) => {
        tokio::spawn(async move {
            let _permit = permit; // Dropped no fim da conexão
            // ... serve connection
        });
    }
    Err(_) => {
        warn!("connection limit reached, dropping connection from {}", peer_addr);
        drop(stream);
    }
}
```

---

## Fase 1 — Alta Prioridade (Semana 1-2)

> Itens que previnem crashes, resource exhaustion e comportamento incorreto.

### 1.1 Request Timeout no Isolate

**Ref:** AUDIT §2.5
**Crate:** `functions`
**Arquivo:** `crates/functions/src/lifecycle.rs`

- [x] Envolver `handler::dispatch_request()` com `tokio::time::timeout()`
- [x] Usar `config.wall_clock_timeout_ms` como timeout
- [x] Retornar HTTP 504 Gateway Timeout quando exceder
- [x] Logar timeout com nome da função e duração
- [x] Incrementar `metrics.total_errors` em timeout
- [x] Adicionar teste com handler que faz `while(true) {}`

**Status:** ✅ Concluído

---

### 1.2 Near-Heap-Limit Callback no V8

**Ref:** AUDIT §2.3
**Crate:** `functions`
**Arquivo:** `crates/functions/src/lifecycle.rs`

- [x] Registrar `v8::Isolate::add_near_heap_limit_callback()` na criação do isolate
- [x] No callback, logar warning e retornar `current_heap + small_delta` (última chance)
- [x] Se chamado segunda vez, terminar o isolate
- [x] Marcar função como `Error` no registry
- [x] Adicionar teste com código que aloca memória infinitamente

TODO (futuro): expor este evento como métrica por função (ex.: `heap_limit_terminations_total`) para observabilidade e alertas.

**Status:** ✅ Concluído

---

### 1.3 Recovery de Panic no Isolate

**Ref:** AUDIT §2.4
**Crate:** `functions`
**Arquivo:** `crates/functions/src/lifecycle.rs`

- [x] Detectar isolate morto e evitar roteamento para handle inválido (`IsolateHandle::alive`)
- [x] Após `catch_unwind` capturar panic, atualizar status para `Error` no registry
- [x] Fechar o `request_tx` channel para que requests pendentes recebam erro
- [x] Implementar auto-restart com backoff exponencial (1s, 2s, 4s, 8s, max 60s)
- [x] Limitar número de restarts consecutivos (max 5)
- [x] Logar cada restart com counter
- [x] Adicionar teste de panic seguido de request

**Status:** ✅ Concluído

---

### 1.4 Reset do CPU Timer por Request

**Ref:** AUDIT §2.6
**Crate:** `runtime-core`
**Arquivo:** `crates/runtime-core/src/cpu_timer.rs`

- [x] Adicionar método `reset` que zera `accumulated_ms` e `exceeded`
- [x] Chamar `reset()` antes de cada `dispatch_request`
- [x] Adicionar teste cobrindo reuso do mesmo timer após reset

**Status:** ✅ Concluído

---

### 1.5 Validar Nome de Função

**Ref:** AUDIT §3.5
**Crate:** `server`
**Arquivo:** `crates/server/src/router.rs`

- [x] Criar função `fn is_valid_function_name(name: &str) -> bool`
- [x] Regex: `^[a-z0-9][a-z0-9-]{0,62}$`
- [x] Validar no deploy (`POST /_internal/functions`)
- [x] Validar no ingress (retornar 400 se inválido)
- [x] Adicionar testes com nomes: válidos, com `..`, com `/`, unicode, vazio, muito longo

**Status:** ✅ Concluído

---

### 1.6 Ativar Rate Limiter

**Ref:** AUDIT §3.1
**Crate:** `server`
**Arquivo:** `crates/server/src/lib.rs`

- [x] Aplicar `RateLimitLayer` da middleware ao serviço HTTP se `rate_limit_rps` configurado
- [x] Retornar `429 Too Many Requests` quando exceder
- [x] Adicionar header `Retry-After` na resposta 429

**Status:** ✅ Concluído

---

## Fase 2 — Média Prioridade (Semana 3-4)

> Melhorias de robustez, observabilidade e operational safety.

### 2.1 CPU Time Real (CLOCK_THREAD_CPUTIME_ID)

**Ref:** AUDIT §2.2
**Crate:** `runtime-core`
**Arquivo:** `crates/runtime-core/src/cpu_timer.rs`

- [x] Usar `libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID)` para medir CPU real
- [x] Manter wall-clock como fallback em plataformas sem suporte
- [x] Documentar diferença entre CPU time e wall-clock time
- [x] Adicionar benchmarks comparando ambas abordagens

**Status:** ✅ Concluído

Nota: benchmark comparativo adicionado como teste `#[ignore]` em `crates/runtime-core/src/cpu_timer.rs` (`benchmark_wall_clock_vs_thread_cpu_time`), executável manualmente via `cargo test -p runtime-core benchmark_wall_clock_vs_thread_cpu_time -- --ignored --nocapture`.

---

### 2.2 Graceful Shutdown Real

**Ref:** AUDIT §4.5 e §2.4
**Crates:** `server`, `functions`
**Arquivos:** `crates/server/src/lib.rs`, `crates/functions/src/registry.rs`

- [x] No shutdown, enviar `CancellationToken` para cada isolate
- [x] Esperar com deadline (ex: 10s) que todos os isolates terminem
- [x] Verificar `request_tx.is_closed()` para cada função
- [x] Após deadline, forçar clear com log warning
- [x] Adicionar teste de shutdown com requests in-flight

**Status:** ✅ Concluído

---

### 2.3 Cache do Endpoint de Metrics

**Ref:** AUDIT §3.2
**Crate:** `server`
**Arquivo:** `crates/server/src/router.rs`

- [x] Criar `MetricsCache` com TTL de 15 segundos
- [x] Armazenar resultado de `sysinfo::System` + function metrics
- [x] Retornar cache se não expirado
- [x] Usar `tokio::sync::RwLock` ou `parking_lot::RwLock`

**Status:** ✅ Concluído

---

### 2.4 Sanitizar Error Messages para Clientes

**Ref:** AUDIT §3.8
**Crate:** `server`
**Arquivo:** `crates/server/src/router.rs`

- [x] Criar enum `ClientError` com mensagens genéricas
- [x] Logar stack trace internamente com `tracing::error!`
- [x] Retornar ao cliente apenas: `{"error": "internal_error", "request_id": "..."}`
- [x] Incluir `request_id` (UUID) para correlação

**Status:** ✅ Concluído

---

### 2.5 Distribuited Tracing (W3C Trace Context)

**Ref:** AUDIT §5 (observações positivas — OpenTelemetry já nas deps)
**Crate:** `server`

- [x] Propagar headers `traceparent` e `tracestate` para dentro dos isolates
- [x] Criar span por request com function name, status, duration
- [x] Exportar via OTLP (já nas dependências)
- [x] Adicionar `correlation-id` header no response

**Status:** ✅ Concluído

Configuração de sampling: `EDGE_RUNTIME_TRACE_SAMPLE_PERCENT` (0..100), default `100`.
Política de trace inválido: descarta `traceparent` inválido e gera novo trace.
Política de correlação: `correlation-id` = `trace_id`.

---

### 2.6 Freeze de Globals no Bootstrap

**Ref:** AUDIT §4.2
**Crate:** `runtime-core`
**Arquivo:** `crates/runtime-core/src/bootstrap.js`

- [x] Após atribuir todas as APIs a `globalThis`, aplicar `Object.freeze()` nos critiais:
  - `fetch`, `Request`, `Response`, `Headers`
  - `crypto`, `URL`, `URLSearchParams`
  - `TextEncoder`, `TextDecoder`
  - `console`
- [x] Testar que user code não consegue sobrescrever `globalThis.fetch`

**Status:** ✅ Concluído

---

### 2.7 Proteger Inspector para Localhost

**Ref:** AUDIT §3.3
**Crate:** `runtime-core`

- [x] Forçar bind do inspector em `127.0.0.1`
- [x] Adicionar flag `--inspect-allow-remote` para override explícito
- [x] Documentar que inspector não deve ser usado em produção
- [x] Logar warning se inspector ativado

**Status:** ✅ Concluído

---

## Fase 3 — Melhoria Contínua (Mês 2+)

> Evolução de features e hardening avançado.

### 3.2 Streaming de Response Body

- [x] Substituir `bytes::Bytes` por body streaming no caminho de resposta HTTP
- [x] Suportar `ReadableStream` no response do user code
- [x] Permitir Server-Sent Events e chunked transfer

**Status:** ✅ Concluido

Implementacao atual:
- Pipeline isolate -> router retorna `IsolateResponseBody::{Full, Stream}`
- `ReadableStream` do user handler e drenado em chunks para o HTTP body
- Routers (`admin`/`ingress`) usam body boxeado compativel com full body e stream
- Documentacao de uso adicionada em `docs/streaming-response-body.md`

### 3.3 Isolate Pooling / Reuse

- [x] Pool de isolates quentes prontos para receber requests
- [x] Reutilizar isolate entre requests da mesma função
- [x] Pre-warm isolates para funções com alto tráfego
- [x] Evict LRU quando pool estiver cheio

**Status:** ✅ Concluído

Implementacao atual:
- Configuracao de pooling em nivel de processo via CLI (`--pool-enabled`, `--pool-global-max-isolates`, `--pool-min-free-memory-mib`)
- Limites dinamicos por funcao via API admin (`GET/PUT /_internal/functions/{name}/pool`)
- Escalonamento com bloqueio por memoria minima livre e logs de guardrail
- Roteamento round-robin entre handles da funcao (primary + replicas)
- Eviccao LRU explicita de replicas extras quando o limite global de isolates e atingido (preserva isolate primario)

### 3.4 Hot-Reload de Certificado TLS

- [x] Watch no cert/key file via `notify`
- [x] Rotacionar `TlsAcceptor` sem restart do servidor
- [x] Logar rotação com fingerprint do novo cert

**Status:** ✅ Concluído

Implementacao atual:
- `DynamicTlsAcceptor` com troca atomica do acceptor em runtime
- Watcher `notify` em background para cert/key com recarga e retries curtos
- Logs de carga inicial e reload com fingerprint SHA-256 do certificado

### 3.5 HTTP/3 (QUIC)

- [ ] Avaliar `quinn` ou `h3` crate
- [ ] Suportar QUIC listeners em paralelo com TCP
- [ ] ALPN negotiation para h2/h3

### 3.6 Module Integrity (Assinatura de Bundles)

- [ ] Assinar bundles eszip com HMAC-SHA256 ou Ed25519
- [ ] Verificar assinatura no load antes de execução
- [ ] Rejeitar bundles sem assinatura válida em modo produção

### 3.7 Resolver Paths Hardcoded no CLI

**Ref:** AUDIT §3.4

- [ ] Usar variável `EDGE_RUNTIME_ROOT` ou auto-detectar via `Cargo.toml` parent walk
- [ ] Ou embutir assets no binário via `include_str!` / `include_bytes!`
- [ ] Adicionar testes que rodam de diretórios não-raiz

---

## Fase 4 — Testes de Segurança

> Testes específicos que devem existir para validar as correções acima e prevenir regressões.

### 4.1 Testes de Sandbox
- [ ] `fetch("http://127.0.0.1:...")` → bloqueado
- [ ] `fetch("http://169.254.169.254/...")` → bloqueado
- [ ] `fetch("https://httpbin.org/get")` → permitido
- [ ] `Deno.readFile("...")` → não existe / permission denied
- [ ] `Deno.env.get("...")` → não existe / permission denied
- [ ] Prototype pollution via `Object.prototype.__proto__` → sem efeito

### 4.2 Testes de Resource Limits
- [x] Teste de término forçado de execução com `while(true){}` (via `terminate_execution`)
- [ ] Handler com `while(true){}` → timeout 504
- [ ] Handler que aloca 1GB → heap limit / OOM kill
- [x] Request body oversized → 413 Payload Too Large
- [ ] 20.000 conexões simultâneas → conexões excedentes dropadas

### 4.3 Testes de Auth
- [x] `POST /_internal/functions` sem API key → 401
- [x] `POST /_internal/functions` com key errada → 401
- [x] `POST /_internal/functions` com key correta → 200
- [x] `GET /{function}/` sem key → funciona (ingress público)

### 4.4 Testes de Resiliência
- [ ] Isolate panic → status muda para Error → auto-restart
- [ ] Shutdown com request in-flight → request completa ou recebe erro
- [ ] Deploy de bundle corrompido → erro 400, não crash

---

## Fase 5 — Compat Runtime (Vinext/Next.js, sem Cloud)

> Escopo desta fase: **somente runtime de execução**.
> Não inclui infraestrutura de cloud, storage distribuído, KV/Durable Objects, roteamento por manifest remoto ou deploy adapters.

### 5.1 Node Compatibility Mínima para Frameworks

**Objetivo:** habilitar superfície Node mínima exigida por toolchains e libs de SSR/RSC.

- [ ] Expor `globalThis.process` (subset seguro e estável)
- [ ] Expor `globalThis.Buffer` compatível (`node:buffer`)
- [ ] Expor `setImmediate`/`clearImmediate`
- [ ] Implementar suporte inicial aos módulos:
    - [ ] `node:buffer`
    - [ ] `node:events`
    - [ ] `node:util`
    - [ ] `node:path`
    - [ ] `node:stream`
    - [ ] `node:process`
- [ ] Implementar `node:os` compatível por contrato (pode ser stub estável)

**Critério de aceite:** app SSR simples com dependências Node utilitárias sobe sem erro de import em `node:*` básicos.

---

### 5.2 Interop de Módulos (ESM/CJS)

**Objetivo:** reduzir quebras por dependências CommonJS ainda presentes no ecossistema Next.

- [ ] Implementar `createRequire` básico para contexto ESM
- [ ] Implementar interop parcial `module.exports` <-> `default` export
- [ ] Suportar `require()` para built-ins permitidos
- [ ] Definir política explícita para módulos Node não suportados (erro determinístico e mensagem clara)
- [ ] Adicionar testes de resolução com pacotes híbridos ESM/CJS

**Critério de aceite:** libs comuns que ainda chamam `require()` indiretamente não falham na inicialização.

---

### 5.3 Semântica de Streams para SSR

**Objetivo:** compatibilizar pipeline de streaming usado por React SSR/Next.

- [ ] Implementar ponte robusta Web Streams <-> Node Streams (quando necessário)
- [ ] Garantir flush/backpressure corretos em resposta incremental
- [ ] Validar `ReadableStream` em respostas longas sem buffering total em memória
- [ ] Garantir comportamento consistente de cancelamento (`AbortSignal`) durante stream
- [ ] Adicionar teste E2E de SSR streaming com chunked body

**Critério de aceite:** SSR com streaming envia chunks progressivos, sem deadlock e sem corrupção de body.

---

### 5.4 Async Context por Request

**Objetivo:** suportar isolamento de contexto assíncrono por request (essencial em stacks Next modernas).

- [ ] Implementar camada compatível com `AsyncLocalStorage` (ou equivalente funcional)
- [ ] Garantir propagação de contexto por awaits/promises/timers
- [ ] Isolar contexto entre requests concorrentes
- [ ] Adicionar testes de concorrência validando não-vazamento de contexto

**Critério de aceite:** dois requests simultâneos não compartilham estado contextual.

---

### 5.5 HTTP/Web Semantics de Produção

**Objetivo:** corrigir nuances de protocolo que quebram app real mesmo com APIs disponíveis.

- [ ] Preservar múltiplos `Set-Cookie` sem flatten indevido
- [ ] Garantir merge de headers sem perda de semântica
- [ ] Validar clone/tee/locking de body em `Request`/`Response`
- [ ] Revisar comportamento de compressão/encoding em proxy e rewrite
- [ ] Adicionar suíte de regressão para casos reportados em ecossistemas SSR

**Critério de aceite:** testes de cookie/header/body passam em dev e prod profile.

---

### 5.6 WebSocket Runtime (Opcional para Vinext, recomendado)

**Objetivo:** habilitar cenários que dependem de upgrade e canais persistentes.

- [ ] Carregar extensão de WebSocket (`deno_websocket`) no runtime
- [ ] Expor `WebSocket` em `globalThis` no bootstrap
- [ ] Implementar testes de handshake + troca de mensagens
- [ ] Garantir limites de recurso e timeout para conexões WS

**Critério de aceite:** cliente `WebSocket` conecta e troca mensagens com estabilidade.

---

### 5.7 Matriz de Compatibilidade (Runtime-Only)

**Objetivo:** tornar explícito o nível de suporte para Vinext/Next sem cloud features.

- [ ] Publicar matriz por feature:
    - [ ] `Full` (funciona sem workaround)
    - [ ] `Partial` (funciona com limite documentado)
    - [ ] `None` (não suportado)
- [ ] Incluir foco em: Node built-ins, SSR streaming, RSC, server actions, headers/cookies
- [ ] Adicionar gate de CI para não regredir status `Full`

**Critério de aceite:** decisão de adoção possível sem leitura de código-fonte.

---

### 5.8 Priorização Recomendada (ordem de entrega)

1. Node globals + `node:buffer`/`node:process`/`node:util`/`node:path`
2. Interop CJS (`createRequire` + require parcial)
3. Streams SSR (bridge + cancelamento)
4. Async context por request
5. `node:os` compatível (stub estável)
6. Semântica HTTP fina (`Set-Cookie`, headers, body)
7. WebSocket

---

### 5.9 Modelo de Compatibilidade Inspirado em `nodejs_compat` (Cloudflare)

**Objetivo:** adotar modelo explícito de suporte por módulo/API para evitar ambiguidades no ecossistema npm.

- [ ] Definir 3 níveis oficiais por API Node:
    - [ ] `Full`: implementação funcional
    - [ ] `Partial`: implementação parcial com limitações documentadas
    - [ ] `Stub`: importável, mas métodos `noop` ou erro determinístico
- [ ] Padronizar erro de stub para métodos não implementados:
    - [ ] Formato recomendado: `[edge-runtime] <api> is not implemented in this runtime profile`
- [ ] Garantir que módulos `Stub` não quebrem no import (quebra apenas na chamada do método)
- [ ] Publicar tabela no docs com status por módulo `node:*`

**Critério de aceite:** qualquer pacote que apenas importa módulo Node não falha na carga por ausência de módulo.

---

### 5.10 Política de `fs` (Compat sem Acesso Real)

**Objetivo:** permitir compatibilidade de ecossistema sem prometer filesystem real.

- [ ] Implementar `node:fs` e `node:fs/promises` em modo `Stub/Partial` por perfil
- [ ] Definir comportamento por categoria:
    - [ ] Operações de leitura/escrita real -> erro determinístico (`EOPNOTSUPP`/mensagem clara)
    - [ ] APIs utilitárias sem side-effect (ex.: normalização de paths em chamadas internas) -> permitido quando seguro
    - [ ] APIs de watch/stream de arquivo -> `not implemented`
- [ ] Garantir que erro indique claramente: "sem acesso real ao FS neste runtime"
- [ ] Adicionar testes cobrindo:
    - [ ] `import "node:fs"` não falha
    - [ ] `readFile` falha com erro esperado
    - [ ] chamadas não suportadas retornam erro estável (sem panic)

**Critério de aceite:** bibliotecas que importam `fs` para feature detection não quebram bootstrap; uso real de disco falha de forma previsível.

---

### 5.11 Backlog de Módulos Node (Paridade por Etapas)

**Objetivo:** transformar compatibilidade em backlog executável por sprint.

- [ ] Etapa A (base de execução):
    - [ ] `node:buffer`
    - [ ] `node:process`
    - [ ] `node:events`
    - [ ] `node:util`
    - [ ] `node:path`
- [ ] Etapa B (SSR/RSC):
    - [ ] `node:stream`
    - [ ] `node:string_decoder`
    - [ ] `node:module` (parcial)
    - [ ] `node:os` (partial/stub)
- [ ] Etapa C (rede e protocolos):
    - [ ] `node:http` (parcial)
    - [ ] `node:https` (parcial)
    - [ ] `node:net` (parcial)
    - [ ] `node:tls` (stub/partial)
- [ ] Etapa D (baixo encaixe serverless):
    - [ ] `node:child_process` (stub)
    - [ ] `node:cluster` (stub)
    - [ ] `node:repl` (stub)
    - [ ] `node:dgram` (stub)

**Critério de aceite:** cada etapa possui suíte de regressão e status atualizado em matriz `Full/Partial/Stub/None`.

---

### 5.12 Flags de Compatibilidade de Runtime

**Objetivo:** permitir evolução incremental sem quebrar workloads existentes.

- [ ] Adicionar flag de runtime para compat Node (ex.: `--node-compat`)
- [ ] Adicionar variante mínima para contexto assíncrono (ex.: `--node-als`)
- [ ] Definir defaults por modo:
    - [ ] `start` produção: perfil conservador
    - [ ] `dev/test`: perfil ampliado para DX
- [ ] Documentar matriz de risco/segurança por flag

**Critério de aceite:** usuário consegue habilitar compat gradualmente sem alterar código da aplicação.

---

## Métricas de Sucesso

| Métrica | Alvo |
|---|---|
| Vulnerabilidades Críticas | 0 |
| Vulnerabilidades Altas | 0 |
| Cobertura de testes de segurança | > 90% dos cenários listados |
| Cold start (eszip) | < 200ms |
| Max concurrent connections | 10.000+ estável |
| Request timeout enforcement | 100% dos casos |
| Memory limit enforcement | 100% dos casos |
