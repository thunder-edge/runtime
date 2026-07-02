Vou montar um plano de implantação realista de snapshots usando:

crates do Deno

engine V8 via rusty_v8

runtime edge próprio em Rust

A ideia é criar algo parecido com:

Deno Deploy

Cloudflare Workers

mas usando componentes do Deno como biblioteca.

1. Arquitetura geral

Arquitetura de alto nível:

deploy service
      │
      ▼
snapshot builder
      │
      ▼
snapshot storage
      │
      ▼
edge node
      │
      ▼
isolate pool
      │
      ▼
request scheduler

Componentes principais:

runtime core

snapshot builder

snapshot storage

edge executor

isolate pool

request scheduler

2. Dependências Rust

Crates principais:

[dependencies]
deno_core = "*"
deno_ast = "*"
deno_graph = "*"
deno_runtime = "*"
rusty_v8 = "*"
tokio = "*"
serde = "*"
anyhow = "*"

Papéis:

crate	função
rusty_v8	bindings diretos do V8
deno_core	runtime JS + ops
deno_runtime	APIs web
deno_ast	parsing/transpile
deno_graph	resolver módulos
3. Estrutura do projeto
edge-runtime/
 ├ runtime-core/
 ├ snapshot-builder/
 ├ isolate-pool/
 ├ edge-executor/
 ├ module-loader/
 ├ ops/
 └ api/
4. Snapshot Strategy

Usar dois níveis de snapshot.

Snapshot nível 1 — Runtime Base

Contém:

console
fetch
URL
crypto
streams
timers
TextEncoder
TextDecoder

Esse snapshot é criado uma vez no build do runtime.

Snapshot nível 2 — Worker

Cada deploy gera um snapshot contendo:

runtime base
+
user modules
+
compiled handler

Fluxo:

user deploy
     ↓
resolve modules
     ↓
compile
     ↓
execute bootstrap
     ↓
snapshot
5. Boot do runtime base

Criar um builder:

let mut creator = v8::SnapshotCreator::new(None);
let isolate = creator.get_owned_isolate();

Criar contexto:

let context = v8::Context::new(scope);
creator.set_default_context(context);

Inicializar runtime Deno:

let mut js_runtime = JsRuntime::new(RuntimeOptions {
    module_loader: Some(loader),
    extensions: vec![edge_extension()],
    ..Default::default()
});

Executar bootstrap:

init web APIs
init ops
init fetch
init streams

Criar snapshot:

let snapshot = creator.create_blob(
  v8::FunctionCodeHandling::Keep
);

Salvar snapshot:

runtime_base.bin
6. Snapshot Builder (user code)

Pipeline de deploy:

user uploads code
        │
        ▼
module graph build
        │
        ▼
transpile (ts/js)
        │
        ▼
bundle
        │
        ▼
execute runtime
        │
        ▼
create snapshot
7. Construção do graph de módulos

Usando deno_graph:

let mut graph = ModuleGraph::new(GraphKind::CodeOnly);

graph.build(
    vec![ModuleSpecifier::parse(entry)?],
    &loader,
    BuildOptions::default(),
).await;
8. Compilação

Usar deno_ast:

typescript
jsx
imports

Transformar para JS.

9. Executar bootstrap do worker

Executar dentro do runtime:

import worker from "./mod.ts"

globalThis.__worker_fetch = worker.fetch

Registrar handler.

10. Criar snapshot do worker
let blob = snapshot_creator.create_blob(
    v8::FunctionCodeHandling::Keep
);

Gerar arquivo:

worker_id.snapshot
11. Armazenamento

Snapshots são pequenos (~100KB–2MB).

Opções:

object storage

KV store

CDN

Exemplo estrutura:

snapshots/
   runtime_base.bin
   worker_abc123.bin
12. Inicialização do edge node

Quando o node inicia:

load runtime_base snapshot

Criar isolate pool.

13. Isolate Pool

Estrutura:

struct IsolatePool {
    isolates: Vec<Isolate>,
}

Inicialização:

N isolates por CPU

Exemplo:

CPU * 4
14. Criar isolate com snapshot
let mut params = v8::Isolate::create_params();
params.snapshot_blob = Some(snapshot_blob);

let isolate = v8::Isolate::new(params);
15. Carregar snapshot do worker

Ao receber request:

lookup worker snapshot

Criar isolate com:

runtime snapshot
+
worker snapshot

Ou usar pool dedicado por worker.

16. Execução do request

Fluxo:

request
   │
   ▼
scheduler
   │
   ▼
isolate
   │
   ▼
fetch handler
   │
   ▼
response

Execução:

handler.call(scope, global, &[request])
17. Reuso de isolates

Depois da execução:

clear context

ou:

destroy context
create new context

Retornar isolate ao pool.

18. Scheduler

Scheduler simples:

queue
↓
available isolate
↓
execute

Scheduler avançado:

priority
backpressure
timeouts
memory limits
19. Limites de execução

Implementar:

CPU time
memory
timeout

V8 permite:

TerminateExecution
20. Multi-tenant

Modelo:

worker isolate pools

ou

shared isolate + contexts

Mais seguro:

1 worker → 1 isolate pool
21. Atualização de snapshots

Deploy novo:

build snapshot
push storage
invalidate edge cache

Edge nodes:

lazy load snapshot
22. Observabilidade

Adicionar:

execution time
memory usage
errors
logs

Logs capturados via:

console API
23. Otimizações avançadas
Snapshot caching
keep snapshot in RAM
Lazy module evaluation

Carregar módulos sob demanda.

Isolate warmup

Executar:

dummy request

para JIT.

24. Pipeline final

Fluxo completo:

deploy
   │
   ▼
build graph
   │
   ▼
compile modules
   │
   ▼
execute bootstrap
   │
   ▼
create snapshot
   │
   ▼
store snapshot
   │
   ▼
edge node loads snapshot
   │
   ▼
isolate pool
   │
   ▼
requests
25. Resultado esperado

Cold start:

~1ms

Memória por isolate:

~2–5MB

Isolates por máquina:

5k–20k

como o runtime interno do Deno cria snapshots e como fazer hot reload de snapshots em edge nodes

Vou explicar como o runtime do Deno realmente cria snapshots e depois como implementar hot reload de snapshots em edge nodes (modelo parecido com Deno Deploy e Cloudflare Workers).

Vou focar no que acontece dentro do runtime, porque isso é o que você precisaria reproduzir num runtime edge baseado em deno_core + rusty_v8.

1. Como o Deno cria snapshots internamente

O Deno cria snapshots em duas fases principais.

Arquivos relevantes no projeto:

deno/runtime/snapshot.rs
deno/runtime/build.rs

A ideia é:

runtime boot
   ↓
executa JS bootstrap
   ↓
serializa heap do V8
   ↓
gera snapshot.bin

Esse snapshot vira parte do binário do Deno.

2. Criação do SnapshotCreator

No runtime builder:

use deno_core::snapshot::create_snapshot

Ou manualmente via rusty_v8:

let mut creator = v8::SnapshotCreator::new(None);
let isolate = creator.get_owned_isolate();

Esse isolate existe apenas para gerar o snapshot.

3. Inicialização do runtime JS

O Deno cria um JsRuntime com extensões.

Exemplo simplificado:

let mut runtime = JsRuntime::new(RuntimeOptions {
    extensions: vec![
        deno_web::init_ops(),
        deno_fetch::init_ops(),
        deno_console::init_ops(),
    ],
    startup_snapshot: None,
});

Essas extensões registram:

ops
web APIs
console
fetch
streams
4. Executar bootstrap JS

Depois o runtime executa scripts JS internos.

Arquivos reais:

runtime/js/99_main.js
runtime/js/01_web_util.js
runtime/js/02_console.js

Exemplo:

globalThis.console = new Console(...)
globalThis.fetch = fetch
globalThis.URL = URL

Isso prepara o ambiente.

5. Compilar módulos JS

Deno também carrega alguns módulos JS internos:

runtime/js/

Eles são executados dentro do isolate.

Isso gera:

bytecode compilado
objetos no heap
estado global
6. Criar o snapshot

Depois de todo bootstrap:

let snapshot = creator.create_blob(
    v8::FunctionCodeHandling::Keep
);

Isso serializa:

heap
bytecode
objects
functions
contexts

Resultado:

snapshot.bin
7. Embedding do snapshot no binário

O snapshot é embutido no binário via build.rs.

Fluxo:

cargo build
   ↓
build.rs executa snapshot builder
   ↓
snapshot.bin
   ↓
include_bytes!

Exemplo:

static SNAPSHOT: &[u8] = include_bytes!("snapshot.bin");
8. Como o runtime usa o snapshot

Quando o runtime inicia:

let params = v8::Isolate::create_params()
    .snapshot_blob(SNAPSHOT);

let isolate = v8::Isolate::new(params);

Agora o isolate já começa com:

console
fetch
streams
timers
ops

Sem bootstrap.

Cold start reduz de ~30ms → ~1ms.

9. Snapshots de user code

Para runtime edge você faria algo similar:

runtime snapshot
+
user snapshot

Pipeline:

deploy worker
   ↓
execute code
   ↓
snapshot
10. Hot reload de snapshots em edge nodes

Agora a parte interessante.

Edge nodes precisam trocar snapshots sem reiniciar o processo.

Arquitetura:

edge node
   ├ isolate pools
   ├ snapshot cache
   └ scheduler
11. Snapshot registry

Manter um registry:

struct SnapshotRegistry {
    snapshots: HashMap<WorkerId, Arc<Snapshot>>,
}

Snapshot:

struct Snapshot {
    data: Vec<u8>
}
12. Lazy loading

Quando chega um request:

lookup worker snapshot

Se não estiver em memória:

download snapshot
cache
13. Criar isolate com snapshot

Cada worker cria isolates assim:

let mut params = v8::Isolate::create_params();
params.snapshot_blob = Some(snapshot_blob);

let isolate = v8::Isolate::new(params);
14. Isolate pools por worker

Arquitetura típica:

worker A
   ├ isolate
   ├ isolate
   ├ isolate

worker B
   ├ isolate
   ├ isolate

Ou pool compartilhado.

15. Hot reload de snapshot

Quando ocorre deploy:

new snapshot version

Exemplo:

worker_abc_v2.snapshot

Atualizar registry:

registry.insert(worker_id, new_snapshot);
16. Atualização sem matar requests

Estratégia:

old isolates continuam
new isolates usam snapshot novo

Fluxo:

request
  ↓
scheduler
  ↓
pick isolate

Scheduler evita isolates antigos.

17. Drain de isolates antigos

Depois do deploy:

mark isolates as draining

Eles continuam executando requests existentes.

Mas não recebem novos.

18. Destruição segura

Quando requests acabam:

drop isolate

Memória liberada.

19. Estrutura completa

Edge node:

edge runtime
   ├ snapshot registry
   ├ isolate pools
   ├ scheduler
   ├ request router
20. Atualização distribuída

Edge nodes geralmente usam:

pub/sub

ou

event stream

Exemplo:

deploy event

Nodes recebem:

worker update
21. Fluxo completo
deploy worker
     ↓
build snapshot
     ↓
store snapshot
     ↓
notify edge nodes
     ↓
edge loads snapshot
     ↓
new isolates
     ↓
old isolates drained
22. Otimização crítica

Manter snapshots em memória.

Snapshot médio:

200KB – 2MB

Carregamento:

<1ms
23. Pré-warm

Após hot reload:

create N isolates

Exemplo:

CPU * 2

Isso evita cold start no primeiro request.

24. Resultado

Tempo típico:

operação	tempo
carregar snapshot	~0.3ms
criar isolate	~1ms
executar handler	<1ms

💡 Se você estiver realmente pensando em construir um runtime edge baseado em Deno, existe uma técnica ainda mais poderosa que snapshots:

multi-tenant isolates (modelo do Cloudflare Workers)

onde:

1 isolate
→ milhares de workers

Se quiser, posso te mostrar como essa arquitetura funciona internamente, porque ela muda completamente o design do runtime.

 como rodar milhares de workers dentro do mesmo isolate (modelo Cloudflare)

Rodar milhares de workers no mesmo isolate (modelo usado por Cloudflare Workers) é uma das arquiteturas mais interessantes possíveis com V8. A ideia central é: um isolate único, múltiplos contexts independentes, cada um representando um worker.

Vou explicar como funciona internamente e como implementar usando deno_core + rusty_v8.

1. Conceito fundamental

No V8 existem três níveis importantes:

Process
  └── Isolate
        └── Context
nível	função
process	runtime host
isolate	heap e GC
context	ambiente global JS

A arquitetura Cloudflare usa:

1 isolate
   ├ context worker A
   ├ context worker B
   ├ context worker C
   └ context worker N

Cada worker = 1 context.

2. Por que isso escala tanto

Criar isolate:

~1–5 MB memória

Criar context:

~50–200 KB

Então em vez de:

1 worker = 1 isolate

você faz:

1000 workers = 1 isolate

Isso permite rodar milhares de workers por processo.

3. Arquitetura geral

Runtime edge:

edge runtime
   ├ isolate
   ├ context registry
   ├ module registry
   └ request scheduler

Estruturas principais:

struct Worker {
    context: v8::Global<v8::Context>,
    handler: v8::Global<v8::Function>,
}

Registry:

HashMap<WorkerId, Worker>
4. Snapshot apenas do runtime

Você cria snapshot apenas com runtime base:

console
fetch
streams
crypto
timers
URL

Isso gera:

runtime_snapshot.bin

Quando o runtime inicia:

create isolate from snapshot
5. Criar workers dinamicamente

Quando um worker é deployado:

create context
execute user code
extract handler
store references

Fluxo:

deploy worker
     ↓
compile modules
     ↓
create context
     ↓
execute module
     ↓
register handler
6. Criar context no isolate

Código simplificado:

let context = v8::Context::new(scope);

Registrar global:

let global = context.global(scope);
7. Executar código do worker

Exemplo de worker:

export default {
  async fetch(req) {
    return new Response("hello")
  }
}

Executar script:

let script = v8::Script::compile(scope, source, None)?;
script.run(scope)?;
8. Extrair handler

Depois pegar o handler:

let handler = global.get(scope, fetch_key)?;

Guardar referência:

v8::Global<v8::Function>

Isso evita GC.

9. Estrutura de registro

Registry:

struct WorkerRegistry {
    workers: HashMap<String, Worker>
}

Worker:

struct Worker {
    context: v8::Global<v8::Context>,
    handler: v8::Global<v8::Function>,
}
10. Scheduler de requests

Quando chega um request:

HTTP request
      ↓
router
      ↓
lookup worker
      ↓
execute handler

Fluxo:

request
   ↓
worker lookup
   ↓
enter context
   ↓
call handler
11. Entrar no context

Antes de executar:

let context = worker.context.open(scope);
let _scope = v8::ContextScope::new(scope, context);

Isso ativa o ambiente do worker.

12. Executar handler
handler.call(scope, global, &[request]);

Isso roda dentro do context isolado.

13. Concurrency

Mesmo isolate pode servir vários requests simultaneamente porque JS é async.

Exemplo:

request A -> await fetch
request B -> crypto
request C -> DB

Todos coexistem no event loop.

14. Event loop

O runtime mantém um loop:

while pending_tasks {
    poll promises
}

Isso é exatamente o que o runtime de Deno já faz.

15. Limites de memória

Cada context precisa de limites.

Você pode medir:

isolate heap usage

ou controlar:

worker memory quotas

Se exceder:

terminate worker
16. Remover worker

Para remover worker:

drop context
drop handler

Quando Global<> é liberado:

V8 GC libera heap
17. Hot reload

Deploy novo:

create new context
register worker
mark old context draining

Depois:

drop old context
18. Distribuição típica

Em um servidor:

1 isolate
    ├ 5000 contexts

Memória:

~1–2 GB

Requests simultâneos:

50k+
19. Otimização importante

Cloudflare não recompila código a cada context.

Eles usam:

compiled module cache

Fluxo:

compile module once
instantiate in contexts

Isso é crucial.

20. Pipeline completo
edge node start
      ↓
load runtime snapshot
      ↓
create isolate
      ↓
deploy worker
      ↓
create context
      ↓
execute module
      ↓
register handler
      ↓
requests
21. Estrutura final do runtime
edge runtime
   ├ V8 isolate
   ├ worker registry
   ├ module cache
   ├ request scheduler
   └ event loop
22. Resultado

Cold start worker:

~1–3 ms

Workers por isolate:

5k–20k

Requests por segundo:

100k+

✅ Resumo da arquitetura

recurso	estratégia
runtime	snapshot
workers	contexts
handler	cached functions
execução	async event loop
deploy	hot context swap

💡 Se você estiver pensando em construir um runtime edge real, existe uma técnica ainda mais avançada usada por Cloudflare Workers que quase ninguém conhece:

“Isolate sharding + module instancing”

Ela permite algo como:

1 máquina
→ 100k workers
→ milhões de requests

Se quiser, posso explicar essa arquitetura também — ela muda completamente como você projeta o runtime.

cada request é um context?

Não. Cada request não é um context.
No modelo usado por plataformas como Cloudflare Workers, o padrão é:

1 worker = 1 context
muitos requests = executados no mesmo context

Ou seja:

Isolate
 ├── Context (worker A)
 │     ├ request 1
 │     ├ request 2
 │     └ request 3
 ├── Context (worker B)
 │     ├ request 1
 │     └ request 2
 └── Context (worker C)
Por que não criar um context por request

Criar context é relativamente barato, mas ainda custa:

operação	custo aproximado
criar context	~0.1–0.5 ms
memória	~50–200 KB

Se você fizer:

request → novo context

com 50k req/s você teria:

50k context/s

Isso gera:

pressão enorme no GC

fragmentação de heap

overhead desnecessário

Por isso runtimes edge reutilizam o context do worker.

Como funciona na prática

Quando um worker é carregado:

deploy worker
   ↓
create context
   ↓
execute module
   ↓
store handler

Depois disso o context permanece vivo.

Execução de cada request

Quando chega um request:

HTTP request
     ↓
lookup worker
     ↓
enter context
     ↓
call fetch handler

Pseudo-código em Rust (usando rusty_v8):

let context = worker.context.open(scope);
let scope = &mut v8::ContextScope::new(scope, context);

handler.call(scope, global, &[request]);

O request apenas entra no context existente.

Concurrency dentro do mesmo context

JavaScript é event loop async.

Então vários requests podem coexistir:

request A -> await fetch(db)
request B -> crypto
request C -> await api

Todos no mesmo context.

Modelo real do runtime

Estrutura típica:

Isolate
   ├ Context Worker A
   │     ├ Promise request 1
   │     ├ Promise request 2
   │     └ Promise request 3
   │
   ├ Context Worker B
   │     └ Promise request 1
   │
   └ Context Worker C

Promises pendentes representam requests ativos.

Quando criar novos contexts

Normalmente só em três casos:

1️⃣ Deploy
novo worker
→ novo context
2️⃣ Hot reload
worker v1 context
worker v2 context
3️⃣ Isolamento extra (opcional)

Alguns runtimes criam context temporário por request para segurança extrema.

Mas isso não é o padrão.

Como evitar vazamento de estado entre requests

Como os requests compartilham context, o runtime depende do fato de que:

handler é stateless

Exemplo correto:

export default {
  async fetch(req) {
    return new Response("ok")
  }
}

Evitar:

let counter = 0

export default {
  async fetch() {
    counter++
    return new Response(counter)
  }
}

Isso criaria estado global compartilhado.

Runtimes geralmente permitem, mas documentam isso.

Como Cloudflare lida com abuso

Eles usam:

CPU limits

memory limits

execution time

isolate termination

Tudo via V8 APIs.

Resumo
conceito	modelo
worker	1 context
request	execução dentro do context
isolate	contém muitos contexts
concurrency	promises/event loop

📌 Regra mental simples:

Isolate = processo JS
Context = sandbox do worker
Request = chamada de função