# `cozo-lib-wasm-next` — threaded WASM flavour of Cozo

Threaded browser build of the Cozo/Datalog database (`mnestic` core):
nightly + atomics + `SharedArrayBuffer` + `wasm-bindgen-rayon` worker pool.

Sibling crate [`cozo-lib-wasm`](../cozo-lib-wasm) is the **frozen
single-thread stable** build. This crate is the **experimental threads
line**. Rule (see `docs/plans/wasm-clean-restart.md` §5):

> **One crate — one toolchain — one `Cargo.lock`.**

The nightly configuration in this directory must never leak into the
workspace root or into `cozo-lib-wasm`.

## How it differs from `cozo-lib-wasm`

|                        | `cozo-lib-wasm` (frozen) | `cozo-lib-wasm-next` (this crate) |
|------------------------|--------------------------|-----------------------------------|
| Toolchain              | stable                   | nightly, pinned in `rust-toolchain.toml` |
| Threads                | no (rayon single-thread fallback) | yes, via `wasm-bindgen-rayon` |
| `build-std`            | no                       | yes (`panic_abort`, `std`), see `.cargo/config.toml` |
| Target features        | default                  | `+atomics,+bulk-memory,+mutable-globals,+simd128`, `--shared-memory`, `--import-memory` |
| JS API extra           | —                        | `init_thread_pool(concurrency)` — **must** be called once after `init()` and before the first query |
| Release `opt-level`    | `"z"`                    | `"s"` (compare on gzipped size before changing; see R2c in the plan doc) |
| Server headers         | not required             | **COOP/COEP required** (below) |
| `wasm-opt` via wasm-pack | disabled (`wasm-opt = false`) | disabled too (bundled wasm-opt 117 cannot parse bulk-memory / modern rustc output) |

Public API is otherwise identical: `CozoDb::new`, `run`, `export_relations`,
`import_relations` (`src/lib.rs` mirrors `cozo-lib-wasm/src/lib.rs` plus the
`init_thread_pool` re-export).

## Prerequisites

- Rust toolchains (both, side by side):
  - `stable` — workspace root, `cozo-lib-wasm`.
  - `nightly-2026-09-25` with `rust-src` and `wasm32-unknown-unknown`
    (already set up on this machine; `rust-toolchain.toml` pins it for this
    directory, so plain `cargo` inside `cozo-lib-wasm-next/` resolves to
    nightly automatically — verify with `rustc --version`).
- No `wasm-pack` (archived upstream; this crate builds manually).
  `wasm-bindgen-cli` and `wasm-opt` live crate-local in `.tools/bin/`
  (see `scripts/fetch-tools.ps1`): `wasm-bindgen 0.2.129` (newest, matches
  `Cargo.lock` — bindgen aborts on mismatch), `wasm-opt` from binaryen
  `version_133` (newest; ≥118 required).
- A static server that sends COOP/COEP headers for the demo page (required
  for `SharedArrayBuffer`):
  `Cross-Origin-Opener-Policy: same-origin` and
  `Cross-Origin-Embedder-Policy: require-corp`.

## Build

All commands run **inside this directory** (never from the workspace root —
nightly flags and `build-std` must not leak into the stable build).
Production pipeline is `scripts/build.ps1` (manual `cargo → wasm-bindgen →
wasm-opt`, no wasm-pack):

```powershell
cd cozo-lib-wasm-next
rustc --version                            # must print nightly
.\scripts\fetch-tools.ps1                  # one-time: .tools/bin bootstrap
.\scripts\build.ps1 -Configuration Debug   # fast validation
.\scripts\build.ps1 -Configuration Release # production pkg (sizes below)
```

Release `pkg/` (2026-09-27; session 7: vendored graph shims included).
Sizes below are pre-fork — remeasure after the first `mnestic/wasm.1` build:
raw 7.37 MB → bindgen 6.56 MB → `wasm-opt -O3` 6.22 MB.
Compressed `*_bg.wasm` (6,217,260 bytes): gzip-9 **1,935,223 (31.1%)**,
brotli (≈q11) **1,320,526 (21.2%)** — measured in-process (no gzip/brotli
CLI on this box).

### Compiler cache: kache, not sccache

`sccache 0.18.0` (latest) cannot build this crate: the `web-sys` rustc
command is 50,105 chars vs the Windows `CreateProcess` limit of 32,767
(41,422 of it a single `--check-cfg` feature list) → `os error 206`.
Upstream sccache parses `@argfile` (PR #2782) but still spawns rustc with
the expanded command; response-file-on-spawn exists only for gcc/msvc.
`kache v0.26.3` handles it (verified: full clean check green, `EXIT:0`) and
is now the machine default (`RUSTC_WRAPPER=kache` in the PowerShell profile;
`sccache.exe` kept as `sccache-0.18.0.bak`). `build.ps1` refuses to run
under sccache. Details + measurements: `WORKLOG.md` sessions 3–4.

## Use from JS

```js
import init, { initThreadPool, CozoDb } from './pkg/cozo_lib_wasm_next.js';

await init();
await initThreadPool(navigator.hardwareConcurrency);

const db = CozoDb.new();
db.run(script, params, immutable);
```

`initThreadPool` must run before the first query. Without COOP/COEP headers
the pool silently falls back to single-threaded execution (same behaviour as
`cozo-lib-wasm`); `std::thread::spawn`-style paths still do not exist on
`wasm32-unknown-unknown` even with atomics — only the rayon pool gains real
parallelism.

## Demo (browser, threaded)

```powershell
cd cozo-lib-wasm-next
node demo/serve.mjs 8080   # static server with COOP/COEP/CORP headers
```

Open `http://localhost:8080/demo/` in Chromium/Firefox: the page runs
`init()` → `initThreadPool(hardwareConcurrency)` → `CozoDb.new()` → one
query and renders `crossOriginIsolated` plus the result. `demo/serve.mjs`
is zero-dependency (`node:http`); correct MIME for `.wasm` included.

## Probe tests (Node, single-thread fallback)

```powershell
cd cozo-lib-wasm-next
.\tests\probe\run-all.ps1 -Threads 4
```

`tests/probe/probe.mjs` ports all 21 cases from
`cozo-lib-wasm/tests/probe/probe.mjs` with `initThreadPool` before the first
case. In Node the pool cannot start (`Worker is not defined`) so the run
verifies query parity in single-thread fallback; the real pool is covered by
the browser demo above. Baseline (session 7): **21/21 green** (the
`hnsw_query` step2 `ok:false` is a script artifact — that case never creates
the index — identical in the frozen build). `run-all.ps1` keeps the old
known-traps list as a regression tripwire and exits 0.

## Deliberate non-goals / limits

- `panic = "abort"` is **not** set: `cozo-core` uses `catch_unwind` in
  `runtime/graph_projection.rs:2383` (commit-fence test). Rewrite that test
  first (plan doc, R2c).
- `Instant`/`SystemTime` still trap on wasm32 with any toolchain flags —
  every reachable site is gated in the fork instead: no-op `Timer` probes in
  the `graph`/`graph_builder` fork, `js_sys::Date` in `cozo-core`, and no
  time budget on wasm (`budget_now() == None`).
- `page_rank` used `std::thread::scope` — fixed in the fork (below).
- `graph 0.3.2` / `graph_builder 0.4.2` (MIT, upstream dormant) come from the
  mnestic fork (`vkozio/graph@mnestic/wasm.1`) via `[patch.crates-io]` in
  this crate only — stable never sees it. Fork deltas vs upstream: no-op
  log timers on wasm32 (no `web-time`), sequential fallback for the three
  runtime `scope` sites, public `Csr::empty()`. Native code paths are
  bit-identical to upstream.
- `tests/probe/` ports the 21 probes from
  `cozo-lib-wasm/tests/probe/probe.mjs` (`initThreadPool` before the first
  case). Node covers query parity (pool falls back there); the real pool is
  covered by the browser demo. All 21 green since session 7.

## Layout

```text
cozo-lib-wasm-next/
  Cargo.toml            # single merged wasm32 target-deps table; own lockfile
  rust-toolchain.toml   # pins nightly-2026-09-25 + rust-src + wasm32 target
  .cargo/config.toml    # atomics/SIMD rustflags + shared-memory link args + build-std
  src/lib.rs            # CozoDb binding + init_thread_pool re-export
  src/utils.rs          # set_panic_hook
  scripts/build.ps1     # manual pipeline (cargo -> bindgen -> wasm-opt)
  scripts/fetch-tools.ps1 # bootstrap pinned .tools/bin from Cargo.lock
  .tools/bin/           # git-ignored: wasm-bindgen 0.2.129, wasm-opt 133
  demo/                 # COOP/COEP static server (serve.mjs) + threaded page
  tests/probe/          # 21 ported probes (probe.mjs) + run-all.ps1
  README.md / WORKLOG.md / HANDOFF.md
```
