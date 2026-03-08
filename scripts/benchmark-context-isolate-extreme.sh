#!/bin/bash

# Extreme benchmark: compares legacy scheduling vs context+isolate scheduling
# and prints a consolidated report to stdout.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

SERVER_HOST="${SERVER_HOST:-127.0.0.1}"
SERVER_PORT="${SERVER_PORT:-9010}"
ADMIN_PORT="${ADMIN_PORT:-9011}"
INGRESS_URL="http://${SERVER_HOST}:${SERVER_PORT}"
ADMIN_URL="http://${SERVER_HOST}:${ADMIN_PORT}"
FUNCTION_NAME="${FUNCTION_NAME:-context-extreme}"

HOLD_MS="${HOLD_MS:-40}"
VUS_WARMUP="${VUS_WARMUP:-40}"
VUS_STEADY="${VUS_STEADY:-120}"
VUS_EXTREME="${VUS_EXTREME:-300}"
DUR_WARMUP="${DUR_WARMUP:-10s}"
DUR_STEADY="${DUR_STEADY:-20s}"
DUR_EXTREME="${DUR_EXTREME:-30s}"
DUR_COOLDOWN="${DUR_COOLDOWN:-10s}"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/context-isolate-extreme.XXXXXX")"
if [[ "${KEEP_BENCH_ARTIFACTS:-0}" == "1" ]]; then
  trap 'echo "keeping artifacts at $TMP_DIR"' EXIT
else
  trap 'rm -rf "$TMP_DIR"' EXIT
fi

if command -v k6 >/dev/null 2>&1; then
  K6_BIN="$(command -v k6)"
else
  echo "ERROR: k6 not found in PATH. Install k6 or run with a containerized k6 wrapper." >&2
  exit 1
fi

if [[ -x "$PROJECT_ROOT/target/debug/thunder" ]]; then
  THUNDER_BIN="$PROJECT_ROOT/target/debug/thunder"
elif [[ -x "$PROJECT_ROOT/target/release/thunder" ]]; then
  THUNDER_BIN="$PROJECT_ROOT/target/release/thunder"
else
  echo "ERROR: thunder binary not found. Build first with 'cargo build --release' or 'cargo build'." >&2
  exit 1
fi

wait_for_server() {
  for _ in $(seq 1 120); do
    if curl -s -f "$ADMIN_URL/_internal/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

stop_server() {
  local pid="${1:-}"
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
}

build_bundle() {
  local src="$TMP_DIR/context_extreme.ts"
  local out="$TMP_DIR/context_extreme.pkg"

  cat > "$src" <<'TS'
Deno.serve(async (req) => {
  const url = new URL(req.url);
  const holdMs = Number(url.searchParams.get('d') ?? '40');
  await new Promise((resolve) => setTimeout(resolve, holdMs));

  return new Response(JSON.stringify({
    ok: true,
    holdMs,
    path: url.pathname,
  }), {
    headers: {
      'content-type': 'application/json',
      'cache-control': 'no-store',
    },
  });
});
TS

  "$THUNDER_BIN" bundle -e "$src" -o "$out" >/dev/null
  echo "$out"
}

deploy_bundle() {
  local bundle_path="$1"
  curl -s -o /dev/null -w "%{http_code}" -X POST "$ADMIN_URL/_internal/functions" \
    -H "content-type: application/octet-stream" \
    -H "x-function-name: ${FUNCTION_NAME}" \
    --data-binary "@${bundle_path}"
}

run_mode() {
  local mode="$1"
  shift
  local -a start_args=("$@")

  local log_file="$TMP_DIR/${mode}.server.log"
  local summary_file="$TMP_DIR/${mode}.k6-summary.json"
  local routing_file="$TMP_DIR/${mode}.routing.json"
  local result_file="$TMP_DIR/${mode}.result.json"

  pkill -f "thunder.*start" >/dev/null 2>&1 || true
  sleep 1

  if ((${#start_args[@]} > 0)); then
    "$THUNDER_BIN" start --host "$SERVER_HOST" --port "$SERVER_PORT" --admin-host "$SERVER_HOST" --admin-port "$ADMIN_PORT" "${start_args[@]}" >"$log_file" 2>&1 &
  else
    "$THUNDER_BIN" start --host "$SERVER_HOST" --port "$SERVER_PORT" --admin-host "$SERVER_HOST" --admin-port "$ADMIN_PORT" >"$log_file" 2>&1 &
  fi
  local server_pid=$!

  if ! wait_for_server; then
    echo "ERROR: server did not become ready in mode=${mode}" >&2
    cat "$log_file" >&2 || true
    stop_server "$server_pid"
    exit 1
  fi

  local bundle
  bundle="$(build_bundle)"
  local deploy_status
  deploy_status="$(deploy_bundle "$bundle")"
  if [[ "$deploy_status" != "201" && "$deploy_status" != "200" ]]; then
    echo "ERROR: deploy failed in mode=${mode} status=${deploy_status}" >&2
    cat "$log_file" >&2 || true
    stop_server "$server_pid"
    exit 1
  fi

  "$K6_BIN" run "$SCRIPT_DIR/context-isolate-extreme-load.js" \
    --summary-export "$summary_file" \
    -e BASE_URL="$INGRESS_URL" \
    -e FUNCTION_NAME="$FUNCTION_NAME" \
    -e HOLD_MS="$HOLD_MS" \
    -e VUS_WARMUP="$VUS_WARMUP" \
    -e VUS_STEADY="$VUS_STEADY" \
    -e VUS_EXTREME="$VUS_EXTREME" \
    -e DUR_WARMUP="$DUR_WARMUP" \
    -e DUR_STEADY="$DUR_STEADY" \
    -e DUR_EXTREME="$DUR_EXTREME" \
    -e DUR_COOLDOWN="$DUR_COOLDOWN" >/dev/null

  curl -s "$ADMIN_URL/_internal/metrics" > "$routing_file"

  python3 - "$mode" "$summary_file" "$routing_file" "$result_file" <<'PY'
import json
import pathlib
import sys

mode = sys.argv[1]
summary_path = pathlib.Path(sys.argv[2])
routing_path = pathlib.Path(sys.argv[3])
result_path = pathlib.Path(sys.argv[4])

summary = json.loads(summary_path.read_text())
metrics = json.loads(routing_path.read_text())

root_metrics = summary.get("metrics", {})
http_duration = root_metrics.get("http_req_duration", {})
http_total = root_metrics.get("http_reqs", {})
failed_rate = root_metrics.get("http_req_failed", {})
status_200 = root_metrics.get("status_200_total", {})
status_503 = root_metrics.get("status_503_total", {})
status_other = root_metrics.get("status_other_total", {})

def as_number(d, key, default=0.0):
    if not isinstance(d, dict):
        return default
    value = d.get(key, default)
    try:
        return float(value)
    except Exception:
        return default

routing = metrics.get("routing", {})
required_routing_keys = [
  "total_contexts",
  "total_isolates",
  "total_active_requests",
  "saturated_contexts",
  "saturated_isolates",
  "saturated_rejections",
]
missing_keys = [k for k in required_routing_keys if k not in routing]

result = {
    "mode": mode,
    "http": {
        "requests_total": int(as_number(http_total, "count", 0)),
        "duration_p95_ms": as_number(http_duration, "p(95)", 0.0),
        "duration_avg_ms": as_number(http_duration, "avg", 0.0),
        "failed_rate": as_number(failed_rate, "rate", 0.0),
        "status_200_total": int(as_number(status_200, "count", 0)),
        "status_503_total": int(as_number(status_503, "count", 0)),
        "status_other_total": int(as_number(status_other, "count", 0)),
    },
    "routing": {
        "total_contexts": int(routing.get("total_contexts", 0)),
        "total_isolates": int(routing.get("total_isolates", 0)),
        "total_active_requests": int(routing.get("total_active_requests", 0)),
        "saturated_contexts": int(routing.get("saturated_contexts", 0)),
        "saturated_isolates": int(routing.get("saturated_isolates", 0)),
        "saturated_rejections": int(routing.get("saturated_rejections", 0)),
    },
      "warnings": {
        "missing_routing_keys": missing_keys,
      },
}

result_path.write_text(json.dumps(result))
PY

  stop_server "$server_pid"

  echo "$result_file"
}

print_mode_report() {
  local result_json="$1"
  python3 - "$result_json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
res = json.loads(path.read_text())
mode = res["mode"]
http = res["http"]
routing = res["routing"]
warnings = res.get("warnings", {})
missing = warnings.get("missing_routing_keys", [])

print(f"mode={mode}")
print(f"  http.requests_total={http['requests_total']}")
print(f"  http.duration_avg_ms={http['duration_avg_ms']:.2f}")
print(f"  http.duration_p95_ms={http['duration_p95_ms']:.2f}")
print(f"  http.failed_rate={http['failed_rate']:.6f}")
print(f"  http.status_200_total={http['status_200_total']}")
print(f"  http.status_503_total={http['status_503_total']}")
print(f"  http.status_other_total={http['status_other_total']}")
print(f"  routing.total_contexts={routing['total_contexts']}")
print(f"  routing.total_isolates={routing['total_isolates']}")
print(f"  routing.total_active_requests={routing['total_active_requests']}")
print(f"  routing.saturated_contexts={routing['saturated_contexts']}")
print(f"  routing.saturated_isolates={routing['saturated_isolates']}")
print(f"  routing.saturated_rejections={routing['saturated_rejections']}")
if missing:
  print(f"  warning.missing_routing_keys={','.join(missing)}")
PY
}

print_comparison_report() {
  local legacy_json="$1"
  local context_json="$2"
  python3 - "$legacy_json" "$context_json" <<'PY'
import json
import pathlib
import sys

legacy = json.loads(pathlib.Path(sys.argv[1]).read_text())
context = json.loads(pathlib.Path(sys.argv[2]).read_text())

l_http = legacy["http"]
c_http = context["http"]
l_route = legacy["routing"]
c_route = context["routing"]

def pct_delta(new, old):
    if old == 0:
        return 0.0
    return ((new - old) / old) * 100.0

print("============================================================")
print("EXTREME CONTEXT+ISOLATE BENCHMARK REPORT")
print("============================================================")
print("legacy mode:")
print(f"  req_total={l_http['requests_total']} avg_ms={l_http['duration_avg_ms']:.2f} p95_ms={l_http['duration_p95_ms']:.2f} failed_rate={l_http['failed_rate']:.6f}")
print(f"  status200={l_http['status_200_total']} status503={l_http['status_503_total']} statusOther={l_http['status_other_total']}")
print(f"  routing contexts={l_route['total_contexts']} isolates={l_route['total_isolates']} sat_ctx={l_route['saturated_contexts']} sat_iso={l_route['saturated_isolates']} sat_rej={l_route['saturated_rejections']}")
print("")
print("context+isolate mode:")
print(f"  req_total={c_http['requests_total']} avg_ms={c_http['duration_avg_ms']:.2f} p95_ms={c_http['duration_p95_ms']:.2f} failed_rate={c_http['failed_rate']:.6f}")
print(f"  status200={c_http['status_200_total']} status503={c_http['status_503_total']} statusOther={c_http['status_other_total']}")
print(f"  routing contexts={c_route['total_contexts']} isolates={c_route['total_isolates']} sat_ctx={c_route['saturated_contexts']} sat_iso={c_route['saturated_isolates']} sat_rej={c_route['saturated_rejections']}")
print("")
print("delta context+isolate vs legacy:")
print(f"  requests_total_delta_pct={pct_delta(c_http['requests_total'], l_http['requests_total']):.2f}")
print(f"  duration_avg_delta_pct={pct_delta(c_http['duration_avg_ms'], l_http['duration_avg_ms']):.2f}")
print(f"  duration_p95_delta_pct={pct_delta(c_http['duration_p95_ms'], l_http['duration_p95_ms']):.2f}")
print(f"  failed_rate_delta_pct={pct_delta(c_http['failed_rate'], l_http['failed_rate']):.2f}")
print(f"  status200_delta_pct={pct_delta(c_http['status_200_total'], l_http['status_200_total']):.2f}")
print(f"  status503_delta_pct={pct_delta(c_http['status_503_total'], l_http['status_503_total']):.2f}")
print("============================================================")
PY
}

echo "[1/4] Running legacy baseline benchmark"
LEGACY_RESULT="$(run_mode "legacy" )"

echo "[2/4] Legacy benchmark summary"
print_mode_report "$LEGACY_RESULT"

echo "[3/4] Running context+isolate benchmark"
CONTEXT_RESULT="$(run_mode "context_isolate" \
  --pool-enabled \
  --pool-global-max-isolates 8 \
  --pool-min-free-memory-mib 0 \
  --context-pool-enabled \
  --max-contexts-per-isolate 2 \
  --max-active-requests-per-context 1 )"

echo "[4/4] Context+isolate benchmark summary"
print_mode_report "$CONTEXT_RESULT"

echo ""
print_comparison_report "$LEGACY_RESULT" "$CONTEXT_RESULT"

echo "artifacts: $TMP_DIR"
