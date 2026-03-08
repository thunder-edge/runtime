import http from 'k6/http';
import { Counter, Rate } from 'k6/metrics';
import { check, sleep } from 'k6';

const BASE_URL = __ENV.BASE_URL || 'http://127.0.0.1:9010';
const FUNCTION_NAME = __ENV.FUNCTION_NAME || 'context-extreme';
const HOLD_MS = Number(__ENV.HOLD_MS || '40');

const VUS_WARMUP = Number(__ENV.VUS_WARMUP || '40');
const VUS_STEADY = Number(__ENV.VUS_STEADY || '120');
const VUS_EXTREME = Number(__ENV.VUS_EXTREME || '300');

const DUR_WARMUP = __ENV.DUR_WARMUP || '10s';
const DUR_STEADY = __ENV.DUR_STEADY || '20s';
const DUR_EXTREME = __ENV.DUR_EXTREME || '30s';
const DUR_COOLDOWN = __ENV.DUR_COOLDOWN || '10s';

const status200 = new Counter('status_200_total');
const status503 = new Counter('status_503_total');
const statusOther = new Counter('status_other_total');
const deterministicStatusRate = new Rate('deterministic_status_rate');

export const options = {
  discardResponseBodies: true,
  thresholds: {
    deterministic_status_rate: ['rate>0.98'],
  },
  stages: [
    { duration: DUR_WARMUP, target: VUS_WARMUP },
    { duration: DUR_STEADY, target: VUS_STEADY },
    { duration: DUR_EXTREME, target: VUS_EXTREME },
    { duration: DUR_COOLDOWN, target: 0 },
  ],
};

export function setup() {
  const metrics = http.get(`${BASE_URL}/_internal/metrics`);
  return {
    startAt: new Date().toISOString(),
    metricsOk: metrics.status === 200,
  };
}

export default function () {
  const jitter = Math.floor(Math.random() * 8);
  const url = `${BASE_URL}/${FUNCTION_NAME}?d=${HOLD_MS + jitter}`;
  const res = http.get(url, {
    headers: {
      connection: 'close',
    },
    timeout: '5s',
  });

  const isDeterministic = res.status === 200 || res.status === 503;
  deterministicStatusRate.add(isDeterministic);

  if (res.status === 200) {
    status200.add(1);
  } else if (res.status === 503) {
    status503.add(1);
  } else {
    statusOther.add(1);
  }

  check(res, {
    'status is deterministic (200 or 503)': () => isDeterministic,
    'latency below 4000ms': (r) => r.timings.duration < 4000,
  });

  sleep(0.02);
}

export function teardown(data) {
  const finalMetrics = http.get(`${BASE_URL}/_internal/metrics`);
  if (finalMetrics.status === 200) {
    try {
      const json = JSON.parse(finalMetrics.body);
      const routing = json.routing || {};
      console.log('[extreme-load] routing metrics snapshot:');
      console.log(`  total_contexts=${routing.total_contexts || 0}`);
      console.log(`  total_isolates=${routing.total_isolates || 0}`);
      console.log(`  total_active_requests=${routing.total_active_requests || 0}`);
      console.log(`  saturated_contexts=${routing.saturated_contexts || 0}`);
      console.log(`  saturated_isolates=${routing.saturated_isolates || 0}`);
      console.log(`  saturated_rejections=${routing.saturated_rejections || 0}`);
    } catch (_err) {
      console.log('[extreme-load] unable to parse /_internal/metrics payload');
    }
  }

  console.log(`[extreme-load] started_at=${data.startAt}`);
  console.log(`[extreme-load] ended_at=${new Date().toISOString()}`);
}
