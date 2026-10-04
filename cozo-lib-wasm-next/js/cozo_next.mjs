// Hand-written parallel bootstrap for cozo-lib-wasm-next.
//
// SOURCE, not output: lives in js/ (git-tracked). Rebuilds never touch it —
// scripts/build.ps1 only regenerates pkg/ (see capabilities.mjs header).
// serve.mjs serves the crate root, so the browser can import both:
//   import ... from '../pkg/cozo_lib_wasm_next.js';  // generated
//   import ... from '../js/cozo_next.mjs';           // this file
//
// Contract (matches tests/probe/probe.mjs + demo/index.html):
//   pool starts ONCE via mod.initThreadPool(n) BEFORE the first query,
//   otherwise rayon runs single-threaded or thread::scope paths trap.
// This module makes that contract explicit and fallible-safe:
//   multi  — pool started, queries may run on workers
//   single — pool unavailable (Node, no COOP/COEP, no SAB), same behaviour
//            as the frozen cozo-lib-wasm build.

import { detectCapabilities, resolveThreadCount } from "./capabilities.mjs";

/**
 * @typedef {Object} PoolResult
 * @property {'multi'|'single'} mode
 * @property {number} threads - effective count (1 in single mode).
 * @property {import('./capabilities.mjs').ThreadCapabilities} caps
 * @property {string} reason - 'ok' or fallback explanation.
 */

/**
 * Start the rayon worker pool once, falling back to single-threaded execution.
 * Never throws for environment reasons (missing Worker/SAB/isolation, pool
 * rejection): it returns { mode: 'single', reason }. Throws only on
 * programmer error (mod without initThreadPool).
 *
 * @param {{ initThreadPool: (n: number) => Promise<unknown> }} mod - bindgen namespace
 *   (browser: `import * as mod from '../pkg/cozo_lib_wasm_next.js'`,
 *    Node: `require(pkg + '/cozo_lib_wasm_next.js')` after init).
 * @param {{ threads?: number, env?: object }} [opts]
 * @returns {Promise<PoolResult>}
 */
export async function ensureThreadPool(mod, opts = {}) {
  if (!mod || typeof mod.initThreadPool !== "function") {
    throw new TypeError(
      "ensureThreadPool(mod): mod.initThreadPool missing — pass the bindgen namespace",
    );
  }
  const caps = detectCapabilities(opts.env ?? globalThis);
  const threads = resolveThreadCount(opts.threads ?? caps.cores, caps);

  if (!caps.canThread) {
    return {
      mode: "single",
      threads: 1,
      caps,
      reason: `fallback: ${caps.reasons.join("; ") || "threading unavailable"}`,
    };
  }
  try {
    await mod.initThreadPool(threads);
    return { mode: "multi", threads, caps, reason: "ok" };
  } catch (e) {
    const msg = String((e && e.message) || e).slice(0, 200);
    return {
      mode: "single",
      threads: 1,
      caps,
      reason: `fallback: initThreadPool(${threads}) failed: ${msg}`,
    };
  }
}

/**
 * Create a database instance from an initialised bindgen namespace.
 * Thin alias so call sites read as init -> ensurePool -> createDb.
 *
 * @param {{ CozoDb: { new(): unknown } }} mod - bindgen namespace after init().
 * @returns {unknown} CozoDb instance.
 */
export function createDb(mod) {
  if (!mod?.CozoDb || typeof mod.CozoDb.new !== "function") {
    throw new TypeError("createDb(mod): mod.CozoDb.new missing — init() the bindgen module first");
  }
  return mod.CozoDb.new();
}
