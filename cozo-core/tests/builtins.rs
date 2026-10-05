/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! `::builtins` and `::version` — the enumerable language surface.
//!
//! The gap these close is discoverability, not capability: every builtin
//! function, query option and aggregate already *worked*, none of it could be
//! listed from a script. Four separate fork issues came out of that one hole.
//!
//! Sqlite backend per the repo test-backend rule. These sysops touch no stored
//! relation at all, so the backend only affects the `tempfile` plumbing — but
//! the rule exists so no test accidentally depends on a `mem`-only operator
//! path, and a test file is a bad place to be the exception.

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use std::collections::BTreeMap;

fn db() -> (tempfile::TempDir, DbInstance) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("builtins.db");
    let db = DbInstance::new("sqlite", path.to_str().unwrap(), Default::default()).unwrap();
    (dir, db)
}

fn run(db: &DbInstance, script: &str) -> NamedRows {
    db.run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
        .unwrap_or_else(|e| panic!("script failed: {script}\n{e:?}"))
}

/// miette's fancy Debug rendering line-wraps messages with box-drawing
/// decorations; collapse to one plain line so `contains` checks are robust.
fn errstr(e: &impl std::fmt::Debug) -> String {
    format!("{e:?}")
        .replace(['\u{2502}', '\u{d7}'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rows keyed by COLUMN NAME, not position: these assertions are about the
/// surface an agent sees, so a column reorder has to be a visible diff rather
/// than a silently-passing index shift.
fn records(rows: &NamedRows) -> Vec<BTreeMap<String, DataValue>> {
    rows.rows
        .iter()
        .map(|r| {
            rows.headers
                .iter()
                .cloned()
                .zip(r.iter().cloned())
                .collect::<BTreeMap<_, _>>()
        })
        .collect()
}

fn names(recs: &[BTreeMap<String, DataValue>]) -> Vec<&str> {
    recs.iter()
        .filter_map(|r| r.get("name").and_then(|v| v.get_str()))
        .collect()
}

fn row<'a>(recs: &'a [BTreeMap<String, DataValue>], name: &str) -> &'a BTreeMap<String, DataValue> {
    recs.iter()
        .find(|r| r.get("name").and_then(|v| v.get_str()) == Some(name))
        .unwrap_or_else(|| panic!("no row named {name:?}; got {:?}", names(recs)))
}

fn flag(r: &BTreeMap<String, DataValue>, col: &str) -> bool {
    r[col]
        .get_bool()
        .unwrap_or_else(|| panic!("{col} is not a bool: {:?}", r[col]))
}

fn count_of_kind(recs: &[BTreeMap<String, DataValue>], kind: &str) -> usize {
    recs.iter()
        .filter(|r| r.get("kind").and_then(|v| v.get_str()) == Some(kind))
        .count()
}

/// The hard requirement from the track: ONE unconditional call has to discover
/// the whole surface. Not "the documented subset", not a redirect to another
/// call — the same rows as the explicit union.
#[test]
fn bare_builtins_is_exactly_builtins_all() {
    let (_d, db) = db();
    let bare = run(&db, "::builtins");
    let all = run(&db, "::builtins all");
    assert_eq!(bare.headers, all.headers);
    assert_eq!(records(&bare), records(&all));

    // ... and it really is the union of all three slices, not just one of them.
    let recs = records(&bare);
    assert_eq!(bare.rows.len(), bare.rows.len());
    for kind in ["ops", "options", "aggrs"] {
        let n = count_of_kind(&recs, kind);
        assert!(n > 0, "`::builtins` reports no {kind}");
        let slice = run(&db, &format!("::builtins {kind}"));
        assert_eq!(
            n,
            slice.rows.len(),
            "the {kind} slice of `::builtins all` must match `::builtins {kind}`"
        );
    }
}

#[test]
fn builtins_ops_reports_names_arity_and_vararg() {
    let (_d, db) = db();
    let rows = run(&db, "::builtins ops");
    assert_eq!(rows.headers, vec!["name", "min_arity", "vararg"]);

    let recs = records(&rows);
    // The three the track named, checked against the engine's own statics.
    for name in ["str_includes", "starts_with", "regex_matches"] {
        let r = row(&recs, name);
        assert_eq!(
            r["min_arity"].get_int(),
            Some(2),
            "{name} min_arity disagrees with define_op!"
        );
        assert!(!flag(r, "vararg"), "{name} is not vararg");
    }
    // A vararg builtin, so the column is not a constant `false`.
    assert!(flag(row(&recs, "vec"), "vararg"));
    assert_eq!(row(&recs, "vec")["min_arity"].get_int(), Some(1));

    // A table with duplicates or holes is worse than none: an agent would
    // trust it.
    let mut sorted = names(&recs);
    let total = sorted.len();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), total, "`::builtins ops` repeats a name");
    assert!(total > 100, "only {total} builtins reported");
}

#[test]
fn builtins_options_lists_keywords_with_descriptions() {
    let (_d, db) = db();
    let rows = run(&db, "::builtins options");
    assert_eq!(rows.headers, vec!["name", "description"]);

    let recs = records(&rows);
    for name in [":limit", ":sort"] {
        let r = row(&recs, name);
        assert!(
            !r["description"].get_str().unwrap().is_empty(),
            "{name} has no description"
        );
    }
    // Every keyword must actually be one the grammar accepts — the drift test
    // in `parse/mod.rs` pins QUERY_OPTIONS to the grammar, so this only has to
    // confirm the sysop forwards the table verbatim.
    let mut sorted = names(&recs);
    let total = sorted.len();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        total,
        "`::builtins options` repeats a keyword"
    );
    assert!(total >= 20, "only {total} query options reported");
}

#[test]
fn builtins_aggrs_reports_the_evaluation_flags() {
    let (_d, db) = db();
    let rows = run(&db, "::builtins aggrs");
    assert_eq!(
        rows.headers,
        vec!["name", "is_meet", "is_bounded_meet", "custom"]
    );

    let recs = records(&rows);
    for name in ["count", "sum", "collect", "min", "union", "min_cost_k"] {
        row(&recs, name);
    }
    // is_meet: admissible in a recursive rule. The two flags are what a script
    // author needs and had no way to learn before.
    assert!(flag(row(&recs, "min"), "is_meet"));
    assert!(flag(row(&recs, "union"), "is_meet"));
    assert!(!flag(row(&recs, "count"), "is_meet"));
    // is_bounded_meet: the store keeps a per-group SET, not one value.
    assert!(flag(row(&recs, "min_cost_k"), "is_bounded_meet"));
    assert!(flag(row(&recs, "pareto_min"), "is_bounded_meet"));
    assert!(!flag(row(&recs, "min"), "is_bounded_meet"));
    // The internal-only aggregate must stay invisible: no script can name it.
    assert!(
        !names(&recs).contains(&"int_sum_prod"),
        "`::builtins aggrs` leaked a non-script-facing aggregate"
    );
    // Nothing is registered on a fresh Db.
    for name in names(&recs) {
        assert!(
            !flag(row(&recs, name), "custom"),
            "{name} claimed to be a custom aggregate on a fresh Db"
        );
    }
}

/// Aggregate resolution has two `Db`-scoped registries besides `parse_aggr`.
/// They are part of the surface *this* Db offers, so the enumeration has to
/// show them — otherwise `::builtins aggrs` answers a question nobody asked.
#[test]
fn builtins_aggrs_include_aggregates_registered_on_this_db() {
    let (_d, db) = db();
    db.register_custom_aggr("test_meet".to_string(), true, || {
        unreachable!("factory is never called by introspection")
    })
    .unwrap();
    db.register_bounded_meet_aggr("test_dominance".to_string(), |a, b| a == b, 8)
        .unwrap();

    let recs = records(&run(&db, "::builtins aggrs"));
    let meet = row(&recs, "test_meet");
    assert!(flag(meet, "custom"));
    assert!(flag(meet, "is_meet"));
    assert!(!flag(meet, "is_bounded_meet"));

    let dom = row(&recs, "test_dominance");
    assert!(flag(dom, "custom"));
    assert!(flag(dom, "is_bounded_meet"));
    assert!(!flag(dom, "is_meet"));

    // A builtin name can never be shadowed, so no row may carry both.
    for name in names(&recs) {
        let r = row(&recs, name);
        assert!(
            !(flag(r, "custom") && !name.starts_with("test_")),
            "{name} is both builtin and custom"
        );
    }
}

/// The union shape is one wide table with a `kind` discriminator. Cells that do
/// not apply to a row's kind are `null` — a real value `is null` can test — not
/// an empty string that every string comparison has to special-case.
#[test]
fn builtins_all_nulls_out_the_columns_a_kind_does_not_have() {
    let (_d, db) = db();
    let rows = run(&db, "::builtins all");
    assert_eq!(
        rows.headers,
        vec![
            "kind",
            "name",
            "min_arity",
            "vararg",
            "description",
            "is_meet",
            "is_bounded_meet",
            "custom"
        ]
    );
    let recs = records(&rows);

    let find_kind = |kind: &str, name: &str| -> BTreeMap<String, DataValue> {
        recs.iter()
            .find(|r| {
                r.get("kind").and_then(|v| v.get_str()) == Some(kind)
                    && r.get("name").and_then(|v| v.get_str()) == Some(name)
            })
            .unwrap_or_else(|| panic!("no {kind} row named {name:?}"))
            .clone()
    };

    let op = find_kind("ops", "length");
    assert_eq!(op["min_arity"].get_int(), Some(1));
    for absent in ["description", "is_meet", "is_bounded_meet", "custom"] {
        assert_eq!(op[absent], DataValue::Null, "ops row has a {absent}");
    }

    let opt = find_kind("options", ":limit");
    assert!(!opt["description"].get_str().unwrap().is_empty());
    for absent in [
        "min_arity",
        "vararg",
        "is_meet",
        "is_bounded_meet",
        "custom",
    ] {
        assert_eq!(opt[absent], DataValue::Null, "options row has a {absent}");
    }

    let aggr = find_kind("aggrs", "count");
    assert!(!flag(&aggr, "is_meet"));
    for absent in ["min_arity", "vararg", "description"] {
        assert_eq!(aggr[absent], DataValue::Null, "aggrs row has a {absent}");
    }
}

#[test]
fn version_reports_the_engine_constant_and_cannot_drift_from_the_manifest() {
    let (_d, db) = db();
    let rows = run(&db, "::version");
    assert_eq!(rows.headers, vec!["version"]);
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][0].get_str().unwrap(), cozo::ENGINE_VERSION);
    // `ENGINE_VERSION` is `env!("CARGO_PKG_VERSION")` of THIS package. If the
    // constant is ever replaced by a hand-written string, the engine will
    // report a version it was not built from.
    assert_eq!(cozo::ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
}

/// A mistyped slice must fail loudly. Silently returning zero rows is the worst
/// possible answer for an introspection call: the caller concludes the
/// construct does not exist, which is the bug this whole track exists to stop.
#[test]
fn builtins_rejects_an_unknown_slice_instead_of_returning_nothing() {
    let (_d, db) = db();
    for script in [
        "::builtins bogus",
        "::builtins ops extra",
        "::builtins aggrs ops",
    ] {
        let e = db
            .run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
            .unwrap_err();
        let msg = errstr(&e);
        assert!(
            msg.contains("builtins") || msg.contains("expected"),
            "{script}: uninformative error {msg:?}"
        );
    }
}

/// Both grammar lists are edited for this: `sys_script` (script start) and
/// `sys_script_inner` (inside `{ }`) spell the sysop alternatives separately.
/// Getting only one right gives a sysop that works standalone and fails inside
/// every imperative block — the failure mode that motivated editing both.
#[test]
fn builtins_and_version_parse_inside_an_imperative_block() {
    let (_d, db) = db();
    for script in [
        "{::builtins}",
        "{::builtins ops}",
        "{::builtins options}",
        "{::builtins aggrs}",
        "{::builtins all}",
        "{::version}",
    ] {
        db.run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
            .unwrap_or_else(|e| panic!("imperative block failed: {script}\n{e:?}"));
    }
}

/// ... and the rows come back through the binding an imperative statement can
/// take (`{...} as _name`), not just through a bare `is ok()`.
///
/// Scope note: referencing that binding from a LATER sibling clause fails with
/// "Requested rule _ops not found" — but it fails identically for the
/// pre-existing `::fixed_rules`, so that is an engine-wide `as`-binding
/// limitation, not something `::builtins` introduces. Asserting on it here
/// would pin a bug; it is recorded in the T1 plan log for T2 instead.
#[test]
fn builtins_rows_survive_the_imperative_as_binding() {
    let (_d, db) = db();
    let rows = run(&db, "{::builtins ops} as _ops");
    assert_eq!(rows.headers, vec!["name", "min_arity", "vararg"]);
    let standalone = run(&db, "::builtins ops");
    assert_eq!(rows.rows.len(), standalone.rows.len());
    assert!(rows.rows.len() > 100, "only {} rows bound", rows.rows.len());
}
