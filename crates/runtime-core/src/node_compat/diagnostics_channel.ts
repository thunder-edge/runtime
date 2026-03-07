type Handler = (message: unknown, name: string) => void;

type TracingHandlers = {
  start?: Handler;
  end?: Handler;
  asyncStart?: Handler;
  asyncEnd?: Handler;
  error?: Handler;
};

type TracingChannelMap = {
  start: Channel;
  end: Channel;
  asyncStart: Channel;
  asyncEnd: Channel;
  error: Channel;
};

class Channel {
  name: string;
  #subscribers = new Set<Handler>();

  constructor(name: string) {
    this.name = name;
  }

  publish(message: unknown): void {
    for (const fn of this.#subscribers) {
      fn(message, this.name);
    }
  }

  subscribe(fn: Handler): void {
    if (typeof fn === "function") this.#subscribers.add(fn);
  }

  unsubscribe(fn: Handler): void {
    this.#subscribers.delete(fn);
  }

  hasSubscribers(): boolean {
    return this.#subscribers.size > 0;
  }
}

const registry = new Map<string, Channel>();

function channel(name: string): Channel {
  const key = String(name);
  const existing = registry.get(key);
  if (existing) return existing;
  const created = new Channel(key);
  registry.set(key, created);
  return created;
}

function hasSubscribers(name: string): boolean {
  return channel(name).hasSubscribers();
}

function normalizeTracingChannels(nameOrChannels?: string | Partial<TracingChannelMap>): TracingChannelMap {
  if (!nameOrChannels) {
    return {
      start: channel("tracing:start"),
      end: channel("tracing:end"),
      asyncStart: channel("tracing:asyncStart"),
      asyncEnd: channel("tracing:asyncEnd"),
      error: channel("tracing:error"),
    };
  }

  if (typeof nameOrChannels === "string") {
    const base = String(nameOrChannels);
    return {
      start: channel(`${base}:start`),
      end: channel(`${base}:end`),
      asyncStart: channel(`${base}:asyncStart`),
      asyncEnd: channel(`${base}:asyncEnd`),
      error: channel(`${base}:error`),
    };
  }

  return {
    start: nameOrChannels.start ?? channel("tracing:start"),
    end: nameOrChannels.end ?? channel("tracing:end"),
    asyncStart: nameOrChannels.asyncStart ?? channel("tracing:asyncStart"),
    asyncEnd: nameOrChannels.asyncEnd ?? channel("tracing:asyncEnd"),
    error: nameOrChannels.error ?? channel("tracing:error"),
  };
}

class TracingChannel {
  channels: TracingChannelMap;

  constructor(nameOrChannels?: string | Partial<TracingChannelMap>) {
    this.channels = normalizeTracingChannels(nameOrChannels);
  }

  hasSubscribers(): boolean {
    return (
      this.channels.start.hasSubscribers() ||
      this.channels.end.hasSubscribers() ||
      this.channels.asyncStart.hasSubscribers() ||
      this.channels.asyncEnd.hasSubscribers() ||
      this.channels.error.hasSubscribers()
    );
  }

  subscribe(handlers: TracingHandlers): this {
    if (!handlers || typeof handlers !== "object") return this;
    if (typeof handlers.start === "function") this.channels.start.subscribe(handlers.start);
    if (typeof handlers.end === "function") this.channels.end.subscribe(handlers.end);
    if (typeof handlers.asyncStart === "function") this.channels.asyncStart.subscribe(handlers.asyncStart);
    if (typeof handlers.asyncEnd === "function") this.channels.asyncEnd.subscribe(handlers.asyncEnd);
    if (typeof handlers.error === "function") this.channels.error.subscribe(handlers.error);
    return this;
  }

  unsubscribe(handlers: TracingHandlers): this {
    if (!handlers || typeof handlers !== "object") return this;
    if (typeof handlers.start === "function") this.channels.start.unsubscribe(handlers.start);
    if (typeof handlers.end === "function") this.channels.end.unsubscribe(handlers.end);
    if (typeof handlers.asyncStart === "function") this.channels.asyncStart.unsubscribe(handlers.asyncStart);
    if (typeof handlers.asyncEnd === "function") this.channels.asyncEnd.unsubscribe(handlers.asyncEnd);
    if (typeof handlers.error === "function") this.channels.error.unsubscribe(handlers.error);
    return this;
  }

  traceSync<T>(fn: (...args: unknown[]) => T, thisArg?: unknown, ...args: unknown[]): T {
    this.channels.start.publish({ thisArg, args });
    try {
      const result = fn.apply(thisArg, args);
      this.channels.end.publish({ thisArg, args, result });
      return result;
    } catch (error) {
      this.channels.error.publish({ thisArg, args, error });
      throw error;
    }
  }

  tracePromise<T>(fn: (...args: unknown[]) => Promise<T>, thisArg?: unknown, ...args: unknown[]): Promise<T> {
    this.channels.start.publish({ thisArg, args });
    this.channels.asyncStart.publish({ thisArg, args });
    let out: Promise<T>;
    try {
      out = Promise.resolve(fn.apply(thisArg, args));
    } catch (error) {
      this.channels.error.publish({ thisArg, args, error });
      throw error;
    }
    return out.then(
      (value) => {
        this.channels.asyncEnd.publish({ thisArg, args, result: value });
        this.channels.end.publish({ thisArg, args, result: value });
        return value;
      },
      (error) => {
        this.channels.asyncEnd.publish({ thisArg, args, error });
        this.channels.error.publish({ thisArg, args, error });
        throw error;
      },
    );
  }

  traceCallback<T>(fn: (...args: unknown[]) => T, thisArg?: unknown, ...args: unknown[]): T {
    return this.traceSync(fn, thisArg, ...args);
  }
}

function tracingChannel(nameOrChannels?: string | Partial<TracingChannelMap>): TracingChannel {
  return new TracingChannel(nameOrChannels);
}

const diagnosticsChannel = { channel, hasSubscribers, Channel, TracingChannel, tracingChannel };

export { channel, hasSubscribers, Channel, TracingChannel, tracingChannel };
export default diagnosticsChannel;
