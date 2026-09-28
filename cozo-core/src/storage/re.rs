/*
 * Copyright 2022, The Cozo Project Authors.
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Pure-Rust persistent storage backend on [redb](https://github.com/cberner/redb).
//!
//! Ported from `lawless-m/cozo-redb` (`storage/re.rs`) and adapted to the
//! mnestic [`Storage`](crate::storage::Storage)/[`StoreTx`](crate::storage::StoreTx)
//! traits. The redb call patterns are theirs (proven by their CI); the shaping
//! around them is ours:
//!
//! * `get`/`exists` take the ignored `_for_update` flag, like [`MemStorage`](crate::storage::mem::MemStorage)
//!   — redb serialises writers internally, so there is nothing to lock.
//! * the bounded [`StoreTx::range_scan`] and the full-table [`StoreTx::total_scan`]
//!   share one private [`RedbTx::scan`] engine over explicit `Bound`s, instead
//!   of two near-identical matches.
//! * [`StoreTx::batch_put`](crate::storage::Storage::batch_put) runs through one
//!   write transaction; [`StoreTx::supports_par_put`] is `false`.
//!
//! Design notes (inherited):
//!
//! * One table (`"cozo"`) holds the whole engine keyspace; relations live in
//!   disjoint key prefixes, so no per-relation tables are needed.
//! * redb has a single writer: a second concurrent write transaction fails at
//!   `begin_write`. The engine serialises system writes, so this matches.
//! * Time travel works through [`check_key_for_validity`], the same helper the
//!   other backends use — no redb-specific validity logic.

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use miette::{bail, miette, IntoDiagnostic, Result};
use redb::backends::InMemoryBackend;
use redb::{
    Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction,
};
// NOTE: `ReadableTable` looks unused but is load-bearing — redb 4.x exposes
// `get`/`range` on the write-`Table` only through this trait. Removing the
// import breaks the build with E0599 on every `Write(Some(..))` arm.

use crate::data::tuple::{check_key_for_validity, Tuple};
use crate::data::value::ValidityTs;
use crate::runtime::relation::try_extend_tuple_from_v;
use crate::storage::{Storage, StoreTx};
use crate::utils::swap_option_result;
use crate::Db;

const TABLE: TableDefinition<'_, &[u8], &[u8]> = TableDefinition::new("cozo");

/// Creates a redb-backed database at `path`, creating the file if missing.
/// ACID, single-file, no C++ and no background threads.
pub fn new_cozo_redb(path: impl AsRef<Path>) -> Result<Db<RedbStorage>> {
    let db = Database::create(path).into_diagnostic()?;
    finish_redb(db)
}

/// Creates a redb-backed database with no file behind it. No filesystem or
/// mmap involvement — for tests and for embeddings that want the redb engine
/// without persistence.
pub fn new_cozo_redb_mem() -> Result<Db<RedbStorage>> {
    let db = Database::builder()
        .create_with_backend(InMemoryBackend::new())
        .into_diagnostic()?;
    finish_redb(db)
}

fn finish_redb(db: Database) -> Result<Db<RedbStorage>> {
    {
        let tx = db.begin_write().into_diagnostic()?;
        tx.open_table(TABLE).into_diagnostic()?;
        tx.commit().into_diagnostic()?;
    }
    let ret = Db::new(RedbStorage { db: Arc::new(db) })?;
    ret.initialize()?;
    Ok(ret)
}

/// Storage engine using redb
#[derive(Clone)]
pub struct RedbStorage {
    db: Arc<Database>,
}

impl<'s> Storage<'s> for RedbStorage {
    type Tx = RedbTx;

    fn storage_kind(&self) -> &'static str {
        "redb"
    }

    fn transact(&'s self, write: bool) -> Result<Self::Tx> {
        if write {
            let tx = self.db.begin_write().into_diagnostic()?;
            Ok(RedbTx::Write(Some(Box::new(tx))))
        } else {
            let tx = self.db.begin_read().into_diagnostic()?;
            Ok(RedbTx::Read(tx))
        }
    }

    fn range_compact(&'s self, _lower: &[u8], _upper: &[u8]) -> Result<()> {
        Ok(())
    }

    fn batch_put<'a>(
        &'a self,
        data: Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>)>> + 'a>,
    ) -> Result<()> {
        let mut tx = self.transact(true)?;
        for pair in data {
            let (k, v) = pair?;
            tx.put(&k, &v)?;
        }
        tx.commit()
    }
}

pub enum RedbTx {
    Read(ReadTransaction),
    // Boxed: `WriteTransaction` is ~700 bytes and the `Read` arm is small —
    // without the box every `RedbTx` moved per query pays for the big arm.
    Write(Option<Box<WriteTransaction>>),
}

// SAFETY: redb's `WriteTransaction` is `Send` but not `Sync` because its
// internal state is mutated when opening tables / performing writes. The
// `StoreTx` trait requires `Sync` so the engine can hand a borrow across
// threads during range scans, but every mutator in this impl takes
// `&mut self`, and read-only methods only perform operations that redb
// itself synchronises internally (table opens take a shared lock). We
// never expose a shared reference to the inner `WriteTransaction` across
// threads in a way that races with mutation, so this impl is sound.
unsafe impl Sync for RedbTx {}

/// Bounds for [`RedbTx::scan`]: an inclusive-lower/exclusive-upper range, or
/// the whole table (`Unbounded`/`Unbounded`) for [`StoreTx::total_scan`].
type KvBounds<'b> = (Bound<&'b [u8]>, Bound<&'b [u8]>);

/// The "already committed" arm of every `Write(None)` match below.
fn already_committed<T>() -> Result<T> {
    bail!("transaction already committed")
}

impl<'s> StoreTx<'s> for RedbTx {
    fn get(&self, key: &[u8], _for_update: bool) -> Result<Option<Vec<u8>>> {
        match self {
            RedbTx::Read(tx) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                Ok(table
                    .get(key)
                    .into_diagnostic()?
                    .map(|v| v.value().to_vec()))
            }
            RedbTx::Write(Some(tx)) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                // NOTE: bind to a local before returning so `table`'s
                // `AccessGuard` drops before the `Ok(...)` wrap; collapsing
                // this into a single expression extends the borrow and fails
                // to compile (see E0597).
                let result = table
                    .get(key)
                    .into_diagnostic()?
                    .map(|v| v.value().to_vec());
                Ok(result)
            }
            RedbTx::Write(None) => already_committed(),
        }
    }

    fn put(&mut self, key: &[u8], val: &[u8]) -> Result<()> {
        match self {
            RedbTx::Read(_) => bail!("write in read transaction"),
            RedbTx::Write(Some(tx)) => {
                let mut table = tx.open_table(TABLE).into_diagnostic()?;
                table.insert(key, val).into_diagnostic()?;
                Ok(())
            }
            RedbTx::Write(None) => already_committed(),
        }
    }

    fn supports_par_put(&self) -> bool {
        false
    }

    fn del(&mut self, key: &[u8]) -> Result<()> {
        match self {
            RedbTx::Read(_) => bail!("write in read transaction"),
            RedbTx::Write(Some(tx)) => {
                let mut table = tx.open_table(TABLE).into_diagnostic()?;
                table.remove(key).into_diagnostic()?;
                Ok(())
            }
            RedbTx::Write(None) => already_committed(),
        }
    }

    fn del_range_from_persisted(&mut self, lower: &[u8], upper: &[u8]) -> Result<()> {
        match self {
            RedbTx::Read(_) => bail!("write in read transaction"),
            RedbTx::Write(Some(tx)) => {
                let mut table = tx.open_table(TABLE).into_diagnostic()?;
                table
                    .retain_in::<&[u8], _>(lower..upper, |_, _| false)
                    .into_diagnostic()?;
                Ok(())
            }
            RedbTx::Write(None) => already_committed(),
        }
    }

    fn exists(&self, key: &[u8], _for_update: bool) -> Result<bool> {
        match self {
            RedbTx::Read(tx) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                Ok(table.get(key).into_diagnostic()?.is_some())
            }
            RedbTx::Write(Some(tx)) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                // NOTE: see `get()` — the intermediate binding forces
                // `table`'s `AccessGuard` to drop before `Ok` wraps the bool.
                let result = table.get(key).into_diagnostic()?.is_some();
                Ok(result)
            }
            RedbTx::Write(None) => already_committed(),
        }
    }

    fn commit(&mut self) -> Result<()> {
        match self {
            RedbTx::Read(_) => Ok(()),
            RedbTx::Write(tx) => {
                if let Some(tx) = tx.take() {
                    (*tx).commit().into_diagnostic()?;
                }
                Ok(())
            }
        }
    }

    fn range_scan<'a>(
        &'a self,
        lower: &[u8],
        upper: &[u8],
    ) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>)>> + 'a>
    where
        's: 'a,
    {
        self.scan((Bound::Included(lower), Bound::Excluded(upper)))
    }

    fn total_scan<'a>(&'a self) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>)>> + 'a>
    where
        's: 'a,
    {
        self.scan((Bound::Unbounded, Bound::Unbounded))
    }

    fn range_skip_scan_tuple<'a>(
        &'a self,
        lower: &[u8],
        upper: &[u8],
        valid_at: ValidityTs,
    ) -> Box<dyn Iterator<Item = Result<Tuple>> + 'a> {
        Box::new(RedbSkipIterator {
            tx: self,
            upper: upper.to_vec(),
            valid_at,
            next_bound: lower.to_vec(),
        })
    }

    fn range_count<'a>(&'a self, lower: &[u8], upper: &[u8]) -> Result<usize>
    where
        's: 'a,
    {
        // Counts straight off the table without materialising owned pairs —
        // deliberately not routed through `scan()`.
        match self {
            RedbTx::Read(tx) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                Ok(table
                    .range::<&[u8]>(lower..upper)
                    .into_diagnostic()?
                    .count())
            }
            RedbTx::Write(Some(tx)) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                Ok(table
                    .range::<&[u8]>(lower..upper)
                    .into_diagnostic()?
                    .count())
            }
            RedbTx::Write(None) => already_committed(),
        }
    }
}

impl RedbTx {
    /// Shared engine for [`StoreTx::range_scan`] (bounded) and
    /// [`StoreTx::total_scan`] (unbounded).
    ///
    /// Read transactions stream straight off the table; write transactions
    /// materialise first — `Table::range` borrows the write transaction, so a
    /// streaming iterator could never outlive this call (reads on the `Read`
    /// branch have no such borrow and do stream).
    fn scan(
        &self,
        bounds: KvBounds<'_>,
    ) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>)>> + '_> {
        match self {
            RedbTx::Read(tx) => {
                let table = match tx.open_table(TABLE) {
                    Ok(t) => t,
                    Err(e) => return Box::new(std::iter::once(Err(miette!("{e}")))),
                };
                match table.range::<&[u8]>(bounds) {
                    Ok(iter) => Box::new(iter.map(|r| match r {
                        Ok(entry) => Ok((entry.0.value().to_vec(), entry.1.value().to_vec())),
                        Err(e) => Err(miette!("{e}")),
                    })),
                    Err(e) => Box::new(std::iter::once(Err(miette!("{e}")))),
                }
            }
            RedbTx::Write(Some(tx)) => {
                let table = match tx.open_table(TABLE) {
                    Ok(t) => t,
                    Err(e) => return Box::new(std::iter::once(Err(miette!("{e}")))),
                };
                match table.range::<&[u8]>(bounds) {
                    Ok(iter) => Box::new(
                        iter.map(|r| match r {
                            Ok(entry) => Ok((entry.0.value().to_vec(), entry.1.value().to_vec())),
                            Err(e) => Err(miette!("{e}")),
                        })
                        .collect::<Vec<_>>()
                        .into_iter(),
                    ),
                    Err(e) => Box::new(std::iter::once(Err(miette!("{e}")))),
                }
            }
            RedbTx::Write(None) => Box::new(std::iter::once(already_committed())),
        }
    }

    fn seek_one(&self, lower: &[u8], upper: &[u8]) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        match self {
            RedbTx::Read(tx) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                match table.range::<&[u8]>(lower..upper).into_diagnostic()?.next() {
                    None => Ok(None),
                    Some(r) => {
                        let entry = r.into_diagnostic()?;
                        Ok(Some((entry.0.value().to_vec(), entry.1.value().to_vec())))
                    }
                }
            }
            RedbTx::Write(Some(tx)) => {
                let table = tx.open_table(TABLE).into_diagnostic()?;
                let result = match table.range::<&[u8]>(lower..upper).into_diagnostic()?.next() {
                    None => None,
                    Some(r) => {
                        let entry = r.into_diagnostic()?;
                        Some((entry.0.value().to_vec(), entry.1.value().to_vec()))
                    }
                };
                Ok(result)
            }
            RedbTx::Write(None) => already_committed(),
        }
    }
}

struct RedbSkipIterator<'a> {
    tx: &'a RedbTx,
    upper: Vec<u8>,
    valid_at: ValidityTs,
    next_bound: Vec<u8>,
}

impl RedbSkipIterator<'_> {
    #[inline]
    fn next_inner(&mut self) -> Result<Option<Tuple>> {
        loop {
            match self.tx.seek_one(&self.next_bound, &self.upper)? {
                None => return Ok(None),
                Some((candidate_key, candidate_val)) => {
                    let (ret, nxt_bound) =
                        check_key_for_validity(&candidate_key, self.valid_at, None);
                    self.next_bound = nxt_bound;
                    if let Some(mut nk) = ret {
                        try_extend_tuple_from_v(&mut nk, &candidate_val)?;
                        return Ok(Some(nk));
                    }
                }
            }
        }
    }
}

impl Iterator for RedbSkipIterator<'_> {
    type Item = Result<Tuple>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        swap_option_result(self.next_inner())
    }
}

#[cfg(test)]
mod tests {
    use crate::data::value::DataValue;
    use crate::runtime::db::ScriptMutability;
    use miette::{IntoDiagnostic, Result};
    use tempfile::TempDir;

    use super::*;

    fn setup_test_db() -> Result<(TempDir, crate::Db<RedbStorage>)> {
        let temp_dir = TempDir::new().into_diagnostic()?;
        let db = new_cozo_redb(temp_dir.path().join("test.redb"))?;
        db.run_script(
            r#"
            {:create plain {k: Int => v}}
            {:create tt {k: Int, vld: Validity => v}}
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        )?;
        Ok((temp_dir, db))
    }

    fn run(db: &crate::Db<RedbStorage>, q: &str) -> Result<crate::NamedRows> {
        db.run_script(q, Default::default(), ScriptMutability::Mutable)
    }

    #[test]
    fn test_basic_operations() -> Result<()> {
        let (_tmp, db) = setup_test_db()?;

        run(
            &db,
            "?[k, v] <- [[1, 'a'], [2, 'b'], [3, 'c']] :put plain {k => v}",
        )?;
        let result = run(&db, "?[k, v] := *plain{k, v}")?;
        assert_eq!(result.rows.len(), 3);

        run(&db, "?[k, v] <- [[2, 'updated']] :put plain {k => v}")?;
        let result = run(&db, "?[v] := *plain{k: 2, v}")?;
        assert_eq!(result.rows[0][0], DataValue::from("updated"));

        Ok(())
    }

    #[test]
    fn test_delete() -> Result<()> {
        let (_tmp, db) = setup_test_db()?;

        run(
            &db,
            "?[k, v] <- [[1, 'a'], [2, 'b'], [3, 'c']] :put plain {k => v}",
        )?;

        let result = run(
            &db,
            r#"
            {?[k] <- [[2]] :rm plain {k}}
            {?[k, v] := *plain{k, v}}
        "#,
        )?;
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], DataValue::from(1));
        assert_eq!(result.rows[1][0], DataValue::from(3));

        Ok(())
    }

    #[test]
    fn test_time_travel() -> Result<()> {
        let (_tmp, db) = setup_test_db()?;

        // Upstream seeds this via `import_relations`; that API is gone here,
        // so the rows go in through the normal `:put` write path with explicit
        // validity pairs (microsecond stamps, well clear of pre-epoch edges).
        run(
            &db,
            "?[k, vld, v] <- [[1, [1000000, true], 10], [1, [5000000, true], 15],
                               [2, [1000000, true], 20], [2, [5000000, true], 25]]
             :put tt {k, vld => v}",
        )?;

        let r = run(&db, "?[k, v] := *tt{k, v @ 1000000}")?;
        assert_eq!(
            r.rows,
            vec![
                vec![DataValue::from(1), DataValue::from(10)],
                vec![DataValue::from(2), DataValue::from(20)],
            ]
        );

        let r = run(&db, "?[k, v] := *tt{k, v @ 5000000}")?;
        assert_eq!(
            r.rows,
            vec![
                vec![DataValue::from(1), DataValue::from(15)],
                vec![DataValue::from(2), DataValue::from(25)],
            ]
        );

        Ok(())
    }

    #[test]
    fn test_fts_roundtrip() -> Result<()> {
        // Spelled in our FTS dialect (`extractor:` + explicit tokenizer), not
        // upstream's `{fields: [...]}` — this doubles as a check that the new
        // custom FTS index is backend-agnostic and rides on redb unchanged.
        let temp_dir = TempDir::new().into_diagnostic()?;
        let db = new_cozo_redb(temp_dir.path().join("test.redb"))?;

        run(&db, ":create doc {id: Int => body: String}")?;
        run(
            &db,
            r#"?[id, body] <- [[1, "the quick brown fox"], [2, "a lazy dog sleeps"],
                               [3, "the fox jumps over the lazy dog"]]
               :put doc {id => body}"#,
        )?;
        run(
            &db,
            "::fts create doc:by_body {extractor: body, tokenizer: Simple}",
        )?;

        let hits = run(
            &db,
            r#"?[id, score] := ~doc:by_body{id | query: "fox", k: 10, bind_score: score} :order id"#,
        )?;
        let ids: Vec<i64> = hits
            .rows
            .iter()
            .map(|r| match &r[0] {
                DataValue::Num(n) => n.get_int().unwrap(),
                other => panic!("expected int id, got {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec![1, 3]);

        run(&db, "::fts drop doc:by_body")?;

        Ok(())
    }
}
