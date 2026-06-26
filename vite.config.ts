import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    // 1420 es el puerto que espera el webview de Tauri (`npm run dev:safe`).
    // Si el harness de preview asigna un puerto vía PORT, lo respetamos para
    // poder levantar una instancia de verificación sin chocar con la app real.
    port: process.env.PORT ? Number(process.env.PORT) : 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));
