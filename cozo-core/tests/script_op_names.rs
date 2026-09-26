/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Pins the CozoScript-level spellings of the ops and aggregates that
//! `cozo-lib-wasm/docs/wasm-script-notes.md` tells wasm users to reach for.
//!
//! Those notes originally claimed `avg` and `sum` were "not wired up in the wasm
//! build" and that `len` / `format` / `~=` were missing. None of that is a wasm
//! limitation: `get_op` (`data/expr.rs`) and `parse_aggr` (`data/aggr.rs`) are
//! static, target-independent name tables, so a name that is absent from them is
//! absent on **every** platform, and `eval::no_implementation` means only
//! "no such op name" — never "unavailable in this build".
//!
//! The correct spellings are `mean` (not `avg`), `length` (not `len`), and
//! `str_includes` (not a `~=` operator). This suite runs them end-to-end so the
//! corrected names are verified rather than merely asserted, and so a future
//! registry change that removes one of them fails here instead of quietly
//! re-breaking the documented workaround.

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use std::collections::BTreeMap;

fn db_with_people() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    db.run_script(
        ":create person {name: String => age: Int, city: String}",
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        r#"?[name, age, city] <- [["Ada", 36, "London"], ["Alan", 41, "Cambridge"],
                                   ["Grace", 45, "New York"], ["Edsger", 41, "Austin"]]
           :put person {name, age, city}"#,
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db
}

fn rows(db: &DbInstance, script: &str) -> NamedRows {
    db.run_script(script, BTreeMap::new(), ScriptMutability::Immutable)
        .unwrap_or_else(|e| panic!("query failed: {e}\n--- script ---\n{script}"))
}

/// Render a scalar cell as text. Aggregates over `age` land in `DataValue::Num`,
/// which is a float whenever any operand was — so `sum` of ints comes back as
/// `163.0`, not `163`. Read int first, then float, so integer-valued results
/// stringify without a trailing `.0`.
fn cell(v: &DataValue) -> String {
    v.get_str()
        .map(str::to_string)
        .or_else(|| v.get_int().map(|i| i.to_string()))
        .or_else(|| v.get_float().map(|f| f.to_string()))
        .unwrap_or_else(|| format!("{v:?}"))
}

/// The first column of the first row, as text. Empty when the query matched
/// nothing.
fn run(db: &DbInstance, script: &str) -> String {
    let res = rows(db, script);
    res.rows.first().map(|r| cell(&r[0])).unwrap_or_default()
}

/// `mean` is the aggregate; `avg` was never a name. The fork's own Cypher
/// frontend translates Cypher's `avg` to this (`cypher/translate.rs`), which is
/// what made the wasm notes' `avg` look plausible.
#[test]
fn average_aggregate_is_spelled_mean() {
    let db = db_with_people();
    // (36 + 41 + 45 + 41) / 4 = 40.75
    assert_eq!(
        run(&db, "?[mean(age)] := *person{name: _, age: age}"),
        "40.75"
    );
}

/// `sum` *is* registered (`AGGR_SUM`, `data/aggr.rs`). The notes reported it as
/// unimplemented, which pointed at a build gap that does not exist.
#[test]
fn sum_aggregate_works() {
    let db = db_with_people();
    assert_eq!(run(&db, "?[sum(age)] := *person{name: _, age: age}"), "163");
}

/// The full aggregate set the notes under-reported as "count/min/max only".
#[test]
fn other_aggregates_are_available() {
    let db = db_with_people();
    assert_eq!(run(&db, "?[count(name)] := *person{name: name}"), "4");
    assert_eq!(run(&db, "?[min(age)] := *person{name: _, age: age}"), "36");
    assert_eq!(run(&db, "?[max(age)] := *person{name: _, age: age}"), "45");
    // Non-scalar results: assert the query resolves and yields its one row.
    assert_eq!(
        rows(&db, "?[std_dev(age)] := *person{name: _, age: age}")
            .rows
            .len(),
        1
    );
    assert_eq!(
        rows(&db, "?[collect(name)] := *person{name: name}")
            .rows
            .len(),
        1
    );
}

/// `len` does not exist; the op is `length` (`OP_LENGTH`). Usable in **filter**
/// position only: in a rule head `length(name)` parses as an *aggregate*
/// (`head_arg = aggr_arg | var`) and fails with `parser::aggr_not_found`, which
/// is a different error from the unknown-op one and a common second trap.
#[test]
fn string_length_is_spelled_length() {
    let db = db_with_people();
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, age: 36}, length(name) == 3"#
        ),
        "Ada"
    );
    assert_eq!(
        run(&db, r#"?[name] := *person{name: name}, length(name) == 5"#),
        "Grace"
    );
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, age: age}, length(name) == 6"#
        ),
        "Edsger"
    );
}

/// `~=` is not an operator in `cozoscript.pest`. The real substring predicate is
/// `str_includes`, which the notes omitted in favour of the far weaker
/// `starts_with`.
#[test]
fn substring_predicate_is_str_includes() {
    let db = db_with_people();
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name}, str_includes(name, "da")"#
        ),
        "Ada"
    );
    // Interior match: `str_includes` finds it where `starts_with` cannot. The
    // city column is what carries "New York"; the name column does not.
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, city: city}, str_includes(city, "York")"#
        ),
        "Grace"
    );
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, city: city}, starts_with(city, "York")"#
        ),
        ""
    );
    // `str_includes` is case-sensitive — wrap in `lowercase()` for a
    // case-insensitive search. Worth knowing, since the notes steered people to
    // `starts_with` and never mentioned either property.
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, city: city}, str_includes(city, "york")"#
        ),
        ""
    );
    assert_eq!(
        run(
            &db,
            r#"?[name] := *person{name: name, city: city}, str_includes(lowercase(city), "york")"#
        ),
        "Grace"
    );
}

/// The trap behind the notes' `~=` claim: bare `~` is the **coalesce** operator
/// (`op_coalesce` in `cozoscript.pest`), so it parses cleanly and returns a
/// value rather than a boolean — anyone probing for `~=` without reading the
/// grammar gets a non-error that reads like a failed search.
#[test]
fn bare_tilde_is_coalesce_not_contains() {
    let db = db_with_people();
    // Coalesce yields the left operand whenever it is non-null, so the fallback
    // is dead and nothing ever matches it.
    assert_eq!(
        run(
            &db,
            "?[count(name)] := *person{name: name, age: age}, (age ~ -1) == -1"
        ),
        "0"
    );
    // Same column, used as an actual comparison, does match — the difference is
    // that `~` substituted a value instead of testing membership.
    assert_eq!(
        run(
            &db,
            "?[count(name)] := *person{name: name, age: age}, age == 36"
        ),
        "1"
    );
    // With a non-null string it passes the left operand straight through, which
    // is why `name ~ "da" == "Ada"` quietly *matches* — a reader expecting a
    // substring test sees a hit and draws the wrong conclusion.
    assert_eq!(
        run(
            &db,
            r#"?[count(name)] := *person{name: name}, (name ~ "fallback") == "Ada""#
        ),
        "1"
    );
    assert_eq!(
        run(
            &db,
            r#"?[count(name)] := *person{name: name}, (name ~ "fallback") == "fallback""#
        ),
        "0"
    );
}

/// `eval::no_implementation` is reachable only from `Expr::UnboundApply`, i.e.
/// an unknown *name*. It says nothing about the target. Lock that in: `avg` is
/// an unknown name and must not resolve.
#[test]
fn unknown_op_name_reports_no_implementation() {
    let db = db_with_people();
    let err = db
        .run_script(
            "?[count(name)] := *person{name: name, age: avg(age)}",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .expect_err("`avg` is not a registered op name");
    assert!(
        err.to_string().contains("No implementation found"),
        "expected the unknown-name error, got: {err}"
    );
}
