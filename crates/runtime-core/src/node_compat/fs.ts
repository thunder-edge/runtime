import { __edgeWrapNodeCallback } from "node:async_hooks";
import { Readable, Writable } from "node:stream";

type FsError = Error & {
  code: string;
  errno: number;
  syscall?: string;
  path?: string;
};

type VfsConfig = {
  totalQuotaBytes: number;
  maxFileBytes: number;
};

type Bytes = Uint8Array<ArrayBufferLike>;

type VfsState = {
  files: Map<string, Bytes>;
  dirs: Set<string>;
  usedBytes: number;
  config: VfsConfig;
};

type NodeStats = {
  size: number;
  mode: number;
  mtimeMs: number;
  ctimeMs: number;
  birthtimeMs: number;
  isFile: () => boolean;
  isDirectory: () => boolean;
};

const DEFAULT_TOTAL_QUOTA_BYTES = 10 * 1024 * 1024;
const DEFAULT_MAX_FILE_BYTES = 5 * 1024 * 1024;

const constants = Object.freeze({
  F_OK: 0,
  R_OK: 4,
  W_OK: 2,
  X_OK: 1,
});

function getRuntimeVfsConfig(): VfsConfig {
  const raw = (globalThis as { __edgeRuntimeVfsConfig?: Partial<VfsConfig> }).__edgeRuntimeVfsConfig;
  const totalQuotaBytes = Math.max(0, Number(raw?.totalQuotaBytes ?? DEFAULT_TOTAL_QUOTA_BYTES));
  const maxFileBytes = Math.max(0, Number(raw?.maxFileBytes ?? DEFAULT_MAX_FILE_BYTES));
  return {
    totalQuotaBytes,
    maxFileBytes: Math.min(maxFileBytes, totalQuotaBytes || maxFileBytes),
  };
}

function getVfsState(): VfsState {
  const g = globalThis as unknown as { __edgeVfsState?: VfsState };
  if (!g.__edgeVfsState) {
    g.__edgeVfsState = {
      files: new Map<string, Uint8Array>([["/dev/null", new Uint8Array()]]),
      dirs: new Set<string>(["/", "/bundle", "/tmp", "/dev"]),
      usedBytes: 0,
      config: getRuntimeVfsConfig(),
    };
  }
  return g.__edgeVfsState;
}

function fsError(code: string, errno: number, syscall: string, path?: string, message?: string): never {
  const err = new Error(
    message ?? `[edge-runtime] ${syscall} failed for '${path ?? ""}' (${code})`,
  ) as FsError;
  err.name = "Error";
  err.code = code;
  err.errno = errno;
  err.syscall = syscall;
  if (path !== undefined) err.path = path;
  throw err;
}

function normalizePath(path: unknown): string {
  const raw = path instanceof URL ? path.toString() : String(path);
  const asString = raw.startsWith("file://") ? new URL(raw).pathname : raw;
  const parts = asString.replace(/\\+/g, "/").split("/").filter(Boolean);
  const stack: string[] = [];
  for (const part of parts) {
    if (part === ".") continue;
    if (part === "..") {
      if (stack.length > 0) stack.pop();
      continue;
    }
    stack.push(part);
  }
  return `/${stack.join("/")}`.replace(/\/+$/, "") || "/";
}

function parentDir(path: string): string {
  if (path === "/") return "/";
  const idx = path.lastIndexOf("/");
  return idx <= 0 ? "/" : path.slice(0, idx);
}

function isTmpPath(path: string): boolean {
  return path === "/tmp" || path.startsWith("/tmp/");
}

function isBundlePath(path: string): boolean {
  return path === "/bundle" || path.startsWith("/bundle/");
}

function isDevNull(path: string): boolean {
  return path === "/dev/null";
}

function isAllowedWritablePath(path: string): boolean {
  return isTmpPath(path) || isDevNull(path);
}

function toBytes(data: unknown): Bytes {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new TextEncoder().encode(String(data ?? ""));
}

function readEncoding(options?: unknown): string | undefined {
  if (!options) return undefined;
  if (typeof options === "string") return options;
  if (typeof options === "object") {
    const maybe = options as { encoding?: string | null };
    if (typeof maybe.encoding === "string") return maybe.encoding;
  }
  return undefined;
}

function decodeOutput(bytes: Bytes, options?: unknown): unknown {
  const encoding = readEncoding(options);
  if (!encoding || encoding === "buffer") {
    const BufferCtor = (globalThis as unknown as { Buffer?: { from: (b: Uint8Array) => unknown } }).Buffer;
    return BufferCtor?.from ? BufferCtor.from(bytes) : bytes;
  }
  return new TextDecoder(encoding === "utf8" ? "utf-8" : encoding).decode(bytes);
}

function statFrom(path: string, isDir: boolean, size: number): NodeStats {
  const now = Date.now();
  return {
    size,
    mode: isDir ? 0o040000 : 0o100000,
    mtimeMs: now,
    ctimeMs: now,
    birthtimeMs: now,
    isFile: () => !isDir,
    isDirectory: () => isDir,
  };
}

function assertPathExists(path: string, syscall: string): void {
  const state = getVfsState();
  if (state.files.has(path) || state.dirs.has(path)) return;
  fsError("ENOENT", 2, syscall, path, `[edge-runtime] ${syscall} '${path}': no such file or directory`);
}

function existsSync(path: unknown): boolean {
  const p = normalizePath(path);
  const state = getVfsState();
  return state.files.has(p) || state.dirs.has(p);
}

function accessSync(path: unknown): void {
  assertPathExists(normalizePath(path), "access");
}

function readFileBytes(path: unknown, syscall: string): Bytes {
  const p = normalizePath(path);
  const state = getVfsState();
  if (state.dirs.has(p)) {
    fsError("EISDIR", 21, syscall, p, `[edge-runtime] ${syscall} '${p}': illegal operation on a directory`);
  }
  if (isDevNull(p)) {
    return new Uint8Array();
  }
  const value = state.files.get(p);
  if (!value) {
    fsError("ENOENT", 2, syscall, p, `[edge-runtime] ${syscall} '${p}': no such file`);
  }
  return value;
}

function writeFileBytes(path: unknown, data: Bytes, syscall: string): void {
  const p = normalizePath(path);
  if (!isAllowedWritablePath(p)) {
    if (isBundlePath(p)) {
      fsError("EROFS", 30, syscall, p, `[edge-runtime] ${syscall} '${p}': read-only mount (/bundle)`);
    }
    fsError("EOPNOTSUPP", 95, syscall, p, `[edge-runtime] ${syscall} '${p}': path is outside writable VFS mounts`);
  }

  if (isDevNull(p)) return;

  const state = getVfsState();
  const parent = parentDir(p);
  if (!state.dirs.has(parent)) {
    fsError("ENOENT", 2, syscall, p, `[edge-runtime] ${syscall} '${p}': parent directory does not exist`);
  }

  if (data.byteLength > state.config.maxFileBytes) {
    fsError("ENOSPC", 28, syscall, p, `[edge-runtime] ${syscall} '${p}': exceeds VFS per-file quota (${state.config.maxFileBytes} bytes)`);
  }

  const current = state.files.get(p);
  const projected = state.usedBytes - (current?.byteLength ?? 0) + data.byteLength;
  if (projected > state.config.totalQuotaBytes) {
    fsError("ENOSPC", 28, syscall, p, `[edge-runtime] ${syscall} '${p}': exceeds VFS total quota (${state.config.totalQuotaBytes} bytes)`);
  }

  state.files.set(p, data);
  state.usedBytes = projected;
}

function readFileSync(path: unknown, options?: unknown): unknown {
  return decodeOutput(readFileBytes(path, "readFile"), options);
}

function writeFileSync(path: unknown, data?: unknown): void {
  writeFileBytes(path, toBytes(data), "writeFile");
}

function concatBytes(left: Bytes, right: Bytes): Bytes {
  const merged = new Uint8Array(left.byteLength + right.byteLength);
  merged.set(left, 0);
  merged.set(right, left.byteLength);
  return merged;
}

function readStreamError(message: string, path?: string): never {
  fsError("EINVAL", 22, "createReadStream", path, message);
}

function parseHighWaterMark(value: unknown, syscall: string): number {
  if (value === undefined || value === null) return 64 * 1024;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    fsError("EINVAL", 22, syscall, undefined, `[edge-runtime] ${syscall}: invalid highWaterMark option`);
  }
  return Math.trunc(parsed);
}

function mkdirSync(path: unknown, options?: unknown): void {
  const p = normalizePath(path);
  if (!isTmpPath(p)) {
    if (isBundlePath(p)) {
      fsError("EROFS", 30, "mkdir", p, `[edge-runtime] mkdir '${p}': read-only mount (/bundle)`);
    }
    fsError("EOPNOTSUPP", 95, "mkdir", p, `[edge-runtime] mkdir '${p}': only /tmp is writable`);
  }

  const state = getVfsState();
  const recursive = Boolean((options as { recursive?: boolean } | undefined)?.recursive);

  if (state.dirs.has(p)) return;
  if (!recursive) {
    const parent = parentDir(p);
    if (!state.dirs.has(parent)) {
      fsError("ENOENT", 2, "mkdir", p, `[edge-runtime] mkdir '${p}': parent directory does not exist`);
    }
    state.dirs.add(p);
    return;
  }

  const parts = p.split("/").filter(Boolean);
  let cursor = "";
  for (const part of parts) {
    cursor += `/${part}`;
    if (!isTmpPath(cursor) && cursor !== "/") continue;
    state.dirs.add(cursor);
  }
}

function statSync(path: unknown): NodeStats {
  const p = normalizePath(path);
  const state = getVfsState();
  if (state.dirs.has(p)) return statFrom(p, true, 0);
  const value = state.files.get(p);
  if (value) return statFrom(p, false, value.byteLength);
  fsError("ENOENT", 2, "stat", p, `[edge-runtime] stat '${p}': no such file or directory`);
}

function lstatSync(path: unknown): NodeStats {
  return statSync(path);
}

function readdirSync(path: unknown): string[] {
  const p = normalizePath(path);
  const state = getVfsState();
  if (!state.dirs.has(p)) {
    fsError("ENOTDIR", 20, "readdir", p, `[edge-runtime] readdir '${p}': not a directory`);
  }

  const prefix = p === "/" ? "/" : `${p}/`;
  const names = new Set<string>();

  for (const dir of state.dirs) {
    if (!dir.startsWith(prefix) || dir === p) continue;
    const rest = dir.slice(prefix.length);
    if (!rest) continue;
    const [name] = rest.split("/");
    if (name) names.add(name);
  }

  for (const file of state.files.keys()) {
    if (!file.startsWith(prefix)) continue;
    const rest = file.slice(prefix.length);
    if (!rest) continue;
    const [name] = rest.split("/");
    if (name) names.add(name);
  }

  return [...names].sort();
}

function parseStart(value: unknown, path: string): number {
  if (value === undefined || value === null) return 0;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    readStreamError("[edge-runtime] createReadStream: invalid start option", path);
  }
  return Math.trunc(parsed);
}

function parseEnd(value: unknown, path: string): number | undefined {
  if (value === undefined || value === null) return undefined;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    readStreamError("[edge-runtime] createReadStream: invalid end option", path);
  }
  return Math.trunc(parsed);
}

function createReadStream(path: unknown, options?: Record<string, unknown>) {
  const p = normalizePath(path);
  const start = parseStart(options?.start, p);
  const end = parseEnd(options?.end, p);
  const highWaterMark = parseHighWaterMark(options?.highWaterMark, "createReadStream");
  const encoding = readEncoding(options);

  if (end !== undefined && end < start) {
    readStreamError("[edge-runtime] createReadStream: end must be >= start", p);
  }

  const stream = new Readable({ highWaterMark }) as Readable & {
    path?: string;
    bytesRead?: number;
  };
  stream.path = p;
  stream.bytesRead = 0;

  queueMicrotask(() => {
    try {
      const source = readFileBytes(p, "createReadStream");
      if (start >= source.byteLength) {
        stream.push(null);
        return;
      }

      const stop = Math.min(end ?? (source.byteLength - 1), source.byteLength - 1);
      let cursor = start;
      while (cursor <= stop) {
        const next = Math.min(cursor + highWaterMark, stop + 1);
        const chunk = source.slice(cursor, next);
        cursor = next;
        stream.bytesRead = (stream.bytesRead ?? 0) + chunk.byteLength;
        if (encoding && encoding !== "buffer") {
          stream.push(decodeOutput(chunk, { encoding }));
        } else {
          stream.push(decodeOutput(chunk, "buffer"));
        }
      }

      stream.push(null);
    } catch (err) {
      stream.destroy(err);
    }
  });

  return stream;
}

function createWriteStream(path: unknown, options?: Record<string, unknown>) {
  const p = normalizePath(path);
  const flags = String(options?.flags ?? "w");
  const highWaterMark = parseHighWaterMark(options?.highWaterMark, "createWriteStream");

  let content = new Uint8Array();
  let ready = false;

  const init = () => {
    if (ready) return;
    ready = true;

    if (!flags.startsWith("w") && !flags.startsWith("a")) {
      fsError(
        "EINVAL",
        22,
        "createWriteStream",
        p,
        `[edge-runtime] createWriteStream '${p}': unsupported flags '${flags}'`,
      );
    }

    if (flags.startsWith("a") && existsSync(p)) {
      content = readFileBytes(p, "createWriteStream");
      return;
    }

    if (flags.startsWith("w")) {
      content = new Uint8Array();
      writeFileBytes(p, content, "createWriteStream");
    }
  };

  const stream = new Writable({
    highWaterMark,
    write(chunk: unknown, _encoding: string, cb: (err?: unknown) => void) {
      try {
        init();
        content = concatBytes(content, toBytes(chunk));
        writeFileBytes(p, content, "createWriteStream");
        cb();
      } catch (err) {
        cb(err);
      }
    },
  }) as Writable & { path?: string; bytesWritten?: number };

  stream.path = p;
  stream.bytesWritten = 0;

  const originalWrite = stream.write.bind(stream);
  stream.write = ((chunk: unknown, encodingOrCb?: unknown, maybeCb?: unknown) => {
    stream.bytesWritten = (stream.bytesWritten ?? 0) + toBytes(chunk).byteLength;
    return originalWrite(
      chunk,
      encodingOrCb as string | ((err?: unknown) => void),
      maybeCb as ((err?: unknown) => void) | undefined,
    );
  }) as typeof stream.write;

  queueMicrotask(() => {
    try {
      init();
    } catch (err) {
      stream.destroy(err);
    }
  });

  return stream;
}

function watch(path: unknown): never {
  fsError(
    "EOPNOTSUPP",
    95,
    "watch",
    normalizePath(path),
    "[thunder] fs.watch is not implemented in this runtime profile",
  );
}

function callbackStyle<T>(fn: () => T, cb?: (...args: unknown[]) => void): void {
  const callback = typeof cb === "function" ? cb : undefined;
  if (!callback) {
    fn();
    return;
  }

  const wrappedCallback =
    typeof __edgeWrapNodeCallback === "function"
      ? __edgeWrapNodeCallback(callback, "FSCallback")
      : callback;

  try {
    const out = fn();
    wrappedCallback(null, out);
  } catch (err) {
    wrappedCallback(err);
  }
}

function readFile(path: unknown, options?: unknown, cb?: (...args: unknown[]) => void): void {
  const callback = (typeof options === "function" ? options : cb) as
    | ((...args: unknown[]) => void)
    | undefined;
  const readOptions = typeof options === "function" ? undefined : options;
  callbackStyle(() => readFileSync(path, readOptions), callback);
}

function writeFile(path: unknown, data?: unknown, options?: unknown, cb?: (...args: unknown[]) => void): void {
  const callback = (typeof options === "function" ? options : cb) as ((...args: unknown[]) => void) | undefined;
  callbackStyle(() => writeFileSync(path, data), callback);
}

function stat(path: unknown, cb?: (...args: unknown[]) => void): void {
  callbackStyle(() => statSync(path), cb);
}

function lstat(path: unknown, cb?: (...args: unknown[]) => void): void {
  callbackStyle(() => lstatSync(path), cb);
}

function readdir(path: unknown, cb?: (...args: unknown[]) => void): void {
  callbackStyle(() => readdirSync(path), cb);
}

function mkdir(path: unknown, options?: unknown, cb?: (...args: unknown[]) => void): void {
  const callback = (typeof options === "function" ? options : cb) as
    | ((...args: unknown[]) => void)
    | undefined;
  const mkdirOptions = typeof options === "function" ? undefined : options;
  callbackStyle(() => mkdirSync(path, mkdirOptions), callback);
}

const fsModule = {
  constants,
  existsSync,
  accessSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  statSync,
  lstatSync,
  readdirSync,
  createReadStream,
  createWriteStream,
  watch,
  readFile,
  writeFile,
  mkdir,
  stat,
  lstat,
  readdir,
};

export {
  constants,
  existsSync,
  accessSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  statSync,
  lstatSync,
  readdirSync,
  createReadStream,
  createWriteStream,
  watch,
  readFile,
  writeFile,
  mkdir,
  stat,
  lstat,
  readdir,
};

export default fsModule;
