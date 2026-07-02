# Roadmap Context + Isolate Scaling

> Plano de evolucao incremental para reaproveitar o pool atual e suportar `N contexts por isolate`, com escalonamento automatico para novos isolates quando atingir o limite de contexts por isolate.
>
> Baseado na implementacao atual em:
> - `crates/functions/src/registry.rs`
> - `crates/functions/src/lifecycle.rs`
> - `crates/functions/src/handler.rs`
> - `crates/runtime-core/src/isolate.rs`
>
> Ultima atualizacao: 08/03/2026

---

## 1. Objetivo

Evoluir o runtime de:

- pool por funcao com replicas de isolate (modelo atual)

para:

- pool de isolates no processo
- `N contexts por isolate` (multi-funcao)
- escalonamento automatico:
  - primeiro tenta alocar novo context em isolate existente
  - ao atingir `max_contexts_per_isolate`, cria/ativa proximo isolate
  - respeita `global_max_isolates` e limites por funcao

Sem quebrar o contrato atual de deploy/ingress (`/_internal/functions` e `/{function_name}/*`).

---

## 2. Estado Atual (as-is)

## 2.1 Modelo de pool atual

- Pool e implementado por `FunctionEntry` com:
  - `isolate_handle` (primario)
  - `extra_isolate_handles` (replicas)
  - `next_handle_index` (round-robin)
- Referencia: `crates/functions/src/types.rs` e `crates/functions/src/registry.rs`.

Comportamento atual:

- `FunctionRegistry::get_handle(name)` escolhe um handle vivo em round-robin.
- Cada handle representa um isolate/thread com um `JsRuntime`.
- Cada funcao escalada replica o runtime inteiro da propria funcao.

## 2.2 Modelo de execucao atual

- `create_function()` cria thread dedicada e um loop `run_isolate()` por funcao.
- Cada isolate recebe requests via canal `mpsc::UnboundedReceiver<IsolateRequest>`.
- `IsolateRequest` nao carrega identidade de context/funcao, apenas request+reply channel.
- Referencia: `crates/functions/src/lifecycle.rs`, `crates/runtime-core/src/isolate.rs`.

## 2.3 Isolamento de request atual

- Ja existe isolamento logico por request com `execution_id`:
  - `startExecution(execution_id)`
  - `endExecution(execution_id)`
  - `clearExecutionTimers(execution_id)`
- Referencia: `crates/functions/src/lifecycle.rs`, `crates/functions/src/handler.rs`.

Limite atual:

- O isolamento por request nao equivale a isolamento por context entre multiplas funcoes no mesmo isolate.

## 2.4 Snapshot

- Formato `BundleFormat::Snapshot` existe, mas `load_from_snapshot()` ainda faz fallback para eszip.
- Referencia: `crates/functions/src/lifecycle.rs`.

Impacto:

- cold start de novos isolates/contexts depende de eszip no estado atual.

---

## 3. Arquitetura Alvo (to-be)

## 3.1 Hierarquia de escalonamento

1. Processo (container)
2. Pool global de isolates
3. Contexts por isolate
4. Funcoes mapeadas para contexts
5. Requests por context (com `execution_id` por request)

## 3.2 Regra de escala desejada

Para uma requisicao de funcao `F`:

1. Se existe context de `F` com capacidade no isolate `I`: rotear para ele.
2. Senao, se existe isolate `I` com `contexts_count < max_contexts_per_isolate`: criar context de `F` em `I`.
3. Senao, se `total_isolates < global_max_isolates`: criar novo isolate `I+1`, criar context de `F` nele e rotear.
4. Senao: aplicar shedding controlado (`503` com erro deterministico de capacidade).

## 3.3 Modelo de dados alvo

- `GlobalIsolatePool`
  - `isolate_id`
  - `status` (`warm`, `draining`, `dead`)
  - `contexts_count`
  - `active_requests`
  - `mem_usage_estimate`
- `FunctionContextRef`
  - `function_name`
  - `context_id`
  - `isolate_id`
  - `version` (para deploy/hot-reload)
  - `status` (`ready`, `loading`, `draining`)

---

## 4. Mudancas por crate (implementacao)

## 4.1 `runtime-core` (`crates/runtime-core/src/isolate.rs`)

Mudancas necessarias:

- Evoluir `IsolateRequest` para carregar destino logico:
  - `function_name` (ou `function_id`)
  - `context_id` (opcional para roteamento deterministico)
  - request e response channel (mantidos)
- Introduzir `ContextCapacity`/`IsolateCapacity` em `IsolateConfig`:
  - `max_contexts_per_isolate`
  - `max_active_requests_per_context`

Exemplo de shape alvo (conceitual):

```rust
pub struct IsolateRequest {
    pub function_name: String,
    pub context_id: Option<String>,
    pub request: http::Request<bytes::Bytes>,
    pub response_tx: oneshot::Sender<Result<IsolateResponse, Error>>,
}
```

## 4.2 `functions` - lifecycle (`crates/functions/src/lifecycle.rs`)

Mudancas necessarias:

- Separar bootstrap em 2 niveis:
  - bootstrap do isolate (uma vez por isolate)
  - bootstrap de context por funcao (carregar modulo e registrar handler no context alvo)
- Substituir `create_function()` (funcao -> thread) por:
  - `create_isolate_unit()`
  - `create_or_update_function_context(isolate_id, function_bundle, function_name)`
- `run_isolate()` precisa operar sobre multiplo mapa de contexts:
  - `HashMap<function_name, ContextState>`

Observacao importante:

- O `handler::dispatch_request(...)` atual usa `main_context()`.
- Para multi-context sera necessario adicionar `dispatch_request_for_context(...)` que entra no `v8::Context` correspondente antes de invocar o handler.

## 4.3 `functions` - registry (`crates/functions/src/registry.rs`)

Mudancas necessarias:

- Trocar indexacao principal de `FunctionEntry -> handles` para:
  - `FunctionRoutingTable` (funcao -> lista de contexts)
  - `GlobalIsolatePool` (isolate -> contexts/carga)
- Substituir `get_handle(name)` por `get_route_target(name)`:
  - retorna `(isolate_handle, context_id)`
- Reusar heuristica existente de capacidade:
  - `global_max_isolates`
  - `min_free_memory_mib`
  - LRU eviction (adaptada para context/isolate)

## 4.4 `functions` - bridge/handler (`crates/functions/src/handler.rs`)

Mudancas necessarias:

- Isolar `__edgeRuntime.handler` por context (hoje e singleton global).
- Criar registro por context no JS bridge:
  - `globalThis.__edgeRuntimeHandlers = Map<contextId, fn>`
- `handleRequest(...)` deve receber `contextId` e despachar handler correto.

Exemplo conceitual de API interna:

```javascript
globalThis.__edgeRuntime.registerHandlerForContext = function(contextId, fn) {
  this._handlers.set(contextId, fn);
};
```

## 4.5 `server` ingress/router (`crates/server/src/router.rs`)

Mudancas necessarias:

- No envio para isolate, alem do request atual, incluir `function_name` no `IsolateRequest`.
- Sem mudanca no contrato externo de rota (`/{function_name}/*`).

---

## 5. Algoritmo de Scheduling

## 5.1 Estrategia recomendada

Algoritmo `context-first, isolate-next`:

1. Encontrar context pronto da funcao com menor carga (`active_requests`).
2. Se nenhum context pronto:
  - procurar isolate com menor carga e slot de context livre.
  - criar context nesse isolate.
3. Se nao houver slot de context:
  - tentar criar novo isolate (respeitando `global_max_isolates`).
  - criar context inicial nele.
4. Se bloqueado por limite global/memoria:
  - responder `503` com metrica de saturacao.

## 5.2 Politicas de afinidade

- Afinidade soft por funcao:
  - priorizar isolates que ja hospedam contexts da mesma funcao (melhor locality).
- Spillover:
  - quando limite de contexts por isolate e atingido, migrar para proximo isolate do pool.

## 5.3 Limites recomendados

- `max_contexts_per_isolate`: iniciar conservador (ex.: 8-32).
- `max_active_requests_per_context`: iniciar com 1-4 conforme perfil.
- `global_max_isolates`: manter limite atual ja configuravel por CLI/env.

---

## 6. Rollout Gradual (sem regressao)

## 6.1 Fase A - Instrumentacao (sem comportamento novo)

- Introduzir metricas de context mesmo no modo atual (1 context por isolate).
- Coletar:
  - tempo de bootstrap por context
  - requests por context
  - erros por context

## 6.2 Fase B - Feature flag de context pool

- Adicionar flags:
  - `EDGE_RUNTIME_CONTEXT_POOL_ENABLED`
  - `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE`
  - `EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT`
- Default: desativado (preserva comportamento atual).

## 6.3 Fase C - Context multi-funcao em isolate unico (canary)

- Habilitar para subconjunto de funcoes marcadas.
- Escalonar context antes de criar novos isolates.

## 6.4 Fase D - Escalonamento automatico isolate->context completo

- Ativar algoritmo completo para todo o pool.
- Manter fallback rapido para modo legacy em caso de incidente.

---

## 7. Deploy, Update e Drain

## 7.1 Deploy de nova funcao

1. Criar contexto em isolate com capacidade.
2. Marcar contexto `ready`.
3. Atualizar tabela de roteamento atomica.

## 7.2 Update de funcao (hot swap)

1. Criar novo context `version = v+1`.
2. Marcar contexts antigos como `draining`.
3. Encerrar contexts antigos quando `active_requests = 0`.

## 7.3 Delete de funcao

- Remover contexts da funcao em todos isolates.
- Se isolate ficar vazio e pool acima de `min`, opcionalmente desalocar isolate.

---

## 8. Observabilidade e SLOs

## 8.1 Novas metricas obrigatorias

- `edge_isolate_pool_size`
- `edge_isolate_contexts{isolate_id}`
- `edge_context_active_requests{function,context_id}`
- `edge_context_cold_start_ms{function}`
- `edge_context_schedule_failures_total{reason}`
- `edge_context_spillover_total{function}`

## 8.2 Alertas recomendados

- Saturacao de contexto: `context_active_requests` alto sustentado.
- Saturacao de isolate: contexts no teto em todos isolates.
- Falhas de schedule por limite global.

---

## 9. Testes Necessarios

## 9.1 Unitarios

- scheduler escolhe context existente antes de criar isolate.
- ao atingir `max_contexts_per_isolate`, cria novo isolate quando permitido.
- respeita `global_max_isolates` e retorna erro deterministico ao exceder.

## 9.2 Integracao/E2E

- duas funcoes no mesmo isolate, contexts distintos, sem vazamento de handler/ALS.
- hot-reload de funcao com drain sem erro 5xx em requests concorrentes.
- caos: matar isolate com multiplos contexts e validar recuperacao de roteamento.

## 9.3 Performance

- benchmark de densidade:
  - modo legacy (1 funcao -> isolates)
  - modo novo (multi-context por isolate)
- medir:
  - p50/p95 latencia
  - memoria por funcao
  - cold start por contexto e por isolate

---

## 10. Compatibilidade e Riscos

## 10.1 Riscos tecnicos

- `main_context()` atual no dispatch pode causar invocacao do handler errado em multi-context.
- `__edgeRuntime.handler` global atual e singleton e precisa virar por-context.
- Pressao de GC/noisy-neighbor entre contexts no mesmo isolate.

## 10.2 Mitigacoes

- Feature flags + canary por funcao.
- Quotas por context e shedding previsivel.
- Draining e rollback para modo legacy.

---

## 11. Plano de atualizacao de documentacao existente

Atualizar os docs abaixo quando cada fase entrar em producao:

1. `ROADMAP.md`
- Incluir macro-etapas e status da trilha context+isolate.

2. `CURRENT_ARCHITECTURE_ANALYSIS.md`
- Atualizar diagramas de pool para refletir `contexts por isolate`.

3. `docs/guides/external-scaling-recommendations.md`
- Incluir sinais de saturacao por context (`contexts_per_isolate`, `context_queue_wait_ms_p95`).

4. `docs/design/timeout-and-resource-tracking.md`
- Explicar lifecycle por request dentro de context e efeito em multi-context.

5. `docs/reference/cli.md`
- Documentar novas flags de context pool e exemplos de tuning.

6. `docs/reference/NODE-COMPAT.md`
- Adicionar nota de isolamento: compat Node em ambiente multi-context dentro de isolate compartilhado.

7. `README.md`
- Atualizar secao de arquitetura e operacao (modo legacy vs modo context+isolate).

---

## 12. Checklist de entrega

- [x] Modelo de dados global de pool+context implementado no registry.
- [x] Dispatch por context implementado no handler/runtime.
- [x] Scheduler `context-first, isolate-next` funcional.
- [x] Feature flags e fallback legacy implementados.
- [ ] Metricas e alertas adicionados.
- [x] Suite de testes unitarios/integracao/perf adicionada.
- [x] Documentacao existente atualizada.

---

## 13. Exemplo de fluxo alvo (com codigo atual como base)

Cenario:

- `EDGE_RUNTIME_POOL_ENABLED=true`
- `EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=4`
- `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=8`

Fluxo esperado:

1. Deploy `func-a`:
- scheduler cria `isolate-1`, cria `context func-a@v1`.

2. Deploy `func-b`:
- scheduler detecta slot livre em `isolate-1`, cria `context func-b@v1` no mesmo isolate.

3. Crescimento de carga em `func-a`:
- scheduler cria contexts adicionais de `func-a` em `isolate-1` ate 8 contexts.

4. Limite atingido:
- proximo scale cria `isolate-2` e aloca novo context de `func-a` nele.

5. Limite global atingido (`4 isolates`):
- scheduler para de escalar isolates e aplica shedding (`503`) com metrica explicita.

Este fluxo aproveita o pool atual, mas muda a unidade de escala primariamente para `context`, usando `isolate` como proximo nivel de expansao.
