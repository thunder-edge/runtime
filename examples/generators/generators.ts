// Example: Generators - Lazy Data Processing
// Demonstrates generator functions for memory-efficient operations

declare const Deno: any;

Deno.serve(async (req: Request) => {
  const url = new URL(req.url);

  // Generator function: create a sequence of numbers
  function* generateNumbers(start: number, end: number) {
    for (let i = start; i <= end; i++) {
      yield i;
    }
  }

  // Generator function: filter values
  function* filterGenerator<T>(
    iterable: Iterable<T>,
    predicate: (value: T) => boolean
  ): Generator<T> {
    for (const value of iterable) {
      if (predicate(value)) {
        yield value;
      }
    }
  }

  // Generator function: map values
  function* mapGenerator<T, U>(
    iterable: Iterable<T>,
    mapper: (value: T) => U
  ): Generator<U> {
    for (const value of iterable) {
      yield mapper(value);
    }
  }

  // Generator function: take n elements
  function* takeGenerator<T>(iterable: Iterable<T>, n: number): Generator<T> {
    let count = 0;
    for (const value of iterable) {
      if (count >= n) break;
      yield value;
      count++;
    }
  }

  // Generator function: Fibonacci sequence
  function* fibonacci(max: number = 100): Generator<number> {
    let a = 0,
      b = 1;
    while (a <= max) {
      yield a;
      [a, b] = [b, a + b];
    }
  }

  // Generator function: paginate data
  function* paginate<T>(
    array: T[],
    pageSize: number
  ): Generator<T[]> {
    for (let i = 0; i < array.length; i += pageSize) {
      yield array.slice(i, i + pageSize);
    }
  }

  // API endpoint: generate and transform data
  if (url.pathname === "/api/generate" && req.method === "GET") {
    const start = parseInt(url.searchParams.get("start") || "1");
    const end = parseInt(url.searchParams.get("end") || "100");
    const operation = url.searchParams.get("op") || "all";

    const results: number[] = [];

    if (operation === "all") {
      for (const num of generateNumbers(start, end)) {
        results.push(num);
      }
    } else if (operation === "even") {
      for (const num of filterGenerator(generateNumbers(start, end), (n) => n % 2 === 0)) {
        results.push(num);
      }
    } else if (operation === "odd") {
      for (const num of filterGenerator(generateNumbers(start, end), (n) => n % 2 !== 0)) {
        results.push(num);
      }
    } else if (operation === "squared") {
      for (const num of mapGenerator(generateNumbers(start, end), (n) => n * n)) {
        results.push(num);
      }
    } else if (operation === "first10") {
      for (const num of takeGenerator(generateNumbers(start, end), 10)) {
        results.push(num);
      }
    }

    return new Response(
      JSON.stringify({
        operation,
        start,
        end,
        count: results.length,
        results: results.slice(0, 100),
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // API endpoint: Fibonacci sequence
  if (url.pathname === "/api/fibonacci" && req.method === "GET") {
    const max = parseInt(url.searchParams.get("max") || "1000");
    const limit = parseInt(url.searchParams.get("limit") || "50");

    const results: number[] = [];
    let count = 0;

    for (const num of fibonacci(max)) {
      if (count >= limit) break;
      results.push(num);
      count++;
    }

    return new Response(
      JSON.stringify({
        max,
        limit,
        count: results.length,
        results,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // API endpoint: paginate data
  if (url.pathname === "/api/paginate" && req.method === "GET") {
    const page = parseInt(url.searchParams.get("page") || "1");
    const pageSize = parseInt(url.searchParams.get("size") || "10");

    // Sample data
    const data = Array.from({ length: 100 }, (_, i) => ({
      id: i + 1,
      name: `Item ${i + 1}`,
      value: Math.random() * 1000,
    }));

    const pages = Array.from(paginate(data, pageSize));
    const currentPage = pages[page - 1] || [];

    return new Response(
      JSON.stringify({
        page,
        pageSize,
        totalPages: pages.length,
        totalItems: data.length,
        items: currentPage,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Generators</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 1000px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          h2 { color: #667eea; margin: 20px 0 15px; }
          .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 15px; margin: 15px 0; }
          input, select { width: 100%; padding: 10px; margin: 5px 0; border: 1px solid #ddd; border-radius: 4px; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin: 15px 0; max-height: 400px; overflow-y: auto; font-family: monospace; font-size: 0.85em; }
          .stat { background: white; padding: 15px; border-radius: 4px; border-left: 4px solid #667eea; }
          .stat-value { font-size: 1.5em; color: #667eea; font-weight: bold; }
          .stat-label { color: #999; font-size: 0.9em; margin-top: 5px; }
          .example-btn { background: #e8f4f8; color: #4a90e2; border: 1px solid #4a90e2; margin: 5px 5px 5px 0; cursor: pointer; }
          .example-btn:hover { background: #4a90e2; color: white; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🔄 Generators - Lazy Data Processing</h1>
          <p class="subtitle">Memory-efficient data processing using generator functions</p>

          <div class="section">
            <h2>1. Number Generation & Filtering</h2>
            <div class="grid">
              <div>
                <label>Start:</label>
                <input type="number" id="genStart" value="1" min="1">
              </div>
              <div>
                <label>End:</label>
                <input type="number" id="genEnd" value="100" min="1">
              </div>
            </div>
            <select id="genOperation">
              <option value="all">All Numbers</option>
              <option value="even">Even Numbers Only</option>
              <option value="odd">Odd Numbers Only</option>
              <option value="squared">Squared Values</option>
              <option value="first10">First 10 Only</option>
            </select>
            <button onclick="generateNumbers()">Generate</button>
            <div id="genOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>2. Fibonacci Sequence</h2>
            <div class="grid">
              <div>
                <label>Max Value:</label>
                <input type="number" id="fibMax" value="1000" min="1">
              </div>
              <div>
                <label>Limit Results:</label>
                <input type="number" id="fibLimit" value="20" min="1">
              </div>
            </div>
            <button onclick="generateFibonacci()">Generate Fibonacci</button>
            <div id="fibOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>3. Pagination with Generators</h2>
            <div class="grid">
              <div>
                <label>Page:</label>
                <input type="number" id="pagePage" value="1" min="1">
              </div>
              <div>
                <label>Page Size:</label>
                <input type="number" id="pageSize" value="10" min="1">
              </div>
            </div>
            <button onclick="paginateData()">Fetch Page</button>
            <button class="example-btn" onclick="navigatePages(1)">Page 1</button>
            <button class="example-btn" onclick="navigatePages(5)">Page 5</button>
            <button class="example-btn" onclick="navigatePages(10)">Page 10</button>
            <div id="pageOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>How Generators Work</h2>
            <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
              <li><strong>function*:</strong> Defines a generator function</li>
              <li><strong>yield:</strong> Pauses execution and returns a value</li>
              <li><strong>Generator.next():</strong> Resumes and gets next value</li>
              <li><strong>for...of:</strong> Automatically iterate through all yielded values</li>
              <li><strong>Memory efficient:</strong> Values computed on-demand, not stored upfront</li>
            </ul>
          </div>

          <div class="section">
            <h2>Benefits</h2>
            <ul style="margin-left: 20px; line-height: 1.8; color: #666;">
              <li>💾 <strong>Memory:</strong> Lazy evaluation - values computed when needed</li>
              <li>⚡ <strong>Performance:</strong> Can stop iteration early without processing all</li>
              <li>🔄 <strong>Composable:</strong> Easy to chain multiple transformations</li>
              <li>🎯 <strong>Readable:</strong> Natural syntax for iterative algorithms</li>
            </ul>
          </div>
        </div>

        <script>
          async function generateNumbers() {
            const start = document.getElementById('genStart').value;
            const end = document.getElementById('genEnd').value;
            const op = document.getElementById('genOperation').value;

            try {
              const response = await fetch(\`/api/generate?start=\${start}&end=\${end}&op=\${op}\`);
              const data = await response.json();
              document.getElementById('genOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('genOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function generateFibonacci() {
            const max = document.getElementById('fibMax').value;
            const limit = document.getElementById('fibLimit').value;

            try {
              const response = await fetch(\`/api/fibonacci?max=\${max}&limit=\${limit}\`);
              const data = await response.json();
              document.getElementById('fibOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('fibOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function paginateData() {
            const page = document.getElementById('pagePage').value;
            const size = document.getElementById('pageSize').value;

            try {
              const response = await fetch(\`/api/paginate?page=\${page}&size=\${size}\`);
              const data = await response.json();
              document.getElementById('pageOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('pageOutput').textContent = 'Error: ' + e.message;
            }
          }

          function navigatePages(page) {
            document.getElementById('pagePage').value = page;
            paginateData();
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
