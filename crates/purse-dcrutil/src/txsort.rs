// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2016 The btcsuite developers
// Copyright (c) 2017 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrutil's `txsort` package: the canonical order of a transaction's
//! inputs and outputs, a Decred variant of BIP69.
//!
//! Inputs are ordered by previous outpoint tree (ascending, as a signed
//! `int8`), then, for different hashes, by the hash read as a big-endian
//! number (the bytes compared from the last in memory to the first, as the
//! hash is displayed), and for equal hashes by output index. Outputs are
//! ordered by value, then script version, then script bytes.
//!
//! # Ties
//!
//! Go sorts with `sort.Sort`, which is not stable. Two outputs that compare
//! equal are identical (value, version and script are all a `TxOut` has),
//! so their order cannot be observed. Two inputs compare equal when they
//! spend the same outpoint, yet can differ in sequence, signature script
//! and the witness's value, height and index, so their relative order is
//! visible in the serialized transaction. Such a transaction double-spends
//! and is invalid, but `InPlaceSort` runs before any check (the mixing
//! client sorts a coinjoin built from peers' inputs). Go's `sort.Sort` is
//! deterministic, so this crate ports it ([`crate::gosort`]) rather than
//! using Rust's sorts, and leaves equal inputs in exactly Go's order.
//!
//! A negative tree, which `MsgTx`'s serialization cannot carry (dcrd
//! refuses it on reading), sorts before tree 0, as in Go.

use dcroxide_wire::{MsgTx, TxIn, TxOut};

use crate::gosort::{is_sorted_by, sort_by};

/// `sortableInputSlice.Less` (`txsort.go:39-60`).
fn input_less(a: &TxIn, b: &TxIn) -> bool {
    let (x, y) = (&a.previous_out_point, &b.previous_out_point);
    if x.tree != y.tree {
        return x.tree < y.tree;
    }
    if x.hash == y.hash {
        return x.index < y.index;
    }
    x.hash.0.iter().rev().lt(y.hash.0.iter().rev())
}

/// `sortableOutputSlice.Less` (`txsort.go:85-98`).
fn output_less(a: &TxOut, b: &TxOut) -> bool {
    if a.value != b.value {
        return a.value < b.value;
    }
    if a.version != b.version {
        return a.version < b.version;
    }
    a.pk_script < b.pk_script
}

/// Go's `InPlaceSort` (`txsort.go:112-115`): sorts `tx`'s inputs, then its
/// outputs, in place.
///
/// As dcrd warns, this changes the transaction's hash unless it was
/// already sorted; call it only on a transaction being built.
pub fn in_place_sort(tx: &mut MsgTx) {
    sort_by(&mut tx.tx_in, input_less);
    sort_by(&mut tx.tx_out, output_less);
}

/// Go's `Sort` (`txsort.go:121-126`): a sorted copy of `tx`, which is left
/// unchanged.
pub fn sort(tx: &MsgTx) -> MsgTx {
    let mut sorted = tx.clone();
    in_place_sort(&mut sorted);
    sorted
}

/// Go's `IsSorted` (`txsort.go:130-138`): whether `tx`'s inputs and outputs
/// are both in the canonical order.
pub fn is_sorted(tx: &MsgTx) -> bool {
    is_sorted_by(&tx.tx_in, input_less) && is_sorted_by(&tx.tx_out, output_less)
}
