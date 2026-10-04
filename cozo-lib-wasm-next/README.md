# `cozo-lib-wasm-next` — threaded WASM build of Cozo

Threaded browser and Node build of the Cozo/Datalog database (`mnestic` core).
It uses nightly Rust, atomics, `SharedArrayBuffer`, and a `wasm-bindgen-rayon` worker pool.

Use this crate for parallel query execution in the browser.
Use `cozo-lib-wasm` for the stable single-thread build.

## Requirements

The worker pool starts only where all of these hold:

| Capability | Check | Source |
| --- | --- | --- |
| `SharedArrayBuffer` allocates | `new SharedArrayBuffer(8)` succeeds | Browser with isolation |
| Isolation active | `crossOriginIsolated === true` | COOP/COEP headers below |
| Worker support | `typeof Worker === "function"` | Browser; plain Node reports `false` |
| Shared WASM memory | `new WebAssembly.Memory({ shared: true })` succeeds | Modern Chromium/Firefox |
| CPU count | `navigator.hardwareConcurrency` | Falls back to `1` |

Call `detectCapabilities()` to read this table at runtime. It returns
`{ hasSAB, isolated, hasWorker, sharedMemory, cores, canThread, reasons }`.
`canThread === true` means `initThreadPool` proceeds.
`canThread === false` means the wrapper keeps single-thread execution with the same query API.

## Installation

Build artifacts from source. `pkg/` stays a local build directory and stays out of git.
Published npm packaging arrives later.

```powershell
cd cozo-lib-wasm-next
rustc --version                  # reports nightly-2026-09-25 inside this directory
.\scripts\fetch-tools.ps1        # one-time bootstrap of .tools/bin
.\scripts\build.ps1 -Configuration Release
```

Build output lands in `pkg/`:
`cozo_lib_wasm_next.js`, `cozo_lib_wasm_next.d.ts`,
`cozo_lib_wasm_next_bg.wasm`, `snippets/` (rayon worker helpers).

## Quickstart

### Browser (ESM)

```js
import init, * as bindgen from "./pkg/cozo_lib_wasm_next.js";
import { ensureThreadPool, createDb } from "./js/cozo_next.mjs";

await init();
const pool = await ensureThreadPool(bindgen);
// pool = { mode: "multi", threads: 8, reason: "ok" }
// pool = { mode: "single", threads: 1, reason: "fallback: ..." }

const db = createDb(bindgen);
const res = JSON.parse(db.run("?[a] := a = 1", "{}", false));
console.log(pool.mode, res.rows);
```

### Node (single-thread execution)

```js
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const mod = require("./pkg/cozo_lib_wasm_next.js");
await mod.default({
  module_or_path: new Uint8Array(readFileSync("./pkg/cozo_lib_wasm_next_bg.wasm")),
});

const { ensureThreadPool, createDb } = await import("./js/cozo_next.mjs");
const pool = await ensureThreadPool(mod); // { mode: "single", threads: 1, ... }
const db = createDb(mod);
console.log(JSON.parse(db.run("?[a] := a = 1", "{}", false)));
```

The call order stays fixed: `init()` first, `ensureThreadPool()` second, first query last.

## API reference

### `js/capabilities.mjs`

```js
import { detectCapabilities, resolveThreadCount } from "./js/capabilities.mjs";
```

`detectCapabilities(env = globalThis)` returns `ThreadCapabilities`.
It probes the current realm and guards every probe, so it returns a result in all runtimes.
Pass a fake `env` in tests to describe browser states from Node.

`resolveThreadCount(want, caps)` clamps the request to `[1, caps.cores]`.
It maps `undefined` to `caps.cores` and maps `0`, negative, and non-numeric input to `1`.
The clamp matches `workerHelpers.js::startWorkers`, which requires thread count above zero.

### `js/cozo_next.mjs`

```js
import { ensureThreadPool, createDb } from "./js/cozo_next.mjs";
```

`ensureThreadPool(mod, { threads, env } = {})` starts the rayon pool once.
It reads capabilities, resolves the thread count, calls `mod.initThreadPool(threads)`,
and returns `{ mode, threads, caps, reason }`.
It returns `{ mode: "single", threads: 1 }` with a `fallback:` reason where threading
stays unavailable or where the pool call fails.
It throws `TypeError` only where `mod.initThreadPool` is missing.

`createDb(mod)` returns `mod.CozoDb.new()`.
It throws `TypeError` only where `mod.CozoDb.new` is missing, which means `init()` ran out of order.

### Generated binding (`pkg/`)

```js
CozoDb.new() -> CozoDb
db.run(script: string, params: string, immutable: boolean) -> string // JSON payload
db.export_relations(data: string) -> string
db.import_relations(data: string) -> string
initThreadPool(num_threads: number) -> Promise<void>
```

`run` accepts a CozoScript string plus a JSON params string and returns a JSON string.
Parse the return value and read `ok`, `rows`, `message`.

## Threading model

`init()` instantiates the module and its shared memory.
`ensureThreadPool()` spawns `N` workers through
`snippets/.../workerHelpers.js::startWorkers` and shares the module plus memory with each worker.
Each worker reports `wasm_bindgen_worker_ready` through `waitForMsgType`, then the pool builder finalizes.

`mode: "multi"` means queries run with the rayon pool active.
`mode: "single"` means queries run with the same API and single-thread execution.
Thread count resolves to `min(requested, hardwareConcurrency)` with a floor of `1`.

## Serving

Browsers activate `SharedArrayBuffer` only with isolation headers plus the WASM MIME type.
`demo/serve.mjs` sends exactly this set with zero dependencies:

```powershell
cd cozo-lib-wasm-next
node demo/serve.mjs 8080
# open http://localhost:8080/demo/
```

Headers:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
Content-Type: application/wasm   # for .wasm
Content-Type: text/javascript    # for .js/.mjs
```

`demo/index.html` shows the full startup sequence with capability logging.
Bundlers resolve the same files: keep `pkg/` next to `js/` and preserve the
`pkg/snippets/` relative path so workers fetch `workerHelpers.js` from the deployed tree.

## Building from source

Run all build commands inside `cozo-lib-wasm-next/`.
`rust-toolchain.toml` pins `nightly-2026-09-25` with `rust-src` plus `wasm32-unknown-unknown`
for this directory, so plain `cargo` resolves to nightly there.
Run host builds from the workspace root with stable Rust.

```powershell
cd cozo-lib-wasm-next
.\scripts\fetch-tools.ps1                  # wasm-bindgen 0.2.129 + wasm-opt 133 into .tools/bin
.\scripts\build.ps1 -Configuration Debug   # fast validation
.\scripts\build.ps1 -Configuration Release # production pkg
```

`scripts/build.ps1` runs `cargo build --target wasm32-unknown-unknown`,
then `wasm-bindgen --target web --out-dir pkg`,
then the rayon worker import patch,
then `wasm-opt` with threads, bulk-memory, SIMD, mutable-globals, and saturating-float-convert flags.
Set `RUSTC_WRAPPER=kache` for this crate.

## Testing

Browser demo (real pool):

```powershell
cd cozo-lib-wasm-next
node demo/serve.mjs 8080
```

Node probes (query parity with single-thread execution):

```powershell
cd cozo-lib-wasm-next
.\tests\probe\run-all.ps1 -Threads 4
```

`tests/probe/probe.mjs` runs the 21 ported cases, one case per child process with a fresh `CozoDb`.
Baseline: 21/21 green. `hnsw_query` step 2 reports `ok: false` in the scripted sequence
because that case queries the index before creating it; the frozen build reports the same shape.

## Troubleshooting

| Observation | Meaning | Action |
| --- | --- | --- |
| `canThread: false`, reason lists isolation | Page serves without COOP/COEP | Serve through `demo/serve.mjs` or send the headers above |
| `mode: "single"` in Node | `Worker` stays unavailable in plain Node | Expect parity coverage there; verify the pool in the browser demo |
| `Failed to fetch dynamically imported module` in workers | Worker helper points at a directory import | Rebuild with `scripts/build.ps1`, which rewrites the helper to `../../../cozo_lib_wasm_next.js` |
| `wasm-bindgen CLI mismatch` | Tool version drifted from `Cargo.lock` | Run `scripts/fetch-tools.ps1` again |
| `os error 206` on `web-sys` | `sccache` exceeds Windows command length | Use `RUSTC_WRAPPER=kache` |

## Limitations

* `panic = "abort"` stays off: `cozo-core` relies on `catch_unwind` in the commit-fence path.
* WASM time stays untimed: graph fork timers report zero durations on `wasm32`,
  `cozo-core` reads the date clock, and query budgets stay inactive (`budget_now() == None`).
* `std::thread::spawn` style threading stays out of scope on `wasm32-unknown-unknown`.
  Parallelism flows through the rayon pool.
* Release profile uses `opt-level = "z"`, full LTO, single codegen unit, and symbol stripping.
  Compare size changes on compressed `*_bg.wasm` (gzip/brotli).

## Layout

```text
cozo-lib-wasm-next/
  js/capabilities.mjs       # hand-written: capability detector (tracked)
  js/cozo_next.mjs          # hand-written: pool bootstrap + Db factory (tracked)
  src/lib.rs                # CozoDb binding + init_thread_pool re-export
  src/utils.rs              # panic hook setup
  scripts/build.ps1         # cargo -> wasm-bindgen -> worker patch -> wasm-opt
  scripts/fetch-tools.ps1   # pinned .tools/bin bootstrap from Cargo.lock
  rust-toolchain.toml       # pinned nightly for this directory
  .cargo/config.toml        # atomics/SIMD flags + shared-memory link args + build-std
  demo/serve.mjs            # zero-dependency COOP/COEP static server
  demo/index.html           # threaded startup through js/ wrappers
  tests/probe/              # 21 ported cases + run-all.ps1
  pkg/                      # generated output (local only, out of git)
  .tools/bin/               # pinned wasm-bindgen + wasm-opt (local only, out of git)
  target/                   # Rust output (local only, out of git)
```

Build writes to `pkg/` only. Sources live in `js/`, `src/`, `demo/`, `scripts/`, and `tests/`.
Keep that split: rebuilds regenerate `pkg/`, hand-written files remain stable across rebuilds.

## License

Mozilla Public License 2.0. See the workspace license.
