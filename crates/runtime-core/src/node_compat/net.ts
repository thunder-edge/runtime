import { EventEmitter } from "node:events";

type NodeLikeError = Error & { code?: string };

type NetConnectOptions = {
  host?: string;
  port?: number;
};

type DenoConnLike = {
  read: (p: Uint8Array) => Promise<number | null>;
  write: (p: Uint8Array) => Promise<number>;
  close: () => void;
};

type DenoLike = {
  connect?: (opts: { hostname: string; port: number; transport?: "tcp" }) => Promise<DenoConnLike>;
};

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function connectionError(api: string, message: string): never {
  const err = new Error(`[edge-runtime] ${api} failed: ${message}`) as NodeLikeError;
  err.code = "ECONNREFUSED";
  throw err;
}

function parseConnectArgs(args: unknown[]): { options: NetConnectOptions; cb?: () => void } {
  let cb: (() => void) | undefined;
  if (typeof args[args.length - 1] === "function") {
    cb = args.pop() as () => void;
  }

  if (typeof args[0] === "object" && args[0] !== null) {
    const opts = args[0] as NetConnectOptions;
    return { options: { host: opts.host ?? "127.0.0.1", port: Number(opts.port) }, cb };
  }

  const port = Number(args[0]);
  const host = typeof args[1] === "string" ? args[1] : "127.0.0.1";
  return { options: { host, port }, cb };
}

function toBuffer(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new TextEncoder().encode(String(data ?? ""));
}

class Socket extends EventEmitter {
  #conn?: DenoConnLike;
  destroyed = false;
  connecting = false;

  connect(...args: unknown[]): this {
    const { options, cb } = parseConnectArgs([...args]);
    const host = options.host ?? "127.0.0.1";
    const port = Number(options.port);

    if (!Number.isInteger(port) || port <= 0 || port > 65535) {
      connectionError("net.connect", "invalid port");
    }

    const denoLike = globalThis as unknown as { Deno?: DenoLike };
    if (typeof denoLike.Deno?.connect !== "function") {
      notImplemented("net.connect (requires Deno.connect)");
    }

    this.connecting = true;
    void denoLike.Deno.connect({ hostname: host, port, transport: "tcp" })
      .then((conn) => {
        this.#conn = conn;
        this.connecting = false;
        if (cb) cb();
        this.emit("connect");
        this.emit("ready");
        void this.#pumpReads();
      })
      .catch((err) => {
        this.connecting = false;
        this.emit("error", err);
        this.emit("close");
      });

    return this;
  }

  async #pumpReads(): Promise<void> {
    const conn = this.#conn;
    if (!conn) return;

    const buf = new Uint8Array(8 * 1024);
    try {
      while (!this.destroyed) {
        const n = await conn.read(buf);
        if (n === null) {
          this.emit("end");
          this.emit("close");
          break;
        }
        if (n > 0) {
          this.emit("data", buf.slice(0, n));
        }
      }
    } catch (err) {
      if (!this.destroyed) this.emit("error", err);
    }
  }

  write(data: unknown, cb?: (err?: unknown) => void): boolean {
    const conn = this.#conn;
    if (!conn) {
      if (cb) cb(new Error("socket is not connected"));
      this.emit("error", new Error("socket is not connected"));
      return false;
    }

    void conn.write(toBuffer(data))
      .then(() => cb?.())
      .catch((err) => {
        cb?.(err);
        this.emit("error", err);
      });
    return true;
  }

  end(data?: unknown, cb?: () => void): this {
    if (data !== undefined) this.write(data);
    this.destroy();
    if (cb) cb();
    return this;
  }

  destroy(error?: unknown): this {
    this.destroyed = true;
    try {
      this.#conn?.close();
    } catch {
      // Ignore close errors in best-effort shutdown.
    }
    if (error !== undefined) this.emit("error", error);
    this.emit("close");
    return this;
  }

  setNoDelay(_noDelay = true): this {
    return this;
  }

  setKeepAlive(_enable = false, _initialDelay = 0): this {
    return this;
  }

  setTimeout(_timeout: number, cb?: () => void): this {
    if (cb) cb();
    return this;
  }
}

class Server extends EventEmitter {
  listen(): never {
    return notImplemented("net.Server.listen");
  }

  close(cb?: () => void): this {
    if (typeof cb === "function") cb();
    this.emit("close");
    return this;
  }
}

function createServer(): Server {
  return new Server();
}

function connect(...args: unknown[]): Socket {
  const maybeBridge = globalThis as unknown as {
    __edgeRuntime?: { consumeEgressToken?: (kind: string, target: string) => void };
  };
  const parsed = parseConnectArgs([...args]);
  const host = parsed.options.host ?? "127.0.0.1";
  const port = Number(parsed.options.port);
  maybeBridge.__edgeRuntime?.consumeEgressToken?.("tcp", `${host}:${port}`);

  return new Socket().connect(...args);
}

const createConnection = connect;

const netModule = { Socket, Server, createServer, connect, createConnection };

export { Socket, Server, createServer, connect, createConnection };
export default netModule;
