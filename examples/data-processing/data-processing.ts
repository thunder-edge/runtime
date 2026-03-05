// Example: Data Processing (CSV/JSON)
// Demonstrates parsing and transforming data formats

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Parse CSV helper
  function parseCSV(csv: string): Record<string, unknown>[] {
    const lines = csv.trim().split("\n");
    if (lines.length < 1) return [];

    const headers = lines[0].split(",").map((h) => h.trim());
    const rows: Record<string, unknown>[] = [];

    for (let i = 1; i < lines.length; i++) {
      const values = lines[i].split(",").map((v) => v.trim());
      const row: Record<string, unknown> = {};

      for (let j = 0; j < headers.length; j++) {
        row[headers[j]] = values[j];
      }

      rows.push(row);
    }

    return rows;
  }

  // Convert to CSV helper
  function toCSV(data: Record<string, unknown>[]): string {
    if (data.length === 0) return "";

    const headers = Object.keys(data[0]);
    const csv = [
      headers.join(","),
      ...data.map((row) =>
        headers.map((h) => JSON.stringify(row[h])).join(",")
      ),
    ].join("\n");

    return csv;
  }

  // Home page
  if (url.pathname === "/" && req.method === "GET") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Data Processing</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; padding: 40px; background: #f5f5f5; }
          .container { max-width: 1000px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; }
          h1 { color: #333; margin-bottom: 10px; }
          .subtitle { color: #999; margin-bottom: 30px; }
          .section { margin: 30px 0; padding: 20px; background: #f9f9f9; border-radius: 4px; border-left: 4px solid #667eea; }
          textarea { width: 100%; padding: 12px; border: 1px solid #ddd; border-radius: 4px; font-family: monospace; font-size: 0.9em; resize: vertical; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 5px 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border-radius: 4px; border: 1px solid #ddd; margin-top: 10px; max-height: 300px; overflow-y: auto; }
          code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>📊 Data Processing</h1>
          <p class="subtitle">Convert, parse, and transform data between formats</p>

          <div class="section">
            <h2>Sample CSV Data:</h2>
            <p style="margin-bottom: 10px;">Try the examples below or paste your own data.</p>
            <textarea id="csvInput" rows="6">name,email,age,city
John Doe,john@example.com,28,New York
Jane Smith,jane@example.com,34,Los Angeles
Bob Johnson,bob@example.com,45,Chicago</textarea>
            <div>
              <button onclick="testCSVToJSON()">Convert CSV to JSON</button>
              <button onclick="testJSONToCSV()">Convert JSON to CSV</button>
              <button onclick="testDataProcessing()">Process Data</button>
              <button onclick="clearOutput()">Clear</button>
            </div>
            <div class="output" id="output"></div>
          </div>

          <div class="section">
            <h2>API Endpoints:</h2>
            <p><code>POST /api/csv-to-json</code> - Convert CSV to JSON</p>
            <p><code>POST /api/json-to-csv</code> - Convert JSON to CSV</p>
            <p><code>POST /api/process</code> - Process and analyze data</p>
          </div>
        </div>

        <script>
          async function testCSVToJSON() {
            const csv = document.getElementById('csvInput').value;
            const response = await fetch('/api/csv-to-json', {
              method: 'POST',
              headers: { 'Content-Type': 'text/plain' },
              body: csv
            });
            const data = await response.json();
            document.getElementById('output').textContent = JSON.stringify(data, null, 2);
          }

          async function testJSONToCSV() {
            const csv = document.getElementById('csvInput').value;
            const response = await fetch('/api/csv-to-json', {
              method: 'POST',
              headers: { 'Content-Type': 'text/plain' },
              body: csv
            });
            const jsonData = await response.json();

            const csvResponse = await fetch('/api/json-to-csv', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(jsonData)
            });
            const csvResult = await csvResponse.text();
            document.getElementById('output').textContent = csvResult;
          }

          async function testDataProcessing() {
            const csv = document.getElementById('csvInput').value;
            const response = await fetch('/api/process', {
              method: 'POST',
              headers: { 'Content-Type': 'text/plain' },
              body: csv
            });
            const data = await response.json();
            document.getElementById('output').textContent = JSON.stringify(data, null, 2);
          }

          function clearOutput() {
            document.getElementById('output').textContent = '';
          }
        </script>
      </body>
      </html>
    `;

    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  // CSV to JSON endpoint
  if (url.pathname === "/api/csv-to-json" && req.method === "POST") {
    try {
      const csv = await req.text();
      const data = parseCSV(csv);

      return new Response(JSON.stringify(data, null, 2), {
        headers: { "content-type": "application/json" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Failed to parse CSV",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // JSON to CSV endpoint
  if (url.pathname === "/api/json-to-csv" && req.method === "POST") {
    try {
      const json = await req.json();
      const csv = toCSV(json);

      return new Response(csv, {
        headers: {
          "content-type": "text/csv",
          "content-disposition": 'attachment; filename="data.csv"',
        },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Failed to convert to CSV",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Data processing endpoint
  if (url.pathname === "/api/process" && req.method === "POST") {
    try {
      const csv = await req.text();
      const data = parseCSV(csv);

      // Process data
      const processed = {
        total_records: data.length,
        columns: data.length > 0 ? Object.keys(data[0]) : [],
        data: data,
        stats: {
          ages: data
            .map((r) => {
              const age = parseInt(r.age as string);
              return isNaN(age) ? null : age;
            })
            .filter((a): a is number => a !== null),
        },
      };

      // Add statistics if age data exists
      if (processed.stats.ages.length > 0) {
        const ages = processed.stats.ages;
        processed.stats.age_average = (
          ages.reduce((a, b) => a + b, 0) / ages.length
        ).toFixed(2);
        processed.stats.age_min = Math.min(...ages);
        processed.stats.age_max = Math.max(...ages);
      }

      return new Response(JSON.stringify(processed, null, 2), {
        headers: { "content-type": "application/json" },
      });
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Failed to process data",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  return new Response("Not found", { status: 404 });
});
