import { runSuite, assert } from "edge://assert/mod.ts";

await runSuite("web-apis-partial-and-none", [
  {
    name: "event source exists and can be constructed",
    run: () => {
      assert(typeof EventSource === "function", "EventSource should exist");
      const source = new EventSource("https://example.com/events");
      source.close();
    },
  },
  {
    name: "file reader exists (partial support)",
    run: () => {
      assert(typeof FileReader === "function", "FileReader should exist");
      const reader = new FileReader();
      assert(typeof reader.readAsText === "function", "FileReader API shape");
    },
  },
  {
    name: "unsupported worker-like/web platform apis",
    run: () => {
      assert(typeof Worker === "undefined", "Worker should not exist");
      assert(typeof WebSocket === "function", "WebSocket should exist");
      assert(typeof indexedDB === "undefined", "indexedDB should not exist");
      assert(typeof Notification === "undefined", "Notification should not exist");
      assert(typeof ServiceWorker === "undefined", "ServiceWorker should not exist");
      assert(typeof HTMLRewriter === "undefined", "HTMLRewriter should not exist");
      assert(typeof caches === "undefined", "CacheStorage should not exist");
      assert(typeof GPU === "undefined", "WebGPU should not exist");
    },
  },
]);
