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

type VfsState = {
  files: Map<string, Uint8Array>;
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

function toBytes(data: unknown): Uint8Array {
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

function decodeOutput(bytes: Uint8Array, options?: unknown): unknown {
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

function readFileSync(path: unknown, options?: unknown): unknown {
  const p = normalizePath(path);
  const state = getVfsState();
  if (state.dirs.has(p)) {
    fsError("EISDIR", 21, "readFile", p, `[edge-runtime] readFile '${p}': illegal operation on a directory`);
  }
  if (isDevNull(p)) {
    return decodeOutput(new Uint8Array(), options);
  }
  const value = state.files.get(p);
  if (!value) {
    fsError("ENOENT", 2, "readFile", p, `[edge-runtime] readFile '${p}': no such file`);
  }
  return decodeOutput(value, options);
}

function writeFileSync(path: unknown, data?: unknown): void {
  const p = normalizePath(path);
  if (!isAllowedWritablePath(p)) {
    if (isBundlePath(p)) {
      fsError("EROFS", 30, "writeFile", p, `[edge-runtime] writeFile '${p}': read-only mount (/bundle)`);
    }
    fsError("EOPNOTSUPP", 95, "writeFile", p, `[edge-runtime] writeFile '${p}': path is outside writable VFS mounts`);
  }

  if (isDevNull(p)) return;

  const state = getVfsState();
  const parent = parentDir(p);
  if (!state.dirs.has(parent)) {
    fsError("ENOENT", 2, "writeFile", p, `[edge-runtime] writeFile '${p}': parent directory does not exist`);
  }

  const next = toBytes(data);
  if (next.byteLength > state.config.maxFileBytes) {
    fsError("ENOSPC", 28, "writeFile", p, `[edge-runtime] writeFile '${p}': exceeds VFS per-file quota (${state.config.maxFileBytes} bytes)`);
  }

  const current = state.files.get(p);
  const projected = state.usedBytes - (current?.byteLength ?? 0) + next.byteLength;
  if (projected > state.config.totalQuotaBytes) {
    fsError("ENOSPC", 28, "writeFile", p, `[edge-runtime] writeFile '${p}': exceeds VFS total quota (${state.config.totalQuotaBytes} bytes)`);
  }

  state.files.set(p, next);
  state.usedBytes = projected;
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

function createReadStream(path: unknown): never {
  fsError("EOPNOTSUPP", 95, "createReadStream", normalizePath(path), "[edge-runtime] createReadStream is not implemented in VFS mode yet");
}

function createWriteStream(path: unknown): never {
  fsError("EOPNOTSUPP", 95, "createWriteStream", normalizePath(path), "[edge-runtime] createWriteStream is not implemented in VFS mode yet");
}

function watch(path: unknown): never {
  fsError("EOPNOTSUPP", 95, "watch", normalizePath(path), "[edge-runtime] watch is not implemented in VFS mode");
}

function callbackStyle<T>(fn: () => T, cb?: (...args: unknown[]) => void): void {
  const callback = typeof cb === "function" ? cb : undefined;
  if (!callback) {
    fn();
    return;
  }
  try {
    const out = fn();
    callback(null, out);
  } catch (err) {
    callback(err);
  }
}

function readFile(path: unknown, options?: unknown, cb?: (...args: unknown[]) => void): void {
  const callback = typeof options === "function" ? options : cb;
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
  const callback = typeof options === "function" ? options : cb;
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
