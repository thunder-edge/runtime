# Request Timeout and Resource Tracking

This document describes the request timeout mechanism and resource tracking system implemented in the Deno Edge Runtime.

## Overview

The runtime implements a comprehensive system to:

1. **Enforce wall-clock timeouts** on request execution
2. **Actively terminate V8 execution** when timeouts occur (not just return errors)
3. **Track per-request resources** (timers, intervals, fetch requests, promises)
4. **Clean up resources** when requests complete or timeout
5. **Preserve isolate reusability** after timeout events

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         Request Lifecycle                             │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│   ┌─────────────┐                       ┌─────────────────────┐      │
│   │   Request   │──── dispatch ────────▶│   Isolate Thread    │      │
│   └─────────────┘                       │                     │      │
│                                         │  startExecution(id) │      │
│   ┌─────────────┐                       │         │           │      │
│   │  Watchdog   │◀──── spawn ──────────│         ▼           │      │
│   │   Thread    │                       │  dispatch_request() │      │
│   │             │                       │         │           │      │
│   │   wait...   │                       │   (JS execution)    │      │
│   │             │                       │         │           │      │
│   │  timeout?   │── terminate_exec() ──▶│    [TERMINATED]     │      │
│   └─────────────┘                       │         │           │      │
│                                         │         ▼           │      │
│                                         │ clearExecutionTimers│      │
│                                         │         │           │      │
│                                         │ cancel_terminate()  │      │
│                                         │         │           │      │
│                                         │   endExecution(id)  │      │
│                                         └─────────────────────┘      │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

## Configuration

### CLI Options

```bash
thunder start --wall-clock-timeout-ms 60000
```

### Environment Variables

```bash
export EDGE_RUNTIME_WALL_CLOCK_TIMEOUT_MS=60000
```

### Default Values

| Option | Default | Description |
|--------|---------|-------------|
| `--wall-clock-timeout-ms` | `60000` (60s) | Maximum wall-clock time per request |

Setting to `0` disables the timeout (not recommended for production).

## How It Works

### 1. Execution Context

Each request gets a unique execution ID:

```rust
// lifecycle.rs
let execution_id = uuid::Uuid::new_v4().to_string();
js_runtime.execute_script(
    "edge-internal:///start_execution.js",
    format!(r#"globalThis.__edgeRuntime.startExecution("{}");"#, execution_id),
);
```

### 2. Resource Tracking

The JavaScript bridge tracks resources per execution ID:

```javascript
globalThis.__edgeRuntime = {
    _timerRegistry: new Map(),      // executionId -> Set<timerId>
    _intervalRegistry: new Map(),   // executionId -> Set<intervalId>
    _abortRegistry: new Map(),      // executionId -> Set<AbortController>
    _promiseRegistry: new Map(),    // executionId -> Set<{promise, reject}>
    // ...
};
```

### 3. Wrapped Functions

Standard functions are wrapped to enable tracking:

| Original Function | Wrapped Behavior |
|-------------------|-----------------|
| `setTimeout()` | Registers timer ID with current execution |
| `setInterval()` | Registers interval ID with current execution |
| `clearTimeout()` | Removes from registry + calls original |
| `clearInterval()` | Removes from registry + calls original |
| `fetch()` | Wraps with AbortController for cancellation |
| `queueMicrotask()` | Tracks execution context |

### 4. Watchdog Thread

When timeout is configured, a watchdog thread monitors execution:

```rust
let watchdog = std::thread::spawn(move || {
    let deadline = std::time::Instant::now() + timeout_duration;

    while std::time::Instant::now() < deadline {
        if request_completed.load(Ordering::SeqCst) {
            return; // Request completed normally
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Deadline reached - terminate V8
    v8_handle.terminate_execution();
});
```

### 5. Timeout Handling

When timeout occurs:

1. **V8 Termination**: `v8_handle.terminate_execution()` forcefully stops JS execution
2. **State Reset**: `js_runtime.v8_isolate().cancel_terminate_execution()` allows isolate reuse
3. **Resource Cleanup**: `clearExecutionTimers(executionId)` cancels all tracked resources
4. **Error Response**: Returns 504-style timeout error

### 6. Resource Cleanup

The `clearExecutionTimers()` function:

```javascript
clearExecutionTimers(executionId) {
    // Clear setTimeout timers
    const timers = this._timerRegistry.get(executionId);
    if (timers) {
        for (const id of timers) {
            globalThis.__originalClearTimeout(id);
        }
    }

    // Clear setInterval intervals
    const intervals = this._intervalRegistry.get(executionId);
    if (intervals) {
        for (const id of intervals) {
            globalThis.__originalClearInterval(id);
        }
    }

    // Abort pending fetch requests
    const abortControllers = this._abortRegistry.get(executionId);
    if (abortControllers) {
        for (const controller of abortControllers) {
            controller.abort(new Error('Request cancelled due to execution timeout'));
        }
    }

    // Reject pending tracked promises
    const promises = this._promiseRegistry.get(executionId);
    if (promises) {
        for (const entry of promises) {
            entry.reject(new Error('Promise cancelled due to execution timeout'));
        }
    }

    // Cleanup registries
    this._timerRegistry.delete(executionId);
    this._intervalRegistry.delete(executionId);
    this._abortRegistry.delete(executionId);
    this._promiseRegistry.delete(executionId);
}
```

## Request Isolation

Resources are isolated per-request by execution ID:

```
Request A (exec-id: "abc-123")     Request B (exec-id: "def-456")
┌─────────────────────────┐       ┌─────────────────────────┐
│ Timers: {1, 2, 3}       │       │ Timers: {4, 5}          │
│ Intervals: {10}         │       │ Intervals: {}           │
│ AbortControllers: {ac1} │       │ AbortControllers: {ac2} │
└─────────────────────────┘       └─────────────────────────┘

// Clearing Request A does NOT affect Request B
clearExecutionTimers("abc-123")  // Only clears timers 1,2,3 and interval 10
```

## Isolate Reuse

After a timeout, the isolate remains usable:

1. V8 termination is "soft" - it throws an exception but doesn't corrupt state
2. `cancel_terminate_execution()` resets the termination flag
3. Resource cleanup removes dangling timers/intervals
4. Subsequent requests can execute normally

```rust
// After timeout:
js_runtime.v8_isolate().cancel_terminate_execution();
js_runtime.execute_script(...);  // Works!
```

## Comparison with Cloudflare Workers

| Aspect | Cloudflare Workers | Deno Edge Runtime |
|--------|-------------------|-------------------|
| CPU Timeout | Hard kill, new isolate | Soft termination, reuse isolate |
| Memory Limit | Hard kill, new isolate | (Future: near-heap-limit callback) |
| Resource Tracking | Implicit per-request | Explicit registry per execution ID |
| Fetch Cancellation | Automatic | Via AbortController tracking |
| Timer Cleanup | Automatic per-isolate | Explicit per-execution |

## Best Practices

### For Function Authors

1. **Avoid infinite loops**: They will be terminated

   ```javascript
   // BAD: Will be terminated
   while (true) { }

   // GOOD: Use bounded iterations or timeouts
   for (let i = 0; i < 1000; i++) { }
   ```

2. **Handle abort signals in fetch**: The runtime will abort, but graceful handling is better

   ```javascript
   const controller = new AbortController();
   const timeoutId = setTimeout(() => controller.abort(), 5000);

   try {
       const response = await fetch(url, { signal: controller.signal });
       clearTimeout(timeoutId);
       return response;
   } catch (e) {
       if (e.name === 'AbortError') {
           return new Response('Request timed out', { status: 504 });
       }
       throw e;
   }
   ```

3. **Clean up intervals**: While the runtime cleans up on timeout, explicit cleanup is cleaner

   ```javascript
   const intervalId = setInterval(() => { }, 1000);

   // Always clean up when done
   clearInterval(intervalId);
   ```

### For Operators

1. **Set appropriate timeouts**:
   - Development: Higher timeouts for debugging
   - Production: Lower timeouts (10-30s) for resource protection

2. **Monitor timeout metrics**:
   - `metrics.total_errors` increments on timeout
   - Log messages indicate which functions timeout

3. **Consider load patterns**:
   - Timeout protects against denial-of-service
   - Adjust based on expected function runtime

## Testing

Tests are located in `crates/functions/tests/timeout_and_timers.rs`:

| Test | Description |
|------|-------------|
| `test_terminate_execution_stops_infinite_loop` | V8 termination works |
| `test_timer_tracking_registration` | setTimeout tracked |
| `test_interval_tracking_registration` | setInterval tracked |
| `test_timer_isolation_between_executions` | Isolation per request |
| `test_isolate_reusable_after_timeout` | Reuse after timeout |
| `test_fetch_abort_controller_tracking` | AbortController registry |
| `test_promise_tracking_registration` | Promise registry |
| `test_original_functions_preserved` | Original functions accessible |
| `test_clear_timeout_removes_from_registry` | clearTimeout updates registry |
| `test_clear_interval_removes_from_registry` | clearInterval updates registry |
| `test_multiple_requests_after_timeout` | Multiple requests after recovery |
| `test_nested_timers_tracking` | Multiple timers/intervals |
| `test_timer_callback_removes_from_registry` | Callback execution cleanup |

Run tests:

```bash
cargo test -p functions timeout_and_timers
```

## Files Reference

| File | Purpose |
|------|---------|
| `crates/functions/src/lifecycle.rs` | Isolate lifecycle, watchdog spawn, timeout handling |
| `crates/functions/src/handler.rs` | JS bridge, resource tracking, wrapped functions |
| `crates/runtime-core/src/isolate.rs` | IsolateConfig definition |

## Sequence Diagram

```
     Client              Router             Isolate Thread        Watchdog Thread
        │                   │                     │                     │
        │── HTTP Request ──▶│                     │                     │
        │                   │                     │                     │
        │                   │── send_request() ──▶│                     │
        │                   │                     │                     │
        │                   │                     │── spawn ───────────▶│
        │                   │                     │                     │
        │                   │                     │──startExecution()   │
        │                   │                     │                     │
        │                   │                     │──dispatch_request() │
        │                   │                     │      │              │
        │                   │                     │   (JS running)      │── wait...
        │                   │                     │      │              │
        │                   │                     │      │              │── deadline!
        │                   │                     │◀─────────────────────│ terminate!
        │                   │                     │      │              │
        │                   │                     │  [TERMINATED]       │
        │                   │                     │      │              │
        │                   │                     │──cancel_terminate() │
        │                   │                     │      │              │
        │                   │                     │──clearExecutionTimers()
        │                   │                     │      │              │
        │                   │◀── Error Response ──│      │              │
        │                   │                     │      │              │
        │◀── 504 Timeout ───│                     │      │              │
        │                   │                     │      │              │
```

## Related Documentation

- [CLI Reference](../reference/cli.md) - Command line options including `--wall-clock-timeout-ms`
- [Load Testing](../guides/load_testing.md) - Performance testing guidance
