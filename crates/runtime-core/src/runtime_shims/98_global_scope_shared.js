// Shim for ext:runtime/98_global_scope_shared.js
// This provides stubs for deno_node modules that expect the full Deno runtime.

// Import console from deno_web
import { Console } from "ext:deno_web/01_console.js";

// Get core for print function
const core = globalThis.Deno?.core ?? globalThis.__bootstrap?.core;

// Create a console instance
const consoleInstance = new Console((msg, level) => {
  if (core?.ops?.op_edge_runtime_console_log) {
    core.ops.op_edge_runtime_console_log(msg, level);
    return;
  }
  core?.print?.(msg, level > 1);
});

// windowOrWorkerGlobalScope provides the global scope objects
// that deno_node's polyfills expect
export const windowOrWorkerGlobalScope = {
  console: {
    value: consoleInstance,
    writable: true,
    enumerable: true,
    configurable: true,
  },
  // Add other properties that might be needed
  Window: {
    value: undefined,
  },
  DedicatedWorkerGlobalScope: {
    value: undefined,
  },
};

export default windowOrWorkerGlobalScope;
