// The UI is built into dist/, which the engine embeds (src/api/mod.rs). `npm run dev` serves it
// with live reload and passes /api to a running engine: WFP_PORT, else its default port. The
// engine only answers its own origin, so the proxy presents that one (Vite listens on localhost
// only).
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const engine = `http://127.0.0.1:${process.env.WFP_PORT ?? 8642}`;

export default defineConfig({
  plugins: [svelte()],
  base: "./",
  build: { outDir: "dist", emptyOutDir: true, target: "es2022", chunkSizeWarningLimit: 800 },
  server: { host: "127.0.0.1", proxy: { "/api": { target: engine, changeOrigin: true, headers: { origin: engine } } } },
});
