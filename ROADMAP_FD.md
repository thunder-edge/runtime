# ROADMAP FD and Process Saturation

> Objective: eliminate EMFILE/ENFILE instability and provide a reliable external autoscaling signal.
> Scope: single-process runtime with many V8 isolates/contexts sharing one OS FD limit.
>
> Last update: 08/03/2026

## Goals

- Keep admin/ingress listeners stable under heavy load.
- Maximize useful concurrency (contexts/isolates) without FD exhaustion.
- Enforce fair and predictable connection usage across tenants/functions.
- Expose process saturation metrics for scale-out automation.

## Non-negotiable constraints

- Do not weaken isolate/sandbox security boundaries.
- Keep backpressure deterministic (bounded queues, explicit refusal when saturated).
- Prefer graceful degradation over process-wide failure.

## Phase 0 - Stabilization (short term)

### P0.1 FD guard hardening in pool scaler

- Status: completed
- Files:
  - `crates/functions/src/registry.rs`
- Changes:
  - adaptive effective FD reserve (`effective_min_free_fds`)
  - conservative estimate of FD per isolate
  - block scale-up when `free < required_free`

### P0.2 Listener resilience on EMFILE

- Status: completed
- Files:
  - `crates/server/src/lib.rs`
- Changes:
  - detect `EMFILE` in admin/ingress accept loops
  - apply retry backoff (`250ms`) instead of tight error loop

### P0.3 FD budget telemetry

- Status: completed
- Files:
  - `crates/functions/src/registry.rs`
  - `crates/server/src/router.rs`
- Changes:
  - `fd_budget_snapshot()` in registry
  - expose `routing.fd_budget` and `routing.expected_max_contexts_by_fd` in `/_internal/metrics`

### P0.4 Process saturation score for autoscaling

- Status: completed
- Files:
  - `crates/server/src/router.rs`
- Changes:
  - add `process_saturation` block in metrics
  - weighted score with FD/memory/CPU components
  - level classification: `healthy`, `warning`, `critical`

## Phase 1 - Centralized outbound connection control

### P1.1 Runtime-owned outbound gate

- Status: completed (first production iteration)
- Deliverables:
  - central connection manager API for fetch/http/https/tcp/tls
  - deny direct uncontrolled socket creation from user code paths
- Files:
  - `crates/functions/src/connection_manager.rs`
  - `crates/functions/src/handler.rs`
  - `crates/server/src/router.rs`
  - `docs/design/egress-connection-manager.md`
- Implemented now:
  - process-global lease manager shared across isolates
  - FD-budget-aware adaptive active concurrency limit
  - token-bucket burst control + bounded async wait queue
  - execution-bound lease cleanup and stale lease reaper
  - metrics block `egress_connection_manager` in `/_internal/metrics`

### P1.2 Global and per-tenant quotas

- Status: planned
- Deliverables:
  - global hard cap for open outbound sockets
  - per-tenant/function soft/hard caps
  - fairness policy to avoid noisy-neighbor starvation

### P1.3 Connection pooling and reuse

- Status: planned
- Deliverables:
  - pooled keep-alive for HTTP/HTTPS upstreams
  - per-origin max connections and idle eviction
  - bounded pending dials

## Phase 2 - Advanced pressure management

### P2.1 Backpressure and load shedding policy

- Status: planned
- Deliverables:
  - bounded queue per tenant/priority
  - overload response contract (`capacity_exhausted` variants)
  - selective shedding for low-priority traffic

### P2.2 Leak detection and automatic cleanup

- Status: planned
- Deliverables:
  - stale socket detector
  - max socket lifetime reaper
  - periodic leak report with owner tags

### P2.3 Dynamic tuning

- Status: planned
- Deliverables:
  - tune pool prewarm and context growth from live pressure
  - reduce aggressive isolate growth when FD pressure rises

## Phase 3 - External autoscaling contract

### P3.1 Stable metrics schema

- Status: in progress
- Current fields:
  - `process_saturation.score`
  - `process_saturation.components.fd`
  - `process_saturation.components.memory`
  - `process_saturation.components.cpu`
  - `routing.fd_budget.*`

### P3.2 Autoscaler guidance

- Status: planned
- Suggested policy:
  - scale out when `process_saturation.score >= 0.75` for 30-60s
  - urgent scale out when `score >= 0.90` for 5-10s
  - scale in only with hysteresis (for example `score <= 0.45` for 3-5 min)

## Validation checklist

- [x] `cargo check -p edge-server -p functions -p edge-cli`
- [ ] sustained load test with 10k+ concurrent requests
- [ ] verify no `admin accept error: Too many open files`
- [ ] verify no `ingress accept error: Too many open files`
- [ ] verify prewarm stops before EMFILE while preserving listener responsiveness
- [ ] verify saturation score tracks stress and predicts overload

## Operational notes

- If `fd_budget.free` trends near `fd_budget.effective_min_free`, expect pool scale-up to stop.
- If `process_saturation.level` becomes `critical`, autoscaler should prioritize scale-out.
- Keep `RLIMIT_NOFILE` explicit in deployment manifests and monitor drift.
