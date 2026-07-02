# External Scaling Recommendations

This document describes the recommended external autoscaling signals for multi-process or multi-unit orchestration.

## Where To Read Metrics

Use either endpoint:
- `GET /_internal/metrics`
- `GET /metrics` (alias)

For control-loop decisions, prefer fresh snapshots:
- `GET /_internal/metrics?fresh=1`
- `GET /metrics?fresh=1`

## Recommended Signals

Use all signals below together. Avoid scaling decisions based on a single metric.

1. `process_saturation` (global, primary signal)
- Fields:
	- `score` (`0.0..1.0`)
	- `level` (`healthy`, `warning`, `critical`)
	- `should_scale_out` (`true`/`false`)
	- `components` (memory/cpu/pool/fd contributions)
- Use: first-pass global decision signal for scale-out.

2. Routing saturation and rejection signals
- `routing.saturated_rejections`
- `routing.saturated_rejections_context_capacity`
- `routing.saturated_rejections_scale_blocked`
- `routing.saturated_rejections_scale_failed`
- Use: identify whether pressure comes from context caps, blocked scale-up, or operational failures.

3. Routing capacity shape
- `routing.total_contexts`
- `routing.total_isolates`
- `routing.global_pool_total_isolates`
- `routing.global_pool_max_isolates`
- `routing.isolate_accounting_gap`
- Use: verify if process is near isolate ceilings and whether accounting is consistent.

4. Burst-pressure hints
- `routing.burst_scale_batch_last`
- `routing.burst_scale_events_total`
- Use: detect burst mode activity and whether scaling reacted in larger batches.

## Suggested External Scale Policy

Scale out when all conditions below are true for a sustained window:
- `process_saturation.should_scale_out = true` or `process_saturation.score` above your threshold (example: `>= 0.75`)
- AND at least one routing pressure signal is active:
	- rising `routing.saturated_rejections`, or
	- `routing.global_pool_total_isolates` close to `routing.global_pool_max_isolates`

Scale in when all conditions below are true for a sustained window:
- `process_saturation.level` is `healthy` for a sustained period
- AND `routing.saturated_rejections` is stable/flat
- AND ingress demand is low in your edge/load balancer metrics

## Important Scale-Down Semantics

Inside a process, context/isolate shrink uses cooldown timers:
- context downscale cooldown: `5s`
- isolate downscale cooldown: `10s`

Current behavior is event-driven by request release, not a periodic background reaper. After traffic drops to zero, counts may stay elevated until a new release event occurs.

## Runtime Pool Scaling Guardrail

The runtime includes a memory-aware guardrail for isolate scale-up:
- If available memory is below configured minimum, isolate scale-up is blocked.
- Runtime logs a warning and skips scaling.

This protects process stability under pressure and avoids self-induced OOM escalation.

## Control Plane Split

Recommended control split:
- CLI/environment: process-level limits and guardrails.
- Admin API (`/_internal`): adjust per-function pool limits (`min` and `max`) at runtime.

This keeps runtime policy mutable without process restart while preserving a hard process-level safety envelope.
