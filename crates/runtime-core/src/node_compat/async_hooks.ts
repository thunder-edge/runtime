type HookCallbacks = {
  init?: (asyncId: number, type: string, triggerAsyncId: number, resource: unknown) => void;
  before?: (asyncId: number) => void;
  after?: (asyncId: number) => void;
  destroy?: (asyncId: number) => void;
  promiseResolve?: (asyncId: number) => void;
};

type HookInstance = HookCallbacks & { enabled: boolean };

// Export the registry so EventEmitter can access it
export const alsRegistry = new Set<AsyncLocalStorage<unknown>>();
const hookRegistry = new Set<HookInstance>();

let nextAsyncId = 2;
let currentAsyncId = 1;
let currentTriggerAsyncId = 0;

function clearAllAlsStores(): void {
  for (const als of alsRegistry) {
    als.__store = undefined;
  }
}

function snapshotAlsStores(): Array<[AsyncLocalStorage<unknown>, unknown]> {
  const out: Array<[AsyncLocalStorage<unknown>, unknown]> = [];
  for (const als of alsRegistry) {
    if (!als.__enabled) continue;
    out.push([als, als.__store]);
  }
  return out;
}

function restoreAlsStores(snapshot: Array<[AsyncLocalStorage<unknown>, unknown]>, prev: Array<[AsyncLocalStorage<unknown>, unknown]>): void {
  for (const [als, value] of prev) {
    als.__store = value;
  }
  const seen = new Set(prev.map(([als]) => als));
  for (const [als] of snapshot) {
    if (!seen.has(als)) {
      als.__store = undefined;
    }
  }
}

function emitHook<K extends keyof HookCallbacks>(kind: K, ...args: Parameters<NonNullable<HookCallbacks[K]>>): void {
  for (const hook of hookRegistry) {
    if (!hook.enabled) continue;
    const fn = hook[kind];
    if (typeof fn !== "function") continue;
    try {
      (fn as (...p: Parameters<NonNullable<HookCallbacks[K]>>) => void)(...args);
    } catch {
      // Hook errors should never break user execution.
    }
  }
}

function wrapCallbackWithContext<T extends (...args: unknown[]) => unknown>(
  type: string,
  callback: T,
  resource?: unknown,
): T {
  const capturedStores = snapshotAlsStores();
  const asyncId = nextAsyncId++;
  const triggerId = currentAsyncId;

  emitHook("init", asyncId, type, triggerId, resource ?? callback);

  const wrapped = ((...args: unknown[]) => {
    const prevStores = snapshotAlsStores();
    const prevAsyncId = currentAsyncId;
    const prevTriggerAsyncId = currentTriggerAsyncId;

    for (const [als, value] of capturedStores) {
      als.__store = value;
    }

    currentAsyncId = asyncId;
    currentTriggerAsyncId = triggerId;
    emitHook("before", asyncId);
    try {
      return callback(...args);
    } finally {
      emitHook("after", asyncId);
      emitHook("destroy", asyncId);
      currentAsyncId = prevAsyncId;
      currentTriggerAsyncId = prevTriggerAsyncId;
      restoreAlsStores(capturedStores, prevStores);
    }
  }) as T;

  return wrapped;
}

let instrumentationInstalled = false;
function installAsyncInstrumentation(): void {
  if (instrumentationInstalled) return;
  instrumentationInstalled = true;

  const originalQueueMicrotask = globalThis.queueMicrotask?.bind(globalThis);
  if (typeof originalQueueMicrotask === "function") {
    globalThis.queueMicrotask = (callback: VoidFunction): void => {
      originalQueueMicrotask(wrapCallbackWithContext("Microtask", callback));
    };
  }

  const originalSetTimeout = globalThis.setTimeout?.bind(globalThis);
  if (typeof originalSetTimeout === "function") {
    globalThis.setTimeout = ((handler: TimerHandler, timeout?: number, ...args: unknown[]) => {
      if (typeof handler !== "function") {
        return originalSetTimeout(handler, timeout, ...args);
      }
      const wrapped = wrapCallbackWithContext("Timeout", (...invokeArgs: unknown[]) => {
        (handler as (...hArgs: unknown[]) => unknown)(...invokeArgs);
      });
      return originalSetTimeout(wrapped as TimerHandler, timeout, ...args);
    }) as typeof globalThis.setTimeout;
  }

  const originalThen = Promise.prototype.then;
  Promise.prototype.then = function thenPatched(onFulfilled?: unknown, onRejected?: unknown) {
    const wrappedFulfilled = typeof onFulfilled === "function"
      ? wrapCallbackWithContext("Promise.then", onFulfilled as (...args: unknown[]) => unknown, this)
      : onFulfilled;
    const wrappedRejected = typeof onRejected === "function"
      ? wrapCallbackWithContext("Promise.catch", onRejected as (...args: unknown[]) => unknown, this)
      : onRejected;
    return originalThen.call(this, wrappedFulfilled, wrappedRejected);
  };
}

class AsyncLocalStorage<T = unknown> {
  __store: T | undefined;
  __enabled = true;

  constructor() {
    alsRegistry.add(this as unknown as AsyncLocalStorage<unknown>);
    installAsyncInstrumentation();
  }

  run<R>(store: T, callback: (...args: unknown[]) => R, ...args: unknown[]): R {
    const prev = this.__store;
    this.__store = store;
    this.__enabled = true;

    const restore = () => {
      this.__store = prev;
    };

    try {
      const result = callback(...args);
      if (
        result !== null &&
        typeof result === "object" &&
        "then" in (result as object) &&
        typeof (result as { then?: unknown }).then === "function"
      ) {
        return (result as Promise<R>).finally(restore) as unknown as R;
      }

      restore();
      return result;
    } catch (error) {
      restore();
      throw error;
    }
  }

  enterWith(store: T): void {
    this.__enabled = true;
    this.__store = store;
  }

  getStore(): T | undefined {
    return this.__enabled ? this.__store : undefined;
  }

  disable(): void {
    this.__enabled = false;
    this.__store = undefined;
  }
}

class AsyncResource {
  type: string;
  asyncId: number;
  triggerAsyncId: number;

  constructor(type: string) {
    this.type = String(type || "AsyncResource");
    this.asyncId = nextAsyncId++;
    this.triggerAsyncId = currentAsyncId;
    emitHook("init", this.asyncId, this.type, this.triggerAsyncId, this);
  }

  runInAsyncScope<R>(fn: (...args: unknown[]) => R, thisArg?: unknown, ...args: unknown[]): R {
    const wrapped = wrapCallbackWithContext(this.type, (...innerArgs: unknown[]) => fn.apply(thisArg, innerArgs), this);
    return wrapped(...args);
  }

  emitDestroy(): void {
    emitHook("destroy", this.asyncId);
  }
}

function createHook(callbacks: HookCallbacks) {
  const hook: HookInstance = {
    ...callbacks,
    enabled: false,
  };

  return {
    enable() {
      hook.enabled = true;
      hookRegistry.add(hook);
      installAsyncInstrumentation();
      return this;
    },
    disable() {
      hook.enabled = false;
      hookRegistry.delete(hook);
      return this;
    },
  };
}

function executionAsyncId(): number {
  return currentAsyncId;
}

function triggerAsyncId(): number {
  return currentTriggerAsyncId;
}

type EdgeRuntimeAsyncHooksBridge = {
  runWithExecutionContext<R>(executionId: string, callback: () => R | Promise<R>): R | Promise<R>;
  clearExecutionContext(executionId: string): void;
};

const edgeRuntimeBridgeGlobal = globalThis as typeof globalThis & {
  __edgeRuntimeAsyncHooks?: EdgeRuntimeAsyncHooksBridge;
};

edgeRuntimeBridgeGlobal.__edgeRuntimeAsyncHooks = {
  runWithExecutionContext<R>(_executionId: string, callback: () => R | Promise<R>): R | Promise<R> {
    const previousAsyncId = currentAsyncId;
    const previousTriggerAsyncId = currentTriggerAsyncId;
    const requestAsyncId = nextAsyncId++;

    // Start each request from a clean ALS store set to avoid bleed across requests.
    clearAllAlsStores();
    currentAsyncId = requestAsyncId;
    currentTriggerAsyncId = previousAsyncId;
    emitHook("init", requestAsyncId, "EDGE_REQUEST", previousAsyncId, callback);
    emitHook("before", requestAsyncId);

    const finalize = () => {
      emitHook("after", requestAsyncId);
      emitHook("destroy", requestAsyncId);
      clearAllAlsStores();
      currentAsyncId = previousAsyncId;
      currentTriggerAsyncId = previousTriggerAsyncId;
    };

    try {
      const result = callback();
      if (
        result !== null &&
        typeof result === "object" &&
        "then" in (result as object) &&
        typeof (result as { then?: unknown }).then === "function"
      ) {
        return (result as Promise<R>).finally(finalize);
      }

      finalize();
      return result;
    } catch (error) {
      finalize();
      throw error;
    }
  },

  clearExecutionContext(_executionId: string): void {
    clearAllAlsStores();
  },
};

const asyncHooks = {
  AsyncLocalStorage,
  AsyncResource,
  createHook,
  executionAsyncId,
  triggerAsyncId,
};

export { AsyncLocalStorage, AsyncResource, createHook, executionAsyncId, triggerAsyncId };
export default asyncHooks;
