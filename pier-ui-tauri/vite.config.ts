import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
const devPort = Number.parseInt(process.env.PIER_DEV_PORT ?? "1420", 10);
const hmrPort = Number.parseInt(
  process.env.PIER_DEV_HMR_PORT ?? String(devPort + 1),
  10,
);
const strictPort = Boolean(process.env.PIER_DEV_PORT);

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. when launched via the Tauri wrapper we pin the resolved port;
  //    standalone `npm run dev` can still fall back automatically
  server: {
    port: devPort,
    strictPort,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: hmrPort,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
