# Request Reference (Deno / Web Fetch API)

This runtime uses the standard Web `Request` object (Fetch API semantics).

## Handler Signature

```ts
export default async function handler(req: Request): Promise<Response> {
  return new Response(req.method);
}
```

For object handlers:

```ts
export default {
  async GET(req: Request) {
    return new Response(req.url);
  },
};
```

## Core Properties

- `req.method`: HTTP method (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, ...)
- `req.url`: absolute URL string
- `req.headers`: `Headers` object (case-insensitive)
- `req.body`: `ReadableStream<Uint8Array> | null`
- `req.bodyUsed`: `boolean` (whether body was consumed)
- `req.signal`: `AbortSignal` for cancellation

## Additional Fetch Properties

Depending on request origin and construction, these may be present:

- `req.cache`
- `req.credentials`
- `req.destination`
- `req.integrity`
- `req.keepalive`
- `req.mode`
- `req.redirect`
- `req.referrer`
- `req.referrerPolicy`
- `req.duplex` (streaming uploads when applicable)

## URL Data (from `req.url`)

```ts
const url = new URL(req.url);

const pathname = url.pathname;          // "/api/users/42"
const search = url.search;              // "?page=2"
const origin = url.origin;              // "http://localhost:8080"
const host = url.host;                  // "localhost:8080"
const userId = pathname.split("/").at(-1);
const page = url.searchParams.get("page");
```

## Header Access

```ts
const contentType = req.headers.get("content-type");
const auth = req.headers.get("authorization");
const requestId = req.headers.get("x-request-id") ?? crypto.randomUUID();

const out = new Headers();
out.set("x-request-id", requestId);
```

## Body Readers

Request body can be consumed once. Use one of:

- `await req.text()`
- `await req.json()`
- `await req.arrayBuffer()`
- `await req.blob()`
- `await req.formData()`
- stream reader from `req.body.getReader()`

Examples:

```ts
// JSON
const payload = await req.json();

// Text
const raw = await req.text();

// FormData
const form = await req.formData();
const file = form.get("file");
```

## `bodyUsed` and `clone()`

If you need to inspect body in middleware and still pass forward, clone first.

```ts
const clone = req.clone();
const audit = await clone.text();
// req remains readable after reading clone
```

## Cookies

`Request` has no built-in cookie parser. Parse from header:

```ts
function parseCookies(req: Request): Record<string, string> {
  const raw = req.headers.get("cookie") ?? "";
  const out: Record<string, string> = {};
  for (const pair of raw.split(";")) {
    const [k, ...rest] = pair.trim().split("=");
    if (!k) continue;
    out[k] = decodeURIComponent(rest.join("=") || "");
  }
  return out;
}
```

## Method Routing Pattern

```ts
import { GenericResponse, HTTP } from "thunder:http";

export default async function handler(req: Request): Promise<Response> {
  if (req.method === "GET") {
    return GenericResponse({ ok: true }, { status: HTTP.Ok });
  }

  if (req.method === "POST") {
    const body = await req.json().catch(() => null);
    if (!body) {
      return GenericResponse({ error: "invalid_json" }, { status: HTTP.BadRequest });
    }
    return GenericResponse({ created: true, body }, { status: HTTP.Created });
  }

  return GenericResponse({ error: "method_not_supported" }, { status: HTTP.MethodNotAllowed });
}
```

## Notes

- Request semantics follow the Fetch standard used by Deno/Web APIs.
- In explicit method object handlers (`export default { GET, POST, ... }`), runtime-level `405 + Allow` handling is applied when the verb is missing.
