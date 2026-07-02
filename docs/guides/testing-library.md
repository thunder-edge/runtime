# Testing Library for Edge Runtime

Biblioteca de testes completa para ambientes edge/serverless (Supabase Edge Runtime, Deno-like runtimes, Cloudflare Workers, runtimes compatíveis com browser).

Princípios de design:

- Sem dependências de Node.js built-ins
- Sem dependências externas
- Footprint leve em runtime
- API amigável para TypeScript
- Suporte a mocks (funções, fetch, timers)
- Snapshot testing integrado
- Test runner com suites, hooks, concorrência e retry

## Importação

Todas as APIs são importadas de um único módulo:

```ts
import {
  assert,
  assertEquals,
  mockFn,
  spyOn,
  mockFetch,
  mockTime,
  runSuite,
  test,
  // ... demais exports
} from "thunder:testing";
```

Observação: o alias `thunder:testing` é resolvido pelo CLI em fluxos locais (`thunder watch`, `thunder bundle`, `thunder test` e `thunder check`).

---

## 1. Assertions

### Assertions básicas

| Função | Descrição |
|--------|-----------|
| `assert(condition, message?)` | Falha se `condition` é falsy |
| `assertEquals(actual, expected, message?)` | Igualdade profunda (deep equality) |
| `assertNotEquals(actual, expected, message?)` | Falha se são iguais (deep) |
| `assertStrictEquals(actual, expected, message?)` | Igualdade estrita via `Object.is()` |
| `assertNotStrictEquals(actual, expected, message?)` | Falha se são estritamente iguais |
| `assertExists(value, message?)` | Falha se `null` ou `undefined` |
| `assertInstanceOf(value, Type, message?)` | Verifica `instanceof` |
| `assertType<T>(value)` | Helper de compile-time (no-op em runtime) |

```ts
import {
  assert,
  assertEquals,
  assertStrictEquals,
  assertExists,
  assertInstanceOf,
  assertType,
} from "thunder:testing";

assert(true);
assertEquals(1 + 1, 2);
assertEquals([1, 2], [1, 2]);           // deep equality
assertEquals({ a: 1 }, { a: 1 });       // deep equality em objetos
assertStrictEquals(NaN, NaN);            // true (usa Object.is)
assertExists("hello");                   // ok
assertInstanceOf(new Error(), Error);    // ok
assertType<number>(42);                  // compile-time check
```

#### Deep equality — tipos suportados

`assertEquals` / `assertNotEquals` comparam profundamente:

- Primitivos
- Arrays e arrays aninhados
- Typed arrays (`Uint8Array`, `Float32Array`, etc.)
- Objetos simples (plain objects)
- `Date` (compara `.getTime()`)
- `RegExp` (compara `.toString()`)
- `Set` (compara elementos com deep equality)
- `Map` (compara chave+valor com deep equality)

Em caso de falha, `assertEquals` gera um diff legível:

```
Values are not equal

- "expected line"
+ "actual line"

Expected: { "a": 1 }
Actual:   { "a": 2 }
```

### Assertions de padrão e coleção

```ts
import {
  assertMatch,
  assertArrayIncludes,
  assertObjectMatch,
} from "thunder:testing";

assertMatch("hello world", /world/);

assertArrayIncludes([1, 2, 3, 4], [2, 4]);

assertObjectMatch(
  { id: 1, name: "Celso", role: "admin" },
  { name: "Celso" },   // subset — não exige todas as chaves
);
```

### Assertions de exceção

```ts
import { assertThrows, assertRejects } from "thunder:testing";

// Verifica que a função lança qualquer erro
const err = assertThrows(() => {
  throw new Error("boom");
});

// Verifica tipo específico do erro
assertThrows(() => {
  throw new TypeError("bad type");
}, TypeError);

// Versão async — verifica rejeição de Promise
const rejectedErr = await assertRejects(async () => {
  throw new Error("async boom");
});

// Verifica tipo específico na rejeição
await assertRejects(async () => {
  throw new RangeError("out of range");
}, RangeError);
```

Ambas retornam o `Error` capturado para inspeção adicional.

### AssertionError

Todas as assertions lançam `AssertionError` em caso de falha:

```ts
import { AssertionError } from "thunder:testing";

try {
  assertEquals(1, 2);
} catch (err) {
  console.log(err instanceof AssertionError); // true
  console.log(err.name);    // "AssertionError"
  console.log(err.message); // diff detalhado
}
```

---

## 2. Escrevendo Testes

Use `test(...)` para criar casos de teste e `runSuite(...)` para executá-los:

```ts
import { runSuite, test, assertEquals } from "thunder:testing";

await runSuite("math", [
  test("soma funciona", () => {
    assertEquals(1 + 1, 2);
  }),

  test("multiplicação funciona", () => {
    assertEquals(3 * 4, 12);
  }),
]);
```

O runner exibe no console:

```
suite: math
soma funciona... OK
multiplicação funciona... OK
suite done: 2/2 (ignored: 0, failed: 0)
```

---

## 3. Test Suites

### Criando suites

| Função | Descrição |
|--------|-----------|
| `suite(name, entries)` | Cria uma suite |
| `suiteIgnore(name, entries)` | Cria uma suite ignorada |
| `suiteOnly(name, entries)` | Cria uma suite com foco (só ela roda) |

### Criando testes

| Função | Descrição |
|--------|-----------|
| `test(name, fn, options?)` | Cria um caso de teste |
| `testIgnore(name, fn, options?)` | Cria um teste ignorado |
| `testOnly(name, fn, options?)` | Cria um teste com foco |
| `testIf(condition)` | Retorna factory de teste condicional |
| `testEach(rows)` | Retorna factory para testes parametrizados |

### TestOptions

```ts
type TestOptions = {
  ignore?: boolean;     // Pula o teste
  only?: boolean;       // Executa apenas este teste na suite
  timeout?: number;     // Timeout em ms
  concurrent?: boolean; // Permite execução paralela
  retry?: number;       // Número de retentativas em caso de falha
};
```

### Executando suites

```ts
import {
  runSuite,
  runSuites,
  suite,
  suiteIgnore,
  suiteOnly,
  test,
  testIgnore,
  testOnly,
  assertEquals,
} from "thunder:testing";

// Executar uma suite diretamente
await runSuite("example", [
  test("funciona", () => assertEquals(2 * 3, 6)),
  testIgnore("pular este", () => assertEquals(1, 2)),
]);

// Executar múltiplas suites
await runSuites([
  suite("math", [
    testOnly("multiplicação", () => assertEquals(3 * 3, 9)),
  ]),
  suiteIgnore("integration", [
    test("pesado", () => assertEquals(1, 1)),
  ]),
]);
```

Comportamento de `only`:

- Se algum **teste** tem `only: true`, apenas esses testes rodam na suite
- Se alguma **suite** tem `only: true`, apenas essas suites rodam em `runSuites`

### Testes condicionais com `testIf`

```ts
import { runSuite, testIf, assert } from "thunder:testing";

const isLinux = Deno.build.os === "linux";

await runSuite("platform", [
  testIf(isLinux)("roda apenas no linux", () => {
    assert(true);
  }),
]);
```

### Testes parametrizados com `testEach`

```ts
import { runSuite, testEach, assertEquals } from "thunder:testing";

await runSuite("parametrized", [
  ...testEach([
    [1, 2, 3],
    [4, 5, 9],
    [10, 20, 30],
  ] as const)("soma", (a, b, expected) => {
    assertEquals(a + b, expected);
  }),
]);
```

Cada linha gera um teste com nome como `soma [0] [1,2,3]`.

### Timeout e Retry

```ts
import { runSuite, test, assert } from "thunder:testing";

await runSuite("resilience", [
  // Falha se demorar mais de 5 segundos
  test("com timeout", async () => {
    const res = await fetch("https://api.example.com/health");
    assert(res.ok);
  }, { timeout: 5000 }),

  // Retenta até 3 vezes em caso de falha
  test("flaky test", async () => {
    const res = await fetch("https://api.example.com/data");
    assert(res.ok);
  }, { retry: 3 }),
]);
```

---

## 4. Lifecycle Hooks

| Hook | Quando executa |
|------|---------------|
| `beforeAll(fn)` | Uma vez antes de todos os testes da suite |
| `afterAll(fn)` | Uma vez depois de todos os testes da suite |
| `beforeEach(fn)` | Antes de cada teste individual |
| `afterEach(fn)` | Depois de cada teste individual |

Hooks são declarados inline na lista de entries da suite:

```ts
import {
  runSuite,
  beforeAll,
  beforeEach,
  afterEach,
  afterAll,
  test,
  assert,
} from "thunder:testing";

let db: Database;

await runSuite("users", [
  beforeAll(async () => {
    db = await connectToTestDatabase();
  }),

  beforeEach(() => {
    // limpa estado antes de cada teste
  }),

  test("criar usuário", async () => {
    const user = await db.createUser({ name: "Celso" });
    assert(user.id > 0);
  }),

  test("deletar usuário", async () => {
    await db.deleteUser(1);
    assert(true);
  }),

  afterEach(() => {
    // limpeza após cada teste
  }),

  afterAll(async () => {
    await db.close();
  }),
]);
```

Hooks suportam funções síncronas e assíncronas. Se `beforeAll` falhar, a suite inteira é marcada como falha.

---

## 5. Mocking

A biblioteca oferece quatro mecanismos de mock:

| Ferramenta | Uso |
|------------|-----|
| `mockFn(impl?)` | Cria uma função mock |
| `spyOn(obj, method)` | Espia um método existente |
| `mockFetch(routes)` | Mock de `fetch` via mapa de rotas |
| `mockFetchHandler(handler)` | Mock de `fetch` com handler dinâmico |
| `mockTime()` | Mock de timers (`setTimeout`, `setInterval`) |

### 5.1 mockFn — Funções mock

Cria uma função com rastreamento de chamadas:

```ts
import { mockFn, assertEquals, assertSpyCalls } from "thunder:testing";

// Mock com implementação
const add = mockFn((a: number, b: number) => a + b);
const result = add(1, 2);

assertEquals(result, 3);
assertEquals(add.calls.length, 1);
assertEquals(add.calls[0].args, [1, 2]);
assertEquals(add.calls[0].result, 3);

// Mock sem implementação (retorna undefined)
const noop = mockFn();
noop("hello");
assertEquals(noop.calls[0].args, ["hello"]);
assertEquals(noop.calls[0].result, undefined);
```

#### Propriedades e métodos de Mock

```ts
type Mock<T> = T & {
  calls: MockCall[];          // histórico de chamadas
  mockClear: () => void;      // limpa o histórico
  mockImplementation: (fn) => void; // troca a implementação
};

type MockCall = {
  args: unknown[];   // argumentos da chamada
  result?: unknown;  // valor retornado (se sucesso)
  error?: unknown;   // erro lançado (se falhou)
};
```

#### Trocando implementação

```ts
const fn = mockFn(() => "original");
assertEquals(fn(), "original");

fn.mockImplementation(() => "novo");
assertEquals(fn(), "novo");
```

#### Limpando histórico

```ts
const fn = mockFn((x: number) => x * 2);
fn(5);
fn(10);
assertEquals(fn.calls.length, 2);

fn.mockClear();
assertEquals(fn.calls.length, 0);
```

#### Mocks assíncronos

`mockFn` rastreia automaticamente promises:

```ts
const fetchUser = mockFn(async (id: number) => {
  return { id, name: "User " + id };
});

const user = await fetchUser(42);
assertEquals(user, { id: 42, name: "User 42" });
assertEquals(fetchUser.calls[0].result, { id: 42, name: "User 42" });
```

Se a promise rejeitar, `call.error` é preenchido:

```ts
const failing = mockFn(async () => {
  throw new Error("falhou");
});

try {
  await failing();
} catch {}

assertEquals(failing.calls[0].error instanceof Error, true);
```

### 5.2 spyOn — Espiar métodos existentes

Substitui um método de um objeto por um spy que rastreia chamadas e delega ao método original:

```ts
import { spyOn, assertEquals } from "thunder:testing";

const spy = spyOn(console, "log");

console.log("hello", "world");

assertEquals(spy.calls.length, 1);
assertEquals(spy.calls[0].args, ["hello", "world"]);

// IMPORTANTE: sempre restaurar no final
spy.restore();
```

O spy herda todas as propriedades de `Mock` (`calls`, `mockClear`, `mockImplementation`) e adiciona:

- `restore()` — restaura o método original no objeto

```ts
import { spyOn, assertEquals, assertSpyCalls } from "thunder:testing";

const obj = {
  greet(name: string) {
    return `Hello, ${name}!`;
  },
};

const spy = spyOn(obj, "greet");

try {
  const result = obj.greet("Celso");
  assertEquals(result, "Hello, Celso!");  // método original executa
  assertSpyCalls(spy, 1);
} finally {
  spy.restore();
}
```

### 5.3 Assertions de spy/mock

| Função | Descrição |
|--------|-----------|
| `assertSpyCalls(spy, count, message?)` | Verifica o número total de chamadas |
| `assertSpyCall(spy, index, expected?)` | Verifica uma chamada específica |

```ts
import {
  mockFn,
  assertSpyCalls,
  assertSpyCall,
} from "thunder:testing";

const fn = mockFn((a: number, b: number) => a + b);
fn(1, 2);
fn(3, 4);

assertSpyCalls(fn, 2);

assertSpyCall(fn, 0, {
  args: [1, 2],
  result: 3,
});

assertSpyCall(fn, 1, {
  args: [3, 4],
  result: 7,
});
```

O objeto `expected` de `assertSpyCall` é parcial — você pode verificar apenas `args`, apenas `result`, apenas `error`, ou qualquer combinação.

---

## 6. Mock de Fetch (HTTP Mocking)

### 6.1 mockFetch — Mapa de rotas

O jeito mais simples de mockar `fetch`. Mapeia URLs exatas para respostas:

```ts
import { mockFetch, assertEquals } from "thunder:testing";

const mock = mockFetch({
  "https://api.example.com/users": {
    status: 200,
    body: [{ id: 1, name: "Celso" }],
    headers: { "x-total": "1" },
  },
  "https://api.example.com/health": {
    status: 204,
  },
});

try {
  const res = await fetch("https://api.example.com/users");
  assertEquals(res.status, 200);
  assertEquals(await res.json(), [{ id: 1, name: "Celso" }]);
  assertEquals(res.headers.get("x-total"), "1");
  assertEquals(res.headers.get("content-type"), "application/json");

  const health = await fetch("https://api.example.com/health");
  assertEquals(health.status, 204);

  // URL não mapeada retorna 404
  const notFound = await fetch("https://api.example.com/other");
  assertEquals(notFound.status, 404);
} finally {
  mock.restore();
}
```

#### MockFetchResponse

```ts
type MockFetchResponse = {
  status?: number;                    // default: 200
  body?: unknown;                     // string, Blob, FormData, JSON object, etc.
  headers?: Record<string, string>;   // headers adicionais
};
```

**Comportamento do body:**
- Se `body` é `undefined`: resposta sem body
- Se `body` é `BodyInit` (string, Blob, FormData, etc.): usado diretamente
- Qualquer outro valor: serializado como JSON, com `content-type: application/json` automático

#### Verificando chamadas ao fetch

`mockFetch` e `mockFetchHandler` retornam um `MockFetchController` com histórico:

```ts
const mock = mockFetch({
  "https://api.example.com/data": { body: { ok: true } },
});

try {
  await fetch("https://api.example.com/data", {
    method: "POST",
    body: JSON.stringify({ key: "value" }),
  });

  assertEquals(mock.calls.length, 1);
  assertEquals(mock.calls[0].args[0] instanceof Request, true);

  const request = mock.calls[0].args[0] as Request;
  assertEquals(request.method, "POST");
  assertEquals(request.url, "https://api.example.com/data");
} finally {
  mock.restore();
}
```

### 6.2 mockFetchHandler — Handler dinâmico

Para cenários mais complexos, use um handler function que recebe o `Request` e retorna um `Response`:

```ts
import { mockFetchHandler, assertEquals } from "thunder:testing";

const mock = mockFetchHandler((request) => {
  const url = new URL(request.url);

  if (url.pathname === "/users" && request.method === "GET") {
    return new Response(JSON.stringify([{ id: 1 }]), {
      headers: { "content-type": "application/json" },
    });
  }

  if (url.pathname === "/users" && request.method === "POST") {
    return new Response(JSON.stringify({ id: 2 }), {
      status: 201,
      headers: { "content-type": "application/json" },
    });
  }

  // Retornar null ou undefined gera Response com status 501
  return null;
});

try {
  const getRes = await fetch("https://api.test/users");
  assertEquals(getRes.status, 200);
  assertEquals(await getRes.json(), [{ id: 1 }]);

  const postRes = await fetch("https://api.test/users", { method: "POST" });
  assertEquals(postRes.status, 201);
} finally {
  mock.restore();
}
```

O handler pode ser assíncrono:

```ts
const mock = mockFetchHandler(async (request) => {
  const body = await request.json();
  return new Response(JSON.stringify({ echo: body }), {
    headers: { "content-type": "application/json" },
  });
});
```

---

## 7. Fake Timers (mockTime)

`mockTime()` substitui `setTimeout`, `clearTimeout`, `setInterval` e `clearInterval` por versões controladas:

```ts
import { mockTime, assert, assertEquals } from "thunder:testing";

const clock = mockTime();

try {
  let called = false;
  setTimeout(() => {
    called = true;
  }, 1000);

  assert(!called);     // ainda não avançou
  clock.tick(999);
  assert(!called);     // 999ms não é suficiente
  clock.tick(1);
  assert(called);      // agora sim, 1000ms
} finally {
  clock.restore();
}
```

### MockClock API

```ts
type MockClock = {
  now: () => number;       // retorna o tempo atual do clock
  tick: (ms: number) => void; // avança o tempo e executa timers devidos
  restore: () => void;     // restaura as funções originais
};
```

### Testando setInterval

```ts
const clock = mockTime();

try {
  const calls: number[] = [];
  const id = setInterval(() => {
    calls.push(clock.now());
  }, 100);

  clock.tick(350);
  assertEquals(calls.length, 3);  // executou em 100, 200, 300

  clearInterval(id);
  clock.tick(200);
  assertEquals(calls.length, 3);  // não executou mais
} finally {
  clock.restore();
}
```

### Combinando com testes

```ts
import {
  runSuite,
  test,
  beforeEach,
  afterEach,
  mockTime,
  assert,
  type MockClock,
} from "thunder:testing";

let clock: MockClock;

await runSuite("timers", [
  beforeEach(() => {
    clock = mockTime();
  }),

  afterEach(() => {
    clock.restore();
  }),

  test("debounce espera o tempo correto", () => {
    let fired = false;
    setTimeout(() => { fired = true; }, 300);

    clock.tick(299);
    assert(!fired);
    clock.tick(1);
    assert(fired);
  }),
]);
```

---

## 8. Snapshot Testing

`assertSnapshot` salva o valor serializado em um arquivo `.snap` e compara em execuções futuras:

```ts
import { assertSnapshot } from "thunder:testing";

const user = { id: 1, name: "Celso", role: "admin" };
assertSnapshot(user);
```

### Comportamento padrão

- Snapshots são armazenados em `__snapshots__/` relativo ao arquivo de teste
- Nome do arquivo: `<nome-do-teste-sem-extensão>.snap`
- Chave do snapshot: nome do teste atual (do runner)
- Formato: JSON

### SnapshotOptions

```ts
type SnapshotOptions = {
  name?: string;       // nome customizado para a chave do snapshot
  filePath?: string;   // caminho do arquivo de teste (auto-detectado normalmente)
  update?: boolean;    // se true, atualiza o snapshot em vez de comparar
};
```

### Exemplos

```ts
import { runSuite, test, assertSnapshot } from "thunder:testing";

await runSuite("snapshot", [
  test("user schema", () => {
    const data = { id: 1, name: "Celso" };
    assertSnapshot(data);
    // Salva em __snapshots__/<test-file>.snap com chave "user schema"
  }),

  test("custom name", () => {
    assertSnapshot({ foo: "bar" }, { name: "my-custom-snapshot" });
  }),

  test("atualizar snapshot", () => {
    assertSnapshot({ updated: true }, { update: true });
    // Sempre sobrescreve o snapshot existente
  }),
]);
```

Na primeira execução, o snapshot é criado. Nas execuções seguintes, o valor é comparado com o snapshot salvo. Se houver diferença, um diff é exibido:

```
Snapshot mismatch: 'user schema'

- "expected value"
+ "actual value"

Snapshot file: /path/__snapshots__/test-file.snap
```

---

## 9. Testes Concorrentes

Testes marcados com `{ concurrent: true }` rodam em paralelo via `Promise.all`:

```ts
import { runSuite, test, assert } from "thunder:testing";

await runSuite("concurrent", [
  test("request A", async () => {
    await new Promise((r) => setTimeout(r, 100));
    assert(true);
  }, { concurrent: true }),

  test("request B", async () => {
    await new Promise((r) => setTimeout(r, 100));
    assert(true);
  }, { concurrent: true }),

  // Testes sem concurrent rodam sequencialmente antes dos concorrentes
  test("sequencial", () => {
    assert(true);
  }),
]);
```

Testes sequenciais executam primeiro na ordem declarada, seguidos pelos concorrentes em paralelo.

---

## 10. Estatísticas do Runner

Após executar suites, você pode consultar estatísticas acumuladas:

```ts
import { getTestRunnerStats } from "thunder:testing";

const stats = getTestRunnerStats();
console.log(stats);
// {
//   suitesTotal: 3,
//   suitesPassed: 2,
//   suitesFailed: 1,
//   suitesIgnored: 0,
//   testsTotal: 10,
//   testsPassed: 8,
//   testsFailed: 1,
//   testsIgnored: 1,
// }
```

---

## 11. Exemplo Completo

```ts
import {
  runSuite,
  test,
  testIgnore,
  beforeAll,
  afterAll,
  beforeEach,
  afterEach,
  assert,
  assertEquals,
  assertThrows,
  assertRejects,
  assertSpyCalls,
  assertSpyCall,
  mockFn,
  spyOn,
  mockFetch,
  mockTime,
  assertSnapshot,
} from "thunder:testing";

let clock;

await runSuite("complete example", [
  beforeAll(() => {
    clock = mockTime();
  }),

  afterAll(() => {
    clock.restore();
  }),

  test("assertions básicas", () => {
    assert(true);
    assertEquals([1, 2, 3], [1, 2, 3]);
    assertThrows(() => { throw new Error("boom"); });
  }),

  test("mock de função", () => {
    const fn = mockFn((x: number) => x * 2);
    fn(5);
    fn(10);

    assertSpyCalls(fn, 2);
    assertSpyCall(fn, 0, { args: [5], result: 10 });
    assertSpyCall(fn, 1, { args: [10], result: 20 });
  }),

  test("spy em método", () => {
    const spy = spyOn(console, "warn");
    try {
      console.warn("atenção!");
      assertSpyCalls(spy, 1);
      assertSpyCall(spy, 0, { args: ["atenção!"] });
    } finally {
      spy.restore();
    }
  }),

  test("mock de fetch", async () => {
    const mock = mockFetch({
      "https://api.test/data": {
        status: 200,
        body: { items: [1, 2, 3] },
      },
    });

    try {
      const res = await fetch("https://api.test/data");
      assertEquals(await res.json(), { items: [1, 2, 3] });
    } finally {
      mock.restore();
    }
  }),

  test("fake timers", () => {
    let count = 0;
    setInterval(() => { count++; }, 100);

    clock.tick(350);
    assertEquals(count, 3);
  }),

  test("snapshot", () => {
    assertSnapshot({ version: 1, data: "test" });
  }),

  testIgnore("teste desabilitado", () => {
    // não executa
  }),
]);

## 10. Table-driven Tests

Use `testEach(rows)`.

```ts
import { runSuite, testEach, assertEquals } from "thunder:testing";

await runSuite("sum", [
  ...testEach([
    [1, 2, 3] as const,
    [2, 3, 5] as const,
  ])("sum test", (a, b, result) => {
    assertEquals(a + b, result);
  }),
]);
```

## 11. Conditional Tests

Use `testIf(condition)`.

```ts
import { runSuite, testIf, assert } from "thunder:testing";

const featureEnabled = typeof Deno === "object";

await runSuite("feature-gated", [
  testIf(featureEnabled)("feature test", () => {
    assert(true);
  }),
]);
```

If the condition is false, the test is skipped.

## Timeouts and Retries

`timeout` fails tests that run too long.

`retry` retries flaky tests before final failure.

```ts
import { runSuite, test } from "thunder:testing";

let attempt = 0;

await runSuite("resilience", [
  test("slow request", async () => {
    await new Promise((r) => setTimeout(r, 10));
  }, { timeout: 1000 }),

  test("flaky test", () => {
    attempt += 1;
    if (attempt < 3) {
      throw new Error("flaky");
    }
  }, { retry: 3 }),
]);
```

## Running Tests in This Repository

```bash
cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**"
```

```bash
make test-js
```
