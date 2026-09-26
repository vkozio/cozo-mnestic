# wasm build: open work

The **work list** for `cozo-lib-wasm` on `wasm32-unknown-unknown`: what is still
broken, what is queued, and what is deliberately unverified.

**Findings do not live here.** Everything an author needs in order to *use* this
build — dialect, op names, traps, gaps — is in
[`wasm-script-notes.md`](wasm-script-notes.md), under stable IDs (`D1`…`D10`,
`W1`…`W8`). This file only tracks what to do about them. If you are about to
re-investigate something, check the [Disproved](#disproved-do-not-re-investigate)
ledger first.

Status of the last audit: 0.18.0 / `mnestic` 0.18.0, against the working tree at
`main` = `4baa95e1`. Script-level findings are pinned by four host test suites
that run on every PR. **No wasm32 finding is pinned by anything** — there is no
wasm32 test target and no wasm32 CI gate.

---

## Confirmed defects, ranked by risk-to-effort

Each was reproduced in Node against a built `pkg/`, or read off a single
unambiguous code path.

### 1. `::query create` traps — notes W1

`runtime/stored_queries.rs:671` calls `SystemTime::now()` with no `cfg` guard.
The only unguarded wall-clock read in a query-execution path.

**Fix:** route through `seconds_since_the_epoch()` (`runtime/db.rs`), which
already has the correct `js_sys::Date` branch. One line. Removes a trap that
leaves the instance unusable.

### 2. Every CSR-backed graph algorithm traps — notes W2

**12 of 18 algorithms, plus `::graph create` and any `graph: 'g'` query.** The
cause is in a third-party crate, not ours: `graph_builder` 0.4.1 calls
`Instant::now()` to time `info!` log lines
(`src/graph/adj_list.rs:225,233,250,…`). The `graph` crate 0.3.1 does the same
(`page_rank.rs:89`, `sssp.rs:43`, `wcc.rs`, `triangle_count.rs`), so `PageRank`
is hit twice.

Reached via two shared seams: `build_unweighted_csr` and `build_weighted_csr`
(`fixed_rule/mod.rs`). Six algorithms bypass them entirely and are safe — see notes W3.

**Two options — this needs a product decision, see [Open questions](#open-questions).**

| | Approach | Cost | Result |
|---|---|---|---|
| **A** | `bail!` at the two CSR seams under `cfg(target_arch = "wasm32")` | ~20 lines, ours | Traps become clean `Result` errors. 12 algos + `::graph` unavailable on wasm. The 6 safe algos keep working. |
| **B** | Fork `graph_builder` + `graph`, delete the `Instant::now()`/`start.elapsed()` log pairs, `[patch.crates-io]` both | ~700 KB vendored, 2 patches | 12 algos work on wasm. |

B has direct precedent in this repo: `cozo-lib-wasm/shim/page_size` is a vendored
patch for the same class of problem (a missing non-unix/non-windows stub), and
`cozo-lib-wasm` is a **separate workspace**, so a `[patch.crates-io]` there
leaves host builds on the stock crates. Option A is the better trade unless
graph algorithms in the browser are a real requirement.

### 3. Triggers are accepted and never fire — notes W5

`current_callback_targets` returns an empty set on wasm
(`runtime/callback.rs`); delivery is `#[cfg]`-ed out. `::set_triggers` parses,
succeeds, does nothing.

**Fix:** `bail!` at parse time in `parse/sys.rs` (`Rule::trigger_relation_op`),
mirroring the existing `:sleep` pattern in `parse/query.rs`. A few lines.
Silent acceptance of a no-op is worse than absence.

### 4. The query budget does not exist on wasm — notes W4

`budget_now()` returns `None` under `cfg(target_arch = "wasm32")`
(`runtime/db.rs`), so `:timeout` and `ScriptRunOptions::timeout` are both
no-ops. A runaway query hangs the tab instead of returning `eval::timeout`. This
is the fork's headline feature and it is invisible to users.

**Blocked on a decision** — see [Open questions](#open-questions). If the answer
is no, the fallback is a loud `bail!` at parse time, same `:sleep` pattern, so a
runaway query fails fast instead of freezing the tab.

Implementation shape if yes: `Option<Instant>` is threaded through ~15 sites in
`runtime/db.rs`, `runtime/transact.rs`, `runtime/imperative.rs` and `lib.rs`, and
has to become a deadline-from-start type. The public `GovernedTransactionOptions`
is already `cfg(not(wasm32))`, so it is untouched. `governed_transaction.rs`,
`callback.rs` and `columnar.rs` are all non-wasm already.

### 5. `took` is dropped from the result — notes W6

`run_script_fold_err` emits it under `#[cfg(not(target_arch = "wasm32"))]`
(`lib.rs`). The wasm entry point goes through it.

**Fix:** ~4 lines, independent of item 4 — `js_sys::Date::now()` under `cfg` is
enough. Do this one first: it is the only observable signal for item 4.

### 6. No wasm32 CI gate

The wasm32 clippy job in `.github/workflows/wasm-release.yml` has been commented
out since it stopped being green, and `build.yml` covers only the host target. An
unused variable in the engine's wasm32-only code path (e.g.
`runtime/db.rs:500`) is currently invisible to CI.

**Fix:** uncomment the job. It is already written and documents its own
reasoning.

### 7. The two traps have no regression test

`cozo-lib-wasm/tests/integration.test.mjs` has six generic tests — none touches
a trap. Items 1 and 2 are one-line mistakes away from regressing, and neither is
covered.

**Fix:** add process-per-case regressions (see [Reproducing](#reproducing) for
the harness shape). Put the clippy gate up first so they are actually enforced.

---

## Disproved — do not re-investigate

Every one of these was believed at some point and is **wrong**. They cost the
most time, because each looked like a wasm limitation and none was.

| Belief | Reality |
|---|---|
| "The op/aggregate registries are built per target, so `avg`/`sum` are not wired up in the wasm build." | Both are static `match name { … }` over `&'static` values with no `cfg` (`data/expr.rs`, `data/aggr.rs`). A missing name is missing everywhere. |
| "`eval::no_implementation` means the op is unavailable in this build." | It is raised in exactly one place (`Expr::UnboundApply`, an unrecognised name) and carries no target information. Any conclusion of that form is unsound. |
| "Use JavaScript to compute averages — the engine has no aggregate for it." | The aggregate is `mean`. `avg` is the Cypher spelling, which `cypher/translate.rs` maps on the way in. `sum`, `std_dev`, `collect` and 25 others all work. |
| "`len` is unimplemented / there is no substring operator, use `starts_with`." | The ops are `length` and `str_includes`. `str_includes` matches anywhere and subsumes `starts_with`. It is case-sensitive. |
| "`~=` does not parse, so substring matching is missing." | Bare `~` is **`coalesce`**. `name ~ "fallback" == "Ada"` parses, evaluates, and *matches* — a non-error that reads like a working search. |
| "The silent DDL drop is a transaction-visibility problem: the relation's metadata is not visible to the DML that creates it." | A parser bug, now fixed. A parsed script carries one `out_opts.store_relation` and the parser reassigned it per `relation_option`, so the `:create` was dropped silently. The transaction was never involved. |
| "Both panicking paths stamp wall-clock time with `SystemTime::now()`." | They are two *different* clocks in two different places. `::query create` is `SystemTime` (`unsupported.rs:35`, ours). The graph algorithms are `Instant` (`unsupported.rs:13`, inside `graph_builder`). |
| "`BudgetedTraversal` stamps wall-clock time, so the fix is the same as for `::query create`." | `budgeted_traversal.rs` contains **no** time call at all. Neither is rayon. The cause is in a vendored dependency's logging. |
| "Only `BudgetedTraversal` traps." | 12 of 18 algorithms, plus `::graph create` and `graph: 'g'` — they share two CSR seams. A fix scoped to `BudgetedTraversal` would have left 11 traps. |
| "Graph objects are unusable from wasm." | Six algorithms and plain Datalog recursion work. It is the CSR-backed subset that traps. |
| "`::index fts create` does not parse, so FTS is probably unavailable on wasm." | A syntax error with no wasm component. The spelling is `::fts create`, with `ident: expr` fields. FTS, HNSW **and** LSH all work. |
| "FTS / vector / LSH index creation is untested on wasm, possibly unavailable." | All three create, answer a real query, and drop. Pinned by two host suites. |
| "The missing Cangjie tokenizer is a trap." | A clean `bail!` with one clear error and no instance loss. Effect is confined to CJK tokenization quality. |

**The lesson behind the table.** Every entry above is either a parse-level
question (settle it in `cozoscript.pest`) or a *silent* behaviour (settle it by
running it). Reading source gets the second class wrong. The three rules at the
top of `wasm-script-notes.md` are the distilled form of this.

---

## Open questions

1. **Is a monotonic clock worth wiring the budget to?** `performance.now()` via
   `web-sys` is truly monotonic but `web_sys::window()` returns `None` in a Web
   Worker — which the crate README recommends — and in Node, where the tests run.
   `js_sys::Date::now()` works everywhere but is wall-clock: a backward clock
   step means the query outlives its budget. This is an environment question that
   source reading cannot answer. Affects item 4.
2. **Fork the graph crates, or accept the `bail!`?** See item 2. Affects whether
   12 algorithms are usable in a browser.
3. **Do `astar`, `degree_centrality`, `dfs`, `random_walk` and
   `shortest_path_bfs` run on wasm?** They bypass the CSR seams and import no
   `graph` symbol, so they are structurally safe, but only `BFS` has actually
   been executed there.
4. **What does a large index build cost in a real tab?** The sequential branch is
   correct; its cost on a real corpus is unmeasured. Anyone targeting a large
   vector or text index in a browser should benchmark before committing.
5. **Does the bundler path work?** The Vite config in
   `examples/wasm-web-demo/` is reasoned from the glue source, never run.

---

## Reproducing

The environment has the `wasm32-unknown-unknown` target, `wasm-pack` and Node,
and resolves offline, so none of this needs a browser or a network.

```sh
# host dialect suites (these run in CI)
cargo test -p mnestic --test script_op_names
cargo test -p mnestic --test fork_regressions
cargo test -p mnestic --test fts_index_surface
cargo test -p mnestic --test vector_lsh_index_surface

# wasm: check, build, run
cd cozo-lib-wasm
cargo check --target wasm32-unknown-unknown          # ~20s incremental
wasm-pack build --target web --release
node --test tests/integration.test.mjs
```

The package is `mnestic` with lib name `cozo` — `-p cozo-core` fails.

**Harness shape for trap work: one child process per case, fresh `CozoDb` per
case.** Both constraints are load-bearing — a failed script leaves partial state
that cascades into unrelated cases, and a panic leaves the instance unusable for
every later case. `console_error_panic_hook` is already installed, so the panic
message (and its `unsupported.rs` line number, which is what distinguishes
`Instant` from `SystemTime`) goes to stderr.

**Caveat:** the last audit ran against a `pkg/` built 2025-09-22, not against a
fresh build of the working tree. Rebuild before re-confirming any item above.

---

## Reference index

| Concern | Location |
|---|---|
| Grammar (the contract) | `cozo-core/src/cozoscript.pest` |
| Expression op registry | `cozo-core/src/data/expr.rs` (`get_op`) |
| Aggregate registry | `cozo-core/src/data/aggr.rs` (`parse_aggr`) |
| `no_implementation` origin | `cozo-core/src/data/expr.rs` (`Expr::UnboundApply`) |
| Unguarded wall clock (item 1) | `cozo-core/src/runtime/stored_queries.rs:671` |
| CSR seams (item 2) | `cozo-core/src/fixed_rule/mod.rs` (`build_unweighted_csr`, `build_weighted_csr`) |
| Third-party clock read (item 2) | `graph_builder-0.4.1/src/graph/adj_list.rs:225,233,250` |
| Query budget clock (item 4) | `cozo-core/src/runtime/db.rs` (`budget_now`) |
| wasm callbacks no-op (item 3) | `cozo-core/src/runtime/callback.rs:61-75` |
| Trigger parse site (item 3) | `cozo-core/src/parse/sys.rs` (`Rule::trigger_relation_op`) |
| `bail!`-at-parse precedent | `cozo-core/src/parse/query.rs:316-318` (`:sleep`) |
| `took` (item 5) | `cozo-core/src/lib.rs:597-637` |
| wasm32 CI gate (item 6) | `.github/workflows/wasm-release.yml` |
| wasm feature selection | `cozo-lib-wasm/Cargo.toml` |
| wasm shim (mem-only) | `cozo-lib-wasm/src/lib.rs:30` |
| vendored-patch precedent | `cozo-lib-wasm/shim/page_size` |

To re-audit the time situation after any change:

```sh
grep -rn 'SystemTime::now\|Instant::now' cozo-core/src --include=*.rs
```

Every hit must be `#[cfg]`-guarded or routed through `seconds_since_the_epoch()`
/ `wall_clock_micros()` — and this grep does **not** cover vendored dependencies,
which is how item 2 stayed hidden.
