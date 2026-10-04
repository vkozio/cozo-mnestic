# cozo-redb fork evaluation (2026-09-28)

[`lawless-m/cozo-redb`](https://github.com/lawless-m/cozo-redb) is a personal
Rust-first fork of upstream CozoDB (dormant since 2024-12). Evaluated at their
`main` (`92525b0`, "Sweep dead code, simplify StoreTx, make rayon
non-optional"). Their own scope statement (`DIFFERENCES.md`): *"redb +
Datalog + time travel + vector search, nothing else"*.

## What they cut (verified against the tree, not just claimed)

- Backends: only `mem` + `redb` remain. Gone: `cozorocks` (C++ FFI + 42 MB
  `librocksdb` submodule), `sled`, `tikv`, `sqlite`, `rocksdb`.
- Bindings: Python/Node/Java/Swift/C gone; only Rust + `cozo-lib-wasm`
  (wasm-pack over `MemStorage`).
- No client-server mode, no backup/restore API (the old one rode on sqlite as
  an intermediate format; backup is now "copy the `.redb` file").
- MinHash-LSH removed entirely.
- FTS modified: tantivy 0.22 **without** jieba/cangjie — zero mentions in
  `fts/mod.rs` or `Cargo.toml` (checked by search).
- HEAD commit additionally: `StoreTx::get/exists` lost `for_update`,
  `RelationStore::get_relation` lost `lock`, dead tests/benches/fixtures
  deleted (`air_routes` + 1.8 MB fixtures, `pokec`, `wiki_pagerank`), rayon
  non-optional.
- Added: the redb backend with time travel, runnable benches (their claim:
  redb beats sqlite 32–49 % on reads; **not independently verified**), CI.

## What we took

`cozo-core/src/storage/re.rs` (~340 LOC + tests), ported as the optional
`storage-redb` feature (redb 4.3.0 locked 2026-09-28). Adaptations for our
diverged traits: ignored `_for_update` (like `mem.rs`), `storage_kind`,
`batch_put`, `total_scan`, one-arg `Db::new`. Time travel ports 1:1 through
the shared `check_key_for_validity` helper. Verified here: clippy clean,
383/383 lib tests (incl. basic/delete/time-travel/FTS-roundtrip on redb).

Two porting notes that will bite on the next redb upgrade:

- redb 4.x moved read methods off the write-`Table` behind the
  `ReadableTable` trait. The `ReadableTable` import in `re.rs` looks unused
  but is load-bearing — removing it fails with E0599.
- `WriteTransaction` is ~700 bytes; it is boxed (`Option<Box<…>>`) to keep
  `clippy::large_enum_variant` quiet.

Added after the port itself:

- `DbInstance::Redb` plus the ~30 dispatch arms it needs in
  `cozo-core/src/lib.rs` — one mechanical pass, all behind the same feature
  flag, so the enum is unchanged for anyone who does not enable it.
- `cozo-bin` exposure: `storage-redb` feature forwarding and `-e redb`.

## What we deliberately did NOT take (and why)

- **Their FTS.** No Cangjie/Chinese tokenizers — incompatible with our
  `fts-cangjie`-on host. Only the storage backend ports.
- **Their `StoreTx` simplification** (dropping `for_update`/`lock`). It is a
  diff across `query/`, `runtime/`, `fixed_rule/` for cosmetic gain. Revisit
  only with a dedicated refactor.

## Open items / risks

- `unsafe impl Sync for RedbTx` carries their SAFETY argument verbatim — read
  it with fresh eyes before trusting it in production.
- redb 4.x freshness: run `cargo audit`/`cargo deny` before any release
  containing `storage-redb`.
- Single-writer semantics: a second concurrent write transaction fails at
  `begin_write`. Matches the engine today, but any future parallel-write work
  must know this.
- Their benchmark numbers are taken on faith; our own baselines
  (`cozo-core/benches/point_lookup.rs` etc.) should get a redb leg before
  performance claims.
- ROADMAP candidate: SST-ingest/`durable_writes` policy for redb (currently
  defaults: no ingest, knob unhonored — redb commits are durable by
  construction). The `DbInstance` `"redb"` dispatch and the `cozo-bin`
  exposure parts of this candidate are done; see above.
