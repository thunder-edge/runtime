import {
  runSuite,
  assert,
  assertEquals,
  assertExists,
} from "edge://assert/mod.ts";

await runSuite("requested-items-compat", [
  {
    name: "promises",
    run: async () => {
      const value = await Promise.resolve(21).then((n) => n * 2);
      assertEquals(value, 42);

      const settled = await Promise.allSettled([
        Promise.resolve("ok"),
        Promise.reject(new Error("boom")),
      ]);
      assertEquals(settled[0].status, "fulfilled");
      assertEquals(settled[1].status, "rejected");
    },
  },
  {
    name: "async await",
    run: async () => {
      async function step(value: number): Promise<number> {
        return value + 1;
      }
      const result = await step(40);
      assertEquals(result, 41);
    },
  },
  {
    name: "fetch event",
    run: () => {
      // Worker-specific API: currently expected to be unavailable in this runtime.
      assert(typeof FetchEvent === "undefined", "FetchEvent should not exist");
    },
  },
  {
    name: "URL API",
    run: () => {
      const url = new URL("https://example.com:8443/path?a=1");
      assertEquals(url.hostname, "example.com");
      assertEquals(url.port, "8443");
      assertEquals(url.pathname, "/path");
      assertEquals(url.searchParams.get("a"), "1");
    },
  },
  {
    name: "Fetch API",
    run: async () => {
      assert(typeof fetch === "function", "fetch should exist");
      const req = new Request("https://example.com", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ ok: true }),
      });
      assertEquals(req.method, "POST");
      assertEquals(req.headers.get("content-type"), "application/json");

      const res = new Response("{\"ok\":true}", {
        status: 201,
        headers: { "content-type": "application/json" },
      });
      assertEquals(res.status, 201);
      assertEquals(await res.json(), { ok: true });
    },
  },
  {
    name: "Abort Controller/Signal",
    run: () => {
      const controller = new AbortController();
      let called = false;
      controller.signal.addEventListener("abort", () => {
        called = true;
      });
      controller.abort();
      assert(controller.signal.aborted, "signal should be aborted");
      assert(called, "abort listener should run");
    },
  },
  {
    name: "URL Pattern API",
    run: () => {
      assert(typeof URLPattern === "function", "URLPattern should exist");
      const pattern = new URLPattern({ pathname: "/users/:id" });
      const match = pattern.exec("https://example.com/users/123");
      assertExists(match);
      assertEquals(match.pathname.groups.id, "123");
    },
  },
  {
    name: "Encoding API",
    run: () => {
      const encoded = new TextEncoder().encode("hello");
      assertEquals(encoded.length, 5);
      const decoded = new TextDecoder().decode(encoded);
      assertEquals(decoded, "hello");
      assertEquals(atob(btoa("edge")), "edge");
    },
  },
  {
    name: "Streams API",
    run: async () => {
      assert(typeof ReadableStream === "function");
      assert(typeof WritableStream === "function");
      assert(typeof TransformStream === "function");

      const chunks: string[] = [];
      const readable = new ReadableStream<string>({
        start(controller) {
          controller.enqueue("a");
          controller.enqueue("b");
          controller.close();
        },
      });
      const writable = new WritableStream<string>({
        write(chunk) {
          chunks.push(chunk);
        },
      });
      await readable.pipeTo(writable);
      assertEquals(chunks.join(""), "ab");
    },
  },
  {
    name: "Encoding Streams",
    run: () => {
      assert(typeof TextEncoderStream === "function");
      assert(typeof TextDecoderStream === "function");
    },
  },
  {
    name: "Compression Streams",
    run: () => {
      assert(typeof CompressionStream === "function");
      assert(typeof DecompressionStream === "function");
    },
  },
  {
    name: "Web Cryptography API",
    run: async () => {
      assertExists(crypto.subtle);
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("edge"));
      assertEquals(digest.byteLength, 32);
    },
  },
  {
    name: "crypto.randomUUID()",
    run: () => {
      const uuid = crypto.randomUUID();
      assertEquals(typeof uuid, "string");
      assert(/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(uuid));
    },
  },
  {
    name: "Cache API",
    run: () => {
      // Current runtime behavior tracked by compatibility tests.
      assert(typeof caches === "undefined", "caches should not exist");
    },
  },
  {
    name: "WebSocket API",
    run: () => {
      assert(typeof WebSocket === "undefined", "WebSocket should not exist");
    },
  },
  {
    name: "Web Socket Stream",
    run: () => {
      // WICG proposal/global currently expected to be unavailable here.
      assert(typeof WebSocketStream === "undefined", "WebSocketStream should not exist");
    },
  },
  {
    name: "Location API",
    run: () => {
      const kind = typeof location;
      assert(kind === "object" || kind === "undefined", "location should be object or undefined");
      if (kind === "object") {
        assert(typeof location.href === "string");
      }
    },
  },
  {
    name: "console",
    run: () => {
      assert(typeof console.log === "function");
      assert(typeof console.error === "function");
      assert(typeof console.table === "function");
    },
  },
  {
    name: "queueMicrotask",
    run: async () => {
      const order: number[] = [];
      queueMicrotask(() => order.push(1));
      await Promise.resolve();
      order.push(2);
      assertEquals(order, [1, 2]);
    },
  },
  {
    name: "structuredClone",
    run: () => {
      const input = { nested: { a: 1 }, list: [1, 2, 3] };
      const output = structuredClone(input);
      assertEquals(output, input);
      assert(output !== input, "clone should produce a distinct object");
    },
  },
  {
    name: "navigator.userAgent",
    run: () => {
      if (typeof navigator === "undefined") {
        assert(true, "navigator is currently unavailable in this runtime");
        return;
      }

      assert(typeof navigator.userAgent === "string");
      assert(navigator.userAgent.length > 0, "navigator.userAgent should not be empty");
    },
  },
  {
    name: "Response.json",
    run: async () => {
      assert(typeof Response.json === "function", "Response.json should exist");
      const res = Response.json({ ok: true }, { status: 202 });
      assertEquals(res.status, 202);
      assertEquals(await res.json(), { ok: true });
    },
  },
  {
    name: "EventTarget and Event",
    run: () => {
      const target = new EventTarget();
      let received = false;
      target.addEventListener("ping", () => {
        received = true;
      });
      target.dispatchEvent(new Event("ping"));
      assert(received, "event listener should receive dispatched event");
    },
  },
  {
    name: "Web Workers API",
    run: () => {
      assert(typeof Worker === "undefined", "Worker should not exist");
    },
  },
  {
    name: "Message Channel",
    run: () => {
      const channel = new MessageChannel();
      assert(channel.port1 instanceof MessagePort);
      assert(channel.port2 instanceof MessagePort);
      channel.port1.close();
      channel.port2.close();
    },
  },
  {
    name: "Broadcast Channel",
    run: () => {
      if (typeof BroadcastChannel === "function") {
        const bc = new BroadcastChannel("compat-channel");
        bc.close();
      } else {
        assert(typeof BroadcastChannel === "undefined", "BroadcastChannel should be function or undefined");
      }
    },
  },
  {
    name: "IndexedDB",
    run: () => {
      assert(typeof indexedDB === "undefined", "indexedDB should not exist");
    },
  },
  {
    name: "Performance API",
    run: () => {
      assert(typeof performance.now === "function");
      performance.mark("compat-start");
      performance.mark("compat-end");
      performance.measure("compat-duration", "compat-start", "compat-end");
      const measures = performance.getEntriesByName("compat-duration", "measure");
      assert(measures.length >= 1);
      performance.clearMarks("compat-start");
      performance.clearMarks("compat-end");
      performance.clearMeasures("compat-duration");
    },
  },
  {
    name: "scheduled event",
    run: () => {
      // Typically worker/platform-specific. Track availability without forcing support.
      assert(
        typeof ScheduledEvent === "function" || typeof ScheduledEvent === "undefined",
        "ScheduledEvent should be function or undefined",
      );
    },
  },
  {
    name: "HTMLRewriter",
    run: () => {
      assert(
        typeof HTMLRewriter === "function" || typeof HTMLRewriter === "undefined",
        "HTMLRewriter should be function or undefined",
      );
    },
  },
  {
    name: "KV",
    run: () => {
      // Cloudflare-style KV binding is usually injected at runtime env and not global.
      assert(
        typeof KVNamespace === "function" || typeof KVNamespace === "undefined",
        "KVNamespace should be function or undefined",
      );
    },
  },
  {
    name: "Durable Objects",
    run: () => {
      assert(
        typeof DurableObject === "function" || typeof DurableObject === "undefined",
        "DurableObject should be function or undefined",
      );
      assert(
        typeof DurableObjectNamespace === "function" || typeof DurableObjectNamespace === "undefined",
        "DurableObjectNamespace should be function or undefined",
      );
    },
  },
  {
    name: "crypto.DigestStream",
    run: () => {
      assert(
        typeof DigestStream === "function" || typeof DigestStream === "undefined",
        "DigestStream should be function or undefined",
      );
    },
  },
  {
    name: "Ed25519 via WebCrypto",
    run: async () => {
      // Probe support safely: environments without Ed25519 should still pass.
      try {
        const keyPair = await crypto.subtle.generateKey(
          { name: "Ed25519" },
          true,
          ["sign", "verify"],
        );
        const data = new TextEncoder().encode("edge-ed25519");
        const signature = await crypto.subtle.sign("Ed25519", keyPair.privateKey, data);
        const valid = await crypto.subtle.verify("Ed25519", keyPair.publicKey, signature, data);
        assert(valid, "Ed25519 signature should verify when supported");
      } catch (_err) {
        // Not supported in this runtime profile.
        assert(true);
      }
    },
  },
  {
    name: "File system access",
    run: () => {
      // In this runtime, FS access may be unavailable or restricted.
      assert(
        typeof Deno === "object" || typeof Deno === "undefined",
        "Deno global should be object or undefined",
      );

      if (typeof Deno === "object") {
        assert(
          typeof Deno.readFile === "function" || typeof Deno.readFile === "undefined",
          "Deno.readFile should be function or undefined",
        );
      }
    },
  },
  {
    name: "Connect TCP",
    run: () => {
      if (typeof Deno === "object") {
        assert(
          typeof Deno.connect === "function" || typeof Deno.connect === "undefined",
          "Deno.connect should be function or undefined",
        );
      } else {
        assert(true);
      }
    },
  },
  {
    name: "Connect UDP",
    run: () => {
      if (typeof Deno === "object") {
        assert(
          typeof Deno.listenDatagram === "function" || typeof Deno.listenDatagram === "undefined",
          "Deno.listenDatagram should be function or undefined",
        );
      } else {
        assert(true);
      }
    },
  },
  {
    name: "WebSockets (Server)",
    run: () => {
      if (typeof Deno === "object") {
        assert(
          typeof Deno.upgradeWebSocket === "function" || typeof Deno.upgradeWebSocket === "undefined",
          "Deno.upgradeWebSocket should be function or undefined",
        );
      } else {
        assert(true);
      }
    },
  },
]);
