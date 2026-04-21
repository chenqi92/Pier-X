import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],

  // Tauri expects a fixed dev server host/port (see src-tauri/tauri.conf.json).
  clearScreen: false,
  server: {
    port: 45120,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? { protocol: "ws", host, port: 45121 }
      : { protocol: "ws", host: "127.0.0.1", port: 45121 },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
