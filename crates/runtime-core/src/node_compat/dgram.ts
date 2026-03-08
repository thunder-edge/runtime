type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function createSocket(): never {
  return notImplemented("dgram.createSocket");
}

const dgramModule = { createSocket };

export { createSocket };
export default dgramModule;
