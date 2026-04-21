import net from "node:net";

const DEFAULT_START_PORT = 45120;
const SEARCH_WINDOW = 200;
const LOOPBACK_IPV4 = "127.0.0.1";
const LOOPBACK_IPV6 = "::1";

function parsePort(value) {
  if (value === undefined || value === null || value === "") {
    return null;
  }

  const port = Number.parseInt(String(value), 10);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    return null;
  }

  return port;
}

function resolveProbeHosts(host) {
  if (!host || host === "localhost") {
    return [LOOPBACK_IPV4, LOOPBACK_IPV6];
  }

  if (host === "0.0.0.0" || host === "::") {
    return [host, LOOPBACK_IPV4, LOOPBACK_IPV6];
  }

  return [host];
}

function resolveUrlHost(host) {
  if (!host || host === "0.0.0.0" || host === "::") {
    return LOOPBACK_IPV4;
  }

  return host;
}

async function probePort(port, host) {
  return await new Promise((resolve) => {
    const server = net.createServer();

    server.once("error", (error) => {
      if (error.code === "EADDRNOTAVAIL" || error.code === "EAFNOSUPPORT") {
        resolve("unsupported");
        return;
      }

      resolve("in-use");
    });

    server.once("listening", () => {
      server.close(() => resolve("available"));
    });

    server.listen(port, host);
  });
}

async function isPortAvailable(port, hosts) {
  for (const host of hosts) {
    const result = await probePort(port, host);
    if (result === "in-use") {
      return false;
    }
  }

  return true;
}

async function findAvailablePort(startPort, endPort, hosts, excludedPorts = new Set()) {
  for (let port = startPort; port <= endPort; port += 1) {
    if (excludedPorts.has(port)) {
      continue;
    }

    if (await isPortAvailable(port, hosts)) {
      return port;
    }
  }

  throw new Error(`No available port found in range ${startPort}-${endPort}.`);
}

const requestedDevPort = parsePort(process.env.PIER_DEV_PORT) ?? DEFAULT_START_PORT;
const requestedHmrPort = parsePort(process.env.PIER_HMR_PORT);
const probeHosts = resolveProbeHosts(process.env.TAURI_DEV_HOST);
const urlHost = resolveUrlHost(process.env.TAURI_DEV_HOST);
const searchEndPort =
  Math.max(requestedDevPort, requestedHmrPort ?? requestedDevPort + 1) + SEARCH_WINDOW;

const devPort = await findAvailablePort(requestedDevPort, searchEndPort, probeHosts);
const hmrPort = await findAvailablePort(
  requestedHmrPort ?? devPort + 1,
  searchEndPort + 1,
  probeHosts,
  new Set([devPort]),
);

process.stdout.write(`PIER_DEV_PORT=${devPort}\n`);
process.stdout.write(`PIER_HMR_PORT=${hmrPort}\n`);
process.stdout.write(`PIER_DEV_URL=http://${urlHost}:${devPort}\n`);
