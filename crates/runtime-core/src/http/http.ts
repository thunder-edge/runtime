export const HTTP = Object.freeze({
  Continue: 100,
  SwitchingProtocols: 101,
  Ok: 200,
  Created: 201,
  Accepted: 202,
  NoContent: 204,
  MovedPermanently: 301,
  Found: 302,
  SeeOther: 303,
  TemporaryRedirect: 307,
  PermanentRedirect: 308,
  BadRequest: 400,
  Unauthorized: 401,
  Forbidden: 403,
  NotFound: 404,
  MethodNotAllowed: 405,
  Conflict: 409,
  UnprocessableEntity: 422,
  TooManyRequests: 429,
  InternalServerError: 500,
  NotImplemented: 501,
  BadGateway: 502,
  ServiceUnavailable: 503,
} as const);

export type HttpStatusCode = (typeof HTTP)[keyof typeof HTTP];
export type HeaderValue = string | readonly string[];
export type HeaderMap = Record<string, HeaderValue>;
export type HeadersInput = HeadersInit | HeaderMap;

export interface GenericResponseEnvelope {
  body: unknown;
  status?: number;
  headers?: HeadersInput;
}

function appendHeaderBag(target: Headers, input?: HeadersInput): void {
  if (!input) return;

  if (input instanceof Headers) {
    for (const [key, value] of input.entries()) {
      target.append(key, value);
    }
    return;
  }

  if (Array.isArray(input)) {
    for (const [key, value] of input) {
      target.append(String(key), String(value));
    }
    return;
  }

  for (const [key, value] of Object.entries(input)) {
    if (Array.isArray(value)) {
      for (const item of value) {
        target.append(key, String(item));
      }
    } else {
      target.set(key, String(value));
    }
  }
}

function cloneHeaders(headers: Headers): Headers {
  const out = new Headers();
  appendHeaderBag(out, headers);
  return out;
}

function isArrayBufferView(value: unknown): value is ArrayBufferView {
  return typeof ArrayBuffer !== "undefined" && ArrayBuffer.isView(value);
}

function isReadableStream(value: unknown): value is ReadableStream<Uint8Array> {
  return typeof ReadableStream !== "undefined" && value instanceof ReadableStream;
}

function isBlob(value: unknown): value is Blob {
  return typeof Blob !== "undefined" && value instanceof Blob;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function hasEnvelopeShape(value: unknown): value is GenericResponseEnvelope {
  return (
    isPlainObject(value) &&
    (Object.prototype.hasOwnProperty.call(value, "body") ||
      Object.prototype.hasOwnProperty.call(value, "status") ||
      Object.prototype.hasOwnProperty.call(value, "headers"))
  );
}

export class ResponseDraft<TBody = unknown> {
  public statusCode: number;
  public headers: Headers;
  public body: TBody | null;

  constructor(body: TBody | null, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    this.statusCode = statusCode;
    this.headers = new Headers();
    appendHeaderBag(this.headers, headers);
    this.body = body;
  }

  status(code: number): this {
    this.statusCode = code;
    return this;
  }

  header(key: string, value: string): this {
    this.headers.set(key, value);
    return this;
  }

  appendHeader(key: string, value: string): this {
    this.headers.append(key, value);
    return this;
  }

  withHeaders(values: HeadersInput): this {
    appendHeaderBag(this.headers, values);
    return this;
  }

  cookie(name: string, value: string, attributes = "Path=/; HttpOnly"): this {
    this.headers.append("set-cookie", `${name}=${value}; ${attributes}`);
    return this;
  }

  toResponse(): Response {
    const body = this.statusCode === HTTP.NoContent ? null : this.body;
    return new Response(body as BodyInit | null, {
      status: this.statusCode,
      headers: cloneHeaders(this.headers),
    });
  }
}

export class JsonResponseDraft<TBody = unknown> extends ResponseDraft<TBody> {
  constructor(body: TBody, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    super(body, statusCode, headers);
    if (!this.headers.has("content-type")) {
      this.headers.set("content-type", "application/json; charset=utf-8");
    }
  }

  override toResponse(): Response {
    const hasNoBody = this.statusCode === HTTP.NoContent || this.body === null;
    return new Response(hasNoBody ? null : JSON.stringify(this.body), {
      status: this.statusCode,
      headers: cloneHeaders(this.headers),
    });
  }
}

export class TextResponseDraft extends ResponseDraft<string> {
  constructor(body: string, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    super(body, statusCode, headers);
    if (!this.headers.has("content-type")) {
      this.headers.set("content-type", "text/plain; charset=utf-8");
    }
  }
}

export class HtmlResponseDraft extends ResponseDraft<string> {
  constructor(body: string, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    super(body, statusCode, headers);
    if (!this.headers.has("content-type")) {
      this.headers.set("content-type", "text/html; charset=utf-8");
    }
  }
}

export class BinaryResponseDraft extends ResponseDraft<ArrayBuffer | ArrayBufferView | Uint8Array> {
  constructor(
    body: ArrayBuffer | ArrayBufferView | Uint8Array,
    statusCode: number = HTTP.Ok,
    headers?: HeadersInput,
  ) {
    super(body, statusCode, headers);
    if (!this.headers.has("content-type")) {
      this.headers.set("content-type", "application/octet-stream");
    }
  }
}

export class StreamResponseDraft extends ResponseDraft<ReadableStream<Uint8Array>> {
  constructor(body: ReadableStream<Uint8Array>, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    super(body, statusCode, headers);
  }

  sse(): this {
    this.headers.set("content-type", "text/event-stream; charset=utf-8");
    this.headers.set("cache-control", "no-cache");
    this.headers.set("connection", "keep-alive");
    return this;
  }

  ndjson(): this {
    this.headers.set("content-type", "application/x-ndjson; charset=utf-8");
    return this;
  }
}

export class BlobResponseDraft extends ResponseDraft<Blob> {
  constructor(body: Blob, statusCode: number = HTTP.Ok, headers?: HeadersInput) {
    super(body, statusCode, headers);
    if (body.type && !this.headers.has("content-type")) {
      this.headers.set("content-type", body.type);
    }
  }

  filename(name: string, disposition: "inline" | "attachment" = "attachment"): this {
    this.headers.set("content-disposition", `${disposition}; filename="${name}"`);
    return this;
  }
}

export class FileResponseDraft extends ResponseDraft<Blob | ArrayBuffer | ArrayBufferView | ReadableStream<Uint8Array>> {
  constructor(
    body: Blob | ArrayBuffer | ArrayBufferView | ReadableStream<Uint8Array>,
    statusCode: number = HTTP.Ok,
    headers?: HeadersInput,
  ) {
    super(body, statusCode, headers);
    if (!this.headers.has("content-type")) {
      this.headers.set("content-type", "application/octet-stream");
    }
  }

  filename(name: string, disposition: "inline" | "attachment" = "attachment"): this {
    this.headers.set("content-disposition", `${disposition}; filename="${name}"`);
    return this;
  }

  inline(name?: string): this {
    if (name) {
      return this.filename(name, "inline");
    }
    this.headers.set("content-disposition", "inline");
    return this;
  }

  attachment(name?: string): this {
    if (name) {
      return this.filename(name, "attachment");
    }
    this.headers.set("content-disposition", "attachment");
    return this;
  }
}

export class RedirectResponseDraft extends ResponseDraft<null> {
  constructor(location: string, statusCode: number = HTTP.Found, headers?: HeadersInput) {
    super(null, statusCode, headers);
    this.headers.set("location", location);
  }
}

export class EmptyResponseDraft extends ResponseDraft<null> {
  constructor(statusCode: number = HTTP.NoContent, headers?: HeadersInput) {
    super(null, statusCode, headers);
  }
}

export class ErrorResponseDraft extends JsonResponseDraft<Record<string, unknown>> {
  constructor(
    error: string,
    details?: unknown,
    statusCode: number = HTTP.InternalServerError,
    headers?: HeadersInput,
  ) {
    const payload: Record<string, unknown> = { error };
    if (details !== undefined) {
      payload.details = details;
    }
    super(payload, statusCode, headers);
  }
}

export function JSONResponse<TBody>(body: TBody): JsonResponseDraft<TBody> {
  return new JsonResponseDraft(body);
}

export function TextResponse(body: string): TextResponseDraft {
  return new TextResponseDraft(body);
}

export function HTMLResponse(body: string): HtmlResponseDraft {
  return new HtmlResponseDraft(body);
}

export function BinaryResponse(body: ArrayBuffer | ArrayBufferView | Uint8Array): BinaryResponseDraft {
  return new BinaryResponseDraft(body);
}

export function StreamResponse(body: ReadableStream<Uint8Array>): StreamResponseDraft {
  return new StreamResponseDraft(body);
}

export function BlobResponse(body: Blob): BlobResponseDraft {
  return new BlobResponseDraft(body);
}

export function FileResponse(
  body: Blob | ArrayBuffer | ArrayBufferView | ReadableStream<Uint8Array>,
): FileResponseDraft {
  return new FileResponseDraft(body);
}

export function RedirectResponse(location: string): RedirectResponseDraft {
  return new RedirectResponseDraft(location);
}

export function EmptyResponse(): EmptyResponseDraft {
  return new EmptyResponseDraft();
}

export function ErrorResponse(
  error: string,
  details?: unknown,
): ErrorResponseDraft {
  return new ErrorResponseDraft(error, details);
}

export function fromGenericResponse(
  value: unknown,
  init?: { status?: number; headers?: HeadersInput },
): Response {
  let status = init?.status ?? HTTP.Ok;
  let headers = new Headers();
  appendHeaderBag(headers, init?.headers);
  let body: unknown = value;

  if (value instanceof Response) {
    return value;
  }

  if (value instanceof ResponseDraft) {
    return value.toResponse();
  }

  if (hasEnvelopeShape(value)) {
    if (typeof value.status === "number") {
      status = value.status;
    }
    appendHeaderBag(headers, value.headers);
    body = value.body;
  }

  if (body === null || body === undefined) {
    return new EmptyResponseDraft(status === HTTP.Ok ? HTTP.NoContent : status, headers).toResponse();
  }

  if (isReadableStream(body)) {
    return new StreamResponseDraft(body, status, headers).toResponse();
  }

  if (isBlob(body)) {
    return new BlobResponseDraft(body, status, headers).toResponse();
  }

  if (body instanceof ArrayBuffer || isArrayBufferView(body)) {
    return new BinaryResponseDraft(body, status, headers).toResponse();
  }

  if (typeof body === "string") {
    return new TextResponseDraft(body, status, headers).toResponse();
  }

  if (
    typeof body === "number" ||
    typeof body === "boolean" ||
    typeof body === "bigint"
  ) {
    return new TextResponseDraft(String(body), status, headers).toResponse();
  }

  return new JsonResponseDraft(body, status, headers).toResponse();
}

export const GenericResponse = fromGenericResponse;
export const AutoResponse = fromGenericResponse;
