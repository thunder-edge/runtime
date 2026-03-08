type NodeLikeError = Error & { code?: string };

function notImplemented(api: string): never {
  const err = new Error(
    `[thunder] ${api} is not implemented in this runtime profile`,
  ) as NodeLikeError;
  err.code = "ERR_NOT_IMPLEMENTED";
  throw err;
}

function spawn(): never {
  return notImplemented("child_process.spawn");
}

function exec(): never {
  return notImplemented("child_process.exec");
}

function execFile(): never {
  return notImplemented("child_process.execFile");
}

function fork(): never {
  return notImplemented("child_process.fork");
}

const childProcess = { spawn, exec, execFile, fork };

export { spawn, exec, execFile, fork };
export default childProcess;
