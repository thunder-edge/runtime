type NodeLikeError = Error & { code?: string };

const PROCESS_NOT_IMPLEMENTED_PREFIX = "[thunder]";
const processEnvStore = Object.create(null);

function nowMs(): number {
  const perf = (globalThis as { performance?: { now?: () => number } }).performance;
  return typeof perf?.now === "function" ? perf.now() : Date.now();
}

const processStartMs = nowMs();

function notImplemented(api: string): never {
  const err = new Error(
    `${PROCESS_NOT_IMPLEMENTED_PREFIX} ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function hostAccessDenied(api: string): never {
  const err = new Error(
    `${PROCESS_NOT_IMPLEMENTED_PREFIX} ${api} is blocked in this sandbox (no host access)`,
  ) as NodeLikeError;
  err.code = "ERR_HOST_ACCESS_DENIED";
  throw err;
}

function nowNs(): bigint {
  const perf = (globalThis as { performance?: { now?: () => number } }).performance;
  if (typeof perf?.now === "function") {
    return BigInt(Math.floor(perf.now() * 1_000_000));
  }
  return BigInt(Date.now()) * 1_000_000n;
}

const envProxy = new Proxy(processEnvStore, {
  get(target, prop: string | symbol) {
    if (prop === Symbol.toStringTag) return "Object";
    if (typeof prop !== "string") return (target as Record<string | symbol, unknown>)[prop];
    return Object.prototype.hasOwnProperty.call(target, prop) ? target[prop] : undefined;
  },
  set(target, prop: string | symbol, value) {
    if (typeof prop === "string") {
      target[prop] = String(value);
      return true;
    }
    return false;
  },
  deleteProperty(target, prop: string | symbol) {
    if (typeof prop === "string") {
      delete target[prop];
      return true;
    }
    return false;
  },
  ownKeys(target) {
    return Reflect.ownKeys(target);
  },
  getOwnPropertyDescriptor(target, prop: string | symbol) {
    if (Object.prototype.hasOwnProperty.call(target, prop)) {
      return {
        configurable: true,
        enumerable: true,
        writable: true,
        value: target[prop as string],
      };
    }
    return undefined;
  },
});

const hrtime = Object.assign(
  function hrtimeImpl(previousTime?: [number, number]): [number, number] {
    if (previousTime !== undefined) {
      if (!Array.isArray(previousTime) || previousTime.length !== 2) {
        throw new TypeError("process.hrtime previousTime must be [seconds, nanoseconds]");
      }
    }

    const nanos = nowNs();
    let seconds = Number(nanos / 1_000_000_000n);
    let nanoseconds = Number(nanos % 1_000_000_000n);

    if (previousTime) {
      seconds -= Number(previousTime[0]);
      nanoseconds -= Number(previousTime[1]);
      if (nanoseconds < 0) {
        seconds -= 1;
        nanoseconds += 1_000_000_000;
      }
    }

    return [seconds, nanoseconds];
  },
  {
    bigint: () => nowNs(),
  },
);

const versions = Object.freeze({
  node: "20.11.1",
  edgeRuntime: "1",
});

type Listener = (...args: unknown[]) => void;

function createEmitter() {
  const listeners = new Map<string, Listener[]>();

  const api = {
    on(event: string, listener: Listener) {
      const current = listeners.get(event) ?? [];
      current.push(listener);
      listeners.set(event, current);
      return api;
    },
    once(event: string, listener: Listener) {
      const wrapped: Listener = (...args: unknown[]) => {
        api.off(event, wrapped);
        listener(...args);
      };
      return api.on(event, wrapped);
    },
    off(event: string, listener: Listener) {
      const current = listeners.get(event) ?? [];
      listeners.set(
        event,
        current.filter((entry) => entry !== listener),
      );
      return api;
    },
    emit(event: string, ...args: unknown[]) {
      const current = listeners.get(event) ?? [];
      for (const listener of current) listener(...args);
      return current.length > 0;
    },
  };

  return api;
}

function createStdioWritable(kind: "stdout" | "stderr") {
  const emitter = createEmitter();
  return {
    ...emitter,
    writable: true,
    isTTY: false,
    write(chunk: unknown, _encoding?: string, cb?: (err?: unknown) => void) {
      const text = typeof chunk === "string" ? chunk : String(chunk ?? "");
      if (kind === "stdout" && typeof console?.log === "function") {
        console.log(`stdout: ${text}`);
      } else if (kind === "stderr" && typeof console?.error === "function") {
        console.error(`stderr: ${text}`);
      }
      if (typeof cb === "function") cb();
      emitter.emit("drain");
      return true;
    },
    end(chunk?: unknown, encoding?: string, cb?: () => void) {
      if (chunk !== undefined) {
        (this as { write: (value: unknown, enc?: string) => boolean }).write(chunk, encoding);
      }
      emitter.emit("finish");
      emitter.emit("close");
      if (typeof cb === "function") cb();
      return this;
    },
  };
}

function createStdinReadable() {
  const emitter = createEmitter();
  const stdin = {
    ...emitter,
    readable: true,
    isTTY: false,
    resume() {
      return stdin;
    },
    pause() {
      return stdin;
    },
  };
  queueMicrotask(() => {
    emitter.emit("end");
    emitter.emit("close");
  });
  return stdin;
}

const stdoutStream = createStdioWritable("stdout");
const stderrStream = createStdioWritable("stderr");
const stdinStream = createStdinReadable();

const processObject: Record<string | symbol, unknown> = {
  version: `v${versions.node}`,
  versions,
  platform: "linux",
  arch: "x64",
  release: Object.freeze({
    name: "node",
    lts: true,
  }),
  title: "edge-runtime",
  argv: ["edge-runtime"],
  argv0: "edge-runtime",
  execArgv: [],
  pid: 1,
  ppid: 0,
  env: envProxy,
  stdout: stdoutStream,
  stderr: stderrStream,
  stdin: stdinStream,
  cwd() {
    // Virtual bundle root; never reflects host filesystem paths.
    return "/bundle";
  },
  chdir() {
    hostAccessDenied("process.chdir");
  },
  nextTick(callback: unknown, ...args: unknown[]) {
    if (typeof callback !== "function") {
      throw new TypeError("process.nextTick callback must be a function");
    }
    queueMicrotask(() => (callback as (...cbArgs: unknown[]) => void)(...args));
  },
  hrtime,
  uptime() {
    const now = nowMs();
    return Math.max(0, (now - processStartMs) / 1000);
  },
  emitWarning(message: unknown) {
    if (typeof console?.warn === "function") {
      console.warn(String(message));
    }
  },
  memoryUsage() {
    return {
      rss: 0,
      heapTotal: 0,
      heapUsed: 0,
      external: 0,
      arrayBuffers: 0,
    };
  },
  cpuUsage() {
    notImplemented("process.cpuUsage");
  },
  getActiveResourcesInfo() {
    notImplemented("process.getActiveResourcesInfo");
  },
  kill() {
    notImplemented("process.kill");
  },
  binding() {
    notImplemented("process.binding");
  },
  dlopen() {
    notImplemented("process.dlopen");
  },
  exit() {
    notImplemented("process.exit");
  },
  abort() {
    notImplemented("process.abort");
  },
  on() {
    return processObject;
  },
  off() {
    return processObject;
  },
  once() {
    return processObject;
  },
  addListener() {
    return processObject;
  },
  removeListener() {
    return processObject;
  },
  removeAllListeners() {
    return processObject;
  },
  emit() {
    return false;
  },
};

Object.defineProperty(processObject, Symbol.toStringTag, {
  value: "process",
  configurable: true,
});

try {
  Object.defineProperty(globalThis, "process", {
    value: processObject,
    writable: false,
    configurable: false,
    enumerable: true,
  });
} catch {
  // Fallback for environments where process is already defined as non-configurable.
  try {
    (globalThis as Record<string, unknown>).process = processObject;
  } catch {
    // Keep module exports safe even if global assignment cannot be replaced.
  }
}

export default processObject;

export const version = processObject.version;
export const platform = processObject.platform;
export const arch = processObject.arch;
export const env = processObject.env;
export const stdout = processObject.stdout as {
  write: (chunk: unknown, encoding?: string, cb?: (err?: unknown) => void) => boolean;
  isTTY?: boolean;
};
export const stderr = processObject.stderr as {
  write: (chunk: unknown, encoding?: string, cb?: (err?: unknown) => void) => boolean;
  isTTY?: boolean;
};
export const stdin = processObject.stdin as {
  on: (event: string, listener: (...args: unknown[]) => void) => unknown;
  isTTY?: boolean;
};
export const cwd = processObject.cwd as () => string;
export const chdir = processObject.chdir as (...args: unknown[]) => never;
export const nextTick = processObject.nextTick as (...args: unknown[]) => void;
export const uptime = processObject.uptime as () => number;
export const emitWarning = processObject.emitWarning as (message: unknown) => void;
export const memoryUsage = processObject.memoryUsage as () => {
  rss: number;
  heapTotal: number;
  heapUsed: number;
  external: number;
  arrayBuffers: number;
};
export const cpuUsage = processObject.cpuUsage as (...args: unknown[]) => never;
export const getActiveResourcesInfo = processObject.getActiveResourcesInfo as (
  ...args: unknown[]
) => never;
export const exit = processObject.exit as (...args: unknown[]) => never;
export const abort = processObject.abort as (...args: unknown[]) => never;
export const kill = processObject.kill as (...args: unknown[]) => never;
export const binding = processObject.binding as (...args: unknown[]) => never;
export const dlopen = processObject.dlopen as (...args: unknown[]) => never;
export { hrtime, versions };
