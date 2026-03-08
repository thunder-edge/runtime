type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

const isPrimary = true;
const isWorker = false;
const worker = undefined;

function setupPrimary(): never {
  return notImplemented("cluster.setupPrimary");
}

function fork(): never {
  return notImplemented("cluster.fork");
}

function disconnect(): void {}

const cluster = {
  isPrimary,
  isWorker,
  worker,
  setupPrimary,
  fork,
  disconnect,
};

export { isPrimary, isWorker, worker, setupPrimary, fork, disconnect };
export default cluster;
