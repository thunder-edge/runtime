// Example: Streaming Data
// Demonstrates reading and transforming large datasets using streams

Deno.serve(async (req) => {
  const url = new URL(req.url);
  const type = url.searchParams.get("type") || "numbers";

  // Create a ReadableStream that generates data
  const stream = new ReadableStream({
    async start(controller) {
      try {
        if (type === "numbers") {
          // Stream numbers from 1 to 100
          for (let i = 1; i <= 100; i++) {
            const data = JSON.stringify({ number: i, squared: i * i }) + "\n";
            controller.enqueue(data);
            // Simulate processing time
            await new Promise((resolve) => setTimeout(resolve, 10));
          }
        } else if (type === "fibonacci") {
          // Stream Fibonacci sequence
          let a = 0,
            b = 1;
          for (let i = 0; i < 50; i++) {
            const data = JSON.stringify({ position: i, value: a }) + "\n";
            controller.enqueue(data);
            [a, b] = [b, a + b];
            await new Promise((resolve) => setTimeout(resolve, 10));
          }
        } else if (type === "timestamps") {
          // Stream timestamps
          for (let i = 0; i < 20; i++) {
            const data =
              JSON.stringify({
                index: i,
                timestamp: new Date().toISOString(),
              }) + "\n";
            controller.enqueue(data);
            await new Promise((resolve) => setTimeout(resolve, 500));
          }
        }
        controller.close();
      } catch (error) {
        controller.error(error);
      }
    },
  });

  return new Response(stream, {
    headers: {
      "content-type": "application/x-ndjson",
      "transfer-encoding": "chunked",
    },
  });
});
