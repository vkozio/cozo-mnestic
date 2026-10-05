# Spec — CozoScript surface: query options, builtins, and the four rules that generate the wrong answer

_Created 2026-10-05. Status: **reference documentation — no engine change in this document**; the surface it describes is shipped (0.18.0). Origin: six items in an external fork-issues report (`xag-ts`, 2026-10-04) plus the analysis note written in response to it. **Four of those six were syntax mistakes against features that already exist**, and the analysis note's P0/P1/P2 ordering would have had the fork implement pipeline operators, a `string::` namespace and a `~=` operator — none of which are missing. The one item that is a genuine engine hazard (projecting a subset of columns silently returns the set of distinct tuples, not stored rows) is §3 and is the reason this document exists in this form. Companion reading: [`cozoscript-extensions.md`](cozoscript-extensions.md) (the other CozoScript-surface spec; its §3.2 records the `:put`/const-rule idioms this one assumes), [`bitemporality.md`](bitemporality.md) §4 (`:as_of`, which appears in §2), [`string-literals.md`](string-literals.md)._

> **Anti-overbuild guardrails (apply throughout).** This spec **builds nothing**. It is a **reference**, and every rule in it is one that already holds in the engine; the deliverable for the four syntax items is *this page*, not a feature. Where the analysis note proposed new syntax, this document records the existing spelling instead. Where a hazard is real (§3), the deliverable is a warning and the two aggregate functions that avoid it — again not new engine work. The tables in §2, §5, §6.1 and §6.2 are **generated from live `::builtins` output and diffed against it**, not transcribed from the source tables, so they cannot drift silently; §10 states how, and the enumeration itself is now available to any script (§5). Every example query below was executed against a live `redb` store on 2026-10-05; where one is shown failing, the message is quoted verbatim from that run.

---

## 1. Why this document exists

The fork reported six problems against a 0.18.0 evaluator. Verification against the engine separated them cleanly:

| # | Reported as | What it actually is | Where |
|---|---|---|---|
| 1 | "no way to limit/sort/offset results — the `::limit` operator is missing" | **Syntax.** Result bounding is trailing **single-colon** options. There is no `::limit`, and there never was: `::` is a sysop and imperative-block prefix only | §4 |
| 2 | "there is no `string::` namespace, so string functions are missing" | **Syntax.** Builtins are a **flat** namespace of bare names. `contains` never existed; the function is `str_includes` | §6 |
| 3 | "no regex match operator — `~=` is undocumented" | **Syntax.** There is no `~=`. `~` is `coalesce`. Pattern matching is `regex_matches(s, "pat")`; for indexed search it is `::fts create` + `~idx{…}` | §7 |
| 3b | "constant relations are broken — `l[s] := [...]` fails" | **Syntax.** The head of a constant rule takes **brackets** (`<-`, not `:=`), and the body is a list of lists | §8 |
| 4 | *(not reported as a bug — but this is the one that is real)* | **A silent wrong answer.** Projecting a subset of columns returns the **set of distinct tuples**, not stored rows. `?[col] := *rel[…]` is not a row count and does not say so | **§3** |
| 5 | "relation patterns give an unhelpful arity error" | **Real, ergonomics.** Arity is required and `_` is the skip. The diagnostic now names the placeholder | §9 |
| 6 | *(not yet reported — anticipated)* | **Real, ergonomics.** `:sort`/`:order` validate against the **entry rule's output columns**, so you cannot sort by a column you did not project | §2.1 |

Items 1, 2, 3 and 3b are the same failure once: a construct existed, under a
different name or a different prefix, and there was no way to discover it from a
script. §5 is the fix for the class. Item 4 is the opposite failure and is the
only one that produced a wrong number rather than an error message.

## 2. Query options

Twenty-two trailing options, listed exactly as `::builtins options` reports
them. **The two you are looking for are the first four**, and they are the
result-shaping set:

| Option | Description |
|---|---|
| `:limit` | cap the number of result rows |
| `:offset` | skip the first N result rows |
| `:sort` | order results by an output column; prefix the column with `-` for descending |
| `:order` | synonym of `:sort` |
| `:reorder` | join reorder mode: `greedy` (default) or `written` to opt out |
| `:timeout` | abort the query after N seconds |
| `:mem_limit` | abort if materialization exceeds N estimated bytes |
| `:as_of` | read at a given validity timestamp |
| `:sleep` | sleep N seconds before running the body |
| `:returning` | return the tuples written by the body |
| `:assert` | `none` asserts the body wrote nothing, `some` that it wrote something |
| `:disable_magic_rewrite` | switch off Datalog subquery rewriting for this script |
| `:create` | create the named relation, replacing an existing one |
| `:replace` | replace the contents of the named relation |
| `:reconcile` | recompute-based belief revision of the named relation |
| `:insert` | insert rows, failing on an existing key |
| `:put` | insert or replace rows (idempotent upsert) |
| `:update` | update existing rows |
| `:rm` | delete the listed keys |
| `:delete` | delete the listed tuples |
| `:ensure` | create the relation only if it does not exist |
| `:ensure_not` | fail if the relation exists |

All 22, in the order `::builtins options` reports them. The order is the
grammar's and carries no meaning; the order you write them in does not matter.

Options are **trailing** — they go at the end of the query, one per line, and
they compose:

```cozoscript
?[use_file, sym, line] := *sym_use[use_file, sym, line, _]
:sort line
:limit 2
:offset 1
```

Verified: returns `(src/a.ts, foo, 10)`, `(src/a.ts, foo, 20)`.

Descending is a `-` prefix on the column inside the option:

```cozoscript
?[use_file, sym, line] := *sym_use[use_file, sym, line, _]
:sort -line
```

Verified: `(src/a.ts, bar, 30)`, `(src/a.ts, foo, 20)`, `(src/a.ts, foo, 10)`,
`(src/b.ts, baz, 5)`.

### 2.1 `:sort` and `:order` validate against the *output* columns

**This is the next surprise, so it is stated before anything else in this
section.** `:sort` resolves its key against the entry rule's output columns
(`get_entry_out_head()`) — the columns in the `?[…]` head — **not** against the
columns of any relation the body read. A column you filtered on but did not
project cannot be sorted by.

Works:

```cozoscript
?[use_file, sym] := *sym_use[use_file, sym, line, _]
:sort sym
```

Fails, with `parser::sort_key_not_found`:

```cozoscript
?[use_file] := *sym_use[use_file, _, _, _]
:sort sym
```

```
Sort key 'sym' not found
```

`:order` is the same option under the other name and fails identically. The
fix is to project the column you want to sort by:

```cozoscript
?[use_file, sym] := *sym_use[use_file, sym, line, _]
:sort sym, use_file
```

Two consequences worth internalising. First, **adding a column to the `?[…]`
head is what makes it sortable**, even when you do not need the value — an
empty binding list `[use_file, _]` does not help. Second, the error message
does **not** distinguish "you read this column but did not project it" from "no
such column exists": `:sort sym` (column exists in `sym_use`, not in the output)
and `:sort nope` (column exists nowhere) produce the byte-identical
`Sort key 'nope' not found`. Recorded as a diagnostic gap, not fixed here — the
file is owned by another track.

### 2.2 Fixed: a bare `true` conjunct plus any query option returned nothing

Found while writing this document, **fixed** — root cause was the parser, not the planner. `boolean`/`null` were non-atomic pest rules, so implicit WHITESPACE extended `pair.as_str()` to `"true "` whenever the literal was not last; `parse/expr.rs` derives `Bool` from `as_str() == "true"`, so `true :limit 2` parsed as `false` and filtered every row silently (`ok:true`). Made atomic (`boolean = @{...}`, `null = @{...}` in `cozoscript.pest`); `null` never read its text but had the same inflated span. Sibling: `:limit 0` leaked one row via the early-return limiter (`QueryLimiter::should_skip_next` in `query/eval.rs`); `total == Some(0)` now skips every tuple. Pinned by `cozo-core/tests/true_conjunct.rs` (27 tests).

A `true` literal in a rule body is a legal conjunct and a no-op:

```cozoscript
?[k] := *r[k, _], true
```

returns the same rows as `?[k] := *r[k, _]`. **Before the fix, adding any trailing query option emptied the result:**

| Query | Rows |
|---|---|
| `?[k] := *r[k, _]` | 2 |
| `?[k] := *r[k, _], true` | 2 |
| `?[k] := *r[k, _], true` + `:sort k` | **0** |
| `?[k] := *r[k, _], true` + `:limit 5` | **0** |
| `?[k] := *r[k, _], true` + `:offset 0` | **0** |
| `?[k] := *r[k, _]` + `:sort k` | 2 |
| `?[k] := *r[k, _]` + `:limit 5` | 2 |

Reproduced on both the `redb` and `sqlite` backends on a two-row relation, so it
is not backend- or fixture-specific. It is **any** option, not just the
result-shaping ones (`:timeout` and `:mem_limit` also zero the result), and it
propagates through a helper rule — `t[k] := *r[k, _], true` with `?[k] := t[k]`
plus `:sort k` is also empty — while a `true` in an *unrelated* rule is harmless.
The aggregate form `?[count(k)] := *r[k, _], true` is unaffected.

Related and correct, for contrast: a non-boolean literal conjunct is rejected
loudly rather than silently emptying the result — `?[k] := *r[k, _], 1` gives
`eval::predicate_not_bool: Found value 1 where a boolean value is expected`. So
the engine does validate that conjunct; `true` specifically is mishandled once an
option is present.

**No workaround needed.** `, true` is a no-op by definition. No example here uses a `true` conjunct.

## 3. Set semantics — the one silent wrong answer

**Read this section before writing any query whose output you will count.**

A CozoScript relation result is a **set of tuples**. Evaluating
`?[use_file] := *sym_use[use_file, _, _, _]` projects each stored 4-tuple onto
its first column and then **de-duplicates**. The number of rows you get back is
the number of *distinct values in the projected column*, not the number of rows
in the relation. Nothing warns you. The query succeeds.

On the four-row fixture used throughout this document (`sym_use`: three rows in
`src/a.ts`, one in `src/b.ts`):

```cozoscript
?[use_file] := *sym_use[use_file, _, _, _]
```

Verified: **2 rows** — `(src/a.ts)`, `(src/b.ts)`. If you read that as "two
uses of symbols", or built a page count out of it, you are wrong by a factor of
two and the engine will never tell you.

The stored row count is an **aggregate**, and aggregates go in the rule head:

```cozoscript
?[count(line)] := *sym_use[use_file, sym, line, _]
```

Verified: **`4`** — the true number of stored rows. (`line` is a key column, so
the body tuple is unique per stored row, which is what makes `count` over it a
row count.) And the distinct-value count is `count_unique`:

```cozoscript
?[count_unique(use_file)] := *sym_use[use_file, sym, line, _]
```

Verified: **`2`** — which is what the projected query returned as its row count.

### 3.1 `count` versus `count_unique`

Same body, same grouping key, different question — this is the pair to reach for
whenever you are unsure which one you meant:

```cozoscript
?[use_file, count(sym), count_unique(sym)] := *sym_use[use_file, sym, line, _]
:sort use_file
```

Verified:

| `use_file` | `count(sym)` | `count_unique(sym)` |
|---|---|---|
| `src/a.ts` | 3 | 2 |
| `src/b.ts` | 1 | 1 |

`src/a.ts` has **three rows** but **two distinct symbols** (`foo` used twice at
lines 10 and 20, `bar` once at line 30). `count` counts rows; `count_unique`
counts distinct values of the aggregated column. **If you want "how many times
is this symbol referenced", that is `count`, and you must keep the row-unique
column in the body** — the `_` you wrote to skip it is the same `_` that makes
the answer collapse to 2.

Note the arithmetic does not help either: `?[sum(line)] := *sym_use[…]` → 65 is
a sum over distinct tuples too. Any function of a projected result is a function
of the **set**.

### 3.2 The rule

**To count stored rows, aggregate. To count distinct things, aggregate with
`count_unique`. To see rows, project all the columns that make them distinct.
Never infer a count from the length of a projected result.**

## 4. There are no pipeline operators

`::` is a **sysop prefix** (`::relation`, `::create`, `::fts`, `::builtins`, …)
and an **imperative-block prefix** (`{ … }`). That is the whole of its grammar
meaning. There is no `::limit`, `::sort`, `::order` or `::offset`, and there
never was — writing one is not a missing feature, it is a wrong prefix.

The engine says so, and names the option you were reaching for:

```cozoscript
?[use_file, sym, line] := *sym_use[use_file, sym, line, _]
::limit 2
```

```
`::limit` is not a pipeline operator: `::` prefixes a sysop or an imperative
block, and CozoScript has no pipeline operators. Query options are trailing and
single-colon — did you mean `:limit`?
```

When the thing after `::` is not an option name at all, the message names the
four options that shape results instead:

```cozoscript
?[use_file, sym, line] := *sym_use[use_file, sym, line, _]
::head 5
```

```
`::head` is not a pipeline operator: `::` prefixes a sysop or an imperative
block, and CozoScript has no pipeline operators. Its 22 query options are
trailing and single-colon — bound or order results with `:limit`, `:offset`,
`:sort`, `:order`.
```

A bare `::` with nothing after it gets the same second form, with the operator
spelled as `::`.

**Chaining is by newline, not by pipe.** CozoScript composes through rules and
through trailing options, never through a pipeline:

```cozoscript
?[use_file, sym, line] := *sym_use[use_file, sym, line, _]   # ends the query
:sort -line                                                  # an option
:limit 5                                                     # another option
```

## 5. Discovery — enumerate the surface from a script

Since 0.18.0 the whole enumerable language surface is listable at runtime. This
is the structural answer to §1: four of the six reported issues were a guessing
failure, and guessing is no longer necessary.

| Call | Columns | Rows (as shipped) |
|---|---|---|
| `::builtins` | `kind, name, min_arity, vararg, description, is_meet, is_bounded_meet, custom` | 209 |
| `::builtins all` | identical to bare `::builtins` | 209 |
| `::builtins ops` | `name, min_arity, vararg` | 158 |
| `::builtins options` | `name, description` | 22 |
| `::builtins aggrs` | `name, is_meet, is_bounded_meet, custom` | 29 |
| `::version` | `version` | 1 |

All counts above were read from a live run on 2026-10-05, not from the source.

- **Bare `::builtins` is exactly `::builtins all`.** It is not a redirect and not
  a subset: 209 = 158 + 22 + 29. One unconditional call discovers everything,
  which is the property that makes this usable from an agent.
- **`::builtins all` is the union with a `kind` discriminator.** A cell that does
  not apply to a row's kind is `null`, a real value — so `is null` is a usable
  predicate rather than a string comparison against `""`. Verified:

  | `kind` | sample row as reported |
  |---|---|
  | `ops` | `ops, coalesce, 0, true, null, null, null, null` |
  | `options` | `options, :limit, null, null, cap the number of result rows, null, null, null` |
  | `aggrs` | `aggrs, and, null, null, null, true, false, false` |

- **`::builtins aggrs` includes aggregates registered on *this* `Db`**
  (`custom` = true), so it answers a question about the live engine, not a
  compiled-in list. On a fresh database every `custom` is false.
- **`::version`** reports the compiled-in engine version — `0.18.0` here.
- **Both work inside an imperative block**: `{::builtins ops}`, `{::version}`.
- **A mistyped slice fails loudly** rather than returning nothing (silently
  returning zero rows is the worst possible answer to an introspection call — the
  caller concludes the construct does not exist). `::builtins bogus` gives
  `expected one of: \`aggrs\`, \`all\`, \`ops\`, \`options\``.
- **`::builtins ops` is in source-declaration order, not alphabetical.** Sort it
  yourself if you want a stable listing.

## 6. Builtin functions — one flat namespace

**There are no namespaces.** Every builtin is a bare name. `string::str_includes`
is not a spelling of anything; it is `str_includes`, and writing it produces the
§4 pipeline diagnostic (correctly — `::` really is the wrong prefix).

`contains` does not exist and never has. The substring test is **`str_includes`**:

```cozoscript
?[v] := v = contains("abc", "b")
```

```
eval::no_implementation: No implementation found for op `contains`
```

```cozoscript
?[v] := v = str_includes("abc", "b")
```

Verified: `True` (and `str_includes("abc", "z")` → `False`).

Worth knowing that the two halves of this mistake fail **differently**: an
unknown bare name is a *compile/eval* error (`no_implementation`), while a
`::`-prefixed name is a *parse* error (§4). Same confusion, two codes.

The other four names from the report all exist, bare, unprefixed:

```cozoscript
?[a, b, c, d] := a = starts_with("src/a.ts", "src/"),
                b = ends_with("src/a.ts", ".ts"),
                c = lowercase("ABC"),
                d = uppercase("abc")
```

Verified: `(true, true, "abc", "ABC")`.

### 6.1 The whole namespace, grouped by what a reader is looking for

All 158 rows, mechanically generated from `::builtins ops` and diffed against it
(§10). `min` is the minimum arity; `vararg` means more arguments are accepted.
There is **no `string::` prefix on anything in this table** — that is the point
of it.

#### String

| Name | min | vararg |
|---|---|---|
| `chars` | 1 | no |
| `decode_base64` | 1 | no |
| `encode_base64` | 1 | no |
| `ends_with` | 2 | no |
| `from_substrings` | 1 | no |
| `length` | 1 | no |
| `lowercase` | 1 | no |
| `slice_string` | 3 | no |
| `starts_with` | 2 | no |
| `str_includes` | 2 | no |
| `t2s` | 1 | no |
| `trim` | 1 | no |
| `trim_end` | 1 | no |
| `trim_start` | 1 | no |
| `unicode_normalize` | 2 | no |
| `uppercase` | 1 | no |

`length` works on a string or a list. `chars("abc")` → `["a","b","c"]`.
`from_substrings(["a","b","c"])` → `"abc"` (it joins a **list of strings** — it
does not split). `slice_string("hello", 1, 3)` → `"el"`. `t2s` is traditional →
simplified Chinese conversion and passes non-strings through unchanged.

#### Regex

| Name | min | vararg |
|---|---|---|
| `regex_extract` | 2 | no |
| `regex_extract_first` | 2 | no |
| `regex_matches` | 2 | no |
| `regex_replace` | 3 | no |
| `regex_replace_all` | 3 | no |
| `snippet` | 3 | **yes** |

Covered in §7.

#### Numeric

| Name | min | vararg |
|---|---|---|
| `abs` | 1 | no |
| `acos` | 1 | no |
| `acosh` | 1 | no |
| `asin` | 1 | no |
| `asinh` | 1 | no |
| `atan` | 1 | no |
| `atan2` | 2 | no |
| `atanh` | 1 | no |
| `ceil` | 1 | no |
| `cos` | 1 | no |
| `cosh` | 1 | no |
| `deg_to_rad` | 1 | no |
| `div` | 2 | no |
| `exp` | 1 | no |
| `exp2` | 1 | no |
| `floor` | 1 | no |
| `haversine` | 4 | no |
| `haversine_deg_input` | 4 | no |
| `ip_dist` | 2 | no |
| `l2_dist` | 2 | no |
| `ln` | 1 | no |
| `log10` | 1 | no |
| `log2` | 1 | no |
| `minus` | 1 | no |
| `mod` | 2 | no |
| `mul` | 0 | **yes** |
| `pow` | 2 | no |
| `rad_to_deg` | 1 | no |
| `round` | 1 | no |
| `signum` | 1 | no |
| `sin` | 1 | no |
| `sinh` | 1 | no |
| `sqrt` | 1 | no |
| `sub` | 2 | no |
| `tan` | 1 | no |
| `tanh` | 1 | no |

#### Collections

| Name | min | vararg |
|---|---|---|
| `append` | 2 | no |
| `chunks` | 2 | no |
| `chunks_exact` | 2 | no |
| `concat` | 1 | **yes** |
| `difference` | 2 | **yes** |
| `first` | 1 | no |
| `int_range` | 1 | **yes** |
| `intersection` | 1 | **yes** |
| `interval_overlaps` | 2 | no |
| `is_in` | 2 | no |
| `last` | 1 | no |
| `list` | 0 | **yes** |
| `max` | 1 | **yes** |
| `min` | 1 | **yes** |
| `pack_bits` | 1 | no |
| `prepend` | 2 | no |
| `reverse` | 1 | no |
| `slice` | 3 | no |
| `sorted` | 1 | no |
| `union` | 1 | **yes** |
| `unpack_bits` | 1 | no |
| `windows` | 2 | no |

`reverse`, `chunks`, `chunks_exact` and `windows` are **list-only** — passing a
string gives `'reverse' requires lists` / `first argument of 'windows' must be a
list`. Use `slice_string` / `chars` for text. `concat("a","b","c")` → `"abc"` is
the string join. `pack_bits` takes a list of **booleans** and returns bytes;
`unpack_bits` takes bytes. `interval_overlaps` is half-open, so touching spans
do **not** overlap: `interval_overlaps([0,5],[5,10])` → `False`.

#### Type predicates and conversions

| Name | min | vararg |
|---|---|---|
| `is_bytes` | 1 | no |
| `is_finite` | 1 | no |
| `is_float` | 1 | no |
| `is_infinite` | 1 | no |
| `is_int` | 1 | no |
| `is_json` | 1 | no |
| `is_list` | 1 | no |
| `is_nan` | 1 | no |
| `is_null` | 1 | no |
| `is_num` | 1 | no |
| `is_string` | 1 | no |
| `is_uuid` | 1 | no |
| `is_vec` | 1 | no |
| `to_bool` | 1 | no |
| `to_float` | 1 | no |
| `to_int` | 1 | no |
| `to_string` | 1 | no |
| `to_uuid` | 1 | no |

#### Vector

| Name | min | vararg |
|---|---|---|
| `cos_dist` | 2 | no |
| `l2_normalize` | 1 | no |
| `rand_vec` | 1 | **yes** |
| `to_unity` | 1 | no |
| `vec` | 1 | **yes** |

`vec` builds a vector from a **Json array** — `vec([1.0, 2.0])` — and takes an
optional second argument naming the element type (`"F32"`, `"Float"`, `"F64"`,
`"Double"`; default `F32`). It does **not** take a variadic list of numbers:
`vec(1.0, 2.0)` fails `'vec' requires a string as second argument`.

#### Temporal

| Name | min | vararg |
|---|---|---|
| `dt_add` | 3 | no |
| `dt_day` | 1 | **yes** |
| `dt_diff` | 3 | no |
| `dt_dow` | 1 | **yes** |
| `dt_doy` | 1 | **yes** |
| `dt_format` | 2 | **yes** |
| `dt_hour` | 1 | **yes** |
| `dt_minute` | 1 | **yes** |
| `dt_month` | 1 | **yes** |
| `dt_second` | 1 | **yes** |
| `dt_to_validity` | 1 | **yes** |
| `dt_trunc` | 2 | **yes** |
| `dt_year` | 1 | **yes** |
| `format_timestamp` | 1 | **yes** |
| `now` | 0 | no |
| `parse_timestamp` | 1 | no |
| `ulid_timestamp` | 1 | no |
| `uuid_timestamp` | 1 | no |
| `validity` | 1 | **yes** |

`dt_*` take a timestamp in **microseconds**. `dt_year(2024)` → `1970`, i.e. 2024
µs is inside the first second after the epoch — the unit is not optional.

#### Comparison, logic and bitwise

| Name | min | vararg |
|---|---|---|
| `add` | 0 | **yes** |
| `and` | 0 | **yes** |
| `assert` | 1 | **yes** |
| `bit_and` | 2 | no |
| `bit_not` | 1 | no |
| `bit_or` | 2 | no |
| `bit_xor` | 2 | no |
| `coalesce` | 0 | **yes** |
| `eq` | 2 | no |
| `ge` | 2 | no |
| `gt` | 2 | no |
| `le` | 2 | no |
| `lt` | 2 | no |
| `negate` | 1 | no |
| `neq` | 2 | no |
| `or` | 0 | **yes** |

`negate` is logical NOT on **booleans** (`'negate' requires booleans`), not
arithmetic negation — that is `minus`. `coalesce` is `~` (§7) and takes any
number of arguments: `coalesce(null, null, "third")` → `"third"`.

#### JSON

| Name | min | vararg |
|---|---|---|
| `dump_json` | 1 | no |
| `get` | 2 | **yes** |
| `json` | 1 | no |
| `json_object` | 0 | **yes** |
| `json_to_scalar` | 1 | no |
| `maybe_get` | 2 | no |
| `parse_json` | 1 | no |
| `remove_json_path` | 2 | no |
| `set_json_path` | 3 | no |

`get` **errors** on a missing key; `maybe_get` (and the `->` infix projection)
return `null`. `json_object("a", 1, "b", 2)` → `{"a":1,"b":2}`.
`parse_json("{\"a\":1}")` → `{"a":1}`.

#### IRI and identifier

| Name | min | vararg |
|---|---|---|
| `curie_compact` | 2 | no |
| `curie_expand` | 2 | no |
| `iri_resolve` | 2 | no |
| `iri_valid` | 1 | no |
| `rand_ulid` | 0 | no |
| `rand_uuid_v1` | 0 | no |
| `rand_uuid_v4` | 0 | no |

#### Random

| Name | min | vararg |
|---|---|---|
| `rand_bernoulli` | 1 | no |
| `rand_choose` | 1 | no |
| `rand_float` | 0 | no |
| `rand_int` | 2 | no |

### 6.2 Aggregates

Aggregates are a **separate namespace** and go in the rule **head**, not in an
expression — `n = count(line)` is not valid; `count(line)` is a head column. The
all-29 table, as `::builtins aggrs` reports it:

| Name | `is_meet` | `is_bounded_meet` | `custom` |
|---|---|---|---|
| `and` | true | false | false |
| `bit_and` | true | false | false |
| `bit_or` | true | false | false |
| `bit_xor` | false | false | false |
| `choice` | true | false | false |
| `choice_rand` | false | false | false |
| `collect` | false | false | false |
| `count` | false | false | false |
| `count_unique` | false | false | false |
| `group_count` | false | false | false |
| `intersection` | true | false | false |
| `interval_coalesce` | false | false | false |
| `latest_by` | false | false | false |
| `max` | true | false | false |
| `mean` | false | false | false |
| `min` | true | false | false |
| `min_cost` | true | false | false |
| `min_cost_k` | false | **true** | false |
| `or` | true | false | false |
| `pareto_max` | false | **true** | false |
| `pareto_min` | false | **true** | false |
| `product` | false | false | false |
| `shortest` | true | false | false |
| `smallest_by` | false | false | false |
| `std_dev` | false | false | false |
| `sum` | false | false | false |
| `union` | true | false | false |
| `unique` | false | false | false |
| `variance` | false | false | false |

`is_meet` means the aggregate's combine is an absorptive semilattice, so it is
**admissible in a recursive rule**; `is_bounded_meet` means its store keeps a
per-group *set* rather than a single value. Both flags decide whether a given
aggregate can be used where, and neither was discoverable before §5.

## 7. Regex and `~`

### 7.1 There is no `~=` operator

`~=` is not in the grammar. A query using it stops at the operator with an
expected-token list, because `=` (assignment/equality) is what follows an
expression and `~` is a binary operator with nothing to do with matching.

### 7.2 `~` is `coalesce`

The tilde is null-coalescing, exactly as in other query languages:

```cozoscript
?[v] := v = null ~ "fallback"
```

Verified: `"fallback"`. And with a bound left side it keeps the bound value:

```cozoscript
?[a, v] := a = null, v = a ~ "fallback"
```

Verified: `(null, "fallback")`.

### 7.3 Pattern matching is `regex_matches(s, "pat")`

**A string literal in argument position 1 is auto-promoted to a compiled regex**,
so there is nothing to wrap:

```cozoscript
?[use_file] := *sym_use[use_file, _, _, _], regex_matches(use_file, "^src")
:sort use_file
```

Verified: `(src/a.ts)`, `(src/b.ts)`. Negation composes as usual:

```cozoscript
?[use_file] := *sym_use[use_file, _, _, _], not regex_matches(use_file, "^lib")
:sort use_file
```

Verified: both rows.

**No `regex(...)` wrapper exists and none is needed.** There is no `regex` row in
`::builtins ops`, and writing one fails:

```
eval::no_implementation: No implementation found for op `regex`
```

The promotion happens for every `regex_*` builtin — argument position 1 is
rewritten to the internal compiled-regex form at parse, which is why
`regex_extract`, `regex_replace` etc. all take a bare literal too:

```cozoscript
?[a, b] := a = regex_extract("abc123", "[0-9]+"),
                b = regex_replace("a1b2", "[0-9]", "#")
```

Verified: `("123", "a#b2")`.

The pattern is a **regex**, not a glob or a SQL `LIKE`. `LIKE` semantics are
`str_includes` for a substring, `starts_with`/`ends_with` for the anchored forms.

### 7.4 Indexed search: `::fts create` + `~idx{…}`

For a search over a corpus, use the full-text index rather than `regex_matches`:
the index is tokenised, stemmed/filtered as configured, and scored.

```
::fts create doc:by_body {extractor: body, tokenizer: Simple, filters: [Lowercase]}
```

`extractor` is the column to index, `tokenizer` is one of `Raw`, `Simple`,
`Whitespace`, `NGram(a, b)`, `Cangjie`, and `filters` is a list of `Lowercase`,
`Stopwords('en')`, `Stemmer('english')`. Both `extractor:` and `tokenizer:` are
required; a bare column name in the body does **not** parse.

The query surface is `~<relation>:<index>{<key columns> | query: "...", k: N}`:

```cozoscript
?[id, score] := ~doc:by_body{id | query: "fox", k: 10, bind_score: score}
:order id
```

Verified: `(1, 0.426…)`, `(3, 0.524…)` over three seeded documents. `bind_score`
is optional — omit it and you get keys only:

```cozoscript
?[id] := ~doc:by_body{id | query: "fox", k: 10}
:order id
```

Verified: `(1)`, `(3)`. Note `::fts drop doc:by_body` removes it.

## 8. Constant relations

A **constant rule** enumerates a literal list. The head takes **brackets** (`<-`),
not `:=`, and the body is **a list of lists of the head's arity**:

```cozoscript
pairs[a, b] <- [[1, "one"], [2, "two"]]
?[a, b] := pairs[a, b]
:sort a
```

Verified: `(1, "one")`, `(2, "two")`.

For a **one-column** head the inner lists are one element each — this is the part
that trips people up, because it does not follow from the two-column example:

```cozoscript
l[s] <- [["abc"], ["x"]]
?[s] := l[s]
:sort s
```

Verified: `(abc)`, `(x)`. A flat `l[s] <- ["abc", "x"]` fails:

```
parser::bad_row_for_const: The body of a constant rule should evaluate to a list of lists
```

**`l[s] := ["abc", "x"]` does not parse the way `l[s] := f(x)` does.** `:=`
writes a rule with an expression head and no body atoms, so:

- with **no** entry rule at all, the parse fails first:
  ```
  parser::no_entry: You need to have one rule named '?'
  ```
- with an entry rule that applies it, `s` is never bound, and the failure moves
  to evaluation:
  ```
  eval::unbound_variable: Atom contains unbound variable, or rule contains no variable at all
  ```

Which error you get depends on whether you wrote an entry rule, which is worth
knowing because the first one says nothing about the real problem.

### 8.1 One-to-many unification is `x in [...]`

To give one variable several values, use `in`:

```cozoscript
?[s] := s in ["abc", "x"]
:sort s
```

Verified: `(abc)`, `(x)`. The left side of `in` is a **single binding**, so a
tuple is not unified this way — `[a, b] in [[1, "one"], …]` does not parse. Use
a constant rule (§8) for tuples.

### 8.2 A related trap: `:put`'s head names the relation's columns

While building fixtures for this document the following cost several
iterations, and it has the same shape as §8 — so it is recorded here rather than
discovered again.

`:put`'s head is the **row spec** (the same `table_col` grammar as `:create`'s
body). It is **not** positional and it is **not** a subset: it must list **every**
column of the target relation, using **the relation's own column names**, keys
before `=>` and values after.

```cozoscript
?[use_file, sym, line, note] <- [["src/a.ts", "foo", 10, ""]] :put sym_use {use_file, sym, line => note}
```

Verified. Alias the query head instead and it fails:

```
?[a, b, c, d] <- [["src/a.ts", "foo", 10, ""]] :put sym_use {a, b, c => d}
→ eval::required_col_not_found: required column a not found
```

Drop a column and you get the other half:

```
eval::required_col_not_provided: required column v not provided by input
```

Both messages name an identifier the *user* wrote rather than the relation
column that is actually missing, so they read as if the query head were at fault.
It is the put-head.

Also note, because it is the same family of surprise: **a plain script carries
at most one relation option.** A `:create` and a `:put` in the same script fail
with `parser::multiple_store_relation` and the help *"Run the `:create` in its
own `run()` call, then the DML in a second."*

## 9. Relation patterns need full arity

A stored-relation pattern must list **every** column. There is no partial-arity
read and no column-name projection:

```cozoscript
?[use_file] := *sym_use[use_file]
```

```
eval::rule_arity_mismatch
Required arity: 4, number of arguments given: 1. Every column must be listed;
write `_` in the positions you do not need, e.g. `*sym_use[id, _, _, _]`
Arity mismatch for rule application sym_use
```

**`_` is how you skip a column.** This is the form to reach for, and it is what
§3 uses:

```cozoscript
?[use_file] := *sym_use[use_file, _, _, _]
```

Verified: `(src/a.ts)`, `(src/b.ts)` — 2 rows, because of §3, not because of
arity. Too many arguments fails the same way, and so does a non-stored rule
application.

Two notes on the message. `_` in the **body** binds nothing, so you still have to
name every column you need in the rule head — a head of `?[one] := *sym_use[_, _,
_, _]` fails with `eval::unbound_symb_in_head`, whose help now says so. And the
worked example in the help text is a fixed four-slot literal rather than an
interpolation of the real arity, so on a three-column relation it suggests
`*three[id, _, _, _]`, which does not parse; recorded as a diagnostic gap for the
owning track, not fixed here.

## 10. How these tables were verified

Not transcribed. The procedure, so the next person can repeat it:

1. Run `::builtins options`, `::builtins ops`, `::builtins aggrs`, `::builtins all`
   and bare `::builtins` against a live store; capture each as JSON.
2. Generate §2 and §6.2 verbatim from the captured rows — §2's table is the
   `options` rows in order, §6.2's is the `aggrs` rows, §6.1's arity/vararg
   columns are the `ops` rows.
3. Check the `ops` classification is a **partition**: every one of the 158 live
   names in exactly one §6.1 group, every group member present live. The check is
   a script, and it reported `UNCLASSIFIED: parse_json`, `UNCLASSIFIED: windows`
   and `UNCLASSIFIED: negate` on its first run — all three real omissions, since
   fixed by putting them in JSON, Collections and Comparison/logic respectively.
   It now reports `classified: 158 / live total: 158 / PARTITION EXACT`.
4. Cross-check the specific claims this document makes about *absence* against the
   same captured rows: `regex` is not an `ops` name, `contains` is not an `ops`
   name, and `str_includes`, `starts_with`, `ends_with`, `lowercase`, `uppercase`
   all are. Each of those claims was also confirmed by running the failing form
   and reading the error.

Every example in this document was executed against a live `redb` store. Where a
query is shown failing, the quoted message is the engine's own output from that
run, with the ANSI decoration stripped — not a reconstruction.

### 10.1 Two things this document does *not* claim

- **The `ops` table's `min_arity`/`vararg` are a lower bound, not a signature.**
  They say how few arguments are accepted and whether more than the minimum are
  allowed. They do not say what the arguments mean; several builtins have
  positional or type requirements the arity cannot express (`vec`'s second
  argument is a type *string*; `windows`/`chunks`/`reverse` reject strings
  outright; `snippet` needs its open and close markers together).
- **This is not a grammar reference.** Rule syntax, unification, the aggregation
  and negation model, and the relation lifecycle are out of scope; the companion
  specs listed in the preamble cover those. What is here is the surface a reader
  most often reaches for and most often guesses wrong about, and nothing else.

---

## Changelog

| Date | Change |
|---|---|
| 2026-10-05 | **First authoring.** Written in response to six items in an external fork-issues report (2026-10-04) and the analysis note answering it. **The analysis note is wrong on 4 of 6** — it proposes pipeline operators, a `string::` namespace and a `~=` operator, all of which exist in different form (§4, §6, §7) — and its P0/P1/P2 ordering is therefore not used as the work plan. This spec answers all six with the corrected facts, and identifies **§3 (set semantics)** as the only silent-wrong-answer item and **§2.1 (`:sort` validates against output columns)** as the hazard the same reporter will hit next. Tables generated from live `::builtins` output and diffed against it (§10); every example executed against a live `redb` store. Two engine/diagnostic gaps found and **recorded, not fixed** (they live in files this track does not own): the arity-hint example in `query/compile.rs` is a fixed four-slot literal rather than an interpolation of the real arity, and `parser::sort_key_not_found` cannot distinguish "not projected" from "does not exist". |
| 2026-10-05 | **§2.2 added — a third gap, and the worst of the three.** A bare `true` conjunct in a rule body is a no-op, but combined with **any** trailing query option it returns an **empty result** instead: `?[k] := *r[k, _], true` gives 2 rows, and the same query plus `:sort k`, `:limit 5`, `:offset 0`, `:timeout` or `:mem_limit` gives 0. Reproduced on `redb` and `sqlite` on a two-row relation; propagates through a helper rule; harmless in an unrelated rule; the aggregate form is unaffected. Documented with the workaround (delete the conjunct — it means nothing) rather than papered over. |
