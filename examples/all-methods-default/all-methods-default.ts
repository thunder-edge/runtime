import { GenericResponse, HTTP } from "thunder:http";

export default async function handler(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const method = req.method.toUpperCase();

  if (method === "HEAD") {
    return GenericResponse(null, {
      status: HTTP.NoContent,
      headers: { "x-handler-mode": "all-methods" },
    });
  }

  if (method === "OPTIONS") {
    return GenericResponse({
      status: HTTP.Ok,
      headers: {
        allow: "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS",
        "x-handler-mode": "all-methods",
      },
      body: "",
    });
  }

  const body = method === "POST" || method === "PUT" || method === "PATCH"
    ? await req.text()
    : null;

  return GenericResponse({
    mode: "all-methods",
    method,
    pathname: url.pathname,
    query: url.search,
    body,
  }, {
    status: HTTP.Ok,
    headers: {
      "x-handler-mode": "all-methods",
    },
  });
}
