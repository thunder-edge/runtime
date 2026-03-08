type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

const constants = {
  NGHTTP2_NO_ERROR: 0,
};

function createServer(): never {
  return notImplemented("http2.createServer");
}

function connect(): never {
  return notImplemented("http2.connect");
}

const http2Module = { constants, createServer, connect };

export { constants, createServer, connect };
export default http2Module;
