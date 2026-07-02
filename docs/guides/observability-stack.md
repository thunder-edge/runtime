# Observability Stack (OTEL + Loki + Tempo + Prometheus + Grafana)

This stack lets you validate end-to-end telemetry from `thunder`:

- Traces -> OpenTelemetry Collector -> Tempo
- Metrics -> OpenTelemetry Collector -> vmagent -> VictoriaMetrics
- Logs -> OpenTelemetry Collector -> Loki
- Unified visualization -> Grafana

## Files

- Compose: `observability/docker-compose.yml`
- OTEL Collector config: `observability/otel-collector/config.yaml`
- vmagent scrape config: `observability/vmagent/prometheus.yml`
- Tempo config: `observability/tempo/config.yaml`
- Loki config: `observability/loki/config.yaml`
- Prometheus config: `observability/prometheus/prometheus.yml`
- Grafana datasources provisioning: `observability/grafana/provisioning/datasources/datasources.yml`
- Grafana dashboard provisioning: `observability/grafana/provisioning/dashboards/dashboards.yml`
- Grafana initial dashboard: `observability/grafana/provisioning/dashboards/files/thunder-overview.json`

## 1. Start the Stack

From repository root:

```bash
docker compose -f observability/docker-compose.yml up -d
```

Check services:

```bash
docker compose -f observability/docker-compose.yml ps
```

## 2. Start Runtime with OTEL Enabled

Run the runtime with OTEL export enabled and isolate collector export active:

```bash
cargo run -- \
  --otel-enabled \
  --otel-endpoint http://127.0.0.1:4318 \
  --otel-service-name thunder-local \
  --otel-enable-traces true \
  --otel-enable-metrics true \
  --otel-enable-logs true \
  --otel-export-isolate-logs true \
  start --print-isolate-logs false
```

Notes:

- `--print-isolate-logs false` is required to route `console.*` logs to the isolate collector and then OTEL logs pipeline.
- If `--print-isolate-logs true`, logs stay in stdout and collector export has no isolate events.

## 3. Generate Traffic

Deploy a function and send requests, for example using your existing admin API flow.

Any request flow should emit:

- request spans from server routers (`http.request`)
- OTEL metrics from runtime telemetry setup
- isolate collector logs (when `--print-isolate-logs=false`)

## 4. Validate in Grafana

Grafana URL:

- `http://localhost:3000`
- User: `admin`
- Password: `admin`

Provisioned datasources:

- VictoriaMetrics (default)
- Prometheus
- Loki
- Tempo

Provisioned dashboard:

- `Edge Runtime - Observability Overview`

Quick checks:

- Explore -> Tempo: search traces by service `thunder-local`
- Explore -> Loki: query `{service_name="thunder-local"}` or `{log_source="isolate"}`
- Explore -> VictoriaMetrics: query `edge_runtime_isolate_logs_exported_total`

## 5. Validate Collector and Backends Directly

- OTEL Collector metrics: `http://localhost:8888/metrics`
- OTEL Collector exported metrics endpoint for Prometheus: `http://localhost:9464/metrics`
- vmagent metrics: `http://localhost:8429/metrics`
- VictoriaMetrics UI: `http://localhost:8428/vmui`
- Tempo health-ish endpoint: `http://localhost:3200/ready`
- Loki readiness: `http://localhost:3100/ready`
- Prometheus: `http://localhost:9090`

## 6. Stop and Cleanup

Stop:

```bash
docker compose -f observability/docker-compose.yml down
```

Stop and remove volumes:

```bash
docker compose -f observability/docker-compose.yml down -v
```

## Troubleshooting

- No isolate logs in Loki:
  - ensure runtime started with `start --print-isolate-logs false`
  - ensure `--otel-export-isolate-logs true`
- No traces in Tempo:
  - confirm runtime started with `--otel-enabled` and `--otel-enable-traces true`
  - check collector logs: `docker logs edge-otel-collector`
- No metrics in Prometheus:
  - check targets in `http://localhost:9090/targets`
  - ensure collector is exposing `otel-collector:9464`

- No metrics in VictoriaMetrics:
  - check vmagent status in `http://localhost:8429/targets`
  - query in VMUI (`http://localhost:8428/vmui`) for `otelcol_receiver_accepted_spans`
