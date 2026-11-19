// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The cursors against a model of bbolt v1.3.12's cursor written from its
//! source (spec 01 §5.7, spec 04 §5.1), on random key sets and random
//! sequences of moves. The comparison against bbolt itself, through the
//! oracle, is the cursor-conformance test of brief §6 Phase 2
//! (`tests/conformance.rs`).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: model indices over small key sets"
)]

use std::path::{Path, PathBuf};

use purse_store::{Error, RawEntry, Store, TableId};
use purse_testutil::SplitMix64;

fn path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("purse-store-cursor");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    p
}

/// bbolt's cursor on one bucket, reduced to what a caller observes: the
/// index of the element the cursor's leaf reference points at, over the
/// sorted keys. Empty leaves, which bbolt skips, are not modelled; they
/// change which page a reference names, not what a move returns.
struct BoltModel {
    keys: Vec<Vec<u8>>,
    /// `None`: never positioned. Otherwise the leaf index, which may be -1
    /// (`Last` on an empty bucket) or `len` (`Seek` past the end).
    i: Option<isize>,
}

impl BoltModel {
    fn at(&self, i: isize) -> Option<Vec<u8>> {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.keys.get(i).cloned())
    }

    fn n(&self) -> isize {
        self.keys.len() as isize
    }

    /// `First` (cursor.go:41-58).
    fn first(&mut self) -> Option<Vec<u8>> {
        self.i = Some(0);
        self.at(0)
    }

    /// `Last` (cursor.go:63-87): the last index, -1 when empty.
    fn last(&mut self) -> Option<Vec<u8>> {
        let i = self.n() - 1;
        self.i = Some(i);
        self.at(i)
    }

    /// `Next` (cursor.go:92-99, 212-244): advance if the index can, else
    /// return nil and stay.
    fn next(&mut self) -> Option<Vec<u8>> {
        let i = self.i?;
        if i < self.n() - 1 {
            self.i = Some(i + 1);
            self.at(i + 1)
        } else {
            None
        }
    }

    /// `Prev` (cursor.go:104-111, 248-277): step back if the index is above
    /// 0, else reposition on the first element and return nil.
    fn prev(&mut self) -> Option<Vec<u8>> {
        let i = self.i?;
        if i > 0 {
            self.i = Some(i - 1);
            self.at(i - 1)
        } else {
            self.i = Some(0);
            None
        }
    }

    /// `Seek` (cursor.go:117-133): the first index with key >= k; past the
    /// end, `Next` cannot advance, so nil with the index left at `len`.
    fn seek(&mut self, k: &[u8]) -> Option<Vec<u8>> {
        let i = self.keys.partition_point(|x| x.as_slice() < k) as isize;
        self.i = Some(i);
        self.at(i)
    }
}

#[derive(Clone, Debug)]
enum Op {
    First,
    Last,
    Next,
    Prev,
    Seek(Vec<u8>),
}

fn key_of(r: &mut SplitMix64) -> Vec<u8> {
    // Short keys over a small alphabet: prefixes and neighbours are common.
    let n = r.below(4) as usize;
    (0..n).map(|_| r.below(4) as u8 * 0x40).collect()
}

/// A key to store: bbolt and the store refuse the empty key.
fn stored_key_of(r: &mut SplitMix64) -> Vec<u8> {
    let n = 1 + r.below(3) as usize;
    (0..n).map(|_| r.below(4) as u8 * 0x40).collect()
}

fn op_of(r: &mut SplitMix64) -> Op {
    match r.below(8) {
        0 => Op::First,
        1 => Op::Last,
        2 | 3 => Op::Next,
        4 | 5 => Op::Prev,
        _ => Op::Seek(key_of(r)),
    }
}

fn keys(e: Option<RawEntry>) -> Option<Vec<u8>> {
    e.map(|(k, v)| {
        assert_eq!(v, value_of(&k));
        k
    })
}

fn value_of(k: &[u8]) -> Vec<u8> {
    let mut v = b"v".to_vec();
    v.extend_from_slice(k);
    v
}

#[test]
fn cursors_match_the_bbolt_model() {
    // In memory: the test makes 600 commits and needs none of them on disk.
    let store = purse_store::OpenOptions::new()
        .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
        .unwrap();
    let mut r = SplitMix64::from_env("cursor-model");
    for round in 0..300 {
        let mut set: Vec<Vec<u8>> = (0..r.below(12)).map(|_| stored_key_of(&mut r)).collect();
        set.sort();
        set.dedup();
        store
            .update(|tx| {
                while tx.cursor(TableId::Tickets)?.first()?.is_some() {
                    let mut c = tx.cursor(TableId::Tickets)?;
                    c.first()?;
                    c.delete()?;
                }
                for k in &set {
                    tx.put_raw(TableId::Tickets, k, &value_of(k))?;
                }
                Ok::<_, Error>(())
            })
            .unwrap();
        let ops: Vec<Op> = (0..40).map(|_| op_of(&mut r)).collect();

        let mut model = BoltModel {
            keys: set.clone(),
            i: None,
        };
        let want: Vec<Option<Vec<u8>>> = ops
            .iter()
            .map(|op| match op {
                Op::First => model.first(),
                Op::Last => model.last(),
                Op::Next => model.next(),
                Op::Prev => model.prev(),
                Op::Seek(k) => model.seek(k),
            })
            .collect();

        let tx = store.read().unwrap();
        let mut c = tx.cursor(TableId::Tickets).unwrap();
        let got: Vec<Option<Vec<u8>>> = ops
            .iter()
            .map(|op| {
                keys(
                    match op {
                        Op::First => c.first(),
                        Op::Last => c.last(),
                        Op::Next => c.next(),
                        Op::Prev => c.prev(),
                        Op::Seek(k) => c.seek(k),
                    }
                    .unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got, want,
            "read cursor, round {round}, keys {set:?}, ops {ops:?}"
        );

        let got: Vec<Option<Vec<u8>>> = store
            .update(|tx| {
                let mut c = tx.cursor(TableId::Tickets)?;
                ops.iter()
                    .map(|op| {
                        Ok(keys(match op {
                            Op::First => c.first()?,
                            Op::Last => c.last()?,
                            Op::Next => c.next()?,
                            Op::Prev => c.prev()?,
                            Op::Seek(k) => c.seek(k)?,
                        }))
                    })
                    .collect::<Result<_, Error>>()
            })
            .unwrap();
        assert_eq!(
            got, want,
            "write cursor, round {round}, keys {set:?}, ops {ops:?}"
        );
    }
}

fn store_with(name: &str, keys: &[&[u8]]) -> Store {
    let store = Store::create(&path(name)).unwrap();
    store
        .update(|tx| {
            for k in keys {
                tx.put_raw(TableId::Unspent, k, &value_of(k))?;
            }
            Ok::<_, Error>(())
        })
        .unwrap();
    store
}

/// The cases of spec 04 §5.1 the input selection relies on, spelled out.
#[test]
fn spec_04_cases() {
    let store = store_with("spec04.redb", &[b"b", b"d", b"f"]);
    let tx = store.read().unwrap();
    let mut c = tx.cursor(TableId::Unspent).unwrap();
    let k = |e: purse_store::Result<Option<RawEntry>>| e.unwrap().map(|(k, _)| k);
    // Seek: first key >= k.
    assert_eq!(k(c.seek(b"c")), Some(b"d".to_vec()));
    assert_eq!(k(c.seek(b"d")), Some(b"d".to_vec()));
    assert_eq!(k(c.seek(b"")), Some(b"b".to_vec()));
    // Prev right after Seek: the greatest key below the one Seek returned.
    assert_eq!(k(c.seek(b"c")), Some(b"d".to_vec()));
    assert_eq!(k(c.prev()), Some(b"b".to_vec()));
    // Previous of end is last.
    assert_eq!(k(c.seek(b"g")), None);
    assert_eq!(k(c.prev()), Some(b"f".to_vec()));
    // Next past the end stays on the last element.
    assert_eq!(k(c.next()), None);
    assert_eq!(k(c.prev()), Some(b"d".to_vec()));
    // Prev from the first returns nil and stays on the first.
    assert_eq!(k(c.first()), Some(b"b".to_vec()));
    assert_eq!(k(c.prev()), None);
    assert_eq!(k(c.prev()), None);
    assert_eq!(k(c.next()), Some(b"d".to_vec()));
    // Seek past the end, then Next: still nil.
    assert_eq!(k(c.seek(b"z")), None);
    assert_eq!(k(c.next()), None);
    assert_eq!(k(c.prev()), Some(b"f".to_vec()));

    let empty = store_with("spec04-empty.redb", &[]);
    let tx = empty.read().unwrap();
    let mut c = tx.cursor(TableId::Unspent).unwrap();
    assert_eq!(k(c.next()), None);
    assert_eq!(k(c.prev()), None);
    assert_eq!(k(c.first()), None);
    assert_eq!(k(c.prev()), None);
    assert_eq!(k(c.last()), None);
    assert_eq!(k(c.next()), None);
    assert_eq!(k(c.seek(b"a")), None);
    assert_eq!(k(c.prev()), None);
}

/// The write cursor deletes the entry it stands on without moving; the
/// next move steps from the deleted key's place.
#[test]
fn write_cursor_deletes_while_iterating() {
    let store = store_with("delete.redb", &[b"a", b"b", b"c", b"d", b"e"]);
    store
        .update(|tx| {
            let mut c = tx.cursor(TableId::Unspent)?;
            let mut seen = Vec::new();
            let mut e = c.first()?;
            while let Some((k, _)) = e {
                seen.push(k.clone());
                // Delete every other entry while walking forward.
                if k == b"b" || k == b"d" {
                    assert!(c.delete()?);
                    assert!(!c.delete()?, "a second delete finds nothing");
                }
                e = c.next()?;
            }
            assert_eq!(seen, [b"a", b"b", b"c", b"d", b"e"].map(|k| k.to_vec()));
            // Walking back from the end sees what is left.
            let mut back = Vec::new();
            let mut e = c.last()?;
            while let Some((k, _)) = e {
                back.push(k);
                e = c.prev()?;
            }
            assert_eq!(back, [b"e", b"c", b"a"].map(|k| k.to_vec()));
            // Off the start the cursor stands on the first entry, as
            // bbolt's does: delete removes it.
            assert!(c.delete()?);
            assert!(!c.delete()?, "a second delete finds nothing");
            // After a seek that found nothing, nothing is deleted.
            assert_eq!(c.seek(b"zz")?, None);
            assert!(!c.delete()?);
            // Delete then step back.
            assert_eq!(c.seek(b"e")?.map(|e| e.0), Some(b"e".to_vec()));
            assert!(c.delete()?);
            assert_eq!(c.prev()?.map(|e| e.0), Some(b"c".to_vec()));
            Ok::<_, Error>(())
        })
        .unwrap();
    let left: Vec<Vec<u8>> = store
        .read()
        .unwrap()
        .range_raw(
            TableId::Unspent,
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )
        .unwrap()
        .map(|e| e.unwrap().0)
        .collect();
    assert_eq!(left, [b"c".to_vec()]);
}
