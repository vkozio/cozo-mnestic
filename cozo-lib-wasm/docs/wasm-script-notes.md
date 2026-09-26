# cozo-lib-wasm: CozoScript dialect and wasm32 runtime limits

Canonical reference for the CozoScript dialect as exposed by this build, and for
what `wasm32-unknown-unknown` cannot do. Audited against `cozo-lib-wasm` 0.18.0 /
`mnestic` 0.18.0.

Companion to `wasm-build-notes.md` (producing the artifact) and
`wasm-gap-report.md` (the **open work list** — what is still broken and queued).
Findings live here and only here; the gap report does not restate them.

**How to read an entry.** Every finding has a stable ID (`D*` = dialect,
`W*` = wasm-only) and a status:

| Status | Meaning |
|---|---|
| `engine` | Property of the engine. **Identical on every target.** |
| `wasm` | Only differs on `wasm32-unknown-unknown`. |
| `settled` | Closed. Recorded so it is not re-investigated. Do not spend time here. |

**Status of this document.** Script-level findings are pinned by
`cozo-core/tests/script_op_names.rs`, `fork_regressions.rs`,
`fts_index_surface.rs` and `vector_lsh_index_surface.rs`, which run on every PR.
Wasm-specific findings are **not** covered by CI — no wasm32 test target exists
today. Re-verify them with the recipe in [Verification](#verification) before
relying on one.

---

## Three rules that prevent most of the wasted investigation

1. **The grammar is the contract.** `cozo-core/src/cozoscript.pest` (~300 lines)
   settles every "why won't this parse" question in one read. Find the rule that
   owns the construct. Do not probe the binary for syntax questions.
2. **`eval::no_implementation` means "no such name" — never "unavailable on
   wasm".** It is raised in exactly one place, `Expr::UnboundApply`
   (`data/expr.rs`), carries no target information, and both registries
   (`get_op` in `data/expr.rs`, `parse_aggr` in `data/aggr.rs`) are plain
   `match name { … }` over `&'static` values with no `cfg` in either. A name
   missing from them is missing on *every* platform. Concluding "X is
   unavailable in the wasm build" from this error is always unsound.
3. **Silent behaviour is the class that source reading gets wrong.** `~`
   quietly matching, `str_includes` being case-sensitive, `:timeout` quietly
   doing nothing — none of these raise. When a claim is about behaviour rather
   than structure, run it.

---

## Part 1 — Dialect (`engine`)

### D1. One relation option per `run()` call — `settled`

Putting a DDL and a DML on the same stored relation in one script (`:create p {…}`
plus `:put p {…}`) used to drop the first silently and fail later with
`query::relation_not_found`, blaming the `:put`.

Cause was a parser bug, not transaction visibility: a parsed script carries a
single `out_opts.store_relation`, and the parser reassigned it on every
`relation_option` it saw. The transaction was always capable of it — `run_query`
creates the relation in the same tx, before running the plan.

**Fixed.** A second relation option is now rejected with
`parser::multiple_store_relation`. On 0.18.0 you still get the old silent-drop
behaviour, so one relation option per call is the correct usage either way:

```js
db.run(':create person {name: String => age: Int, city: String}', '{}', false);
db.run('?[name, age, city] <- [[...]] :put person {name, age, city}', '{}', false);
```

### D2. `*rel[…]` is positional and needs every column; `*rel{…}` is by name

`?[n, a] := *person[n, a]` on a 3-column relation → `eval::rule_arity_mismatch`.
So does the "ignore the rest" form `*person[n, _]`. And `*person{n, a, c}` →
`eval::named_field_not_found: … does not have field 'a'`.

Two different rules: `relation_apply` (`[…]`) is positional and the arity must
match; `relation_named_apply` (`{…}`) resolves each pair as a **column name**,
so `{n, a, c}` looks for columns literally named `n`, `a`, `c`.

```cozoscript
?[name] := *person{name: name, age: a}, a >= 41
```

*Note:* `*kv{k, v}` in the engine tests is not positional shorthand that happens
to work — `kv` is declared `{k: Int => v: Int}`, so those are real column names.

### D3. No pipe syntax; sorting and limits are option statements

`?[n] := *person[n, _] | order by n | limit 2` → `parser::pest`, with the reported
offset sitting exactly on the `|`. There is no `|` in the grammar; the knobs are
separate `option` rules.

```cozoscript
?[name, age] := *person{name: name, age: age}
:sort -age
:limit 5
```

Direction is `sort_dir = "+" | "-"` **in front of** the variable. `:sort n(name)`
parses but fails at runtime with `parser::sort_key_not_found` — sort by a plain
output variable. This is the largest single deviation from upstream CozoScript
docs: anything copied from upstream with pipes has to be rewritten.

### D4. `=` unifies, `==` compares

`unify = {var ~ "=" ~ expr}` and `op_eq = {"=="}` are distinct rules. Both forms
are accepted; use `==` when you mean comparison, so a stray `=` cannot silently
bind a variable instead of constraining it.

### D5. Aggregates are head-only, and the spellings are not what you expect

`?[c] := count(*person[name: _n])` → `parser::pest` at the `(`. An aggregate is a
head argument whose operand is a **variable**, not a relation application.

The same trap applies to plain functions in a head: `length(name)` in head
position is read as an *aggregate* and fails with `parser::aggr_not_found`, not
with an unknown-op error. Filter position is where `length` belongs.

The average is spelled **`mean`**, not `avg`. `avg` is the Cypher spelling;
`cypher/translate.rs` maps it to `mean` on the way in, which is what makes `avg`
look plausible. `sum` is registered and works.

```cozoscript
?[city, count(name)] := *person{name: name, city: city}
?[city, mean(age)]  := *person{name: name, city: city, age: age}
```

All 29 registered, verified end-to-end: `and`, `or`, `unique`, `group_count`,
`union`, `intersection`, `count`, `count_unique`, `variance`, `std_dev`, `sum`,
`product`, `min`, `max`, `mean`, `choice`, `collect`, `interval_coalesce`,
`shortest`, `min_cost`, `bit_and`, `bit_or`, `bit_xor`, `latest_by`,
`smallest_by`, `choice_rand`, `min_cost_k`, `pareto_min`, `pareto_max`.

### D6. String ops: `length` not `len`, and `~` is not "contains"

| Wanted | Actual |
|---|---|
| `len(n)` | **`length`**, filter position only (see D5) |
| substring test | **`str_includes(s, sub)`** — matches anywhere, so it subsumes `starts_with`; **case-sensitive** |
| `format("{} {}", a, b)` | no such op. Use `++`, `t2s`, or `from_substrings` |
| `n ~= "da"` | no such operator parses |
| `if a >= 40 then "x" else "y"` | does not parse. Conditionals exist only as `%if … %then … %end` inside `{ }` blocks |

**Bare `~` is the `coalesce` operator**, not a contains test. It substitutes its
right operand for a null left one, so `name ~ "fallback" == "Ada"` quietly
*matches*. A reader probing for a substring operator gets a non-error that reads
like a successful search — worse than a parse failure, and the trap that made
`~=` look like a missing feature.

For case-insensitive search, compose: `str_includes(lowercase(s), lowercase(sub))`.
Regex is available: `regex_matches`, `regex_replace`, `regex_extract`.

Works today: `++`, `to_string`, `to_int`, `starts_with`, `ends_with`, `lowercase`,
`uppercase`, `trim`, `length`, `str_includes`, `slice_string`, `chars`,
`regex_matches`, `round`, `in`, `not`, `or`, `&&`, `||`, `^`.

### D7. Index creation: `::fts` / `::hnsw` / `::lsh`, never `::index fts`

All three index kinds work. The trap is purely syntactic and hits all three
alike: `fts_idx_op`, `vec_idx_op` and `lsh_idx_op` are **separate top-level
`sys_script` alternatives**. `index_op` is `{"index" ~ (index_create |
index_drop)}` and takes only the plain form, so `::index fts create`,
`::index hnsw create` and `::index lsh create` are all parse errors that look
like "the feature is missing".

Second trap, also common to all three: the body is `index_create_adv`, whose
members are `ident ~ ":" ~ expr`. A bare column name is not a field.

```cozoscript
::fts create person:by_name {extractor: name, tokenizer: Simple}
::hnsw create pts:idx {dim: 2, m: 16, dtype: F32, fields: [emb],
                       distance: L2, ef_construction: 50}
::lsh  create doc:idx {extractor: body, tokenizer: Simple}
```

Retrieval is the `~rel:index` form with a pipe-delimited body for all three:
`bind_score` for FTS, `bind_distance` for HNSW, neither for LSH.

```cozoscript
?[id, score] := ~person:by_name{id | query: "ada", k: 10, bind_score: score}
?[id, dist]  := ~pts:idx{id | query: vec([40.0, 0.0]), k: 5, ef: 80,
                           bind_distance: dist} :order dist
```

Three further silent traps:

- `dtype` and `distance` are matched on the raw option text with no
  `eval_to_const`, so they must be **bare** — `dtype: F32`, never
  `dtype: "F32"`. The quoted form is rejected.
- `m` and `ef_construction` are **required** for HNSW and fail loudly rather than
  defaulting to a degenerate index.
- **LSH is minhash similarity search, not keyword search.** At the default
  `target_threshold: 0.9` a short query against unrelated text correctly returns
  zero rows — the query runs, raises nothing, and looks like a broken index. Use
  it for near-duplicate detection; use FTS for keyword search.

None of the three is feature-gated. All build paths resolve to 1 thread on wasm
(`build_threads()` finds no `available_parallelism`) and each guards its fan-out
with an explicit sequential branch — FTS `runtime/relation.rs:1468`, HNSW
`runtime/hnsw_build.rs:255`. LSH has no fan-out at all. They build correctly in a
browser, without a thread pool.

### D8. `::` system ops and read-only mode

The grammar's `sys_script` rule enumerates every valid op. There is **no
`::headers` and no `::lte`** — both exist upstream, both fail here with a span
near the `::` you typed, which sends you looking at the wrong character. You do
not need `::headers`: `headers` is in every successful result already.

With `immutable=true`, every mutating op returns `Cannot <do X> in read-only
mode` — including `::describe`, `::index create`, `::access_level`, `::compact`,
`::remove`, `::rename`, `::query create`. That is a clean error, but it means
"works in read-only" and "works at all" are different questions: always probe
DDL with `immutable=false`.

`::history rel [[1], [2]]` needs a `TxTime`-typed relation; on an ordinary one it
returns `::history requires a TxTime relation`.

### D9. Graphs are projections over relations — and the `::graph` syntax

`::graph create :knows {…}` does not parse (position 15 is the `:`). The name is
a plain `ident`: `::graph create g1 {edges: e}`, plus optional `nodes:`.

`::graph list` then returns `[]` and `::graph drop g1` fails with
`graph::projection_not_found` — the registered object is a traversal *projection*
bound at query time via `BudgetedTraversal(s[n], graph: 'g1', …)`, which is on
the wasm trap list (W2). So graph objects are unusable from wasm today; use the
recursion form in W3.

`?[] <~ BFS(person{name, age}, ["Ada"])` does not parse: `fixed_rel` wants a rule
`ident[…]` or a relation `*ident[…]`.

### D10. Only `in`, `not`, `or` and the comparison ops filter

`==`, `!=`, `>`, `>=`, `<`, `<=`, `in`, `not`, `or` all work as filters. This one
is listed only because it is short and there is no trap in it.

---

## Part 2 — wasm-only

### W1. `::query create` traps — `wasm`, unfixed

`::query create adults { … }` aborts with
`panicked at …/unsupported.rs:35: time not implemented on this platform`, then
`RuntimeError: unreachable` in JS. `unsupported.rs:35` is `SystemTime::now()`.

Cause: `runtime/stored_queries.rs:671` calls `SystemTime::now()` with no `cfg`
guard — the only unguarded wall-clock read in a query-execution path. Other time
ops (`op_now`, `current_validity`, `wall_clock_micros`, `seconds_since_the_epoch`,
`op_rand_uuid_v1`, `op_rand_ulid`) all have `js_sys::Date` branches, so this is a
missed spot in an already-done port, not a platform limit.

Workaround: don't use `::query create` on wasm. Persist queries as script
constants or re-issue them per call.

### W2. Every CSR-backed graph algorithm traps — `wasm`, unfixed, **cause was long misdiagnosed**

Any construct that builds a CSR adjacency traps with
`panicked at …/unsupported.rs:13: time not implemented on this platform`, then
`RuntimeError: unreachable`. Note `:13`, not `:35` — that is **`Instant::now()`**,
the monotonic clock, not `SystemTime`.

Cause: **not in our code.** `graph_builder` 0.4.1 calls `Instant::now()` purely
to time `info!` log lines:

```rust
// graph_builder-0.4.1/src/graph/adj_list.rs:225
let start = Instant::now();
info!("Initialized adjacency list in {:?}", start.elapsed());
```

The engine reaches it through two shared seams, `build_unweighted_csr` and
`build_weighted_csr` (`fixed_rule/mod.rs`), which every CSR-backed algorithm and
the graph-projection cache funnel through. The `graph` crate 0.3.1 has the same
logging-only reads (`page_rank.rs:89`, `sssp.rs:43`, `wcc.rs`,
`triangle_count.rs`), so `PageRank` is hit twice over.

**Blast radius — 12 of the 18 algorithms, plus `::graph create`:**
`all_pairs_shortest_path`, `budgeted_traversal`, `kruskal`, `label_propagation`,
`louvain`, `pagerank`, `prim`, `shortest_path_dijkstra`,
`strongly_connected_components`, `top_sort`, `triangles`, `yen`. Anything using
`graph: 'g'` goes through the projection cache and is equally affected.

An earlier revision of this document named only `BudgetedTraversal` and blamed
`SystemTime::now()` in the engine. Both were wrong; a fix scoped to
`BudgetedTraversal` would have left the other 11 in the same trap.

### W3. The portable graph-hop path — `engine`, verified on wasm

Six algorithms never reach the CSR seam — `astar`, `bfs`, `degree_centrality`,
`dfs`, `random_walk`, `shortest_path_bfs` — because they do not import `graph` at
all. `BFS` is confirmed working on wasm; the other five are structurally safe but
**unverified at runtime** (see the gap report).

The fully portable form, which needs no library support at all, is Datalog
recursion:

```cozoscript
start[n] <- [["Ada"]]
r[n, d] := start[n], d = 0
r[t, d1] := r[f, d], *edge{f, t, w}, d < 2, d1 = d + 1
?[n, min(d)] := r[n, d]
```

### W4. The query budget does not exist on wasm — `wasm`, unfixed

`budget_now()` returns `None` under `target_arch = "wasm32"`
(`runtime/db.rs`) because there is no monotonic `Instant`. Therefore `:timeout`
is a **no-op**, and the per-call `ScriptRunOptions::timeout` never engages. The
whole-script wall-clock deadline is absent.

It fails silently: a runaway query hangs the tab instead of returning
`eval::timeout`. The memory budget (`:mem_limit`) is unaffected.

**This is the largest wasm gap, and it is invisible.** Mitigations today: run in
a Web Worker (see the crate README for the cross-browser module caveat) and
keep queries bounded by construction.

### W5. Triggers are accepted and never fire — `wasm`, unfixed

`::set_triggers` parses, returns `{"ok":true,"rows":[["OK"]]}`, and does nothing.
`current_callback_targets` returns an empty set on wasm
(`runtime/callback.rs`), and the delivery machinery is `#[cfg]`-ed out.

Silent acceptance of a no-op is the worst failure shape available: it looks like
it worked. Verify triggers are absent on wasm rather than assuming they fired.

### W6. `took` is dropped from the result — `wasm`, unfixed

`run_script_fold_err` emits `took` under `#[cfg(not(target_arch = "wasm32"))]`.
The wasm entry point goes through it, so the one signal that would let a caller
see a slow query (relevant to W4) is absent on the target that needs it.

### W7. API removed at compile time — `wasm`, by design

- `GovernedTransaction` — whole type is `#[cfg(not(target_arch = "wasm32"))]`,
  taking the fork's admission control and cancellation ownership with it.
- `multi_transaction` — spawns a thread. `::kill` rides on the same machinery.
- `register_callback` / `unregister_callback` — `#[cfg]`-ed out.
- `::headers`, `::lte` — never existed here (D8).

These fail to compile rather than misbehave, which is the correct outcome.

### W8. Feature flags, storage, and threading — `wasm`, by design

`cozo-lib-wasm/Cargo.toml` resolves the engine as
`default-features = false, features = ["wasm", "graph-algo"]`, dropping the core
defaults `["compact", "fts-cangjie"]`:

- `fts-cangjie` — no Cangjie/jieba tokenizer. `Raw`, `Simple`, `Whitespace` and
  `NGram` are unconditional. Asking for `Cangjie` is a **clean `bail!`**
  (`fts/mod.rs`), not a trap: no instance loss, one clear error. Effect is
  confined to CJK tokenization quality — Chinese text is tokenized as whole runs.
- `storage-sqlite` / `storage-sqlite-src`, and `requests`.

Storage is **`mem` only**: `CozoDb.new()` is hardcoded to
`DbInstance::new("mem", "", "")`. There is no `open`/persistence API;
`export_relations` / `import_relations` (JSON strings) is the only way to move a
database in and out.

The API is **synchronous and main-thread**. A long script blocks the page, and
combined with W4 that is an uncancellable freeze rather than a slow query.

---

## Reference

### Result and error shape

`run()` returns a **JSON string**; parse it before use.

- Success: `{"ok":true,"headers":[…],"rows":[[…]],"next":null}`
- Failure: `{"ok":false,"code":…,"message":…,"display":…,"severity":…,"labels":[],
  "related":[],"causes":[],"filename":""}`

There is **no `reason` field** (older Cozo examples use it) — branch on `ok` and
render `code` + `message`. `display` is miette-rendered and **contains ANSI
colour escapes**; strip them or render `message` instead.

`code` is the useful part to branch on. The two that cause the most confusion are
`eval::no_implementation` (unknown *name* — see rule 2) and
`parser::aggr_not_found` (the name was fine but you used it in head position,
D5).

### JS API and the `init()` contract

- The default export is the async initialiser; `initSync` is also exported.
  `await init()` must complete before anything else touches the module.
- With no argument, `init()` resolves the binary as
  `new URL('cozo_lib_wasm_bg.wasm', import.meta.url)` and `fetch`es it. It also
  accepts a `Uint8Array`, `Response`, `Request`, `URL` or path string.
  **`await init({ module_or_path: bytes })` is the only form that works under
  Node**, because `fetch` refuses `file:` URLs. That is what
  `tests/integration.test.mjs` does, and why it is a Node test, not a browser one.
- The page must be served over HTTP; opening `index.html` from disk fails. Under
  Vite, the `new URL(..., import.meta.url)` lookup only survives if the package is
  kept out of the dep pre-bundler — see `examples/wasm-web-demo/vite.config.js`.
- After `init()`, any number of `CozoDb.new()` instances are allowed.
- The generated `package.json` `files` array omits `cozo_lib_wasm_bg.js`. That
  file is only needed for a `--target no-modules` build, so this is harmless for
  `--target web` consumers — but it is why the artifact looks one file short.

### Traps are traps, not errors

W1 and W2 are Rust panics. In wasm a panic is a **trap**: the `RuntimeError`
escapes whatever `await`/`try` you wrap it in, and the instance is left in an
undefined state — every later call on that `db` is suspect. Treat the whole
`CozoDb` as disposable after one.

### Verification

Everything tagged `engine` was executed on the host target, which is valid
because that code is target-independent. Everything tagged `wasm` was executed in
Node against a built `pkg/`.

```sh
# host: the pinned dialect suites
cargo test -p mnestic --test script_op_names
cargo test -p mnestic --test fork_regressions
cargo test -p mnestic --test fts_index_surface
cargo test -p mnestic --test vector_lsh_index_surface

# wasm: build, then run the integration suite (needs the wasm32 target + wasm-pack)
cd cozo-lib-wasm
cargo check --target wasm32-unknown-unknown
wasm-pack build --target web --release
node --test tests/integration.test.mjs
```

**Use one child process per case with a fresh `CozoDb`.** Both are load-bearing:
a failed script leaves partial state that cascades into unrelated cases, and a
panic leaves the instance unusable for every later case.

**There is no wasm32 CI gate.** The wasm32 clippy job in
`.github/workflows/wasm-release.yml` has been commented out since it stopped
being green, and `build.yml` covers only the host target. Nothing above is
currently enforced on a PR.

### Unverified

- Bundler/browser wiring. Everything above was executed through Node against
  `pkg/`; the Vite path in `examples/wasm-web-demo/` is reasoned from the glue
  source, not run.
- `astar`, `degree_centrality`, `dfs`, `random_walk`, `shortest_path_bfs` on
  wasm — structurally safe (W3) but never executed there.
- Large index builds in a real tab. The sequential branch is correct; its cost on
  a real corpus is unmeasured.
- Anything needing more memory than a browser tab will give it.
