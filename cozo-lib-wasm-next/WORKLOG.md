# WORKLOG — `cozo-lib-wasm-next`

Append-only journal. Session 1 reconstructed from `HANDOFF.md` §6; session 2
onwards written by the acting agent.

## 2026-09-27 — session 1 (before provider drop)

- Studied fork history, `git log` on `cozo-lib-wasm`.
- Confirmed: `std::thread` / `std::time` on `wasm32-unknown-unknown` stay
  stubs even with atomics (`library/std/src/sys/thread/mod.rs:98-104` takes
  only `sleep` from the wasm branch; `Thread` / `available_parallelism` come
  from `unsupported` in both branches; `Instant` / `SystemTime` trap).
- Created `docs/plans/wasm-clean-restart.md` (SPEC, waves R0–R3).
- Updated §5: threads line moves to `cozo-lib-wasm-next`, old
  `cozo-lib-wasm` frozen (single-thread, stable).
- Scaffolded `cozo-lib-wasm-next/`: `Cargo.toml`, `rust-toolchain.toml`,
  `.cargo/config.toml`, `src/lib.rs`, `src/utils.rs`. (`tests/probe/` was
  planned but not created — fixed in session 2 with a `.gitkeep`
  placeholder.)
- Added `wasm32-unknown-unknown` target on nightly.
- Checked `wasm-bindgen-rayon 1.3.0` requirements: `wasm-bindgen ^0.2.99`
  (compatible with locked `0.2.128`), `rayon ^1.8.1` with `web_spin_lock`,
  `js-sys ^0.3.70`, `crossbeam-channel ^0.5.9`.
- **Left undone:** duplicate-TOML-table fix, root `exclude`, `Cargo.lock`,
  README, WORKLOG, probe tests, demo, CI, build check.

## 2026-09-27 — session 2 (handoff continuation)

Environment (verified, not assumed):

- `rustup toolchain list`: `stable`, `nightly`, `nightly-2026-09-25` —
  the pinned `nightly-2026-09-25` **already exists locally** (handoff
  option A, no install needed).
- `nightly-2026-09-25` has `rust-src` and `wasm32-unknown-unknown`
  pre-installed. Inside `cozo-lib-wasm-next/`, `cargo`/`rustc` resolve to
  `1.100.0-nightly` via `rust-toolchain.toml` (verified:
  `cargo 1.100.0-nightly`, `rustc 1.100.0-nightly`).
- `wasm-pack` present; system `wasm-bindgen` CLI is `0.2.114` vs locked
  crate `0.2.129` — **mismatch noted, CLI upgrade (or wasm-pack-managed
  backend) still TODO** before `wasm-pack build`.

Fixes applied:

1. `cozo-lib-wasm-next/Cargo.toml` — merged the two duplicate
   `[target.'cfg(target_arch = "wasm32")'.dependencies]` tables into one
   (`getrandom_03`, `nanorand`, `wasm-bindgen-rayon`). Verified the
   `getrandom`/`nanorand` features are 1:1 with `cozo-lib-wasm/Cargo.toml`.
2. Added `[package.metadata.wasm-pack.profile.release] wasm-opt = false`
   (same reason as the old crate: bundled wasm-opt 117 cannot parse
   bulk-memory / modern rustc output).
3. Root `Cargo.toml` — added `cozo-lib-wasm-next` to workspace `exclude`.
4. Created `tests/probe/.gitkeep` placeholder (directory did not exist).
5. Wrote `README.md` (this crate's doc) and this `WORKLOG.md`.
6. `docs/plans/wasm-clean-restart.md` §3.3 R2b rewritten: `pkg-threads/`
   flavour → `cozo-lib-wasm-next` crate, matching §5. (R2a tool-state notes
   updated to observed values.)
7. `HANDOFF.md` session-2 section filled in.

Build verification:

- `cargo check --target wasm32-unknown-unknown` → **green (exit 0)**,
  `Cargo.lock` generated (219 packages; `wasm-bindgen 0.2.129`,
  `wasm-bindgen-rayon 1.3.0`, `rayon-core 1.13.0`, `rayon 1.10.0`).
  - First attempt **failed** on `web-sys`: global `RUSTC_WRAPPER=sccache`
    hits Windows `os error 206` (command line too long from the `--cfg
    feature="..."` list). Retried with `$env:RUSTC_WRAPPER=""` → green
    in ~15 s. Documented in `README.md`; needs a permanent per-crate/CI
    exemption.
  - Remaining warnings are pre-existing, not ours: ~286 `bail!(err),`
    trailing-semicolon future-compat lints in `cozo-core/src/lib.rs`,
    one `page_size` shim warning, one informational
    `unstable feature specified for -Ctarget-feature: atomics`.
- Stable invariant: root `cargo check` — TODO in this session (run before
  closing; must stay green).
- `wasm-pack build --target web --release` — **not run** (blocked on the
  `wasm-bindgen` CLI `0.2.114` vs `0.2.129` mismatch + needs COOP/COEP demo
  harness). Next session's first build step.

## 2026-09-27 — session 3 (sccache proper fix, option 3)

Diagnosis (measured, not assumed):

- Reproduced `os error 206` on `web-sys` with default env (sccache active,
  no overrides): both `cargo check` and `cargo build` fail identically.
- Measured the sccache→rustc command from the error output: **50,105 chars**
  vs Windows `CreateProcess` limit **32,767** (over by **17,338**).
- Of that, **41,422 chars is a single `--check-cfg "cfg(feature,
  values(...))"` argument**: cargo enumerates ALL declared features of
  `web-sys 0.3.106` (~1500, enabled or not). Only 2 `--cfg feature=` flags
  are actually enabled (`EventTarget`, `Window`, via `wasm_sync 0.1.2` ←
  `wasm-bindgen-rayon 1.3.0`).
- sccache `0.18.0` installed == upstream latest (v0.18.0, 2026-09-14).
  Upstream PR #2782 ("arg files in Rust", merged 2026-07-28, released in
  0.17.0) only taught sccache to *parse* `@argfile` for cache keys — it
  still spawns `rustc` with the fully expanded 50k command. Response-file
  support in sccache exists only for gcc/msvc (docs/ResponseFiles.md).
- Shorter-paths lever quantified and rejected: path content in the command
  is ~15 KB; even halving every path saves ~7 KB < 17.3 KB needed.
- Notable: the `zccache` fork on crates.io implements exactly the missing
  half (`write_response_file_if_needed`, 30k threshold, rustc line format,
  regression test using the web-sys `--check-cfg` value). Third-party fork
  — trust decision, not taken.
- Blast radius: only this crate (only tree containing `web-sys`+wasm).
  Everything else in the workspace fits and keeps using sccache normally.

Conclusion: not fixable by sccache config/version — missing upstream
feature (spill to response file when spawning rustc on Windows). Decision
(escalated to user): upstream issue report vs fork vs source patch vs
scoped no-wrapper for this crate's wasm builds.

## 2026-09-27 — session 4 (wrapper trials; user approved process-local override, option B)

Mechanics proven first (no more guessing):

- `RUSTC_WRAPPER=sccache` comes from the PowerShell user profile script
  (`Documents\PowerShell\Microsoft.PowerShell_profile.ps1:6`, + `SCCACHE_DIR`
  and 50G cache). Registry User/Machine are empty; profile file untouched.
- `cargo --config build.rustc-wrapper=...` provably does NOT override env
  (bogus-wrapper probe still failed inside sccache; `cargo -Zunstable-options
  config get` doesn't even see the env value — cargo reads it directly at
  spawn). Per-invocation wrapper selection is only possible via process env.
- cargo freshness itself is NORMAL (earlier confusion was `\r`-glued status
  lines + over-aggressive output filters; `-v` shows proper Fresh/Dirty).

Trial 1 — kache v0.26.3 (`cargo install kache --locked`, stable 1.98.1,
8.5 min, `~/.cargo/bin/kache.exe`):

- Full clean `cargo check --target wasm32-unknown-unknown` through kache:
  **green, EXIT 0, no 206** — `Checking web-sys v0.3.106` compiled fine.
  Consistent model: cargo passes `@file` to the wrapper; sccache expands it
  (#2782) and spawns raw 50k → 206; kache handles it → green.
- Daemon auto-started, defaults (store under `AppData\Local\kache`,
  `~/.config/kache/config.toml`, no remote). First run: 11 misses / 0 hits.
- Trial 2 (zccache) skipped: kache passed, and kache wins on trust anyway
  (935 stars, Apache-2.0, company-backed vs 19 stars, AGPL-3.0 solo fork).

Pending user decision: flip profile line to kache + back up/remove
`sccache.exe` (user's hands or explicit approval to edit the profile .ps1).

## 2026-09-27 — session 5 (switch to kache + manual pipeline green)

User approved (`B`, then full go-ahead). Done:

1. Profile: `Documents\PowerShell\Microsoft.PowerShell_profile.ps1`
   `RUSTC_WRAPPER` `sccache` → `kache` (rollback note in comments);
   `SCCACHE_*` lines commented out. `sccache.exe` → `sccache-0.18.0.bak`,
   `sccache --stop-server`. NOTE: already-running processes (incl. this
   agent session) keep inheriting `sccache` from the session-start env —
   only new shells pick up kache. Verified root `cargo check` green under
   kache too (9.3 s) — daily builds safe.
2. `.tools/bin/`: `wasm-bindgen 0.2.129` + `wasm-opt 133`, verified
   `--version`. `scripts/fetch-tools.ps1` bootstraps these pins
   (bindgen version auto-read from `Cargo.lock`); `scripts/build.ps1`
   runs the guide-§8 pipeline (Debug/Release, bindgen, wasm-opt,
   twiggy report) with preconditions (nightly, tool presence + version
   match, sccache refusal). wasm-pack metadata removed from `Cargo.toml`.
3. Guide corrections found by linking (check doesn't link!):
   - `--export=__wasm_init_memory` does NOT exist in
     nightly-2026-09-25 std (`rust-lld: symbol exported via --export not
     found`) — removed, documented in `.cargo/config.toml`.
   - `wasm-opt` needs `--enable-nontrapping-float-to-int` (rustc emits
     `i64.trunc_sat_f64_s`; without it validator fails). Dev build passed
     without it; release needs it.
4. Release `pkg/` built 2026-09-27 (kache wrapper):
   raw 7,369,652 → bindgen 6,560,236 → `wasm-opt -O3` **6,214,264 bytes**.
   `initThreadPool` export present in JS glue. Pre-gzip (no brotli/gzip
   CLIs on this box).
5. `cozo-lib-wasm-next/.gitignore`: `.tools/`, `target/`, `pkg/`, `dist/`.

## 2026-09-27 — session 6 (NEXT_SESSION steps 1–6)

1. **Re-verify (step 1).** Release rebuild under kache → byte-identical to
   session 5: raw 7,369,652 → bindgen 6,560,236 → `wasm-opt -O3`
   **6,214,264 bytes** (`twiggy` top: `data[0]` 34.11%). Note: this agent
   shell still inherited stale `RUSTC_WRAPPER=sccache` (session predates the
   profile flip); the build ran with a process-local `$env:RUSTC_WRAPPER="kache"`
   override. New shells get kache from the profile. `rustc --version` prints
   `1.100.0-nightly (… 2026-09-24)` — that is the compiler build date inside
   the pinned    `nightly-2026-09-25` toolchain (active via `rust-toolchain.toml`),
   not a toolchain mismatch.
   Stable invariant re-checked: root `cargo check --workspace` green
   (exit 0) under kache. Note: a plain root check in a pre-flip shell fails
   with `program not found` because that shell still exports
   `RUSTC_WRAPPER=sccache` while `sccache.exe` is now `sccache-0.18.0.bak`
   (session 5, by design) — stale-shell artifact, not a repo problem.
2. **Demo harness (step 2).** `demo/serve.mjs` (zero-dep `node:http`, serves
   the crate dir with `COOP: same-origin` + `COEP: require-corp` +
   `CORP: same-origin` and correct MIME incl. `application/wasm`) and
   `demo/index.html` (`init()` → `initThreadPool(hardwareConcurrency)` →
   `CozoDb.new()` → `?[a] := a = 1`, renders `crossOriginIsolated` + result).
   Verified with curl: `200` on `/demo/` and on `pkg/*_bg.wasm` with all
   three headers present. In-browser execution still pending (no browser in
   this environment) — open `http://localhost:8080/demo/` after
   `node demo/serve.mjs`.
3. **Probes (step 3).** `tests/probe/probe.mjs` ports all 21 CASES verbatim
   from `cozo-lib-wasm/tests/probe/probe.mjs`, plus `initThreadPool(threads)`
   before the first case (pool status recorded as first output row;
   `tests/probe/run-all.ps1` runs one case per child process, exit 0 unless
   an *unexpected* trap appears). Node findings:
   - `--target web` glue imports rayon `workerHelpers.js` at module top,
     which touches browser-only `self` → probe stubs
     `{addEventListener, removeEventListener, postMessage}` for the import.
   - The stub must forward `globalThis.crypto` as `self.crypto`, otherwise
     getrandom 0.2 fails with "Web Crypto API is unavailable" (initially
     masked `lsh_rows`/`random_walk`/`hnsw_incremental`; after the fix all
     three are green). Dependency versions are 1:1 with the frozen crate
     (rand 0.8.8, getrandom 0.2.17/0.3.4) — the delta was the shim, not the
     build.
   - Node model: `initThreadPool` falls back (`Worker is not defined`) →
     single-threaded query parity; the real pool is browser-only (demo).
   - Parity vs frozen `cozo-lib-wasm/pkg` (same Node): 16/21 fully green;
     `hnsw_query` step2 `ok:false` (`eval::index_not_found` — the case never
     creates the index, script artifact, identical in old); 5 clock traps
     (`query_create`, `hnsw_rows`, `pagerank`, `top_sort`, `graph_query`,
     all `time not implemented`) identical in old — except `query_create`,
     where old traps as `unreachable` (stable std) vs `time not
     implemented` (nightly std): outcome parity (fail/fail), R1 wave owns
     both. **Zero regressions from the threaded build.**
   - `run-all.ps1` encodes the 5 as `$knownTraps` baseline: green exit 0.
   - Placeholder `tests/probe/.gitkeep` removed.
4. **Measurements (step 5).** `pkg/cozo_lib_wasm_next_bg.wasm` (6,214,264):
   gzip-9 (CPython `gzip.compress`) **1,934,817 bytes (31.1%)**; brotli
   (.NET `BrotliStream`, SmallestSize ≈ q11) **1,319,693 (21.2%)**. No
   gzip/brotli CLI on this box — compressed in-process, not on a server.
   `opt-level s/z` comparison (R2c) still open.
5. **CI (step 4).** `.github/workflows/wasm-next.yml` (new file, outside the
   crate): `windows-latest` (scripts are the Windows-edition pipeline),
   pinned `nightly-2026-09-25` + `rust-src` + wasm target,
   `fetch-tools.ps1` → `build.ps1 -Configuration Release`, plus an
   `initThreadPool`-in-glue grep gate and `pkg/` artifact upload.
   Path-filtered to `cozo-lib-wasm-next/**`, never a required check —
   stable stays unblocked. Probes deliberately NOT in CI (5 known traps
   until the R1 clock shim).

## 2026-09-27 — session 7 (graph vendoring: R1 clock wave)

User approved vendoring the dormant graph stack. Result: **21/21 probes
green** (only the pre-existing `hnsw_query` step2 script artifact remains —
`ok:false`, identical in the frozen build).

1. **Pinned.** `graph 0.3.1` + `graph_builder 0.4.1` (both MIT,
   neo4j-labs/graph, dormant). Upstream findings: all `Instant::now` uses
   are log-only timings; real wasm blockers are `std::thread::scope` in
   `page_rank.rs::page_rank_iteration`, `triangle_count.rs::
   global_triangle_count`, `graph_builder::edgelist.rs::TryFrom<&[u8]>`
   (`dss.rs`/`adj_list.rs` scopes are `#[cfg(test)]`-only — untouched).
   `graph_builder` build.rs only probes nightly features — kept as-is.
   tests/benches/examples kept in-tree (smaller upstream diff; cargo
   ignores dev-deps of non-member path deps — check passed).
2. **Vendored.** Registry copies →
   `cozo-lib-wasm-next/vendor/{graph,graph_builder}` (registry junk
   `.cargo-ok`/`.cargo_vcs_info.json`/`Cargo.lock` removed, LICENSE kept),
   wired via `[patch.crates-io]` in the crate's `Cargo.toml` next to the
   `page_size` precedent. Versions unchanged → lock churn minimal. The patch
   is invisible to stable (crate excluded from workspace, separate lock).
3. **Vendor patches (all `cfg(target_arch = "wasm32")`, native bit-identical).**
   - `web-time 1.1` target-dep in both vendored manifests; every
     `use std::time::Instant` swapped for the cfg pair (`page_rank`,
     `triangle_count`, `sssp`, `wcc`, `graph_ops`, `csr`, `adj_list`,
     `graph500`, `edgelist`).
   - The three runtime `scope` sites run the same chunk-stealing loop via a
     `worker` closure, spawned N× natively, called 1× on wasm.
4. **cozo-core (shared crate, wasm-only cfg — stable unaffected).**
   - `runtime/stored_queries.rs::create`: unguarded `SystemTime::now()` for
     `created_at` → `js_sys::Date::now()/1000.` on wasm (this was the
     `query_create` trap).
   - `runtime/hnsw.rs::hnsw_build_index`: unconditional `Instant::now()` for
     the `MNESTIC_BUILD_PROFILE` readout → clock + print cfg'd out on wasm
     (this was the `hnsw_rows` trap).
   - Audited the rest: `budget_now`→None, `effective_outer_deadline`→`?`
     short-circuit, `Poison::check` deadline-gated, `run_script_fold_err`
     `took` gated, `op_now`/`current_validity`/`op_rand_uuid_v1`/`op_rand_ulid`/
     `seconds_since_the_epoch`/`wall_clock_micros` already `Date`-gated.
     `governed_transaction.rs` clocks live on the host-thread protocol path
     (cfg(not wasm) callers) — left alone.
   - `pagerank`/`top_sort`/`graph_query` traps were the vendored
     `Instant::now` (pagerank) reached via web-time; web-time itself first
     panicked as "`Performance` object not found" because `js_sys::global()`
     prefers `self` and the probe's `self` stub lacked `performance` —
     probe-side fix (forward `globalThis.performance`), real browsers have it.
5. **Verify.** Release rebuild: `pkg/*_bg.wasm` **6,217,260 bytes** (+4 KB
   shims); `run-all.ps1` → 21/21 ok, exit 0. Fresh sizes: gzip-9
   **1,935,223 (31.1%)**, brotli SmallestSize **1,320,526 (21.2%)**.
   Stable: root `cargo check --workspace` exit 0; `cargo test -p mnestic
   --lib` **379 passed, 0 failed**.
6. Open: real `initThreadPool` still browser-only (demo pending);
   `opt-level s/z` comparison (R2c); probes still out of CI (revisit — the
   original reason is gone, but CI has no browser/Node-pool either).
