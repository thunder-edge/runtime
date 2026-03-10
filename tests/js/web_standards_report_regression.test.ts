import { runSuite, assert, assertEquals, assertExists } from "thunder:testing";

await runSuite("web-standards-report-regression", [
  {
    name: "URL.parse handles valid and invalid inputs",
    run: () => {
      assert(typeof URL.parse === "function", "URL.parse should exist");

      const parsed = URL.parse("https://example.com/docs?id=7");
      assertExists(parsed);
      assertEquals(parsed.hostname, "example.com");
      assertEquals(parsed.pathname, "/docs");
      assertEquals(parsed.searchParams.get("id"), "7");

      const invalid = URL.parse("https://exa mple.com");
      assertEquals(invalid, null);
    },
  },
  {
    name: "URLSearchParams supports append and repeated keys",
    run: () => {
      const params = new URLSearchParams("a=1");
      params.append("a", "2");
      params.set("b", "3");

      assertEquals(params.get("a"), "1");
      assertEquals(params.getAll("a"), ["1", "2"]);
      assertEquals(params.toString(), "a=1&a=2&b=3");
    },
  },
  {
    name: "FormData supports set/get/getAll semantics",
    run: () => {
      const form = new FormData();
      form.append("tag", "a");
      form.append("tag", "b");
      form.set("single", "x");

      assertEquals(form.get("single"), "x");
      assertEquals(form.getAll("tag"), ["a", "b"]);
      assert(form.has("tag"), "form should contain key 'tag'");
    },
  },
  {
    name: "ErrorEvent exposes message and error",
    run: () => {
      const sourceError = new TypeError("boom");
      const event = new ErrorEvent("error", {
        message: "failed",
        error: sourceError,
      });

      assertEquals(event.type, "error");
      assertEquals(event.message, "failed");
      assertEquals(event.error, sourceError);
    },
  },
  {
    name: "PromiseRejectionEvent stores reason and promise",
    run: () => {
      const promise = Promise.reject(new Error("test-rejection"));
      void promise.catch(() => {});

      const event = new PromiseRejectionEvent("unhandledrejection", {
        promise,
        reason: "test-reason",
      });

      assertEquals(event.type, "unhandledrejection");
      assertEquals(event.reason, "test-reason");
      assertEquals(event.promise, promise);
    },
  },
  {
    name: "crypto.getRandomValues fills typed arrays",
    run: () => {
      const bytes = new Uint8Array(32);
      crypto.getRandomValues(bytes);

      const sum = bytes.reduce((acc, value) => acc + value, 0);
      assert(sum > 0, "expected non-zero random payload");
    },
  },
  {
    name: "Intl formatters produce deterministic string types",
    run: () => {
      const dateText = new Intl.DateTimeFormat("en-US", { timeZone: "UTC" }).format(new Date(0));
      const numberText = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(1234.56);
      const collator = new Intl.Collator("en-US");

      assert(typeof dateText === "string" && dateText.length > 0);
      assert(typeof numberText === "string" && numberText.length > 0);
      assert(collator.compare("a", "b") < 0, "expected collator lexical ordering");
    },
  },
  {
    name: "MessageChannel transfers messages between ports",
    run: async () => {
      const channel = new MessageChannel();

      try {
        const payload = await new Promise<string>((resolve) => {
          channel.port1.onmessage = (event) => {
            resolve(String(event.data));
          };
          channel.port2.postMessage("hello-channel");
        });

        assertEquals(payload, "hello-channel");
      } finally {
        channel.port1.close();
        channel.port2.close();
      }
    },
  },
  {
    name: "FileReader reads text blobs",
    run: async () => {
      const blob = new Blob(["edge-runtime"]);
      const reader = new FileReader();

      const text = await new Promise<string>((resolve, reject) => {
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error ?? new Error("reader failed"));
        reader.readAsText(blob);
      });

      assertEquals(text, "edge-runtime");
      assertEquals(reader.readyState, FileReader.DONE);
    },
  },
]);
