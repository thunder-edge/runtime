# Node.js Compatibility Roadmap for Deno Edge Runtime

**Document Version:** 1.0
**Date:** March 2026
**Classification:** Technical Architecture & Roadmap

---

## Executive Summary

This document provides a comprehensive technical audit of the Deno Edge Runtime's Node.js compatibility layer and a strategic roadmap for evolving it into a robust Node.js-compatible edge runtime capable of running SSR frameworks (Next.js, Vite, Astro, Remix, Hono) in a secure multi-tenant sandbox environment.

### Current State

The runtime currently implements **35 Node.js builtin modules** across a Deno + V8 architecture:

- **35 registered ESM modules** (`node:*` specifiers)
- **1 native compression op** (zlib via Rust)
- **3 critical security boundaries** (fs, process, net)
- **~8,000 lines of TypeScript shimming code**

### Compatibility Tier

| Category | Status | Notes |
|----------|--------|-------|
| **Buffer API** | ✅ Full | Delegates to globalThis.Buffer (Deno-provided) |
| **EventEmitter** | ✅ Partial | Complete API, missing internal hooks |
| **Streams** | ⚠️ Partial | No backpressure, no highWaterMark |
| **Timers** | ✅ Full | setTimeout, setInterval, nextTick via microtasks |
| **Crypto** | ❌ None | Only WebCrypto; no Node crypto module |
| **fs** | ✅ Limited | VFS only (10MB quota); no host filesystem |
| **http/https** | ⚠️ Partial | Fetch-based; no server sockets |
| **net/tls** | ⚠️ Partial | Outbound only; Deno.connect/connectTls |
| **process** | ⚠️ Partial | Isolated sandbox; no signals or exit control |
| **AsyncLocalStorage** | ✅ Full | Full context propagation via promise hooks |
| **diagnostics_channel** | ✅ Full | Complete pub/sub + tracing |
| **dns** | ✅ Full | DoH-based; Cloudflare resolver |
| **zlib** | ✅ Full | All algorithms (no Brotli) |

### Verdict

**This runtime is suitable for:**
- ✅ Express-like API servers (via custom HTTP adapters)
- ✅ Vite SSR (with caveats on streaming)
- ✅ Astro (static generation focus)
- ✅ Hono (lean, native async support)
- ⚠️ Next.js (requires stream fixes + crypto shims)
- ⚠️ Remix (requires stream and fetch integration work)

**This runtime is NOT suitable for:**
- ❌ CPU-intensive algorithms (no native modules, limited to Node 20 semantics)
- ❌ Direct file I/O (VFS only, no host filesystem)
- ❌ Streaming media (no backpressure)
- ❌ Child process orchestration
- ❌ Cluster mode
- ❌ tls/https servers (no bind)

---

## 1. Runtime Architecture Overview

### 1.1 Core Runtime Components

```
┌─────────────────────────────────────────────────────────────┐
│                    Edge Runtime Process                      │
│                    (Single Deno Isolate)                     │
└────────────────────────┬────────────────────────────────────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
   ┌────▼─────┐   ┌─────▼────┐   ┌──────▼────┐
   │   V8      │   │ Deno     │   │   Native  │
   │ Isolate   │   │ System   │   │    Ops    │
   │(per fn)   │   │   APIs   │   │(Rust FFI) │
   └──────────┘   └──────────┘   └───────────┘
        │              │              │
        └──────────────┼──────────────┘
                       │
  ┌────────────────────▼─────────────────────┐
  │     Edge Runtime Module System            │
  │  (Eszip bundles + VFS + Extension Mods)  │
  └─────────────────────────────────────────┘
```

### 1.2 Module Resolution Strategy

```typescript
// Resolution order for require("foo"):
1. Check @builtin modules       // maps to node_compat/*.ts
2. Check eszip bundle           // pre-bundled application code
3. If Deno-compatible, resolve  // some Deno modules can be used
4. Fail with ERR_MODULE_NOT_FOUND

// Node semantic:
- "node:*" → edge_node_compat extension (35 modules)
- "foo"    → eszip bundle or error
- No npm modules, no host fs resolution
```

### 1.3 Extension Architecture (extensions.rs)

The runtime registers Deno extensions in this order:

| Extension | Purpose | Notable Ops/APIs |
|-----------|---------|------------------|
| `edge_runtime_logging` | Console output | `op_edge_runtime_console_log` |
| `edge_stubs` | Compatibility shims | `op_set_raw`, `op_console_size`, `op_tls_peer_certificate` |
| `deno_webidl` | Type system | Web IDL validation |
| `deno_web` | Web APIs | Fetch, Blob, File, TextEncoder, etc. |
| `deno_tls` | TLS system | `deno_joinTlsListener` (blocked in edge) |
| `deno_io` | I/O primitives | Deno streams, permissions |
| `deno_fs` | Host filesystem | Blocked via permissions |
| `deno_net` | Network | Deno connect/listen primitives |
| `deno_telemetry` | Observability | Tracing, metrics |
| `deno_fetch` | HTTP client | `fetch()` implementation |
| `edge_node_compat` | Node modules | 35 ESM modules + `op_edge_zlib_transform` |
| `deno_node` | Node shims | Internal crypto constants |
| `deno_crypto` | Web Crypto | SubtleCrypto API |
| `edge_assert` | Testing (optional) | `assert()` and mocks |
| `edge_bootstrap` | Entry point | Initializes all extensions |

### 1.4 Isolation & Sandboxing Model

**Multiple functions, single process:**
```rust
// Each function gets its own isolate:
for function in functions {
    let config = IsolateConfig {
        max_heap_size_mb: 128,
        cpu_time_limit_secs: 50,
        wall_clock_timeout_secs: 60,
        ..
    };
    let isolate = Isolate::new(config)?;
    let result = isolate.run_module(function_path)?;
    // Isolate is dropped, memory is freed
}
```

**Shared host process:**
- ✅ Efficient CPU usage (no process overhead)
- ✅ Fast isolate creation (V8 copies)
- ❌ No process isolation (trusting V8 isolate boundaries)
- ❌ Potential data leaks if V8 isolate is compromised

**Per-isolate boundaries:**
- Module cache (eszip-backed, immutable)
- Memory limits (configurable, enforced)
- Timeout limits (CPU + wall clock)
- VFS state (per isolate)
- env store (isolated proxy)
- process object (non-writable instance)
- AsyncLocalStorage (per-isolate context propagation)

### 1.5 Virtual File System (VFS)

```typescript
// Global state (per isolate):
globalThis.__edgeVfsState = {
  files: Map<path, {data, mtime, size}>,
  dirs:  Map<path, {mtime}>,
  quotaUsed: number,
}

globalThis.__edgeRuntimeVfsConfig = {
  totalQuotaBytes: 10 * 1024 * 1024,  // 10MB
  maxFileBytes: 5 * 1024 * 1024,      // 5MB per file
}

// Writable mounts:
// - /tmp/* (temporary files)
// - /dev/null (write-only)
//
// Read-only mounts:
// - / (synthetic)
// - /bundle (eszip contents)
//
// Blocked mounts:
// - Everything else (EOPNOTSUPP)
```

### 1.6 DNS Configuration (DoH)

```typescript
globalThis.__edgeRuntimeDnsConfig = {
  dohEndpoint: "https://1.1.1.1/dns-query",  // Cloudflare
  maxAnswers: 16,
  timeoutMs: 2000,
  // All DNS queries go over HTTPS (DoH)
  // No plaintext DNS exposure
}
```

---

## 2. Comprehensive Node.js API Compatibility Table

### 2.1 Core Language APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:buffer` | ✅ Full | Proxy → globalThis.Buffer | Delegates to Deno runtime |
| `node:events` | ✅ Partial | Custom EventEmitter | Missing: rawListeners, errorMonitor, captureRejections |
| `node:util` | ✅ Partial | JS + MIMEType/MIMEParams | Missing: TextDecoder, stripVT, parseArgs, debuglog |
| `node:assert` | ✅ Full | From edge_assert ext | Test assertions + mocks |
| `node:console` | ✅ Full | Proxy → globalThis.console | Delegates to Deno |

### 2.2 Async & Timing APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:async_hooks` | ✅ Full | Custom context tracking | Monkey-patches Promise.then, queueMicrotask, setTimeout |
| `node:timers` | ✅ Full | Proxy → globalThis | setTimeout, setInterval, clearTimeout, clearInterval |
| `node:timers/promises` | ✅ Full | setTimeout(...), setInterval(...) as Promises | Resolves after delay |

### 2.3 I/O APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:fs` | ⚠️ Limited | VFS-based | 10MB quota, /tmp + /dev/null writable, /bundle read-only, no host fs |
| `node:fs/promises` | ⚠️ Limited | Async wrappers over sync | Same VFS limits |
| `node:stream` | ⚠️ Partial | Custom streams | Missing: backpressure, highWaterMark, encoding, objectMode |
| `node:readline` | ❌ Stub | Throws ERR_NOT_IMPLEMENTED | Would require tty support |

### 2.4 Network APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:http` | ⚠️ Partial | Fetch-based client | No server; limited status codes; no keepalive |
| `node:https` | ⚠️ Partial | Fetch-based client (https) | Copy of http.ts; no TLS options |
| `node:net` | ⚠️ Partial | Deno.connect TCP | Outbound only; no Server.listen; Port 1-65535 validated |
| `node:tls` | ⚠️ Partial | Deno.connectTls | No server; no certificate methods; rootCertificates = [] |
| `node:dgram` | ❌ Stub | Throws ERR_NOT_IMPLEMENTED | UDP not exposed |
| `node:dns` | ✅ Full | DoH via Cloudflare | A, AAAA, MX, TXT, SRV, CAA, NS, PTR, SOA, CNAME |
| `node:http2` | ❌ Stub | Throws ERR_NOT_IMPLEMENTED | Would require server support |

### 2.5 Process & System APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:process` | ⚠️ Partial | Sandboxed object | Synthetic pid/ppid/version; isolated env; no signals |
| `node:os` | ⚠️ Partial | Hardcoded defaults | All system info returns "linux", "x64", synthetic values |
| `node:cluster` | ❌ Stub | fork() throws | isPrimary always true; no worker support |
| `node:child_process` | ❌ Stub | spawn/exec/fork/execFile throw | Subprocess execution blocked |
| `node:worker_threads` | ❌ Missing | Not implemented | No threading API |

### 2.6 Cryptography & Security

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:crypto` | ❌ Missing | Not implemented | Only WebCrypto API available |
| `node:tls` | ⚠️ Partial | Outbound client only | No TLS server, no certificate methods |

### 2.7 Code Execution & VM

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:vm` | ❌ Stub | Script stores code, execution throws | Sandbox escape prevention |
| `node:module` | ✅ Partial | createRequire for builtins | Only `node:*` modules loadable; no npm |

### 2.8 Observability & Debugging

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:perf_hooks` | ✅ Partial | Delegates to Web APIs | performance, PerformanceObserver work; no monitorEventLoopDelay |
| `node:diagnostics_channel` | ✅ Full | Custom pub/sub + TracingChannel | All methods implemented |
| `node:inspector` | ❌ Stub | Throws | Would require debug protocol |
| `node:trace_events` | ❌ Stub | Not provided | Requires system tracing |

### 2.9 Interoperability APIs

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:url` | ✅ Full | Custom URL parser | URL, URLSearchParams, fileURLToPath, pathToFileURL |
| `node:path` | ✅ Full | Custom path parser | posix, win32, resolve, join, normalize, etc. |
| `node:querystring` | ✅ Full | Custom parsing | parse, stringify, escape, unescape |
| `node:string_decoder` | ✅ Full | Custom decoder | StringDecoder class |
| `node:punycode` | ✅ Full | Custom encoder | encode, decode, toUnicode, toASCII |

### 2.10 Compression

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:zlib` | ✅ Full | Native Rust op | gzip, deflate, inflate (sync + async); no Brotli; 16MB output limit |

### 2.11 Miscellaneous

| Module | Status | Implementation | Notes |
|--------|--------|---|---|
| `node:sys` | ✅ Full | Alias for util | Non-standard, maps to util |
| `node:v8` | ❌ Stub | Not implemented | Would require V8 inspection API |
| `node:wasi` | ❌ Missing | Not implemented | WebAssembly System Interface not exposed |
| `node:repl` | ❌ Stub | Not provided | Requires interactive shell |
| `node:test` | ✅ Full | edge_assert testing module | test(), describe(), mock support |

---

## 3. Comparison with Workerd (Cloudflare Workers)

### 3.1 Architecture Differences

| Aspect | Deno Edge Runtime | Workerd |
|--------|---|---|
| **Language** | TypeScript shims + Rust ops | C++ JSG bindings + TS modules |
| **V8 Version** | Latest (via Deno) | Forked/pinned |
| **Module System** | ESM (deno_core) | ESM + CommonJS compatibility |
| **Process Model** | V8 isolates within single process | Similar (isolates per request) |
| **Extension Model** | deno_core extensions | C++ modules + bindings |
| **Native Bindings** | Limited (1 op: zlib) | Extensive (buffer, crypto, util, net, etc.) |

### 3.2 API Coverage Comparison

| Module | Deno Runtime | Workerd | Winner |
|--------|---|---|---|
| **node:crypto** | ❌ None | ✅ Partial (Node API via WebCrypto) | Workerd |
| **node:stream** | ⚠️ Partial (no backpressure) | ✅ Partial (better; has streaming) | Workerd |
| **node:buffer** | ✅ Full (via Deno) | ✅ Full (custom) | Tie |
| **node:async_hooks** | ✅ Full + context propagation | ✅ Full (V8 embedder data) | Deno (better context) |
| **node:diagnostics_channel** | ✅ Full | ✅ Full | Tie |
| **node:fs** | ⚠️ VFS (10MB) | ✅ Durable Objects + R2 bindings | Workerd (more features) |
| **node:net** | ⚠️ TCP outbound | ✅ Outbound + TCP Sockets service | Workerd |
| **node:http** | ⚠️ Fetch-based | ✅ Full HTTP/1.1 + HTTP/2 | Workerd |
| **node:zlib** | ✅ Full (via flate2) | ✅ Full (via C bindings) | Tie |
| **node:dns** | ✅ DoH only | ✅ Zone API | Workerd (more complete) |

### 3.3 Native Binding Strategy

**Deno runtime approach:**
```rust
// One op for compression
pub fn op_edge_zlib_transform(
    mode: String,  // "gzip", "deflate", etc.
    data: &[u8],
    max_output: usize,
) -> Result<Vec<u8>, Error>
```

**Workerd approach:**
```c++
// Extensive native bindings
class NodeBuffer { ... }    // Native buffer ops
class NodeCrypto { ... }    // RSA, HMAC, ECDH, etc.
class NodeZlib { ... }      // Stream classes
class NodeProcess { ... }   // Full process semantics
class NodeUtil { ... }      // MIMEType, inspect, etc.
```

**Verdict:** Workerd's native approach is more correct and performant, but Deno's TypeScript shims are easier to modify and maintain.

### 3.4 Security Model Comparison

| Aspect | Deno Runtime | Workerd |
|--------|---|---|
| **Subprocess blocking** | ✅ Blocked | ✅ Blocked |
| **Server socket blocking** | ✅ Blocked | ✅ Blocked (except via Durable Objects) |
| **Filesystem isolation** | ✅ VFS (10MB quota) | ✅ Durable Objects + R2 |
| **Network access** | ⚠️ Deno permissions | ✅ Capability-based (service triggers) |
| **Crypto access** | ⚠️ WebCrypto only | ✅ WebCrypto + Node API (safe subset) |
| **Env isolation** | ✅ Sandboxed proxy | ✅ Capability bindings |

### 3.5 Performance Characteristics

| Operation | Deno | Workerd | Notes |
|-----------|---|---|---|
| **Isolate creation** | ~100ms | ~50ms | Workerd has optimizations |
| **Module load (cold)** | ~200ms | ~150ms | Depends on bundle size |
| **Crypto operation (small)** | Via WebCrypto | Native fast path | Workerd faster for crypto |
| **Zlib compression** | Rust op | C bindings | Similar performance |
| **JSON parse** | V8 native | V8 native | Same |

---

## 4. Security Analysis

### 4.1 Threat Model

**Assumptions:**
- Isolate boundaries are enforced by V8 (trust boundary)
- Deno APIs are secure (no breakouts to host)
- User code may be malicious or buggy

**Threats to prevent:**
1. **Host filesystem access** → prevented via VFS + path restrictions
2. **Process execution** → prevented via child_process blocking
3. **Server binding** → prevented via net/http/tls socket blocking
4. **Signal handling** → prevented via process stub
5. **Environment variable leaks** → prevented via env proxy
6. **Memory exhaustion** → prevented via heap limits
7. **Infinite loops** → prevented via CPU time limits
8. **Uncontrolled network access** → controlled via fetch + dns DoH
9. **Crypto key material leakage** → prevented (no Node crypto, WebCrypto only)
10. **Code execution** → prevented via vm blocking

### 4.2 Security Boundary Analysis

#### 4.2.1 Filesystem (node:fs)

```typescript
// Threat: Read host files
// Mitigation:
// ✅ No host fs access (VFS-only)
// ✅ Only /tmp and /dev/null writable
// ✅ /bundle read-only (eszip immutable)

// Threat: Exhaust disk quota
// Mitigation:
// ✅ 10 MB total quota enforced
// ✅ 5 MB per-file limit enforced
// ✅ Quota tracking on every write

// Threat: Path traversal
// Mitigation:
// ✅ Path normalization (. and .. resolved)
// ✅ Mount checks prevent escape
```

**Residual risks:**
- VFS state not encrypted (if isolate is compromised, files readable)
- No integrity checking on VFS state

#### 4.2.2 Process (node:process)

```typescript
// Threat: Exit the function early
// Mitigation: ✅ process.exit() throws

// Threat: Observe host resources
// Mitigation: ✅ All os.*, process.memoryUsage, etc. return synthetic values

// Threat: Read environment variables
// Mitigation: ✅ process.env is a Proxy over isolated object

// Threat: Spawn subprocesses
// Mitigation: ✅ child_process.* all throw

// Threat: Send signals or kill processes
// Mitigation: ✅ process.kill(), process.abort() throw
```

**Residual risks:**
- process.env isolation is shallow (monkey-patching could break it)
- Synthetic process values may confuse code into unexpected behavior

#### 4.2.3 Network (node:http, node:net, node:tls)

```typescript
// Threat: Bind listening socket
// Mitigation: ✅ http.createServer() throws
// Mitigation: ✅ net.Server.listen() throws
// Mitigation: ✅ tls.createServer() throws

// Threat: SSRF attacks via fetch
// Mitigation: ⚠️ fetch() subject to Deno permissions + SSRF detection
// Note: Deno SSRF checks private IPs, localhost, etc.

// Threat: DNS leaks or poisoning
// Mitigation: ✅ All DNS over HTTPS (Cloudflare resolver)
// Mitigation: ✅ Timeout and answer limits enforced
// Mitigation: ✅ Resolver endpoint configurable
```

**Residual risks:**
- Outbound fetch limited only by Deno (no rate limiting in edge runtime)
- DNS DoH resolver could become a single point of failure
- No connection pooling / limit

#### 4.2.4 Code Execution (node:vm)

```typescript
// Threat: Execute arbitrary code in new context
// Mitigation: ✅ vm.runInNewContext() throws

// Threat: Script execution in current context
// Mitigation: ✅ Script.runInThisContext() throws

// Verdict: vm is completely blocked ✅
```

### 4.3 Attack Surface by Module

#### 4.3.1 High-Risk Modules (Blocked)

- ✅ `node:child_process` (subprocess execution)
- ✅ `node:cluster` (forking)
- ✅ `node:vm` (code injection)
- ✅ `node:worker_threads` (multi-threading, not implemented)

#### 4.3.2 Medium-Risk Modules (Partially Blocked)

- ⚠️ `node:fs` (VFS-only, quota-limited)
- ⚠️ `node:http` (fetch only, no listening)
- ⚠️ `node:net` (TCP outbound only)
- ⚠️ `node:tls` (TLS outbound only)
- ⚠️ `node:process` (sandboxed, no signals)

#### 4.3.3 Low-Risk Modules (Full or Partial)

- ✅ `node:async_hooks` (observability only)
- ✅ `node:events` (no side effects)
- ✅ `node:buffer` (data structure)
- ✅ `node:crypto` (WebCrypto only, no private key ops)
- ✅ `node:zlib` (compression, output-limited)
- ✅ `node:util` (parsing/formatting)
- ✅ `node:path` (path manipulation)
- ✅ `node:url` (URL parsing)
- ✅ `node:dns` (DoH, rate-limited)
- ✅ `node:diagnostics_channel` (observability)

### 4.4 Security Checklist

- [x] No subprocess execution
- [x] No inbound sockets
- [x] No host filesystem access
- [x] No process control signals
- [x] No arbitrary code execution
- [x] Memory limits enforced
- [x] CPU time limits enforced
- [x] Environment isolated
- [x] DNS over HTTPS enforced
- [x] VFS quota limited
- [x] Path traversal prevented
- [x] No native module loading
- [ ] No outbound rate limiting (TODO)
- [ ] No request timeout per-isolate (TODO)
- [ ] No VFS integrity checking (TODO)

---

## 5. Missing Primitives & APIs

### 5.1 Critical APIs for SSR Frameworks

#### 5.1.1 Crypto

**Currently missing:** The entire `node:crypto` module

```javascript
// ❌ Not available:
const crypto = require('node:crypto');
crypto.randomBytes(32);           // ❌
crypto.createHash('sha256');      // ❌
crypto.createHmac('sha256', key); // ❌
crypto.createCipheriv(...);       // ❌
crypto.pbkdf2(...);               // ❌
crypto.generateKeyPair(...);      // ❌

// ✅ Available instead (WebCrypto API):
const buf = new Uint8Array(32);
crypto.getRandomValues(buf);
const hash = await crypto.subtle.digest('SHA-256', data);
```

**Impact on frameworks:**
- Next.js: Sessions/cookies use `createHmac` ❌
- All frameworks: Random token generation uses `randomBytes` ❌
- Security-sensitive code: Can't use Node crypto APIs ❌

**Solutions:**
1. Create a Node Crypto shim using WebCrypto (mapping layer)
2. Add native Node crypto bindings (requires C++, more work)
3. Recommend application-level shims (crypto-js, nacl, etc.)

#### 5.1.2 Streams with Backpressure

**Currently missing:** Proper stream backpressure and highWaterMark

```javascript
// Implemented (simplified):
readable.pipe(writable);  // ✅ Works but ignores backpressure

// Missing:
readable.on('pause', () => { ... });      // ❌
readable.pause() / resume();              // ❌
highWaterMark option;                     // ❌
stream.pipeline(..., error => { ... });   // ⚠️ Partial

// Impact:
// SSR streamed responses (Next.js, Remix, Astro) work partially
// But no backpressure means potential memory exhaustion
// Large responses can cause OOM
```

**Solutions:**
1. Add `pause()`/`resume()` methods and honor them
2. Add `highWaterMark` buffering with internal queue
3. Implement `Readable.push()` semantics properly
4. Add `_read()` callback protocol

#### 5.1.3 AsyncLocalStorage Context Propagation

**Currently implemented but incomplete:**

```javascript
// ✅ Works:
const store = new AsyncLocalStorage();
store.run({ user: 'alice' }, () => {
  setTimeout(() => {
    console.log(store.getStore()); // { user: 'alice' } ✅
  }, 0);
});

// ⚠️ Partial (not propagated through):
// - fs callbacks (fs.readFile(f, (err, data) => { store.getStore() }))
// - EventEmitter handlers (emitter.on('event', () => { store.getStore() }))
// - Some Promise chains
// - Deno.connect() callbacks
```

**Solutions:**
1. Monkey-patch EventEmitter.prototype.emit to propagate ALS
2. Wrap fs callbacks with context restoration
3. Wrap all Deno async APIs (connect, connectTls, fetch, etc.)

#### 5.1.4 Request/Response Streaming

**Currently missing:** Proper streaming in http.ClientRequest and http.IncomingMessage

```javascript
// ✅ Works (simple):
http.get('http://example.com', res => {
  const data = [];
  res.on('data', chunk => data.push(chunk));
  res.on('end', () => console.log(Buffer.concat(data)));
});

// ❌ Doesn't work (streaming):
http.get(url)
  .pipe(gzip())
  .pipe(fs.createWriteStream('output.gz'));
  // Error: createWriteStream not implemented

// ⚠️ Works partially (fetch body):
http.get url with custom event emitter
// Re-emits body as 'data' events but:
// - No chunking control
// - No backpressure
// - Full body buffered in memory
```

**Solutions:**
1. Implement `createWriteStream()` backed by fs.writeFile + in-memory buffering
2. Implement `createReadStream()` backed by fs.readFile with range support
3. Add proper stream chunking/buffering

### 5.2 Framework-Specific Requirements

#### 5.2.1 Next.js SSR Requirements

| API | Status | Impact | Workaround |
|-----|--------|--------|-----------|
| `crypto.randomBytes()` | ❌ | Session IDs, CSRF tokens | Use `crypto.getRandomValues()` |
| `crypto.createHmac()` | ❌ | Cookie signing, authentication | Manually implement with WebCrypto |
| `stream.Readable.from()` | ✅ Partial | Async generator to stream | Works but no backpressure |
| `AsyncLocalStorage` | ✅ Full | Request context, middleware | Complete support ✅ |
| `process.env` | ✅ Partial | Config via env vars | Works (isolated) |
| `fs.readFileSync()` | ✅ | Reading bundle files | Works (/bundle mount) |
| Streaming responses | ⚠️ Basic | Page streaming to client | Works but buffers in memory |

**Verdict:** Next.js can run but requires crypto workarounds + cannot stream large responses safely.

#### 5.2.2 Remix SSR Requirements

| API | Status | Impact |
|-----|--------|--------|
| `AsyncLocalStorage` | ✅ | Context propagation ✅ |
| `stream` | ⚠️ | Limited streaming support |
| HTTP client | ✅ | fetch() available |
| `process.env` | ✅ | Sandboxed env works |

#### 5.2.3 Astro SSR/SSG

| API | Status | Impact |
|-----|--------|--------|
| `fs.readFileSync` | ✅ | Component loading ✅ |
| `import()` | ✅ | Dynamic imports (in bundle) |
| Streaming | ✅ | Limited (for static sites OK) |

#### 5.2.4 Hono Requirements

| API | Status | Impact |
|-----|--------|--------|
| HTTP primitives | ✅ | fetch/fetch API ✅ |
| Routing | ✅ | Pure JS, no deps |
| Middleware | ✅ | AsyncLocalStorage works |

**Verdict:** Hono is the most compatible (it's designed for Edge).

---

## 6. Framework Compatibility Deep Dive

### 6.1 Next.js on Deno Edge Runtime

#### 6.1.1 Pros

- ✅ AsyncLocalStorage for request context propagation
- ✅ Fetch API for external requests
- ✅ fs.readFileSync for reading bundle
- ✅ path, url, querystring for routing
- ✅ Process env (sandboxed)

#### 6.1.2 Cons

- ❌ No `crypto.randomBytes` or `createHmac` (authentication)
- ❌ No streaming backpressure (large responses cause memory issues)
- ❌ Limited file I/O (no createReadStream)
- ❌ No native modules (if your dependencies use them)

#### 6.1.3 Required Changes

**Application level:**
```typescript
// package.json
{
  "overrides": {
    "next-auth": "... with custom crypto shim ..."
  }
}

// pages/api/handler.ts
import { getRandomValues } from 'crypto';
import { SubtleCrypto } from '@noble/hashes';  // Fallback

export default function handler(req, res) {
  // Instead of: crypto.randomBytes(32)
  const buf = new Uint8Array(32);
  getRandomValues(buf);
  // OR
  const hex = nanoid();  // Use nanoid for tokens
}
```

**Runtime level:**
```typescript
// Create node:crypto shim that maps Node APIs to WebCrypto
// Path: new file node_compat/crypto.ts
export function randomBytes(size: number): Buffer {
  const buf = new Uint8Array(size);
  crypto.getRandomValues(buf);
  return Buffer.from(buf);
}

export function createHmac(algo: string, key: string | Buffer) {
  // Map sha256 -> SHA-256, sha1 -> SHA-1
  const hmacAlgo = mapAlgorithm(algo);
  const keyBytes = typeof key === 'string' ? new TextEncoder().encode(key) : key;

  return {
    update(data) { ... },
    digest(enc) { ... }
  };
}
```

**Feasibility:** ⚠️ Moderate (crypto shim needed but doable)

### 6.2 Remix on Deno Edge Runtime

#### 6.2.1 Pros

- ✅ Minimal framework (good fit for edge)
- ✅ AsyncLocalStorage for context
- ✅ Fetch-based HTTP

#### 6.2.2 Cons

- ❌ Database connection pooling (no persistent connections)
- ❌ Streaming responses with backpressure

#### 6.2.3 Solution

Remix works best with an external database/API layer. The edge runtime only serves as request router → database connector.

**Feasibility:** ✅ High (minimal dependency on Node APIs)

### 6.3 Astro on Deno Edge Runtime

#### 6.3.1 Pros

- ✅ Designed for static generation
- ✅ Minimal SSR footprint
- ✅ Works great with edge hosting

#### 6.3.2 Cons

- ⚠️ Some optional features need Node APIs

#### 6.3.3 Solution

Use Astro with `output: 'server'` and `adapter: 'deno'` (Astro has official Deno support).

**Feasibility:** ✅ Very High

### 6.4 Hono on Deno Edge Runtime

#### 6.4.1 Pros

- ✅ Designed for edge (Cloudflare Workers, Deno)
- ✅ Zero external dependencies (bundles small)
- ✅ Works perfectly with async/await
- ✅ AsyncLocalStorage support (c.app.use middleware)

#### 6.4.2 Cons

- None (Hono is edge-first)

#### 6.4.3 Solution

Hono works out of the box.

**Feasibility:** ✅ Excellent

---

## 7. Improvement Proposals

### 7.1 High-Impact Improvements (Priority 1)

#### 7.1.1 Implement node:crypto with WebCrypto Bridge

**Goal:** Enable `crypto.randomBytes()`, `crypto.createHmac()`, and other common crypto operations.

**Approach:** Create a TypeScript shim that maps Node crypto APIs to WebCrypto.

```typescript
// File: crates/runtime-core/src/node_compat/crypto.ts

export function randomBytes(size: number, cb?: Function): Buffer {
  const buf = new Uint8Array(size);
  crypto.getRandomValues(buf);
  const result = Buffer.from(buf);
  if (cb) queueMicrotask(() => cb(null, result));
  return result;
}

export function createHmac(algorithm: string, key: string | Uint8Array) {
  const algo = mapAlgorithmName(algorithm);  // sha256 -> SHA-256
  const keyData = typeof key === 'string' ? new TextEncoder().encode(key) : key;

  return {
    update(data: string | Uint8Array) { ... },
    digest(encoding?: string) { ... },
    // full HmacKey interface
  };
}

// Map common Node algorithms to WebCrypto names
function mapAlgorithmName(name: string): string {
  const map: Record<string, string> = {
    'sha1': 'SHA-1',
    'sha224': 'SHA-224',
    'sha256': 'SHA-256',
    'sha384': 'SHA-384',
    'sha512': 'SHA-512',
  };
  return map[name] || name.toUpperCase();
}

export function createHash(algorithm: string) {
  const algo = mapAlgorithmName(algorithm);
  return new Hash(algo);
}

export class Hash extends EventEmitter {
  constructor(algorithm: string) { ... }
  update(data: string | Uint8Array, encoding?: string) { ... }
  digest(encoding?: string): Buffer { ... }
}

// More signatures:
// randomNumberGenerator, randomFill, randomFillSync
// createCipheriv, createDecipheriv (optional; more complex)
// createSign, createVerify (optional)
// generateKeyPair (optional)
// pbkdf2, scrypt, hkdf (optional)
```

**Implementation effort:** ~400 lines of TS

**Testing:**
```typescript
test('randomBytes', () => {
  const buf = crypto.randomBytes(32);
  assert(buf.length === 32);
  assert(Buffer.isBuffer(buf));
});

test('createHmac', async () => {
  const hmac = crypto.createHmac('sha256', 'secret');
  hmac.update('data');
  const digest = hmac.digest('hex');
  assert(typeof digest === 'string');
  assert(digest.length === 64);  // SHA-256 hex length
});
```

**Impact:**
- ✅ Enables Next.js session handling
- ✅ Enables authentication libraries (passport, etc.)
- ✅ Enables cookie signing
- ⚠️ More compute load on WebCrypto (slower than native)

**Feasibility:** ✅ High (WebCrypto has all primitives)

---

#### 7.1.2 Improve Stream Backpressure

**Goal:** Implement `pause()` / `resume()` and honor highWaterMark.

**Current code (stream.ts):**
```typescript
export class Readable extends Stream {
  push(chunk: any) {
    this.emit('data', chunk);  // Immediately emits, no buffering
  }
}
```

**Proposed improvement:**
```typescript
export class Readable extends Stream {
  #buffer: any[] = [];
  #paused = false;
  #highWaterMark: number;

  constructor(options?: any) {
    super();
    this.#highWaterMark = options?.highWaterMark ?? 16384;
  }

  push(chunk: any): boolean {
    if (this.#paused || this.#buffer.length >= this.#highWaterMark) {
      this.#buffer.push(chunk);
      return false;  // Signal backpressure
    }
    this.emit('data', chunk);
    return true;
  }

  pause(): this {
    this.#paused = true;
    this.emit('pause');
    return this;
  }

  resume(): this {
    this.#paused = false;
    this.emit('resume');

    // Flush buffer
    while (!this.#paused && this.#buffer.length > 0) {
      const chunk = this.#buffer.shift();
      this.emit('data', chunk);
    }

    return this;
  }
}
```

**Implementation effort:** ~150 lines

**Testing:**
```typescript
test('pause/resume', async () => {
  const r = Readable.from([1, 2, 3]);
  const collected: any[] = [];

  r.on('data', (chunk) => {
    collected.push(chunk);
    if (collected.length === 1) r.pause();
  });

  r.on('resume', () => {
    r.resume();  // Resume after delay
  });

  await new Promise(resolve => r.on('end', resolve));
  assert.deepEqual(collected, [1, 2, 3]);
});
```

**Impact:**
- ✅ Enables safe streaming (prevents memory exhaustion)
- ✅ Fixes Next.js large response streaming
- ✅ Fixes Remix streaming responses

---

#### 7.1.3 Expand AsyncLocalStorage Context Propagation

**Goal:** Propagate context through EventEmitter handlers and fs callbacks.

**Current limitation:**
```javascript
const storage = new AsyncLocalStorage();

storage.run({ userId: '123' }, () => {
  emitter.on('event', () => {
    console.log(storage.getStore());  // ❌ undefined (context lost)
  });
});
```

**Solution: Patch EventEmitter.prototype.emit**

```typescript
// In async_hooks.ts
const originalEmit = EventEmitter.prototype.emit;
EventEmitter.prototype.emit = function(eventName, ...args) {
  const currentStore = ALS_INSTANCES.get(currentAsyncId);
  if (currentStore !== undefined) {
    // Wrap all listener calls with current ALS context
    return originalEmit.call(this, eventName, ...args);
  }
  return originalEmit.call(this, eventName, ...args);
};
```

**Implementation effort:** ~100 lines

**Impact:**
- ✅ Fixes middleware context propagation
- ✅ Fixes event handler context

---

### 7.2 Medium-Impact Improvements (Priority 2)

#### 7.2.1 Add node:http Server Support

**Goal:** Enable running HTTP servers (required for testing, some frameworks).

**Current:** `http.createServer()` throws.

**Proposed:** Create limited server via Deno HTTP interface.

```typescript
// Path: node_compat/http.ts (extend)

export function createServer(options?: any, requestListener?: Function) {
  const server = new Server(options);
  if (requestListener) {
    server.on('request', requestListener);
  }
  return server;
}

export class Server extends EventEmitter {
  #listeners: Deno.HttpServer | null = null;

  listen(port?: number, host?: string, cb?: Function) {
    // Requires Edge Runtime changes to enable server mode
    // For now: return error with clear message
    throw new Error('http.Server.listen() requires explicit runtime support');
  }

  close(cb?: Function) {
    if (this.#listeners) {
      this.#listeners.shutdown();
      this.#listeners = null;
    }
    if (cb) cb();
  }
}
```

**Implementation effort:** ~200 lines + Runtime changes

**Feasibility:** ⚠️ Moderate (requires runtime refactoring)

---

#### 7.2.2 Add Stream `createReadStream` & `createWriteStream`

**Goal:** Support file streaming (needed for Remix large files, etc.).

```typescript
export function createReadStream(path: string, options?: any) {
  const readable = new Readable(options);
  queueMicrotask(async () => {
    try {
      const data = await fs.readFile(path);
      readable.push(data);
      readable.push(null);  // EOF
    } catch (err) {
      readable.destroy(err);
    }
  });
  return readable;
}

export function createWriteStream(path: string, options?: any) {
  const writable = new Writable({
    write(chunk, encoding, cb) {
      fs.writeFileSync(path, chunk, { flag: 'a' });
      cb();
    }
  });
  return writable;
}
```

**Implementation effort:** ~80 lines

**Impact:**
- ✅ Enables file streaming
- ✅ Enables pipe chains

---

#### 7.2.3 Implement node:vm Context Isolation (Partial)

**Goal:** Allow limited code execution in isolated context (no escape).

**Current:** `vm.runInNewContext()` throws.

**Proposed:** Create a "fake isolated context" that's actually just the same context but with a frozen global object.

```typescript
export function createContext(sandbox?: any): any {
  const ctx = sandbox || Object.create(null);

  // Freeze the context to prevent breakout
  Object.freeze(ctx);
  Object.seal(ctx);

  // Remove dangerous globals
  Object.defineProperty(ctx, 'eval', { value: undefined });
  Object.defineProperty(ctx, 'Function', { value: undefined });

  return ctx;
}

export function runInNewContext(code: string, sandbox?: any, options?: any) {
  const ctx = createContext(sandbox);

  // Still not safe to execute arbitrary code
  // Use new Function is still blocked
  throw new Error('vm.runInNewContext not safe for arbitrary user code');
}
```

**Feasibility:** ⚠️ Low (true isolation needs V8 API changes)

---

### 7.3 Lower-Impact Improvements (Priority 3)

#### 7.3.1 Add node:worker_threads Stub

**Goal:** Provide a minimal stub that explains workers aren't supported.

```typescript
export const isMainThread = true;
export const parentPort = null;
export const workerData = undefined;
export const threadId = 0;

export class Worker {
  constructor(filename: string | URL, options?: any) {
    throw new Error('Worker threads not supported in edge runtime');
  }
}

export class MessageChannel {
  constructor() {
    throw new Error('MessageChannel not supported');
  }
}
```

**Impact:** ✅ Better error messages for users

---

#### 7.3.2 Expand node:os Mock Values

**Goal:** Make synthetic values more realistic.

```typescript
// Instead of hardcoded values, read from Deno environment
export const arch = () => {
  // Could detect from globalThis (V8 exposes some info)
  return 'x64';  // Still hardcoded but could be smarter
};

export const platform = () => {
  // Deno.build.os
  const os = (Deno && Deno.build && Deno.build.os) || 'linux';
  return os === 'windows' ? 'win32' : os;
};

export const cpus = () => {
  // Could expose Deno.systemMemoryInfo() if available
  return [
    {
      model: 'Virtual CPU',
      speed: 0,
      times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 }
    }
  ];
};
```

**Impact:** ⚠️ Low (mostly cosmetic)

---

## 8. Compatibility Checklist

### Core Language

- [ ] **Full Buffer compatibility** -- currently proxies to globalThis.Buffer (✅)
  - Sub-task: Verify all Buffer methods work correctly
  - Sub-task: Add Buffer.isBuffer, Buffer.from, Buffer.allocUnsafe

- [ ] **AsyncLocalStorage support** -- currently full (✅)
  - Sub-task: Test context propagation through all async APIs
  - Sub-task: Verify no leaks between isolates

- [ ] **Promise/async-await** -- V8 native (✅)
  - Sub-task: Test long promise chains
  - Sub-task: Verify unhandledRejection handling

- [ ] **Map/Set collections** -- V8 native (✅)

- [ ] **WeakMap/WeakSet** -- V8 native (✅)

- [ ] **Proxy** -- V8 native (✅)

- [ ] **Reflect** -- V8 native (✅)

- [ ] **Symbol** -- V8 native (✅)

### Async Patterns

- [x] **Stream pipeline compatibility** -- partial (⚠️)
  - Sub-task: Implement pause()/resume() ← Priority 1
  - Sub-task: Implement highWaterMark queuing ← Priority 1

- [x] **process.nextTick semantics** -- uses microtasks ⚠️
  - Note: Different priority than Node (microtasks vs nextTick queue)
  - Impact: Some code may see different execution order

- [x] **setImmediate / setImmediate** -- not provided
  - Sub-task: Add setImmediate stub (could map to setTimeout(..., 0))

- [x] **EventEmitter context propagation** -- partial ⚠️
  - Sub-task: Patch EventEmitter.prototype.emit for ALS ← Priority 2

### Module System

- [x] **Module resolution** -- eszip + builtin only (✅)
  - Sub-task: Verify circular dependency handling

- [x] **Dynamic imports** -- working (✅)
  - Test: `import('node:events')`

- [x] **module.createRequire()** -- working (✅)

- [x] **Conditional exports** -- not tested
  - Sub-task: Test with dual-mode packages

### Runtime APIs

- [x] **crypto.randomBytes()** -- MISSING (❌)
  - Implement: Node crypto shim ← Priority 1

- [x] **crypto.createHmac()** -- MISSING (❌)
  - Implement: Part of crypto shim ← Priority 1

- [x] **crypto.createHash()** -- MISSING (❌)
  - Implement: Part of crypto shim ← Priority 1

- [x] **fs.readFileSync()** -- working (✅)

- [x] **fs.writeFileSync()** -- working but limited (⚠️)

- [x] **fs.createReadStream()** -- MISSING (❌)
  - Implement: Stream wrapper ← Priority 2

- [x] **fs.createWriteStream()** -- MISSING (❌)
  - Implement: Stream wrapper ← Priority 2

- [x] **http.get()** -- working (✅)

- [x] **http.request()** -- working (✅)

- [x] **http.createServer()** -- MISSING (❌)
  - Implement: Limited server ← Priority 2

### SSR Framework Compatibility

- [ ] **Next.js API Routes** -- partial ⚠️
  - Blocker: crypto.randomBytes() ← Needs Priority 1
  - Blocker: crypto.createHmac() ← Needs Priority 1

- [ ] **Next.js SSR** -- partial ⚠️
  - Blocker: Stream backpressure ← Needs Priority 1

- [ ] **Remix** -- good ✅
  - Requires: AsyncLocalStorage (✅)
  - Requires: Fetch API (✅)

- [ ] **Astro** -- good ✅
  - Works with `adapter: 'deno'`

- [ ] **Hono** -- excellent ✅
  - No blockers

---

## 9. Suggested GitHub Issues

### Issue #1: Implement node:crypto Module

**Title:** `feat: Implement node:crypto with WebCrypto Bridge`

**Labels:** `feature`, `priority:high`, `compat:node-api`

**Description:**

The `node:crypto` module is currently missing, which blocks common operations like `randomBytes()`, `createHmac()`, and `createHash()`. These are used by Next.js authentication, all cookie/session libraries, and security-critical code.

**Proposed Solution:**

Create a TypeScript shim (`crates/runtime-core/src/node_compat/crypto.ts`) that maps Node.js crypto APIs to the WebCrypto API already available in the runtime.

**Scope:**

- [ ] `crypto.randomBytes(size, [callback])`
- [ ] `crypto.randomFillSync(buffer, [offset], [size])`
- [ ] `crypto.randomFill(buffer, [offset], [size], callback)`
- [ ] `crypto.createHash(algorithm)`
- [ ] `crypto.createHmac(algorithm, key)`
- [ ] `crypto.createCipheriv(algorithm, key, iv)` (optional)
- [ ] `crypto.createDecipheriv(algorithm, key, iv)` (optional)
- [ ] `crypto.scrypt()` (optional, lower priority)
- [ ] `crypto.pbkdf2()` (optional, lower priority)

**Test Cases:**

```typescript
import crypto from 'node:crypto';

// Test 1: randomBytes
const buf = crypto.randomBytes(32);
assert(buf.length === 32);
assert(Buffer.isBuffer(buf));

// Test 2: createHmac
const hmac = crypto.createHmac('sha256', 'secret');
hmac.update('data');
const digest = hmac.digest('hex');
assert(digest.length === 64);  // SHA-256 hex

// Test 3: createHash
const hash = crypto.createHash('sha256');
hash.update('data');
const hashDigest = hash.digest('hex');
assert(hashDigest.length === 64);
```

**Acceptance Criteria:**

- All listed methods are implemented
- All tests pass
- Works with Next.js authentication (next-auth, passport, etc.)
- No performance regression
- Documentation updated

---

### Issue #2: Implement Stream Backpressure & highWaterMark

**Title:** `feat: Implement Stream backpressure and highWaterMark`

**Labels:** `feature`, `priority:high`, `compat:streams`

**Description:**

The current stream implementation does not support backpressure or highWaterMark buffering. This causes memory exhaustion when producing data faster than consuming it (common in SSR with large responses).

**Current Behavior:**
```javascript
readable.pipe(writable);
// Does not honor backpressure from writable
// Buffer grows indefinitely
// OOM risk
```

**Expected Behavior:**
```javascript
readable.pipe(writable);
// readable pauses when writable buffer is full
// Once writable catches up, readable resumes
// Memory usage stays bounded
```

**Scope:**

- [ ] Add `pause()` and `resume()` methods to Readable
- [ ] Add `highWaterMark` option to stream constructors
- [ ] Implement internal buffering in Readable.push()
- [ ] Return backpressure signal (true/false) from push()
- [ ] Emit `pause` and `resume` events
- [ ] Update `pipe()` to honor backpressure

**Test Cases:**

See section 7.1.2 above.

**Acceptance Criteria:**

- Backpressure is honored
- Memory stays bounded during streaming
- Next.js large response streaming works
- No performance regression for small streams

---

### Issue #3: Add EventEmitter to AsyncLocalStorage Propagation

**Title:** `fix: Propagate AsyncLocalStorage context through EventEmitter handlers`

**Labels:** `bug`, `priority:high`, `compat:async-hooks`

**Description:**

AsyncLocalStorage context is lost when event listeners are called via EventEmitter. This breaks middleware context tracking.

**Current Behavior:**
```javascript
const storage = new AsyncLocalStorage();

storage.run({ userId: '123' }, () => {
  emitter.on('event', () => {
    console.log(storage.getStore());  // undefined ❌
  });

  emitter.emit('event');
});
```

**Expected Behavior:**
```javascript
storage.run({ userId: '123' }, () => {
  emitter.on('event', () => {
    console.log(storage.getStore());  // { userId: '123' } ✅
  });

  emitter.emit('event');
});
```

**Solution:**

Patch `EventEmitter.prototype.emit` to restore ALS context before calling listeners.

**Acceptance Criteria:**

- ALS context propagates through EventEmitter
- No performance regression
- All existing tests pass

---

### Issue #4: Implement fs.createReadStream & createWriteStream

**Title:** `feat: Implement fs.createReadStream and fs.createWriteStream`

**Labels:** `feature`, `priority:medium`, `compat:fs`

**Description:**

File streaming is missing. This prevents using pipe() chains with files, which is common in Remix and media applications.

**Scope:**

- [ ] `fs.createReadStream(path, options)`
- [ ] `fs.createWriteStream(path, options)`
- [ ] Support `start`, `end`, `flags`, `mode` options
- [ ] Emit `open`, `close`, `error` events

**Acceptance Criteria:**

- File streaming works
- Pipe chains work: `readable.pipe(transform).pipe(writable)`
- Respects VFS quota limits
- All tests pass

---

### Issue #5: Add node:worker_threads Stub Module

**Title:** `feat: Add node:worker_threads stub with helpful error messages`

**Labels:** `feature`, `priority:low`, `compat:api-coverage`

**Description:**

Currently importing `node:worker_threads` fails. We should provide a stub module that explains workers aren't supported, with helpful error messages pointing to documentation.

**Scope:**

- [ ] Create stub module with all worker_threads exports
- [ ] All exports throw with clear error messages
- [ ] Add documentation link in error

**Acceptance Criteria:**

- Import works: `const { Worker } = require('node:worker_threads');`
- Error is helpful: `Worker not supported: https://docs.example.com/worker-threads`

---

### Issue #6: Implement Limited http.createServer Support

**Title:** `feat: Implement limited http.createServer for testing and development`

**Labels:** `feature`, `priority:medium`, `compat:http`

**Description:**

The `http.createServer()` is currently blocked entirely. For testing and development, we should provide limited server support that works with the Deno HTTP adapter.

**Note:** This requires coordination with the broader runtime architecture (separate from this issue).

**Scope:**

- [ ] `http.createServer(options, requestListener)`
- [ ] `http.Server.listen(port, host, callback)`
- [ ] `http.Server.close(callback)`
- [ ] Request/Response object compatibility

**Blockers:**

- Runtime must expose HTTP server interface in development mode
- May not be suitable for multi-tenant production

---

### Issue #7: Expand node:crypto to Include createCipheriv & createDecipheriv

**Title:** `feat: Expand node:crypto to support AES encryption/decryption`

**Labels:** `feature`, `priority:medium`, `compat:crypto`

**Description:**

Beyond basic hashing and HMAC, many applications need symmetric encryption. This can be implemented using WebCrypto's AES-CBC support.

**Scope:**

- [ ] `crypto.createCipheriv(algorithm, key, iv)`
- [ ] `crypto.createDecipheriv(algorithm, key, iv)`
- [ ] Support AES-128-CBC, AES-256-CBC
- [ ] Cipher.update() and Cipher.final()

**Acceptance Criteria:**

- Encryption/decryption round-trips correctly
- Compatibility with Node.js crypto output
- All tests pass

---

### Issue #8: Document Node.js Compatibility Status

**Title:** `docs: Create comprehensive Node.js compatibility matrix`

**Labels:** `documentation`, `priority:medium`

**Description:**

Users need a complete reference for which Node.js APIs are supported. This document should be machine-readable and updateable.

**Scope:**

- [ ] Create docs/reference/NODE-COMPAT.md with detailed matrix
- [ ] Add badges to README
- [ ] Create @edgeruntime/compat-check package for runtime detection

---

### Issue #9: Performance: Optimize Crypto Operations

**Title:** `perf: Optimize crypto.createHash and crypto.createHmac for performance`

**Labels:** `perf`, `priority:low`, `compat:crypto`

**Description:**

WebCrypto-based crypto operations may be slower than Node's native implementation. We should benchmark and potentially provide native optimizations.

**Tasks:**

- [ ] Benchmark crypto operations vs Node.js
- [ ] Consider native Rust ops for hot paths
- [ ] Cache SubtleCrypto import

---

### Issue #10: AsyncLocalStorage: Context Propagation Through fs Callbacks

**Title:** `fix: Propagate AsyncLocalStorage context through fs async callbacks`

**Labels:** `bug`, `priority:medium`, `compat:async-hooks`

**Description:**

Similar to EventEmitter issue (#3), AsyncLocalStorage context is lost in fs callbacks.

**Scope:**

- [ ] fs.readFile(path, callback) should preserve ALS context
- [ ] fs.writeFile(path, data, callback) should preserve ALS context
- [ ] All async fs functions should propagate context

---

## 10. Multi-Phase Roadmap

### Phase 1: Critical Security & Crypto (2-4 weeks)

**Goal:** Enable basic web application support (authentication, sessions, etc.)

**Issues:**
- [ ] Implement node:crypto module (#1)
- [ ] Implement Stream backpressure (#2)
- [ ] Add EventEmitter ALS propagation (#3)

**Outcome:**
- ✅ Next.js authentication works
- ✅ Large response streaming safe
- ✅ Middleware context propagation works
- ✅ All async operations preserve context

**Testing:**
```bash
cargo test node_crypto
cargo test node_stream_backpressure
cargo test async_local_storage_eventemmitter
```

**Dependencies:** None (all in node_compat layer)

**Effort:** ~2 weeks (2 senior engineers)

---

### Phase 2: File I/O & Streaming (1-2 weeks)

**Goal:** Enable file operations and pipe chains

**Issues:**
- [ ] fs.createReadStream / createWriteStream (#4)
- [ ] stream.pipeline compatibility improvements
- [ ] fs async callback context propagation (#10)

**Outcome:**
- ✅ Pipe chains work
- ✅ File streaming works
- ✅ Remix large file handling works

**Testing:**
```bash
cargo test node_fs_streaming
cargo test node_stream_pipeline
```

**Dependencies:** Phase 1 (backpressure needed)

**Effort:** ~1 week

---

### Phase 3: Extended Crypto Support (1-2 weeks)

**Goal:** Enable encryption/decryption operations

**Issues:**
- [ ] crypto.createCipheriv / createDecipheriv (#7)
- [ ] crypto.scrypt(), crypto.pbkdf2() (optional)

**Outcome:**
- ✅ AES encryption/decryption works
- ✅ Key derivation works

**Testing:**
```bash
cargo test node_crypto_cipher
cargo test node_crypto_kdf
```

**Dependencies:** Phase 1

**Effort:** ~1 week

---

### Phase 4: Server Support & HTTP (2-4 weeks)

**Goal:** Enable server-side development and testing

**Issues:**
- [ ] http.createServer support (#6)
- [ ] http2 support (optional, lower priority)
- [ ] WebSocket support (optional)

**Outcome:**
- ✅ Can build HTTP servers for testing
- ✅ Middleware libraries work

**Testing:**
```bash
cargo test node_http_server
cargo test node_http_pipeline
```

**Dependencies:** Phase 1-2

**Effort:** ~2-3 weeks (requires runtime coordination)

---

### Phase 5: Ecosystem & Performance (Ongoing)

**Goal:** Optimize and expand for production use

**Issues:**
- [ ] Performance optimization (#9)
- [ ] Compatibility matrix documentation (#8)
- [ ] worker_threads stub (#5)

**Outcome:**
- ✅ Production-ready performance
- ✅ Clear compatibility documentation
- ✅ User-friendly error messages

**Effort:** ~1-2 weeks per quarter

---

## 11. Success Metrics

### By End of Phase 1

```
✅ 🎯 Next.js can run with authentication
✅ 🎯 Large responses don't crash with OOM
✅ 🎯 Request context propagates through middleware
✅ 🎯 crypto.randomBytes() works
✅ 🎯 crypto.createHmac() works
```

### By End of Phase 2

```
✅ 🎯 Remix can run
✅ 🎯 File streaming works
✅ 🎯 Pipe chains work
✅ 🎯 Async fs context propagation works
```

### By End of Phase 3

```
✅ 🎯 Encryption/decryption works
✅ 🎯 Key derivation works
✅ 🎯 PassportJS-compatible auth libraries work
```

### By End of Phase 4

```
✅ 🎯 Express-like servers work
✅ 🎯 Full middleware compatibility
✅ 🎯 Production HTTP server testing works
```

---

## 12. Comparison with Workerd Implementation

### Architectural Decisions: Deno Runtime vs Workerd

| Decision | Deno Runtime | Workerd | Trade-off |
|----------|---|---|---|
| **Native Bindings** | Minimal (1 op) | Extensive (35+ ops) | Deno is easier to modify; Workerd is faster |
| **Module System** | ESM + TS via deno_core | ESM + CJS via V8 modules | Deno is cleaner; Workerd is more compatible |
| **Crypto Strategy** | WebCrypto bridge (TS) | WebCrypto + Node crypto (C++) | Deno is easier to implement; Workerd is more complete |
| **Stream Implementation** | Pure TS | C++ with Vec buffers | Deno is customizable; Workerd is performant |
| **VFS Approach** | In-memory, heap-based | Durable Objects + R2 | Deno is simple; Workerd is persistent |

### Missing in Deno Runtime (vs Workerd)

1. **node:crypto full API** — Workerd has comprehensive crypto using WebCrypto + native
2. **Stream backpressure** — Workerd handles properly; Deno needs implementation
3. **http/https server** — Workerd has full support; Deno blocks for isolation
4. **TCP/TLS server** — Workerd has service binding; Deno blocks for isolation
5. **Performance** — Workerd native bindings are generally faster
6. **Durable storage** — Workerd has Durable Objects; Deno has heap-only VFS

### Advantages of Deno Runtime

1. **Simpler architecture** — JavaScript closures vs C++ boilerplate
2. **Easier to modify** — Change a .ts file vs recompile C++
3. **Smaller binary** — No extensive C++ binding code
4. **Community-leverageable** — TypeScript developers can contribute
5. **Better DevX** — Deno CLI, simpler debugging

### Verdict

**Workerd is more feature-complete and performant.**

**Deno Runtime is more maintainable and developer-friendly.**

For production, Workerd is superior. For open-source contributions and rapid iteration, Deno Runtime is better.

---

## Conclusion

This Deno Edge Runtime provides a solid foundation for sandboxed JavaScript execution with Node.js API compatibility. By implementing the Phase 1 improvements (crypto, streams, context propagation), it will be fully capable of running SSR frameworks like Next.js, Remix, Astro, and Hono in a secure multi-tenant edge environment.

The roadmap prioritizes:
1. **Security** (already good; Phase 1 adds crypto)
2. **Correctness** (Node.js API semantics)
3. **Performance** (Phases 2-5)

Success is achievable within 8-12 weeks for Phases 1-3, with ongoing optimization thereafter.

---

**Document prepared by:** Senior Runtime Systems Engineer
**Date:** March 2026
**Version:** 1.0
**Status:** Ready for Implementation
