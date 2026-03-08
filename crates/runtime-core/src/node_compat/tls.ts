type NodeLikeError = Error & { code?: string };

type TlsConnectOptions = {
  host?: string;
  port?: number;
  servername?: string;
};

type DenoTlsConnLike = {
  read: (p: Uint8Array) => Promise<number | null>;
  write: (p: Uint8Array) => Promise<number>;
  close: () => void;
};

type DenoLike = {
  connectTls?: (opts: {
    hostname: string;
    port: number;
    certChain?: string;
    privateKey?: string;
    caCerts?: string[];
  }) => Promise<DenoTlsConnLike>;
};

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function parseConnectArgs(args: unknown[]): { options: TlsConnectOptions; cb?: () => void } {
  let cb: (() => void) | undefined;
  if (typeof args[args.length - 1] === "function") {
    cb = args.pop() as () => void;
  }

  if (typeof args[0] === "object" && args[0] !== null) {
    const opts = args[0] as TlsConnectOptions;
    return {
      options: {
        host: opts.host ?? opts.servername ?? "127.0.0.1",
        port: Number(opts.port),
        servername: opts.servername,
      },
      cb,
    };
  }

  const port = Number(args[0]);
  const host = typeof args[1] === "string" ? args[1] : "127.0.0.1";
  return { options: { host, port, servername: host }, cb };
}

function toBuffer(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new TextEncoder().encode(String(data ?? ""));
}

class TLSSocket {
  #conn?: DenoTlsConnLike;
  #listeners = new Map<string, Array<(...args: unknown[]) => void>>();
  destroyed = false;
  encrypted = true;
  authorized = true;

  constructor(conn?: DenoTlsConnLike) {
    this.#conn = conn;
  }

  on(event: string, listener: (...args: unknown[]) => void): this {
    const current = this.#listeners.get(event) ?? [];
    current.push(listener);
    this.#listeners.set(event, current);
    return this;
  }

  once(event: string, listener: (...args: unknown[]) => void): this {
    const wrapped = (...args: unknown[]) => {
      this.off(event, wrapped);
      listener(...args);
    };
    return this.on(event, wrapped);
  }

  off(event: string, listener: (...args: unknown[]) => void): this {
    const current = this.#listeners.get(event) ?? [];
    this.#listeners.set(
      event,
      current.filter((entry) => entry !== listener),
    );
    return this;
  }

  emit(event: string, ...args: unknown[]): boolean {
    const current = this.#listeners.get(event) ?? [];
    for (const listener of current) listener(...args);
    return current.length > 0;
  }

  _setConn(conn: DenoTlsConnLike) {
    this.#conn = conn;
  }

  write(data: unknown, cb?: (err?: unknown) => void): boolean {
    const conn = this.#conn;
    if (!conn) {
      cb?.(new Error("tls socket is not connected"));
      return false;
    }
    void conn.write(toBuffer(data))
      .then(() => {
        cb?.();
        this.emit("drain");
      })
      .catch((err) => {
        cb?.(err);
        this.emit("error", err);
      });
    return true;
  }

  end(data?: unknown, cb?: () => void): this {
    if (data !== undefined) this.write(data);
    this.destroy();
    cb?.();
    return this;
  }

  destroy(): this {
    this.destroyed = true;
    try {
      this.#conn?.close();
    } catch {
      // Best-effort close.
    }
    this.emit("close");
    return this;
  }
}

function createServer(): never {
  return notImplemented("tls.createServer");
}

function connect(...args: unknown[]): TLSSocket {
  const { options, cb } = parseConnectArgs([...args]);
  const host = options.host ?? "127.0.0.1";
  const port = Number(options.port);

  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    const err = new Error("[edge-runtime] tls.connect failed: invalid port") as NodeLikeError;
    err.code = "ECONNREFUSED";
    throw err;
  }

  const denoLike = globalThis as unknown as { Deno?: DenoLike };
  if (typeof denoLike.Deno?.connectTls !== "function") {
    notImplemented("tls.connect (requires Deno.connectTls)");
  }

  const socket = new TLSSocket();
  void denoLike.Deno.connectTls({ hostname: host, port })
    .then((conn) => {
      socket._setConn(conn);
      cb?.();
      socket.emit("connect");
      socket.emit("secureConnect");
    })
    .catch((err) => {
      socket.emit("error", err);
      socket.destroy();
    });

  return socket;
}

function createSecureContext(): never {
  return notImplemented("tls.createSecureContext");
}

const rootCertificates: string[] = [];

const tlsModule = {
  TLSSocket,
  createServer,
  connect,
  createSecureContext,
  rootCertificates,
};

export { TLSSocket, createServer, connect, createSecureContext, rootCertificates };
export default tlsModule;
