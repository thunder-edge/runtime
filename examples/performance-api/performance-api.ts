// Example: Performance API - Profiling and Metrics
// Demonstrates performance measurement and profiling

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Helper to measure function execution
  async function measureAsync(
    name: string,
    fn: () => Promise<unknown>
  ): Promise<{ result: unknown; duration: number }> {
    const start = performance.now();
    const result = await fn();
    const duration = performance.now() - start;

    performance.mark(`${name}-end`, {
      detail: { duration },
    });

    return { result, duration };
  }

  // Fibonacci calculation for benchmarking
  function fibonacci(n: number): number {
    if (n <= 1) return n;
    return fibonacci(n - 1) + fibonacci(n - 2);
  }

  // API endpoint for performance metrics
  if (url.pathname === "/api/metrics" && req.method === "GET") {
    const operations: Record<string, unknown> = {};

    // Measure different operations
    const metrics = {
      fibonacci_30: (() => {
        const start = performance.now();
        const result = fibonacci(30);
        const duration = performance.now() - start;
        return { result, duration };
      })(),

      array_operations: (() => {
        const start = performance.now();
        const arr = Array.from({ length: 10000 }, (_, i) => i);
        const mapped = arr.map((x) => x * 2);
        const filtered = mapped.filter((x) => x % 3 === 0);
        const sum = filtered.reduce((a, b) => a + b, 0);
        const duration = performance.now() - start;
        return { result: sum, duration };
      })(),

      string_operations: (() => {
        const start = performance.now();
        const str = "Hello, World! ".repeat(1000);
        const upper = str.toUpperCase();
        const lower = str.toLowerCase();
        const split = str.split(",");
        const joined = split.join("|");
        const duration = performance.now() - start;
        return { result: joined.length, duration };
      })(),

      object_operations: (() => {
        const start = performance.now();
        const obj: Record<string, number> = {};
        for (let i = 0; i < 1000; i++) {
          obj[`key_${i}`] = i;
        }
        const keys = Object.keys(obj).length;
        const values = Object.values(obj).reduce((a, b) => a + b, 0);
        const duration = performance.now() - start;
        return { result: keys + values, duration };
      })(),

      json_operations: (() => {
        const start = performance.now();
        const data = {
          items: Array.from({ length: 100 }, (_, i) => ({
            id: i,
            name: `Item ${i}`,
            value: Math.random() * 1000,
          })),
        };
        const json = JSON.stringify(data);
        const parsed = JSON.parse(json);
        const duration = performance.now() - start;
        return { result: parsed.items.length, duration };
      })(),
    };

    return new Response(JSON.stringify({
      timestamp: new Date().toISOString(),
      timeOrigin: performance.timeOrigin,
      metrics,
      summary: {
        total_duration: Object.values(metrics).reduce(
          (sum, m: any) => sum + m.duration,
          0
        ),
      },
    }, null, 2), {
      headers: { "content-type": "application/json" },
    });
  }

  // Benchmark a specific operation
  if (url.pathname === "/api/benchmark" && req.method === "POST") {
    try {
      const { operation, iterations = 1 } = await req.json();

      const measurements: { duration: number; result: unknown }[] = [];

      for (let i = 0; i < iterations; i++) {
        const start = performance.now();
        let result = null;

        switch (operation) {
          case "fibonacci":
            result = fibonacci(25);
            break;
          case "sorting":
            result = Array.from({ length: 1000 }, () => Math.random())
              .sort((a, b) => a - b).length;
            break;
          case "json":
            result = JSON.parse(JSON.stringify({ test: "data" }));
            break;
          case "string":
            result = "test".repeat(100).toUpperCase().length;
            break;
          default:
            return new Response(
              JSON.stringify({ error: "Unknown operation" }),
              { status: 400, headers: { "content-type": "application/json" } }
            );
        }

        const duration = performance.now() - start;
        measurements.push({ duration, result });
      }

      const durations = measurements.map((m) => m.duration);
      const stats = {
        min: Math.min(...durations),
        max: Math.max(...durations),
        avg: durations.reduce((a, b) => a + b, 0) / durations.length,
        median:
          durations.length % 2 === 0
            ? (durations[durations.length / 2 - 1] +
              durations[durations.length / 2]) /
            2
            : durations[Math.floor(durations.length / 2)],
      };

      return new Response(
        JSON.stringify({
          operation,
          iterations,
          measurements,
          stats,
        }, null, 2),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Benchmark failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Performance API</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 1000px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          h2 { color: #667eea; margin: 20px 0 15px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 5px 10px 0; }
          button:hover { background: #764ba2; }
          select, input { padding: 10px; margin: 10px 0; border: 1px solid #ddd; border-radius: 4px; }
          .metric { background: white; padding: 15px; margin: 10px 0; border-radius: 4px; border-left: 4px solid #667eea; }
          .metric-name { font-weight: bold; color: #333; }
          .metric-value { font-size: 1.3em; color: #667eea; margin: 5px 0; }
          .metric-unit { color: #999; font-size: 0.9em; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin: 15px 0; max-height: 400px; overflow-y: auto; font-family: monospace; font-size: 0.85em; white-space: pre-wrap; }
          .graph { background: white; padding: 20px; border-radius: 4px; margin: 15px 0; border: 1px solid #ddd; }
          .bar { height: 25px; background: linear-gradient(90deg, #667eea, #764ba2); margin: 5px 0; border-radius: 4px; display: flex; align-items: center; padding: 0 10px; color: white; font-size: 0.85em; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>📊 Performance API - Profiling & Metrics</h1>
          <p class="subtitle">Measure and analyze code performance</p>

          <div class="section">
            <h2>1. System Metrics</h2>
            <button onclick="getSystemMetrics()">Get Metrics</button>
            <div id="metricsOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>2. Benchmark Operations</h2>
            <select id="operationSelect">
              <option value="fibonacci">Fibonacci (CPU intensive)</option>
              <option value="sorting">Sorting (1000 integers)</option>
              <option value="json">JSON stringify/parse</option>
              <option value="string">String operations</option>
            </select>
            <label style="display: block; margin: 10px 0;">Iterations:</label>
            <input type="number" id="iterations" value="5" min="1" max="100">
            <button onclick="runBenchmark()">Run Benchmark</button>
            <div id="benchmarkOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>3. Performance Visualization</h2>
            <div id="visualization" class="graph"></div>
          </div>

          <div class="section">
            <h2>How It Works</h2>
            <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
              <li><strong>performance.now():</strong> High-resolution timestamp (nanosecond precision)</li>
              <li><strong>performance.mark():</strong> Mark specific points in time</li>
              <li><strong>performance.measure():</strong> Measure time between marks</li>
              <li><strong>performance.timeOrigin:</strong> When the runtime started</li>
            </ul>
          </div>
        </div>

        <script>
          async function getSystemMetrics() {
            try {
              const response = await fetch('/api/metrics');
              const data = await response.json();

              let output = JSON.stringify(data, null, 2);
              document.getElementById('metricsOutput').textContent = output;

              // Also visualize
              visualizeMetrics(data.metrics);
            } catch (e) {
              document.getElementById('metricsOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function runBenchmark() {
            const operation = document.getElementById('operationSelect').value;
            const iterations = parseInt(document.getElementById('iterations').value);

            try {
              const response = await fetch('/api/benchmark', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ operation, iterations })
              });

              const data = await response.json();
              document.getElementById('benchmarkOutput').textContent = JSON.stringify(data, null, 2);

              visualizeBenchmark(data);
            } catch (e) {
              document.getElementById('benchmarkOutput').textContent = 'Error: ' + e.message;
            }
          }

          function visualizeMetrics(metrics) {
            const viz = document.getElementById('visualization');
            let html = '';

            for (const [key, value] of Object.entries(metrics)) {
              const duration = value.duration;
              const barWidth = Math.max(100, duration * 10);
              html += \`
                <div style="margin-bottom: 15px;">
                  <div style="font-weight: bold; margin-bottom: 5px;">\${key}</div>
                  <div class="bar" style="width: \${barWidth}px;">
                    \${duration.toFixed(3)}ms
                  </div>
                </div>
              \`;
            }

            viz.innerHTML = html || '<p style="color: #999;">No data</p>';
          }

          function visualizeBenchmark(data) {
            const stats = data.stats;
            const viz = document.getElementById('visualization');
            let html = \`
              <div style="display: grid; grid-template-columns: 1fr 1fr 1fr 1fr; gap: 10px;">
                <div style="background: #f0f0f0; padding: 10px; border-radius: 4px; text-align: center;">
                  <div style="color: #999; font-size: 0.9em;">Min</div>
                  <div style="font-size: 1.5em; color: #667eea; font-weight: bold;">\${stats.min.toFixed(2)}ms</div>
                </div>
                <div style="background: #f0f0f0; padding: 10px; border-radius: 4px; text-align: center;">
                  <div style="color: #999; font-size: 0.9em;">Avg</div>
                  <div style="font-size: 1.5em; color: #667eea; font-weight: bold;">\${stats.avg.toFixed(2)}ms</div>
                </div>
                <div style="background: #f0f0f0; padding: 10px; border-radius: 4px; text-align: center;">
                  <div style="color: #999; font-size: 0.9em;">Median</div>
                  <div style="font-size: 1.5em; color: #667eea; font-weight: bold;">\${stats.median.toFixed(2)}ms</div>
                </div>
                <div style="background: #f0f0f0; padding: 10px; border-radius: 4px; text-align: center;">
                  <div style="color: #999; font-size: 0.9em;">Max</div>
                  <div style="font-size: 1.5em; color: #667eea; font-weight: bold;">\${stats.max.toFixed(2)}ms</div>
                </div>
              </div>
            \`;

            viz.innerHTML = html;
          }
        </script>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  return new Response("Not found", { status: 404 });
});
