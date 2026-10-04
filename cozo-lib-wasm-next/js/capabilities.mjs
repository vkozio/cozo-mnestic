// Hand-written capability detector for cozo-lib-wasm-next (threaded flavour).
//
// SOURCE OF TRUTH, not build output:
//   - lives in js/ (tracked by git)
//   - scripts/build.ps1 writes ONLY to pkg/ (git-ignored scratch:
//     cargo -> wasm-bindgen --out-dir pkg -> wasm-opt + workerHelpers patch)
//   - wasm-bindgen overwrites pkg/*.js, pkg/*.d.ts, pkg/*_bg.wasm,
//     pkg/snippets/** — it never touches js/.
//   - demo/serve.mjs serves the whole crate dir, so both /pkg/ and /js/
//     are reachable from the browser demo.
//
// No wasm imports here: pure environment probing, works in browser + Node.

/**
 * @typedef {Object} ThreadCapabilities
 * @property {boolean} hasSAB - SharedArrayBuffer constructor exists and allocates.
 * @property {boolean} isolated - crossOriginIsolated === true (needs COOP/COEP from demo/serve.mjs).
 * @property {boolean} hasWorker - global Worker constructor exists (false in plain Node).
 * @property {boolean} sharedMemory - new WebAssembly.Memory({ shared: true }) succeeds.
 * @property {number} cores - hardwareConcurrency or 1.
 * @property {boolean} canThread - all of the above; only then initThreadPool can succeed.
 * @property {string[]} reasons - human-readable blockers when canThread is false.
 */

/**
 * Probe the current JS realm for rayon thread-pool prerequisites.
 * Never throws: every individual probe is guarded.
 *
 * @param {object} [env=globalThis] - injectable root (tests pass fakes).
 * @returns {ThreadCapabilities}
 */
export function detectCapabilities(env = globalThis) {
  const reasons = [];

  let hasSAB = false;
  try {
    const SAB = env?.SharedArrayBuffer;
    if (typeof SAB === "function") {
      // Allocation (not just typeof) fails without isolation in some browsers.
      new SAB(8);
      hasSAB = true;
    } else {
      reasons.push("SharedArrayBuffer missing");
    }
  } catch {
    reasons.push("SharedArrayBuffer unusable (needs COOP/COEP isolation)");
  }

  const isolated = env?.crossOriginIsolated === true;
  if (!isolated)
    reasons.push("not crossOriginIsolated (serve with COOP:same-origin + COEP:require-corp)");

  const hasWorker = typeof env?.Worker === "function";
  if (!hasWorker) reasons.push("Worker missing (plain Node falls back to single-thread)");

  let sharedMemory = false;
  try {
    if (typeof WebAssembly?.Memory === "function") {
      // Throws TypeError where shared memory is unsupported.
      new WebAssembly.Memory({ initial: 1, maximum: 2, shared: true });
      sharedMemory = true;
    } else {
      reasons.push("WebAssembly.Memory missing");
    }
  } catch {
    reasons.push("shared WebAssembly.Memory unsupported");
  }

  const rawCores = env?.navigator?.hardwareConcurrency;
  const cores = Number.isInteger(rawCores) && rawCores > 0 ? rawCores : 1;

  const canThread = hasSAB && isolated && hasWorker && sharedMemory;

  return { hasSAB, isolated, hasWorker, sharedMemory, cores, canThread, reasons };
}

/**
 * Clamp a requested thread count to [1, cores].
 * Mirrors workerHelpers.js::startWorkers which throws on numThreads() === 0,
 * so 0/negative/NaN never reach the wasm pool.
 *
 * @param {unknown} want - requested threads (navigator.hardwareConcurrency by default).
 * @param {ThreadCapabilities} caps - from detectCapabilities().
 * @returns {number} safe count >= 1.
 */
export function resolveThreadCount(want, caps) {
  const cores = caps?.cores && caps.cores > 0 ? caps.cores : 1;
  const n = want === undefined ? cores : Number(want);
  if (!Number.isFinite(n) || n < 1) return 1;
  return Math.min(Math.floor(n), cores);
}
