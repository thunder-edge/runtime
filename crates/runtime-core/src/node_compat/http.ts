type NodeLikeError = Error & { code?: string };
type HeadersLike = Record<string, string>;
type RequestOptionsLike = {
  protocol?: string;
  hostname?: string;
  host?: string;
  port?: number | string;
  path?: string;
  method?: string;
  headers?: HeadersLike;
};

class SimpleEmitter {
  #listeners = new Map<string, Array<(...args: unknown[]) => void>>();

  on(event: string, listener: (...args: unknown[]) => void) {
    const current = this.#listeners.get(event) ?? [];
    current.push(listener);
    this.#listeners.set(event, current);
    return this;
  }

  once(event: string, listener: (...args: unknown[]) => void) {
    const wrapped = (...args: unknown[]) => {
      this.off(event, wrapped);
      listener(...args);
    };
    return this.on(event, wrapped);
  }

  off(event: string, listener: (...args: unknown[]) => void) {
    const current = this.#listeners.get(event) ?? [];
    this.#listeners.set(
      event,
      current.filter((entry) => entry !== listener),
    );
    return this;
  }

  emit(event: string, ...args: unknown[]) {
    const current = this.#listeners.get(event) ?? [];
    for (const listener of current) listener(...args);
    return current.length > 0;
  }
}

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function runtimeFetch(): typeof fetch {
  if (typeof globalThis.fetch !== "function") {
    const err = new Error("[edge-runtime] fetch is not available") as NodeLikeError;
    err.code = "ERR_NOT_IMPLEMENTED";
    throw err;
  }
  return globalThis.fetch.bind(globalThis);
}

function normalizeOptions(options?: RequestOptionsLike): RequestOptionsLike {
  return {
    protocol: options?.protocol ?? "http:",
    hostname: options?.hostname,
    host: options?.host,
    port: options?.port,
    path: options?.path ?? "/",
    method: options?.method,
    headers: options?.headers,
  };
}

function buildUrl(input: string | URL | RequestOptionsLike): string {
  if (input instanceof URL) return input.toString();
  if (typeof input === "string") {
    if (/^https?:\/\//i.test(input)) return input;
    return `http://${input}`;
  }

  const options = normalizeOptions(input);
  const host = options.hostname ?? options.host;
  if (!host) {
    throw new TypeError("http.request requires hostname/host or absolute URL");
  }
  const protocol = options.protocol ?? "http:";
  const port = options.port !== undefined ? `:${String(options.port)}` : "";
  const path = options.path ?? "/";
  return `${protocol}//${host}${port}${path}`;
}

class IncomingMessage extends SimpleEmitter {
  statusCode: number;
  statusMessage: string;
  headers: HeadersLike;
  complete = false;

  constructor(response: Response) {
    super();
    this.statusCode = response.status;
    this.statusMessage = response.statusText;
    this.headers = {};
    response.headers.forEach((value, key) => {
      this.headers[key] = value;
    });

    queueMicrotask(async () => {
      try {
        const text = await response.text();
        if (text.length > 0) this.emit("data", text);
        this.complete = true;
        this.emit("end");
      } catch (err) {
        this.emit("error", err);
      }
    });
  }

  setEncoding(_encoding: string) {
    return this;
  }
}

class ClientRequest extends SimpleEmitter {
  #url: string;
  #method: string;
  #headers: HeadersLike;
  #chunks: string[] = [];
  #ended = false;
  #callback?: (res: IncomingMessage) => void;

  constructor(url: string, method: string, headers: HeadersLike, callback?: (res: IncomingMessage) => void) {
    super();
    this.#url = url;
    this.#method = method;
    this.#headers = { ...headers };
    this.#callback = callback;
  }

  setHeader(name: string, value: string) {
    this.#headers[String(name).toLowerCase()] = String(value);
  }

  getHeader(name: string) {
    return this.#headers[String(name).toLowerCase()];
  }

  write(chunk: unknown) {
    this.#chunks.push(typeof chunk === "string" ? chunk : String(chunk ?? ""));
    return true;
  }

  end(chunk?: unknown) {
    if (this.#ended) return this;
    this.#ended = true;
    if (chunk !== undefined) this.write(chunk);

    const fetchImpl = runtimeFetch();
    const body = this.#chunks.length > 0 ? this.#chunks.join("") : undefined;

    fetchImpl(this.#url, {
      method: this.#method,
      headers: this.#headers,
      body,
    })
      .then((res) => {
        const incoming = new IncomingMessage(res);
        if (typeof this.#callback === "function") this.#callback(incoming);
        this.emit("response", incoming);
      })
      .catch((err) => {
        this.emit("error", err);
      });

    return this;
  }
}

class Agent {
  protocol = "http:";
}

function createServer(): never {
  return notImplemented("http.createServer");
}

function request(
  input: string | URL | RequestOptionsLike,
  optionsOrCb?: RequestOptionsLike | ((res: IncomingMessage) => void),
  maybeCb?: (res: IncomingMessage) => void,
): ClientRequest {
  const options = typeof optionsOrCb === "object" && optionsOrCb !== null ? optionsOrCb : undefined;
  const cb = (typeof optionsOrCb === "function" ? optionsOrCb : maybeCb) ?? undefined;

  const merged: RequestOptionsLike = typeof input === "object" && !(input instanceof URL)
    ? { ...input, ...options }
    : { ...options };

  const url = buildUrl(typeof input === "object" && !(input instanceof URL) ? merged : input);
  const method = (merged.method ?? "GET").toUpperCase();
  const headers = merged.headers ?? {};
  return new ClientRequest(url, method, headers, cb);
}

function get(
  input: string | URL | RequestOptionsLike,
  optionsOrCb?: RequestOptionsLike | ((res: IncomingMessage) => void),
  maybeCb?: (res: IncomingMessage) => void,
): ClientRequest {
  const options = typeof optionsOrCb === "object" && optionsOrCb !== null ? optionsOrCb : undefined;
  const cb = (typeof optionsOrCb === "function" ? optionsOrCb : maybeCb) ?? undefined;
  const req = request(input, { ...(options ?? {}), method: "GET" }, cb);
  req.end();
  return req;
}

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const STATUS_CODES = {
  200: "OK",
  400: "Bad Request",
  404: "Not Found",
  500: "Internal Server Error",
};

const httpModule = {
  Agent,
  IncomingMessage,
  ClientRequest,
  METHODS,
  STATUS_CODES,
  createServer,
  request,
  get,
};

export { Agent, IncomingMessage, ClientRequest, METHODS, STATUS_CODES, createServer, request, get };
export default httpModule;
