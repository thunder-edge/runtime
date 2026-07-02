# Metrics Endpoint Reference

This document is the canonical reference for the runtime metrics JSON served by:

- `GET /_internal/metrics`
- `GET /metrics` (alias)

## Endpoint behavior

- Response format: JSON
- Cache TTL: 15s (in-memory snapshot cache)
- Fresh query override: `?fresh=1`
  - `/_internal/metrics?fresh=1`
  - `/metrics?fresh=1`

Notes:
- Without `fresh=1`, endpoint returns cached data while cache age is less than 15s.
- With `fresh=1`, endpoint recomputes immediately and refreshes cache.
- If admin API key is enabled, `/_internal/*` routes require `X-API-Key`; `/metrics` does not use the admin prefix.

## Top-level schema

`GET /metrics` returns a JSON object with:

- `function_count` (number)
- `total_requests` (number)
- `total_errors` (number)
- `total_cold_starts` (number)
- `avg_cold_start_ms` (number)
- `avg_cold_start_us` (number)
- `avg_cold_start_ms_precise` (number)
- `avg_warm_start_ms` (number)
- `avg_warm_start_us` (number)
- `avg_warm_start_ms_precise` (number)
- `total_cold_start_time_us` (number)
- `total_warm_start_time_us` (number)
- `memory` (object)
- `process_saturation` (object)
- `routing` (object)
- `listener_connection_capacity` (object)
- `egress_connection_manager` (object)
- `top10` (object)
- `functions` (array of function objects)

## Global aggregates

### `function_count`
- Type: integer
- Meaning: number of deployed functions known by the registry.

### `total_requests`
- Type: integer
- Meaning: sum of `functions[*].metrics.total_requests`.

### `total_errors`
- Type: integer
- Meaning: sum of `functions[*].metrics.total_errors`.

### `total_cold_starts`
- Type: integer
- Meaning: sum of `functions[*].metrics.cold_starts`.

### `avg_cold_start_ms`
- Type: integer
- Unit: milliseconds
- Formula: `total_cold_start_time_ms / total_cold_starts`, rounded down (integer division).
- Behavior: `0` when `total_cold_starts == 0`.

### `avg_cold_start_us`
- Type: integer
- Unit: microseconds
- Formula: `total_cold_start_time_us / total_cold_starts`, rounded down.
- Behavior: `0` when `total_cold_starts == 0`.

### `avg_cold_start_ms_precise`
- Type: float
- Unit: milliseconds
- Formula: `total_cold_start_time_us / total_cold_starts / 1000.0`.
- Behavior: `0.0` when `total_cold_starts == 0`.

### `avg_warm_start_ms`
- Type: integer
- Unit: milliseconds
- Formula: `total_warm_start_time_ms / total_requests`, rounded down.
- Behavior: `0` when `total_requests == 0`.

### `avg_warm_start_us`
- Type: integer
- Unit: microseconds
- Formula: `total_warm_start_time_us / total_requests`, rounded down.
- Behavior: `0` when `total_requests == 0`.

### `avg_warm_start_ms_precise`
- Type: float
- Unit: milliseconds
- Formula: `total_warm_start_time_us / total_requests / 1000.0`.
- Behavior: `0.0` when `total_requests == 0`.

### `total_cold_start_time_us`
- Type: integer
- Unit: microseconds
- Meaning: aggregate cold-start elapsed time across functions.

### `total_warm_start_time_us`
- Type: integer
- Unit: microseconds
- Meaning: aggregate warm-request elapsed time across functions.

## `memory`

- `process_memory_mb` (float): process memory in MB from `sysinfo` current process.
- `total_memory_mib` (float): host total memory in MiB.
- `available_memory_mib` (float): host available/free memory in MiB (max of available and free).
- `estimated_per_function_mb` (float): `process_memory_mb / function_count`; `0.0` when no functions.

## `process_saturation`

Global autoscaling signal derived from memory, CPU, pool, and FD pressure.

### Fields

- `score` (float): global saturation score in `[0.0, 1.0]`.
- `level` (string): one of:
  - `healthy` when `score < 0.75`
  - `warning` when `0.75 <= score < 0.90`
  - `critical` when `score >= 0.90`
- `should_scale_out` (boolean): `true` when `score >= 0.75`.
- `active_signals` (array[string]): subset of `memory`, `cpu`, `pool_isolates`, `pool_contexts`, `fd` where each component is `>= 0.75`.
- `thresholds` (object): static thresholds used by leveling.
  - `warning` (float): `0.75`
  - `critical` (float): `0.90`
- `components` (object): normalized component values in `[0.0, 1.0]`.
  - `memory`
  - `cpu`
  - `pool`
  - `pool_isolates`
  - `pool_contexts`
  - `fd`
- `debug` (object): memory internals.
  - `memory_host_raw`
  - `memory_process_raw`

### Component definitions

- `memory_host_raw`: `1 - (available_memory_mib / total_memory_mib)`, clamped `[0,1]`.
- `memory_process_raw`: `process_memory_mb / total_memory_mib`, clamped `[0,1]`.
- `components.memory`: `max(memory_process_raw, memory_host_raw * memory_process_raw)`.
- `components.cpu`: ratio of accumulated CPU time to warm request time estimate.
- `components.pool_isolates`: `routing.total_isolates / sum(functions[*].pool.max)`.
- `components.pool_contexts`: `routing.saturated_contexts / routing.total_contexts`.
- `components.pool`: `max(pool_isolates, pool_contexts)`.
- `components.fd`: max of runtime FD pressure and listener clamp pressure.

### Score aggregation

`score` is the max of:
- `components.memory`
- `components.cpu`
- `components.pool_isolates`
- `components.pool_contexts`
- `components.fd`

## `routing`

Snapshot of scheduler and capacity state.

- `total_contexts` (integer): total logical contexts currently tracked.
- `total_isolates` (integer): isolate count derived from routing state rollups.
- `global_pool_total_isolates` (integer): alive isolate count from global function pool accounting.
- `global_pool_max_isolates` (integer): configured global isolate cap.
- `isolate_accounting_gap` (integer): `global_pool_total_isolates - total_isolates`.
- `total_active_requests` (integer): active requests summed over routing entries.
- `saturated_rejections` (integer): total route-target capacity rejections.
- `saturated_rejections_context_capacity` (integer): subset caused by context saturation.
- `saturated_rejections_scale_blocked` (integer): subset where scale-up was blocked.
- `saturated_rejections_scale_failed` (integer): subset where scale-up attempted but failed.
- `burst_scale_batch_last` (integer): last burst scale batch size chosen.
- `burst_scale_events_total` (integer): count of burst scale events (batch > 1).
- `saturated_contexts` (integer): contexts at active-request cap.
- `saturated_isolates` (integer): isolates where all contexts are saturated and isolate context cap is reached.

## `listener_connection_capacity`

Process listener capacity snapshot considering FD budget.

- `configuredMaxConnections` (integer): configured `max_connections`.
- `effectiveMaxConnections` (integer): post-clamp effective listener limit.
- `softLimit` (integer): process `RLIMIT_NOFILE` soft limit.
- `reservedFd` (integer): reserved FD headroom for safety/system use.
- `fdBudget` (integer): FD budget available to listener capacity.

## `egress_connection_manager`

Global outbound lease manager snapshot.

- `softLimit` (integer): process `RLIMIT_NOFILE` soft limit.
- `openFdCount` (integer): observed open FD count.
- `reservedFd` (integer): reserved FD for non-egress use.
- `outboundFdBudget` (integer): estimated FD budget for outbound leases.
- `adaptiveActiveLimit` (integer): dynamic upper bound for active leases.
- `activeLeases` (integer): currently active leases.
- `queuedWaiters` (integer): current number of waiters for lease acquisition.
- `totalAcquired` (integer): cumulative successful acquisitions.
- `totalReleased` (integer): cumulative releases.
- `totalRejected` (integer): cumulative rejections due to backpressure.
- `totalTimeouts` (integer): cumulative timed-out waits.
- `totalReaped` (integer): cumulative stale leases reaped.
- `knownTenants` (integer): tenants currently tracked.
- `topTenantsByActive` (array): top tenants by active leases (max 10).
  - `tenant` (string)
  - `active` (integer)
- `tokenBucket` (object): token bucket state.
  - `tokens` (float)
  - `capacity` (float)
  - `refillPerSec` (float)

## `top10`

Ranked views computed from current function list. Each array returns up to 10 items.

- `cold_slowest`
  - `name` (string)
  - `avg_cold_start_ms` (float)
  - `cold_starts` (integer)
- `cold_fastest`
  - same shape as `cold_slowest`
- `warm_slowest`
  - `name` (string)
  - `avg_warm_request_ms` (float)
  - `requests` (integer)
- `warm_fastest`
  - same shape as `warm_slowest`
- `cpu_bound`
  - `name` (string)
  - `cpu_bound_ratio` (float)
  - `avg_cpu_time_ms_per_request` (float)
  - `avg_warm_request_ms` (float)
- `blocking_cpu`
  - `name` (string)
  - `avg_cpu_time_ms_per_request` (float)
  - `requests` (integer)
- `memory_usage`
  - `name` (string)
  - `peak_heap_used_mb` (float)
  - `current_heap_used_mb` (float)
  - `peak_heap_used_bytes` (integer)
- `cpu_time_total`
  - `name` (string)
  - `total_cpu_time_ms` (integer)
  - `requests` (integer)

## `functions[]`

Array of per-function objects. Each entry is the serialized `FunctionInfo`.

### FunctionInfo fields

- `name` (string): function slug/name.
- `status` (string): `loading`, `running`, `error`, `shutting_down`.
- `metrics` (object): function metrics snapshot.
- `bundle_format` (string): `eszip` or `snapshot`.
- `package_v8_version` (string): V8 version recorded in deploy package.
- `runtime_v8_version` (string): V8 version currently running.
- `snapshot_compatible_with_runtime` (boolean)
- `requires_snapshot_regeneration` (boolean)
- `stored_eszip_size_bytes` (integer)
- `can_regenerate_snapshot_from_stored_eszip` (boolean)
- `pool` (object): isolate pool snapshot.
- `created_at` (string, RFC3339 datetime)
- `updated_at` (string, RFC3339 datetime)
- `last_error` (string or null)

### `functions[].pool`

- `min` (integer): configured per-function minimum isolates.
- `max` (integer): configured per-function maximum isolates.
- `current` (integer): currently allocated isolates for this function.

### `functions[].metrics`

- `total_requests` (integer)
- `active_requests` (integer)
- `total_errors` (integer)
- `total_cpu_time_ms` (integer)
- `cold_starts` (integer)
- `avg_cold_start_ms` (integer)
- `total_cold_start_time_ms` (integer)
- `total_cold_start_time_us` (integer)
- `avg_cold_start_us` (integer)
- `avg_cold_start_ms_precise` (float)
- `total_warm_start_time_ms` (integer)
- `total_warm_start_time_us` (integer)
- `avg_warm_request_ms` (integer)
- `avg_warm_request_us` (integer)
- `avg_warm_request_ms_precise` (float)
- `current_heap_used_bytes` (integer)
- `peak_heap_used_bytes` (integer)
- `current_heap_used_mb` (float)
- `peak_heap_used_mb` (float)

## Example query commands

```bash
curl -s http://localhost:9000/metrics | jq
curl -s 'http://localhost:9000/metrics?fresh=1' | jq '.process_saturation'
curl -s 'http://localhost:9000/_internal/metrics?fresh=1' | jq '.routing'
```

## Compatibility and evolution

- This schema is generated directly in `crates/server/src/router.rs` by `build_metrics_body`.
- New fields can be added without notice in minor runtime updates.
- Clients should tolerate unknown fields.
- For strict integrations, pin runtime version and validate expected keys.
