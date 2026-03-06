// Example: HTTP Request
// Demonstrates making HTTP requests and returning the response

Deno.serve(async (req) => {
  try {
    // Make a request to JSONPlaceholder (free public API)
    const response = await fetch("https://jsonplaceholder.typicode.com/posts/1");
    const data = await response.json();

    return new Response(JSON.stringify(data, null, 2), {
      headers: { "content-type": "application/json" },
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return new Response(JSON.stringify({ error: message }), {
      status: 500,
      headers: { "content-type": "application/json" },
    });
  }
});
