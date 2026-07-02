# Testing API Reference

Referência completa da API de testes do Edge Runtime.

## Importação

```ts
import * as test from "edge://assert/mod.ts";
```

Ou importação seletiva:

```ts
import { assertEquals, assertThrows, runSuite, mockFn } from "edge://assert/mod.ts";
```

---

## Tipos

### `AssertionError`

```ts
class AssertionError extends Error
```

Lançada por todas as assertions em caso de falha. Contém mensagem descritiva com diff quando aplicável.

### `TestCase`

```ts
type TestCase = {
  kind: "test";
  name: string;
  run: () => void | Promise<void>;
  ignore?: boolean;
  only?: boolean;
  timeout?: number;
  concurrent?: boolean;
  retry?: number;
};
```

### `TestOptions`

```ts
type TestOptions = {
  ignore?: boolean;      // pula o teste
  only?: boolean;        // executa apenas este teste
  timeout?: number;      // timeout em ms
  concurrent?: boolean;  // execução paralela
  retry?: number;        // retentativas em caso de falha
};
```

### `TestSuite`

```ts
type TestSuite = {
  name: string;
  tests: SuiteEntry[];
  ignore?: boolean;
  only?: boolean;
};
```

### `SuiteOptions`

```ts
type SuiteOptions = {
  ignore?: boolean;
  only?: boolean;
};
```

### `SuiteEntry`

```ts
type SuiteEntry = TestCase | HookCase | LegacyTestCase | SuiteEntry[];
```

Entries podem ser aninhadas em arrays para composição.

### `HookCase`

```ts
type HookCase = {
  kind: "hook";
  hook: "beforeAll" | "afterAll" | "beforeEach" | "afterEach";
  run: () => void | Promise<void>;
};
```

### `SnapshotOptions`

```ts
type SnapshotOptions = {
  name?: string;       // chave customizada do snapshot
  filePath?: string;   // caminho do arquivo de teste
  update?: boolean;    // forçar atualização do snapshot
};
```

---

## Assertions

### `assert(condition, message?)`

```ts
function assert(condition: unknown, message?: string): asserts condition
```

Falha quando `condition` é falsy.

---

### `assertEquals(actual, expected, message?)`

```ts
function assertEquals<T>(actual: T, expected: T, message?: string): void
```

Comparação profunda (deep equality). Suporta: primitivos, arrays, typed arrays, objetos, `Date`, `RegExp`, `Set`, `Map`.

Mensagem de falha inclui diff line-by-line + expected/actual.

---

### `assertNotEquals(actual, expected, message?)`

```ts
function assertNotEquals<T>(actual: T, expected: T, message?: string): void
```

Falha quando os valores são profundamente iguais.

---

### `assertStrictEquals(actual, expected, message?)`

```ts
function assertStrictEquals<T>(actual: T, expected: T, message?: string): void
```

Usa `Object.is(actual, expected)`. Diferente de `===`, trata `NaN === NaN` como `true` e distingue `+0` de `-0`.

---

### `assertNotStrictEquals(actual, expected, message?)`

```ts
function assertNotStrictEquals<T>(actual: T, expected: T, message?: string): void
```

Falha se `Object.is(actual, expected)` é `true`.

---

### `assertExists(value, message?)`

```ts
function assertExists<T>(value: T, message?: string): asserts value is NonNullable<T>
```

Falha quando `value` é `null` ou `undefined`.

---

### `assertInstanceOf(value, Type, message?)`

```ts
function assertInstanceOf<T>(
  value: unknown,
  type: new (...args: any[]) => T,
  message?: string,
): asserts value is T
```

Verifica `value instanceof type`.

---

### `assertType<T>(value)`

```ts
function assertType<T>(_value: T): void
```

Helper de compile-time apenas. Sem efeito em runtime. Útil para validar que um valor é de um tipo específico.

---

### `assertMatch(text, regex, message?)`

```ts
function assertMatch(text: string, regex: RegExp, message?: string): void
```

Falha quando `regex.test(text)` retorna `false`.

---

### `assertArrayIncludes(array, values, message?)`

```ts
function assertArrayIncludes<T>(
  array: readonly T[],
  values: readonly T[],
  message?: string,
): void
```

Falha se qualquer item em `values` não está em `array` (comparação por deep equality).

---

### `assertObjectMatch(actual, expected, message?)`

```ts
function assertObjectMatch(
  actual: Record<string, unknown>,
  expected: Record<string, unknown>,
  message?: string,
): void
```

Assertion de subconjunto. Falha se qualquer chave em `expected` está ausente ou diferente em `actual`. Chaves de `actual` que não estão em `expected` são ignoradas.

---

### `assertThrows(fn, ErrorClassOrMessage?, message?)`

```ts
function assertThrows(
  fn: () => unknown,
  ErrorClassOrMessage?: (new (...args: any[]) => Error) | string,
  message?: string,
): Error
```

- Falha se `fn` não lança
- Opcionalmente valida o tipo do erro lançado
- Retorna o `Error` capturado

Overloads:

```ts
assertThrows(fn);                        // qualquer erro
assertThrows(fn, TypeError);             // erro do tipo TypeError
assertThrows(fn, "mensagem customizada"); // qualquer erro, mensagem customizada
assertThrows(fn, TypeError, "msg");      // tipo + mensagem
```

---

### `assertRejects(fn, ErrorClassOrMessage?, message?)`

```ts
function assertRejects(
  fn: () => Promise<unknown>,
  ErrorClassOrMessage?: (new (...args: any[]) => Error) | string,
  message?: string,
): Promise<Error>
```

Versão assíncrona de `assertThrows`. Falha se a promise resolve. Retorna o `Error` de rejeição.

---

### `assertSnapshot(value, options?)`

```ts
function assertSnapshot(value: unknown, options?: SnapshotOptions): void
```

Compara `value` com um snapshot salvo em arquivo. Na primeira execução, cria o snapshot. Nas seguintes, compara.

- Snapshots ficam em `__snapshots__/<filename>.snap`
- Formato: JSON
- Chave: `options.name` ou nome do teste atual
- Se `options.update` é `true`, sempre sobrescreve

Requer Deno sync file APIs (`readTextFileSync`, `writeTextFileSync`, `mkdirSync`).

---

## Spy e Mock Assertions

### `assertSpyCalls(spy, count, message?)`

```ts
function assertSpyCalls(spy: SpyLike, count: number, message?: string): void
```

Verifica que o spy/mock foi chamado exatamente `count` vezes.

```ts
const fn = mockFn();
fn(); fn(); fn();
assertSpyCalls(fn, 3);
```

---

### `assertSpyCall(spy, index, expected?)`

```ts
function assertSpyCall(
  spy: SpyLike,
  index: number,
  expected?: {
    args?: unknown[];
    result?: unknown;
    error?: unknown;
  },
): void
```

Verifica uma chamada específica por índice. O objeto `expected` é parcial — verifique somente os campos desejados.

```ts
const fn = mockFn((a: number) => a * 2);
fn(5);

assertSpyCall(fn, 0, { args: [5], result: 10 });
assertSpyCall(fn, 0, { args: [5] });     // apenas args
assertSpyCall(fn, 0, { result: 10 });    // apenas result
```

Falha se `index` é negativo ou fora do range de `spy.calls`.

---

## Test Helpers

### `test(name, run, options?)`

```ts
function test(
  name: string,
  run: () => void | Promise<void>,
  options?: TestOptions,
): TestCase
```

Cria um caso de teste.

---

### `testIgnore(name, run, options?)`

```ts
function testIgnore(
  name: string,
  run: () => void | Promise<void>,
  options?: Omit<TestOptions, "ignore">,
): TestCase
```

Cria um caso de teste ignorado (não executa).

---

### `testOnly(name, run, options?)`

```ts
function testOnly(
  name: string,
  run: () => void | Promise<void>,
  options?: Omit<TestOptions, "only">,
): TestCase
```

Cria um caso de teste com foco — quando presente, só testes `only` rodam.

---

### `testIf(condition)`

```ts
function testIf(condition: boolean): (
  name: string,
  run: () => void | Promise<void>,
  options?: TestOptions,
) => TestCase
```

Retorna uma factory de teste condicional. Se `condition` é `false`, o teste é ignorado.

```ts
const hasGPU = checkGPUSupport();
testIf(hasGPU)("usa GPU", () => { /* ... */ });
```

---

### `testEach(rows)`

```ts
function testEach<T extends readonly unknown[]>(
  rows: readonly T[],
): (
  name: string,
  run: (...args: T) => void | Promise<void>,
  options?: TestOptions,
) => TestCase[]
```

Gera um array de testes, um para cada linha de dados. Nome gerado: `<name> [<index>] <JSON dos args>`.

```ts
const cases = testEach([
  [2, 3, 5],
  [10, 20, 30],
] as const)("soma", (a, b, expected) => {
  assertEquals(a + b, expected);
});
// Gera: "soma [0] [2,3,5]" e "soma [1] [10,20,30]"
```

---

## Suite Helpers

### `suite(name, tests)`

```ts
function suite(name: string, tests: SuiteEntry[]): TestSuite
```

Cria uma suite de testes.

---

### `suiteIgnore(name, tests)`

```ts
function suiteIgnore(name: string, tests: SuiteEntry[]): TestSuite
```

Cria uma suite ignorada.

---

### `suiteOnly(name, tests)`

```ts
function suiteOnly(name: string, tests: SuiteEntry[]): TestSuite
```

Cria uma suite com foco.

---

## Lifecycle Hooks

### `beforeAll(run)`

```ts
function beforeAll(run: () => void | Promise<void>): HookCase
```

Executa **uma vez** antes de todos os testes da suite.

---

### `afterAll(run)`

```ts
function afterAll(run: () => void | Promise<void>): HookCase
```

Executa **uma vez** após todos os testes da suite.

---

### `beforeEach(run)`

```ts
function beforeEach(run: () => void | Promise<void>): HookCase
```

Executa antes de **cada teste** individual.

---

### `afterEach(run)`

```ts
function afterEach(run: () => void | Promise<void>): HookCase
```

Executa após **cada teste** individual.

---

## Test Runner

### `runSuite(suiteName, tests, options?)`

```ts
function runSuite(
  suiteName: string,
  tests: SuiteEntry[],
  options?: SuiteOptions,
): Promise<void>
```

Executa uma suite de testes. Comportamento:

1. Se `options.ignore` é `true`, a suite inteira é ignorada
2. Hooks `beforeAll` executam primeiro
3. Se algum teste tem `only: true`, apenas esses executam
4. Testes sequenciais executam na ordem
5. Testes `concurrent: true` executam em paralelo via `Promise.all`
6. Hooks `afterAll` executam no final
7. Se houver falhas, lança `AssertionError` com resumo

Saída no console:

```
suite: math
soma... OK
subtração... OK
divisão por zero... FAIL (ERROR)
suite done: 2/3 (ignored: 0, failed: 1)
```

---

### `runSuites(suites)`

```ts
function runSuites(suites: TestSuite[]): Promise<void>
```

Executa múltiplas suites em sequência. Se alguma suite tem `only: true`, apenas essas executam.

---

### `getTestRunnerStats()`

```ts
function getTestRunnerStats(): RunnerStats
```

Retorna estatísticas acumuladas de todas as execuções:

```ts
type RunnerStats = {
  suitesTotal: number;
  suitesPassed: number;
  suitesFailed: number;
  suitesIgnored: number;
  testsTotal: number;
  testsPassed: number;
  testsFailed: number;
  testsIgnored: number;
};
```

---

## Mock API

### `mockFn(impl?)`

```ts
function mockFn<T extends AnyFunction>(impl?: T): Mock<T>
```

Cria uma função mock com rastreamento de chamadas.

```ts
type AnyFunction = (this: unknown, ...args: any[]) => any;

type Mock<T> = ((...args: Parameters<T>) => ReturnType<T>) & {
  calls: MockCall[];
  mockClear: () => void;
  mockImplementation: (nextImpl: T) => void;
};

type MockCall = {
  args: unknown[];
  result?: unknown;
  error?: unknown;
};
```

**Propriedades:**

| Propriedade | Descrição |
|-------------|-----------|
| `calls` | Array de todas as chamadas registradas |
| `mockClear()` | Limpa o histórico de chamadas |
| `mockImplementation(fn)` | Substitui a implementação |

**Comportamento com Promises:** Se a implementação retorna uma Promise, `result` é preenchido com o valor resolvido e `error` com o erro de rejeição.

```ts
const fn = mockFn((x: number) => x * 2);
fn(5);
// fn.calls[0] === { args: [5], result: 10 }

fn.mockClear();
// fn.calls.length === 0

fn.mockImplementation((x) => x + 1);
fn(5);
// fn.calls[0] === { args: [5], result: 6 }
```

---

### `spyOn(target, key)`

```ts
function spyOn<T extends object, K extends MethodKey<T>>(
  target: T,
  key: K,
): Spy<Extract<T[K], AnyFunction>>
```

Substitui o método `key` de `target` por um spy que delega ao método original. Lança `TypeError` se o membro não é uma função.

```ts
type Spy<T> = Mock<T> & {
  restore: () => void;  // restaura o método original
};
```

```ts
const spy = spyOn(console, "log");
console.log("hello");
// spy.calls[0].args === ["hello"]
spy.restore(); // console.log volta ao original
```

---

## Mock de Fetch

### `mockFetch(routes)`

```ts
function mockFetch(routes: MockFetchRoutes): MockFetchController
```

Substitui `globalThis.fetch` por uma versão que resolve respostas a partir de um mapa URL → resposta.

```ts
type MockFetchRoutes = Record<string, MockFetchResponse>;

type MockFetchResponse = {
  status?: number;                    // default: 200
  body?: unknown;                     // BodyInit ou serializado como JSON
  headers?: Record<string, string>;
};

type MockFetchController = {
  calls: MockCall[];   // histórico de chamadas (args[0] é Request)
  restore: () => void; // restaura fetch original
};
```

**Regras do `body`:**
- `undefined` → Response sem body
- `BodyInit` (string, Blob, FormData, URLSearchParams, ReadableStream, ArrayBuffer, TypedArray) → usado diretamente
- Qualquer outro valor → `JSON.stringify(body)` + `content-type: application/json` automático

**URLs não mapeadas** retornam `Response` com status `404`.

```ts
const mock = mockFetch({
  "https://api.test/users": { status: 200, body: [{ id: 1 }] },
});

const res = await fetch("https://api.test/users");
// res.status === 200
// await res.json() === [{ id: 1 }]

mock.restore();
```

---

### `mockFetchHandler(handler)`

```ts
function mockFetchHandler(handler: MockFetchHandler): MockFetchController
```

Versão flexível: substitui `globalThis.fetch` por um handler customizado.

```ts
type MockFetchHandler = (request: Request) => Response | Promise<Response> | null | undefined;
```

Se o handler retorna `null` ou `undefined`, uma `Response` com status `501` é gerada automaticamente.

```ts
const mock = mockFetchHandler((req) => {
  if (req.method === "POST") {
    return new Response("created", { status: 201 });
  }
  return new Response("ok");
});

const res = await fetch("https://any.url", { method: "POST" });
// res.status === 201

mock.restore();
```

---

## Mock de Timers

### `mockTime()`

```ts
function mockTime(): MockClock
```

Substitui `setTimeout`, `clearTimeout`, `setInterval` e `clearInterval` por versões controladas por clock virtual.

```ts
type MockClock = {
  now: () => number;          // tempo atual do clock (inicializado com Date.now())
  tick: (ms: number) => void; // avança o tempo e executa timers devidos
  restore: () => void;        // restaura timers originais e limpa pendentes
};
```

**Comportamento de `tick(ms)`:**

1. Avança o clock em `ms` milissegundos
2. Executa todos os timers cujo `runAt <= now` na ordem correta
3. Timers de `setInterval` são reagendados automaticamente
4. `ms` deve ser um número finito >= 0

```ts
const clock = mockTime();

let calls = 0;
setInterval(() => calls++, 50);

clock.tick(150);
// calls === 3 (executou em 50, 100, 150)

clock.restore();
```

---

## Exports completos

Tudo disponível via `edge://assert/mod.ts`:

**Assertions:**
`assert`, `assertEquals`, `assertNotEquals`, `assertStrictEquals`, `assertNotStrictEquals`,
`assertExists`, `assertInstanceOf`, `assertType`, `assertMatch`, `assertArrayIncludes`,
`assertObjectMatch`, `assertThrows`, `assertRejects`, `assertSnapshot`, `assertSpyCalls`, `assertSpyCall`

**Test runner:**
`test`, `testIgnore`, `testOnly`, `testIf`, `testEach`,
`suite`, `suiteIgnore`, `suiteOnly`,
`beforeAll`, `afterAll`, `beforeEach`, `afterEach`,
`runSuite`, `runSuites`, `getTestRunnerStats`

**Mocks:**
`mockFn`, `spyOn`, `mockFetch`, `mockFetchHandler`, `mockTime`

**Tipos:**
`AssertionError`, `TestCase`, `TestOptions`, `TestSuite`, `SuiteEntry`, `SuiteOptions`, `HookCase`, `SnapshotOptions`,
`Mock`, `MockCall`, `AnyFunction`, `Spy`,
`MockFetchResponse`, `MockFetchRoutes`, `MockFetchController`, `MockFetchHandler`, `MockClock`
