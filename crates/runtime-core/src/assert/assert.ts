export class AssertionError extends Error {
  constructor(message = "Assertion failed") {
    super(message);
    this.name = "AssertionError";
  }
}

type ErrorClass = new (...args: any[]) => Error;

type MaybePromise<T> = T | Promise<T>;

type HookType = "beforeAll" | "afterAll" | "beforeEach" | "afterEach";

export type HookRun = () => void | Promise<void>;

export type HookCase = {
  kind: "hook";
  hook: HookType;
  run: HookRun;
};

export type TestOptions = {
  ignore?: boolean;
  only?: boolean;
  timeout?: number;
  concurrent?: boolean;
  retry?: number;
  expectFailure?: boolean;
};

export type TestCase = {
  kind: "test";
  name: string;
  run: () => void | Promise<void>;
  ignore?: boolean;
  only?: boolean;
  timeout?: number;
  concurrent?: boolean;
  retry?: number;
  expectFailure?: boolean;
};

type LegacyTestCase = {
  name: string;
  run: () => void | Promise<void>;
  ignore?: boolean;
  only?: boolean;
  timeout?: number;
  concurrent?: boolean;
  retry?: number;
  expectFailure?: boolean;
};

export type SuiteEntry = TestCase | HookCase | LegacyTestCase | SuiteEntry[];

export type TestSuite = {
  name: string;
  tests: SuiteEntry[];
  ignore?: boolean;
  only?: boolean;
};

export type SuiteOptions = {
  ignore?: boolean;
  only?: boolean;
};

type SpyCallLike = {
  args: unknown[];
  result?: unknown;
  error?: unknown;
};

type SpyLike = {
  calls: SpyCallLike[];
};

type SpyCallExpectation = {
  args?: unknown[];
  result?: unknown;
  error?: unknown;
};

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

type RunnerContext = {
  suiteName?: string;
  testName?: string;
  filePath?: string;
};

export type SnapshotOptions = {
  name?: string;
  filePath?: string;
  update?: boolean;
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function formatValue(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function diffLines(expectedText: string, actualText: string): string {
  const expectedLines = expectedText.split("\n");
  const actualLines = actualText.split("\n");
  const max = Math.max(expectedLines.length, actualLines.length);
  const out: string[] = [];

  for (let i = 0; i < max; i += 1) {
    const expected = expectedLines[i];
    const actual = actualLines[i];
    if (expected === actual) continue;
    if (expected !== undefined) out.push(`- ${expected}`);
    if (actual !== undefined) out.push(`+ ${actual}`);
    if (out.length >= 80) {
      out.push("... diff truncated ...");
      break;
    }
  }

  return out.length > 0 ? out.join("\n") : "(no diff available)";
}

function equal(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;

  if (a instanceof Date && b instanceof Date) {
    return a.getTime() === b.getTime();
  }

  if (a instanceof RegExp && b instanceof RegExp) {
    return a.toString() === b.toString();
  }

  if (ArrayBuffer.isView(a) && ArrayBuffer.isView(b)) {
    const av = a as unknown as { length: number; [index: number]: unknown };
    const bv = b as unknown as { length: number; [index: number]: unknown };
    if (av.length !== bv.length) return false;
    for (let i = 0; i < av.length; i += 1) {
      if (!Object.is(av[i], bv[i])) return false;
    }
    return true;
  }

  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i += 1) {
      if (!equal(a[i], b[i])) return false;
    }
    return true;
  }

  if (a instanceof Set && b instanceof Set) {
    if (a.size !== b.size) return false;
    for (const av of a) {
      let found = false;
      for (const bv of b) {
        if (equal(av, bv)) {
          found = true;
          break;
        }
      }
      if (!found) return false;
    }
    return true;
  }

  if (a instanceof Map && b instanceof Map) {
    if (a.size !== b.size) return false;
    for (const [ak, av] of a) {
      let found = false;
      for (const [bk, bv] of b) {
        if (equal(ak, bk) && equal(av, bv)) {
          found = true;
          break;
        }
      }
      if (!found) return false;
    }
    return true;
  }

  if (isObject(a) && isObject(b)) {
    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    for (const key of keysA) {
      if (!Object.prototype.hasOwnProperty.call(b, key)) return false;
      if (!equal(a[key], b[key])) return false;
    }
    return true;
  }

  return false;
}

export function assert(condition: unknown, message = "Assertion failed"): asserts condition {
  if (!condition) {
    throw new AssertionError(message);
  }
}

export function assertEquals<T>(actual: T, expected: T, message?: string): void {
  if (!equal(actual, expected)) {
    const expectedText = formatValue(expected);
    const actualText = formatValue(actual);
    const details = [
      message ?? "Values are not equal",
      "",
      diffLines(expectedText, actualText),
      "",
      `Expected: ${expectedText}`,
      `Actual:   ${actualText}`,
    ].join("\n");
    throw new AssertionError(details);
  }
}

export function assertNotEquals<T>(actual: T, expected: T, message?: string): void {
  if (equal(actual, expected)) {
    throw new AssertionError(message ?? "Expected values to be different");
  }
}

export function assertStrictEquals<T>(actual: T, expected: T, message?: string): void {
  if (!Object.is(actual, expected)) {
    throw new AssertionError(message ?? `Expected strictly equal values, got ${formatValue(actual)} and ${formatValue(expected)}`);
  }
}

export function assertNotStrictEquals<T>(actual: T, expected: T, message?: string): void {
  if (Object.is(actual, expected)) {
    throw new AssertionError(message ?? "Expected values to be not strictly equal");
  }
}

export function assertInstanceOf<T>(value: unknown, type: new (...args: any[]) => T, message?: string): asserts value is T {
  if (!(value instanceof type)) {
    throw new AssertionError(message ?? `Expected value to be instance of ${type.name}`);
  }
}

export function assertType<T>(_value: T): void {
  // TypeScript compile-time helper. Runtime no-op.
}

export function assertMatch(text: string, regex: RegExp, message?: string): void {
  if (!regex.test(text)) {
    throw new AssertionError(message ?? `Expected '${text}' to match ${String(regex)}`);
  }
}

export function assertArrayIncludes<T>(
  array: readonly T[],
  values: readonly T[],
  message?: string,
): void {
  const missing = values.filter((value) => !array.some((item) => equal(item, value)));
  if (missing.length > 0) {
    throw new AssertionError(
      message ?? `Expected array to include all values. Missing: ${formatValue(missing)}`,
    );
  }
}

export function assertObjectMatch(
  actual: Record<string, unknown>,
  expected: Record<string, unknown>,
  message?: string,
): void {
  for (const key of Object.keys(expected)) {
    if (!Object.prototype.hasOwnProperty.call(actual, key)) {
      throw new AssertionError(message ?? `Expected object to include key '${key}'`);
    }
    if (!equal(actual[key], expected[key])) {
      throw new AssertionError(
        message ?? `Expected object key '${key}' to match. Expected ${formatValue(expected[key])}, got ${formatValue(actual[key])}`,
      );
    }
  }
}

export function assertExists<T>(value: T, message = "Expected value to exist"): asserts value is NonNullable<T> {
  if (value === null || value === undefined) {
    throw new AssertionError(message);
  }
}

function parseAssertArgs(
  second?: ErrorClass | string,
  third?: string,
): { errorClass?: ErrorClass; message?: string } {
  if (typeof second === "string") {
    return { message: second };
  }
  return { errorClass: second, message: third };
}

export async function assertRejects(
  fn: () => Promise<unknown>,
  ErrorClassOrMessage?: ErrorClass | string,
  message?: string,
): Promise<Error> {
  const parsed = parseAssertArgs(ErrorClassOrMessage, message);

  try {
    await fn();
  } catch (err: unknown) {
    if (!(err instanceof Error)) {
      throw new AssertionError(parsed.message ?? "Promise rejected with a non-Error value");
    }
    if (parsed.errorClass && !(err instanceof parsed.errorClass)) {
      const errName = (err as Error).name;
      throw new AssertionError(
        parsed.message ?? `Expected promise to reject with ${parsed.errorClass.name}, got ${errName}`,
      );
    }
    return err;
  }

  throw new AssertionError(parsed.message ?? "Expected promise to reject");
}

export function assertThrows(
  fn: () => unknown,
  ErrorClassOrMessage?: ErrorClass | string,
  message?: string,
): Error {
  const parsed = parseAssertArgs(ErrorClassOrMessage, message);

  try {
    fn();
  } catch (err: unknown) {
    if (!(err instanceof Error)) {
      throw new AssertionError(parsed.message ?? "Function threw a non-Error value");
    }
    if (parsed.errorClass && !(err instanceof parsed.errorClass)) {
      const errName = (err as Error).name;
      throw new AssertionError(
        parsed.message ?? `Expected function to throw ${parsed.errorClass.name}, got ${errName}`,
      );
    }
    return err;
  }

  throw new AssertionError(parsed.message ?? "Expected function to throw");
}

// deno-lint-ignore no-explicit-any
function getRunnerStatsStore(): RunnerStats {
  const globalScope = globalThis as any;
  if (!globalScope.__edgeTestStats) {
    globalScope.__edgeTestStats = {
      suitesTotal: 0,
      suitesPassed: 0,
      suitesFailed: 0,
      suitesIgnored: 0,
      testsTotal: 0,
      testsPassed: 0,
      testsFailed: 0,
      testsIgnored: 0,
    } as RunnerStats;
  }
  return globalScope.__edgeTestStats as RunnerStats;
}

export function getTestRunnerStats(): RunnerStats {
  const stats = getRunnerStatsStore();
  return { ...stats };
}

// deno-lint-ignore no-explicit-any
function getRunnerContextStore(): RunnerContext {
  const globalScope = globalThis as any;
  if (!globalScope.__edgeTestContext) {
    globalScope.__edgeTestContext = {} as RunnerContext;
  }

  if (!globalScope.__edgeTestContext.filePath && typeof globalScope.__edgeTestFilePath === "string") {
    globalScope.__edgeTestContext.filePath = globalScope.__edgeTestFilePath;
  }

  return globalScope.__edgeTestContext as RunnerContext;
}

function isColorEnabled(): boolean {
  // Respect common terminal color disable signals.
  // deno-lint-ignore no-explicit-any
  const env = (globalThis as any).Deno?.env;
  if (!env?.get) return true;
  return env.get("NO_COLOR") == null && env.get("TERM") !== "dumb";
}

function color(text: string, code: string): string {
  if (!isColorEnabled()) return text;
  return `\x1b[${code}m${text}\x1b[0m`;
}

function green(text: string): string {
  return color(text, "32");
}

function red(text: string): string {
  return color(text, "31");
}

function gray(text: string): string {
  return color(text, "90");
}

export function test(name: string, run: () => void | Promise<void>, options: TestOptions = {}): TestCase {
  return { kind: "test", name, run, ...options };
}

export function testIgnore(name: string, run: () => void | Promise<void>, options: Omit<TestOptions, "ignore"> = {}): TestCase {
  return test(name, run, { ...options, ignore: true });
}

export function testOnly(name: string, run: () => void | Promise<void>, options: Omit<TestOptions, "only"> = {}): TestCase {
  return test(name, run, { ...options, only: true });
}

export function testIf(condition: boolean): (
  name: string,
  run: () => void | Promise<void>,
  options?: TestOptions,
) => TestCase {
  return (name, run, options = {}) => test(name, run, { ...options, ignore: options.ignore ?? !condition });
}

export function testEach<T extends readonly unknown[]>(
  rows: readonly T[],
): (
  name: string,
  run: (...args: T) => void | Promise<void>,
  options?: TestOptions,
) => TestCase[] {
  return (name, run, options = {}) => rows.map((row, index) => {
    const testName = `${name} [${index}] ${formatValue(row)}`;
    return test(testName, () => run(...row), options);
  });
}

export function beforeAll(run: HookRun): HookCase {
  return { kind: "hook", hook: "beforeAll", run };
}

export function afterAll(run: HookRun): HookCase {
  return { kind: "hook", hook: "afterAll", run };
}

export function beforeEach(run: HookRun): HookCase {
  return { kind: "hook", hook: "beforeEach", run };
}

export function afterEach(run: HookRun): HookCase {
  return { kind: "hook", hook: "afterEach", run };
}

export function suite(name: string, tests: SuiteEntry[]): TestSuite {
  return { name, tests };
}

export function suiteIgnore(name: string, tests: SuiteEntry[]): TestSuite {
  return { name, tests, ignore: true };
}

export function suiteOnly(name: string, tests: SuiteEntry[]): TestSuite {
  return { name, tests, only: true };
}

function flattenEntries(entries: SuiteEntry[]): Array<TestCase | HookCase> {
  const out: Array<TestCase | HookCase> = [];
  for (const entry of entries) {
    if (Array.isArray(entry)) {
      out.push(...flattenEntries(entry));
    } else {
      if ((entry as HookCase).kind === "hook") {
        out.push(entry as HookCase);
      } else {
        const testEntry = entry as LegacyTestCase;
        out.push({
          kind: "test",
          name: testEntry.name,
          run: testEntry.run,
          ignore: testEntry.ignore,
          only: testEntry.only,
          timeout: testEntry.timeout,
          concurrent: testEntry.concurrent,
          retry: testEntry.retry,
          expectFailure: testEntry.expectFailure,
        });
      }
    }
  }
  return out;
}

function withTimeout(run: () => Promise<void>, timeoutMs?: number): Promise<void> {
  if (!timeoutMs || timeoutMs <= 0) {
    return run();
  }

  return new Promise<void>((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      reject(new AssertionError(`Test timeout: exceeded ${timeoutMs}ms`));
    }, timeoutMs);

    run()
      .then(() => {
        clearTimeout(timeoutId);
        resolve();
      })
      .catch((error) => {
        clearTimeout(timeoutId);
        reject(error);
      });
  });
}

async function runHooks(hooks: HookCase[]): Promise<void> {
  for (const hook of hooks) {
    await hook.run();
  }
}

async function runTestWithRetry(testCase: TestCase): Promise<void> {
  const retry = Math.max(0, testCase.retry ?? 0);
  let lastError: unknown;

  for (let attempt = 0; attempt <= retry; attempt += 1) {
    try {
      await withTimeout(async () => {
        await testCase.run();
      }, testCase.timeout);
      return;
    } catch (error) {
      lastError = error;
      if (attempt === retry) {
        throw error;
      }
    }
  }

  throw lastError;
}

async function runSingleTestCase(
  suiteName: string,
  testCase: TestCase,
  beforeEachHooks: HookCase[],
  afterEachHooks: HookCase[],
): Promise<{ ok: boolean; error?: unknown }> {
  const context = getRunnerContextStore();
  context.suiteName = suiteName;
  context.testName = testCase.name;

  try {
    await runHooks(beforeEachHooks);
    await runTestWithRetry(testCase);
    return { ok: true };
  } catch (error) {
    return { ok: false, error };
  } finally {
    try {
      await runHooks(afterEachHooks);
    } catch (hookError) {
      return { ok: false, error: hookError };
    }
  }
}

export function assertSpyCalls(spy: SpyLike, count: number, message?: string): void {
  assertEquals(
    spy.calls.length,
    count,
    message ?? `Expected mock/spy to be called ${count} time(s), got ${spy.calls.length}`,
  );
}

export function assertSpyCall(
  spy: SpyLike,
  index: number,
  expected: SpyCallExpectation = {},
): void {
  assert(index >= 0, `Spy call index must be >= 0, got ${index}`);
  assert(index < spy.calls.length, `Spy call index out of range: ${index} (calls: ${spy.calls.length})`);

  const call = spy.calls[index];

  if (Object.prototype.hasOwnProperty.call(expected, "args")) {
    assertEquals(call.args, expected.args, `Expected spy call ${index} args to match`);
  }

  if (Object.prototype.hasOwnProperty.call(expected, "result")) {
    assertEquals(call.result, expected.result, `Expected spy call ${index} result to match`);
  }

  if (Object.prototype.hasOwnProperty.call(expected, "error")) {
    assertEquals(call.error, expected.error, `Expected spy call ${index} error to match`);
  }
}

function normalizePath(input: string): string {
  if (input.startsWith("file://")) {
    try {
      return decodeURIComponent(new URL(input).pathname);
    } catch {
      return input;
    }
  }
  return input;
}

function dirname(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf("/");
  if (index <= 0) return ".";
  return normalized.slice(0, index);
}

function basename(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf("/");
  return index >= 0 ? normalized.slice(index + 1) : normalized;
}

function removeExtension(fileName: string): string {
  const index = fileName.lastIndexOf(".");
  if (index <= 0) return fileName;
  return fileName.slice(0, index);
}

function getSnapshotRuntime() {
  // deno-lint-ignore no-explicit-any
  const deno = (globalThis as any).Deno;
  if (!deno || typeof deno.readTextFileSync !== "function" || typeof deno.writeTextFileSync !== "function" || typeof deno.mkdirSync !== "function") {
    throw new AssertionError("Snapshot testing requires Deno sync file APIs (readTextFileSync/writeTextFileSync/mkdirSync)");
  }
  return deno;
}

function safeSnapshotValue(value: unknown): unknown {
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    return String(value);
  }
}

export function assertSnapshot(value: unknown, options: SnapshotOptions = {}): void {
  const deno = getSnapshotRuntime();
  const context = getRunnerContextStore();
  const filePath = normalizePath(options.filePath ?? context.filePath ?? "");

  if (!filePath) {
    throw new AssertionError("assertSnapshot could not resolve current test file path; pass { filePath } explicitly");
  }

  const testFileName = basename(filePath);
  const snapshotName = options.name ?? context.testName ?? "default";
  const snapshotDir = `${dirname(filePath)}/__snapshots__`;
  const snapshotFile = `${snapshotDir}/${removeExtension(testFileName)}.snap`;

  deno.mkdirSync(snapshotDir, { recursive: true });

  let snapshotData: Record<string, unknown> = {};
  try {
    const text = deno.readTextFileSync(snapshotFile);
    const parsed = JSON.parse(text);
    if (isObject(parsed)) {
      snapshotData = parsed;
    }
  } catch {
    snapshotData = {};
  }

  const serialized = safeSnapshotValue(value);
  const hasExisting = Object.prototype.hasOwnProperty.call(snapshotData, snapshotName);

  if (!hasExisting || options.update) {
    snapshotData[snapshotName] = serialized;
    deno.writeTextFileSync(snapshotFile, JSON.stringify(snapshotData, null, 2) + "\n");
    return;
  }

  const expected = snapshotData[snapshotName];
  if (!equal(serialized, expected)) {
    const expectedText = formatValue(expected);
    const actualText = formatValue(serialized);
    throw new AssertionError(
      [
        `Snapshot mismatch: '${snapshotName}'`,
        "",
        diffLines(expectedText, actualText),
        "",
        `Snapshot file: ${snapshotFile}`,
      ].join("\n"),
    );
  }
}

export async function runSuite(
  suiteName: string,
  tests: SuiteEntry[],
  options: SuiteOptions = {},
): Promise<void> {
  const stats = getRunnerStatsStore();
  const context = getRunnerContextStore();
  context.suiteName = suiteName;

  stats.suitesTotal += 1;

  if (options.ignore) {
    stats.suitesIgnored += 1;
    console.log(`suite: ${suiteName} (${gray("IGNORED")})`);
    return;
  }

  console.log(`suite: ${suiteName}`);

  const entries = flattenEntries(tests);
  const hooks: Record<HookType, HookCase[]> = {
    beforeAll: [],
    afterAll: [],
    beforeEach: [],
    afterEach: [],
  };

  const allTests = entries.filter((entry): entry is TestCase => entry.kind === "test");
  const onlyTests = allTests.filter((t) => t.only);
  const selectedTests = onlyTests.length > 0 ? onlyTests : allTests;

  for (const entry of entries) {
    if (entry.kind === "hook") {
      hooks[entry.hook].push(entry);
    }
  }

  let passed = 0;
  let ignored = 0;
  let failed = 0;
  const failures: string[] = [];

  try {
    await runHooks(hooks.beforeAll);
  } catch (error) {
    failed += 1;
    stats.suitesFailed += 1;
    failures.push(`beforeAll: ${error instanceof Error ? error.message : String(error)}`);
    throw new AssertionError([`Suite '${suiteName}' failed in beforeAll`, ...failures].join("\n"));
  }

  const sequentialTests: TestCase[] = [];
  const concurrentTests: TestCase[] = [];

  for (const testCase of selectedTests) {
    stats.testsTotal += 1;

    if (testCase.ignore) {
      ignored += 1;
      stats.testsIgnored += 1;
      console.log(`${testCase.name}... ${gray("IGNORED")}`);
      continue;
    }

    if (testCase.concurrent) {
      concurrentTests.push(testCase);
    } else {
      sequentialTests.push(testCase);
    }
  }

  for (const testCase of sequentialTests) {
    const result = await runSingleTestCase(suiteName, testCase, hooks.beforeEach, hooks.afterEach);
    if (result.ok && !testCase.expectFailure) {
      passed += 1;
      stats.testsPassed += 1;
      console.log(`${testCase.name}... ${green("OK")}`);
    } else if (!result.ok && testCase.expectFailure) {
      passed += 1;
      stats.testsPassed += 1;
      console.log(`${testCase.name}... ${green("OK")} ${gray("(expected failure)")}`);
    } else {
      failed += 1;
      stats.testsFailed += 1;
      const error = result.error;
      const isError = error instanceof Error;
      const status = testCase.expectFailure
        ? `${red("FAIL")} ${gray("(expected failure but passed)")}`
        : (isError ? `${red("FAIL")} (${red("ERROR")})` : red("FAIL"));
      console.log(`${testCase.name}... ${status}`);
      if (testCase.expectFailure) {
        failures.push(`${testCase.name}: expected failure but test passed`);
      } else {
        failures.push(isError ? `${testCase.name}: ${error.message}` : `${testCase.name}: ${String(error)}`);
      }
    }
  }

  if (concurrentTests.length > 0) {
    const concurrentResults = await Promise.all(
      concurrentTests.map(async (testCase) => ({
        testCase,
        result: await runSingleTestCase(suiteName, testCase, hooks.beforeEach, hooks.afterEach),
      })),
    );

    for (const { testCase, result } of concurrentResults) {
      if (result.ok && !testCase.expectFailure) {
        passed += 1;
        stats.testsPassed += 1;
        console.log(`${testCase.name}... ${green("OK")}`);
      } else if (!result.ok && testCase.expectFailure) {
        passed += 1;
        stats.testsPassed += 1;
        console.log(`${testCase.name}... ${green("OK")} ${gray("(expected failure)")}`);
      } else {
        failed += 1;
        stats.testsFailed += 1;
        const error = result.error;
        const isError = error instanceof Error;
        const status = testCase.expectFailure
          ? `${red("FAIL")} ${gray("(expected failure but passed)")}`
          : (isError ? `${red("FAIL")} (${red("ERROR")})` : red("FAIL"));
        console.log(`${testCase.name}... ${status}`);
        if (testCase.expectFailure) {
          failures.push(`${testCase.name}: expected failure but test passed`);
        } else {
          failures.push(isError ? `${testCase.name}: ${error.message}` : `${testCase.name}: ${String(error)}`);
        }
      }
    }
  }

  try {
    await runHooks(hooks.afterAll);
  } catch (error) {
    failed += 1;
    failures.push(`afterAll: ${error instanceof Error ? error.message : String(error)}`);
  }

  const total = selectedTests.length;
  console.log(`suite done: ${passed}/${total} (ignored: ${ignored}, failed: ${failed})`);

  if (failures.length > 0) {
    stats.suitesFailed += 1;
    throw new AssertionError(
      [
        `Suite '${suiteName}' failed with ${failed} issue(s):`,
        ...failures,
      ].join("\n"),
    );
  }

  stats.suitesPassed += 1;
}

export async function runSuites(suites: TestSuite[]): Promise<void> {
  const onlySuites = suites.filter((s) => s.only);
  const selectedSuites = onlySuites.length > 0 ? onlySuites : suites;

  for (const item of selectedSuites) {
    await runSuite(item.name, item.tests, { ignore: item.ignore });
  }
}

export {
  mockFn,
  spyOn,
  mockFetch,
  mockFetchHandler,
  mockTime,
  type Mock,
  type MockCall,
  type AnyFunction,
  type Spy,
  type MockFetchResponse,
  type MockFetchRoutes,
  type MockFetchController,
  type MockFetchHandler,
  type MockClock,
} from "./mock/mod.ts";
