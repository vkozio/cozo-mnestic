import { defineConfig } from "vite";

export default defineConfig({
  // The published package is ESM glue that locates its own binary with
  // `new URL('cozo_lib_wasm_bg.wasm', import.meta.url)`. Vite's dependency
  // pre-bundling rewrites module URLs, which breaks that lookup, so the package
  // is excluded from the optimizer and served as-is.
  optimizeDeps: {
    exclude: ["cozo-lib-wasm"],
  },
  build: {
    // async/await + top-level await in the glue
    target: "es2022",
  },
  server: {
    host: "127.0.0.1",
  },
});
