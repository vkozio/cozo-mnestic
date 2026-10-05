/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Issue 5: the two diagnostics a newcomer hits when they want one column out
//! of a wide relation must name the `_` placeholder.
//!
//! `?[id] := *ast_node[id]` used to say only "Required arity: 7, number of
//! arguments given: 1", which reads as an engine bug: the grammar requires
//! *full* arity by design, so the fix is `*ast_node[id, _, _, _, _, _, _]`.
//! The numbers stayed (they are what lets a user diagnose a genuine bug) and a
//! `_` clause was appended to the `help` of `ArityMismatch` in
//! `cozo-core/src/query/compile.rs`.
//!
//! `?[v] := *lone[_]` is the other shape: `_` binds nothing, so a head that
//! names it fails with `eval::unbound_symb_in_head`. Its help gained the same
//! one-clause pointer.
//!
//! The guidance lives in `#[diagnostic(help(...))]`, shared by all four
//! `ArityMismatch` raise sites. These tests pin all four anyway, because the
//! attribute could later be split per site with no compiler help.
//!
//! Two language details the tests lean on, both discovered the hard way:
//!
//! - The two arities differ on purpose. `ast_node` declares six columns but the
//!   fork appends a hidden `tt` column, so its *query* arity is seven
//!   (`RelationHandle::arity` is `non_keys.len() + keys.len()`); `node6` is a
//!   rule, and a rule's arity is its head's column count, so it is six. That is
//!   why the help's `_` example is schematic rather than a fixed count.
//! - A *relation* is applied as `*name[...]`, a *rule* as `name[...]` — writing
//!   `*node6[...]` looks up a relation and fails with
//!   `query::relation_not_found` before the arity check. Rule bodies also
//!   separate atoms with `,`, not with a newline.

use cozo::DbInstance;

/// miette's fancy Debug rendering includes the diagnostic code and hard-wraps
/// at ~80 columns, so collapse whitespace before `contains` checks — same
/// harness as `tests/index_diagnostics.rs`.
fn err(db: &DbInstance, s: &str) -> String {
    let e = db.run_default(s).unwrap_err();
    format!("{e:?}")
        .replace(['\u{2502}', '\u{d7}'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Six declared columns (seven of query arity) and a one-column relation, so
/// `*lone[_]` clears the arity check and fails on the head instead.
fn make_db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    db.run_default(
        ":create ast_node {id: Int => kind: String, name: String, parent: Int, \
         score: Float, tag: String, depth: Int}",
    )
    .unwrap();
    db.run_default(":create lone {v: Int}").unwrap();
    db
}

/// A six-arity rule over `ast_node`. It has to be defined in the same script
/// that applies it: a rule is not visible to a later script (the next one gets
/// `eval::rule_not_found`).
const NODE6_DEF: &str =
    "node6[n_a, n_b, n_c, n_d, n_e, n_f] := *ast_node[n_a, n_b, n_c, n_d, n_e, n_f, _]";

/// The reported shape, on the `Relation` raise site.
#[test]
fn arity_mismatch_help_names_the_placeholder() {
    let db = make_db();
    let m = err(&db, "?[id] := *ast_node[id]");
    assert!(m.contains("eval::rule_arity_mismatch"), "{m}");
    // The pre-existing numbers survive — they are the diagnostic part.
    assert!(m.contains("Required arity: 7"), "{m}");
    assert!(m.contains("number of arguments given: 1"), "{m}");
    // The new part.
    assert!(m.contains("`_`"), "help does not name `_`: {m}");
    assert!(m.contains("*ast_node[id, _, _, _]"), "{m}");
    // Full arity is required by the grammar; the message must not imply
    // otherwise.
    assert!(m.contains("Every column must be listed"), "{m}");
}

/// `Rule` raise site: same struct, same attribute. A rule's arity is its head's
/// column count (six), unlike the relation's seven.
#[test]
fn arity_mismatch_help_names_the_placeholder_for_rule_applications() {
    let db = make_db();
    let m = err(&db, &format!("{NODE6_DEF}\n?[id] := node6[id]"));
    assert!(m.contains("eval::rule_arity_mismatch"), "{m}");
    assert!(m.contains("Required arity: 6"), "{m}");
    assert!(m.contains("number of arguments given: 1"), "{m}");
    assert!(m.contains("*node6[id, _, _, _]"), "{m}");
}

/// `NegatedRelation` raise site.
#[test]
fn arity_mismatch_help_names_the_placeholder_when_negated() {
    let db = make_db();
    let m = err(
        &db,
        "?[id] := *ast_node[id, _, _, _, _, _, _], not *ast_node[id]",
    );
    assert!(m.contains("eval::rule_arity_mismatch"), "{m}");
    assert!(m.contains("Required arity: 7"), "{m}");
    assert!(m.contains("number of arguments given: 1"), "{m}");
    assert!(m.contains("`_`"), "{m}");
}

/// `NegatedRule` raise site.
#[test]
fn arity_mismatch_help_names_the_placeholder_when_a_rule_is_negated() {
    let db = make_db();
    let m = err(
        &db,
        &format!("{NODE6_DEF}\n?[id] := *ast_node[id, _, _, _, _, _, _], not node6[id]"),
    );
    assert!(m.contains("eval::rule_arity_mismatch"), "{m}");
    assert!(m.contains("Required arity: 6"), "{m}");
    assert!(m.contains("*node6[id, _, _, _]"), "{m}");
}

/// The form the help recommends must actually be accepted, otherwise the advice
/// is worse than no advice.
#[test]
fn the_suggested_full_arity_form_runs() {
    let db = make_db();
    db.run_default(
        "?[id, kind, name, parent, score, tag, depth] <- \
         [[1, 'a', 'n', 0, 0.5, 't', 1]] :put ast_node {id => kind, name, parent, \
         score, tag, depth}",
    )
    .unwrap();
    let out = db
        .run_default("?[id] := *ast_node[id, _, _, _, _, _, _]")
        .expect("the suggested fix must parse and run");
    let s = format!("{out:?}");
    assert!(s.contains('1'), "expected row id 1, got {s}");
}

/// The other diagnostic the reporter hit: `_` in the body binds nothing, so a
/// head naming it fails. Its help gained the same pointer.
#[test]
fn unbound_head_help_names_the_placeholder() {
    let db = make_db();
    let m = err(&db, "?[v] := *lone[_]");
    assert!(m.contains("eval::unbound_symb_in_head"), "{m}");
    assert!(m.contains("`_` in the body binds nothing"), "{m}");
    assert!(
        m.contains("name every column you need in the rule head"),
        "{m}"
    );
    // The pre-existing clause survives.
    assert!(
        m.contains("negated positions are not considered bound"),
        "{m}"
    );
}

/// The fix for the unbound head: name the column in both places.
#[test]
fn naming_the_column_in_head_and_body_binds_it() {
    let db = make_db();
    db.run_default("?[v] <- [[7]] :put lone {v}").unwrap();
    let out = db.run_default("?[v] := *lone[v]").expect("must bind");
    let s = format!("{out:?}");
    assert!(s.contains('7'), "expected row v 7, got {s}");
}
