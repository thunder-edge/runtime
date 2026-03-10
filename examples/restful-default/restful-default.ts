import { EmptyResponse, ErrorResponse, HTTP, JSONResponse } from "thunder:http";

let nextId = 3;
const items = new Map<number, { id: number; name: string }>([
  [1, { id: 1, name: "alpha" }],
  [2, { id: 2, name: "beta" }],
]);

function idFromUrl(req: Request): number | null {
  const match = new URL(req.url).pathname.match(/\/(\d+)$/);
  if (!match) return null;
  const parsed = Number(match[1]);
  return Number.isFinite(parsed) ? parsed : null;
}

export default {
  async GET(req: Request) {
    const id = idFromUrl(req);
    if (id !== null) {
      const item = items.get(id);
      if (!item) {
        return ErrorResponse("not_found")
          .status(HTTP.NotFound)
          .toResponse();
      }
      return JSONResponse(item).toResponse();
    }

    return JSONResponse({
      resource: "items",
      data: Array.from(items.values()),
      hint: "POST /items with { name }, GET /items/:id, DELETE /items/:id",
    })
      .status(HTTP.Ok)
      .toResponse();
  },

  async POST(req: Request) {
    const body = await req.json().catch(() => null);
    const name = typeof body?.name === "string" ? body.name.trim() : "";
    if (!name) {
      return JSONResponse({ error: "invalid_payload", expected: "{ name: string }" })
        .status(HTTP.BadRequest)
        .toResponse();
    }

    const item = { id: nextId++, name };
    items.set(item.id, item);
    return JSONResponse(item)
      .status(HTTP.Created)
      .header("location", `/items/${item.id}`)
      .toResponse();
  },

  async DELETE(req: Request) {
    const id = idFromUrl(req);
    if (id === null) {
      return ErrorResponse("id_required")
        .status(HTTP.BadRequest)
        .toResponse();
    }

    if (!items.delete(id)) {
      return ErrorResponse("not_found")
        .status(HTTP.NotFound)
        .toResponse();
    }

    return EmptyResponse()
      .status(HTTP.NoContent)
      .toResponse();
  },
};
