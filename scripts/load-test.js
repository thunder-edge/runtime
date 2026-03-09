import http from 'k6/http';
import { check, group, sleep } from 'k6';

// Configuration
const BASE_URL = __ENV.BASE_URL || 'http://localhost:8080';
const ADMIN_URL = __ENV.ADMIN_URL || 'http://localhost:9000';
const EXAMPLES = [
  'hello',
  // 'json-api',
  // 'cors',
  // 'basic-auth',
  'error-handling',
  'middleware',
  'url-redirect',
  'documentation',
  'html-page',
  
];

export const options = {
  stages: [
    { duration: '10s', target: 150 },    // Cold start test: 50 VUs for 10s
    { duration: '30s', target: 150 },   // Warm start ramp: 50 VUs for 990s
    { duration: '0s', target: 0 },      // Cooldown: ramp down to 0 VUs
  ],
};

export function setup() {
  console.log('📊 Starting load test setup...');
  console.log(`   Base URL: ${BASE_URL}`);
  console.log(`   Admin URL: ${ADMIN_URL}`);
  console.log(`   Examples: ${EXAMPLES.join(', ')}`);

  // Collect initial metrics
  const metricsRes = http.get(`${ADMIN_URL}/_internal/metrics`);
  return {
    initialMetrics: metricsRes.body,
    startTime: new Date().toISOString(),
  };
}

export default function (data) {
  // Test each example with varied load
  for (const example of EXAMPLES) {
    group(`${example} - API Test`, () => {
      const url = `${BASE_URL}/${example}`;

      const res = http.get(url);

      check(res, {
        'status is 200': (r) => r.status === 200,
        'response time < 500ms': (r) => r.timings.duration < 500,
        'has content': (r) => typeof r.body === 'string' && r.body.length > 0,
      });
    });

    sleep(0.5); // Light sleep between requests
  }
}

export function teardown(data) {
  console.log('\n📊 Collecting final metrics...');

  // Collect final metrics
  const metricsRes = http.get(`${ADMIN_URL}/_internal/metrics`);

  if (metricsRes.status === 200) {
    try {
      const metrics = JSON.parse(metricsRes.body);

      console.log('\n═══════════════════════════════════════════════════════════');
      console.log('📊 LOAD TEST METRICS SUMMARY');
      console.log('═══════════════════════════════════════════════════════════');

      if (metrics.functions && Array.isArray(metrics.functions)) {
        for (const fn of metrics.functions) {
          console.log(`\n📦 Function: ${fn.name}`);
          console.log(`   Status: ${fn.status}`);
          console.log(`   Total Requests: ${fn.metrics.total_requests}`);
          console.log(`   Cold Starts: ${fn.metrics.cold_starts}`);
          console.log(`   Avg Cold Start: ${fn.metrics.avg_cold_start_ms}ms`);
          console.log(`   Avg Warm Request: ${fn.metrics.avg_warm_request_ms}ms`);
          console.log(`   Total Errors: ${fn.metrics.total_errors}`);
        }
      }

      console.log('\n═══════════════════════════════════════════════════════════');
    } catch (e) {
      console.log('⚠️  Could not parse metrics:', e);
    }
  } else {
    console.log(`⚠️  Failed to get metrics (status: ${metricsRes.status})`);
  }

  console.log(`\nTest started at: ${data.startTime}`);
  console.log(`Test ended at: ${new Date().toISOString()}`);
}
