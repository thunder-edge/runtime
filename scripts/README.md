# Benchmark and Deployment Scripts

This directory contains scripts for bundling, deploying, and load-testing the Deno Edge Runtime with different bundle formats.

## Overview

### Bundle Scripts
- **`bundle-eszip.sh`** - Bundle all examples in ESZIP format
- **`bundle-snapshot.sh`** - Bundle all examples in SNAPSHOT format

### Deployment & Test Scripts
- **`deploy-and-test-eszip.sh`** - Deploy ESZIP bundles and run k6 load tests
- **`deploy-and-test-snapshot.sh`** - Deploy SNAPSHOT bundles and run k6 load tests
- **`load-test.js`** - k6 load testing script (JavaScript)

### Automation Scripts
- **`run-benchmarks.sh`** - Full end-to-end benchmark (build, bundle, deploy, test everything)
- **`quick-benchmark.sh`** - Fast re-run of benchmarks without rebuilding

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

# Run complete benchmark (build + bundle + deploy + test both formats)
./scripts/run-benchmarks.sh
```

This will:
1. Build the release binary
2. Bundle all examples in ESZIP format
3. Bundle all examples in SNAPSHOT format
4. Start the server
5. Deploy and test ESZIP bundles with k6
6. Deploy and test SNAPSHOT bundles with k6
7. Display metrics and performance summary

### Quick Re-test (no rebuild)

```bash
# Test both formats
./scripts/quick-benchmark.sh both

# Test only ESZIP
./scripts/quick-benchmark.sh eszip

# Test only SNAPSHOT
./scripts/quick-benchmark.sh snapshot
```

### Individual Steps

#### 1. Build and Bundle

```bash
# Build the project
cargo build --release

# Bundle examples (one or both)
./scripts/bundle-eszip.sh
./scripts/bundle-snapshot.sh
```

#### 2. Start the Server

```bash
./target/release/deno-edge-runtime start --host 0.0.0.0 --port 9000
```

#### 3. Deploy and Test

In a new terminal:

```bash
# Test ESZIP format
./scripts/deploy-and-test-eszip.sh

# Or test SNAPSHOT format
./scripts/deploy-and-test-snapshot.sh
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

## Comparing Formats

### ESZIP vs SNAPSHOT

Currently, both formats produce similar performance because snapshots require dynamic loading support from deno_core. Once that's implemented:

**ESZIP (Current)**:
- ✅ Modules loaded from archive at runtime
- ✅ Better compatibility
- ⚠️ Extension initialization happens per-function
- Average cold start: ~250-300ms

**SNAPSHOT (Future)**:
- ✅ Pre-initialized state captured in snapshot
- ✅ Faster startup (skip extension init)
- ✅ Better warm start performance
- Expected cold start: ~100-150ms (when supported)

## Troubleshooting

### "Port 9000 already in use"
```bash
# Kill existing process
pkill -f "deno-edge-runtime.*start"
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
tail -f /tmp/edge-runtime.log

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

- **Bundles**: `./bundles/eszip/` and `./bundles/snapshot/`
- **Server Log**: `/tmp/edge-runtime.log`
- **k6 Results**: Displayed in console (HTML report optional with `-o html`)

## Advanced Usage

### Custom Load Parameters

Edit `load-test.js` to change:
- `stages` - Test duration and VU (virtual user) count
- `EXAMPLES` - List of functions to test
- `BASE_URL` - Server address

### Generate HTML Report

```bash
./scripts/quick-benchmark.sh both --out html --summary-export=/tmp/summary.json
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
4. **Snapshot usage**: Once enabled, will significantly reduce cold start time

## See Also

- Main README: `../README.md`
- Bundle format docs: `../crates/functions/src/types.rs`
- Load test configuration: `./load-test.js`
