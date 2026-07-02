# Documentação interna — Thunder Runtime

Docs técnicas internas do `runtime/` (referência, guias, design e relatórios). **Não** são o
site público (`docs/` da raiz do workspace) — este material só chega ao site por consolidação
de `plans` concluídos (ver `CLAUDE.md` §3.3).

Artefatos SDD ficam em pastas próprias: [adr/](./adr/README.md), [knowledge/](./knowledge/README.md),
[spec/](./spec/README.md), [plans/](./plans/README.md).

## reference/ — referências de API/contrato/config
- [cli.md](./reference/cli.md) — comandos da CLI (`start`, `bundle`, `watch`, `test`, `check`)
- [function-manifest.md](./reference/function-manifest.md) — manifest v2 (flavor, rotas, recursos, network)
- [FUNCTION_CONTRACT_README.md](./reference/FUNCTION_CONTRACT_README.md) — contrato da função de usuário
- [request-reference.md](./reference/request-reference.md) — objeto `Request` recebido pelo handler
- [http-response-helpers.md](./reference/http-response-helpers.md) — helpers `thunder:http`
- [metrics-endpoint-reference.md](./reference/metrics-endpoint-reference.md) — schema dos endpoints de métricas
- [testing-api-reference.md](./reference/testing-api-reference.md) — API da biblioteca de testes
- [vfs.md](./reference/vfs.md) — Virtual File System (mounts, quotas, cobertura)
- [NODE-COMPAT.md](./reference/NODE-COMPAT.md) — matriz de compatibilidade Node.js

## guides/ — how-to e operação
- [bundle-signing.md](./guides/bundle-signing.md) — assinatura Ed25519 de bundles
- [debugging.md](./guides/debugging.md) — debug com V8 inspector (VS Code, DevTools, nvim-dap)
- [observability-stack.md](./guides/observability-stack.md) — stack OTEL/Tempo/Loki/Prometheus/Grafana
- [load_testing.md](./guides/load_testing.md) — metodologia de load testing com k6
- [streaming-response-body.md](./guides/streaming-response-body.md) — streaming/SSE em handlers
- [testing-library.md](./guides/testing-library.md) — guia da biblioteca `thunder:testing`
- [external-scaling-recommendations.md](./guides/external-scaling-recommendations.md) — sinais e guardrails de autoscaling

## design/ — deep-dives de arquitetura
- [egress-connection-manager.md](./design/egress-connection-manager.md) — leasing de egress, FD budget
- [high-load-capacity-fd-saturation.md](./design/high-load-capacity-fd-saturation.md) — mitigação de saturação de FD
- [timeout-and-resource-tracking.md](./design/timeout-and-resource-tracking.md) — timeouts (CPU/wall-clock) e limpeza

## reports/ — relatórios gerados
- [web_standards_api_report.md](./reports/web_standards_api_report.md) — **auto-gerado** por `web_api_report` test (não editar à mão)
