# Roadmap de Correções — Deno Edge Runtime

> Baseado na auditoria de segurança e arquitetura realizada em 05/03/2026.
> Cada item referencia o finding correspondente no `AUDIT.md`.
>
> Última atualização: 08/03/2026 (P1 de VFS seguro em `node:fs` concluído com quotas configuráveis por manifest/flag/env, `http/https` client-side compat, P2 de `node:dns` via DoH controlado, expansão de `node:util`/`node:diagnostics_channel`, `async_hooks`/ALS com propagação real, P3 de `node:zlib` funcional parcial com backend nativo, P1 inicial de `node:crypto`, avanço de `node:stream` com cancelamento por `AbortSignal` em `pipeline` e teste E2E de resposta chunked progressiva, P3 de hardening/governança com rate limiting de egress por execução, verificação de integridade de VFS e gate de matriz Node em relatório/CI, e P4 inicial com benchmark/otimização de throughput/latência de `node:crypto`; backlog do `ROADMAP-NODE-COMPAT.md` consolidado neste documento).
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

- [x] Implementar ponte robusta Web Streams <-> Node Streams (quando necessário)
- [x] Garantir flush/backpressure corretos em resposta incremental
- [x] Validar `ReadableStream` em respostas longas sem buffering total em memória
- [x] Garantir comportamento consistente de cancelamento (`AbortSignal`) durante stream
- [x] Adicionar teste E2E de SSR streaming com chunked body

Status aplicado (07/03/2026):
- `node:stream.pipeline` agora aceita `signal` em options e aborta a cadeia com teardown/destroy determinístico.
- Teste E2E `e2e_ingress_streaming_returns_progressive_chunked_body` em `crates/server/src/lib.rs` valida resposta chunked progressiva no ingress.
- `Writable` em `node:stream` passou a aplicar pressão por `highWaterMark` com contagem de bytes em buffer e `end()` aguardando drenagem completa antes de `finish/close`.
- Teste E2E `e2e_ingress_streaming_long_chunked_body_completes` em `crates/server/src/lib.rs` valida fluxo chunked longo com marcador inicial/final e conclusão estável.
- Teste `node_stream_pipeline_handles_backpressure_on_long_flow` em `crates/functions/tests/node_module_imports.rs` valida sinalização real de backpressure (`write()` retornando `false`) e `drain` no fluxo com escrita assíncrona.
- Bridge Web Streams <-> Node Streams implementada em `node:stream` com `Readable.fromWeb`/`Readable.toWeb` e `Writable.fromWeb`/`Writable.toWeb`, validada por testes dedicados em `crates/functions/tests/node_module_imports.rs`.

**Critério de aceite:** SSR com streaming envia chunks progressivos, sem deadlock e sem corrupção de body.

---

### 5.4 Async Context por Request

**Objetivo:** suportar isolamento de contexto assíncrono por request (essencial em stacks Next modernas).

- [x] Implementar camada compatível com `AsyncLocalStorage` (ou equivalente funcional)
- [x] Garantir propagação de contexto por awaits/promises/timers
- [x] Isolar contexto entre requests concorrentes
- [x] Adicionar testes de concorrência validando não-vazamento de contexto

Status aplicado (07/03/2026):
- `node:async_hooks` agora expõe bridge interno (`__edgeRuntimeAsyncHooks`) para executar cada request em contexto isolado (`runWithExecutionContext`) e limpar stores em `startExecution/endExecution/clearExecutionTimers`.
- `AsyncLocalStorage.run` passou a preservar contexto corretamente quando o callback retorna `Promise` (restauração adiada para `finally`).
- Teste E2E `e2e_async_local_storage_isolated_between_overlapping_requests` em `crates/server/src/lib.rs` valida requests sobrepostos com IDs distintos sem vazamento de contexto.

**Critério de aceite:** dois requests simultâneos não compartilham estado contextual.

---

### 5.5 HTTP/Web Semantics de Produção

**Objetivo:** corrigir nuances de protocolo que quebram app real mesmo com APIs disponíveis.

- [ ] Preservar múltiplos `Set-Cookie` sem flatten indevido
- [ ] Garantir merge de headers sem perda de semântica
- [ ] Validar clone/tee/locking de body em `Request`/`Response`
- [ ] Revisar comportamento de compressão/encoding em proxy e rewrite
- [ ] Adicionar suíte de regressão para casos reportados em ecossistemas SSR

Status aplicado (07/03/2026):
- Bridge HTTP Rust<->JS em `crates/functions/src/handler.rs` migrou de serialização por `HashMap` para lista ordenada de pares de header (`Vec<(String, String)>`), preservando semântica de headers repetidos.
- `Set-Cookie` múltiplo preservado explicitamente no retorno de `handleRequest` usando entries de headers e `response.headers.getSetCookie()` quando disponível.
- Merge de headers não-`Set-Cookie` mantido conforme semântica Fetch (`Headers`), sem perda de valores lógicos.
- Regressões de body semantics adicionadas em `crates/functions/tests/node_module_imports.rs`:
    - `web_request_clone_preserves_body_and_locks_original_after_read`
    - `web_response_clone_preserves_body_and_locks_original_after_read`
    - `web_stream_tee_splits_stream_without_data_loss`
- Regressão E2E de protocolo adicionada em `crates/server/src/lib.rs`:
    - `e2e_ingress_preserves_http_header_semantics_on_rewrite` valida preservação de `content-encoding`, forwarding de `accept-encoding` no rewrite e múltiplos `Set-Cookie` sem flatten.
- Regressão unitária do bridge adicionada em `crates/functions/src/handler.rs`:
    - `dispatch_preserves_multiple_set_cookie_headers`.

- [x] Preservar múltiplos `Set-Cookie` sem flatten indevido
- [x] Garantir merge de headers sem perda de semântica
- [x] Validar clone/tee/locking de body em `Request`/`Response`
- [x] Revisar comportamento de compressão/encoding em proxy e rewrite
- [x] Adicionar suíte de regressão para casos reportados em ecossistemas SSR

**Critério de aceite:** testes de cookie/header/body passam em dev e prod profile.

---

### 5.6 WebSocket Runtime (Opcional para Vinext, recomendado)

**Objetivo:** habilitar cenários que dependem de upgrade e canais persistentes.

- [x] Carregar extensão de WebSocket (`deno_websocket`) no runtime
- [x] Expor `WebSocket` em `globalThis` no bootstrap
- [x] Implementar testes de handshake + troca de mensagens
- [x] Garantir limites de recurso e timeout para conexões WS

Status aplicado (08/03/2026):
- Runtime carrega `deno_websocket` em `crates/runtime-core/src/extensions.rs` e habilita WebSocket no isolate padrão.
- Bootstrap expõe `globalThis.WebSocket` via wrapper `EdgeWebSocket` em `crates/runtime-core/src/bootstrap.js`, mantendo API padrão e adicionando guardrails:
    - limite de conexões simultâneas por isolate (`128`),
    - timeout de conexão em estado `CONNECTING` (`30s`).
- Regressões de disponibilidade/semântica adicionadas:
    - `crates/functions/tests/cloudflare_networking.rs` valida construtor, constantes e metadados de guardrails;
    - `crates/functions/tests/web_api_compat.rs` valida presença e constantes da API WebSocket.
- Documentação operacional de proxy externo adicionada em `docs/cli.md` com requisitos de forwarding `Upgrade` HTTP/1.1, headers obrigatórios e timeouts de conexão longa.

**Critério de aceite:** cliente `WebSocket` conecta e troca mensagens com estabilidade.

---

### 5.7 Matriz de Compatibilidade (Runtime-Only) — Baixa Prioridade

**Objetivo:** tornar explícito o nível de suporte para Vinext/Next sem cloud features.

> Status de priorização: item explicitamente rebaixado para baixa prioridade e pode ser postergado.

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

- [x] Definir 3 níveis oficiais por API Node:
    - [x] `Full`: implementação funcional
    - [x] `Partial`: implementação parcial com limitações documentadas
    - [x] `Stub`: importável, mas métodos `noop` ou erro determinístico
- [x] Padronizar erro de stub para métodos não implementados:
    - [x] Formato recomendado: `[thunder] <api> is not implemented in this runtime profile`
- [x] Garantir que módulos `Stub` não quebrem no import (quebra apenas na chamada do método)
- [x] Publicar tabela no docs com status por módulo `node:*`

Status aplicado (07/03/2026):
- Runtime `edge_node_compat` expandido para registrar matriz completa de módulos `node:*` suportados pelo perfil, incluindo módulos `Stub` importáveis (`node:test`, `node:sqlite` e demais stubs) em `crates/runtime-core/src/extensions.rs`.
- Erro determinístico de métodos stub padronizado para o formato `[thunder] <api> is not implemented in this runtime profile` nos módulos compat em `crates/runtime-core/src/node_compat/*`.
- Cobertura de regressão atualizada para validar prefixo `[thunder]` e importabilidade de módulos `Stub` em `crates/functions/tests/node_module_imports.rs` e `crates/functions/tests/node_process_compat.rs`.
- Matriz publicada em `docs/NODE-COMPAT.md` e refletida no relatório automático (`crates/functions/tests/web_api_report.rs` -> `docs/web_standards_api_report.md`).

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

- [x] Etapa A (base de execução):
    - [x] `node:buffer`
    - [x] `node:process`
    - [x] `node:events`
    - [x] `node:util`
    - [x] `node:path`
- [x] Etapa B (SSR/RSC):
    - [x] `node:stream`
    - [x] `node:string_decoder`
    - [x] `node:module` (parcial)
    - [x] `node:os` (partial/stub)
- [x] Etapa C (rede e protocolos):
    - [x] `node:http` (parcial)
    - [x] `node:https` (parcial)
    - [x] `node:net` (parcial)
    - [x] `node:tls` (stub/partial)
- [x] Etapa D (baixo encaixe serverless):
    - [x] `node:child_process` (stub)
    - [x] `node:cluster` (stub)
    - [x] `node:repl` (stub)
    - [x] `node:dgram` (stub)

Status aplicado desta trilha: etapas A/B/C/D concluídas em perfil `Full/Partial/Stub` com cobertura em `node_module_imports` e classificação no relatório `web_api_report`.

**Critério de aceite:** cada etapa possui suíte de regressão e status atualizado em matriz `Full/Partial/Stub/None`.

---

### 5.12 Flags de Compatibilidade de Runtime

Não implementar flag de compatibilidade, node compat será ativo por padrão.
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
- **Runtime atual:** subset compatível com `env` em memória, `stdout/stderr/stdin` compatíveis, `cwd` virtual `/bundle` e `chdir` controlado por VFS seguro.
- **Status:** gap reduzido; divergências remanescentes em integração com bindings/plataforma.

2. `node:http` e `node:https`
- **Cloudflare:** `request/get` funcionais como wrapper de `fetch` (com restrições); suporte adicional a server-side APIs via `cloudflare:node` + flags.
- **Runtime atual:** `request/get` funcionais via wrapper de `fetch`; APIs server-side (`createServer`) seguem não funcionais por sandbox.
- **Status:** gap reduzido; divergência principal permanece no lado server-side.

3. `node:fs` e `node:fs/promises`
- **Cloudflare:** VFS com `/bundle` (read-only), `/tmp` (ephemeral por request), `/dev/*`; ampla API com limitações documentadas.
- **Runtime atual:** VFS seguro com `/bundle` read-only, `/tmp` efêmero e `/dev/null`, com quotas configuráveis por manifest/CLI/env.
- **Status:** gap reduzido; cobertura de APIs ainda parcial por design de sandbox.

4. `node:dns`
- **Cloudflare:** maioria da API disponível via DoH/1.1.1.1; apenas alguns métodos não implementados (`lookup`, `lookupService`, `resolve`).
- **Runtime atual:** subset funcional via DoH (`lookup`, `resolve*`, `reverse` e `dns.promises` equivalentes), com limites/timeout configuráveis; restante em stub determinístico.
- **Status:** gap reduzido para médio.

5. `node:net`
- **Cloudflare:** `net.Socket`/`connect` suportados para outbound TCP; `net.Server` não suportado.
- **Runtime atual:** outbound `connect/createConnection` disponível; `net.Server` permanece stub.
- **Status:** alinhado no essencial de outbound, com gap residual em superfície avançada.

6. `node:tls`
- **Cloudflare:** `connect`, `TLSSocket`, `checkServerIdentity`, `createSecureContext` disponíveis; server-side TLS Node não suportado.
- **Runtime atual:** `connect` disponível para subset cliente outbound; APIs de contexto/servidor permanecem stub determinístico.
- **Status:** gap reduzido para médio.

7. `node:url`
- **Cloudflare:** `domainToASCII`/`domainToUnicode` e demais APIs de URL documentadas.
- **Runtime atual:** suporte funcional em subset com `domainToASCII`/`domainToUnicode` e helpers de file URL.
- **Status:** item priorizado concluído, gap reduzido.

8. `node:util`
- **Cloudflare:** `promisify/callbackify`, `util.types` (com subset explícito), `MIMEType`.
- **Runtime atual:** subset prático com `format`, `inspect`, `promisify`, `types`, `MIMEType` e `MIMEParams`.
- **Status:** gap médio.

9. `node:diagnostics_channel`
- **Cloudflare:** inclui `TracingChannel` e integração com Tail Workers.
- **Runtime atual:** pub/sub com `TracingChannel`/`tracingChannel` e hooks de trace (`start/end/asyncStart/asyncEnd/error`).
- **Status:** gap reduzido; diferenças remanescentes em integração de plataforma.

10. `node:async_hooks` / `AsyncLocalStorage`
- **Cloudflare:** ALS funcional com caveats documentados, `AsyncResource` parcial.
- **Runtime atual:** ALS funcional com propagação em `Promise`/microtask, propagação adicional em handlers de `EventEmitter` e hooks básicos (`createHook`, async IDs, `AsyncResource` subset).
- **Status:** gap reduzido para médio.

11. `node:zlib`
- **Cloudflare:** módulo funcional (gzip/deflate/brotli).
- **Runtime atual:** subset funcional one-shot async+sync (`gzip/gunzip/deflate/inflate/deflateRaw/inflateRaw`) com backend nativo, limites configuráveis e hard ceilings.
- **Status:** gap reduzido; brotli e stream constructors permanecem pendentes.

12. `node:events` e `node:buffer`
- **Cloudflare:** suporte amplo (com diferenças específicas documentadas).
- **Runtime atual:** funcionais para casos comuns, com `EventEmitter` preservando contexto ALS no registro/execução de listeners; cobertura ainda parcial no relatório.
- **Status:** reduzir gap via testes de semântica avançada e edge-cases.

13. `node:crypto`
- **Cloudflare:** módulo funcional com subset amplo de hash/HMAC/cipher/KDF e APIs síncronas/assíncronas.
- **Runtime atual:** subset mínimo funcional com `randomBytes`, `randomFill`, `randomFillSync`, `createHash` e `createHmac`; backend híbrido WebCrypto + ops nativas para hash/HMAC (atualmente `SHA-256`/`SHA-512`).
- **Status:** gap reduzido para médio; `createCipheriv`/`createDecipheriv`/KDFs permanecem pendentes.

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
- [x] **P2:** elevar `async_hooks`/ALS de stub para uso real com testes de propagação de contexto.
    - Status aplicado: `AsyncLocalStorage` com propagação de contexto para `Promise.then/catch` e `queueMicrotask` via instrumentação de callbacks.
    - Status aplicado: `createHook` funcional (subset) com `enable/disable` e eventos (`init`, `before`, `after`, `destroy`) em recursos instrumentados.
    - Status aplicado: `executionAsyncId`/`triggerAsyncId` e `AsyncResource.runInAsyncScope` com IDs estáveis no escopo compat.
- [x] **P3:** substituir `zlib` stub por implementação funcional (ou bridge para APIs nativas de compressão).
    - Status aplicado: subset funcional one-shot assíncrono e síncrono em `node:zlib` (`gzip/gunzip/deflate/inflate/deflateRaw/inflateRaw` e `*Sync`) com bridge para op nativa (`op_edge_zlib_transform`), defaults configuráveis por runtime (`IsolateConfig`/CLI) e caps rígidos imutáveis de input/output, além de guardrail de tempo por operação.
    - Status aplicado: APIs sync e construtores de stream não suportados permanecem em stub determinístico (`ERR_NOT_IMPLEMENTED`) para manter previsibilidade no sandbox.

**Critério de aceite desta trilha:**
- Matriz `node:*` no relatório com classificação convergente ao baseline Cloudflare.
- Diferenças remanescentes explicitamente documentadas como "intencionais por sandbox".
- Nenhuma feature de compatibilidade libera acesso ao host físico.

---

### 5.14 Backlog Integrado Node Compat (Fonte Única de Execução)

> Esta seção consolida os itens pendentes do `ROADMAP-NODE-COMPAT.md` para evitar backlog paralelo.
> Referências canônicas: `ROADMAP-NODE-COMPAT.md §5`, `§7`, `§8`, `§9`, `§10`.

#### P1 — Crítico para SSR Frameworks (Next/Remix)

- [x] Implementar `node:crypto` (bridge sobre WebCrypto) com subset mínimo:
    - `randomBytes`, `randomFill`, `randomFillSync`, `createHash`, `createHmac`.
    - Status aplicado: módulo `node:crypto` disponível no runtime com `randomBytes`/`randomFill` via WebCrypto e `createHash`/`createHmac` via ops nativas (`op_edge_crypto_hash`, `op_edge_crypto_hmac`) para algoritmos suportados (`SHA-256`/`SHA-512`).
    - Status aplicado: suíte dedicada adicionada em `crates/functions/tests/node_crypto_streams_async_hooks.rs` para cobertura inicial de carregamento e APIs principais.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.1`, `§7.1.1`, `§9 Issue #1`, `§10 Phase 1`.
- [x] Fechar semântica de streams com backpressure real:
    - `pause/resume`, `highWaterMark`, sinalização de pressão em `push`, ajuste em `pipeline/pipe`.
    - Status aplicado: `Readable.pause/resume`, `highWaterMark` e sinalização de backpressure em `push`/`pipe` implementados; `pipeline` suporta `AbortSignal` com cancelamento/teardown da cadeia e callback de erro determinístico.
    - Status aplicado: `Writable` considera bytes enfileirados contra `highWaterMark` e finaliza `end()` somente após drenagem completa (evitando perda de chunks em escrita assíncrona).
    - Status aplicado: cobertura com teste dedicado `node_stream_pipeline_handles_backpressure_on_long_flow`, E2Es de ingress chunked (progressivo e fluxo longo) e bridges Web<->Node streams (`fromWeb`/`toWeb`) com testes dedicados.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.2`, `§7.1.2`, `§9 Issue #2`, `§10 Phase 1`.
- [x] Expandir propagação de contexto ALS além de Promise/microtask/timers:
    - EventEmitter handlers e callbacks assíncronos críticos (incluindo `fs`).
    - Status aplicado: propagação de ALS para listeners de `EventEmitter` implementada e preservação de contexto em callbacks de `node:fs` via wrapper dedicado em `node:async_hooks`.
    - Status aplicado: bridge de execução (`__edgeRuntime`) passou a aplicar guard de lifecycle para callbacks pendentes (`setTimeout`, `setInterval`, `queueMicrotask`), evitando execução após `clearExecutionTimers/endExecution`.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.3`, `§7.1.3`, `§9 Issue #3`, `§9 Issue #10`, `§10 Phase 1/2`.

#### P2 — Compatibilidade de I/O e HTTP em Perfil Seguro

- [x] Implementar `fs.createReadStream` e `fs.createWriteStream` no VFS (sem acesso ao host).
    - Status aplicado: `node:fs` agora expõe `createReadStream` com leitura chunked sobre VFS (suporte a `start/end/highWaterMark` e `encoding`) e `createWriteStream` com escrita incremental em `/tmp` e `/dev/null`, com `flags` `w/a`.
    - Status aplicado: validações de sandbox e quotas do VFS aplicadas nos streams com erros determinísticos (`EROFS`, `EOPNOTSUPP`, `ENOSPC`, `ENOENT`).
    - Status aplicado: cobertura adicionada em `crates/functions/tests/node_fs_compat.rs` para roundtrip via stream e erro em mount read-only.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.4`, `§7.2.2`, `§9 Issue #4`, `§10 Phase 2`.
- [x] Suporte limitado de `http.createServer` stub
    - Status aplicado: `node:http` agora expõe `createServer()` retornando instância `Server` importável para feature detection.
    - Status aplicado: `Server.listen()` permanece não funcional por sandbox e falha com erro determinístico `[thunder] http.Server.listen is not implemented in this runtime profile`.
    - Status aplicado: cobertura de regressão adicionada em `crates/functions/tests/node_module_imports.rs` e no relatório automático (`crates/functions/tests/web_api_report.rs`).
    - Referência: `ROADMAP-NODE-COMPAT.md §7.2.1`, `§9 Issue #6`, `§10 Phase 4`.
- [ ] Opcional de segurança criptográfica (após P1) — **Baixa prioridade / postergado**:
    - `createCipheriv`/`createDecipheriv` e KDFs (`pbkdf2`/`scrypt`) conforme perfil de risco.
    - Status de priorização: item explicitamente removido da trilha imediata de entrega; executar apenas após fechamento dos itens de hardening/governança mais críticos.
    - Referência: `ROADMAP-NODE-COMPAT.md §7.3`, `§9 Issue #7`, `§10 Phase 3`.

#### P3 — Hardening e Governança de Compat

- [x] Implementar rate limiting de saída (egress) por função/perfil para reduzir abuso de rede.
    - Status aplicado: novo limite `egress_max_requests_per_execution` em `IsolateConfig`, com configuração via manifest (`resources.egressMaxRequestsPerExecution`) e flags/env da CLI (`--egress-max-requests-per-execution` / `EDGE_RUNTIME_EGRESS_MAX_REQUESTS_PER_EXECUTION`).
    - Status aplicado: enforcement por execução no bridge/runtime para `fetch`, `WebSocket`, `node:net.connect` e `node:tls.connect`, com erro determinístico quando o limite é excedido.
    - Status aplicado: regressão adicionada em `crates/functions/src/handler.rs` (`dispatch_enforces_egress_rate_limit_per_execution`).
    - Referência: `ROADMAP-NODE-COMPAT.md §4.4` (Security checklist TODO: outbound rate limiting).
- [x] Implementar verificação de integridade do VFS (detecção de corrupção/estado inválido).
    - Status aplicado: `node:fs` passou a validar invariantes estruturais e contábeis do VFS (diretórios obrigatórios, parents, quotas e `usedBytes`) antes de operações críticas.
    - Status aplicado: corrupção/estado inválido resulta em falha determinística `EIO` sem panic.
    - Status aplicado: regressão adicionada em `crates/functions/tests/node_fs_compat.rs` (`node_fs_detects_vfs_integrity_corruption`).
    - Referência: `ROADMAP-NODE-COMPAT.md §4.4` (Security checklist TODO: VFS integrity checking).
- [x] Publicar matriz de compatibilidade Node em formato consultável por humanos e CI:
    - Status aplicado: validação automática da matriz em `docs/NODE-COMPAT.md` no teste `crates/functions/tests/web_api_report.rs`, com verificação de níveis oficiais e cobertura de módulos esperados.
    - Status aplicado: CI atualizado para executar geração do relatório e falhar quando `docs/web_standards_api_report.md` estiver desatualizado.
    - Documento `docs/NODE-COMPAT.md` + gate de regressão no CI para níveis `Full/Partial/Stub/None`.
    - Referência: `ROADMAP-NODE-COMPAT.md §8`, `§9 Issue #8`, `§10 Phase 5`.
- [x] Adicionar stub explícito para `node:worker_threads` com erro determinístico orientando limitações de sandbox.
    - Status aplicado: módulo `node:worker_threads` agora é importável no perfil compat, com `Worker` e APIs relacionadas falhando de forma determinística sob política de sandbox.
    - Status aplicado: mensagens de erro mantêm prefixo padrão `[thunder] <api> is not implemented in this runtime profile` e explicitam limitação de criação de threads no runtime.
    - Status aplicado: cobertura adicionada em `crates/functions/tests/node_module_imports.rs` e `crates/functions/tests/web_api_report.rs`.
    - Referência: `ROADMAP-NODE-COMPAT.md §7.3.1`, `§9 Issue #5`.

#### P4 — Performance e Operação Contínua

- [x] Benchmark e otimização de throughput/latência das novas APIs de `node:crypto`.
    - Status aplicado: micro-otimizações em `crates/runtime-core/src/node_compat/crypto.ts` para reduzir overhead de hot path (cache de ops nativas, eliminação de cópias evitáveis de `Buffer` e fast path para digest com chunk único).
    - Status aplicado: benchmark dedicado adicionado via `scripts/node-crypto-benchmark.sh` com execução do teste `crypto_microbenchmark_reports_metrics` em `crates/functions/tests/node_crypto_streams_async_hooks.rs`.
    - Resultado de referência (execução local em 08/03/2026): `createHash('sha256')` ~80.476 ops/s (37,28ms/3k), `createHmac('sha256')` ~36.382 ops/s (82,46ms/3k), `randomBytes(32)` ~237.784 ops/s (12,62ms/3k).
    - Referência: `ROADMAP-NODE-COMPAT.md §9 Issue #9`, `§10 Phase 5`.

#### Critério de Conclusão da Consolidação

- [X] Todo item pendente de `ROADMAP-NODE-COMPAT.md` deve apontar para esta seção ou estar marcado como concluído/descartado com justificativa.
- [X] Não manter backlog duplicado divergente entre `ROADMAP.md` e `ROADMAP-NODE-COMPAT.md`.

---

## Fase 6 — Roteamento e Contrato de Funções Moderno

> Objetivo: evoluir o runtime para suportar roteamento baseado em filesystem, deploys multi-rota e um contrato RESTful baseado em `export default`, preservando compatibilidade com o modelo atual e mantendo o prefixo canônico `/{function_id}/...` no runtime.
>
> Documento de referência detalhado: [ROADMAP_ROUTING.md](./ROADMAP_ROUTING.md)

### 6.1 Manifest v2 e Flavors de Deploy

- [ ] Criar `schemas/function-manifest.v2.schema.json`
- [ ] Adicionar parsing e validação v2 em `crates/runtime-core/src/manifest.rs`
- [ ] Introduzir `flavor: single | routed-app`
- [ ] Modelar `routes[]` e `asset` routes para apps frontend/backend
- [ ] Corrigir documentação que hoje pressupõe schema v2 já existente

**Referência:** `ROADMAP_ROUTING.md` seções 5, 6, 7 e 15.

### 6.2 Build, Bundle e Deploy Multi-Rota

- [ ] Estender `crates/cli/src/commands/bundle.rs` para scan de `functions/`
- [ ] Detectar colisões e prioridade de rotas em build time
- [ ] Gerar metadata de rotas e embuti-la no artefato de deploy
- [ ] Aceitar deploys `routed-app` no fluxo atual de `POST /_internal/functions`
- [ ] Preparar suporte opcional a `public/` para assets estáticos

**Referência:** `ROADMAP_ROUTING.md` seções 5, 7, 8, 10 e 13.

### 6.3 Ingress em Dois Estágios e Compatibilidade com Proxy Reverso

- [ ] Preservar `/{function_id}` como primeiro segmento canônico do runtime
- [ ] Resolver o deployment pelo prefixo e rotear por manifest apenas no sufixo restante
- [ ] Documentar explicitamente o mapeamento `{function_id}.my-edge-runtime.com/... -> localhost:9000/{function_id}/...`
- [ ] Indexar e expor rotas no `FunctionRegistry`
- [ ] Implementar matching com prioridade determinística e erro em ambiguidades
- [ ] Fazer short-circuit de rotas de asset sem entrar no isolate

**Referência:** `ROADMAP_ROUTING.md` seções 2, 5, 8, 12 e 14.

### 6.4 Contrato RESTful Baseado em `export default`

- [ ] Implementar suporte oficial a `export default function(req, params?)`
- [ ] Implementar suporte oficial a `export default { GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS }`
- [ ] Retornar `405 Method Not Allowed` com header `Allow` para object handlers sem o verbo correspondente
- [ ] Manter `Deno.serve()` apenas como compatibilidade transitória
- [ ] Alinhar `docs/function-contract-design.md` ao contrato-alvo e remover named exports do caminho recomendado

**Referência:** `ROADMAP_ROUTING.md` seções 5, 7, 9, 11 e 13.

### 6.5 Migração, Exemplos e Observabilidade

- [ ] Criar exemplos completos para `single` e `routed-app`
- [ ] Documentar deploy de app backend e app frontend com assets
- [ ] Escrever guia de migração de `Deno.serve()` para `export default`
- [ ] Adicionar testes E2E cobrindo manifest v1, manifest v2, prefixo `/{function_id}`, routing, params, 405 e assets
- [ ] Expor introspecção administrativa e documentação operacional por rota

**Referência:** `ROADMAP_ROUTING.md` seções 10, 11, 12, 13 e 14.

---

## Fase 7 — Escalabilidade Context + Isolate (Pool Evolutivo)

> Objetivo: evoluir do pool atual (replicas de isolate por função) para um modelo gradual de `N contexts por isolate`, com escala automática para o próximo isolate ao atingir limite de contexts por isolate, respeitando o teto global de isolates do processo.
>
> Documento técnico detalhado: [ROADMAP_CONTEXT_ISOLATE.md](./ROADMAP_CONTEXT_ISOLATE.md)

### 7.1 Macro Passo A — Baseline e Arquitetura-Alvo

- [ ] Validar baseline de comportamento atual do pool por função e registrar gaps para multi-context.
- [ ] Formalizar arquitetura-alvo (`processo -> pool global de isolates -> contexts -> requests`).
- [ ] Definir limites operacionais iniciais (`max_contexts_per_isolate`, `global_max_isolates`, `max_active_requests_per_context`).

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#2-estado-atual-as-is](./ROADMAP_CONTEXT_ISOLATE.md#2-estado-atual-as-is), [ROADMAP_CONTEXT_ISOLATE.md#3-arquitetura-alvo-to-be](./ROADMAP_CONTEXT_ISOLATE.md#3-arquitetura-alvo-to-be)

### 7.2 Macro Passo B — Mudanças Estruturais por Crate

- [x] Evoluir `IsolateRequest` para roteamento por função/context.
- [ ] Refatorar lifecycle para separar bootstrap de isolate e bootstrap de context.
- [x] Migrar `FunctionRegistry` para pool global de isolates + tabela de roteamento por contexts.
- [x] Tornar o bridge JS (`__edgeRuntime`) context-aware (handler por context).

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#4-mudancas-por-crate-implementacao](./ROADMAP_CONTEXT_ISOLATE.md#4-mudancas-por-crate-implementacao)

### 7.3 Macro Passo C — Scheduler Context-First

- [x] Implementar scheduler `context-first, isolate-next`.
- [x] Criar context novo antes de escalar isolate; ao atingir limite de contexts, abrir novo isolate automaticamente.
- [x] Aplicar shedding determinístico quando limite global de isolates for atingido.

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#5-algoritmo-de-scheduling](./ROADMAP_CONTEXT_ISOLATE.md#5-algoritmo-de-scheduling)

### 7.4 Macro Passo D — Rollout Gradual com Feature Flags

- [ ] Entregar instrumentação primeiro (sem mudança de comportamento).
- [x] Habilitar modo context pool via flag, iniciando por canário.
- [ ] Expandir progressivamente até ativação ampla com fallback para modo legado.

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#6-rollout-gradual-sem-regressao](./ROADMAP_CONTEXT_ISOLATE.md#6-rollout-gradual-sem-regressao)

### 7.5 Macro Passo E — Deploy/Update/Drain Sem Interrupção

- [ ] Garantir deploy de context com atualização atômica de roteamento.
- [ ] Implementar update por versão (`v+1`) com draining dos contexts antigos.
- [ ] Implementar remoção de função em todos os isolates com reciclagem de capacidade.

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#7-deploy-update-e-drain](./ROADMAP_CONTEXT_ISOLATE.md#7-deploy-update-e-drain)

### 7.6 Macro Passo F — SLOs, Testes e Hardening

- [x] Adicionar métricas de saturação por context e por isolate.
- [x] Cobrir trilha completa com testes unitários, integração, caos e benchmark comparativo.
- [x] Fechar riscos de isolamento e roteamento incorreto em multi-context.

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#8-observabilidade-e-slos](./ROADMAP_CONTEXT_ISOLATE.md#8-observabilidade-e-slos), [ROADMAP_CONTEXT_ISOLATE.md#9-testes-necessarios](./ROADMAP_CONTEXT_ISOLATE.md#9-testes-necessarios), [ROADMAP_CONTEXT_ISOLATE.md#10-compatibilidade-e-riscos](./ROADMAP_CONTEXT_ISOLATE.md#10-compatibilidade-e-riscos)

### 7.7 Macro Passo G — Atualização de Documentação Existente

- [ ] Atualizar documentação de arquitetura, operação e tuning para refletir modo multi-context.
- [ ] Publicar diferenças de comportamento entre modo legado e novo modo.

Detalhes: [ROADMAP_CONTEXT_ISOLATE.md#11-plano-de-atualizacao-de-documentacao-existente](./ROADMAP_CONTEXT_ISOLATE.md#11-plano-de-atualizacao-de-documentacao-existente)

### 7.8 Backlog Executável por PRs (Plano de Entrega)

> Objetivo: transformar a Fase 7 em entregas incrementais, com merge seguro e rollback simples.
>
> Dependência técnica detalhada: [ROADMAP_CONTEXT_ISOLATE.md](./ROADMAP_CONTEXT_ISOLATE.md)

#### PR1 — `IsolateRequest` + Feature Flags + Instrumentação Base

**Objetivo:** preparar o runtime para roteamento por contexto sem alterar comportamento padrão.

**Escopo:**
- [x] Evoluir `IsolateRequest` para incluir metadados de destino lógico (`function_name` e `context_id` opcional).
- [x] Adicionar flags de runtime:
    - [x] `EDGE_RUNTIME_CONTEXT_POOL_ENABLED`
    - [x] `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE`
    - [x] `EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT`
- [x] Manter default em modo legado (sem multi-context ativo por padrão).
- [ ] Incluir métricas base de contexto no modo legado (1 context por isolate), sem impacto funcional.

**Arquivos-alvo (mínimo):**
- `crates/runtime-core/src/isolate.rs`
- `crates/cli/src/commands/start.rs`
- `crates/functions/src/types.rs`
- `crates/functions/src/registry.rs`
- `docs/cli.md`

**Critério de aceite:**
- [x] Nenhuma regressão na rota atual (`/{function_name}/*`).
- [x] Novas flags aparecem no CLI/help e envs funcionam.
- [x] Build/test atuais passam sem habilitar context pool.

#### PR2 — Dispatch por Context (Bridge + Runtime)

**Objetivo:** habilitar execução orientada a contexto dentro do isolate.

**Escopo:**
- [x] Tornar bridge JS context-aware (`handler` por context em vez de singleton global).
- [x] Introduzir caminho de dispatch por contexto no runtime (`dispatch_request_for_context(...)`).
- [ ] Ajustar lifecycle para carregar/atualizar context de função sem quebrar bootstrap atual.

**Arquivos-alvo (mínimo):**
- `crates/functions/src/handler.rs`
- `crates/functions/src/lifecycle.rs`
- `crates/runtime-core/src/isolate.rs`

**Critério de aceite:**
- [ ] Duas funções no mesmo isolate podem coexistir com handlers distintos.
- [ ] Sem vazamento de handler/estado entre contexts.
- [x] Modo legado continua funcionando quando flag desabilitada.

#### PR3 — Scheduler `context-first, isolate-next` + Pool Global + Métricas

**Objetivo:** ativar estratégia de escala automática context->isolate com limites configuráveis.

**Escopo:**
- [x] Migrar `FunctionRegistry` para tabela de roteamento por context + pool global de isolates.
- [x] Implementar scheduler:
    - [x] reutiliza context existente antes de criar novo
    - [x] cria novo isolate quando `max_contexts_per_isolate` for atingido
    - [x] respeita `global_max_isolates` e aplica shedding determinístico em saturação
- [x] Expor métricas de saturação por context e isolate.

**Arquivos-alvo (mínimo):**
- `crates/functions/src/registry.rs`
- `crates/functions/src/types.rs`
- `crates/functions/src/metrics.rs`
- `crates/server/src/router.rs`
- `docs/external-scaling-recommendations.md`

**Critério de aceite:**
- [x] Escala automática comprovada em teste (context lotado -> novo isolate).
- [x] Erro determinístico (`503`) quando limite global for atingido.
- [x] Métricas novas disponíveis no endpoint de métricas.

#### PR4 — E2E, Hardening, Rollout e Atualização de Docs

**Objetivo:** fechar trilha com qualidade de produção e documentação consolidada.

**Escopo:**
- [ ] Adicionar E2Es de concorrência, drain e hot-reload por versão de context.
- [ ] Adicionar testes de caos (queda de isolate com múltiplos contexts).
- [ ] Documentar rollout gradual (canário, fallback legado, playbook operacional).
- [ ] Atualizar documentos canônicos para refletir modo multi-context.

**Arquivos-alvo (mínimo):**
- `crates/server/src/lib.rs`
- `docs/timeout-and-resource-tracking.md`
- `docs/external-scaling-recommendations.md`
- `CURRENT_ARCHITECTURE_ANALYSIS.md`
- `README.md`
- `docs/cli.md`
- `docs/NODE-COMPAT.md`

**Critério de aceite:**
- [ ] Suite E2E cobrindo coexistência multi-funcao no mesmo isolate e isolamento por context.
- [ ] Documentação operacional atualizada com tuning e troubleshooting.
- [ ] Procedimento de rollback para modo legado documentado e validado.

#### Ordem Recomendada de Merge

1. PR1
2. PR2
3. PR3
4. PR4

#### Regras de Go/No-Go por PR

- [ ] Todo PR deve manter compatibilidade com modo legado por default.
- [ ] Todo PR deve incluir regressões automatizadas para o escopo alterado.
- [ ] Nenhum PR deve alterar contrato externo de deploy/ingress sem seção de migração explícita.
- [ ] Se houver degradação de latência p95 significativa, bloquear avanço para o próximo PR até mitigação.

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
| **Roteamento FS - Acurácia de Matching** | > 99.9% rotas matched corretamente |
| **Contrato RESTful - Adoção** | > 80% novas functions em v2.0+ |
| **Migration Success Rate** | > 95% funções migram sem reescrita |
