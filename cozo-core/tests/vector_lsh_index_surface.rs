/*
 * Copyright 2026, Shan Rizvi (mnestic fork).
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Vector (HNSW) and LSH index surfaces, checked the same way as
//! `fts_index_surface.rs` — because the same class of mistake was suspected in
//! all three.
//!
//! The suspicion: the notes' FTS entry (`::index fts create …`) was a syntax
//! error, not a wasm gap. `fts_idx_op` (`cozoscript.pest:22`),
//! `vec_idx_op` (`pest:21`) and `lsh_idx_op` (`pest:23`) are three *separate*
//! top-level sys-op alternatives, all with the shape
//! `{"<kind>" ~ (index_create_adv | index_drop)}`. None of them accepts an
//! `index` prefix: `index_op` (`pest:20`) is
//! `{"index" ~ (index_create | index_drop)}` and takes only the plain form. So
//! the same trap that made FTS look unavailable would have hidden the other
//! two.
//!
//! It does. And once past the syntax, all three work on wasm: every build path
//! threads through `build_threads()` and guards its fan-out
//! (`runtime/relation.rs:1468` for FTS, `runtime/hnsw_build.rs:255` for HNSW),
//! which resolves to 1 without `available_parallelism` and takes the sequential
//! branch. LSH has no thread fan-out at all. Nothing here is feature-gated.

use cozo::{DbInstance, ScriptMutability};
use std::collections::BTreeMap;

fn run(db: &DbInstance, s: &str) {
    db.run_script(s, BTreeMap::new(), ScriptMutability::Mutable)
        .unwrap_or_else(|e| panic!("{s}\n  -> {e:?}"));
}

/// A vector relation and a text relation, for the two index kinds.
fn vector_db() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    run(&db, ":create pts { id: Int => emb: <F32; 2> }");
    let rows: Vec<String> = (0..64).map(|i| format!("[{i},[{i}.0,0.0]]")).collect();
    run(
        &db,
        &format!(
            "?[id, emb] <- [{}] :put pts {{ id => emb }}",
            rows.join(",")
        ),
    );
    db
}

fn text_db() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    run(&db, ":create doc { id: Int => body: String }");
    run(
        &db,
        r#"?[id, body] <- [[1, "the quick brown fox"], [2, "a lazy dog sleeps"],
                           [3, "quick silver bullet"]]
           :put doc { id => body }"#,
    );
    db
}

/// The trap, verbatim, for both kinds: `index_op` accepts only `create`/`drop`
/// after `index`, so `::index hnsw` / `::index lsh` are not sys-ops at all.
#[test]
fn index_prefixed_vector_and_lsh_create_do_not_parse() {
    for (op, body) in [
        ("hnsw", "pts:idx { dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2, ef_construction: 50 }"),
        ("lsh", "doc:idx { extractor: body, tokenizer: Simple }"),
    ] {
        let db = if op == "hnsw" { vector_db() } else { text_db() };
        let err = db
            .run_script(
                &format!("::index {op} create {body}"),
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .expect_err("`::index {op}` is not a valid sys-op");
        let rendered = format!("{err:?}").to_lowercase();
        assert!(
            rendered.contains("parse") || rendered.contains("pest"),
            "`::index {op}` should be a parse error, got: {err}"
        );
    }
}

/// The bare column-name body fails the same way it does for FTS: every
/// `index_opt_field` needs a `:` and a value.
#[test]
fn bare_column_name_body_does_not_parse() {
    let db = text_db();
    db.run_script(
        "::lsh create doc:idx { body }",
        BTreeMap::new(),
        ScriptMutability::Mutable,
    )
    .expect_err("a bare `body` is not an `index_opt_field`");
}

/// Vector index, end to end: create, then retrieve nearest neighbours. This is
/// the case that was never actually reached before.
#[test]
fn hnsw_index_answers_a_nearest_neighbour_query() {
    let db = vector_db();
    run(
        &db,
        "::hnsw create pts:idx { dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2, ef_construction: 50 }",
    );

    let res = db
        .run_script(
            "?[id, dist] := ~pts:idx{ id | query: vec([40.0, 0.0]), k: 5, ef: 80, bind_distance: dist } :order dist",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap_or_else(|e| panic!("hnsw query: {e:?}"));
    let ids: Vec<i64> = res.rows.iter().map(|r| r[0].get_int().unwrap()).collect();
    assert_eq!(ids.len(), 5, "expected 5 neighbours, got {ids:?}");
    assert_eq!(ids[0], 40, "nearest to x=40 must be id 40; got {ids:?}");
}

/// `ef_construction` and `m_neighbours` are required and fail loudly
/// (`parse/sys.rs:834-839`) rather than defaulting to a degenerate index.
#[test]
fn hnsw_requires_its_two_mandatory_options() {
    let db = vector_db();
    for script in [
        "::hnsw create pts:idx { dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2 }",
        "::hnsw create pts:idx { dim: 2, dtype: F32, fields: [emb], distance: L2, ef_construction: 50 }",
    ] {
        let err = db
            .run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
            .expect_err("missing a mandatory option must be an error");
        let rendered = format!("{err:?}").to_lowercase();
        assert!(
            rendered.contains("ef_construction") || rendered.contains("m_neighbours"),
            "the error should name the missing option, got: {err}"
        );
    }
}

/// `dtype` and `distance` are matched on `opt_val.as_str()` without
/// `eval_to_const` (`parse/sys.rs:796-821`), so they must be written **bare**.
/// The quoted spelling looks equivalent and silently does not match — a
/// third distinct failure mode, and one the FTS pass would not have caught.
#[test]
fn vector_dtype_and_distance_must_be_bare() {
    let db = vector_db();
    run(
        &db,
        "::hnsw create pts:idx { dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2, ef_construction: 50 }",
    );
    let err = db
        .run_script(
            r#"::hnsw create pts:other { dim: 2, m: 16, dtype: "F32", fields: [emb], distance: "L2", ef_construction: 50 }"#,
            BTreeMap::new(),
            ScriptMutability::Mutable,
        )
        .expect_err("a quoted dtype does not match the bare-token match");
    assert!(
        format!("{err:?}").to_lowercase().contains("dtype"),
        "the error should name dtype, got: {err}"
    );
}

/// LSH, end to end. Defaults are `n_gram: 1`, `n_perm: 200`,
/// `target_threshold: 0.9` (`parse/sys.rs:350-354`), so only an extractor and
/// tokenizer are needed.
///
/// The corpus is near-duplicates on purpose. LSH is minhash **similarity**
/// search, not keyword search: at the default 0.9 Jaccard threshold a
/// two-word query against unrelated text correctly returns nothing. Asking it
/// for "quick brown" the way you would ask FTS is a usage error that looks
/// like a broken index — the query runs, returns zero rows, and raises nothing.
#[test]
fn lsh_index_finds_near_duplicates() {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    run(&db, ":create doc { id: Int => body: String }");
    run(
        &db,
        r#"?[id, body] <- [[1, "the quick brown fox jumps over the lazy dog"],
                           [2, "the quick brown fox jumps over the lazy dog"],
                           [3, "entirely unrelated content about databases"]]
           :put doc { id => body }"#,
    );
    run(
        &db,
        "::lsh create doc:idx { extractor: body, tokenizer: Simple }",
    );

    let res = db
        .run_script(
            r#"?[id] := ~doc:idx{ id | query: "the quick brown fox jumps over the lazy dog", k: 5 }"#,
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap_or_else(|e| panic!("lsh query: {e:?}"));
    let ids: Vec<i64> = res.rows.iter().map(|r| r[0].get_int().unwrap()).collect();
    assert!(
        ids.contains(&1) && ids.contains(&2),
        "both near-duplicates must be found; got {ids:?}"
    );
    assert!(
        !ids.contains(&3),
        "the unrelated document must not match; got {ids:?}"
    );
}

/// LSH validates its threshold, and does so as an error rather than a panic.
#[test]
fn lsh_rejects_an_out_of_range_threshold() {
    let db = text_db();
    for bad in ["0", "1", "1.5"] {
        db.run_script(
            &format!(
                "::lsh create doc:idx {{ extractor: body, tokenizer: Simple, target_threshold: {bad} }}"
            ),
            BTreeMap::new(),
            ScriptMutability::Mutable,
        )
        .expect_err("target_threshold outside (0, 1) must be rejected");
    }
}

/// Drop works for both, so they are managed objects and not write-only stubs.
#[test]
fn vector_and_lsh_indices_can_be_dropped() {
    let db = vector_db();
    run(&db, "::hnsw create pts:idx { dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2, ef_construction: 50 }");
    run(&db, "::hnsw drop pts:idx");

    let db = text_db();
    run(
        &db,
        "::lsh create doc:idx { extractor: body, tokenizer: Simple }",
    );
    run(&db, "::lsh drop doc:idx");
}
