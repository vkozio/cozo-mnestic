# wasm build: audit, corrected dialect notes, and capability gaps

Companion to `wasm-build-notes.md` (producing the artifact) and
`wasm-script-notes.md` (running it). This document records an audit of the
CozoScript dialect as exposed to `wasm32-unknown-unknown`, the corrections
applied to `wasm-script-notes.md` as a result, one engine bug found and fixed
along the way, and a survey of what the wasm build still cannot do.

**Scope and date.** `cozo-lib-wasm` 0.18.0 / `mnestic` 0.18.0, audited against
the working tree at `main` = `4baa95e1`. Findings are referenced to
`file:line` so they can be re-checked as the code moves.

**How to read the confidence tags.** Everything tagged *confirmed* was either
reproduced by a test in this tree or read off a single unambiguous code path.
*Inferred* means the code supports the claim but it was not executed on a real
wasm runtime in this environment. *Unverified* means I found the absence but
could not establish the cause. These are not interchangeable — the original
`wasm-script-notes.md` lost exactly that distinction, which is how three of its
diagnoses ended up wrong.

---

## 1. Summary

Five things worth knowing before the detail:

1. **One real engine bug, found and fixed.** A script containing two relation
   options (`:create` + `:put`) silently dropped the first. Fixed in
   `parse/query.rs`; regression tests added.
2. **The premise under three sections of the existing notes was false.**
   "The op registry is built per target and is not wired up in the wasm build."
   Both registries are static, target-independent name tables. This one wrong
   premise produced three wrong diagnoses.
3. **The wasm notes contained three incorrect claims** about available ops
   (`avg`, `sum`, `len`, `~=`). The working forms are `mean`, `sum`, `length`,
   `str_includes`. Two of the four were prescribed away in the notes in favour
   of a worse workaround or a JS reimplementation.
4. **The largest wasm gap is undocumented, silent, and hits the fork's headline
   feature.** The query budget — per-call `timeout` and `:timeout` — is a
   **no-op on wasm**, because there is no monotonic `Instant`. It is not in
   `wasm-script-notes.md` at all. A runaway query freezes the tab rather than
   returning `eval::timeout`.
5. **One of the two documented panics has a documented cause that the code
   contradicts.** `::query create` does panic on an unguarded `SystemTime` (*confirmed*).
   `BudgetedTraversal` does **not** touch the clock anywhere; the "both paths
   stamp wall-clock time" explanation is *unverified and probably wrong*.

---

## 2. Method

The grammar (`cozo-core/src/cozoscript.pest`) is the contract for anything
parse-level, and it settled every "why won't this parse" question in one read.
Behavioural questions went to the source, then to tests.

Three things shaped the approach:

- **Distinguish "not in this build" from "not a name."** `eval::no_implementation`
  is raised in exactly one place — `Expr::UnboundApply`, an unrecognised name
  (`data/expr.rs:247-251`, reached from six call sites in `expr.rs` plus
  `parse/expr.rs:110`). It carries no target information whatsoever. Any
  conclusion of the form "X is unavailable on wasm" derived from it is unsound.
- **Read the registries directly.** `get_op` (`data/expr.rs:794`) and
  `parse_aggr` (`data/aggr.rs:1618`) are plain `match name { ... }` over
  `&'static` values with no `cfg` in either. What is in them, is in them on
  every platform.
- **Execute the claim, don't reason about it.** Several apparent findings turned
  out wrong on contact with a test — see §6.3.

**Limits of this environment.** No network, and no wasm runtime, so §7 is
derived from source rather than from a browser. Script-level behaviour (§3-§6)
was verified end-to-end on the host target, which is valid because the relevant
code is target-independent — that independence is the point of §3.1. §7's
wasm-specific claims are marked accordingly.

---

## 3. Corrections to `wasm-script-notes.md`

### 3.1 The load-bearing error: the registries are not per-target

The notes asserted, for both aggregates and string ops:

> The op registry is built per target and `avg`/`sum` are not wired up in the
> wasm build.

This is false, and it is the single most consequential finding, because it is
load-bearing for three separate sections. Both registries are static:

- `get_op` — `cozo-core/src/data/expr.rs:794`, a `match` over ~180 string
  names. No `cfg`, no target parameter.
- `parse_aggr` — `cozo-core/src/data/aggr.rs:1618`, same shape, 29 names.

Consequences: a name missing from them is missing on *every* platform, and
`eval::no_implementation` means "no such name", full stop. The prescriptive
harm was concrete — the notes told wasm users to compute averages in JavaScript
when the engine has a first-class aggregate for it.

### 3.2 §5 — aggregates: `mean`, not `avg`

| Notes claimed | Actual | Evidence |
|---|---|---|
| `avg` unimplemented | the aggregate is `mean` (`AGGR_MEAN`) | `aggr.rs:1634` |
| `sum` unimplemented | `sum` → `AGGR_SUM`, works | `aggr.rs:1630` |
| only `count`/`min`/`max` | 29 aggregates available | `aggr.rs:1618-1650` |

`avg` looks plausible because it is the **Cypher** spelling, and the fork's
Cypher frontend translates it to `mean` on the way in
(`cypher/translate.rs:984-999`). That is almost certainly how it got into the
notes.

Full verified set: `and`, `or`, `unique`, `group_count`, `union`,
`intersection`, `count`, `count_unique`, `variance`, `std_dev`, `sum`,
`product`, `min`, `max`, `mean`, `choice`, `collect`, `interval_coalesce`,
`shortest`, `min_cost`, `bit_and`, `bit_or`, `bit_xor`, `latest_by`,
`smallest_by`, `choice_rand`, `min_cost_k`, `pareto_min`, `pareto_max`.

**Head-position trap (new, not in the original notes).** `head_arg = aggr_arg | var`
(`cozoscript.pest:94-95`) means `foo(x)` in a rule head is always read as an
*aggregate*. So `?[length(name)] := ...` fails with `parser::aggr_not_found`
("Aggregation 'length' not found") — a **different** error from the unknown-op
one, and easy to misread as "that op is missing" when the op exists and works
fine one position over. Filter position is where `length` belongs.

### 3.3 §6 — string ops: wrong names, and `~` is not "contains"

| Notes said | Actual |
|---|---|
| `len(n)` unimplemented | the op is `length` (`OP_LENGTH`, `expr.rs:876`) |
| no substring operator; use `starts_with` | `str_includes` (`expr.rs:855`) matches anywhere |
| `~=` does not parse | correct — no such operator |
| — | bare `~` is **coalesce** (`op_coalesce`, `cozoscript.pest:148`) |

The `~` point is the subtle one, and it is the likely reason `~=` was recorded
as a missing feature. `~` is not a prefix of a contains-operator; it is a
value-substitution operator. Written as `name ~ "fallback" == "Ada"` it parses,
evaluates, and **matches** — it returns the left operand when non-null. A
reader probing for `~=` gets a non-error that reads like a successful search.
That is worse than a parse failure, and it is exactly the failure mode that
produced the wrong note.

Additional facts, all *confirmed* by test and not in the original notes:
`str_includes` is **case-sensitive** (wrap in `lowercase()` for
case-insensitive matching); regex ops `regex_matches` / `regex_replace` /
`regex_extract` are all present, as are `ends_with`, `slice_string`, `chars`,
`t2s`, `from_substrings`. There is no `format`; use `++`, `t2s`, or
`from_substrings`.

### 3.4 §1 — the silent DDL drop was a parser bug, not transaction visibility

Original diagnosis:

> the two statements share one transaction, and the relation's metadata is not
> visible to the DML that creates it

Not the mechanism. The actual one, in `parse/query.rs`, was that
`stored_relation` — a local — was **reassigned** on every `relation_option`
it encountered, with no duplicate check. `out_opts.store_relation` is a single
`Option`, so a program can only carry one. The second option won; the
`:create` was discarded without a diagnostic and the `:put` inherited the
blame. The observed error, `query::relation_not_found`, is exactly what a
`:put` into a never-created relation produces.

The transaction was never involved. `run_query` creates the relation in the
same tx, before running the plan (`runtime/db.rs` ~3525) — had the metadata
survived parsing, this would have worked.

The instructive detail is that the fix was already in the file. The neighbouring
`assert_none_option` / `assert_some_option` arms reject duplicates via
`DuplicateQueryAssertion`; the `relation_option` arm simply never got the same
guard. See §4.

### 3.5 Verified correct, left untouched

Checked against grammar and source, found accurate, not modified: §2
(positional vs. named apply, `cozoscript.pest:111-112`), §3 (no `|` in the
grammar; `sort_dir` precedes the variable, `pest:165`), §4 (`=` unifies, `==`
compares, `pest:118` vs `:141`), §7 (`::headers`/`::lte` genuinely absent from
`sys_script`, `pest:14-19`), §9 (graph projection shape), §10 (result and error
shape), §11 (the `init()` contract).

---

## 4. Engine bug fixed

**Symptom.** `:create person {...}` and a `:put person {...}` in one
`run_script` produced `query::relation_not_found: Cannot find requested stored
relation 'person'`, with the `:create` apparently succeeding. The error lands on
the read-back, pointing the reader at the wrong statement.

**Fix.** `cozo-core/src/parse/query.rs` — reject a second relation option with
a labelled diagnostic, mirroring the existing assertion-duplicate pattern:

```
parser::multiple_store_relation
"A query can only have one stored relation"
  help: Each script may contain at most one relation option (`:create`,
        `:put`, `:rm`, …). Run the `:create` in its own `run()` call, then
        the DML in a second.
```

**Tests.** `cozo-core/tests/fork_regressions.rs`:

- `duplicate_relation_option_is_rejected` — reproduces the original failure mode,
  asserts the error names the real problem, and asserts the rejected script
  created nothing. Fails on the pre-fix code with the misleading
  `relation_not_found` (verified by running it before the fix).
- `single_relation_option_still_accepted` — guards against over-correcting the
  fix into a blanket rejection.

This is a general parser fix, not a wasm-specific one. Every binding and every
host build had the behaviour.

**Commits.** `34036499` (fix + regression tests), `3c720324` (notes correction
+ the new op-name suite).

---

## 5. New test suite: `cozo-core/tests/script_op_names.rs`

The corrections in §3 are only durable if the spellings are pinned. Seven tests,
all executed on every PR:

- `average_aggregate_is_spelled_mean` — `mean`, not `avg`
- `sum_aggregate_works` — guards the false "unimplemented" claim
- `other_aggregates_are_available` — `count`/`min`/`max`/`std_dev`/`collect`
- `string_length_is_spelled_length` — `length` in filter position, and the
  head-position `aggr_not_found` trap
- `substring_predicate_is_str_includes` — interior match beats `starts_with`;
  case-sensitivity; `lowercase()` composition
- `bare_tilde_is_coalesce_not_contains` — the pass-through behaviour that makes
  `~` look like a working contains-test
- `unknown_op_name_reports_no_implementation` — pins the meaning of
  `no_implementation` itself

---

## 6. What I got wrong along the way

Recorded because the failures are instructive and the notes' own errors came
from the same place — asserting engine behaviour from source reading without
executing it.

- **Four wrong assertions in my own first draft of the suite**, all caught by
  running it: used `length` in head position; searched `"york"` in a name
  column that does not contain it; asserted `length(name) == 8` for
  `"Edsger"` (6 characters); and expected a type error from `(name ~ "da")`
  that does not occur, because coalesce on a non-null string simply returns it.
- **`str_includes` case-sensitivity was a guess.** I assumed a case-insensitive
  match. It is case-sensitive; `"york"` does not match `"New York"`, `"York"`
  does. Found by a scratch probe, not by reading `OP_STR_INCLUDES`.
- **The `BudgetedTraversal` cause in the notes is not supported by the code**
  (§7.2). I repeated it in my first analysis before checking.

The general lesson, and the one the original notes most needed: `~` failing
quietly, `str_includes` being case-sensitive, and `no_implementation` meaning
"unknown name" are all **silent** behaviours. Silent ones are exactly the class
that source reading gets wrong.

---

## 7. Capability gaps in the wasm build

### 7.0 First, the thing that is not a gap

Time mostly **works** on wasm, via systematic `js_sys::Date` fallbacks:

| Function | Site |
|---|---|
| `op_now` | `data/functions.rs:2437` |
| `current_validity` (bitemporal) | `data/functions.rs:2453` |
| `wall_clock_micros` | `runtime/tt_clock.rs:46-59` |
| `seconds_since_the_epoch` | `runtime/db.rs:4412` |
| `op_rand_uuid_v1` | `data/functions.rs:2921` |
| `op_rand_ulid` | `data/functions.rs:3023` |

The fork has already done the porting work. The gaps below are holes in it, not
a platform that cannot do time. That framing matters for effort estimation: most
of these are one-liners against an existing pattern.

### 7.1 Panics — traps, not errors

**`::query create` — confirmed.** `runtime/stored_queries.rs:669` calls
`SystemTime::now()` with no `cfg` guard. This is the only unguarded wall-clock
read I found in a query-execution path. Cheap fix: route it through
`seconds_since_the_epoch()`, which already has the correct wasm branch.

**`BudgetedTraversal` — cause unverified, doc's claim probably wrong.**
`fixed_rule/algos/budgeted_traversal.rs` contains **no** time call at all
(zero matches for `Instant`, `SystemTime`, `now()`). The notes' explanation —
that both panicking paths stamp wall-clock time — therefore does not hold for
this one. The build reaches `graph_input()` → `build_weighted_csr` /
`build_unweighted_csr` (`fixed_rule/mod.rs:303,356`), and the `graph` crate's
CSR builder pulls rayon, which `graph-algo` enables on wasm
(`cozo-lib-wasm/Cargo.toml:20`). **Inferred, not established.** Reproduce
before attempting a fix; do not implement the notes' theory.

The crate already has the right pattern for this class: `#[cfg(not(target_arch
= "wasm32"))]` around the `thread::sleep` in `execute_single_program`
(`runtime/db.rs:2048`), and an explicit `bail!(":sleep is not supported under
WASM")` at parse time (`parse/query.rs:316`).

### 7.2 Absent API — removed at compile time

- **`GovernedTransaction`** — the entire type is
  `#[cfg(not(target_arch = "wasm32"))]` (`lib.rs:127`), taking the fork's
  admission control and cancellation ownership with it.
- **`multi_transaction`** — spawns a thread (`lib.rs:1109`); unavailable on
  `wasm32-unknown-unknown`. `::kill` rides on the same machinery, so it is
  unavailable too.
- **`register_callback` / `unregister_callback`** — `#[cfg]`-ed out
  (`runtime/db.rs:1873`).

### 7.3 Silent degradation — the dangerous category

- **The query budget does not exist on wasm.** `budget_now()` returns `None`
  under `target_arch = "wasm32"` (`runtime/db.rs:4304-4313`) because there is
  no monotonic `Instant`. Consequences: `:timeout` is a no-op, and the per-call
  `ScriptRunOptions::timeout` never engages. The whole-script wall-clock
  deadline that the fork's query-budget work is built around is **absent**.
  This is the fork's headline feature and it is not in the notes at all.
  Worse, it fails silently: a runaway query hangs the tab instead of returning
  `eval::timeout`.
- **Triggers are accepted and never fire.** `current_callback_targets` returns
  `Default::default()` on wasm (`runtime/callback.rs:61-75`). A `::set_triggers`
  script parses, succeeds, and does nothing — no error, no execution. Silent
  acceptance of a no-op is the worst failure shape available.
- **`took` is dropped from the result JSON** (`lib.rs:597,625`) under
  `#[cfg(not(target_arch = "wasm32"))]`. The one signal that would let a caller
  detect the problem in 7.3 is itself absent on the target where it is needed.

### 7.4 Degraded performance

- Parallel stratum evaluation falls back to sequential on wasm
  (`query/eval.rs:296`).
- FTS and HNSW index builds use OS threads — `runtime/relation.rs:1487` and
  `runtime/hnsw_build.rs:260` both call `std::thread::scope`, and
  `build_threads` consults `available_parallelism()`. On
  `wasm32-unknown-unknown` these either panic or collapse to one thread.
  *Inferred* — needs a wasm run to classify.

### 7.5 By design, not a defect

- **Storage is `mem` only.** `DbInstance::new("mem", "", "")` is hardcoded in
  `cozo-lib-wasm/src/lib.rs:30`. There is no `open`/persistence API;
  `export_relations` / `import_relations` is the only way to persist.
- **Synchronous, main-thread API.** A long script blocks the page. Combined
  with 7.3, this is an uncancellable freeze rather than a slow query. A Web
  Worker is the mitigation, with the cross-browser module caveat already noted
  in the crate README.

### 7.6 Feature flags

`cozo-lib-wasm/Cargo.toml:20` resolves the core crate as
`default-features = false, features = ["wasm", "graph-algo"]`, while the core
default is `["compact", "fts-cangjie"]` (`cozo-core/Cargo.toml`). Dropped:

- `fts-cangjie` — no Cangjie/jieba tokenizer for Chinese FTS. FTS itself still
  works; only that tokenizer is missing.
- `storage-sqlite` / `storage-sqlite-src` (pulled in by `compact`)
- `requests`, and the `data-import` / `rdf-io` / `columnar-io` / `cypher`
  surfaces, none of which are default-on anyway.

---

## 8. Recommendations, ranked by risk-to-effort

1. **Restore or explicitly refuse the query budget on wasm** (§7.3). Highest
   value, and currently invisible to users. Needs a monotonic time source
   (`performance.now()` via `web-sys` is the usual route) — that is an
   environment question code reading cannot answer. If it is not worth
   implementing, the next-best thing is a loud `bail!` at parse time, matching
   the existing `:sleep` pattern, so a runaway query fails fast instead of
   freezing the tab.
2. **Guard `runtime/stored_queries.rs:669`** (§7.1). One line, unambiguous, and
   it removes a trap that corrupts the instance. Same for any other unguarded
   `SystemTime::now()` — grep is in §9.
3. **Make triggers fail loudly on wasm** (§7.3). Silent acceptance of a no-op
   is worse than absence. A `bail!` when `::set_triggers` is used on wasm costs
   a few lines.
4. **Reproduce the `BudgetedTraversal` panic** before fixing (§7.1). The
   documented cause is not supported by the code; fixing to the documented cause
   would likely not fix it.
5. **Restore `took` on wasm** (§7.3). Cheap, and it is the only observable
   signal for item 1.

---

## 9. Verification commands and reference index

Tests added/changed:

```
cargo test -p mnestic --test fork_regressions    # 8 tests, 7 pass, 1 ignored
cargo test -p mnestic --test script_op_names      # 7 tests, all pass
cargo clippy -p mnestic --tests                  # clean
cargo fmt -p mnestic                             # no changes
```

A full `cargo test -p mnestic` run after the parser fix reported 60 test
binaries passing with no failures. The consolidated run including
`script_op_names` was interrupted before completion; that file was verified on
its own. **Re-run the full suite before relying on that number.**

Note the package is `mnestic`, with lib name `cozo` — `-p cozo-core` fails.

Key sites referenced throughout:

| Concern | Location |
|---|---|
| Grammar (the contract) | `cozo-core/src/cozoscript.pest` |
| Expression op registry | `cozo-core/src/data/expr.rs:794` |
| Aggregate registry | `cozo-core/src/data/aggr.rs:1618` |
| `no_implementation` origin | `cozo-core/src/data/expr.rs:247-251` |
| Relation-option overwrite (fixed) | `cozo-core/src/parse/query.rs:377` |
| Duplicate-assertion precedent | `cozo-core/src/parse/query.rs` (`DuplicateQueryAssertion`) |
| Query budget clock | `cozo-core/src/runtime/db.rs:4304-4313` |
| Unguarded wall clock | `cozo-core/src/runtime/stored_queries.rs:669` |
| wasm callbacks no-op | `cozo-core/src/runtime/callback.rs:61-75` |
| wasm-visible API surface | `cozo-core/src/lib.rs:127,597,1109` |
| wasm shim (mem-only storage) | `cozo-lib-wasm/src/lib.rs:30` |
| wasm feature selection | `cozo-lib-wasm/Cargo.toml:20` |

To re-audit the time situation after any change:

```
grep -rn 'SystemTime::now' cozo-core/src --include=*.rs
```

Every hit must be either `#[cfg]`-guarded or routed through
`seconds_since_the_epoch()` / `wall_clock_micros()`.

---

## 10. Open questions

- Does the `BudgetedTraversal` panic reproduce, and from what? (§7.1)
- Do the FTS/HNSW threaded builds panic or silently serialise on wasm? (§7.4)
- Is there a monotonic clock available in the target wasm environment, and is
  it worth wiring the budget to it? (§8.1)
- FTS/vector/LSH index creation on wasm remains untested — the grammar permits
  `::index fts create` (`pest:22`), which contradicts the original note that
  it "did not parse". Unresolved, and possibly another case of a misattributed
  parse error.
- Browser/bundler wiring (Vite, `import.meta.url` resolution) is still reasoned
  from source, not executed. The smoke test commented out in
  `.github/workflows/wasm-release.yml` would close that.
