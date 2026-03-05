export class AssertionError extends Error {
  constructor(message = "Assertion failed") {
    super(message);
    this.name = "AssertionError";
  }
}

type ErrorClass = new (...args: any[]) => Error;

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

function equal(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;

  if (a instanceof Date && b instanceof Date) {
    return a.getTime() === b.getTime();
  }

  if (a instanceof RegExp && b instanceof RegExp) {
    return a.toString() === b.toString();
  }

  if (ArrayBuffer.isView(a) && ArrayBuffer.isView(b)) {
    const av = a as ArrayLike<unknown>;
    const bv = b as ArrayLike<unknown>;
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
    const details = [
      message ?? "Expected values to be equal",
      `Expected: ${formatValue(expected)}`,
      `Actual:   ${formatValue(actual)}`,
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
  } catch (err) {
    if (!(err instanceof Error)) {
      throw new AssertionError(parsed.message ?? "Promise rejected with a non-Error value");
    }
    if (parsed.errorClass && !(err instanceof parsed.errorClass)) {
      throw new AssertionError(
        parsed.message ?? `Expected promise to reject with ${parsed.errorClass.name}, got ${err.constructor.name}`,
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
  } catch (err) {
    if (!(err instanceof Error)) {
      throw new AssertionError(parsed.message ?? "Function threw a non-Error value");
    }
    if (parsed.errorClass && !(err instanceof parsed.errorClass)) {
      throw new AssertionError(
        parsed.message ?? `Expected function to throw ${parsed.errorClass.name}, got ${err.constructor.name}`,
      );
    }
    return err;
  }

  throw new AssertionError(parsed.message ?? "Expected function to throw");
}

export type TestCase = {
  name: string;
  run: () => void | Promise<void>;
  ignore?: boolean;
  only?: boolean;
};

export type TestSuite = {
  name: string;
  tests: TestCase[];
  ignore?: boolean;
  only?: boolean;
};

export type SuiteOptions = {
  ignore?: boolean;
  only?: boolean;
};

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

export function test(name: string, run: () => void | Promise<void>): TestCase {
  return { name, run };
}

export function testIgnore(name: string, run: () => void | Promise<void>): TestCase {
  return { name, run, ignore: true };
}

export function testOnly(name: string, run: () => void | Promise<void>): TestCase {
  return { name, run, only: true };
}

export function suite(name: string, tests: TestCase[]): TestSuite {
  return { name, tests };
}

export function suiteIgnore(name: string, tests: TestCase[]): TestSuite {
  return { name, tests, ignore: true };
}

export function suiteOnly(name: string, tests: TestCase[]): TestSuite {
  return { name, tests, only: true };
}

export async function runSuite(
  suiteName: string,
  tests: TestCase[],
  options: SuiteOptions = {},
): Promise<void> {
  if (options.ignore) {
    console.log(`suite: ${suiteName} (${gray("IGNORED")})`);
    return;
  }

  console.log(`suite: ${suiteName}`);

  const onlyTests = tests.filter((t) => t.only);
  const selectedTests = onlyTests.length > 0 ? onlyTests : tests;

  let passed = 0;
  let ignored = 0;
  let failed = 0;
  const failures: string[] = [];

  for (const testCase of selectedTests) {
    if (testCase.ignore) {
      ignored += 1;
      console.log(`${testCase.name}... ${gray("IGNORED")}`);
      continue;
    }

    try {
      await testCase.run();
      passed += 1;
      console.log(`${testCase.name}... ${green("OK")}`);
    } catch (error) {
      failed += 1;
      const isError = error instanceof Error;
      const status = isError ? `${red("FAIL")} (${red("ERROR")})` : red("FAIL");
      console.log(`${testCase.name}... ${status}`);
      failures.push(
        isError && error.message ? `${testCase.name}: ${error.message}` : `${testCase.name}: ${String(error)}`,
      );
    }
  }

  const total = selectedTests.length;
  console.log(`suite done: ${passed}/${total} (ignored: ${ignored}, failed: ${failed})`);

  if (failures.length > 0) {
    throw new AssertionError(
      [
        `Suite '${suiteName}' failed with ${failed} test(s):`,
        ...failures,
      ].join("\n"),
    );
  }
}

export async function runSuites(suites: TestSuite[]): Promise<void> {
  const onlySuites = suites.filter((s) => s.only);
  const selectedSuites = onlySuites.length > 0 ? onlySuites : suites;

  for (const item of selectedSuites) {
    await runSuite(item.name, item.tests, { ignore: item.ignore });
  }
}
