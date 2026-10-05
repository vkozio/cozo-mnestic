/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! T5: a bare boolean literal conjunct plus *any* trailing query option used
//! to silently return zero rows.
//!
//! ```cozoscript
//! ?[a] := a in [1,2,3], true :limit 2   // returned 0, must return 2
//! ?[a] := a in [1,2,3], true             // returned 3, correct
//! ```
//!
//! This is the worst failure class: `ok:true`, no warning, and a `:limit 1`
//! query that looks exactly like "no such data". No stored relation and no
//! storage engine are involved — the repro is pure in-memory evaluation.
//!
//! Every assertion below states the **correct** answer, so this file is a
//! specification, not a pin of current behaviour. The four rows that already
//! passed on the buggy build are kept anyway: they are the differential
//! controls that prove the defect needs all three of {bare literal conjunct,
//! an option, the non-parenthesised parse path}, so a future "fix" that
//! special-cases too eagerly (e.g. rejecting `true` in a body) breaks here.
//!
//! Nothing here needs a rule to be defined in the same script: a bare
//! `?[a] := ...` query is its own entry rule.

use cozo::DbInstance;

fn mem() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

/// Row count of a query, asserting it succeeds. On failure the engine's own
/// error is printed rather than swallowed.
fn rows(db: &DbInstance, script: &str) -> usize {
    match db.run_default(script) {
        Ok(out) => out.rows.len(),
        Err(e) => panic!("`{script}` failed: {e:?}"),
    }
}

/// The single value of the single-column result, rendered with `Debug`, so an
/// assertion compares contents rather than row count alone.
fn only(db: &DbInstance, script: &str) -> String {
    let out = db
        .run_default(script)
        .unwrap_or_else(|e| panic!("`{script}` failed: {e:?}"));
    assert_eq!(out.rows.len(), 1, "`{script}` should give 1 row: {out:?}");
    assert_eq!(out.rows[0].len(), 1, "`{script}` should have 1 column: {out:?}");
    format!("{:?}", out.rows[0][0])
}

/// Every row's first column, rendered with `Debug`, so an assertion compares
/// contents and not incidental iteration order.
fn cells(db: &DbInstance, script: &str) -> Vec<String> {
    let out = db
        .run_default(script)
        .unwrap_or_else(|e| panic!("`{script}` failed: {e:?}"));
    out.rows
        .iter()
        .map(|r| {
            assert_eq!(r.len(), 1, "`{script}` should have 1 column: {out:?}");
            format!("{:?}", r[0])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Controls: these are the rows of the characterisation table that were already
// correct on the buggy build. They must stay correct.
// ---------------------------------------------------------------------------

/// No option present: the bare path. 3 rows.
#[test]
fn bare_true_conjunct_without_option_is_unaffected() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true"), 3);
}

/// `false` genuinely filters everything out — 0 rows is the *correct* answer
/// here, and it is the control for the row below. If the fix ever made
/// constant folding swallow `false`, this fails.
#[test]
fn bare_false_conjunct_without_option_filters_everything() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], false"), 0);
}

/// Two bare literal conjuncts, no option: still fine.
#[test]
fn two_bare_literal_conjuncts_without_option_are_unaffected() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true, true"), 3);
}

/// A comparison rather than a bare literal, plus an option: already correct.
#[test]
fn comparison_conjunct_with_option_is_unaffected() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], 1 == 1 :sort a"), 3);
}

/// Parenthesising the literal: already correct.
#[test]
fn parenthesised_literal_conjunct_with_option_is_unaffected() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], (true) :sort a"), 3);
}

/// `not false` rather than a bare literal: already correct.
#[test]
fn negated_literal_conjunct_with_option_is_unaffected() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], not false :sort a"), 3);
}

/// Aggregate head: already correct, 1 row counting 3.
#[test]
fn aggregate_head_with_bare_true_conjunct_is_unaffected() {
    let db = mem();
    assert_eq!(only(&db, "?[count(a)] := a in [1,2,3], true"), "3");
}

// ---------------------------------------------------------------------------
// The defect: bare literal conjunct + any option.
// ---------------------------------------------------------------------------

/// `:limit` under-reports. `:limit 2` on three rows must give 2, and
/// `:limit 1` — a "top 1" query — must give 1. This is the row that reads as
/// "no such data" rather than "engine bug".
#[test]
fn bare_true_conjunct_with_limit_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :limit 2"), 2);
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :limit 1"), 1);
}

/// `:limit` is not the only size-affecting option: `:offset` is the control.
/// `:offset 0` is a no-op option that must still yield 3 rows.
#[test]
fn bare_true_conjunct_with_offset_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :offset 0"), 3);
}

/// `true` combined with an offset that actually skips, and with
/// offset+limit together.
#[test]
fn bare_true_conjunct_with_offset_and_limit_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :offset 1"), 2);
    assert_eq!(
        rows(&db, "?[a] := a in [1,2,3], true :offset 1 :limit 1"),
        1
    );
}

/// `:sort` — content must be right too, not just the count.
#[test]
fn bare_true_conjunct_with_sort_returns_sorted_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :sort a"), 3);
    assert_eq!(
        cells(&db, "?[a] := a in [3,1,2], true :sort a"),
        vec!["1", "2", "3"]
    );
    // descending, and sort combined with limit
    assert_eq!(
        cells(&db, "?[a] := a in [3,1,2], true :sort -a"),
        vec!["3", "2", "1"]
    );
    assert_eq!(rows(&db, "?[a] := a in [3,1,2], true :sort a :limit 2"), 2);
}

/// `:timeout` and `:mem_limit` have nothing to do with sorting or limiting.
/// They triggered the defect too, which is what ruled out the
/// `QueryLimiter`/sort machinery as the locus.
#[test]
fn bare_true_conjunct_with_budget_options_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :timeout 5"), 3);
    assert_eq!(
        rows(&db, "?[a] := a in [1,2,3], true :mem_limit 1000000"),
        3
    );
}

/// `:returning` is the store-relation option; on a non-mutating rule it is a
/// no-op that must still yield 3 rows.
#[test]
fn bare_true_conjunct_with_returning_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :returning"), 3);
}

/// `:disable_magic_rewrite false` + `:limit 2`. Kept as an explicit regression
/// pin because it was the cheapest discriminator during diagnosis: it decides
/// whether the Datalog magic-sets rewrite is implicated at all.
#[test]
fn bare_true_conjunct_survives_magic_rewrite_option() {
    let db = mem();
    assert_eq!(
        rows(
            &db,
            "?[a] := a in [1,2,3], true :disable_magic_rewrite false :limit 2"
        ),
        2
    );
}

/// `false` plus an option: 0 rows is correct, and the constant must still be
/// honoured. If the fix folded constants to a blanket "true", this fails —
/// which is the point of keeping it.
#[test]
fn bare_false_conjunct_with_option_still_filters_everything() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], false :limit 2"), 0);
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], false :sort a"), 0);
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], false :offset 0"), 0);
}

/// A bare literal conjunct that is *not* the last one in the body.
#[test]
fn bare_true_conjunct_before_another_atom_with_option() {
    let db = mem();
    assert_eq!(rows(&db, "?[b] := true, b in [1,2,3] :limit 2"), 2);
    assert_eq!(rows(&db, "?[b] := true, b in [1,2,3] :sort b"), 3);
}

/// Two columns, so the literal conjunct's arity is not the whole story.
#[test]
fn bare_true_conjunct_over_two_column_head() {
    let db = mem();
    assert_eq!(
        rows(&db, "?[a, b] := a in [1,2], b in [3,4], true :limit 3"),
        3
    );
}

/// `:order` is the grammar's alias for `:sort`; the defect covered both because
/// they share one rule, so both are pinned.
#[test]
fn bare_true_conjunct_with_order_alias_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :order a"), 3);
}

/// `:reorder <strategy>` takes an ident argument, so it is the one option that
/// is present in the syntax even when it is a no-op for this body.
#[test]
fn bare_true_conjunct_with_reorder_option_returns_rows() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [3,1,2], true :reorder written"), 3);
}

/// `:assert some` would pass vacuously on a silently empty result, which is the
/// most dangerous shape of this bug. With the fix in place the assertion must
/// hold on the real rows, and `:assert none` on `false` must too.
#[test]
fn bare_literal_conjunct_with_assertion_options() {
    let db = mem();
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], true :assert some"), 3);
    assert_eq!(rows(&db, "?[a] := a in [1,2,3], false :assert none"), 0);
    // `:assert some` must actually fail on an empty result, otherwise the
    // check above proves nothing.
    let e = db
        .run_default("?[a] := a in [1,2,3], false :assert some")
        .unwrap_err();
    let m = format!("{e:?}");
    assert!(m.contains("assert_some_failure"), "{m}");
}

/// Sibling probe found while fixing the main defect: `:limit 0`.
#[test]
fn zero_limit_returns_no_rows() {
    let db = mem();
    let got = vec![
        rows(&db, "?[a] := a in [1,2,3] :limit 0"),
        rows(&db, "?[a] := a in [1,2,3] :limit 0 :sort a"),
        rows(&db, "?[a] := a in [1,2,3] :limit 0 :offset 0"),
        rows(&db, "?[a] := a in [1,2,3] :offset 0 :limit 0"),
        rows(&db, "?[a] := a in [1,2,3] :limit 0 :reorder written"),
        rows(&db, "?[count(a)] := a in [1,2,3] :limit 0"),
    ];
    assert_eq!(got, vec![0, 0, 0, 0, 0, 0]);
}

// ---------------------------------------------------------------------------
// Sibling probes in the same class. These pin the probed behaviour so a
// regression in any of them is caught here rather than in the field.
//
// The three constant-literal probes below are all **negatives**: a non-boolean
// bare constant in condition position is a hard parse-time-shaped error
// (`eval::predicate_not_bool`), never a silent empty result and never a
// tautology. Only `Bool` can be a bare conjunct at all, which is what narrows
// the fix to the boolean case.
// ---------------------------------------------------------------------------

/// miette's fancy Debug hard-wraps at ~80 columns; collapse whitespace before
/// `contains` - same harness as `tests/index_diagnostics.rs`.
fn err(db: &DbInstance, s: &str) -> String {
    let e = db.run_default(s).unwrap_err();
    format!("{e:?}")
        .replace(['\u{2502}', '\u{d7}'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A bare **integer** conjunct is rejected, with or without an option.
#[test]
fn bare_int_conjunct_is_rejected_not_silently_empty() {
    let db = mem();
    for script in [
        "?[a] := a in [1,2,3], 1",
        "?[a] := a in [1,2,3], 1 :limit 2",
        "?[a] := a in [1,2,3], 1 :sort a",
    ] {
        let m = err(&db, script);
        assert!(m.contains("eval::predicate_not_bool"), "`{script}`: {m}");
        assert!(m.contains("where a boolean value is expected"), "`{script}`: {m}");
    }
}

/// A bare **null** conjunct is rejected the same way.
#[test]
fn bare_null_conjunct_is_rejected_not_silently_empty() {
    let db = mem();
    for script in [
        "?[a] := a in [1,2,3], null",
        "?[a] := a in [1,2,3], null :sort a",
    ] {
        let m = err(&db, script);
        assert!(m.contains("eval::predicate_not_bool"), "`{script}`: {m}");
    }
}

/// A bare **string** conjunct is rejected the same way.
#[test]
fn bare_string_conjunct_is_rejected_not_silently_empty() {
    let db = mem();
    for script in [
        "?[a] := a in [1,2,3], 'x'",
        "?[a] := a in [1,2,3], 'x' :sort a",
        "?[a] := a in [1,2,3], '' :limit 2",
    ] {
        let m = err(&db, script);
        assert!(m.contains("eval::predicate_not_bool"), "`{script}`: {m}");
    }
}

/// The imperative path: an `imperative_clause` wrapping the same query. The
/// imperative parser accepts the identical text, so this rules out the defect
/// being specific to `query_script`.
#[test]
fn imperative_path_with_bare_literal_is_unaffected() {
    let db = mem();
    let out = db
        .run_default("{ ?[a] := true, a in [1,2,3] :limit 2 }")
        .expect("imperative clause must parse and run");
    assert_eq!(out.rows.len(), 2, "{out:?}");
    assert_eq!(out.headers, vec!["a".to_string()], "{out:?}");
}

/// Options on a *mutating* query with a bare literal conjunct in the body —
/// the `:insert` path, where the silent empty result **loses a write** rather
/// than under-reporting a read. This is the most severe shape of the defect.
///
/// The literal has to ride on a `rule` body, not on `<-`: `const_rule` is
/// `rule_head ~ "<-" ~ expr`, so a `<-` table takes no extra conjuncts.
#[test]
fn mutating_query_with_bare_literal_conjunct_writes_rows() {
    let db = mem();
    db.run_default(":create tc_src {k: Int}").unwrap();
    db.run_default(":create tc_dst {k: Int}").unwrap();
    db.run_default("?[k] <- [[1], [2], [3]] :insert tc_src {k}")
        .unwrap();
    let out = db
        .run_default("?[k] := *tc_src[k], true :insert tc_dst {k} :returning")
        .expect("insert with a bare literal conjunct must not silently write nothing");
    // The destination is checked *before* the returned rows: the severity of
    // this shape is that the rows are never written at all, not merely that
    // `:returning` under-reports.
    let stored = db.run_default("?[k] := *tc_dst[k] :sort k").unwrap();
    assert_eq!(
        stored.rows.len(),
        3,
        "the insert was silently dropped: {stored:?}"
    );
    assert_eq!(out.rows.len(), 3, "{out:?}");
}

/// A rule defined in the same script and applied with an option — the literal
/// lives in the rule body, not in the query text.
#[test]
fn named_rule_with_bare_literal_body_and_option() {
    let db = mem();
    let out = db
        .run_default("r[x] := x in [1,2,3], true\n?[x] := r[x] :limit 2")
        .expect("a named rule carrying the literal must not empty out");
    assert_eq!(out.rows.len(), 2, "{out:?}");
}