import fs from "node:fs";

function resolveSync<T>(fn: () => T): Promise<T> {
  try {
    return Promise.resolve(fn());
  } catch (e) {
    return Promise.reject(e);
  }
}

function readFile(path: string, options?: unknown): Promise<unknown> {
  return resolveSync(() => fs.readFileSync(path, options));
}

function writeFile(path: string, data?: unknown): Promise<void> {
  return resolveSync(() => {
    fs.writeFileSync(path, data);
  });
}

function stat(path: string): Promise<unknown> {
  return resolveSync(() => fs.statSync(path));
}

function lstat(path: string): Promise<unknown> {
  return resolveSync(() => fs.lstatSync(path));
}

function readdir(path: string): Promise<string[]> {
  return resolveSync(() => fs.readdirSync(path));
}

function mkdir(path: string, options?: unknown): Promise<void> {
  return resolveSync(() => {
    fs.mkdirSync(path, options as { recursive?: boolean } | undefined);
  });
}

const fsPromises = {
  readFile,
  writeFile,
  stat,
  lstat,
  readdir,
  mkdir,
};

export { readFile, writeFile, stat, lstat, readdir, mkdir };
export default fsPromises;
