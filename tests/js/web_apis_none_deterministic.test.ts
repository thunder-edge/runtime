import { runSuite, assertEquals, assertThrows } from "thunder:testing";

const globals = globalThis as Record<string, unknown>;

function assertMissingConstructor(name: string): void {
  assertEquals(typeof globals[name], "undefined", `${name} should be unavailable`);
  assertThrows(() => {
    Reflect.construct(globals[name] as never, []);
  }, TypeError, `${name} constructor path should fail deterministically`);
}

await runSuite("web-apis-none-deterministic", [
  {
    name: "Worker API is unavailable and construct path fails deterministically",
    run: () => {
      assertMissingConstructor("Worker");
    },
  },
  {
    name: "Cache API is unavailable and method call fails deterministically",
    run: () => {
      assertEquals(typeof globals.caches, "undefined", "caches should be unavailable");
      assertThrows(() => {
        (globals.caches as { open(name: string): unknown }).open("runtime-cache");
      }, TypeError, "caches.open should fail deterministically");
    },
  },
  {
    name: "Service Worker API is unavailable and registration path fails deterministically",
    run: () => {
      assertEquals(typeof globals.ServiceWorker, "undefined", "ServiceWorker should be unavailable");
      assertThrows(() => {
        (globals.navigator as { serviceWorker: { register(url: string): unknown } }).serviceWorker.register("/sw.js");
      }, TypeError, "navigator.serviceWorker.register should fail deterministically");
    },
  },
  {
    name: "Notification API is unavailable and construct path fails deterministically",
    run: () => {
      assertMissingConstructor("Notification");
    },
  },
  {
    name: "IndexedDB API is unavailable and open path fails deterministically",
    run: () => {
      assertEquals(typeof globals.indexedDB, "undefined", "indexedDB should be unavailable");
      assertThrows(() => {
        (globals.indexedDB as { open(name: string): unknown }).open("compat-db");
      }, TypeError, "indexedDB.open should fail deterministically");
    },
  },
  {
    name: "WebGPU API is unavailable and request path fails deterministically",
    run: () => {
      assertEquals(typeof globals.GPU, "undefined", "GPU constructor should be unavailable");
      assertThrows(() => {
        (globals.navigator as { gpu: { requestAdapter(): unknown } }).gpu.requestAdapter();
      }, TypeError, "navigator.gpu.requestAdapter should fail deterministically");
    },
  },
  {
    name: "HTMLRewriter is unavailable and construct path fails deterministically",
    run: () => {
      assertMissingConstructor("HTMLRewriter");
    },
  },
  {
    name: "Cloudflare-style non-standard globals remain unavailable",
    run: () => {
      assertMissingConstructor("ScheduledEvent");
      assertMissingConstructor("KVNamespace");
      assertMissingConstructor("DurableObject");
      assertMissingConstructor("DurableObjectNamespace");
    },
  },
]);
