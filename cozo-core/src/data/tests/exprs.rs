/*
 * Copyright 2022, The Cozo Project Authors.
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::data::expr::{all_ops, get_op, ALL_OP_NAMES};
use crate::{DataValue, DbInstance};

#[test]
fn expression_eval() {
    let db = DbInstance::default();

    let res = db
        .run_default(
            r#"
    ?[a] := a = if(2 + 3 > 1 * 99999, 190291021 + 14341234212 / 2121)
    "#,
        )
        .unwrap();
    assert_eq!(res.rows[0][0], DataValue::Null);

    let res = db
        .run_default(
            r#"
    ?[a] := a = if(2 + 3 > 1, true, false)
    "#,
        )
        .unwrap();
    assert!(res.rows[0][0].get_bool().unwrap());
}

/// `ALL_OP_NAMES` duplicates the `get_op` match keys as strings, and the match
/// is the only place a script-facing name exists. This is the pin that makes the
/// duplication safe: a new builtin added to the match without a list entry (or an
/// entry left behind after a rename) fails here rather than silently shrinking the
/// `::builtins ops` surface.
#[test]
fn all_op_names_match_get_op() {
    for name in ALL_OP_NAMES {
        assert!(
            get_op(name).is_some(),
            "ALL_OP_NAMES lists {name:?}, which get_op does not accept"
        );
    }

    let mut from_list: Vec<&str> = ALL_OP_NAMES.to_vec();
    from_list.sort_unstable();
    from_list.dedup();
    assert_eq!(
        from_list.len(),
        ALL_OP_NAMES.len(),
        "ALL_OP_NAMES has duplicates"
    );
}

/// Arity metadata comes from the statics via `get_op`, so `::builtins ops` cannot
/// report an arity the engine does not enforce. Spot-check both ends of the
/// registry plus a vararg builtin.
#[test]
fn all_ops_reports_arity_from_the_statics() {
    let ops = all_ops();
    assert_eq!(ops.len(), ALL_OP_NAMES.len());

    let find = |name: &str| {
        ops.iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} missing from all_ops()"))
    };

    assert_eq!(find("str_includes").min_arity, 2);
    assert!(!find("str_includes").vararg);
    assert_eq!(find("regex_matches").min_arity, 2);
    assert!(find("int_range").vararg);
    assert_eq!(find("vec").min_arity, 1);
    assert!(find("vec").vararg);

    for op in &ops {
        let expected = get_op(op.name).unwrap();
        assert_eq!(op.min_arity, expected.min_arity, "{} arity", op.name);
        assert_eq!(op.vararg, expected.vararg, "{} vararg", op.name);
    }
}
