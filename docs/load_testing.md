# Load Testing Guide

## Overview

Load testing the Deno Edge Runtime helps understand performance characteristics, measure cold/warm start latency, and identify bottlenecks. This guide covers the k6-based load testing infrastructure.

For deep operational tuning under sustained/high arrival rates, see:
- `docs/high-load-capacity-fd-saturation.md`

## What is Load Testing?

Load testing simulates real-world usage by generating multiple concurrent requests to your application. It measures:

- **Cold Start**: Time to initialize a function for the first time
- **Warm Start**: Time for subsequent requests after initialization
- **Throughput**: Number of requests handled per second
- **Response Time**: Latency from request to response
- **Error Rate**: Percentage of failed requests
- **Resource Usage**: CPU and memory consumption under load

## Test Stages

The load test follows a 4-stage progression to measure different scenarios:

### Stage 1: Cold Start Detection (10 seconds)
- **Virtual Users (VU)**: 1
- **Purpose**: Measure initial function initialization time
- **Metric**: `avg_cold_start_ms` (time to load and initialize isolate)

### Stage 2: Warm Start Ramp (30 seconds)
- **Virtual Users**: 1 → 5 (gradual increase)
- **Purpose**: Transition from cold to warm execution
- **Metric**: Observe latency stabilization as isolates warm up

### Stage 3: Sustained Load (30 seconds)
- **Virtual Users**: 10
- **Purpose**: Measure steady-state performance under load
- **Metric**: `avg_warm_request_ms` (per-request latency when function is already loaded)

### Stage 4: Cooldown (10 seconds)
- **Virtual Users**: 10 → 0 (gradual decrease)
- **Purpose**: Graceful shutdown and cleanup
- **Metric**: Ensure clean resource deallocation

**Total Test Duration**: ~80 seconds per test run

## Metrics Explained

### Cold Start Metrics

```
cold_starts: 1
avg_cold_start_ms: 250
```

- **cold_starts**: Number of times the isolate was initialized
- **avg_cold_start_ms**: Average milliseconds to initialize (lower is better)
- **Why it matters**: Cold starts impact user experience on first request
- **Typical range**: 150-300ms (varies by extensions and dependencies)

### Warm Start Metrics

```
avg_warm_request_ms: 15
```

- **avg_warm_request_ms**: Average latency for cached isolate requests
- **Why it matters**: Most user requests hit warm isolates (after first initialization)
- **Typical range**: 5-20ms (dominated by JavaScript execution time)

### Throughput Metrics

```
total_requests: 150
rate_limit_errors: 0
```

- **total_requests**: Total requests processed
- **rate_limit_errors**: Requests rejected due to rate limiting
- **Why it matters**: Shows how many requests your functions can handle

### Error Metrics

```
total_errors: 0
error_rate: 0%
```

- **total_errors**: Number of failed requests
- **error_rate**: Percentage of requests that failed
- **Why it matters**: Errors indicate crashes, timeouts, or bugs

## Test Examples

The load test targets 7 representative examples:

| Example | Type | Purpose |
|---------|------|---------|
| `hello` | Simple | Baseline response time |
| `json-api` | Data Processing | Serialization overhead |
| `cors` | Headers | CORS handling |
| `basic-auth` | Authentication | Auth verification |
| `error-handling` | Error Cases | Exception handling |
| `middleware` | Processing | Middleware chain overhead |
| `url-redirect` | Routing | Path matching |

## Running Load Tests

### Quick Test (Existing Bundles)

```bash
./scripts/quick-benchmark.sh
```

Runs ESZIP benchmark using existing bundles without rebuilding.

### Full Benchmark (Clean Build)

```bash
./scripts/run-benchmarks.sh
```

Rebuilds, bundles, and tests ESZIP flow. Takes ~1-2 minutes.

### Manual Testing

For custom scenarios:

```bash
# Start server
./target/debug/thunder start --host 0.0.0.0 --port 9000 &

# In another terminal, deploy a function
curl -X POST http://localhost:9000/_internal/functions \
  -H "x-function-name: hello" \
  --data-binary @bundles/eszip/hello.pkg

# Run k6 manually
k6 run scripts/load-test.js

# Stop server
pkill -f "thunder start"
```

## Understanding Results

### Sample Output

```
checks........................: 100.00% (1400 passed, 0 failed)
data_received..................: 68 kB
data_sent.......................: 49 kB
http_req_blocked................: avg=0.5ms min=0.1ms max=8.2ms p(90)=0.7ms p(95)=0.9ms
http_req_connecting.............: avg=0.1ms min=0.0ms max=4.3ms p(90)=0.1ms p(95)=0.2ms
http_req_duration...............: avg=12.3ms min=2.1ms max=85.4ms p(90)=18.2ms p(95)=25.1ms
http_req_failed.................: 0.00%
http_req_receiving..............: avg=0.3ms min=0.0ms max=5.2ms p(90)=0.4ms p(95)=0.5ms
http_req_sending................: avg=0.2ms min=0.1ms max=2.5ms p(90)=0.3ms p(95)=0.4ms
http_req_tls_handshaking........: avg=0ms min=0ms max=0ms p(90)=0ms p(95)=0ms
http_req_waiting................: avg=11.8ms min=1.9ms max=84.6ms p(90)=17.5ms p(95)=24.2ms
http_requests_per_sec...........: 101.23
iteration_duration..............: avg=1.03s min=1.00s max=1.08s
iterations.......................: 1400
vus............................: 0
vus_max..........................: 10
```

### Key Metrics to Watch

**Passing Checks**: Should be 100% - indicates all responses are valid
```
checks........................: 100.00% (1400 passed, 0 failed)
```

**Http Request Duration**: Average latency - lower is better
```
http_req_duration...............: avg=12.3ms      <-- warm start time
```

**Requests Per Second**: Throughput - higher is better
```
http_requests_per_sec...........: 101.23         <-- sustained throughput
```

**Request Failures**: Should be 0% - indicates stability
```
http_req_failed.................: 0.00%
```

**P95/P99 Latency**: Worst-case response times
```
p(95)=25.1ms            <-- 95th percentile
p(99)=45.2ms            <-- 99th percentile
```

## Performance Benchmarks

### Expected Results (ESZIP, Apple Silicon)

```
Cold Start:     ~250-300ms
Warm Start:     ~10-20ms
Throughput:     ~80-120 req/s per VU
Error Rate:     0%
```

### ESZIP Baseline Targets

| Metric | ESZIP |
|--------|-------|
| Cold Start | ~250-300ms |
| Warm Start | ~10-20ms |
| Memory | ~50MB |

## Metrics Endpoint

The server exposes metrics at:

```bash
curl http://localhost:9000/_internal/metrics
curl http://localhost:9000/metrics
```

When you need immediate post-test values (without cache lag), use:

```bash
curl http://localhost:9000/_internal/metrics?fresh=1
# or
curl http://localhost:9000/metrics?fresh=1
```

### Sample Response

```json
{
  "functions": [
    {
      "name": "hello",
      "status": "running",
      "metrics": {
        "total_requests": 150,
        "cold_starts": 1,
        "avg_cold_start_ms": 248,
        "avg_warm_request_ms": 14,
        "total_errors": 0,
        "error_rate": 0.0
      }
    },
    {
      "name": "json-api",
      "status": "running",
      "metrics": {
        "total_requests": 143,
        "cold_starts": 1,
        "avg_cold_start_ms": 256,
        "avg_warm_request_ms": 18,
        "total_errors": 0,
        "error_rate": 0.0
      }
    }
  ]
}
```

## Performance Optimization Tips

### 1. Reduce Cold Start Time

- **Minimize dependencies**: Each import adds initialization time
- **Use lightweight libraries**: Avoid heavy frameworks if possible
- **Pre-load critical modules**: Initialize in function entry point

### 2. Improve Warm Start

- **Cache expensive operations**: Use module-level variables
- **Stream responses**: Use Response streaming for large payloads
- **Connection pooling**: Reuse database/API connections

### 3. Monitor Resource Usage

Watch for memory leaks:
```bash
# Check metrics over time
while true; do
  curl -s http://localhost:9000/_internal/metrics?fresh=1 | jq '.functions[].metrics'
  sleep 5
done
```

### 4. Tuning Parameters

Adjust server-side limits:

```bash
./target/debug/thunder start \
  --max-heap-mib 256 \              # Increase if out of memory
  --cpu-time-limit-ms 100000 \      # CPU timeout per request
  --wall-clock-timeout-ms 120000    # Total timeout per request
```

## Troubleshooting

### High Cold Start Time

**Symptom**: `avg_cold_start_ms > 500ms`

**Causes**:
- Heavy dependencies or large modules
- Slow disk I/O during module loading
- Large bundle files

**Solutions**:
- Audit imported modules with `wc -l` on bundle
- Profile with `--verbose` logging
- Check available disk space and I/O performance

### High Warm Start Latency

**Symptom**: `avg_warm_request_ms > 50ms`

**Causes**:
- CPU-intensive JavaScript code
- Synchronous I/O operations
- Blocked event loop

**Solutions**:
- Profile JavaScript with v8 profiling
- Use async/await for all I/O
- Consider splitting large computations

### Random Request Failures

**Symptom**: `error_rate > 0.5%`

**Causes**:
- Timeouts from slow operations
- Memory pressure on isolate
- Uncaught exceptions

**Solutions**:
- Check server logs: `tail -f /tmp/thunder.log`
- Increase timeouts if operations are legitimately slow
- Add error handling in functions

### k6 Connection Refused

**Symptom**: `connection refused` errors

**Causes**:
- Server not started
- Wrong host/port

**Solutions**:
```bash
# Verify server is running
curl http://localhost:9000/hello

# Check processes
ps aux | grep "thunder"

# Kill and restart if stuck
pkill -f "thunder"
./target/debug/thunder start
```

## Advanced Configuration

### Custom Load Patterns

Edit `scripts/load-test.js` to modify test stages:

```javascript
export const options = {
  stages: [
    { duration: '1m', target: 1 },    // 1 minute at 1 VU
    { duration: '5m', target: 50 },   // 5 minutes ramp to 50 VU
    { duration: '10m', target: 50 },  // 10 minutes sustained
    { duration: '1m', target: 0 },    // 1 minute cooldown
  ],
};
```

### Generate HTML Report

```bash
k6 run scripts/load-test.js -o html
# Opens report.html with detailed visualizations
```

### Run Specific Examples

Modify the EXAMPLES array in `load-test.js`:

```javascript
const EXAMPLES = [
  'hello',
  'json-api',
  // Comment out others to test selectively
];
```

## Benchmark Workflow

Run the quick ESZIP benchmark and inspect metrics:

```bash
./scripts/quick-benchmark.sh
curl http://localhost:9000/_internal/metrics?fresh=1 | jq '.functions[] | {name, metrics}'
```

## Best Practices

1. **Test regularly**: Include in CI/CD pipeline
2. **Monitor trends**: Track metrics over time to detect regressions
3. **Load test different scenarios**: Peak load, sustained load, sustained with errors
4. **Baseline your functions**: Know expected performance before optimization
5. **Test in production-like environments**: Use similar CPU/memory if possible
6. **Simulate realistic traffic**: Mix of request types, sizes, and patterns

## See Also

- [README.md](./README.md) - Project overview
- [scripts/README.md](./scripts/README.md) - Benchmark script documentation
- [k6 docs](https://k6.io/docs/) - Complete k6 reference
- [deno_core](https://github.com/denoland/deno_core) - Runtime details
