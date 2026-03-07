import {
  runSuite,
  assert,
  assertEquals,
  assertRejects,
  test,
  testIf,
  testEach,
  beforeAll,
  beforeEach,
  afterEach,
  afterAll,
  getTestRunnerStats,
  assertSnapshot,
} from "edge://assert/mod.ts";

await runSuite("runner-advanced-features", [
  {
    kind: "test",
    name: "lifecycle hooks run in expected order",
    run: async () => {
      const events: string[] = [];
      await runSuite("inner-hooks", [
        beforeAll(() => {
          events.push("beforeAll");
        }),
        beforeEach(() => {
          events.push("beforeEach");
        }),
        test("a", () => {
          events.push("test:a");
        }),
        test("b", () => {
          events.push("test:b");
        }),
        afterEach(() => {
          events.push("afterEach");
        }),
        afterAll(() => {
          events.push("afterAll");
        }),
      ]);

      assertEquals(events, [
        "beforeAll",
        "beforeEach",
        "test:a",
        "afterEach",
        "beforeEach",
        "test:b",
        "afterEach",
        "afterAll",
      ]);
    },
  },
  {
    kind: "test",
    name: "test timeout fails suite",
    run: async () => {
      await runSuite("inner-timeout", [
        test("slow", async () => {
          await new Promise((resolve) => setTimeout(resolve, 25));
        }, { timeout: 5, expectFailure: true }),
      ]);
    },
  },
  {
    kind: "test",
    name: "retry recovers flaky test",
    run: async () => {
      let attempts = 0;
      await runSuite("inner-retry", [
        test("flaky", () => {
          attempts += 1;
          if (attempts < 3) {
            throw new Error("flaky");
          }
        }, { retry: 2 }),
      ]);
      assertEquals(attempts, 3);
    },
  },
  {
    kind: "test",
    name: "concurrent tests execute together",
    run: async () => {
      const started = performance.now();
      await runSuite("inner-concurrent", [
        test("c1", async () => {
          await new Promise((resolve) => setTimeout(resolve, 30));
        }, { concurrent: true }),
        test("c2", async () => {
          await new Promise((resolve) => setTimeout(resolve, 30));
        }, { concurrent: true }),
      ]);
      const elapsed = performance.now() - started;
      assert(elapsed < 55, `expected concurrent execution, got ${elapsed.toFixed(2)}ms`);
    },
  },
  {
    kind: "test",
    name: "testEach expands table-driven tests",
    run: async () => {
      await runSuite("inner-table", [
        ...testEach([
          [1, 2, 3] as const,
          [2, 3, 5] as const,
        ])("sum", (a: number, b: number, result: number) => {
          assertEquals(a + b, result);
        }),
      ]);
    },
  },
  {
    kind: "test",
    name: "testIf false marks test as ignored",
    run: async () => {
      const before = getTestRunnerStats();
      await runSuite("inner-conditional", [
        testIf(false)("skipped", () => {
          throw new Error("should not run");
        }),
      ]);
      const after = getTestRunnerStats();
      assertEquals(after.testsIgnored, before.testsIgnored + 1);
    },
  },
  testIf(
    typeof (globalThis as { Deno?: { readTextFileSync?: unknown; writeTextFileSync?: unknown; mkdirSync?: unknown } }).Deno?.readTextFileSync === "function"
      && typeof (globalThis as { Deno?: { readTextFileSync?: unknown; writeTextFileSync?: unknown; mkdirSync?: unknown } }).Deno?.writeTextFileSync === "function"
      && typeof (globalThis as { Deno?: { readTextFileSync?: unknown; writeTextFileSync?: unknown; mkdirSync?: unknown } }).Deno?.mkdirSync === "function",
  )("snapshot stores and compares values", () => {
    const value = { ok: true, nested: { n: 1 } };
    assertSnapshot(value, { name: "basic-snapshot" });
    assertSnapshot(value, { name: "basic-snapshot" });
  }),
]);
