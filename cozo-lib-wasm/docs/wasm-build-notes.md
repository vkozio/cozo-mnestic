# cozo-lib-wasm: build pitfalls journal (wasm32-unknown-unknown)

Target triple: `wasm32-unknown-unknown`. Crate: `cozo-lib-wasm`
(consumes `mnestic` = `cozo-core` with `default-features = false`,
features `["wasm", "graph-algo"]`). Every entry below already bit us once —
read before touching wasm deps/features/toolchain.

## 1. `nanorand 0.7.0` — `E0425 backup_entropy` (transitive via `graph`)
- Symptom: `error[E0425]: cannot find function 'backup_entropy'`,
  `nanorand-0.7.0/src/entropy.rs:51`.
- Cause: upstream bug. On wasm32 without the `getrandom` feature, `system()`
  calls `backup_entropy()`, which only exists with default-features OFF (no `std`).
  `graph 0.3.1` pulls `nanorand` with `std` on, `getrandom` off → broken combo.
- Fix (in `cozo-lib-wasm/Cargo.toml`, wasm-only deps):
  `nanorand = { version = "0.7", features = ["getrandom"] }` — Cargo feature
  unification flips the transitive `nanorand` to the getrandom-backed branch.
- Lesson: transitive RNG/entropy crates need explicit feature unification per
  target; check the `--cfg feature="..."` list in `cargo build -v` output.

## 2. `getrandom` 0.2 vs 0.3 — two crates, two feature flags
- `rand 0.8` / `nanorand` → `getrandom 0.2`, wasm flag is `js`.
- `ahash 0.8` (via `graph`, default `runtime-rng`) → `getrandom 0.3`, wasm flag
  is `wasm_js`. Major versions do NOT unify.
- So BOTH are needed:
  `getrandom = { version = "0.2", features = ["js", "wasm-bindgen"] }` (normal dep)
  and `getrandom_03 = { package = "getrandom", version = "0.3", features = ["wasm_js"] }`
  (wasm-only dep). Verified by `cargo tree --target wasm32-unknown-unknown -i getrandom@0.3.4`.
- Lesson: when adding a dep that pulls `ahash`/`rand`, re-run the `cargo tree -i getrandom`
  check for BOTH majors.

## 3. `jieba-rs 0.9` — `Jieba::new()` gated behind `default-dict`
- Symptom: `E0599`, misleading `Digest`-bounds error (compiler resolves the missing
  inherent `new()` to the `Digest` trait method).
- `cozo-core` uses `default-features = false` → only `Jieba::empty()` exists.
- Fix applied: `CangJieTokenizer` uses `Jieba::empty()`; then CJK support fully
  gated behind new `fts-cangjie = ["dep:jieba-rs"]` feature (in `default`,
  OFF for wasm since wasm uses `default-features = false`). See `fts-cangjie`
  precedent if more FTS weight needs gating.
- Lesson: after any `jieba-rs`/`cedarwood`/`phf` bump, rebuild wasm; gate
  language-specific tokenizers behind features.

## 4. `page_size 0.4.2` — missing wasm stub (`get_granularity_helper`)
- Fixed via `[patch.crates-io] page_size = { path = "shim/page_size" }`
  (`cozo-lib-wasm/shim/page_size`). Before upgrading `page_size`, check upstream
  for a wasm stub; remove the shim only then.

## 5. `wasm-pack`'s bundled `wasm-opt 117` is too old
- Symptom: `wasm-validator error ... Bulk memory operations require bulk memory`,
  `i64.trunc_sat_f64_s ... all used features should be allowed`, `wasm-opt` exit 1.
- Cause: modern `rustc` emits bulk-memory + saturating-float ops by default;
  pinned binaryen 117 doesn't know them. NOT our code.
- Fix: `[package.metadata.wasm-pack.profile.release] wasm-opt = false` in
  `cozo-lib-wasm/Cargo.toml`. For manual optimize install fresh binaryen and run
  with `--enable-bulk-memory --enable-nontrapping-float-to-int`.
- Lesson: do NOT "fix" by disabling rustc target features (`-C target-feature=-bulk-memory`)
  — that sacrifices perf/size to accommodate an old tool.

## 6. Size workflow (what we measure, baseline 2026-09-16)
- Baseline: raw 6 057 652 → gzip 1 642 239 → brotli 1 132 393 (max levels, pwsh one-liner, see below).
- `twiggy top -n 25 pkg/cozo_lib_wasm_bg.wasm`: `data[0]` = 2 135 507 (35%) —
  mostly `chrono-tz` tables (see `docs/dt-tz-gate.md`). Code is a long tail of
  ~20k small functions (largest 51 KB) — LTO + `wasm-opt -Oz` territory.
- `[IO.File]` resolves relative paths against the PROCESS cwd, not the pwsh
  location — always `Resolve-Path` first; `GZipStream`/`BrotliStream` close the
  underlying stream unless constructed with `leaveOpen=$true`.
- Measure-compressed one-liner (run from repo root):
  ```powershell
  $f=(Resolve-Path 'cozo-lib-wasm/pkg/cozo_lib_wasm_bg.wasm').Path; 'orig : {0:N0}'-f(Get-Item $f).Length; $m=[IO.MemoryStream]::new(); $g=[IO.Compression.GZipStream]::new($m,[IO.Compression.CompressionLevel]::SmallestSize,$true); [IO.File]::OpenRead($f).CopyTo($g); $g.Close(); 'gzip : {0:N0}'-f $m.Length; $m2=[IO.MemoryStream]::new(); $b=[IO.Compression.BrotliStream]::new($m2,[IO.Compression.CompressionLevel]::SmallestSize,$true); [IO.File]::OpenRead($f).CopyTo($b); $b.Close(); 'brotli: {0:N0}'-f $m2.Length
  ```
- Always measure `pkg/*.wasm` (post-bindgen), not `target/.../release/*.wasm`.

## 7. Open size tasks
- `docs/dt-tz-gate.md` — gate `chrono-tz` behind `dt-tz` (UTC + fixed offsets always work).
- Release profile ideas: `panic = "abort"` (check `catch_unwind` first), `opt-level "s"` vs `"z"`,
  dedicated `[profile.wasm-release]`, `console_error_panic_hook` off for prod.
