# cozo-lib-wasm: CozoScript dialect and runtime limits (wasm32)

Companion to `wasm-build-notes.md` (that one is about *producing* the artifact;
this one is about *running* it). Everything here was executed against
`cozo-lib-wasm` 0.18.0 on `wasm32-unknown-unknown` with `mem` storage.

The engine's grammar is the contract: `cozo-core/src/cozoscript.pest`. When a
construct is rejected here, grep that file for the rule that owns it — the
grammar is small enough to read in one sitting and it settles every "why won't
this parse" question faster than probing. Working examples live in
`cozo-core/tests/*.rs` (several tests exercise exactly the forms below, e.g.
`backend_transaction_contract.rs` for `:put`/`:rm`, `budgeted_traversal.rs` for
`<~` traversal and `::graph`).

**Verification method.** One child process per case, fresh `CozoDb` per case.
Both are load-bearing: a Rust panic in wasm32 traps as `RuntimeError:
unreachable` and leaves the instance unusable for every later case, and a failed
script leaves partial state that cascades into unrelated cases. See §8.

**Which claims are pinned by tests.** Anything in this document about a *script*
surface — op names, aggregate names, the DDL/DML rule — is now executed by
`cozo-core/tests/script_op_names.rs` and
`cozo-core/tests/fork_regressions.rs` on the host target. Those run on every PR,
so the spellings below cannot rot silently. Claims about the *wasm wrapper*
itself (the `init()` contract, the bundler path, the §8 panics) are not covered
by CI and are marked where they appear.

**Reading errors.** `eval::no_implementation` comes from exactly one place in
the engine — an unrecognised *name* (`Expr::UnboundApply`). It is not a
target-availability signal. When you see it, the fix is the spelling, never a
workaround. The parallel `parser::aggr_not_found` means the name was fine but
you used it in head position, where it was read as an aggregate.

## Support matrix (executed, 0.18.0)

| Area | Status |
|---|---|
| `:create` / `:put` / `:insert` / `:set` / `:rm` / `:delete` | works, but one relation option per `run()` — §1 |
| `*rel[a, b, c]` positional | works, **must list every column** — §2 |
| `*rel{name: n}` by name | works, subset allowed — §2 |
| Filters `==` `!=` `>` `>=` `<` `<=`, `in`, `not`, `or` | works |
| `:sort` / `:order` (with `+` / `-`), `:limit`, `:offset` | works — §3 |
| `\| order by ... \| limit ...` | **does not parse** — §3 |
| `count` / `min` / `max` / `sum` / **`mean`** in the head | works — §5 |
| `avg` in the head | unknown name — the aggregate is spelled `mean` — §5 |
| `std_dev`, `variance`, `product`, `collect`, `count_unique`, `unique`, … | work — §5 |
| `++`, `to_string`, `to_int`, `starts_with`, `lowercase`, `round` | works |
| `length`, `str_includes` | work — §6 |
| `len`, `format`, `~=` | not registered names; `~` is **coalesce** — §6 |
| `if/then/else` expression | does not parse — §6 |
| Datalog recursion | works — the portable way to do graph hops — §9 |
| `::relations`, `::columns`, `::explain { }`, `::fixed_rules`, `::warnings` | works read-only |
| `::describe`, `::index create`, `::access_level`, `::compact`, `::remove`, `::rename` | need `immutable=false` — §7 |
| `::query create` | **panics** — §8 |
| `<~ BudgetedTraversal(...)` | **panics** — §8 |
| `::headers`, `::lte` | do not exist in this build — §7 |
| Persistence | none: `CozoDb.new()` is `mem` only |

## 1. One relation option per `run()` call
- Symptom: `:create person {...}` and the `?[...] <- [...] :put person {...}` in
  one script → `query::relation_not_found: Cannot find requested stored relation
  'person'`. The `:create` looks like it succeeded — the error only surfaces
  later, on the read-back, which sends you hunting in the wrong place.
- Cause: **not** a transaction-visibility problem. A parsed script carries a
  single `out_opts.store_relation`, and the parser *reassigned* it on every
  `relation_option` it saw. The second one won, so the `:create` was dropped on
  the floor with no diagnostic and the `:put` inherited the blame. The
  transaction was always capable of it: `run_query` creates the relation in the
  same tx, before running the plan.
- Status: **fixed upstream.** A second relation option is now rejected with
  `parser::multiple_store_relation` ("A query can only have one stored
  relation"), so a combined script fails loudly at the statement you actually
  wrote. The one-option-per-call rule below is still the correct usage.
- Fix: one relation option per call.
  ```js
  db.run(':create person {name: String => age: Int, city: String}', '{}', false);
  db.run('?[name, age, city] <- [[...]] :put person {name, age, city}', '{}', false);
  ```
- Lesson: a `relation_not_found` right after a successful-looking DDL is this.
  If you see it on 0.18.0 you have the silent-drop behaviour; on a build with
  the fix you get `parser::multiple_store_relation` instead, which names the
  real problem.

## 2. `*rel[...]` is positional and needs every column; `*rel{...}` is by name
- Symptom: `?[n, a] := *person[n, a]` on a 3-column relation →
  `eval::rule_arity_mismatch`. So does the "ignore the rest" form
  `*person[n, _]`. And `*person{n, a, c}` → `eval::named_field_not_found:
  stored relation 'person' does not have field 'a'`.
- Cause: two different rules. `relation_apply` (`[...]`) is positional and the
  arity must match; `relation_named_apply` (`{...}`) resolves each
  `named_apply_pair` as a **column name** (`underscore_ident ~ (":" ~ expr)?`),
  so `{n, a, c}` looks for columns literally named `n`, `a`, `c`.
- Fix: positional only when you want every column; use the named form for
  anything with a filter, and it is also the only way to bind a subset:
  ```cozoscript
  ?[name] := *person{name: name, age: a}, a >= 41
  ```
- Lesson: `*kv{k, v}` in the engine tests is not positional shorthand that happens
  to work — `kv` is declared `{k: Int => v: Int}`, so those are real column
  names. The coincidence is what makes the tests misleading.

## 3. No pipe syntax: sorting and limits are option statements
- Symptom: `?[n] := *person[n, _] | order by n | limit 2` →
  `parser::pest` "unexpected input / end of input", with the reported offset
  sitting exactly on the `|`. Same for `| first`, `| offset`.
- Cause: there is no `|` in the grammar. `query_script` is
  `(option | rule | const_rule | fixed_rule)+`, and the ordering knobs are
  separate `option` rules: `limit_option`, `offset_option`,
  `sort_option = {(":sort" | ":order") ~ (sort_arg ~ ",")* ~ sort_arg}`.
- Fix: one option per line (or `;`-terminated) after the rule.
  ```cozoscript
  ?[name, age] := *person[name: name, age: age]
  :sort -age
  :limit 5
  ```
  Direction is `sort_dir = "+" | "-"` **in front of** the variable, so
  descending is `:sort -age`, not `:sort age desc`.
- Also: `sort_arg` allows `out_arg = var ~ ("(" ~ var ~ ")")?`, but
  `:sort n(name)` fails at runtime with `parser::sort_key_not_found`. The
  parenthesised form is not usable in this build — sort by a plain output
  variable.
- Lesson: this is the single biggest deviation from upstream CozoScript docs.
  Anything you copy from upstream with pipes has to be rewritten.

## 4. `=` unifies, `==` compares
- `unify = {var ~ "=" ~ expr}` and `unify_multi = {var ~ in_op ~ expr}` are
  distinct rules from `op_eq = {"=="}` in the expression operator table. Both
  forms are accepted; use `==` when you mean comparison so a stray `=` does not
  silently bind a variable instead of constraining it.

## 5. Aggregates: head-only, and the spellings are not what you expect
- Symptom: `?[c] := count(*person[name: _n])` → `parser::pest` at the `(`. And
  `avg` returns `eval::no_implementation`.
- Cause: `head_arg = {aggr_arg | var}` and
  `aggr_arg = {ident ~ "(" ~ var ~ ("," ~ expr)* ~ ")"}` — an aggregate is a
  head argument whose operand is a **variable**, not a relation application.
  Note the same restriction applies to plain functions in a head: `length(name)`
  in head position is read as an *aggregate* and fails with
  `parser::aggr_not_found`, not with the unknown-op error. Filter position is
  where `length` belongs.
- The op and aggregate registries are **static name tables**, identical on every
  target: `get_op` (`cozo-core/src/data/expr.rs`) and `parse_aggr`
  (`cozo-core/src/data/aggr.rs`) are plain `match name { ... }` over
  `&'static` values. So `eval::no_implementation` means *no such name*, and
  never means "unavailable in the wasm build". An earlier revision of this
  document drew the opposite conclusion and prescribed computing averages in JS.
- The average is spelled **`mean`**, not `avg`. (`avg` is the Cypher spelling;
  `cypher/translate.rs` maps it to `mean` on the way in, which is what makes
  `avg` look plausible.) `sum` is registered and works.
- Fix:
  ```cozoscript
  ?[city, count(name)] := *person{name: name, city: city}
  ?[city, mean(age)]  := *person{name: name, city: city, age: age}
  ```
- Available, all verified end-to-end in `cozo-core/tests/script_op_names.rs`:
  `and`, `or`, `unique`, `group_count`, `union`, `intersection`, `count`,
  `count_unique`, `variance`, `std_dev`, `sum`, `product`, `min`, `max`, `mean`,
  `choice`, `collect`, `interval_coalesce`, `shortest`, `min_cost`, `bit_and`,
  `bit_or`, `bit_xor`, `latest_by`, `smallest_by`, `choice_rand`,
  `min_cost_k`, `pareto_min`, `pareto_max`.

## 6. String ops: wrong names, and `~` is not "contains"
- `len(n)` → unknown name. The op is **`length`** (`OP_LENGTH`), and only in
  filter position — see §5 on the head-position trap.
- `format("{} {}", a, b)` → unknown name. There is no `format`; use `++` for
  concatenation, `t2s` for a timestamp, or `from_substrings` to build one.
- `n ~= "da"` does not parse, because no such operator exists. The real
  substring predicate is **`str_includes(s, sub)`** — it matches anywhere in the
  string, not just the prefix, so it subsumes the `starts_with` advice this
  document used to give. It is **case-sensitive**: wrap in `lowercase()` for a
  case-insensitive search. For regex, `regex_matches` / `regex_replace` /
  `regex_extract` are all present.
- Bare `~` is the **coalesce** operator (`op_coalesce` in the grammar), not a
  contains test. It substitutes its right operand for a null left one, so
  `name ~ "fallback" == "Ada"` quietly *matches* — a reader expecting a
  substring test sees a hit and concludes the operator works. This is the trap
  that made `~=` look like a missing feature.
- `if a >= 40 then "x" else "y"` does not parse. Conditionals exist only in
  the imperative form `%if cond %then ... %else ... %end` inside `{ }` blocks
  (`imperative_script`), which is a much larger surface than an expression.
- Works today: `++`, `to_string`, `to_int`, `starts_with`, `ends_with`,
  `lowercase`, `uppercase`, `trim`, `length`, `str_includes`, `slice_string`,
  `chars`, `regex_matches`, `round`, `in`, `not`, `or`, `&&`, `||`, `^`.

## 7. `::` system ops — the allow-list, and read-only mode
- The grammar's `sys_script` rule enumerates every valid op (cozoscript.pest
  lines 14-19). There is **no `::headers` and no `::lte`** — both exist upstream
  and both fail here with `parser::pest`, reporting a span near the `::` you just
  typed (`::headers` reports offset 2), which sends you looking at the wrong
  character. You do not need `::headers`: `headers` is already in every
  successful result.
- With `immutable=true` (read-only), every mutating op returns
  `Cannot <do X> in read-only mode` — including `::describe`, `::index create`,
  `::access_level`, `::compact`, `::remove`, `::rename`, and `::query create`.
  This is a clean error, not a crash, but it means "works in read-only" and
  "works at all" are different questions: always probe DDL with
  `immutable=false`.
- `::history rel [[1], [2]]` needs a `TxTime`-typed relation; on an ordinary
  relation it returns `::history requires a TxTime relation`.
- `::index fts create person:by_name {name}` did not parse. The grammar
  (`fts_idx_op = {"fts" ~ (index_create_adv | index_drop)}`) suggests it should,
  so this one is unresolved rather than known-broken — treat FTS index creation
  as untested on wasm.

## 8. Two constructs panic instead of erroring — the dangerous ones
- `::query create adults { ... }` and
  `?[n, c, p, d] <~ BudgetedTraversal(*e[f, t, w], s[n], max_nodes: 10)` both
  abort with `panicked at .../unsupported.rs: time not implemented on this
  platform`, then `RuntimeError: unreachable` in JS.
- Cause: `std::time::SystemTime::now()` has no implementation on
  `wasm32-unknown-unknown`. Both paths stamp something with wall-clock time.
  Not fixed yet — see §12 for why.
- Why this is worse than a normal error: it is a **trap**, not a Result. The
  `RuntimeError` escapes whatever `await`/`try` you wrap it in, and the wasm
  instance is left in an undefined state — every later call on that `db` is
  suspect. Treat the whole `CozoDb` as disposable after one.
- Fix: avoid both. For graph hops, plain Datalog recursion is the portable form
  and runs fine on wasm:
  ```cozoscript
  start[n] <- [["Ada"]]
  r[n, d] := start[n], d = 0
  r[t, d1] := r[f, d], *edge{f, t, w}, d < 2, d1 = d + 1
  ?[n, min(d)] := r[n, d]
  ```
- Lesson: before putting any new query in a browser, run it once in Node
  against `pkg/` with the process-per-case harness from the verification method
  above. A panic costs you the whole page's database.

## 9. Graphs are projections over relations, not literal adjacency maps
- `::graph create :knows {...}` does not parse — position 15 is the `:`.
  The name is a plain `ident`: `::graph create g1 {edges: e}`. The body is
  `graph_opt_field = ident ~ ":" ~ expr`, i.e. a relation (plus optional
  `nodes:`), per `budgeted_traversal.rs`.
- After `::graph create g1 {edges: e}`, `::graph list` returns `[]` and
  `::graph drop g1` fails with `graph::projection_not_found`. So the registered
  object is a traversal *projection* bound at query time via
  `BudgetedTraversal(s[n], graph: 'g1', ...)` — which is itself one of the
  panicking constructs from §8. Net effect: graph objects are not usable from
  wasm today; use the recursion form above.
- Also note `?[] <~ BFS(person{name, age}, ["Ada"])` does not parse: `fixed_rel`
  wants a rule `ident[...]` or a relation `*ident[...]`.

## 10. Result and error shape
- `run()` returns a **JSON string**; parse it before use. Success:
  `{"ok":true,"headers":[...],"rows":[[...]],"next":null}`.
- Failure: `{"ok":false,"code":...,"message":...,"display":...,"severity":...,
  "labels":[],"related":[],"causes":[],"filename":""}`. There is **no `reason`
  field** (older Cozo examples use `reason`) — branch on `ok` and render
  `code` + `message`.
- `display` is miette-rendered and **contains ANSI colour escapes**; strip them
  or render `message` instead, or the user sees raw escape codes.
- `code` is the useful part to branch on (`query::relation_not_found`,
  `eval::rule_arity_mismatch`, `parser::pest`, `eval::no_implementation`, …).

## 11. JS API and the `init()` contract
- The default export is the async initialiser; `initSync` is also exported.
  `await init()` must complete before anything else touches the module.
- With no argument, `init()` resolves the binary as
  `new URL('cozo_lib_wasm_bg.wasm', import.meta.url)` and `fetch`es it. It also
  accepts a `Uint8Array`, `Response`, `Request`, `URL` or path string —
  `await init({ module_or_path: bytes })` is the only form that works under
  Node, because `fetch` refuses `file:` URLs. This is what
  `tests/integration.test.mjs` does, and why it is a Node test, not a browser
  one.
- Therefore the page must be served over HTTP; opening `index.html` from disk
  fails. Under Vite the `new URL(..., import.meta.url)` lookup only survives if
  the package is kept out of the dep pre-bundler — see
  `examples/wasm-web-demo/vite.config.js`.
- After `init()`, any number of `CozoDb.new()` instances are allowed. There is
  no `open`/persistence API in the wasm wrapper: `CozoDb.new()` is hardcoded to
  `DbInstance::new("mem", "", "")`, so the database is memory-only and dies with
  the page. `export_relations` / `import_relations` (JSON strings) are the way to
  move a database in and out — e.g. to `localStorage`.
- The whole API is synchronous and runs on the main thread. A long script freezes
  the page. A Web Worker works, but see the caveat in the crate README about
  module support in workers across browsers.
- The generated `package.json` `files` array omits `cozo_lib_wasm_bg.js`. That
  file is only needed for a `--target no-modules` build, so this is harmless for
  `--target web` consumers — but it is why the artifact looks one file short.

## 12. Not verified
- The bundler/browser wiring itself. Everything in §1-§10 was executed through
  Node against `pkg/`; the Vite path in `examples/wasm-web-demo/` is reasoned
  from the glue source, not run. Enabling the smoke test that is commented out
  in `.github/workflows/wasm-release.yml` would close that gap in CI.
- FTS / vector / LSH index creation and the full-text query surface.
- Anything relying on more memory than a browser tab will give it.
- The two §8 panics were located but not fixed. They are `SystemTime::now()`
  hitting `unsupported.rs` on `wasm32-unknown-unknown`, which the crate already
  works around elsewhere by cfg-gating the feature out
  (`#[cfg(not(target_arch = "wasm32"))]` around the `:sleep` option in
  `runtime/db.rs`). The same treatment is the obvious mitigation, but it
  changes behaviour on the host target, so it is left as a deliberate decision
  rather than slipped in here. Until then, avoid both constructs and treat a
  `CozoDb` as disposable after any `RuntimeError`.
