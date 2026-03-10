# Security & Architecture Audit — Deno Edge Runtime

> **Initial audit:** 05/03/2026
> **Re-audit:** 06/03/2026
> **Scope:** Complete analysis of the 4 crates (`runtime-core`, `functions`, `server`, `cli`), JS tests, scripts, schemas, and configuration.
> **Objective:** Identify security vulnerabilities, breaking points, design flaws, and test gaps before production use.
> **Method:** Full source code review of every file in each crate, cross-referenced against ROADMAP.md.

---

## Summary

| Severity | Original (05/03) | Current (06/03) | Notes |
|---|---|---|---|
| **Critical** | 4 | 0 fixed, 2 new | 4 originais corrigidos; 2 novos descobertos nesta re-auditoria |
| **High** | 6 | 0 fixed, 3 new | 6 originais corrigidos; 3 novos descobertos |
| **Medium** | 8 | 0 fixed, 7 new | 8 originais corrigidos; 7 novos descobertos |
| **Low** | 5 | 3 fixed, 8 new | 3 originais corrigidos; 2 permanecem; 8 novos descobertos |

---

## Correcoes Implementadas (desde audit original)

As seguintes vulnerabilidades do audit original foram **confirmadas como corrigidas** via revisao de codigo:

### Critical (4/4 corrigidos)

| # | Finding original | Status | Evidencia |
|---|---|---|---|
| 1.1 | TLS configurado mas nunca usado | **CORRIGIDO** | `crates/server/src/lib.rs`: `DynamicTlsAcceptor` com handshake TLS real em todos os accept loops (admin, ingress TCP). `MaybeHttpsStream` enum abstrai TcpPlain/TcpTls/Unix. Hot-reload via `notify` watcher com retry e fingerprint logging. |
| 1.2 | Endpoints `/_internal` sem autenticacao | **CORRIGIDO** | Arquitetura dual-listener: `AdminRouter` (`admin_router.rs`) com `check_auth()` via header `X-API-Key`; `IngressRouter` (`ingress_router.rs`) rejeita `/_internal/*` com 404. |
| 1.3 | SSRF via `fetch()` sem restricao de IP privado | **CORRIGIDO** (com ressalva IPv6) | `crates/runtime-core/src/ssrf.rs`: `DEFAULT_DENY_RANGES` bloqueia `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `0.0.0.0/8`, `[::1]`. IPv6 CIDR pendente (ver NEW-C1). |
| 1.4 | Sem limite de tamanho no body | **CORRIGIDO** | `crates/server/src/body_limits.rs`: dual-layer com `check_content_length()` + `http_body_util::Limited`. Defaults: 5 MiB request, 10 MiB response. |

### High (6/6 corrigidos)

| # | Finding original | Status | Evidencia |
|---|---|---|---|
| 2.1 | Sem limite de conexoes simultaneas | **CORRIGIDO** | `tokio::sync::Semaphore` com `try_acquire_owned()` em todos os accept loops. Default 10.000. Flag `--max-connections`. |
| 2.2 | CPU timer usa wall-clock | **CORRIGIDO** | `crates/runtime-core/src/cpu_timer.rs`: `CLOCK_THREAD_CPUTIME_ID` via `libc` (Unix), wall-clock como fallback. |
| 2.3 | Heap limit sem enforcement real | **CORRIGIDO** | `near_heap_limit_callback` registrado em `lifecycle.rs`. 1a chamada: extensao de 10%. 2a chamada: `terminate_execution()` + `should_terminate` flag. |
| 2.4 | Panic no isolate nao atualiza status | **CORRIGIDO** | `catch_unwind` + `mark_dead()` + `close_request_tx()` + `fail_pending_requests()` + auto-restart com backoff exponencial (1s, 2s, 4s, 8s, 16s, max 5 restarts). |
| 2.5 | Sem request timeout no dispatch | **CORRIGIDO** | Dual-layer timeout: watchdog thread com `terminate_execution()` + `tokio::time::timeout()`. Retorna 504 Gateway Timeout. Cleanup de timers/intervals/promises por `execution_id`. |
| 2.6 | Flag `exceeded` do CPU timer nunca resetada | **CORRIGIDO** | `CpuTimer::reset()` chamado antes de cada request em `lifecycle.rs`. Zera `accumulated_ms` e `exceeded` flag. |

### Medium (8/8 corrigidos)

| # | Finding original | Status | Evidencia |
|---|---|---|---|
| 3.1 | Rate limiter definido mas nunca aplicado | **CORRIGIDO** | `RateLimitLayer` aplicado no `IngressRouter` antes do processamento. Fixed-window 1s. Retorna 429 com `Retry-After`. |
| 3.2 | Metrics endpoint sem cache | **CORRIGIDO** | `MetricsCache` com `RwLock` e TTL de 15s em `router.rs`. |
| 3.3 | V8 Inspector sem protecao de rede | **CORRIGIDO** | `inspect_allow_remote` default `false` (bind `127.0.0.1`). Flag `--inspect-allow-remote` para override. Warnings quando inspector ativado. |
| 3.4 | Paths hardcoded no CLI | **CORRIGIDO** | `include_str!()` para modulos assert embutidos no binario. `embedded_assert.rs` mapeia `edge://assert/*` e `ext:edge_assert/*`. |
| 3.5 | Nome de funcao sem validacao | **CORRIGIDO** | `is_valid_function_name()` com regex `^[a-z0-9][a-z0-9-]{0,62}$`. Validacao em deploy e ingress. Schema `common.schema.json` com mesmo pattern. |
| 3.6 | Stub ops silenciosos | **PERMANECE** | `op_set_raw`, `op_console_size`, `op_tls_peer_certificate` ainda retornam no-op silencioso. Reclassificado como LOW. |
| 3.7 | Source maps habilitados por default | **CORRIGIDO** | `IsolateConfig.enable_source_maps` default alterado; CLI flag `--sourcemap` com opcoes `none` (default) e `inline`. |
| 3.8 | Error messages vazam informacao interna | **CORRIGIDO** | `sanitize_internal_error()` em `router.rs`: log completo server-side, retorna `{"error":"internal_error","request_id":"..."}` ao cliente. |

### Low (3/5 corrigidos)

| # | Finding original | Status | Evidencia |
|---|---|---|---|
| 4.1 | `Ordering::Relaxed` em metrics | **PERMANECE** | Atomics de metricas ainda usam `Relaxed`. Risco real baixo em pratica. |
| 4.2 | Globals mutaveis no bootstrap | **CORRIGIDO** | 10 globals criticos frozen via `Object.freeze()`: `fetch`, `Request`, `Response`, `Headers`, `crypto`, `URL`, `URLSearchParams`, `TextEncoder`, `TextDecoder`, `console`. |
| 4.3 | Excecao silenciosa no edge_assert import | **CORRIGIDO** | Modulos assert agora embutidos via `include_str!()` e registrados como extensao nativa. |
| 4.4 | HTTP parser custom no test runner | **PERMANECE** | `test.rs` linhas 636-810 ainda usa parser HTTP manual com peek de 2048 bytes. Risco mitigado: inspector apenas dev, bind localhost. |
| 4.5 | Graceful shutdown com sleep fixo | **CORRIGIDO** | `shutdown_all_with_deadline()` no dual-server: cancela token, poll ate channels fecharem ou deadline, force-clear. |

---

## Novas Vulnerabilidades Descobertas (Re-Audit 06/03/2026)

### Critical

#### NEW-C1: IPv6 SSRF Bypass

**File:** `crates/runtime-core/src/ssrf.rs` (linhas 25-27)
**Severity:** CRITICO

Apenas `[::1]` e bloqueado para IPv6. Ranges CIDR IPv6 nao sao suportados pelo parser `deno_permissions` nesta versao. Um atacante pode contornar a protecao SSRF via:

```javascript
// IPv4-mapped IPv6 (bypass total)
fetch("http://[::ffff:169.254.169.254]/latest/meta-data/")

// Unique Local Address (equivalente RFC 1918)
fetch("http://[fd00::1]:8080/admin")

// Link-local
fetch("http://[fe80::1%25eth0]/")
```

**Ranges faltantes:**
- `fc00::/7` (Unique Local Addresses)
- `fe80::/10` (Link-Local)
- `::ffff:0:0/96` (IPv4-mapped IPv6 -- vetor de bypass critico)
- `100::/64` (Discard prefix)
- `2001:db8::/32` (Documentation prefix)

**Impacto:** Bypass completo da protecao SSRF via enderecos IPv6. Exfiltracao de credenciais cloud via `::ffff:169.254.169.254`.

**Fix:** Quando o parser `deno_permissions` suportar CIDR IPv6, adicionar os ranges acima ao `DEFAULT_DENY_RANGES`. Como workaround imediato, adicionar IPs IPv6 individuais mais criticos (e.g., `[::ffff:169.254.169.254]`, `[::ffff:127.0.0.1]`, etc.).

**Status de cobertura (10/03/2026):**
- Teste ofensivo de deteccao adicionado em `crates/functions/tests/sandbox_security.rs`:
	- `sandbox_detects_ipv6_ssrf_bypass_vectors`
- O teste valida baseline de bloqueio IPv4 privado e detecta pelo menos um vetor IPv6 nao-denied (bypass), mantendo o finding visivel em CI.

**Gate de regressao pos-fix (alinhado ao ROADMAP):**
- Quando a correcao de IPv6 SSRF entrar, inverter o teste para exigir `denied` em todos os vetores IPv6 (`::ffff:169.254.169.254`, `fd00::/7`, `fe80::/10`).

---

#### NEW-C2: Legacy Router sem Autenticacao nem Verificacao de Assinatura

**File:** `crates/server/src/router.rs` (linha 429)
**Severity:** CRITICO

O router legado (`Router` + `run_server()`) que combina admin e ingress num unico listener **nao possui**:
- Autenticacao em `/_internal/*` (nenhum `check_auth()`)
- Verificacao de assinatura de bundles no deploy
- Endpoints de pool
- Rejeicao de `/_internal` no ingress

O `watch` command usa `run_server()` (legado), o que e aceitavel para dev. Porem, se qualquer caminho de producao invocar `run_server()` ao inves de `run_dual_server()`, toda a seguranca do admin e contornada.

**Impacto:** Se o router legado for usado em producao, endpoints admin ficam abertos sem autenticacao.

**Fix:** Deprecar `run_server()` para uso em producao. Adicionar warning ou panic se chamado sem flag de override explicito. Idealmente, remover ou feature-gate o `Router` legado.

---

### High

#### NEW-H1: Sem Timeout no TLS Handshake

**File:** `crates/server/src/lib.rs` (accept loops)
**Severity:** ALTO

O `tls_acceptor.accept(stream).await` nao possui timeout. Um cliente malicioso pode abrir uma conexao TCP, consumir um permit do semaforo, e nunca completar o handshake TLS (slowloris-on-TLS). Isso esgota o limite de conexoes sem enviar nenhum byte util.

**Impacto:** DoS via exaustao de permits do semaforo com conexoes TLS penduradas.

**Fix:** Envolver `accept()` com `tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await`.

---

#### NEW-H2: Response Body de Streaming sem Limite de Tamanho

**File:** `crates/server/src/ingress_router.rs` (linhas 211-235)
**Severity:** ALTO

`check_response_body_size()` e chamado apenas para `IsolateResponseBody::Full`. Respostas de streaming (`IsolateResponseBody::Stream`) nao possuem nenhum controle de tamanho. Uma funcao maliciosa pode enviar dados ilimitados via `ReadableStream`.

**Impacto:** Funcao maliciosa pode usar streaming para enviar gigabytes de dados sem limitacao.

**Fix:** Implementar `ByteCountingStream` wrapper que conta bytes e aborta apos `MAX_RESPONSE_BODY_BYTES`.

**Status de cobertura (10/03/2026):**
- Teste E2E de deteccao do gap adicionado em `crates/server/src/lib.rs`:
	- `e2e_ingress_streaming_response_exceeds_limit_without_rejection`
- O teste configura `max_response_body_bytes` baixo e confirma que a resposta streaming ainda retorna `200` e ultrapassa o limite (comportamento atual vulneravel).

**Gate de regressao pos-fix (alinhado ao ROADMAP):**
- Depois do enforcement em `IsolateResponseBody::Stream`, inverter o E2E para esperar rejeicao deterministica (status/payload definidos) ao ultrapassar o limite.

---

#### NEW-H3: Comparacao de API Key Nao e Constant-Time

**File:** `crates/server/src/admin_router.rs` (linha 166)
**Severity:** ALTO

```rust
key == expected  // String equality padrao, nao constant-time
```

A comparacao de igualdade entre a chave fornecida e a esperada nao usa comparacao constant-time. Embora explorar timing side-channels sobre HTTP seja dificil na pratica, e uma falha criptografica conhecida.

**Fix:** Usar `subtle::ConstantTimeEq` ou `ring::constant_time::verify_slices_are_equal()`.

---

### Medium

#### NEW-M1: Sem Backoff no Accept Loop em Erro

**File:** `crates/server/src/lib.rs` (todos os accept loops)
**Severity:** MEDIO

Se `listener.accept()` falhar repetidamente (e.g., `EMFILE` -- limite de file descriptors), o loop gira em CPU max sem backoff. Pode causar 100% CPU sob exaustao de FDs.

**Fix:** Adicionar `tokio::time::sleep(Duration::from_millis(50))` apos erro consecutivo de accept.

---

#### NEW-M2: Rate Limiter Global (nao Per-IP/Per-Function)

**File:** `crates/server/src/middleware/mod.rs`
**Severity:** MEDIO

O rate limiter usa fixed-window de 1 segundo e e global. Um unico cliente abusivo esgota o limite para todos. Alem disso, o algoritmo fixed-window permite ate 2x o limite na fronteira de janelas.

**Mitigacao futura:** Considerar rate limiting per-IP (e.g., `governor` crate) ou per-function.

---

#### NEW-M3: Endpoints Admin sem Rate Limiting

**File:** `crates/server/src/admin_router.rs`
**Severity:** MEDIO

Nenhum rate limiting e aplicado aos endpoints admin. Um atacante com API key valida (ou sem key em modo dev) pode fazer requests ilimitados a `/_internal/metrics`, causando CPU burn via `sysinfo::System::new_all()`.

---

#### NEW-M4: Erro no Pool Leaks Detalhes Internos

**File:** `crates/server/src/admin_router.rs` (linha 526)
**Severity:** MEDIO

```rust
format!(r#"{{"error":"{}"}}"#, e)  // 'e' pode conter detalhes internos
```

O endpoint `set_pool_limits` formata o erro diretamente no JSON retornado ao cliente, ao inves de usar `sanitize_internal_error()`.

**Fix:** Usar `sanitize_internal_error()` consistentemente em todos os endpoints.

---

#### NEW-M5: Legacy run_server Graceful Shutdown Usa Sleep ao Inves de Drain

**File:** `crates/server/src/lib.rs` (linhas 601-604)
**Severity:** MEDIO

O `run_server()` legado dorme pelo deadline em vez de aguardar conexoes drenarem. Conexoes podem ser cortadas mid-flight ou o servidor esperar mais que o necessario.

---

#### NEW-M6: Default Permissions sem Protecao SSRF

**File:** `crates/runtime-core/src/permissions.rs`
**Severity:** MEDIO

`create_permissions_container()` (funcao default) configura `deny_net: None` -- zero protecao de rede. Qualquer caminho de codigo que use esta funcao ao inves de `create_permissions_with_policy()` fica desprotegido. A funcao existe para conveniencia mas e perigosa se usada incorretamente.

**Fix:** Considerar deprecar `create_permissions_container()` ou fazer SSRF protection o default.

---

#### NEW-M7: CPU Time Monitorado mas Nao Enforced

**File:** `crates/functions/src/lifecycle.rs`
**Severity:** MEDIO

O `CpuTimer` rastreia tempo de CPU real (`CLOCK_THREAD_CPUTIME_ID`) e seta um flag `exceeded`, mas **nenhum codigo verifica este flag** no loop de requests. O `cpu_time_limit_ms` (default 50s) e puramente informativo -- nao causa terminacao. Apenas o wall-clock timeout fornece enforcement real.

**Impacto:** Uma funcao que consome CPU intensivamente dentro do wall-clock timeout nao sera terminada por limites de CPU. O campo `cpu_time_limit_ms` e enganoso.

**Fix:** Verificar `cpu_timer.is_exceeded()` apos `stop()` e retornar erro / terminar isolate. Ou documentar que CPU time e apenas metricas, nao enforcement.

---

### Low

#### NEW-L1: Watchdog Thread por Request

**File:** `crates/functions/src/lifecycle.rs` (linha 475)
**Severity:** BAIXO

Cada request com timeout spawna uma nova `std::thread` dedicada para o watchdog. Sob alta concorrencia, isso cria churn de threads significativo. Funcional mas nao otimo.

**Mitigacao futura:** Timer wheel compartilhado ou tokio timer.

---

#### NEW-L2: Stream Sender Leak em Timeout

**File:** `crates/functions/src/handler.rs`
**Severity:** BAIXO

Quando um request de streaming sofre timeout, `unregister_response_stream` nunca e chamado para o stream ID que ja foi registrado. A entry do sender fica no `ResponseStreamRegistry` HashMap ate o receiver ser dropado e o sender detectar channel fechado. Leak de memoria menor por request de streaming com timeout.

---

#### NEW-L3: `eval()` e `Function()` Nao Removidos do Sandbox

**File:** `crates/runtime-core/src/bootstrap.js` (linhas 308-311)
**Severity:** BAIXO

```javascript
// Prevent dynamic code execution
// Note: These should be done carefully as some libraries need them
// delete globalThis.eval;
// delete globalThis.Function;
```

`eval()` e o constructor `Function()` permanecem disponiveis. Decisao consciente para compatibilidade de bibliotecas, mas permite execucao dinamica de codigo no sandbox. O setTimeout wrapper tambem usa `eval(fn)` para argumentos string (handler.rs linha 276).

---

#### NEW-L4: Maioria dos Globals Nao Frozen

**File:** `crates/runtime-core/src/bootstrap.js`
**Severity:** BAIXO

Apenas 10 globals criticos sao frozen. Dezenas de outros globals podem ser monkey-patched por user code:
- `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`
- `AbortController`, `AbortSignal`
- `ReadableStream`, `WritableStream`, `TransformStream`
- `Blob`, `File`, `FileReader`, `FormData`
- `Event`, `EventTarget`, `CustomEvent`, `MessageEvent`
- `CompressionStream`, `DecompressionStream`
- `Performance`, `PerformanceEntry`
- `MessageChannel`, `MessagePort`
- `Crypto`, `CryptoKey`, `SubtleCrypto` (apenas a instancia `crypto` e frozen, nao os constructors)
- `atob`, `btoa`, `structuredClone`, `reportError`

---

#### NEW-L5: MetricsCache Thundering Herd

**File:** `crates/server/src/router.rs` (linhas 65-86)
**Severity:** BAIXO

Quando o cache de metrics expira, multiplos readers concorrentes podem todos falhar o check de TTL, liberar o read lock, e todos tentar adquirir o write lock. Apenas um computa, os outros bloqueiam.

---

#### NEW-L6: Sem mTLS (Client Certificate Verification)

**File:** `crates/server/src/tls.rs` (linha 61)
**Severity:** BAIXO

`with_no_client_auth()` significa zero verificacao de certificado do cliente. Tipico para edge servers, mas impede autenticacao mTLS de servicos internos.

---

#### NEW-L7: `Deno.permissions.query` Nao Deletado

**File:** `crates/runtime-core/src/bootstrap.js`
**Severity:** BAIXO

`Deno.permissions.request` e `Deno.permissions.revoke` sao deletados, mas `Deno.permissions.query` permanece. User code pode inspecionar quais permissoes estao configuradas, vazando informacao sobre a configuracao do runtime.

---

#### NEW-L8: HeapLimitState Pointer Leak em Panic

**File:** `crates/functions/src/lifecycle.rs`
**Severity:** BAIXO

`HeapLimitState` e alocado via `Box::into_raw` e liberado na saida de `run_isolate`. Se `run_isolate` sofre panic antes do cleanup (linhas 626-629), o pointer leaka. O `catch_unwind` no supervisor captura o panic e o V8 isolate e dropado, entao o callback nao pode ser invocado -- leak de memoria menor, sem risco de use-after-free.

---

#### NEW-L9: FileLoader Duplicado 4 Vezes

**File:** `crates/cli/src/commands/{test,bundle,check,watch}.rs`
**Severity:** BAIXO

O struct `FileLoader` e sua implementacao de `Loader` sao copy-paste identicos em 4 arquivos. Aumenta risco de patches inconsistentes.

---

#### NEW-L10: Sem Versao TLS Minima Explicita

**File:** `crates/server/src/tls.rs` (linha 60)
**Severity:** BAIXO

O `ServerConfig` nao define versao TLS minima explicitamente. Depende dos defaults do `rustls` (TLS 1.2+), que sao seguros, mas configuracao explicita seria preferivel para hardening.

---

## 5. Test Coverage Gaps

| Categoria | Status Original | Status Atual | Evidencia |
|---|---|---|---|
| Permission enforcement (network, fs denied) | Missing | **COBERTO** | `sandbox_security.rs`: 4 testes (SSRF 127/169, readFile, env.get, prototype pollution) |
| Memory limit (OOM) | Missing | **COBERTO** | `timeout_and_timers.rs`: `test_heap_limit_infinite_allocation_marks_function_error` |
| CPU timeout (infinite loop) | Missing | **COBERTO** | `timeout_and_timers.rs`: `test_terminate_execution_stops_infinite_loop`, `test_isolate_timeout_returns_504` |
| Isolate panic recovery | Missing | **PARCIAL** | `test_panic_followed_by_request_marks_error_and_fails_fast` ok; `test_panic_auto_restart_recovers_to_running` ainda `#[ignore]` |
| Concurrent requests to same isolate | Missing | **COBERTO** | `timeout_and_timers.rs`: `test_concurrent_requests_to_same_isolate` |
| Graceful shutdown with in-flight requests | Missing | **FALTA** | Nenhum teste de integracao encontrado |
| SSRF (fetch to private IPs) | Missing | **COBERTO** | `sandbox_security.rs`: `sandbox_blocks_private_fetch_targets` |
| Maximum body size | Missing | **FALTA** | Feature existe (`body_limits.rs`), mas sem teste enviando payload oversized e verificando 413 |
| Prototype pollution / sandbox escape | Missing | **COBERTO** | `sandbox_security.rs`: `sandbox_blocks_prototype_pollution_via_object_prototype_proto` |
| Negative tests for Web APIs | Nearly zero | **FALTA** | Testes de Web API permanecem verificacoes de existencia/constructor |
| Internal endpoint authentication | Missing | **PARCIAL** | Auth implementada no `AdminRouter`; testes E2E existem em `crates/server/src/lib.rs` mas sem teste funcional externo completo |
| End-to-end TLS handshake | Missing | **PARCIAL** | Teste E2E existe em `crates/server/src/lib.rs` (`e2e_tls_accepts_https_connection`); sem teste funcional externo via CLI |
| Web APIs existence/constructors | OK (~70 APIs) | **OK** | 70+ testes em `web_api_compat.rs` + 60+ em `web_api_report.rs` |
| Isolate boot and basic dispatch | OK | **OK** | `isolate_boot.rs`: 3 testes |
| Load testing (k6) | OK | **OK** | `scripts/load-test.js` |
| Connection limit enforcement | N/A (novo) | **PARCIAL** | `e2e_connection_limit_drops_excess_connections` em `crates/server/src/lib.rs`; stress 20k `#[ignore]` |
| Rate limiter activation | N/A (novo) | **FALTA** | Nenhum teste de integracao para 429 Too Many Requests |
| Bundle signature verification | N/A (novo) | **FALTA** | Feature implementada, mas sem teste E2E de assinatura invalida |
| SSRF ranges IPv6 | N/A (novo) | **PARCIAL** | Gap coberto por teste ofensivo de deteccao (`sandbox_detects_ipv6_ssrf_bypass_vectors`); bloqueio completo IPv6 ainda pendente |
| Additional SSRF ranges (10.x, 172.16.x, etc.) | N/A | **FALTA** | Testes cobrem apenas 127.0.0.1 e 169.254.169.254 |
| Streaming response timeout | N/A (novo) | **FALTA** | Ainda sem teste dedicado de timeout; existe cobertura do gap de limite em stream via `e2e_ingress_streaming_response_exceeds_limit_without_rejection` |

---

## 6. Inventario de Arquivos por Crate

### server (12 arquivos)

| Arquivo | Linhas | Funcao |
|---|---|---|
| `src/lib.rs` | ~1333 | Config, `run_dual_server()`, `run_server()`, accept loops, semaforo, shutdown, testes E2E |
| `src/admin_router.rs` | 614 | Router admin: auth API key, deploy/update/delete/reload/pool |
| `src/ingress_router.rs` | 346 | Router ingress: rate limiter, body limits, timeout, streaming, rejeita `/_internal` |
| `src/router.rs` | 996 | Router legado (combinado), utilities compartilhadas, MetricsCache, validacao de nomes |
| `src/service.rs` | 76 | Adapter `EdgeService<R>` para `hyper::service::Service` |
| `src/tls.rs` | 336 | `DynamicTlsAcceptor`, hot-reload via `notify`, `MaybeHttpsStream`, fingerprint |
| `src/body_limits.rs` | 195 | `check_content_length()`, `collect_body_with_limit()`, payload_too_large |
| `src/graceful.rs` | 34 | `wait_for_shutdown_signal()`: Ctrl+C/SIGTERM |
| `src/middleware/mod.rs` | 94 | `RateLimitLayer`: fixed-window 1s |
| `src/trace_context.rs` | 188 | W3C Trace Context, correlation-id, sampling configuravel |
| `src/bundle_signature.rs` | 288 | Ed25519 bundle signature verification (PEM/base64/hex) |

### functions (6 arquivos + 13 testes)

| Arquivo | Funcao |
|---|---|
| `src/handler.rs` | Request dispatch bridge: inject JS bridge, V8 API dispatch, streaming ops |
| `src/lifecycle.rs` | Create/boot/run_isolate, panic recovery, timeout, heap limit, inspector |
| `src/registry.rs` | FunctionRegistry (DashMap), pool, CRUD, LRU eviction, shutdown |
| `src/types.rs` | BundlePackage, FunctionEntry, FunctionMetrics, FunctionStatus, PoolLimits |
| `src/metrics.rs` | GlobalMetrics: atomic counters |

### runtime-core (10 arquivos Rust + 6 arquivos JS/TS + assert library)

| Arquivo | Funcao |
|---|---|
| `src/ssrf.rs` | DEFAULT_DENY_RANGES, SsrfConfig |
| `src/permissions.rs` | PermissionsContainer creation com politicas variadas |
| `src/cpu_timer.rs` | CLOCK_THREAD_CPUTIME_ID + wall-clock fallback, reset per-request |
| `src/mem_check.rs` | HeapLimitState, near_heap_limit_callback, GlobalMemoryTracker |
| `src/isolate.rs` | IsolateConfig, IsolateHandle, request/response types |
| `src/isolate_logs.rs` | Log collector (ring buffer 10k entries) |
| `src/extensions.rs` | Extension registration, stub ops, transpiler |
| `src/module_loader.rs` | EszipModuleLoader com source maps opcionais |
| `src/manifest.rs` | JSON Schema validation, profile resolution, denylist collision check |
| `src/bootstrap.js` | Global setup, API deletion (sandbox), global freezing |

### cli (9 arquivos)

| Arquivo | Funcao |
|---|---|
| `src/main.rs` | CLI entrypoint (clap), telemetry init, V8 platform |
| `src/telemetry.rs` | OpenTelemetry setup (traces/metrics/logs), isolate log bridge |
| `src/commands/start.rs` | Producao: dual-listener, todos os flags de seguranca |
| `src/commands/test.rs` | Test runner, inspector server com HTTP parser custom |
| `src/commands/bundle.rs` | Bundler offline (eszip) |
| `src/commands/check.rs` | TypeScript/JS checker |
| `src/commands/watch.rs` | Dev mode: file watching, auto-bundle, live reload |
| `src/commands/embedded_assert.rs` | Modulos assert embutidos via `include_str!()` |

---

## 7. Observacoes Positivas

A arquitetura fundamental e solida e evoluiu significativamente desde o audit original:

**Seguranca implementada:**
- **TLS funcional** com hot-reload de certificados e fingerprint logging
- **Dual-listener** separando admin (porta 9000) de ingress (porta 8080 / Unix socket)
- **Autenticacao API key** no admin router
- **SSRF protection** com deny ranges IPv4 completos
- **Body limits** dual-layer (Content-Length + streaming Limited)
- **Connection semaphore** com permit lifecycle correto
- **Bundle signing** Ed25519 com enforcement configuravel
- **Function name validation** com regex estrita
- **Error sanitization** consistente nos routers principal
- **W3C Trace Context** com sampling configuravel
- **Global freezing** dos 10 APIs mais criticos

**Arquitetura:**
- **Escolha tecnologica solida:** Deno core + V8 + eszip + hyper + tower + rustls
- **Separacao de crates bem definida:** `runtime-core` (sandbox), `functions` (lifecycle), `server` (HTTP), `cli` (tooling)
- **Panic recovery robusto:** `catch_unwind` + status update + channel replacement + backoff exponencial
- **Dual-layer timeout:** V8 `terminate_execution` + `tokio::time::timeout` + watchdog thread
- **Near-heap-limit callback** com extensao + terminacao
- **DashMap** para registry thread-safe sem locks coarse
- **Isolate pooling** com round-robin, LRU eviction e guardas de memoria
- **Observabilidade completa** com OpenTelemetry (traces, metrics, logs)
- **Test coverage significativamente melhorada** com 13 arquivos de teste Rust + 10 suites JS

**O progresso desde o audit original e substancial.** Dos 23 findings originais, 21 foram corrigidos. As vulnerabilidades restantes sao predominantemente novas descobertas desta re-auditoria, concentradas em:
1. SSRF IPv6 bypass (limitacao de parser)
2. Router legado sem security features
3. Gaps em streaming, timing e enforcement de CPU
4. Cobertura de testes para features recem-implementadas
