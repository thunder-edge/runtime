# thunder:http - Response Helpers

`thunder:http` provides response helper classes and a generic auto-response adapter for `export default` handlers.

## Import

```ts
import {
  HTTP,
  JSONResponse,
  TextResponse,
  HTMLResponse,
  BinaryResponse,
  FileResponse,
  BlobResponse,
  StreamResponse,
  RedirectResponse,
  EmptyResponse,
  ErrorResponse,
  GenericResponse,
  AutoResponse,
  fromGenericResponse,
} from "thunder:http";
```

The alias `thunder:http` is resolved by Thunder CLI flows (`thunder watch`, `thunder bundle`, `thunder test`, `thunder check`).

For a complete reference of the incoming `Request` object (properties, body readers, headers, URL parsing, and `clone()` semantics), see:
- [request-reference.md](request-reference.md)

## HTTP Status Map

`HTTP` exports named status codes.

```ts
HTTP.Ok; // 200
HTTP.Created; // 201
HTTP.NoContent; // 204
HTTP.MethodNotAllowed; // 405
HTTP.InternalServerError; // 500
```

## Fluent Builders

All builders support chaining and conversion with `.toResponse()`.

```ts
return JSONResponse({ message: "foo" })
  .status(HTTP.Ok)
  .header("x-request-id", "abc")
  .toResponse();
```

Shared methods:
- `.status(code)`
- `.header(key, value)`
- `.appendHeader(key, value)`
- `.withHeaders(headers)`
- `.cookie(name, value, attributes?)`
- `.toResponse()`

Public mutable properties:
- `statusCode`
- `headers`
- `body`

Example with direct mutation:

```ts
const resp = JSONResponse({ ok: true });
resp.statusCode = HTTP.Ok;
resp.headers.set("x-env", "dev");
return resp.toResponse();
```

## Available Builders

### JSONResponse

```ts
return JSONResponse({ id: 42 })
  .status(HTTP.Created)
  .toResponse();
```

### TextResponse

```ts
return TextResponse("pong")
  .status(HTTP.Ok)
  .toResponse();
```

### HTMLResponse

```ts
return HTMLResponse("<h1>Hello</h1>")
  .status(HTTP.Ok)
  .toResponse();
```

### BinaryResponse

```ts
const bytes = new Uint8Array([1, 2, 3]);
return BinaryResponse(bytes)
  .status(HTTP.Ok)
  .toResponse();
```

### FileResponse

```ts
const bytes = new TextEncoder().encode("report");
return FileResponse(bytes)
  .attachment("report.txt")
  .status(HTTP.Ok)
  .toResponse();
```

### BlobResponse

```ts
const blob = new Blob(["hello"], { type: "text/plain" });
return BlobResponse(blob)
  .filename("hello.txt", "attachment")
  .toResponse();
```

### StreamResponse

```ts
const stream = new ReadableStream<Uint8Array>({
  start(controller) {
    controller.enqueue(new TextEncoder().encode("line-1\n"));
    controller.close();
  },
});

return StreamResponse(stream)
  .ndjson()
  .toResponse();
```

SSE helper:

```ts
return StreamResponse(stream)
  .sse()
  .toResponse();
```

### RedirectResponse

```ts
return RedirectResponse("/new-path")
  .status(HTTP.PermanentRedirect)
  .toResponse();
```

### EmptyResponse

```ts
return EmptyResponse()
  .status(HTTP.NoContent)
  .toResponse();
```

### ErrorResponse

```ts
return ErrorResponse("invalid_payload", { expected: "{ name: string }" })
  .status(HTTP.BadRequest)
  .toResponse();
```

## Generic Auto Response

For quick handlers, use `GenericResponse(value, init?)` (aliases: `AutoResponse`, `fromGenericResponse`).

```ts
export default async function handler(req: Request): Promise<Response> {
  if (req.method === "GET") {
    return GenericResponse({ ok: true }); // JSON
  }

  if (req.method === "POST") {
    return GenericResponse("accepted", { status: HTTP.Accepted }); // text/plain
  }

  return GenericResponse(null); // 204
}
```

### Inference Rules

`GenericResponse` applies these rules:

1. `Response` -> passthrough.
2. `ResponseDraft` -> calls `.toResponse()`.
3. Envelope `{ body, status?, headers? }` -> uses envelope metadata.
4. `null` / `undefined` -> empty response (default `204` when no explicit status).
5. `ReadableStream` -> stream response.
6. `Blob` -> blob response.
7. `ArrayBuffer` / typed arrays -> binary response.
8. `string` -> text response.
9. `number` / `boolean` / `bigint` -> text response (`String(value)`).
10. objects / arrays -> JSON response.

### Generic Envelope Example

```ts
return GenericResponse({
  status: HTTP.Created,
  headers: { "x-resource-id": "42" },
  body: { id: 42, name: "item" },
});
```

## Using with export default object

```ts
import { HTTP, JSONResponse } from "thunder:http";

export default {
  GET() {
    return JSONResponse({ ok: true }).status(HTTP.Ok).toResponse();
  },
  POST(req: Request) {
    return JSONResponse({ created: true }).status(HTTP.Created).toResponse();
  },
};
```

When method handlers are explicit (`export default { GET, POST, ... }`), Thunder runtime handles `405 Method Not Allowed` + `Allow` automatically when the incoming method has no match.
