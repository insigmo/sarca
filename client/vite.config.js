import { defineConfig } from "vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Connect shell for Tauri: server URL only, then Rust navigates to the
// server's web UI (same SolidJS app as the browser; sign-in is on the site).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, "index.html"),
        sync: path.resolve(__dirname, "sync.html"),
      },
    },
  },
  resolve: {
    alias: {
      "@sarca-ui": path.resolve(__dirname, "../ui/src"),
    },
  },
});
