import { AsyncLocalStorage, alsRegistry as globalAlsRegistry } from "node:async_hooks";

/**
 * Captures current ALS context snapshot
 */
function captureAlsContext(): Map<AsyncLocalStorage<unknown>, unknown> {
  const context = new Map<AsyncLocalStorage<unknown>, unknown>();

  // Iterate through all AsyncLocalStorage instances from async_hooks registry.
  for (const als of globalAlsRegistry ?? []) {
    if ((als as any).__enabled) {
      context.set(als, (als as any).__store);
    }
  }

  return context;
}

/**
 * Restores ALS context from snapshot
 */
function restoreAlsContext(context: Map<AsyncLocalStorage<unknown>, unknown>): void {
  // Restore each ALS instance to its captured value
  for (const [als, value] of context) {
    (als as any).__store = value;
  }
}

class EventEmitter {
  #events = new Map<
    string | symbol,
    Array<{ fn: (...args: unknown[]) => void; context: Map<AsyncLocalStorage<unknown>, unknown> }>
  >();
  #maxListeners = 10;

  setMaxListeners(n: number) {
    this.#maxListeners = Number(n);
    return this;
  }

  getMaxListeners() {
    return this.#maxListeners;
  }

  emit(eventName: string | symbol, ...args: unknown[]) {
    const listeners = this.#events.get(eventName);
    if (!listeners || listeners.length === 0) return false;

    for (const { fn, context: savedContext } of listeners) {
      // Save current ALS state
      const currentContext = captureAlsContext();

      try {
        // Restore the context from when listener was registered
        restoreAlsContext(savedContext);

        // Execute the listener
        fn.apply(this, args);
      } finally {
        // Restore the previous context
        restoreAlsContext(currentContext);
      }
    }
    return true;
  }

  addListener(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    if (typeof listener !== "function") {
      throw new TypeError("listener must be a function");
    }

    // Capture ALS context at registration time
    const context = captureAlsContext();

    const listeners = this.#events.get(eventName) ?? [];
    listeners.push({ fn: listener, context });
    this.#events.set(eventName, listeners);
    return this;
  }

  on(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    return this.addListener(eventName, listener);
  }

  prependListener(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    if (typeof listener !== "function") {
      throw new TypeError("listener must be a function");
    }

    const context = captureAlsContext();

    const listeners = this.#events.get(eventName) ?? [];
    listeners.unshift({ fn: listener, context });
    this.#events.set(eventName, listeners);
    return this;
  }

  once(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    if (typeof listener !== "function") {
      throw new TypeError("listener must be a function");
    }

    const context = captureAlsContext();

    const wrapped = (...args: unknown[]) => {
      this.removeListener(eventName, wrapped);
      listener.apply(this, args);
    };

    const listeners = this.#events.get(eventName) ?? [];
    listeners.push({ fn: wrapped, context });
    this.#events.set(eventName, listeners);
    return this;
  }

  off(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    return this.removeListener(eventName, listener);
  }

  removeListener(eventName: string | symbol, listener: (...args: unknown[]) => void) {
    const listeners = this.#events.get(eventName);
    if (!listeners || listeners.length === 0) return this;

    const next = listeners.filter(({ fn }) => fn !== listener);
    if (next.length > 0) {
      this.#events.set(eventName, next);
    } else {
      this.#events.delete(eventName);
    }

    return this;
  }

  removeAllListeners(eventName?: string | symbol) {
    if (eventName === undefined) {
      this.#events.clear();
      return this;
    }
    this.#events.delete(eventName);
    return this;
  }

  listenerCount(eventName: string | symbol) {
    return (this.#events.get(eventName) ?? []).length;
  }

  listeners(eventName: string | symbol) {
    return (this.#events.get(eventName) ?? []).map(({ fn }) => fn);
  }

  eventNames() {
    return [...this.#events.keys()];
  }
}

function once(emitter: EventEmitter, eventName: string | symbol): Promise<unknown[]> {
  return new Promise((resolve, reject) => {
    const onEvent = (...args: unknown[]) => {
      emitter.removeListener("error", onError);
      resolve(args);
    };
    const onError = (error: unknown) => {
      emitter.removeListener(eventName, onEvent);
      reject(error);
    };

    emitter.once(eventName, onEvent);
    emitter.once("error", onError);
  });
}

export { EventEmitter, once };

export default {
  EventEmitter,
  once,
};
