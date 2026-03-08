const EOL = "\n";

function arch(): string {
  return "x64";
}

function platform(): string {
  return "linux";
}

function type(): string {
  return "Linux";
}

function release(): string {
  return "edge-runtime";
}

function version(): string {
  return "edge-runtime";
}

function endianness(): "LE" | "BE" {
  return "LE";
}

function hostname(): string {
  return "edge-runtime";
}

function uptime(): number {
  return 0;
}

function loadavg(): [number, number, number] {
  return [0, 0, 0];
}

function totalmem(): number {
  return 0;
}

function freemem(): number {
  return 0;
}

function cpus() {
  return [];
}

function networkInterfaces() {
  return {};
}

function homedir(): string {
  return "/";
}

function tmpdir(): string {
  return "/tmp";
}

function machine(): string {
  return arch();
}

function availableParallelism(): number {
  return 1;
}

const constants = Object.freeze({
  signals: {},
  errno: {},
  priority: {
    PRIORITY_LOW: 19,
    PRIORITY_BELOW_NORMAL: 10,
    PRIORITY_NORMAL: 0,
    PRIORITY_ABOVE_NORMAL: -7,
    PRIORITY_HIGH: -14,
    PRIORITY_HIGHEST: -20,
  },
});

const devNull = "/dev/null";

function getPriority(): number {
  return 0;
}

function setPriority(): void {
  throw new Error("[thunder] os.setPriority is not implemented in this runtime profile");
}

function userInfo() {
  return {
    uid: 0,
    gid: 0,
    username: "edge",
    homedir: homedir(),
    shell: "",
  };
}

const osModule = {
  EOL,
  arch,
  platform,
  type,
  release,
  version,
  endianness,
  hostname,
  uptime,
  loadavg,
  totalmem,
  freemem,
  cpus,
  networkInterfaces,
  homedir,
  tmpdir,
  machine,
  availableParallelism,
  constants,
  devNull,
  getPriority,
  setPriority,
  userInfo,
};

export {
  EOL,
  arch,
  platform,
  type,
  release,
  version,
  endianness,
  hostname,
  uptime,
  loadavg,
  totalmem,
  freemem,
  cpus,
  networkInterfaces,
  homedir,
  tmpdir,
  machine,
  availableParallelism,
  constants,
  devNull,
  getPriority,
  setPriority,
  userInfo,
};

export default osModule;
