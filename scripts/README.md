# Benchmark and Deployment Scripts

This directory contains scripts for bundling, deploying, and load-testing the Deno Edge Runtime using ESZIP bundles.

## Overview

### Bundle Scripts
- **`bundle-eszip.sh`** - Bundle all examples in ESZIP format

### Deployment & Test Scripts
- **`deploy-and-test-eszip.sh`** - Deploy ESZIP bundles and run k6 load tests
- **`load-test.js`** - k6 load testing script (JavaScript)

### Security / Integrity Scripts
- **`sign-bundle.sh`** - Sign an ESZIP/package bundle with Ed25519 and print base64 signature/header
- **`deploy-signed-bundle.sh`** - Sign and deploy/update a bundle in a single command (sign + curl)

### Automation Scripts
- **`run-benchmarks.sh`** - Full end-to-end benchmark (build, bundle, deploy, test everything)
- **`quick-benchmark.sh`** - Fast re-run of benchmarks without rebuilding
- **`benchmark-context-isolate-extreme.sh`** - Extreme comparative benchmark (legacy vs context+isolate) with consolidated stdout report
- **`node-crypto-benchmark.sh`** - Focused benchmark/check for `node:crypto` throughput/latency (`createHash`, `createHmac`, `randomBytes`)
- **`zlib-guardrail-benchmark.sh`** - Focused benchmark/check for `node:zlib` hardening guardrails
- **`start-observability-runtime.sh`** - Start observability docker stack + run edge runtime with OTEL + open Grafana

## Prerequisites

1. **Rust & Cargo** - For building the project
2. **k6** - For load testing
   ```bash
   # macOS
   brew install k6

   # Linux / using Docker
   docker run -i grafana/k6 run - < load-test.js
   ```
3. **curl** - For deployment (usually pre-installed)
4. **jq** (optional) - For prettier JSON output of metrics

## Quick Start

### Complete Benchmark (Recommended for first run)

```bash
# Make scripts executable
chmod +x ./scripts/*.sh

# Run complete benchmark (build + bundle + deploy + test ESZIP)
./scripts/run-benchmarks.sh
```

This will:
1. Build the release binary
2. Bundle all examples in ESZIP format
3. Start the server
4. Deploy and test ESZIP bundles with k6
5. Display metrics and performance summary

### Quick Re-test (no rebuild)

```bash
./scripts/quick-benchmark.sh
```

### Focused Node Crypto Benchmark

```bash
./scripts/node-crypto-benchmark.sh
```

This runs a focused microbenchmark test and reports throughput/latency for:
- `createHash('sha256')`
- `createHmac('sha256')`
- `randomBytes(32)`

### Extreme Context+Isolate Comparative Benchmark

```bash
./scripts/benchmark-context-isolate-extreme.sh
```

This benchmark runs two scenarios and prints a final comparative report in stdout:
- `legacy` mode (no context-pool scheduler)
- `context+isolate` mode (`--pool-enabled --context-pool-enabled`)

Measured outputs include:
- HTTP totals and latency (`avg`, `p95`)
- deterministic status distribution (`200`, `503`, unexpected)
- routing saturation metrics (`total_contexts`, `total_isolates`, `saturated_contexts`, `saturated_isolates`, `saturated_rejections`)
- percentage delta between context+isolate and legacy

Main tuning knobs (via env vars):

```bash
VUS_WARMUP=50 \
VUS_STEADY=150 \
VUS_EXTREME=400 \
DUR_EXTREME=45s \
HOLD_MS=50 \
./scripts/benchmark-context-isolate-extreme.sh
```

### Start Observability + Runtime (OTEL)

```bash
./scripts/start-observability-runtime.sh
```

Open all observability UIs:

```bash
./scripts/start-observability-runtime.sh --all
```

This will:
1. Start `observability/docker-compose.yml`
2. Run `cargo run -- start --print-isolate-logs false` with OTEL env vars set
3. Wait for readiness checks (`Grafana` and runtime `/_internal/health`)
4. Open Grafana (`http://localhost:3000`) in your browser

With `--all`, it also opens:
- VictoriaMetrics UI (`http://localhost:8428/vmui`)
- Prometheus (`http://localhost:9090`)
- Tempo (`http://localhost:3200`)
- Loki (`http://localhost:3100`)

You can override OTEL vars before running, for example:

```bash
EDGE_RUNTIME_OTEL_SERVICE_NAME=my-local-runtime \
EDGE_RUNTIME_OTEL_ENDPOINT=http://127.0.0.1:4318 \
./scripts/start-observability-runtime.sh
```

### Individual Steps

#### 1. Build and Bundle

```bash
# Build the project
cargo build --release

# Bundle examples
./scripts/bundle-eszip.sh
```

#### 2. Start the Server

```bash
./target/release/thunder start --host 0.0.0.0 --port 9000
```

#### 2.5 (Optional) Sign Bundles for Integrity Verification

```bash
./scripts/sign-bundle.sh \
  --bundle ./hello.eszip \
  --private-key ./bundle-signing-private.pem \
  --print-header
```

This prints the `x-bundle-signature-ed25519` value to include in deploy/update requests when bundle signature enforcement is enabled.

#### 2.6 (Optional) Sign + Deploy in One Command

```bash
./scripts/deploy-signed-bundle.sh \
  --bundle ./hello.eszip \
  --function hello \
  --private-key ./bundle-signing-private.pem \
  --api-key admin-secret
```

For update flow, pass `--method PUT`.

#### 3. Deploy and Test

In a new terminal:

```bash
# Test ESZIP format
./scripts/deploy-and-test-eszip.sh
```

## Load Test Details

The k6 load test (`load-test.js`) measures:

### Metrics Recorded
- **Cold Start**: Time to initialize a new function instance
- **Warm Start**: Time for subsequent requests (function already loaded)
- **Response Time**: Total request/response time
- **Throughput**: Requests per second
- **Error Rate**: Failed requests percentage

### Test Stages
1. **10s ramp-up** (1 VU) - Single user, measures cold start
2. **30s ramp** (1→5 VU) - Gradual increase
3. **30s sustained load** (10 VU) - Full load warm start measurement
4. **10s ramp-down** (10→0 VU) - Graceful shutdown

### Examples Tested
- `hello` - Simple response
- `json-api` - JSON serialization
- `cors` - CORS headers
- `basic-auth` - Authentication
- `error-handling` - Error scenarios
- `middleware` - Middleware chains
- `url-redirect` - URL redirection

## Understanding the Metrics

### Function Metrics Endpoint

The server exposes metrics at `http://localhost:9000/_internal/metrics`:

```json
{
  "functions": [
    {
      "name": "hello",
      "status": "running",
      "metrics": {
        "total_requests": 150,
        "cold_starts": 1,
        "avg_cold_start_ms": 250,
        "avg_warm_request_ms": 15,
        "total_errors": 0
      }
    }
  ]
}
```

### Key Metrics
- **cold_starts**: Number of times the function was initialized
- **avg_cold_start_ms**: Average time to initialize (lower is better)
- **avg_warm_request_ms**: Average time per request once loaded (lower is better)
- **total_requests**: Total requests processed
- **total_errors**: Number of failed requests

## Bundle Format

**ESZIP (Current)**:
- ✅ Modules loaded from archive at runtime
- ✅ Better compatibility
- ⚠️ Extension initialization happens per-function
- Average cold start: ~250-300ms

## Troubleshooting

### "Port 9000 already in use"
```bash
# Kill existing process
pkill -f "thunder.*start"
```

### "k6 not found"
```bash
# Install k6
brew install k6  # macOS
# or use Docker
docker run -i grafana/k6 run - < ./scripts/load-test.js
```

### Server won't start
```bash
# Check logs
tail -f /tmp/thunder.log

# Ensure no other process is using port 9000
lsof -i :9000
```

### Metrics endpoint returns empty
```bash
# May need to wait for functions to initialize
sleep 5

# Check if functions were deployed
curl http://localhost:9000/_internal/functions
```

## Output Files

After running benchmarks:

- **Bundles**: `./bundles/eszip/`
- **Server Log**: `/tmp/thunder.log`
- **k6 Results**: Displayed in console (HTML report optional with `-o html`)

## Advanced Usage

### Custom Load Parameters

Edit `load-test.js` to change:
- `stages` - Test duration and VU (virtual user) count
- `EXAMPLES` - List of functions to test
- `BASE_URL` - Server address

### Generate HTML Report

Use k6 directly to export summaries and reports:

```bash
k6 run scripts/load-test.js --summary-export=/tmp/summary.json
```

### Run Specific Examples Only

Edit `load-test.js`:
```javascript
const EXAMPLES = [
  'hello',
  'json-api',
  // etc.
];
```

## Performance Optimization Tips

1. **Increase cache locality**: More warm starts = better amortized performance
2. **Monitor cold starts**: Look for patterns in initialization time
3. **Profile extensions**: Check which extensions take longest to initialize
4. **Bundle size control**: Keep dependencies lean to reduce startup overhead

## See Also

- Main README: `../README.md`
- Bundle format docs: `../crates/functions/src/types.rs`
- Load test configuration: `./load-test.js`
