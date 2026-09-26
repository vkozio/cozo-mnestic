/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! FTS index surface, as it behaves for the wasm target.
//!
//! `wasm-script-notes.md` recorded `::index fts create person:by_name {name}`
//! as "did not parse" and left FTS index creation as "untested on wasm",
//! implying a possible wasm limitation. Both parts are wrong, and for the same
//! reason: the example is a syntax error, and nothing about it is
//! target-specific.
//!
//! Two mistakes in that one line:
//!
//! 1. `::index fts create` — `index_op` is
//!    `{"index" ~ (index_create | index_drop)}` (`cozoscript.pest:20`), so
//!    after `index` only `create`/`drop` may follow. `fts` is a *separate*
//!    top-level sys-op alternative (`fts_idx_op`, `pest:22`), spelled
//!    `::fts create ...`. The working form in the engine's own tests is
//!    `::fts create doc:idx {extractor: body, tokenizer: Simple}`.
//! 2. `{name}` — the body is `index_create_adv`, whose members are
//!    `index_opt_field = {ident ~ ":" ~ expr}`. Every entry needs a `:` and a
//!    value. `{name}` is a bare ident, so the parse stops at the `}`.
//!
//! The wasm-relevant part is the **tokenizer**, not the syntax. FTS itself is
//! not feature-gated (`lib.rs` has an unconditional `pub(crate) mod fts`), and
//! the index build has a sequential fallback (`runtime/relation.rs:1468`,
//! `if threads <= 1 || tuples.len() < 64`) that the wasm target always takes
//! because `build_threads()` resolves to 1 without `available_parallelism`.
//! Only the Cangjie tokenizer is compiled out, by `fts-cangjie` — which the
//! wasm build does not enable (`cozo-lib-wasm/Cargo.toml:20`).

use cozo::{DbInstance, ScriptMutability};
use std::collections::BTreeMap;

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    db.run_script(
        ":create doc {id: Int => body: String}",
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        r#"?[id, body] <- [[1, "the quick brown fox"], [2, "a lazy dog sleeps"],
                           [3, "the fox jumps over the lazy dog"]]
           :put doc {id => body}"#,
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db
}

fn run(db: &DbInstance, s: &str) {
    db.run_script(s, BTreeMap::new(), ScriptMutability::Mutable)
        .unwrap_or_else(|e| panic!("{s}\n  -> {e:?}"));
}

/// The notes' spelling is a parse error, and the error is a *parse* error — not
/// a missing-op error, not a wasm gap. Pinning the distinction matters: the
/// notes read it as "FTS may be unavailable in this build", which would push a
/// reader toward a JS workaround for a feature that works.
#[test]
fn index_prefixed_fts_create_does_not_parse() {
    let db = db();
    let err = db
        .run_script(
            "::index fts create doc:by_body {body}",
            BTreeMap::new(),
            ScriptMutability::Mutable,
        )
        .expect_err("`::index fts` is not a valid sys-op: index_op takes only create/drop");
    let rendered = format!("{err:?}");
    assert!(
        rendered.to_lowercase().contains("parse") || rendered.to_lowercase().contains("pest"),
        "expected a parse error, got: {err}"
    );
}

/// The half-fixed form — right op, wrong body. Each `index_opt_field` needs a
/// `:` and a value, so a bare column name still does not parse.
#[test]
fn bare_column_name_body_does_not_parse() {
    let db = db();
    db.run_script(
        "::fts create doc:by_body {body}",
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .expect_err("a bare `body` is not an `index_opt_field`; the form is `extractor: body`");
}

/// The working form: `::fts create`, an `extractor`, and an explicit tokenizer.
#[test]
fn fts_index_creates_with_extractor_and_tokenizer() {
    let db = db();
    run(
        &db,
        "::fts create doc:by_body {extractor: body, tokenizer: Simple}",
    );
    let idxs = db
        .run_script(
            "::indices doc",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(
        !idxs.rows.is_empty(),
        "the FTS index should be registered on the relation"
    );
}

/// End to end: create the index, then retrieve. This is the part the notes
/// never got to, and the reason it was worth chasing — FTS does work.
#[test]
fn fts_index_answers_a_full_text_query() {
    let db = db();
    run(
        &db,
        "::fts create doc:by_body {extractor: body, tokenizer: Simple}",
    );

    let hits = db
        .run_script(
            "?[id, score] := ~doc:by_body{id | query: \"fox\", k: 10, bind_score: score} :order id",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap_or_else(|e| panic!("fts query: {e:?}"));
    let ids: Vec<i64> = hits.rows.iter().map(|r| r[0].get_int().unwrap()).collect();
    assert_eq!(ids, vec![1, 3], "both 'fox' documents must be found");
}

/// Dropping works too, so the index is a real managed object and not a
/// write-only stub.
#[test]
fn fts_index_can_be_dropped() {
    let db = db();
    run(
        &db,
        "::fts create doc:by_body {extractor: body, tokenizer: Simple}",
    );
    run(&db, "::fts drop doc:by_body");
    let idxs = db
        .run_script(
            "::indices doc",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(idxs.rows.is_empty(), "the index should be gone after drop");
}

/// Tokenizers available on the wasm target. `Raw`, `Simple`, `Whitespace` and
/// `NGram` are unconditional in `fts/mod.rs:96-127`; only `Cangjie` sits behind
/// a `cfg`. Run each so a future change cannot silently remove one.
#[test]
fn non_cangjie_tokenizers_all_construct() {
    for (name, args) in [
        ("Raw", ""),
        ("Simple", ""),
        ("Whitespace", ""),
        ("NGram", "(1, 3)"),
    ] {
        let db = db();
        let create =
            format!("::fts create doc:by_body {{extractor: body, tokenizer: {name}{args}}}");
        db.run_script(&create, BTreeMap::new(), ScriptMutability::Mutable)
            .unwrap_or_else(|e| panic!("tokenizer {name}: {e:?}"));
    }
}

/// Without `fts-cangjie` — the configuration `cozo-lib-wasm` actually builds —
/// `Cangjie` resolves to a clean `bail!` at index-create time
/// (`fts/mod.rs:160-163`), not a panic. The host build has the feature on, so
/// the wasm arm is asserted here by reference rather than executed; the
/// important property is that the failure is a `Result`, because a trap would
/// cost the caller their whole `CozoDb`.
#[test]
fn cangjie_absence_is_a_clean_error_not_a_trap() {
    // Host builds enable `fts-cangjie`, so this can only assert the invariant
    // that holds on both sides: the call returns a Result rather than panicking.
    let db = db();
    let res = db.run_script(
        "::fts create doc:by_body {extractor: body, tokenizer: Cangjie}",
        BTreeMap::new(),
        ScriptMutability::Mutable,
    );
    match res {
        Ok(_) => { /* feature enabled: fine */ }
        Err(e) => {
            let rendered = format!("{e:?}").to_lowercase();
            assert!(
                rendered.contains("cangjie"),
                "the error should name the tokenizer, got: {e}"
            );
        }
    }
}
