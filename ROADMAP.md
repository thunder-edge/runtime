# Roadmap de Correções — Deno Edge Runtime

> Baseado na auditoria de segurança e arquitetura realizada em 05/03/2026.
> Cada item referencia o finding correspondente no `AUDIT.md`.
>
> Última atualização: 07/03/2026 (P1 de VFS seguro em `node:fs` concluído com quotas configuráveis por manifest/flag/env, `http/https` client-side compat, P2 de `node:dns` via DoH controlado, expansão de `node:util`/`node:diagnostics_channel`, `async_hooks`/ALS com propagação real, P3 de `node:zlib` funcional parcial com backend nativo, P1 inicial de `node:crypto`, avanço de `node:stream` com cancelamento por `AbortSignal` em `pipeline` e teste E2E de resposta chunked progressiva; backlog do `ROADMAP-NODE-COMPAT.md` consolidado neste documento).
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
- [ ] Fechar semântica de streams com backpressure real:
    - `pause/resume`, `highWaterMark`, sinalização de pressão em `push`, ajuste em `pipeline/pipe`.
    - Status aplicado (parcial): `Readable.pause/resume`, `highWaterMark` e sinalização básica de backpressure em `push`/`pipe` já implementados; `pipeline` suporta `AbortSignal` com cancelamento/teardown da cadeia e callback de erro determinístico.
    - Status aplicado (parcial): `Writable` passou a considerar bytes enfileirados contra `highWaterMark` e a finalizar `end()` somente após drenagem completa (evitando perda de chunks em escrita assíncrona).
    - Status aplicado (parcial): cobertura com teste dedicado `node_stream_pipeline_handles_backpressure_on_long_flow`, E2Es de ingress chunked (progressivo e fluxo longo) e bridges Web<->Node streams (`fromWeb`/`toWeb`) com testes dedicados; cenários extremos de pressão ainda evolutivos.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.2`, `§7.1.2`, `§9 Issue #2`, `§10 Phase 1`.
- [ ] Expandir propagação de contexto ALS além de Promise/microtask/timers:
    - EventEmitter handlers e callbacks assíncronos críticos (incluindo `fs`).
    - Status aplicado (parcial): propagação de ALS para listeners de `EventEmitter` implementada; callbacks críticos adicionais (incluindo `fs`) ainda pendentes.
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.3`, `§7.1.3`, `§9 Issue #3`, `§9 Issue #10`, `§10 Phase 1/2`.

#### P2 — Compatibilidade de I/O e HTTP em Perfil Seguro

- [ ] Implementar `fs.createReadStream` e `fs.createWriteStream` no VFS (sem acesso ao host).
    - Referência: `ROADMAP-NODE-COMPAT.md §5.1.4`, `§7.2.2`, `§9 Issue #4`, `§10 Phase 2`.
- [ ] Suporte limitado de `http.createServer` stub
    - Referência: `ROADMAP-NODE-COMPAT.md §7.2.1`, `§9 Issue #6`, `§10 Phase 4`.
- [ ] Opcional de segurança criptográfica (após P1):
    - `createCipheriv`/`createDecipheriv` e KDFs (`pbkdf2`/`scrypt`) conforme perfil de risco.
    - Referência: `ROADMAP-NODE-COMPAT.md §7.3`, `§9 Issue #7`, `§10 Phase 3`.

#### P3 — Hardening e Governança de Compat

- [ ] Implementar rate limiting de saída (egress) por função/perfil para reduzir abuso de rede.
    - Referência: `ROADMAP-NODE-COMPAT.md §4.4` (Security checklist TODO: outbound rate limiting).
- [ ] Implementar verificação de integridade do VFS (detecção de corrupção/estado inválido).
    - Referência: `ROADMAP-NODE-COMPAT.md §4.4` (Security checklist TODO: VFS integrity checking).
- [ ] Publicar matriz de compatibilidade Node em formato consultável por humanos e CI:
    - Documento `docs/NODE-COMPAT.md` + gate de regressão no CI para níveis `Full/Partial/Stub/None`.
    - Referência: `ROADMAP-NODE-COMPAT.md §8`, `§9 Issue #8`, `§10 Phase 5`.
- [ ] Adicionar stub explícito para `node:worker_threads` com erro determinístico orientando limitações de sandbox.
    - Referência: `ROADMAP-NODE-COMPAT.md §7.3.1`, `§9 Issue #5`.

#### P4 — Performance e Operação Contínua

- [ ] Benchmark e otimização de throughput/latência das novas APIs de `node:crypto`.
    - Referência: `ROADMAP-NODE-COMPAT.md §9 Issue #9`, `§10 Phase 5`.

#### Critério de Conclusão da Consolidação

- [ ] Todo item pendente de `ROADMAP-NODE-COMPAT.md` deve apontar para esta seção ou estar marcado como concluído/descartado com justificativa.
- [ ] Não manter backlog duplicado divergente entre `ROADMAP.md` e `ROADMAP-NODE-COMPAT.md`.

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
