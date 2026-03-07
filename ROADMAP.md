# Roadmap de Correções — Deno Edge Runtime

> Baseado na auditoria de segurança e arquitetura realizada em 05/03/2026.
> Cada item referencia o finding correspondente no `AUDIT.md`.
>
> Última atualização: 07/03/2026 (P1 de VFS seguro em `node:fs` concluído com quotas configuráveis por manifest/flag/env, `http/https` client-side compat, P2 de `node:dns` via DoH controlado e expansão de `node:util`/`node:diagnostics_channel`).
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

### 3.5 HTTP/3 (QUIC) — Futuro

**Prioridade:** Baixa (postergado)
**Status:** Futuro (fora do escopo imediato)

- [ ] Avaliar `quinn` ou `h3` crate
- [ ] Suportar QUIC listeners em paralelo com TCP
- [ ] ALPN negotiation para h2/h3

### 3.6 Module Integrity (Assinatura de Bundles)

- [x] Assinar bundles eszip com HMAC-SHA256 ou Ed25519
- [x] Verificar assinatura no load antes de execução
- [x] Rejeitar bundles sem assinatura válida em modo produção

**Status:** ✅ Concluído

Implementacao atual:
- Verificacao opcional por flag de assinatura Ed25519 nos endpoints de deploy/update (`POST/PUT /_internal/functions...`)
- Modo de enforcement via `--require-bundle-signature` + `--bundle-public-key-path`
- Rejeicao com `401` para assinatura ausente/invalida quando enforcement ativo
- Documentacao operacional detalhada em `docs/bundle-signing.md`

### 3.7 Resolver Paths Hardcoded no CLI

**Ref:** AUDIT §3.4

- [x] Usar variável `EDGE_RUNTIME_ROOT` ou auto-detectar via `Cargo.toml` parent walk (não necessário após adoção de assets embutidos)
- [x] Ou embutir assets no binário via `include_str!` / `include_bytes!`
- [x] Adicionar testes que rodam de diretórios não-raiz

**Status:** Concluido via assets TS nativos embutidos no binario (`include_str!`) para `edge://assert/*` e `ext:edge_assert/*`, removendo dependencia de paths da raiz do repositorio.

### 3.8 Observabilidade de Logs (Runtime + Isolate)

- [x] Adicionar formato JSON opcional de logs (`--log-format json`)
- [x] Manter formato default `pretty` (incluindo `watch`)
- [x] Enriquecer logs internos com `function_name` e `request_id` onde aplicável
- [x] Tornar saída de logs `console.*` de isolate configurável (`--print-isolate-logs`)
- [x] Expor coletor interno de logs de isolate para stack externa via OTLP (collector)
- [x] Exportar tracing HTTP request spans para OTLP
- [x] Exportar métricas de exportação de logs de isolate para OTLP

**Status:** Concluido

### 3.9 Proxy de Rede de Saida (Outgoing)

**Objetivo:** suportar proxy de saida para trafego HTTP, HTTPS e TCP com bypass configuravel por protocolo.

- [x] Adicionar suporte a proxy HTTP de saida
- [x] Adicionar suporte a proxy HTTPS de saida
- [x] Adicionar suporte a proxy TCP de saida
- [x] Adicionar configuracao `no-proxy` para HTTP
- [x] Adicionar configuracao `no-proxy` para HTTPS
- [x] Adicionar configuracao `no-proxy` para TCP
- [x] Expor configuracao via CLI e env vars dedicadas
- [x] Garantir compatibilidade com regras SSRF e allowlists existentes
- [x] Adicionar testes de integracao para:
    - [x] rota com proxy habilitado
    - [x] rota em `no-proxy` (bypass)
    - [x] fallback quando proxy indisponivel (erro claro)

**Critério de aceite:** requests e conexoes de saida usam proxy por protocolo quando configurado e respeitam `no-proxy` sem regressao de seguranca.

**Status:** ✅ Concluído

Implementacao atual:
- Configuracao de proxy em escopo global de runtime/processo (nao por isolate), via `PoolRuntimeConfig`.
- Flags e env vars dedicadas no CLI (`start` e `watch`) para HTTP/HTTPS/TCP e respectivos `no-proxy`.
- Aplicacao do proxy por variaveis de ambiente do runtime (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`) para compatibilidade nativa com stack de rede.
- Cobertura de integracao em `crates/functions/tests/outgoing_proxy.rs` com cenarios de proxy habilitado, bypass e proxy indisponivel.

---

## Fase 4 — Testes de Segurança

> Testes específicos que devem existir para validar as correções acima e prevenir regressões.

### 4.1 Testes de Sandbox
- [x] `fetch("http://127.0.0.1:...")` → bloqueado
- [x] `fetch("http://169.254.169.254/...")` → bloqueado
- [x] `fetch("https://httpbin.org/get")` → permitido
- [x] `Deno.readFile("...")` → não existe / permission denied
- [x] `Deno.env.get("...")` → não existe / permission denied
- [x] Prototype pollution via `Object.prototype.__proto__` → sem efeito

### 4.2 Testes de Resource Limits
- [x] Teste de término forçado de execução com `while(true){}` (via `terminate_execution`)
- [x] Handler com `while(true){}` → timeout 504
- [x] Handler que aloca 1GB → heap limit / OOM kill
- [x] Request body oversized → 413 Payload Too Large
- [x] 20.000 conexões simultâneas → conexões excedentes dropadas

### 4.3 Testes de Auth
- [x] `POST /_internal/functions` sem API key → 401
- [x] `POST /_internal/functions` com key errada → 401
- [x] `POST /_internal/functions` com key correta → 200
- [x] `GET /{function}/` sem key → funciona (ingress público)

### 4.4 Testes de Resiliência
- [x] Isolate panic → status muda para Error → auto-restart
- [x] Shutdown com request in-flight → request completa ou recebe erro
- [x] Deploy de bundle corrompido → erro 400, não crash

Notas de cobertura:
- Testes de sandbox adicionados em `crates/functions/tests/sandbox_security.rs`.
- Stress de `20.000` conexões foi adicionado como teste `#[ignore]` em `crates/server/src/lib.rs` (`stress_20k_connections_excess_are_dropped`) para evitar flakiness em ambientes com limite de recursos. O comportamento de drop também é validado por teste rápido não-ignorado (`e2e_connection_limit_drops_excess_connections`).
- Auth da fase 4.3 agora também possui cobertura E2E em `crates/server/src/lib.rs` (`e2e_admin_auth_and_public_ingress_behavior`) para `POST /_internal/functions` sem key, key inválida, key válida e `GET /{function}/` público sem key.
- Auto-restart após panic validado por teste ativo em `crates/functions/tests/timeout_and_timers.rs` (`test_panic_auto_restart_recovers_to_running`).

---

## Fase 5 — Compat Runtime (Vinext/Next.js, sem Cloud)

> Escopo desta fase: **somente runtime de execução**.
> Não inclui infraestrutura de cloud, storage distribuído, KV/Durable Objects, roteamento por manifest remoto ou deploy adapters.

### 5.1 Node Compatibility Mínima para Frameworks

**Objetivo:** habilitar superfície Node mínima exigida por toolchains e libs de SSR/RSC.

- [x] Expor `globalThis.process` (subset seguro e estável)
- [x] Expor `globalThis.Buffer` compatível (`node:buffer`)
- [x] Expor `setImmediate`/`clearImmediate`
- [ ] Implementar suporte inicial aos módulos:
    - [x] `node:buffer`
    - [x] `node:events`
    - [x] `node:util`
    - [x] `node:path`
    - [x] `node:stream`
    - [x] `node:process`
- [x] Implementar `node:os` compatível por contrato (pode ser stub estável)

**Critério de aceite:** app SSR simples com dependências Node utilitárias sobe sem erro de import em `node:*` básicos.

---

### 5.2 Interop de Módulos (ESM/CJS)

**Objetivo:** reduzir quebras por dependências CommonJS ainda presentes no ecossistema Next.

- [x] Implementar `createRequire` básico para contexto ESM
- [x] Implementar interop parcial `module.exports` <-> `default` export
- [x] Suportar `require()` para built-ins permitidos
- [x] Definir política explícita para módulos Node não suportados (erro determinístico e mensagem clara)
- [x] Adicionar testes de resolução com pacotes híbridos ESM/CJS

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
    - [ ] Formato recomendado: `[thunder] <api> is not implemented in this runtime profile`
- [ ] Garantir que módulos `Stub` não quebrem no import (quebra apenas na chamada do método)
- [ ] Publicar tabela no docs com status por módulo `node:*`

**Critério de aceite:** qualquer pacote que apenas importa módulo Node não falha na carga por ausência de módulo.

---

### 5.10 Política de `fs` (Compat sem Acesso Real)

**Objetivo:** permitir compatibilidade de ecossistema sem prometer filesystem real.

- [x] Implementar `node:fs` e `node:fs/promises` em modo `Stub/Partial` por perfil
- [x] Definir comportamento por categoria:
    - [x] Operações de leitura/escrita real -> erro determinístico (`EOPNOTSUPP`/mensagem clara)
    - [x] APIs utilitárias sem side-effect (ex.: normalização de paths em chamadas internas) -> permitido quando seguro
    - [x] APIs de watch/stream de arquivo -> `not implemented`
- [x] Garantir que erro indique claramente: "sem acesso real ao FS neste runtime"
- [x] Adicionar testes cobrindo:
    - [x] `import "node:fs"` não falha
    - [x] `readFile` falha com erro esperado
    - [x] chamadas não suportadas retornam erro estável (sem panic)

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

### 5.13 Gap Analysis — Cloudflare Workers Node APIs (baseline oficial)

**Objetivo:** alinhar o runtime com o comportamento documentado da Cloudflare para Node APIs, preservando as garantias de sandbox (sem acesso ao host físico).

**Fontes de referência (baseline):**
- [Node.js compatibility index](https://developers.cloudflare.com/workers/runtime-apis/nodejs/)
- [process](https://developers.cloudflare.com/workers/runtime-apis/nodejs/process/)
- Diretório de docs: `src/content/docs/workers/runtime-apis/nodejs/*` em `cloudflare/cloudflare-docs` (branch `production`)

**Catálogo de diferenças atuais (runtime vs Cloudflare):**

1. `node:process`
- **Cloudflare:** `process.env` pode ser populado por bindings/flags, `stdout/stderr/stdin` como streams, `cwd` inicial `/bundle`, `chdir` suportado com FS virtual.
- **Runtime atual:** `env` apenas em memória local (não populado por bindings), `stdout/stderr/stdin` ausentes, `cwd` fixo `/`, `chdir` bloqueado por sandbox.
- **Status:** divergência funcional relevante.

2. `node:http` e `node:https`
- **Cloudflare:** `request/get` funcionais como wrapper de `fetch` (com restrições); suporte adicional a server-side APIs via `cloudflare:node` + flags.
- **Runtime atual:** `request/get` bloqueados por política (`ERR_USE_FETCH`), `createServer` não implementado.
- **Status:** divergência funcional intencional (segurança), precisa de modo de compat opcional para paridade.

3. `node:fs` e `node:fs/promises`
- **Cloudflare:** VFS com `/bundle` (read-only), `/tmp` (ephemeral por request), `/dev/*`; ampla API com limitações documentadas.
- **Runtime atual:** stub seguro (`EOPNOTSUPP`) sem VFS.
- **Status:** grande gap de paridade.

4. `node:dns`
- **Cloudflare:** maioria da API disponível via DoH/1.1.1.1; apenas alguns métodos não implementados (`lookup`, `lookupService`, `resolve`).
- **Runtime atual:** módulo majoritariamente stub/non-functional.
- **Status:** gap alto.

5. `node:net`
- **Cloudflare:** `net.Socket`/`connect` suportados para outbound TCP; `net.Server` não suportado.
- **Runtime atual:** `connect` não implementado; existe `createServer` stub.
- **Status:** gap alto e desalinhamento de superfície.

6. `node:tls`
- **Cloudflare:** `connect`, `TLSSocket`, `checkServerIdentity`, `createSecureContext` disponíveis; server-side TLS Node não suportado.
- **Runtime atual:** `connect/createSecureContext` stubs não funcionais.
- **Status:** gap alto.

7. `node:url`
- **Cloudflare:** `domainToASCII`/`domainToUnicode` e demais APIs de URL documentadas.
- **Runtime atual:** check de compat está `None` no relatório para `node:url`.
- **Status:** gap funcional imediato (alta prioridade).

8. `node:util`
- **Cloudflare:** `promisify/callbackify`, `util.types` (com subset explícito), `MIMEType`.
- **Runtime atual:** subset básico (`format`, `inspect`, `promisify`, `types`), sem confirmação de `MIMEType`.
- **Status:** gap médio.

9. `node:diagnostics_channel`
- **Cloudflare:** inclui `TracingChannel` e integração com Tail Workers.
- **Runtime atual:** pub/sub básico.
- **Status:** gap médio.

10. `node:async_hooks` / `AsyncLocalStorage`
- **Cloudflare:** ALS funcional com caveats documentados, `AsyncResource` parcial.
- **Runtime atual:** classificação stub/parcial.
- **Status:** gap alto para frameworks modernos.

11. `node:zlib`
- **Cloudflare:** módulo funcional (gzip/deflate/brotli).
- **Runtime atual:** stub/non-functional.
- **Status:** gap médio/alto.

12. `node:events` e `node:buffer`
- **Cloudflare:** suporte amplo (com diferenças específicas documentadas).
- **Runtime atual:** funcionais para casos comuns, mas com cobertura parcial no relatório.
- **Status:** reduzir gap via testes de semântica avançada e edge-cases.

**Backlog de convergência (prioridade):**

- [x] **P0:** fechar `node:url` para sair de `None` no relatório (incluindo `domainToASCII`/`domainToUnicode`).
- [x] **P0:** adicionar `process.stdout/stderr/stdin` compatíveis e `cwd` virtual (`/bundle`), sem acesso ao host.
- [x] **P1:** implementar VFS seguro (`/bundle`, `/tmp`, `/dev`) para `node:fs` sem quebrar isolamento.
    - Status aplicado: `/bundle` read-only (`EROFS`), `/tmp` writable efêmero em memória, `/dev/null` como sink virtual.
    - Status aplicado: quotas VFS com defaults de 10 MiB total e 5 MiB por arquivo.
    - Status aplicado: quotas ajustáveis por função via manifest (`resources.vfsTotalQuotaBytes`, `resources.vfsMaxFileBytes`) e globalmente via CLI/env (`--vfs-total-quota-bytes`, `--vfs-max-file-bytes`).
- [x] **P1:** modo `http/https` compat opcional (wrapper `fetch` dentro de handler) mantendo default seguro atual.
    - Status aplicado: client-side `request/get` em `node:http` e `node:https` já operam via wrapper `fetch`; APIs server-side permanecem não funcionais por sandbox.
    - Status aplicado: adapter `request` com contrato básico (`get/post/put/patch/del/delete`, callback `(err,res,body)`, `write/end`) sobre o wrapper HTTP compat.
- [x] **P1:** `node:net` outbound-only (sem `net.Server`) e `node:tls` outbound compatível.
    - Status aplicado: `net.connect`/`createConnection` e `tls.connect` expostos para subset cliente outbound.
    - Status aplicado: superfícies server/context não implementadas continuam em stub determinístico (`net.Server.listen`, `tls.createServer`, `tls.createSecureContext`).
- [x] **P2:** `dns` funcional via resolver controlado (DoH/subrequest), com limites explícitos.
    - Status aplicado: subset funcional em `node:dns` para `lookup`, `resolve*`, `reverse` e equivalentes em `dns.promises`.
    - Status aplicado: respostas limitadas por consulta (`dns_max_answers`) e endpoint/timeout configuráveis globalmente (`--dns-doh-endpoint`, `--dns-max-answers`, `--dns-timeout-ms` + envs equivalentes).
    - Status aplicado: APIs fora do subset permanecem em stub determinístico (`ERR_NOT_IMPLEMENTED`).
- [x] **P2:** expandir `util` (`MIMEType`) e `diagnostics_channel` (`TracingChannel`) conforme documentação.
    - Status aplicado: `node:util` agora expõe `MIMEType` e `MIMEParams` com parsing básico, `params` mutáveis e serialização determinística.
    - Status aplicado: `node:diagnostics_channel` inclui `TracingChannel`/`tracingChannel` com hooks `start/end/asyncStart/asyncEnd/error` e helpers `traceSync`/`tracePromise`/`traceCallback`.
- [ ] **P2:** elevar `async_hooks`/ALS de stub para uso real com testes de propagação de contexto.
- [ ] **P3:** substituir `zlib` stub por implementação funcional (ou bridge para APIs nativas de compressão).

**Critério de aceite desta trilha:**
- Matriz `node:*` no relatório com classificação convergente ao baseline Cloudflare.
- Diferenças remanescentes explicitamente documentadas como "intencionais por sandbox".
- Nenhuma feature de compatibilidade libera acesso ao host físico.

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
