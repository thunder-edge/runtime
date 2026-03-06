// Bootstrap module: imports all extension ESM and exposes Web API globals.
//
// deno_core loads extension ESM as side-modules but only evaluates those
// reachable from an esm_entry_point.  This module is that entry point.
// After evaluation we assign the standard Web API classes to globalThis
// so user code can use them (e.g. new Request(), fetch(), etc.).

// -- 1. Import all extension ESM (forces evaluation) -----------

// deno_webidl
import "ext:deno_webidl/00_webidl.js";

// deno_io (required to be evaluated)
import "ext:deno_io/12_io.js";

// deno_fs (required to be evaluated)
import "ext:deno_fs/30_fs.js";

// deno_web (now includes console, URL, and other web APIs)
import "ext:deno_web/00_infra.js";
import { Console } from "ext:deno_web/01_console.js";
import { URL, URLSearchParams } from "ext:deno_web/00_url.js";
import { URLPattern } from "ext:deno_web/01_urlpattern.js";
import { DOMException } from "ext:deno_web/01_dom_exception.js";
import "ext:deno_web/01_mimesniff.js";
import "ext:deno_web/01_broadcast_channel.js";
import {
  Event, EventTarget, ErrorEvent, CloseEvent, CustomEvent,
  MessageEvent, ProgressEvent, PromiseRejectionEvent,
  reportError,
} from "ext:deno_web/02_event.js";
import { structuredClone } from "ext:deno_web/02_structured_clone.js";
import {
  setTimeout, setInterval, clearTimeout, clearInterval,
} from "ext:deno_web/02_timers.js";
import { AbortController, AbortSignal } from "ext:deno_web/03_abort_signal.js";
import "ext:deno_web/04_global_interfaces.js";
import { atob, btoa } from "ext:deno_web/05_base64.js";
import {
  ReadableStream, WritableStream, TransformStream,
  ByteLengthQueuingStrategy, CountQueuingStrategy,
} from "ext:deno_web/06_streams.js";
import {
  TextEncoder, TextDecoder, TextEncoderStream, TextDecoderStream,
} from "ext:deno_web/08_text_encoding.js";
import { Blob, File } from "ext:deno_web/09_file.js";
import { FileReader } from "ext:deno_web/10_filereader.js";
import "ext:deno_web/12_location.js";
import { MessageChannel, MessagePort } from "ext:deno_web/13_message_port.js";
import { CompressionStream, DecompressionStream } from "ext:deno_web/14_compression.js";
import { Performance, performance, PerformanceEntry, PerformanceMark, PerformanceMeasure } from "ext:deno_web/15_performance.js";
import { ImageData } from "ext:deno_web/16_image_data.js";

// deno_crypto (using minimal deno_node shim for crypto constants)
import { Crypto, crypto, CryptoKey, SubtleCrypto } from "ext:deno_crypto/00_crypto.js";

// deno_telemetry
import "ext:deno_telemetry/telemetry.ts";
import "ext:deno_telemetry/util.ts";

// deno_fetch
import { Headers } from "ext:deno_fetch/20_headers.js";
import { FormData } from "ext:deno_fetch/21_formdata.js";
import "ext:deno_fetch/22_body.js";
import "ext:deno_fetch/22_http_client.js";
import { Request } from "ext:deno_fetch/23_request.js";
import { Response } from "ext:deno_fetch/23_response.js";
import { fetch } from "ext:deno_fetch/26_fetch.js";
import { EventSource } from "ext:deno_fetch/27_eventsource.js";

// deno_net (required by fetch)
import "ext:deno_net/01_net.js";
import "ext:deno_net/02_tls.js";

// edge_assert (native assert helpers for user imports)
// Optional: only present when CLI test mode enables this extension.
import("ext:edge_assert/mod.ts").catch(() => {
  // no-op in production runtime profile
});

// -- 2. Expose Web API globals on globalThis ---------------------

// console
const core = globalThis.Deno?.core ?? globalThis.__bootstrap?.core;
if (!globalThis.console) {
  globalThis.console = new Console((msg, level) => {
    core?.print?.(msg, level > 1);
  });
}

// Deno namespace (minimal, for Deno.serve interception)
if (!globalThis.Deno) {
  globalThis.Deno = {};
}

// URL
Object.assign(globalThis, {
  URL,
  URLSearchParams,
  URLPattern,
});

// Events
Object.assign(globalThis, {
  Event,
  EventTarget,
  ErrorEvent,
  CloseEvent,
  CustomEvent,
  MessageEvent,
  ProgressEvent,
  PromiseRejectionEvent,
  reportError,
});

// Timers
Object.assign(globalThis, {
  setTimeout,
  setInterval,
  clearTimeout,
  clearInterval,
});

// Abort
Object.assign(globalThis, {
  AbortController,
  AbortSignal,
});

// Encoding
Object.assign(globalThis, {
  atob,
  btoa,
  TextEncoder,
  TextDecoder,
  TextEncoderStream,
  TextDecoderStream,
});

// Streams
Object.assign(globalThis, {
  ReadableStream,
  WritableStream,
  TransformStream,
  ByteLengthQueuingStrategy,
  CountQueuingStrategy,
});

// DOM
Object.assign(globalThis, {
  DOMException,
  structuredClone,
});

// Files
Object.assign(globalThis, {
  Blob,
  File,
  FileReader,
});

// Compression
Object.assign(globalThis, {
  CompressionStream,
  DecompressionStream,
});

// Performance
Object.assign(globalThis, {
  Performance,
  performance,
  PerformanceEntry,
  PerformanceMark,
  PerformanceMeasure,
});

// Messaging
Object.assign(globalThis, {
  MessageChannel,
  MessagePort,
  ImageData,
});

// Crypto (Web Crypto API)
Object.assign(globalThis, {
  Crypto,
  crypto,
  CryptoKey,
  SubtleCrypto,
});

// Fetch
Object.assign(globalThis, {
  Headers,
  FormData,
  Request,
  Response,
  fetch,
  EventSource,
});

// === SANDBOX SECURITY ===
// Remove dangerous APIs that should not be available in edge runtime.
// This ensures code cannot escape the sandbox.

// Delete dangerous Deno APIs (if any were exposed)
if (globalThis.Deno) {
  // Subprocess execution - NOT ALLOWED
  delete globalThis.Deno.run;
  delete globalThis.Deno.Command;

  // Direct filesystem access - controlled via permissions
  delete globalThis.Deno.readFile;
  delete globalThis.Deno.readFileSync;
  delete globalThis.Deno.writeFile;
  delete globalThis.Deno.writeFileSync;
  delete globalThis.Deno.readTextFile;
  delete globalThis.Deno.readTextFileSync;
  delete globalThis.Deno.writeTextFile;
  delete globalThis.Deno.writeTextFileSync;
  delete globalThis.Deno.mkdir;
  delete globalThis.Deno.mkdirSync;
  delete globalThis.Deno.remove;
  delete globalThis.Deno.removeSync;
  delete globalThis.Deno.rename;
  delete globalThis.Deno.renameSync;
  delete globalThis.Deno.symlink;
  delete globalThis.Deno.symlinkSync;
  delete globalThis.Deno.link;
  delete globalThis.Deno.linkSync;
  delete globalThis.Deno.chmod;
  delete globalThis.Deno.chmodSync;
  delete globalThis.Deno.chown;
  delete globalThis.Deno.chownSync;
  delete globalThis.Deno.stat;
  delete globalThis.Deno.statSync;
  delete globalThis.Deno.lstat;
  delete globalThis.Deno.lstatSync;
  delete globalThis.Deno.realPath;
  delete globalThis.Deno.realPathSync;
  delete globalThis.Deno.readDir;
  delete globalThis.Deno.readDirSync;
  delete globalThis.Deno.copyFile;
  delete globalThis.Deno.copyFileSync;
  delete globalThis.Deno.readLink;
  delete globalThis.Deno.readLinkSync;
  delete globalThis.Deno.truncate;
  delete globalThis.Deno.truncateSync;
  delete globalThis.Deno.open;
  delete globalThis.Deno.openSync;
  delete globalThis.Deno.create;
  delete globalThis.Deno.createSync;
  delete globalThis.Deno.makeTempDir;
  delete globalThis.Deno.makeTempDirSync;
  delete globalThis.Deno.makeTempFile;
  delete globalThis.Deno.makeTempFileSync;

  // FFI - NOT ALLOWED
  delete globalThis.Deno.dlopen;

  // Environment variables - NOT ALLOWED
  delete globalThis.Deno.env;

  // Exit/Signals - NOT ALLOWED
  delete globalThis.Deno.exit;
  delete globalThis.Deno.kill;
  delete globalThis.Deno.addSignalListener;
  delete globalThis.Deno.removeSignalListener;

  // Raw network - controlled via permissions
  delete globalThis.Deno.listen;
  delete globalThis.Deno.listenTls;
  delete globalThis.Deno.connect;
  delete globalThis.Deno.connectTls;
  delete globalThis.Deno.listenDatagram;

  // Permissions API - allow reading but not requesting
  delete globalThis.Deno.permissions?.request;
  delete globalThis.Deno.permissions?.revoke;
}

// Prevent dynamic code execution
// Note: These should be done carefully as some libraries need them
// delete globalThis.eval;
// delete globalThis.Function;

// === GLOBAL HARDENING ===
// Lock down critical API references so user code cannot overwrite runtime
// primitives like fetch/Request/crypto/console.
function defineImmutableGlobal(name) {
  if (!(name in globalThis)) return;
  const value = globalThis[name];
  Object.defineProperty(globalThis, name, {
    value,
    writable: false,
    configurable: false,
    enumerable: true,
  });
}

const criticalGlobalNames = [
  "fetch",
  "Request",
  "Response",
  "Headers",
  "crypto",
  "URL",
  "URLSearchParams",
  "TextEncoder",
  "TextDecoder",
  "console",
];

for (const name of criticalGlobalNames) {
  defineImmutableGlobal(name);
}

for (const name of criticalGlobalNames) {
  const target = globalThis[name];
  if ((typeof target === "object" && target !== null) || typeof target === "function") {
    try {
      Object.freeze(target);
    } catch {
      // Best-effort freeze: some native objects may reject freezing.
    }
  }
}
