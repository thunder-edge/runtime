---
id: ADR-RUNTIME-0010
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Observability
created_at: 2026-06-26
---

# ADR 0010: Observabilidade via OpenTelemetry (OTLP)

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/docs/guides/observability-stack.md`

## Contexto

Operar um edge runtime exige traces, métricas e logs padronizados, sem lock-in de vendor.

## Decisão

Integrar **OpenTelemetry** (traces, métricas, logs) com exportador **OTLP HTTP** (padrão
`http://127.0.0.1:4318`) e **W3C Trace Context** (`traceparent`). Coletor encaminha para
Tempo (traces), Loki (logs) e Prometheus/Victoria Metrics. Endpoints `/metrics` e `/_internal/metrics`.

## Alternativas consideradas

- **Formato/agente proprietário:** lock-in e menor interoperabilidade.
- **Somente logs:** insuficiente para latência/tracing distribuído.

## Consequências

### Positivas
- Compatível com stacks major (Grafana, Datadog, New Relic); correlação por trace-id.

### Negativas / Trade-offs
- Footprint de dependências OTEL; cardinalidade de métricas exige tuning.

## Referências
- `runtime/crates/cli/src/telemetry.rs`
- `runtime/docs/{observability-stack.md,metrics-endpoint-reference.md}`
