import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Monaco, pdf.js e gli altri chunk caricati su richiesta superano da soli i 500 kB (Vite conta a 1000 byte): il
  // limite a 3000 serve a tacere quegli avvisi. Il chunk principale è a circa 507 kB, quindi l'avviso è spento anche
  // per lui: controllare a mano che non contenga Monaco, pdf.js, xterm né markdown-it (vedi i chunk in dist/assets).
  build: { chunkSizeWarningLimit: 3000 },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
