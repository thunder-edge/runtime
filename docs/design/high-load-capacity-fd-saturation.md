# High Load Capacity, FD, and Saturation Guide

This guide documents how the runtime behaves under high load, how file descriptors (FD) and connection limits interact, and how to tune queueing and timeout controls to reduce request failures.

Scope:
- Ingress and admin listener connection limits
- Process FD budget and RLIMIT_NOFILE
- Context/isolate saturation and scale blocking
- Queueing behavior on temporary capacity exhaustion
- Tuning strategies and tradeoffs

## 1. Mental Model

High-load stability is controlled by four layers:

1. OS FD ceiling (RLIMIT_NOFILE)
2. Listener connection concurrency (`EDGE_RUNTIME_MAX_CONNECTIONS`, clamped by FD budget)
3. Function execution capacity (isolates, contexts, active requests per context)
4. Queueing and timeout behavior when execution capacity is temporarily exhausted

Most high-load incidents are not a single bottleneck. They are a chain reaction across these four layers.

## 2. FD and RLIMIT_NOFILE

The runtime uses a best-effort startup raise for `RLIMIT_NOFILE`.

- Env: `EDGE_RUNTIME_NOFILE_TARGET`
- Default target: `10000`
- Behavior:
  - Reads current soft/hard limits
  - If soft < target and hard allows, calls `setrlimit`
  - If target exceeds hard limit, logs warning and continues with existing soft limit

Why this matters:
- Listener accepts, outbound sockets, and internal runtime handles all consume FDs.
- A high `max_connections` value is meaningless if effective FD budget is lower.

## 3. Listener Connection Capacity and Clamp

The runtime computes listener capacity from configured connections plus FD budget.

Inputs:
- Configured: `EDGE_RUNTIME_MAX_CONNECTIONS` (default `50000`)
- Soft limit: current `RLIMIT_NOFILE`
- Reserved FD strategy:
  - ratio reserve: 10% of soft limit
  - absolute reserve: 64
  - reserved = `max(64, round(soft_limit * 0.10))`, capped to keep at least 32 FDs available

Derived values in metrics:
- `listener_connection_capacity.configuredMaxConnections`
- `listener_connection_capacity.effectiveMaxConnections`
- `listener_connection_capacity.softLimit`
- `listener_connection_capacity.reservedFd`
- `listener_connection_capacity.fdBudget`

Behavior:
- If configured > FD budget, runtime clamps to `effectiveMaxConnections`.
- Runtime logs `max_connections clamped by RLIMIT_NOFILE budget`.

Tradeoff:
- Higher configured max can improve throughput only if FD and CPU headroom exist.
- Over-configuring without FD headroom only increases refusal pressure.

## 4. EMFILE/ENFILE Handling

When accept fails with FD exhaustion (`EMFILE`/`ENFILE`):
- Runtime logs explicit refusal message
- Listener loop backs off for `50ms`

This avoids tight error loops and protects process responsiveness.

Tradeoff:
- Backoff reduces CPU thrash and log storms.
- During persistent exhaustion, some incoming connections are still refused until FD pressure drops.

## 5. Permit Wait Timeout at Listener Gate

Before serving a connection, listeners wait for a semaphore permit.

- Internal wait timeout: `500ms`
- On timeout, connection is refused by runtime

Why this exists:
- Prevents unbounded waiting at accept boundary.
- Keeps process behavior deterministic under overload.

Tradeoff:
- Lower timeout: faster fail, lower tail latency amplification, potentially higher refusal rate.
- Higher timeout: more smoothing under bursts, potentially higher queueing latency.

## 6. Execution Capacity: Isolates, Contexts, and Active Requests

Execution-side knobs:
- `EDGE_RUNTIME_POOL_ENABLED` (default `true`)
- `EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES` (default `64`)
- `EDGE_RUNTIME_POOL_MIN_ISOLATES` (default `5`)
- `EDGE_RUNTIME_POOL_MAX_ISOLATES` (default `10`)
- `EDGE_RUNTIME_CONTEXT_POOL_ENABLED` (default `true`)
- `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE` (default `64`)
- `EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT` (default `8`)
- `EDGE_RUNTIME_CONTEXT_POOL_MIN_CONTEXTS` (default `1`)
- `EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS` (default `256`)

Effective rough capacity per function is bounded by:
- contexts <= `context_pool_max_contexts`
- contexts per isolate <= `max_contexts_per_isolate`
- active req <= `contexts * max_active_requests_per_context`
- isolate creation can still be blocked by global isolate limit or memory guardrail

Common saturation signature:
- `routing.total_contexts` plateau (for example fixed at 64 before tuning)
- rising `routing.saturated_rejections`
- rising reason-specific counters (below)

## 7. Saturation Counters and Reasons

The metrics endpoint includes these fields:

`routing`:
- `total_contexts`
- `total_isolates`
- `total_active_requests`
- `saturated_rejections`
- `saturated_rejections_context_capacity`
- `saturated_rejections_scale_blocked`
- `saturated_rejections_scale_failed`
- `saturated_contexts`
- `saturated_isolates`

How to interpret:
- High `...context_capacity`: context cap is too low for demand.
- High `...scale_blocked`: runtime wanted to scale but was blocked (global isolate or guardrails).
- High `...scale_failed`: attempted scale operation failed.

## 8. Queueing on Temporary Capacity Exhaustion

When route targeting returns temporary `CapacityExhausted`, requests can wait briefly instead of immediate reject.

Queue controls:
- `EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS` (default `300`)
- `EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS` (default `20000`)

Behavior:
- Request waits in short intervals for released capacity or successful scale
- Stops waiting at timeout deadline
- Stops waiting if waiter cap is reached
- Returns capacity error (usually reflected as 503) when deadline/cap is hit

Tradeoff:
- Higher wait timeout:
  - Pros: fewer burst-time failures
  - Cons: higher tail latency during sustained overload
- Higher max waiters:
  - Pros: absorbs larger bursts
  - Cons: higher memory pressure and latency spread

Practical guidance:
- For bursty traffic, increase timeout moderately (for example 300-500ms).
- For sustained overload, avoid masking hard capacity limits with very large queue timeout.

## 8.1 Scale-Down Cooldown Semantics

Context/isolate shrink after burst is governed by cooldown timers:
- context downscale cooldown: `5s`
- isolate downscale cooldown: `10s`

Important behavior detail:
- Downscale is currently executed on request release events, not by a periodic background cleanup loop.
- If traffic goes to zero immediately after burst, context/isolate counts can remain elevated until a new release path is triggered.

Current downscale gates:
- Context removal requires idle context + cooldown elapsed.
- Isolate removal requires no active contexts on that isolate + cooldown elapsed.
- Capacity waiter count is used to prefer burst scale-up decisions, but is not a direct blocker for downscale evaluation.

## 9. High-Load Tuning Profiles

These are starting points, not universal values.

### Profile A: Balanced (default-ish)
- `EDGE_RUNTIME_NOFILE_TARGET=10000`
- `EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=256`
- `EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=300`
- `EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=20000`

Use when:
- moderate bursts, moderate p95 goals

### Profile B: Burst-Tolerant
- `EDGE_RUNTIME_NOFILE_TARGET=20000`
- `EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512`
- `EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500`
- `EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000`

Use when:
- arrival spikes are short
- brief queueing is acceptable

Watch:
- p95/p99 increase during spikes
- memory growth due to waiters

### Profile C: Low-Latency Fail-Fast
- `EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=50..150`
- moderate waiter cap

Use when:
- strict latency SLO is more important than completion ratio

Watch:
- refusal/error rate rises sooner under bursts

## 10. Operational Runbook

### Step 1: Confirm listener FD clamp
Check:
- `listener_connection_capacity.effectiveMaxConnections`
- `listener_connection_capacity.configuredMaxConnections`
- `listener_connection_capacity.softLimit`

If effective << configured:
- increase OS/container `nofile`
- set `EDGE_RUNTIME_NOFILE_TARGET`

### Step 2: Identify saturation reason
Check routing counters:
- `saturated_rejections_context_capacity`
- `saturated_rejections_scale_blocked`
- `saturated_rejections_scale_failed`

### Step 3: Tune the right layer
- Context-cap dominated:
  - increase `EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS`
  - maybe increase `EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE`
- Scale-blocked dominated:
  - increase isolate limits or reduce memory guardrail constraints
- Burst-dominated with low sustained saturation:
  - increase queue timeout and/or waiters

### Step 4: Re-test with arrival-rate
Track at least:
- `http_req_failed`
- p95/p99 duration
- routing saturation counters
- listener connection capacity snapshot

## 10.1 Practical Profile Examples

Two practical profiles are useful in day-to-day operation:

1. `run-latency`:
- Objective: keep tail latency tighter
- Strategy: smaller queue timeout, smaller waiter pool, fail earlier under overload

2. `run-throughput`:
- Objective: maximize successful completions under short bursts
- Strategy: larger queue timeout, larger waiter pool, absorb burst pressure

### Example A: Makefile targets

```makefile
run-latency:
  @cur=$$(ulimit -n); \
  if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
    ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
  fi; \
  eff=$$(ulimit -n); \
  echo "Starting runtime (latency profile) RLIMIT_NOFILE=$$eff"; \
  EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
  EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
  EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=256 \
  EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=24 \
  EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=32 \
  EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=100 \
  EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=10000 \
  cargo run -- start 2>&1

run-throughput:
  @cur=$$(ulimit -n); \
  if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
    ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
  fi; \
  eff=$$(ulimit -n); \
  echo "Starting runtime (throughput profile) RLIMIT_NOFILE=$$eff"; \
  EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
  EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
  EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
  EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=32 \
  EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=100 \
  EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
  EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
  cargo run -- start 2>&1
```

### Example B: Direct CLI launch without Makefile

Latency-oriented profile:

```bash
EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=256 \
EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=24 \
EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=32 \
EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=100 \
EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=10000 \
cargo run -- start
```

Throughput-oriented profile:

```bash
EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=32 \
EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=100 \
EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
cargo run -- start
```

Validation tip:
- Use `/_internal/metrics?fresh=1` (or `/metrics?fresh=1`) at test end to avoid stale cached snapshots.

## 11. Tradeoff Matrix

1. Increase `max_connections`
- Gain: higher ingress concurrency ceiling
- Cost: no benefit if FD budget is lower; may increase contention

2. Increase context caps
- Gain: higher in-flight execution capacity
- Cost: memory/CPU pressure and possible noisy-neighbor effects

3. Increase queue timeout
- Gain: fewer short-burst failures
- Cost: larger tail latency and more queued work under sustained overload

4. Increase waiter cap
- Gain: absorb bigger bursts
- Cost: memory growth and slower overload recovery

5. Increase nofile
- Gain: broader FD headroom for listeners and sockets
- Cost: must be supported by host/container limits and monitored carefully

## 12. Recommended Alerts

1. Listener clamp active
- Trigger when `effectiveMaxConnections < configuredMaxConnections`

2. Capacity rejection rate
- Trigger on sustained growth of `routing.saturated_rejections`

3. Root-cause split
- Trigger when one reason dominates:
  - context capacity
  - scale blocked
  - scale failed

4. FD pressure symptoms
- Trigger on EMFILE/ENFILE logs and sudden refusal spikes

## 13. Example Launch Command for 3k rps Trials

```bash
EDGE_RUNTIME_NOFILE_TARGET=20000 \
EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
cargo run -- start
```

Then run your k6 `constant-arrival-rate` scenario and compare:
- failures
- p95/p99
- routing saturation reason counters

## 14. Final Notes

- Prefer incremental tuning with metrics-driven feedback.
- Treat queueing as burst smoothing, not as substitute for real capacity.
- If overload is sustained, scale out process/replica count and keep per-process settings conservative.
