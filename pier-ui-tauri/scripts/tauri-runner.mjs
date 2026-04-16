import { spawn } from "node:child_process";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiDir = path.resolve(__dirname, "..");
const tauriCliEntry = path.join(
  uiDir,
  "node_modules",
  "@tauri-apps",
  "cli",
  "tauri.js",
);

const args = process.argv.slice(2);

function parsePort(value, fallback) {
  if (!value) {
    return fallback;
  }

  const port = Number.parseInt(value, 10);
  if (Number.isNaN(port) || port < 1 || port > 65535) {
    return fallback;
  }

  return port;
}

function canListenOnHost(port, host) {
  return new Promise((resolve) => {
    const tester = net.createServer();

    tester.once("error", () => resolve(false));
    tester.once("listening", () => {
      tester.close(() => resolve(true));
    });

    tester.listen({
      port,
      host,
      exclusive: true,
    });
  });
}

async function canListenOnPort(port) {
  const tauriHost = process.env.TAURI_DEV_HOST;
  const hostsToCheck = tauriHost
    ? [tauriHost]
    : ["127.0.0.1", "::1"];

  for (const host of hostsToCheck) {
    if (!(await canListenOnHost(port, host))) {
      return false;
    }
  }

  return true;
}

async function findAvailablePort(startPort, reservedPorts = new Set()) {
  for (let port = startPort; port <= 65535; port += 1) {
    if (reservedPorts.has(port)) {
      continue;
    }

    if (await canListenOnPort(port)) {
      return port;
    }
  }

  throw new Error(`No free TCP port found starting from ${startPort}.`);
}

function spawnTauri(commandArgs) {
  const child = spawn(process.execPath, [tauriCliEntry, ...commandArgs], {
    cwd: uiDir,
    env: process.env,
    stdio: "inherit",
  });

  child.on("error", (error) => {
    console.error(`[pier-ui-tauri] Failed to launch Tauri CLI: ${error.message}`);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }

    process.exit(code ?? 0);
  });
}

async function main() {
  if (args[0] !== "dev") {
    spawnTauri(args);
    return;
  }

  const requestedPort = parsePort(process.env.PIER_DEV_PORT, 1420);
  const devPort = await findAvailablePort(requestedPort);
  const requestedHmrPort = parsePort(process.env.PIER_DEV_HMR_PORT, devPort + 1);
  const hmrPort = await findAvailablePort(requestedHmrPort, new Set([devPort]));

  process.env.PIER_DEV_PORT = String(devPort);
  process.env.PIER_DEV_HMR_PORT = String(hmrPort);

  if (devPort !== requestedPort) {
    console.log(
      `[pier-ui-tauri] Port ${requestedPort} is busy, falling back to ${devPort}.`,
    );
  }

  const devUrl = `http://localhost:${devPort}`;
  const configOverride = JSON.stringify({
    build: {
      devUrl,
    },
  });

  spawnTauri([...args, "--config", configOverride]);
}

main().catch((error) => {
  console.error(`[pier-ui-tauri] ${error.message}`);
  process.exit(1);
});
