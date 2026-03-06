# External Scaling Recommendations

This document describes the recommended external autoscaling signals for multi-process or multi-unit orchestration.

## Recommended Signals

Use all signals below together. Avoid scaling decisions based on a single metric.

1. `pool_busy_ratio` (global)
- Definition: busy isolates / total isolates in process.
- Use: high sustained values indicate process-level saturation.

2. `queue_depth` and `queue_wait_ms_p95` (per function)
- Definition: pending requests and p95 queue wait time before execution starts.
- Use: detect function-level hotspots and local starvation.

3. `reject_rate` (`429`/`503`)
- Definition: ratio of throttled or shed requests.
- Use: direct overload indicator for scale-out.

4. `cold_start_rate`
- Definition: frequency of isolate cold starts.
- Use: indicates lack of warm capacity or excessive churn.

5. Process memory and CPU saturation
- Definition: available memory and CPU usage/saturation.
- Use: hard guardrails for scale-up and safe capacity planning.

## Suggested External Scale Policy

Scale out when all conditions below are true for a sustained window:
- `pool_busy_ratio` above threshold (example: > 0.8)
- AND (`queue_wait_ms_p95` high OR `reject_rate` non-zero)
- AND memory/CPU still allow safe expansion

Scale in when all conditions below are true for a sustained window:
- `pool_busy_ratio` low (example: < 0.3)
- AND queue near zero
- AND reject rate at zero

## Runtime Pool Scaling Guardrail

The runtime includes a memory-aware guardrail for isolate scale-up:
- If available memory is below configured minimum, isolate scale-up is blocked.
- Runtime logs a warning and skips scaling.
- Alert dispatch is intentionally left as a TODO hook for the orchestrator/monitoring stack.

This protects process stability under pressure and avoids self-induced OOM escalation.

## Control Plane Split

Recommended control split:
- CLI: enable/disable pooling and process-level guardrails.
- Admin API (`/_internal`): adjust per-function pool limits (`min` and `max`) at runtime.

This keeps runtime policy mutable without process restart while preserving a hard process-level safety envelope.
