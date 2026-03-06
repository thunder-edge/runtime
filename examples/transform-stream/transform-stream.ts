// Example: TransformStream - Pipeline Data Processing
// Demonstrates chaining multiple transformations

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Custom transform: convert to uppercase
  const uppercaseTransform = new TransformStream({
    transform(chunk: string, controller) {
      controller.enqueue(chunk.toUpperCase());
    },
  });

  // Custom transform: add line numbers
  const lineNumberTransform = new TransformStream({
    transform(chunk: string, controller) {
      const lines = chunk.split("\n");
      const numbered = lines
        .map((line: string, i: number) => `${String(i + 1).padStart(3, " ")} | ${line}`)
        .join("\n");
      controller.enqueue(numbered);
    },
  });

  // Custom transform: count characters
  let count = 0;

  const charCountTransform = new TransformStream<string, string>({
    start(
      _controller: TransformStreamDefaultController<string>
    ) {
      count = 0;
    },
    transform(
      chunk: string,
      controller: TransformStreamDefaultController<string>
    ) {
      count += chunk.length;
      controller.enqueue(chunk);
    },
    flush(controller: TransformStreamDefaultController<string>) {
      controller.enqueue(`\n\n[Total characters: ${count}]`);
    },
  });

  // Custom transform: CSV to formatted table
  const csvTableTransform = new TransformStream({
    transform(chunk: string, controller) {
      const lines = chunk.trim().split("\n");
      if (lines.length === 0) return;

      const headers = lines[0].split(",").map((h: string) => h.trim());
      const rows = lines.slice(1);

      let output = "┌─" + headers.map(() => "─".repeat(20)).join("─┬─") + "─┐\n";
      output += "│ " + headers.map((h: string) => h.padEnd(20)).join(" │ ") + " │\n";
      output += "├─" + headers.map(() => "─".repeat(20)).join("─┼─") + "─┤\n";

      rows.forEach((row: string) => {
        const cols = row.split(",").map((c: string) => c.trim().padEnd(20));
        output += "│ " + cols.join(" │ ") + " │\n";
      });

      output += "└─" + headers.map(() => "─".repeat(20)).join("─┴─") + "─┘\n";
      controller.enqueue(output);
    },
  });

  // Pipe endpoint
  if (url.pathname === "/api/transform" && req.method === "POST") {
    try {
      const type = url.searchParams.get("type") || "uppercase";
      const body = await req.text();

      let result = body;

      switch (type) {
        case "uppercase": {
          const chunks: string[] = [];
          const reader = ReadableStream.from([body])
            .pipeThrough(uppercaseTransform)
            .getReader();
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
          }
          result = chunks.join("");
          break;
        }
        case "linenumber": {
          const chunks: string[] = [];
          const reader = ReadableStream.from([body])
            .pipeThrough(lineNumberTransform)
            .getReader();
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
          }
          result = chunks.join("");
          break;
        }
        case "charcount": {
          const chunks: string[] = [];
          const reader = ReadableStream.from([body])
            .pipeThrough(charCountTransform)
            .getReader();
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
          }
          result = chunks.join("");
          break;
        }
        case "csvtable": {
          const chunks: string[] = [];
          const reader = ReadableStream.from([body])
            .pipeThrough(csvTableTransform)
            .getReader();
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
          }
          result = chunks.join("");
          break;
        }
      }

      return new Response(
        JSON.stringify({
          type,
          original: body,
          transformed: result,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Transform failed",
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
        <title>TransformStream</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 1000px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          h2 { color: #667eea; margin: 30px 0 15px; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          textarea { width: 100%; padding: 12px; border: 1px solid #ddd; border-radius: 4px; font-family: monospace; font-size: 0.9em; }
          select { padding: 10px; border: 1px solid #ddd; border-radius: 4px; margin: 5px 0; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin-top: 15px; max-height: 300px; overflow-y: auto; white-space: pre; font-family: monospace; font-size: 0.85em; }
          .pipes { display: flex; align-items: center; gap: 10px; margin: 15px 0; font-size: 0.9em; }
          .pipe-item { background: #e8f4f8; padding: 8px 12px; border-radius: 4px; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🔄 TransformStream - Pipeline Processing</h1>
          <p style="color: #999; margin-bottom: 20px;">Chain multiple data transformations using Web Streams</p>

          <div class="section">
            <h2>Transform Types</h2>
            <select id="transformType">
              <option value="uppercase">Uppercase</option>
              <option value="linenumber">Add Line Numbers</option>
              <option value="charcount">Count Characters</option>
              <option value="csvtable">Format CSV as Table</option>
            </select>
            <div class="pipes" id="pipeVisualization"></div>
          </div>

          <div class="section">
            <h2>Input</h2>
            <textarea id="inputData" placeholder="Enter text..." rows="8">The quick brown fox jumps over the lazy dog
This is a test of the transform stream
Multiple lines of text will be processed</textarea>
            <button onclick="loadExample('text')">Load Text</button>
            <button onclick="loadExample('csv')">Load CSV</button>
            <button onclick="doTransform()">Transform</button>
          </div>

          <div class="section">
            <h2>Output</h2>
            <div id="outputData" class="output"></div>
          </div>

          <h2>How It Works</h2>
          <ul style="line-height: 1.8; color: #666; margin: 15px 20px;">
            <li><strong>ReadableStream.from():</strong> Creates a readable stream from input</li>
            <li><strong>pipeThrough():</strong> Passes data through transform pipeline</li>
            <li><strong>TransformStream:</strong> Each transform can read, modify, and enqueue data</li>
            <li><strong>getReader():</strong> Consumes the transformed output</li>
          </ul>
        </div>

        <script>
          const transforms = {
            uppercase: ["Input", "Uppercase", "Output"],
            linenumber: ["Input", "Line Numbering", "Output"],
            charcount: ["Input", "Read all", "Count", "Output"],
            csvtable: ["Input", "Parse CSV", "Format Table", "Output"]
          };

          const examples = {
            text: 'The quick brown fox jumps over the lazy dog\\nThis is a test of transform streams\\nMultiple lines will be processed',
            csv: 'name,email,score\\nAlice,alice@example.com,95\\nBob,bob@example.com,87\\nCharlie,charlie@example.com,92'
          };

          function updatePipeVisualization() {
            const type = document.getElementById('transformType').value;
            const steps = transforms[type] || transforms.uppercase;
            const html = steps.map(s => \`<div class="pipe-item">\${s}</div>\`).join(' ➜ ');
            document.getElementById('pipeVisualization').innerHTML = html;
          }

          async function doTransform() {
            const type = document.getElementById('transformType').value;
            const input = document.getElementById('inputData').value;

            try {
              const response = await fetch('/api/transform?type=' + type, {
                method: 'POST',
                body: input
              });
              const data = await response.json();
              document.getElementById('outputData').textContent = data.transformed;
            } catch (e) {
              document.getElementById('outputData').textContent = 'Error: ' + e.message;
            }
          }

          function loadExample(type) {
            document.getElementById('inputData').value = examples[type];
          }

          document.getElementById('transformType').addEventListener('change', updatePipeVisualization);
          updatePipeVisualization();
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
