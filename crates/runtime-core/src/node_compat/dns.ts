type NodeLikeError = Error & { code?: string };

type LookupAddress = { address: string; family: 4 | 6 };
type LookupOptions = {
  family?: number;
  all?: boolean;
};
type DnsConfig = {
  dohEndpoint: string;
  maxAnswers: number;
  timeoutMs: number;
};

type DnsJsonAnswer = {
  name?: string;
  type?: number;
  TTL?: number;
  data?: string;
};

type DnsJsonResponse = {
  Status?: number;
  Answer?: DnsJsonAnswer[];
};

const DNS_TYPE_ID: Record<string, number> = {
  A: 1,
  NS: 2,
  CNAME: 5,
  SOA: 6,
  PTR: 12,
  MX: 15,
  TXT: 16,
  AAAA: 28,
  SRV: 33,
  CAA: 257,
};

const DEFAULT_DOH_ENDPOINT = "https://1.1.1.1/dns-query";
const DEFAULT_MAX_ANSWERS = 16;
const DEFAULT_TIMEOUT_MS = 2000;

function dnsError(code: string, message: string): never {
  const err = new Error(message) as NodeLikeError;
  err.code = code;
  throw err;
}

function notImplemented(api: string): never {
  return dnsError("ERR_NOT_IMPLEMENTED", `[thunder] ${api} is not implemented in this runtime profile`);
}

function runtimeFetch(): typeof fetch {
  if (typeof globalThis.fetch !== "function") {
    return dnsError("ERR_NOT_IMPLEMENTED", "fetch is not available");
  }
  return globalThis.fetch.bind(globalThis);
}

function getRuntimeDnsConfig(): DnsConfig {
  const raw = (globalThis as {
    __edgeRuntimeDnsConfig?: Partial<DnsConfig>;
  }).__edgeRuntimeDnsConfig;

  const dohEndpoint = String(raw?.dohEndpoint ?? DEFAULT_DOH_ENDPOINT).trim() || DEFAULT_DOH_ENDPOINT;
  const maxAnswers = Math.max(1, Number(raw?.maxAnswers ?? DEFAULT_MAX_ANSWERS) || DEFAULT_MAX_ANSWERS);
  const timeoutMs = Math.max(1, Number(raw?.timeoutMs ?? DEFAULT_TIMEOUT_MS) || DEFAULT_TIMEOUT_MS);

  return {
    dohEndpoint,
    maxAnswers,
    timeoutMs,
  };
}

function parseLookupOptions(raw?: number | LookupOptions): LookupOptions {
  if (typeof raw === "number") return { family: raw };
  if (!raw) return {};
  return {
    family: raw.family,
    all: Boolean(raw.all),
  };
}

function normalizeType(rrtype?: string): string {
  const normalized = String(rrtype ?? "A").toUpperCase();
  if (!Object.prototype.hasOwnProperty.call(DNS_TYPE_ID, normalized)) {
    return dnsError(
      "ERR_NOT_IMPLEMENTED",
      `[thunder] dns.resolve type '${normalized}' is not implemented in this runtime profile`,
    );
  }
  return normalized;
}

function withTimeoutSignal(timeoutMs: number): { signal?: AbortSignal; cleanup: () => void } {
  const hasManagedRuntime = Boolean((globalThis as { __edgeRuntime?: unknown }).__edgeRuntime);
  if (!hasManagedRuntime) {
    return { signal: undefined, cleanup: () => {} };
  }

  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort(new Error("dns query timeout"));
  }, timeoutMs);

  return {
    signal: controller.signal,
    cleanup: () => clearTimeout(timeoutId),
  };
}

async function queryDoh(hostname: string, rrtype: string): Promise<DnsJsonAnswer[]> {
  if (typeof hostname !== "string" || hostname.trim().length === 0) {
    return dnsError("EINVAL", "dns query requires a non-empty hostname");
  }

  const cfg = getRuntimeDnsConfig();
  const endpoint = new URL(cfg.dohEndpoint);
  endpoint.searchParams.set("name", hostname);
  endpoint.searchParams.set("type", rrtype);

  const fetchImpl = runtimeFetch();
  const { signal, cleanup } = withTimeoutSignal(cfg.timeoutMs);
  try {
    const requestInit: RequestInit = {
      method: "GET",
      headers: { accept: "application/dns-json" },
    };
    if (signal) requestInit.signal = signal;

    const response = await fetchImpl(endpoint.toString(), requestInit);

    if (!response.ok) {
      return dnsError("EAI_AGAIN", `dns query failed with status ${response.status}`);
    }

    const payload = (await response.json()) as DnsJsonResponse;
    if (Number(payload.Status ?? 0) !== 0) {
      return dnsError("ENOTFOUND", `dns query failed with status code ${payload.Status}`);
    }

    const answers = Array.isArray(payload.Answer) ? payload.Answer : [];
    return answers.slice(0, cfg.maxAnswers);
  } catch (err) {
    if ((err as { name?: string } | undefined)?.name === "AbortError") {
      return dnsError("EAI_AGAIN", "dns query timed out");
    }
    throw err;
  } finally {
    cleanup();
  }
}

function asLookupAddress(answer: DnsJsonAnswer): LookupAddress | null {
  const data = String(answer.data ?? "").trim();
  if (!data) return null;
  if (answer.type === DNS_TYPE_ID.A) return { address: data, family: 4 };
  if (answer.type === DNS_TYPE_ID.AAAA) return { address: data, family: 6 };
  return null;
}

function toReversePointer(ip: string): string {
  const input = String(ip ?? "").trim();
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(input)) {
    const parts = input.split(".").map((part) => Number(part));
    if (parts.some((n) => Number.isNaN(n) || n < 0 || n > 255)) {
      return dnsError("EINVAL", `invalid IP address '${ip}'`);
    }
    return `${parts.reverse().join(".")}.in-addr.arpa`;
  }

  // Minimal IPv6 support for reverse PTR by expanding to 32 nibbles.
  if (input.includes(":")) {
    const sections = input.toLowerCase().split("::");
    if (sections.length > 2) {
      return dnsError("EINVAL", `invalid IP address '${ip}'`);
    }

    const left = sections[0] ? sections[0].split(":").filter(Boolean) : [];
    const right = sections[1] ? sections[1].split(":").filter(Boolean) : [];
    if (left.length + right.length > 8) {
      return dnsError("EINVAL", `invalid IP address '${ip}'`);
    }

    const fill = new Array(8 - left.length - right.length).fill("0");
    const full = [...left, ...fill, ...right].map((part) => part.padStart(4, "0"));
    if (full.length !== 8 || full.some((part) => !/^[0-9a-f]{4}$/.test(part))) {
      return dnsError("EINVAL", `invalid IP address '${ip}'`);
    }

    const nibbles = full.join("").split("").reverse().join(".");
    return `${nibbles}.ip6.arpa`;
  }

  return dnsError("EINVAL", `invalid IP address '${ip}'`);
}

async function lookupInternal(hostname: string, options?: number | LookupOptions): Promise<LookupAddress | LookupAddress[]> {
  const opts = parseLookupOptions(options);
  const family = Number(opts.family ?? 0);

  if (![0, 4, 6].includes(family)) {
    return dnsError("EINVAL", "dns.lookup family must be 0, 4, or 6");
  }

  const answers: LookupAddress[] = [];
  if (family === 6) {
    const aaaa = await queryDoh(hostname, "AAAA");
    answers.push(...aaaa.map(asLookupAddress).filter((x): x is LookupAddress => x !== null));
  } else if (family === 4) {
    const a = await queryDoh(hostname, "A");
    answers.push(...a.map(asLookupAddress).filter((x): x is LookupAddress => x !== null));
  } else {
    const [a, aaaa] = await Promise.all([queryDoh(hostname, "A"), queryDoh(hostname, "AAAA")]);
    answers.push(...a.map(asLookupAddress).filter((x): x is LookupAddress => x !== null));
    answers.push(...aaaa.map(asLookupAddress).filter((x): x is LookupAddress => x !== null));
  }

  if (answers.length === 0) {
    return dnsError("ENOTFOUND", `dns lookup failed for '${hostname}'`);
  }

  if (opts.all) return answers;
  return answers[0];
}

function parseResolveData(answer: DnsJsonAnswer, rrtype: string): unknown {
  const value = String(answer.data ?? "").trim();
  if (!value) return null;

  switch (rrtype) {
    case "A":
    case "AAAA":
    case "CNAME":
    case "NS":
    case "PTR":
      return value;
    case "TXT":
      return [value.replace(/^"|"$/g, "")];
    case "MX": {
      const [priority, exchange] = value.split(/\s+/, 2);
      return {
        priority: Number(priority) || 0,
        exchange: exchange ?? "",
      };
    }
    case "SRV": {
      const [priority, weight, port, name] = value.split(/\s+/, 4);
      return {
        priority: Number(priority) || 0,
        weight: Number(weight) || 0,
        port: Number(port) || 0,
        name: name ?? "",
      };
    }
    case "CAA": {
      const [flags, tag, ...rest] = value.split(/\s+/);
      return {
        critical: Number(flags) || 0,
        issue: tag?.replace(/^"|"$/g, "") ?? "",
        value: rest.join(" ").replace(/^"|"$/g, ""),
      };
    }
    default:
      return value;
  }
}

async function resolveInternal(hostname: string, rrtype?: string): Promise<unknown[]> {
  const normalizedType = normalizeType(rrtype);
  const answers = await queryDoh(hostname, normalizedType);
  return answers
    .map((answer) => parseResolveData(answer, normalizedType))
    .filter((entry) => entry !== null);
}

function settleLookupCallback(
  promise: Promise<LookupAddress | LookupAddress[]>,
  options: LookupOptions,
  cb: (...args: unknown[]) => void,
): void {
  void promise
    .then((result) => {
      if (options.all) {
        cb(null, result);
        return;
      }
      const single = result as LookupAddress;
      cb(null, single.address, single.family);
    })
    .catch((err) => cb(err));
}

function settleArrayCallback(promise: Promise<unknown[]>, cb?: (...args: unknown[]) => void): Promise<unknown[]> | void {
  if (typeof cb !== "function") return promise;
  void promise.then((res) => cb(null, res)).catch((err) => cb(err));
  return undefined;
}

function lookup(
  hostname: string,
  optionsOrCb?: number | LookupOptions | ((...args: unknown[]) => void),
  maybeCb?: (...args: unknown[]) => void,
): void {
  const cb = typeof optionsOrCb === "function" ? optionsOrCb : maybeCb;
  const options = parseLookupOptions(typeof optionsOrCb === "function" ? undefined : optionsOrCb);
  const promise = lookupInternal(hostname, options);

  if (typeof cb !== "function") {
    dnsError("EINVAL", "dns.lookup requires a callback");
  }

  settleLookupCallback(promise, options, cb);
}

function resolve(
  hostname: string,
  rrtypeOrCb?: string | ((...args: unknown[]) => void),
  maybeCb?: (...args: unknown[]) => void,
): void | Promise<unknown[]> {
  const rrtype = typeof rrtypeOrCb === "string" ? rrtypeOrCb : undefined;
  const cb = typeof rrtypeOrCb === "function" ? rrtypeOrCb : maybeCb;
  return settleArrayCallback(resolveInternal(hostname, rrtype), cb);
}

function resolve4(hostname: string, cb?: (...args: unknown[]) => void): void | Promise<unknown[]> {
  return settleArrayCallback(resolveInternal(hostname, "A"), cb);
}

function resolve6(hostname: string, cb?: (...args: unknown[]) => void): void | Promise<unknown[]> {
  return settleArrayCallback(resolveInternal(hostname, "AAAA"), cb);
}

function reverse(ip: string, cb?: (...args: unknown[]) => void): void | Promise<unknown[]> {
  const ptr = toReversePointer(ip);
  return settleArrayCallback(resolveInternal(ptr, "PTR"), cb);
}

function lookupService(): never {
  return notImplemented("dns.lookupService");
}

function setServers(): never {
  return notImplemented("dns.setServers");
}

function getServers(): string[] {
  return [getRuntimeDnsConfig().dohEndpoint];
}

const promises = {
  async lookup(hostname: string, options?: number | LookupOptions) {
    return lookupInternal(hostname, options);
  },
  async resolve(hostname: string, rrtype?: string) {
    return resolveInternal(hostname, rrtype);
  },
  async resolve4(hostname: string) {
    return resolveInternal(hostname, "A");
  },
  async resolve6(hostname: string) {
    return resolveInternal(hostname, "AAAA");
  },
  async reverse(ip: string) {
    return reverse(ip) as Promise<unknown[]>;
  },
};

const dnsModule = {
  lookup,
  resolve,
  resolve4,
  resolve6,
  reverse,
  lookupService,
  setServers,
  getServers,
  promises,
};

export {
  lookup,
  resolve,
  resolve4,
  resolve6,
  reverse,
  lookupService,
  setServers,
  getServers,
  promises,
};
export default dnsModule;
