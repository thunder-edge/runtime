const kCustomInspect = Symbol.for("nodejs.util.inspect.custom");
const kPromisifyCustom = Symbol.for("nodejs.util.promisify.custom");

const TOKEN_RE = /^[!#$%&'*+\-.^_`|~0-9a-zA-Z]+$/;

function assertToken(value: string, field: string): string {
  const token = String(value ?? "").trim().toLowerCase();
  if (!token || !TOKEN_RE.test(token)) {
    throw new TypeError(`Invalid MIME ${field}: '${value}'`);
  }
  return token;
}

function parseMime(input: string): { type: string; subtype: string; params: Array<[string, string]> } {
  const raw = String(input ?? "").trim();
  if (!raw) throw new TypeError("MIMEType input must be a non-empty string");

  const [essencePart, ...paramParts] = raw.split(";");
  const essence = essencePart.trim().toLowerCase();
  const slashIdx = essence.indexOf("/");
  if (slashIdx <= 0 || slashIdx === essence.length - 1) {
    throw new TypeError(`Invalid MIME type: '${input}'`);
  }

  const type = assertToken(essence.slice(0, slashIdx), "type");
  const subtype = assertToken(essence.slice(slashIdx + 1), "subtype");

  const params: Array<[string, string]> = [];
  for (const part of paramParts) {
    const trimmed = part.trim();
    if (!trimmed) continue;
    const eqIdx = trimmed.indexOf("=");
    const keyRaw = eqIdx >= 0 ? trimmed.slice(0, eqIdx) : trimmed;
    const valueRaw = eqIdx >= 0 ? trimmed.slice(eqIdx + 1) : "";
    const key = assertToken(keyRaw, "parameter name");
    const value = valueRaw.trim().replace(/^"|"$/g, "");
    params.push([key, value]);
  }

  return { type, subtype, params };
}

class MIMEParams {
  #map = new Map<string, string>();

  constructor(initial?: Array<[string, string]>) {
    for (const [k, v] of initial ?? []) this.set(k, v);
  }

  get(name: string): string | null {
    const key = assertToken(name, "parameter name");
    return this.#map.has(key) ? this.#map.get(key)! : null;
  }

  has(name: string): boolean {
    const key = assertToken(name, "parameter name");
    return this.#map.has(key);
  }

  set(name: string, value: string): void {
    const key = assertToken(name, "parameter name");
    this.#map.set(key, String(value ?? ""));
  }

  delete(name: string): boolean {
    const key = assertToken(name, "parameter name");
    return this.#map.delete(key);
  }

  entries(): IterableIterator<[string, string]> {
    return this.#map.entries();
  }

  keys(): IterableIterator<string> {
    return this.#map.keys();
  }

  values(): IterableIterator<string> {
    return this.#map.values();
  }

  forEach(callback: (value: string, key: string, parent: MIMEParams) => void, thisArg?: unknown): void {
    for (const [key, value] of this.#map.entries()) {
      callback.call(thisArg, value, key, this);
    }
  }

  [Symbol.iterator](): IterableIterator<[string, string]> {
    return this.entries();
  }

  toString(): string {
    return Array.from(this.#map.entries())
      .map(([k, v]) => `${k}=${v}`)
      .join(";");
  }
}

class MIMEType {
  #type: string;
  #subtype: string;
  #params: MIMEParams;

  constructor(input: string) {
    const parsed = parseMime(input);
    this.#type = parsed.type;
    this.#subtype = parsed.subtype;
    this.#params = new MIMEParams(parsed.params);
  }

  get type(): string {
    return this.#type;
  }

  set type(value: string) {
    this.#type = assertToken(value, "type");
  }

  get subtype(): string {
    return this.#subtype;
  }

  set subtype(value: string) {
    this.#subtype = assertToken(value, "subtype");
  }

  get essence(): string {
    return `${this.#type}/${this.#subtype}`;
  }

  get params(): MIMEParams {
    return this.#params;
  }

  toString(): string {
    const params = Array.from(this.#params.entries())
      .map(([k, v]) => `${k}=${v}`)
      .join("; ");
    return params ? `${this.essence}; ${params}` : this.essence;
  }
}

function formatValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "bigint" || typeof value === "boolean") {
    return String(value);
  }
  if (typeof value === "undefined") return "undefined";
  if (value === null) return "null";
  if (typeof value === "symbol") return value.toString();
  if (typeof value === "function") return `[Function: ${value.name || "anonymous"}]`;

  try {
    return JSON.stringify(value);
  } catch {
    return Object.prototype.toString.call(value);
  }
}

function format(fmt: unknown, ...args: unknown[]): string {
  if (typeof fmt !== "string") {
    return [fmt, ...args].map(formatValue).join(" ");
  }

  let argIndex = 0;
  const out = fmt.replace(/%[sdifoOj%]/g, (token) => {
    if (token === "%%") return "%";
    const value = args[argIndex++];
    switch (token) {
      case "%s":
        return String(value);
      case "%d":
      case "%i":
        return Number(value).toString();
      case "%f":
        return Number(value).toString();
      case "%o":
      case "%O":
      case "%j":
        return formatValue(value);
      default:
        return token;
    }
  });

  if (argIndex < args.length) {
    return `${out} ${args.slice(argIndex).map(formatValue).join(" ")}`;
  }

  return out;
}

function inspect(value: unknown): string {
  if (value && typeof value === "object") {
    const maybeCustom = (value as Record<PropertyKey, unknown>)[kCustomInspect];
    if (typeof maybeCustom === "function") {
      try {
        return String((maybeCustom as (...args: unknown[]) => unknown).call(value));
      } catch {
        // Fall back to default formatting.
      }
    }
  }

  return formatValue(value);
}

function inherits(ctor: Function, superCtor: Function) {
  if (typeof ctor !== "function" || typeof superCtor !== "function") {
    throw new TypeError("inherits expects constructor functions");
  }
  Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
  Object.setPrototypeOf(ctor, superCtor);
}

function deprecate<T extends (...args: unknown[]) => unknown>(
  fn: T,
  _message: string,
): T {
  return fn;
}

function callbackify(fn: (...args: unknown[]) => Promise<unknown>) {
  return (...args: unknown[]) => {
    const cb = args.pop();
    if (typeof cb !== "function") {
      throw new TypeError("The last argument must be a callback function");
    }
    Promise.resolve(fn(...args)).then(
      (value) => (cb as (err: unknown, value?: unknown) => void)(null, value),
      (err) => (cb as (err: unknown) => void)(err),
    );
  };
}

function promisify(fn: (...args: unknown[]) => unknown) {
  const custom = (fn as Record<PropertyKey, unknown>)[kPromisifyCustom];
  if (typeof custom === "function") {
    return custom;
  }

  return (...args: unknown[]) =>
    new Promise((resolve, reject) => {
      fn(...args, (err: unknown, value: unknown) => {
        if (err) {
          reject(err);
          return;
        }
        resolve(value);
      });
    });
}

(promisify as Record<PropertyKey, unknown>).custom = kPromisifyCustom;

function isObjectLike(value: unknown) {
  return typeof value === "object" && value !== null;
}

const types = {
  isArrayBufferView(value: unknown) {
    return ArrayBuffer.isView(value);
  },
  isUint8Array(value: unknown) {
    return value instanceof Uint8Array;
  },
  isDate(value: unknown) {
    return value instanceof Date;
  },
  isRegExp(value: unknown) {
    return value instanceof RegExp;
  },
  isPromise(value: unknown) {
    return isObjectLike(value) && typeof (value as Promise<unknown>).then === "function";
  },
  isNativeError(value: unknown) {
    return value instanceof Error;
  },
};

const utilModule = {
  format,
  inspect,
  inherits,
  deprecate,
  callbackify,
  promisify,
  MIMEType,
  MIMEParams,
  types,
};

export {
  format,
  inspect,
  inherits,
  deprecate,
  callbackify,
  promisify,
  MIMEType,
  MIMEParams,
  types,
};

export default utilModule;
