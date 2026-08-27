import { defineConfig } from "vite";

// Vanilla TS on purpose. The product's pitch is weight, so the frontend ships no
// framework runtime. If screen state outgrows this, Svelte is the escape hatch —
// it compiles away and keeps the size claim intact.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", minify: "esbuild", sourcemap: false },
});
