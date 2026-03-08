type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile (sandbox does not allow worker thread spawning)`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

const isMainThread = true;
const threadId = 0;
const workerData = undefined;
const parentPort = null;

class Worker {
  constructor(_filename: string | URL, _options?: unknown) {
    notImplemented("worker_threads.Worker");
  }
}

function receiveMessageOnPort(_port: unknown): never {
  return notImplemented("worker_threads.receiveMessageOnPort");
}

function markAsUntransferable(_object: unknown): never {
  return notImplemented("worker_threads.markAsUntransferable");
}

const workerThreadsModule = {
  Worker,
  isMainThread,
  threadId,
  workerData,
  parentPort,
  receiveMessageOnPort,
  markAsUntransferable,
};

export {
  Worker,
  isMainThread,
  threadId,
  workerData,
  parentPort,
  receiveMessageOnPort,
  markAsUntransferable,
};
export default workerThreadsModule;
