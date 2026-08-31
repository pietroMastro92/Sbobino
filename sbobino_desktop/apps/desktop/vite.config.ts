import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Ship source maps with the release bundle so React's minified
    // production errors (e.g. "Minified React error #185") map back to
    // real component names and line numbers in DevTools instead of
    // anonymous `Nb` / `Tb` frames.
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    globals: false,
    css: true,
    // The Tauri/jsdom suites share a process-heavy mock environment. Running
    // every file in parallel can starve a synchronous render test until
    // Vitest's 5s per-test timeout expires, producing a false release-gate
    // failure. Keep the gate deterministic; individual tests remain parallel
    // inside their own file where that is safe.
    fileParallelism: false,
  }
});
