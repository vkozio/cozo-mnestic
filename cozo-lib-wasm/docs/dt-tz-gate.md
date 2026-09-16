# Task: make `chrono-tz` optional behind a `dt-tz` cargo feature (drop ~2 MB from wasm)

> Location note: this doc lives with the wasm crate (`cozo-lib-wasm/docs/`)
> because it exists to shrink the wasm build, but all code paths below are
> project-root-relative (root = repo root).

## 1. Goal
`cozo-core` (crate `mnestic`) unconditionally depends on `chrono-tz 0.9.0`.
Its `build.rs` generates `timezones.rs` (**6 888 557 bytes**, verified at
`cozo-lib-wasm/target/wasm32-unknown-unknown/release/build/chrono-tz-*/out/`),
which compiles into static transition tables that dominate wasm `data[0]`
(**2 135 507 bytes, 35.25%** of `pkg/cozo_lib_wasm_bg.wasm` = 6 057 652 bytes,
measured with `twiggy top`). Wasm builds that only compare epoch timestamps
don't need IANA tables. Make the dependency optional so
`default-features = false` (the `cozo-lib-wasm` configuration) drops it.

Expected effect (estimate): raw −30% (~6.06 → ~4.2–4.4 MB),
brotli −40–50% (1.13 MB → ~0.55–0.7 MB). Confirm by measuring both configs.

Precedent: feature `fts-cangjie = ["dep:jieba-rs"]` (same pattern, already in
`cozo-core/Cargo.toml`, in `default`).

## 2. Cargo changes (`cozo-core/Cargo.toml`)
- `chrono-tz = "0.9.0"` → `chrono-tz = { version = "0.9.0", optional = true }`
- Add feature: `dt-tz = ["dep:chrono-tz"]` (with doc comment, next to `fts-cangjie`)
- `default = ["compact", "fts-cangjie"]` → add `"dt-tz"`
- Do NOT touch `chrono` (non-optional, needed for the UTC path).
- `cozo-lib-wasm/Cargo.toml` needs NO change (already `default-features = false`
  without the new feature).

## 3. Code map — every `chrono-tz` touch point

File for rows 1–9: `cozo-core/src/data/functions.rs`
(tests: `cozo-core/src/data/tests/functions.rs` — see §4).

Imports (lines 18–22): `chrono::{DateTime, Datelike, Days, Duration, LocalResult,
Months, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike, Utc}` — all stay
(`TimeZone`/`LocalResult`/`Offset`/`FixedOffset` are needed for the `Utc` fallback).

| # | Symbol | Lines | What it does | Gate strategy |
|---|--------|-------|--------------|---------------|
| 1 | `dt_tz` | 2592–2602 | parses optional trailing tz arg, default `Tz::UTC` | `#[cfg]`-split (see §3.1 three-tier semantics). With feature = as-is. Without = `parse_tz_fallback` (§3.1): missing/UTC/fixed-offset → `Utc`/`FixedOffset`, IANA name → `bail!` with the §3.1 error text. Return type becomes the `Zoned` alias below |
| 2 | `dt_zoned` | 2605–2609 | instant + tz → zoned datetime | change return type to `Zoned` alias; body unchanged |
| 3 | `dt_resolve_local` | 2627–2656 | DST fold/gap resolution via `tz.from_local_datetime` | change signature to `Zoned`; body compiles unchanged against `Utc` (`from_local_datetime` on `Utc` always `Single`; `Duration::minutes`, `LocalResult` already imported) |
| 4 | `dt_instant_to_secs` | 2658–2660 | zoned → float secs | change param type to `Zoned`; body unchanged (`timestamp_micros` is on all `DateTime<Tz>`) |
| 5 | `define_dt_component!` macro + 8 expansions (`op_dt_year/month/day/hour/minute/second/dow/doy`) | 2662–2687 | `fn(&DateTime<chrono_tz::Tz>) -> i64` | change bound to `fn(&Zoned) -> i64`; `Datelike`/`Timelike`/`weekday()`/`ordinal()` work for `DateTime<Utc>` identically |
| 6 | `op_dt_trunc` | 2690–2751 | trunc in zone; calls `dt_tz` (line 2694), `dt_resolve_local` (2745) | no direct `chrono_tz::` mention except via 1+3; compiles once they are gated. `dt.offset().fix()` (2744) works on `Utc` |
| 7 | `op_format_timestamp` tz branch | 2487–2497 (`Tz::from_str` at 2492) | formats with named zone | route through the same `parse_tz_fallback` (§3.1): without feature, UTC/fixed-offset names format with that offset, IANA names `bail!` with the §3.1 error text. `None` arm (2498) untouched |
| 8 | `op_dt_format` | 2869–2884, calls `dt_zoned` (2880) | strftime in zone | compiles via item 2; `format_with_items` is zone-agnostic |
| 9 | `op_dt_add` (2753), `op_dt_diff` (2829), `dt_whole_months` (2811), `dt_instant` (2576), `op_parse_timestamp`/`str2vld`/`parse_datetime_utc` (2530–2564) | — | pure-`Utc` code, NO `chrono_tz` use | touch nothing (this is the functionality that keeps working without the feature) |

New alias (put near line 2566 `// ---- datetime function library ----`):
```rust
#[cfg(feature = "dt-tz")]
type Zoned = DateTime<chrono_tz::Tz>;
#[cfg(not(feature = "dt-tz"))]
type Zoned = DateTime<Utc>;
```

### 3.1 Three-tier tz semantics (no tables below tier 3 — by construction)

`Utc` and `FixedOffset` carry the offset as a plain number of seconds: no IANA
data involved, and results for a constant offset are bit-identical to what the
tables would produce. So the no-feature configuration is NOT a stub — it is
fully correct for everything except DST-ruled IANA names:

- **Tier 1 — always:** missing tz arg → UTC; `"UTC"`, `"Z"`, `"z"` → UTC.
- **Tier 2 — always:** fixed offsets, no tables needed. Accept (case-insensitive
  `UTC` prefix optional, at least): `"UTC+3"`, `"UTC-05:30"`, `"+03:00"`,
  `"-0530"`, `"+03"`. Parse to `chrono::FixedOffset` (validate range ±23:59,
  else `bail!` as bad specification). In the `not(dt-tz)` build the `Zoned`
  alias is `DateTime<Utc>`-shaped — implement tier 2 by converting through
  `FixedOffset` at the boundary: `dt.with_timezone(&off)` yields
  `DateTime<FixedOffset>`, then `.with_timezone(&Utc)`-style normalization is
  NOT correct for display; instead keep the offset alongside: simplest conforming
  approach is a tiny local enum
  ```rust
  #[cfg(not(feature = "dt-tz"))]
  enum ZonedFallback { Utc(DateTime<Utc>), Fixed(DateTime<FixedOffset>) }
  ```
  with the dozen trait forwards the call sites need (`year/month/day/hour/minute/
  second/weekday/ordinal` via `Datelike`+`Timelike`, `offset().fix()`,
  `timestamp_micros`, `format_with_items`, `with_timezone` NOT needed downstream
  since construction happens in `dt_zoned`/`op_dt_trunc` only). Prefer this over
  stretching the plain `DateTime<Utc>` alias — display in `+03:00` must render
  the offset, not silently UTC.
- **Tier 3 — `dt-tz` feature only:** IANA names (`"Europe/Moscow"`, …) via
  `chrono_tz::Tz::from_str`. Without the feature any other string fails with
  exactly:
  `bad timezone specification: '{s}' (IANA zones require the `dt-tz` feature;
  UTC and fixed offsets like 'UTC+3' work without it)`

Note for `dt_resolve_local` without the feature: `FixedOffset::from_local_datetime`
is always `Single`, so the fold/gap machinery compiles and collapses to the
trivial path — DST simply cannot occur at a constant offset, no special-casing.
`prefer_offset` (`dt.offset().fix()`, line 2744) works for both variants.

## 4. Verified dependency chains (gortex relations, AST-resolved)

All paths project-root-relative.

- `cozo-core/src/data/functions.rs::dt_zoned` (:2605) → calls
  `cozo-core/src/data/functions.rs::dt_tz` (:2607) and
  `cozo-core/src/data/functions.rs::dt_instant` (:2576)
- `cozo-core/src/data/functions.rs::op_dt_trunc` (:2690) → calls
  `cozo-core/src/data/functions.rs::dt_tz` (:2694) and
  `cozo-core/src/data/functions.rs::dt_resolve_local` (:2745); callers of
  `cozo-core/src/data/functions.rs::op_dt_trunc` are ONLY tests:
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_units` (:1546),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_dst_trichotomy` (:1579),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_dst_fold_prefers_input_offset` (:2005),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_range_edges_error_not_panic` (:2054),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_resolves_deep_historic_gaps` (:2078)
- `cozo-core/src/data/functions.rs::op_dt_format` (:2870) → calls
  `cozo-core/src/data/functions.rs::dt_zoned` (:2880)
- `cozo-core/src/data/functions.rs::op_format_timestamp` (:2472) → references the
  `dt_tz` pattern
  (`cozo-core/src/data/functions.rs::op_format_timestamp:2492–2494`); test caller
  `cozo-core/src/data/tests/functions.rs::test_now` (:1406)
- `cozo-core/src/data/functions.rs::dt_instant_to_secs` (:2658) ← called by
  `cozo-core/src/data/functions.rs::op_dt_trunc` (:2745)
- `cozo-core/src/data/functions.rs::define_dt_component!` (:2662) + 8 expansions
  `cozo-core/src/data/functions.rs::op_dt_year/month/day/hour/minute/second/dow/doy`
  (:2673–2687) ← all call `cozo-core/src/data/functions.rs::dt_zoned` (:2666)
- Direct `chrono_tz::` users in tests:
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_dst_trichotomy`
  (Havana/Santiago, :1590/:1639),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_dst_fold_prefers_input_offset`
  (New_York, :2011),
  `cozo-core/src/data/tests/functions.rs::test_dt_trunc_resolves_deep_historic_gaps`
  (Casey, :2084) — gate these tests (or the non-UTC assertions) with
  `#[cfg(feature = "dt-tz")]`
- No `chrono_tz` usage outside `cozo-core/src/data/functions.rs` +
  `cozo-core/src/data/tests/functions.rs`
  (gortex text search over repo: 14 matches, all in these two files)
- Registration: `define_op!` constants (`OP_DT_*`, defined in
  `cozo-core/src/data/functions.rs`) stay registered in both configs; without the
  feature the ops exist with UTC + fixed-offset semantics (§3.1 tiers 1–2).
  A user passing `+03:00` never observes the missing feature; only IANA names bail.

## 5. Verification (must run all)
1. `cargo check -p mnestic` — default features, must pass (tz path intact)
2. `cargo check -p mnestic --no-default-features --features wasm,graph-algo` (match
   wasm consumer config) — must pass with `chrono-tz` absent; confirm via
   `cargo tree -p mnestic --no-default-features --features wasm,graph-algo -i chrono-tz`
   → "did not match any packages"
3. `cargo test -p mnestic --lib data::tests` (default) — tz tests still pass.
   Add/keep no-feature tests asserting tiers 1–2: `dt_zoned` with missing/`"UTC"`/
   `"UTC+3"`/`"+03:00"` works without `dt-tz`; `"Europe/Moscow"` errors with the
   §3.1 message. These tests must COMPILE in both configs (no `chrono_tz::` mention).
4. `cargo build --manifest-path cozo-lib-wasm/Cargo.toml --target wasm32-unknown-unknown --release`
   then `wasm-pack build cozo-lib-wasm --target bundler --release` (note:
   `[package.metadata.wasm-pack.profile.release] wasm-opt = false` is set because
   bundled wasm-opt 117 rejects rustc's bulk-memory/saturating-float ops)
5. Measure: `twiggy top -n 5 pkg/cozo_lib_wasm_bg.wasm` (`data[0]` should drop ~2 MB)
   + raw/gzip/brotli sizes via the `Measure-Compressed` pwsh function; report before/after

## 6. Out of scope
- `regex-automata` unicode tables + FTS stopword lists also in data segment
  (tens–hundreds of KB) — separate task if needed
- `panic = "abort"`, fresh `wasm-opt -Oz` — separate size tasks, orthogonal
