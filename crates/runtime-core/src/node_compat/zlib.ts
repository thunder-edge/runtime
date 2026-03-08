type NodeLikeError = Error & { code?: string };

const DEFAULT_MAX_OUTPUT_LENGTH = 16 * 1024 * 1024;
const DEFAULT_MAX_INPUT_LENGTH = 8 * 1024 * 1024;
const HARD_MAX_OUTPUT_LENGTH = 64 * 1024 * 1024;
const HARD_MAX_INPUT_LENGTH = 8 * 1024 * 1024;
const MAX_OUTPUT_LENGTH_FALLBACK = 0x7fff_ffff;
const DEFAULT_OPERATION_TIMEOUT_MS = 250;

type ZlibOptions = {
  maxOutputLength?: unknown;
  maxInputLength?: unknown;
  operationTimeoutMs?: unknown;
};

type ZlibFormat = "gzip" | "deflate" | "deflate-raw";
type ZlibMode = "compress" | "decompress";

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function runtimeCore(): { ops?: Record<string, (...args: unknown[]) => unknown> } {
  return (globalThis as unknown as {
    Deno?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
    __bootstrap?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
  }).Deno?.core ??
    (globalThis as unknown as {
      __bootstrap?: { core?: { ops?: Record<string, (...args: unknown[]) => unknown> } };
    }).__bootstrap?.core ?? {};
}

function toBytes(input: unknown): Uint8Array {
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (ArrayBuffer.isView(input)) {
    return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  }
  if (typeof input === "string") {
    return new TextEncoder().encode(input);
  }
  return new TextEncoder().encode(String(input ?? ""));
}

function toNodeBufferLike(bytes: Uint8Array): unknown {
  const BufferCtor = (globalThis as unknown as { Buffer?: { from: (arg: Uint8Array) => unknown } }).Buffer;
  if (BufferCtor?.from) return BufferCtor.from(bytes);
  return bytes;
}

function toUint8Array(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (Array.isArray(value)) return Uint8Array.from(value);
  return toBytes(value);
}

function runtimeZlibDefaults(): {
  maxOutputLength: number;
  maxInputLength: number;
  operationTimeoutMs: number;
} {
  const cfg = (globalThis as unknown as {
    __edgeRuntimeZlibConfig?: {
      maxOutputLength?: unknown;
      maxInputLength?: unknown;
      operationTimeoutMs?: unknown;
    };
  }).__edgeRuntimeZlibConfig;

  const maxOutputLength = Number(cfg?.maxOutputLength);
  const maxInputLength = Number(cfg?.maxInputLength);
  const operationTimeoutMs = Number(cfg?.operationTimeoutMs);

  return {
    maxOutputLength: Number.isFinite(maxOutputLength) && maxOutputLength > 0
      ? Math.floor(maxOutputLength)
      : DEFAULT_MAX_OUTPUT_LENGTH,
    maxInputLength: Number.isFinite(maxInputLength) && maxInputLength > 0
      ? Math.floor(maxInputLength)
      : DEFAULT_MAX_INPUT_LENGTH,
    operationTimeoutMs: Number.isFinite(operationTimeoutMs) && operationTimeoutMs > 0
      ? Math.floor(operationTimeoutMs)
      : DEFAULT_OPERATION_TIMEOUT_MS,
  };
}

function resolveOptions(optionsOrCb?: unknown): { maxOutputLength: number; maxInputLength: number; operationTimeoutMs: number } {
  const defaults = runtimeZlibDefaults();
  if (!optionsOrCb || typeof optionsOrCb === "function") {
    return {
      maxOutputLength: defaults.maxOutputLength,
      maxInputLength: defaults.maxInputLength,
      operationTimeoutMs: defaults.operationTimeoutMs,
    };
  }
  if (typeof optionsOrCb !== "object") {
    return {
      maxOutputLength: defaults.maxOutputLength,
      maxInputLength: defaults.maxInputLength,
      operationTimeoutMs: defaults.operationTimeoutMs,
    };
  }

  const opts = optionsOrCb as ZlibOptions;

  let maxOutputLength = defaults.maxOutputLength;
  if (opts.maxOutputLength !== undefined) {
    const max = Number(opts.maxOutputLength);
    if (!Number.isFinite(max) || max <= 0) {
      throw Object.assign(new TypeError("maxOutputLength must be a positive number"), {
        code: "ERR_INVALID_ARG_VALUE",
      });
    }
    if (max > HARD_MAX_OUTPUT_LENGTH) {
      throw Object.assign(new RangeError(`maxOutputLength exceeds hard cap (${HARD_MAX_OUTPUT_LENGTH} bytes)`), {
        code: "ERR_BUFFER_TOO_LARGE",
      });
    }
    maxOutputLength = Math.min(Math.floor(max), MAX_OUTPUT_LENGTH_FALLBACK);
  }

  let maxInputLength = defaults.maxInputLength;
  if (opts.maxInputLength !== undefined) {
    const max = Number(opts.maxInputLength);
    if (!Number.isFinite(max) || max <= 0) {
      throw Object.assign(new TypeError("maxInputLength must be a positive number"), {
        code: "ERR_INVALID_ARG_VALUE",
      });
    }
    if (max > HARD_MAX_INPUT_LENGTH) {
      throw Object.assign(new RangeError(`maxInputLength exceeds hard cap (${HARD_MAX_INPUT_LENGTH} bytes)`), {
        code: "ERR_ZLIB_INPUT_TOO_LARGE",
      });
    }
    maxInputLength = Math.floor(max);
  }

  let operationTimeoutMs = defaults.operationTimeoutMs;
  if (opts.operationTimeoutMs !== undefined) {
    const timeout = Number(opts.operationTimeoutMs);
    if (!Number.isFinite(timeout) || timeout <= 0) {
      throw Object.assign(new TypeError("operationTimeoutMs must be a positive number"), {
        code: "ERR_INVALID_ARG_VALUE",
      });
    }
    operationTimeoutMs = Math.floor(timeout);
  }

  return { maxOutputLength, maxInputLength, operationTimeoutMs };
}

function mapNativeZlibError(err: unknown): never {
  const nodeErr = err as NodeLikeError;
  const msg = String(nodeErr?.message || "");
  if (msg.includes("maxOutputLength")) {
    nodeErr.code = nodeErr.code ?? "ERR_BUFFER_TOO_LARGE";
  } else if (msg.includes("maxInputLength")) {
    nodeErr.code = nodeErr.code ?? "ERR_ZLIB_INPUT_TOO_LARGE";
  } else if (msg.includes("timeout")) {
    nodeErr.code = nodeErr.code ?? "ERR_ZLIB_OPERATION_TIMEOUT";
  } else if (msg.includes("must be greater than zero")) {
    nodeErr.code = nodeErr.code ?? "ERR_INVALID_ARG_VALUE";
  } else {
    nodeErr.code = nodeErr.code ?? "ERR_ZLIB_ERROR";
  }
  throw nodeErr;
}

function resolveMaxOutputLength(optionsOrCb?: unknown): number {
  return resolveOptions(optionsOrCb).maxOutputLength;
}

function resolveOperationTimeout(optionsOrCb?: unknown): number {
  return resolveOptions(optionsOrCb).operationTimeoutMs;
}

function resolveMaxInputLength(optionsOrCb?: unknown): number {
  return resolveOptions(optionsOrCb).maxInputLength;
}

function transformBytesSync(
  input: unknown,
  format: ZlibFormat,
  mode: ZlibMode,
  maxOutputLength: number,
  operationTimeoutMs: number,
  maxInputLength: number,
): unknown {
  const op = runtimeCore().ops?.op_edge_zlib_transform;
  if (typeof op !== "function") {
    notImplemented("zlib native transform op");
  }
  const data = toBytes(input);
  try {
    const output = op(format, mode, data, maxOutputLength, operationTimeoutMs, maxInputLength);
    return toNodeBufferLike(toUint8Array(output));
  } catch (err) {
    mapNativeZlibError(err);
  }
}

function transformBytesAsync(
  input: unknown,
  format: ZlibFormat,
  mode: ZlibMode,
  maxOutputLength: number,
  operationTimeoutMs: number,
  maxInputLength: number,
): Promise<unknown> {
  return Promise.resolve().then(() =>
    transformBytesSync(input, format, mode, maxOutputLength, operationTimeoutMs, maxInputLength)
  );
}

type ZlibCallback = (err: unknown, result?: unknown) => void;

function resolveCallback(
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): ZlibCallback | undefined {
  if (typeof optionsOrCb === "function") return optionsOrCb as ZlibCallback;
  if (typeof maybeCb === "function") return maybeCb as ZlibCallback;
  return undefined;
}

function runWithCallback(
  op: Promise<unknown>,
  cb?: ZlibCallback,
): Promise<unknown> | void {
  if (typeof cb !== "function") return op;
  void op.then((result) => cb(null, result)).catch((err) => cb(err));
  return undefined;
}

function createGzip(): never {
  return notImplemented("zlib.createGzip");
}

function createGunzip(): never {
  return notImplemented("zlib.createGunzip");
}

function gzip(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "gzip", "compress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function gunzip(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "gzip", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function deflate(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "deflate", "compress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function inflate(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "deflate", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function deflateRaw(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "deflate-raw", "compress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function inflateRaw(
  input: unknown,
  optionsOrCb?: unknown,
  maybeCb?: unknown,
): Promise<unknown> | void {
  const cb = resolveCallback(optionsOrCb, maybeCb);
  const maxOutputLength = resolveMaxOutputLength(optionsOrCb);
  const operationTimeoutMs = resolveOperationTimeout(optionsOrCb);
  const maxInputLength = resolveMaxInputLength(optionsOrCb);
  return runWithCallback(transformBytesAsync(input, "deflate-raw", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength), cb);
}

function brotliCompress(): never {
  return notImplemented("zlib.brotliCompress");
}

function brotliDecompress(): never {
  return notImplemented("zlib.brotliDecompress");
}

function gzipSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "gzip", "compress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function gunzipSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "gzip", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function deflateSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "deflate", "compress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function inflateSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "deflate", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function deflateRawSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "deflate-raw", "compress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function inflateRawSync(input: unknown, options?: unknown): unknown {
  const maxOutputLength = resolveMaxOutputLength(options);
  const operationTimeoutMs = resolveOperationTimeout(options);
  const maxInputLength = resolveMaxInputLength(options);
  return transformBytesSync(input, "deflate-raw", "decompress", maxOutputLength, operationTimeoutMs, maxInputLength);
}

function brotliCompressSync(): never {
  return notImplemented("zlib.brotliCompressSync");
}

function brotliDecompressSync(): never {
  return notImplemented("zlib.brotliDecompressSync");
}

const constants = {
  Z_NO_FLUSH: 0,
  Z_FINISH: 4,
  Z_OK: 0,
  Z_STREAM_END: 1,
};

const zlibModule = {
  createGzip,
  createGunzip,
  gzip,
  gunzip,
  deflate,
  inflate,
  deflateRaw,
  inflateRaw,
  brotliCompress,
  brotliDecompress,
  gzipSync,
  gunzipSync,
  deflateSync,
  inflateSync,
  deflateRawSync,
  inflateRawSync,
  brotliCompressSync,
  brotliDecompressSync,
  constants,
};

export {
  createGzip,
  createGunzip,
  gzip,
  gunzip,
  deflate,
  inflate,
  deflateRaw,
  inflateRaw,
  brotliCompress,
  brotliDecompress,
  gzipSync,
  gunzipSync,
  deflateSync,
  inflateSync,
  deflateRawSync,
  inflateRawSync,
  brotliCompressSync,
  brotliDecompressSync,
  constants,
};
export default zlibModule;
