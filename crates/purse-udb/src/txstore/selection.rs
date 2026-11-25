// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Input selection and `Spender`: `RandomUTXO` with `randomUTXO`
//! (`txmined.go:2356-2459`), `MakeInputSource` and
//! `InputSource.SelectInputs` (`txmined.go:3010-3350`), and `Spender`
//! (`txquery.go:640-725`).
//!
//! # The random stream
//!
//! Go draws from dcrd's `crypto/rand` package generator: `randomUTXO` reads
//! 33 bytes (a 32-byte seek key and a direction bit) and, when it found
//! candidates, one `IntN` over their number; the input source's enumeration
//! phase shuffles the remaining outputs with `ShuffleSlice`. The port takes
//! the stream as a [`RandomSource`] (the address manager's seam) and
//! derives `IntN` and `ShuffleSlice` from it exactly as `crypto/rand`'s
//! `uniform.go` does (`Uint64N`'s multiply-shift with its rejection loop,
//! the Fisher-Yates walk from the top), so a source that yields the bytes
//! Go's generator yields makes the same choices, draw for draw (spec 23
//! §5.6.2; `crates/purse-udb/tests/oracle.rs` checks the selections and the
//! bytes drawn against Go's on a seeded stream). Every buffer is zeroed
//! before it is filled, as Go's callers pass fresh buffers to a `Read` that
//! XORs.
//!
//! # bbolt's cursor at the bucket ends
//!
//! `randomUTXO` seeks its random key in `unspent` and walks forward with
//! `Next`, or steps back first with `Prev` and walks backward. The walk's
//! order is what bbolt's cursor gives (spec 04 §5.1, Q-31; `purse-store`'s
//! conformance tests hold its `ReadCursor` to bbolt): a seek lands on the
//! first key at or above the 32-byte seek key, and a `Prev` after it gives
//! the key below, or the last key when the seek found nothing. So the
//! forward walk is every key at or above the seek key in order, and the
//! backward walk every key below it in reverse order, whatever the bucket
//! holds; a 32-byte seek key sorts before every 36-byte key it prefixes, so
//! both are ranges of the typed outpoint keys, read a page at a time
//! (`store::walk`). Go seeks the chosen key again to return its value; the
//! port keeps each candidate's row from the walk, the same row in the same
//! transaction.

use std::collections::HashSet;
use std::ops::Bound;

use dcroxide_chainhash::Hash;
use dcroxide_txscript::stdscript::{ScriptType, determine_script_type};
use dcroxide_wire::{
    MAX_TX_IN_SEQUENCE_NUM, MsgTx, NULL_BLOCK_HEIGHT, NULL_BLOCK_INDEX, OutPoint, TX_TREE_REGULAR,
    TX_TREE_STAKE, TxIn,
};
use purse_dcrutil::Amount;
use purse_errors::{AnyError, Kind, e};
use purse_snacl::RandomSource;
use purse_store::{
    CreditKey, Credits, Outpoint, OutputTag, TableId, TxMined, TxRecKey, TxUnmined,
    UnminedCreditRow, UnminedCredits, UnminedInputs, Unpublished, Unspent, UnspentRow,
};
use purse_txauthor::InputDetail;
use purse_txrules::rules::stake_sub_script_type;
use purse_txrules::txsizes::{REDEEM_P2PK_SIG_SCRIPT_SIZE, REDEEM_P2PKH_SIG_SCRIPT_SIZE};

use super::record::{Credit, canonical, display};
use super::store::{TxRead, TxStore, store_err, walk};
use super::unmined::latest_mined_height;
use super::utxo::{Reads, output_credit, script_at};

// ------------------------------------------------------------ the stream

/// One `Read` of `buf.len()` bytes from the stream, into a zeroed buffer.
fn read(rng: &mut (impl RandomSource + ?Sized), buf: &mut [u8]) -> Result<(), AnyError> {
    buf.fill(0);
    rng.fill(buf)
        .map_err(|err| e!(Kind::Crypto, format!("random source: {err}")).into())
}

/// `crypto/rand`'s `Uint64`: eight bytes, little endian.
fn uint64(rng: &mut (impl RandomSource + ?Sized)) -> Result<u64, AnyError> {
    let mut b = [0u8; 8];
    read(rng, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

/// The double-width product `x * n` as `(hi, lo)` (`bits.Mul64`).
fn mul64(x: u64, n: u64) -> (u64, u64) {
    let wide = u128::from(x).wrapping_mul(u128::from(n));
    ((wide >> 64) as u64, wide as u64)
}

/// `crypto/rand`'s `Uint64N` on a 64-bit platform (`uniform.go:102-148`):
/// a mask for a power of two (zero included), else the high half of
/// `Uint64() * n`, drawn again while the low half is below `2⁶⁴ mod n`,
/// which is computed only when the low half is below `n`, as Go's
/// shortcut does. The shortcut only avoids the 64-bit division: `2⁶⁴ mod n`
/// is below `n`, so a low half at or above `n` would end the loop anyway,
/// and the draws made are the same either way.
fn uint64n(rng: &mut (impl RandomSource + ?Sized), n: u64) -> Result<u64, AnyError> {
    if n & n.wrapping_sub(1) == 0 {
        return Ok(uint64(rng)? & n.wrapping_sub(1));
    }
    let (mut hi, mut lo) = mul64(uint64(rng)?, n);
    if lo < n {
        // -n % n; n is not zero on this branch.
        let thresh = n.wrapping_neg().checked_rem(n).unwrap_or(0);
        while lo < thresh {
            (hi, lo) = mul64(uint64(rng)?, n);
        }
    }
    Ok(hi)
}

/// `crypto/rand`'s `IntN` for a positive `n`.
fn int_n(rng: &mut (impl RandomSource + ?Sized), n: usize) -> Result<usize, AnyError> {
    let n64 = u64::try_from(n).unwrap_or(u64::MAX);
    Ok(usize::try_from(uint64n(rng, n64)?).unwrap_or(usize::MAX))
}

/// `crypto/rand`'s `ShuffleSlice` (`Shuffle`, `uniform.go:214-230`): for
/// `i` from `len - 1` down to 1, swap `i` with `Uint64N(i + 1)`.
fn shuffle<T>(rng: &mut (impl RandomSource + ?Sized), s: &mut [T]) -> Result<(), AnyError> {
    for i in (1..s.len()).rev() {
        let bound = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
        let j = usize::try_from(uint64n(rng, bound)?).unwrap_or(0);
        s.swap(i, j);
    }
    Ok(())
}

// ------------------------------------------------------- confirmations

/// Go's `confirms` in `int32` arithmetic, which wraps.
fn confirms(tx_height: i32, cur_height: i32) -> i32 {
    if tx_height == -1 || tx_height > cur_height {
        0
    } else {
        cur_height.wrapping_sub(tx_height).wrapping_add(1)
    }
}

/// Go's `confirmed`.
fn confirmed(min_conf: i32, tx_height: i32, cur_height: i32) -> bool {
    confirms(tx_height, cur_height) >= min_conf
}

/// Go's `coinbaseMatured` and `ticketChangeMatured`, by maturity.
fn matured(maturity: i32, tx_height: i32, cur_height: i32) -> bool {
    tx_height >= 0 && cur_height.wrapping_sub(tx_height).wrapping_add(1) > maturity
}

/// A stored height as Go's `int32` conversion reads it.
fn signed(height: u32) -> i32 {
    height.cast_signed()
}

// --------------------------------------------------------- randomUTXO

/// One pass of `randomUTXO`'s walk: from the seek key `start` forward, or
/// backward from the key below it, collecting the keys of the first
/// transaction hash met that `skip` keeps (keys `skip` drops do not end
/// the walk), with their rows.
fn utxo_pass<R: TxRead + ?Sized>(
    r: &mut R,
    start: &Outpoint,
    back: bool,
    skip: &mut dyn FnMut(&mut R, &Outpoint, &UnspentRow) -> Result<bool, AnyError>,
) -> Result<Vec<(Outpoint, UnspentRow)>, AnyError> {
    let mut keys: Vec<(Outpoint, UnspentRow)> = Vec::new();
    let (lo, hi) = if back {
        (Bound::Unbounded, Bound::Excluded(*start))
    } else {
        (Bound::Included(*start), Bound::Unbounded)
    };
    walk::<Unspent, _, _>(r, lo, hi, back, |r, k, v| {
        if keys.first().is_some_and(|(f, _)| f.hash != k.hash) {
            return Ok(true);
        }
        if !skip(r, &k, &v)? {
            keys.push((k, v));
        }
        Ok(false)
    })?;
    Ok(keys)
}

/// `randomUTXO`: a random unspent output `skip` keeps, or `None`. It
/// draws 33 bytes, a seek key and a direction, walks one way from the key
/// and, finding nothing, the other, then picks one of the keys found with
/// `IntN`.
fn random_unspent<R: TxRead + ?Sized>(
    r: &mut R,
    rng: &mut (impl RandomSource + ?Sized),
    skip: &mut dyn FnMut(&mut R, &Outpoint, &UnspentRow) -> Result<bool, AnyError>,
) -> Result<Option<(Outpoint, UnspentRow)>, AnyError> {
    let mut drawn = [0u8; 33];
    read(rng, &mut drawn)?;
    let mut key = [0u8; 32];
    key.copy_from_slice(&drawn[..32]);
    let prev_first = drawn[32] & 1 == 1;
    let start = Outpoint {
        hash: purse_store::Hash32(key),
        index: 0,
    };
    let mut keys = utxo_pass(r, &start, prev_first, skip)?;
    if keys.is_empty() {
        keys = utxo_pass(r, &start, !prev_first, skip)?;
    }
    if keys.is_empty() {
        return Ok(None);
    }
    let i = int_n(rng, keys.len())?;
    Ok(Some(keys.swap_remove(i)))
}

// ------------------------------------------------------- input source

/// An output the enumeration phase found: a mined unspent output or an
/// unmined credit, with its row.
#[derive(Clone, Copy, Debug)]
enum Candidate {
    Mined(Outpoint, UnspentRow),
    Unmined(Outpoint, UnminedCreditRow),
}

/// An input source over one account's spendable outputs (Go's
/// `InputSource` made by `MakeInputSource`), holding the reader of the
/// transaction it was made in, as Go's closure holds its `dbtx`, and the
/// random stream it draws from.
///
/// [`InputSource::select_inputs`] may be called several times with
/// increasing targets; each answer holds every input selected so far.
pub struct InputSource<'a, R: TxRead + ?Sized, G: RandomSource + ?Sized> {
    store: &'a TxStore,
    r: &'a mut R,
    rng: &'a mut G,
    account: u32,
    min_conf: i32,
    sync_height: i32,
    ignore: Option<&'a dyn Fn(&OutPoint) -> bool>,
    detail: InputDetail,
    /// The keys the random phase returned (Go's `seen`).
    seen: HashSet<Outpoint>,
    /// `unspent`'s row count when the source was made (Go's `KeyN`).
    num_unspent: u64,
    rand_tries: u64,
    /// The shuffled outputs of the enumeration phase and the next to take
    /// (Go's `remainingKeys`, nil until built).
    remaining: Option<(Vec<Candidate>, usize)>,
}

impl<R: TxRead + ?Sized, G: RandomSource + ?Sized> std::fmt::Debug for InputSource<'_, R, G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSource")
            .field("account", &self.account)
            .field("min_conf", &self.min_conf)
            .field("sync_height", &self.sync_height)
            .field("detail", &self.detail)
            .field("rand_tries", &self.rand_tries)
            .finish_non_exhaustive()
    }
}

/// Go's skip test for a mined unspent output: spent by an unmined
/// transaction, not confirmed at `min_conf`, or already returned by the
/// random phase.
fn skip_unspent<R: TxRead + ?Sized>(
    r: &mut R,
    k: &Outpoint,
    u: &UnspentRow,
    min_conf: i32,
    sync_height: i32,
    seen: Option<&HashSet<Outpoint>>,
) -> Result<bool, AnyError> {
    if r.contains::<UnminedInputs>(k).map_err(store_err)? {
        return Ok(true);
    }
    if !confirmed(min_conf, signed(u.height), sync_height) {
        return Ok(true);
    }
    Ok(seen.is_some_and(|s| s.contains(k)))
}

impl<'a, R: TxRead + ?Sized, G: RandomSource + ?Sized> InputSource<'a, R, G> {
    /// `SelectInputs`: inputs whose total reaches `target`, added to those
    /// earlier calls selected (every output that qualifies when `target`
    /// is 0), in the order Go selects them.
    ///
    /// The random phase (`min_conf` not 0 and `target` not 0) tries
    /// `randomUTXO` up to half the unspent outputs' number of times over
    /// the source's life; a try that finds nothing ends the call, as Go's
    /// does (QK-0096). Then every unspent output the random phase did not
    /// return, and with `min_conf` 0 every unmined credit of a published
    /// transaction without a stake tag (QK-0105: whether or not an unmined
    /// transaction spends it), is shuffled and taken in turn. An output is
    /// selected when it belongs to the account and passes Go's tests in
    /// Go's order (mined: not spent, not zero, not a ticket output,
    /// confirmed, mature), is not ignored, and pays a P2PKH or P2PK script,
    /// stake-tagged or not; any other script is passed over, where Go logs
    /// an error.
    ///
    /// # Errors
    ///
    /// An unspent output whose credit is missing: `IO` "missing mined and
    /// unmined credit"; its record missing or the script location outside
    /// it: `IO`, Go's text; an unmined credit's record missing or too
    /// short: `IO`. A failing random source: `Crypto`. The store's.
    pub fn select_inputs(&mut self, target: Amount) -> Result<&InputDetail, AnyError> {
        let target = target.0;
        while self.detail.amount.0 < target || target == 0 {
            let mut picked: Option<Candidate> = None;
            let tries_left = self.rand_tries < self.num_unspent / 2;
            if self.min_conf != 0 && target != 0 && tries_left {
                self.rand_tries = self.rand_tries.saturating_add(1);
                let (min_conf, sync_height) = (self.min_conf, self.sync_height);
                let seen = &self.seen;
                let found = random_unspent(&mut *self.r, &mut *self.rng, &mut |r, k, u| {
                    skip_unspent(r, k, u, min_conf, sync_height, Some(seen))
                })?;
                if let Some((k, u)) = found {
                    self.seen.insert(k);
                    picked = Some(Candidate::Mined(k, u));
                }
            } else if self.remaining.is_none() {
                let mut all = self.enumerate()?;
                shuffle(&mut *self.rng, &mut all)?;
                self.remaining = Some((all, 0));
            }
            let c = match picked {
                Some(c) => c,
                None => {
                    // Go's remainingKeys is nil after a random try that
                    // found nothing, so this ends the call too.
                    let Some((all, next)) = self.remaining.as_mut() else {
                        break;
                    };
                    let Some(c) = all.get(*next).copied() else {
                        break;
                    };
                    *next = next.saturating_add(1);
                    c
                }
            };
            let Some((op, amount, script)) = self.qualify(&c)? else {
                continue;
            };
            if self.ignore.is_some_and(|ignore| ignore(&op)) {
                continue;
            }
            let (sub, _) = stake_sub_script_type(determine_script_type(0, &script));
            let size = match sub {
                ScriptType::PubKeyHashEcdsaSecp256k1 => REDEEM_P2PKH_SIG_SCRIPT_SIZE,
                ScriptType::PubKeyEcdsaSecp256k1 => REDEEM_P2PK_SIG_SCRIPT_SIZE,
                // Go logs "Unexpected script class for credit" and passes
                // the output over.
                _ => continue,
            };
            self.detail.amount = Amount(self.detail.amount.0.wrapping_add(amount));
            self.detail.inputs.push(TxIn {
                previous_out_point: op,
                sequence: MAX_TX_IN_SEQUENCE_NUM,
                value_in: amount,
                block_height: NULL_BLOCK_HEIGHT,
                block_index: NULL_BLOCK_INDEX,
                signature_script: Vec::new(),
            });
            self.detail.scripts.push(script);
            self.detail.redeem_script_sizes.push(size);
        }
        Ok(&self.detail)
    }

    /// The enumeration phase's outputs, in key order before the shuffle:
    /// every unspent output Go's skip test keeps, then, with `min_conf` 0,
    /// the unmined credits of published transactions without a stake tag.
    ///
    /// The unmined-inputs index is read once, in key order, beside the
    /// walk of `unspent` (a merge, where Go looks each output up): the
    /// same outputs, one scan of the index instead of a lookup per output.
    fn enumerate(&mut self) -> Result<Vec<Candidate>, AnyError> {
        let spent = self
            .r
            .keys::<UnminedInputs>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        let mut si = 0usize;
        let mut out = Vec::new();
        let (min_conf, sync_height) = (self.min_conf, self.sync_height);
        let seen = &self.seen;
        walk::<Unspent, _, _>(
            &mut *self.r,
            Bound::Unbounded,
            Bound::Unbounded,
            false,
            |_, k, u| {
                while spent.get(si).is_some_and(|s| *s < k) {
                    si = si.saturating_add(1);
                }
                if spent.get(si) == Some(&k)
                    || !confirmed(min_conf, signed(u.height), sync_height)
                    || seen.contains(&k)
                {
                    return Ok(false);
                }
                out.push(Candidate::Mined(k, u));
                Ok(false)
            },
        )?;
        if self.min_conf == 0 {
            walk::<UnminedCredits, _, _>(
                &mut *self.r,
                Bound::Unbounded,
                Bound::Unbounded,
                false,
                |r, k, m| {
                    if seen.contains(&k) || r.contains::<Unpublished>(&k.hash).map_err(store_err)? {
                        return Ok(false);
                    }
                    // Ticket outputs, and the stake outputs not yet mature
                    // (Go also names OP_TADD, never a stored tag).
                    if m.flags.tag != OutputTag::NonStake {
                        return Ok(false);
                    }
                    out.push(Candidate::Unmined(k, m));
                    Ok(false)
                },
            )?;
        }
        Ok(out)
    }

    /// Go's tests of one candidate before the ignore function: the
    /// outpoint (with its tree), amount and script when the output can be
    /// selected for the account.
    ///
    /// For a mined output the credit and the record are read before the
    /// account is compared, as Go reads them: a missing record fails the
    /// selection whichever account the output belongs to. Comparing the
    /// account first (the unspent row holds it) would save a record read
    /// per output of another account; the order is kept for parity.
    fn qualify(&mut self, c: &Candidate) -> Result<Option<(OutPoint, i64, Vec<u8>)>, AnyError> {
        match c {
            Candidate::Mined(k, u) => {
                let ck = CreditKey {
                    hash: k.hash,
                    height: u.height,
                    index: k.index,
                };
                let Some(credit) = self.r.get::<Credits>(&ck).map_err(store_err)? else {
                    return Err(e!(Kind::Io, "missing mined and unmined credit").into());
                };
                let rec = self
                    .r
                    .get::<TxMined>(&TxRecKey {
                        hash: k.hash,
                        height: u.height,
                    })
                    .map_err(store_err)?;
                let script = script_at(
                    rec.as_ref().map(|t| t.tx.as_slice()),
                    &credit.script,
                    &k.hash,
                    k.index,
                )?;
                if credit.account != self.account
                    || credit.spender.is_some()
                    || credit.amount == 0
                    || credit.flags.tag == OutputTag::Ticket
                {
                    return Ok(None);
                }
                let height = signed(u.height);
                let sync = self.sync_height;
                if !confirmed(self.min_conf, height, sync) {
                    return Ok(None);
                }
                let coinbase_maturity = self.store.coinbase_maturity;
                let mature = match credit.flags.tag {
                    OutputTag::NonStake if credit.flags.coinbase => {
                        matured(coinbase_maturity, height, sync)
                    }
                    OutputTag::Vote | OutputTag::Revocation | OutputTag::TreasuryGen => {
                        matured(coinbase_maturity, height, sync)
                    }
                    OutputTag::TicketChange => {
                        matured(self.store.sstx_change_maturity, height, sync)
                    }
                    _ => true,
                };
                if !mature {
                    return Ok(None);
                }
                let tree = if credit.flags.tag == OutputTag::NonStake {
                    TX_TREE_REGULAR
                } else {
                    TX_TREE_STAKE
                };
                Ok(Some((wire_op(k, tree), credit.amount, script)))
            }
            Candidate::Unmined(k, m) => {
                let script = unmined_script(&mut *self.r, k, m)?;
                if m.account != self.account {
                    return Ok(None);
                }
                let tree = if m.flags.tag == OutputTag::NonStake {
                    TX_TREE_REGULAR
                } else {
                    TX_TREE_STAKE
                };
                Ok(Some((wire_op(k, tree), m.amount, script)))
            }
        }
    }
}

impl<R: TxRead + ?Sized, G: RandomSource + ?Sized> purse_txauthor::InputSource
    for InputSource<'_, R, G>
{
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError> {
        self.select_inputs(target).cloned()
    }
}

fn wire_op(k: &Outpoint, tree: i8) -> OutPoint {
    OutPoint {
        hash: Hash(k.hash.0),
        index: k.index,
        tree,
    }
}

/// An unmined credit's script, cut from its unmined record at the location
/// the credit records (Go decodes the record and takes the output's
/// script: the same bytes).
fn unmined_script(
    r: &mut (impl TxRead + ?Sized),
    k: &Outpoint,
    m: &UnminedCreditRow,
) -> Result<Vec<u8>, AnyError> {
    let Some(row) = r.get::<TxUnmined>(&k.hash).map_err(store_err)? else {
        return Err(e!(Kind::Io, "unmined len 0").into());
    };
    let start = usize::try_from(m.script.offset).unwrap_or(usize::MAX);
    row.tx
        .get(start..start.saturating_add(usize::from(m.script.len)))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| {
            e!(
                Kind::Io,
                format!("no output {} for tx {}", k.index, display(&k.hash))
            )
            .into()
        })
}

impl TxStore {
    /// `RandomUTXO`: a random mined unspent output confirmed at `min_conf`
    /// at `sync_height` that no unmined transaction spends, as a [`Credit`]
    /// (as [`TxStore::unspent_outputs`] gives it), or `None`; drawn from
    /// `rng` as Go draws from its generator.
    ///
    /// # Errors
    ///
    /// `min_conf` 0: `Invalid`, Go's text, before any draw. As
    /// [`TxStore::unspent_outputs`] for the output picked. A failing random
    /// source: `Crypto`. The store's.
    pub fn random_utxo(
        &self,
        r: &mut (impl TxRead + ?Sized),
        rng: &mut (impl RandomSource + ?Sized),
        min_conf: i32,
        sync_height: i32,
    ) -> Result<Option<Credit>, AnyError> {
        if min_conf == 0 {
            return Err(e!(
                Kind::Invalid,
                "optimized random utxo selection not possible with minConf=0"
            )
            .into());
        }
        let found = random_unspent(r, rng, &mut |r, k, u| {
            skip_unspent(r, k, u, min_conf, sync_height, None)
        })?;
        let Some((op, u)) = found else {
            return Ok(None);
        };
        output_credit(r, &op, Some(&u), &mut Reads::default()).map(Some)
    }

    /// `MakeInputSource`: an input source over `account`'s outputs
    /// confirmed at `min_conf` at `sync_height`, reading through `r` and
    /// drawing from `rng`; `ignore` excludes outputs (by outpoint and
    /// tree, as Go's function sees them).
    ///
    /// # Errors
    ///
    /// The store's (counting `unspent`, Go's `KeyN`, which has none).
    pub fn make_input_source<'a, R: TxRead + ?Sized, G: RandomSource + ?Sized>(
        &'a self,
        r: &'a mut R,
        rng: &'a mut G,
        account: u32,
        min_conf: i32,
        sync_height: i32,
        ignore: Option<&'a dyn Fn(&OutPoint) -> bool>,
    ) -> Result<InputSource<'a, R, G>, AnyError> {
        let num_unspent = r.table_len(TableId::Unspent).map_err(store_err)?;
        Ok(InputSource {
            store: self,
            r,
            rng,
            account,
            min_conf,
            sync_height,
            ignore,
            detail: InputDetail::default(),
            seen: HashSet::new(),
            num_unspent,
            rand_tries: 0,
            remaining: None,
        })
    }

    /// `Spender`: the transaction spending the credit `out` and the input
    /// that spends it.
    ///
    /// The credit is looked up under the latest mined record of `out`'s
    /// transaction; when it is spent by a mined transaction that
    /// transaction's record gives the answer; otherwise, or when the
    /// transaction is not mined, the unmined-inputs index names an unmined
    /// spender, whose inputs are searched for `out` (hash and index, the
    /// tree not compared).
    ///
    /// # Errors
    ///
    /// Go's, with its texts: `out` is not a credit (no credit under the
    /// latest mined record, or no unmined credit): `Invalid`; it is
    /// unspent, or the spender named is not recorded or does not spend it:
    /// `NotExist`; a mined spender's record missing or not decoding: `IO`;
    /// an unmined spender not decoding: `Bug`. The store's.
    pub fn spender(
        &self,
        r: &mut (impl TxRead + ?Sized),
        out: &OutPoint,
    ) -> Result<(MsgTx, u32), AnyError> {
        let k = canonical(out);
        let mut spender_hash = None;
        if let Some(height) = latest_mined_height(r, &k.hash)? {
            let ck = CreditKey {
                hash: k.hash,
                height,
                index: k.index,
            };
            let Some(credit) = r.get::<Credits>(&ck).map_err(store_err)? else {
                return Err(not_a_credit());
            };
            if let Some(sp) = credit.spender {
                let row = r
                    .get::<TxMined>(&TxRecKey {
                        hash: sp.hash,
                        height: sp.height,
                    })
                    .map_err(store_err)?;
                let Some(row) = row else {
                    return Err(e!(Kind::Io, "tx record len 0").into());
                };
                let (tx, _) = MsgTx::from_bytes(&row.tx)
                    .map_err(|err| AnyError::from(e!(Kind::Io, AnyError::foreign(err))))?;
                return Ok((tx, sp.input));
            }
            let Some(h) = r.get::<UnminedInputs>(&k).map_err(store_err)? else {
                return Err(unspent());
            };
            spender_hash = Some(h);
        }
        // Go tests the hash for zero: a mined credit's unmined spender is
        // known here; otherwise it is the unmined credit's.
        let spender_hash = match spender_hash {
            Some(h) if h != purse_store::Hash32::default() => h,
            _ => {
                if !r.contains::<UnminedCredits>(&k).map_err(store_err)? {
                    return Err(not_a_credit());
                }
                let Some(h) = r.get::<UnminedInputs>(&k).map_err(store_err)? else {
                    return Err(unspent());
                };
                h
            }
        };
        let Some(row) = r.get::<TxUnmined>(&spender_hash).map_err(store_err)? else {
            return Err(e!(Kind::NotExist, "missing unmined spending tx").into());
        };
        let (tx, _) = MsgTx::from_bytes(&row.tx)
            .map_err(|err| AnyError::from(e!(Kind::Bug, AnyError::foreign(err))))?;
        let found = tx.tx_in.iter().position(|input| {
            input.previous_out_point.hash == out.hash && input.previous_out_point.index == out.index
        });
        match found {
            Some(i) => Ok((tx, u32::try_from(i).unwrap_or(u32::MAX))),
            None => Err(e!(Kind::NotExist, "recorded spending tx does not spend credit").into()),
        }
    }
}

fn not_a_credit() -> AnyError {
    e!(Kind::Invalid, "output is not a credit").into()
}

fn unspent() -> AnyError {
    e!(Kind::NotExist, "credit is unspent").into()
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    reason = "test code: reference arithmetic on bounded values"
)]
mod tests {
    use super::*;
    use purse_snacl::RandomSourceError;

    /// A source over a fixed byte string, recording how much was drawn.
    struct Bytes<'b> {
        bytes: &'b [u8],
        at: usize,
    }

    impl RandomSource for Bytes<'_> {
        fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
            let end = self.at + buf.len();
            let src = self
                .bytes
                .get(self.at..end)
                .ok_or_else(|| RandomSourceError::new("exhausted"))?;
            for (b, s) in buf.iter_mut().zip(src) {
                *b ^= s;
            }
            self.at = end;
            Ok(())
        }
    }

    fn le(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn uint64n_masks_a_power_of_two_and_rejects_the_short_class() {
        // A power of two (and zero) is one masked draw.
        let b = le(&[0xffff_ffff_ffff_fff5]);
        let mut s = Bytes { bytes: &b, at: 0 };
        assert_eq!(uint64n(&mut s, 8).unwrap(), 5);
        assert_eq!(s.at, 8);
        let mut s = Bytes { bytes: &b, at: 0 };
        assert_eq!(uint64n(&mut s, 0).unwrap(), 0xffff_ffff_ffff_fff5);
        // n = 3: 2^64 mod 3 = 1. x = 0 gives lo 0 < 1, rejected; x = 1
        // gives hi 0, lo 3, kept.
        let b = le(&[0, 1]);
        let mut s = Bytes { bytes: &b, at: 0 };
        assert_eq!(uint64n(&mut s, 3).unwrap(), 0);
        assert_eq!(s.at, 16, "the rejected draw is redrawn");
        // x = u64::MAX: hi 2, lo 2^64 - 3, above the threshold.
        let b = le(&[u64::MAX]);
        let mut s = Bytes { bytes: &b, at: 0 };
        assert_eq!(uint64n(&mut s, 3).unwrap(), 2);
    }

    #[test]
    fn shuffle_walks_down_from_the_top() {
        // Three elements: j for i = 2 from Uint64N(3), then for i = 1 from
        // Uint64N(2) (a mask). Draws: u64::MAX -> 2 (no swap), then 0 -> 0
        // (swap 1 and 0).
        let b = le(&[u64::MAX, 0]);
        let mut s = Bytes { bytes: &b, at: 0 };
        let mut v = [10, 20, 30];
        shuffle(&mut s, &mut v).unwrap();
        assert_eq!(v, [20, 10, 30]);
        assert_eq!(s.at, 16);
        // Zero and one element draw nothing.
        let mut s = Bytes { bytes: &[], at: 0 };
        shuffle::<u8>(&mut s, &mut []).unwrap();
        shuffle(&mut s, &mut [1]).unwrap();
    }

    #[test]
    fn confirmations_wrap_in_int32() {
        assert!(confirmed(1, 5, 5));
        assert!(!confirmed(2, 5, 5));
        assert!(!confirmed(1, -1, 5));
        assert!(!confirmed(1, 6, 5));
        // i32::MAX - 0 + 1 wraps to i32::MIN: no confirmation.
        assert!(!confirmed(1, 0, i32::MAX));
        assert!(confirmed(i32::MIN, 0, i32::MAX));
        assert!(matured(16, 0, 16));
        assert!(!matured(16, 0, 15));
        assert!(!matured(16, -1, 100));
    }

    /// dcroxide's port of dcrd's generator as a source.
    struct Chacha(dcroxide_crypto::rand::Prng);

    impl RandomSource for Chacha {
        fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
            self.0.read(buf);
            Ok(())
        }
    }

    /// `IntN` and `ShuffleSlice` draw what dcroxide's port of
    /// `crypto/rand` draws from the same keyed stream, over bounds that
    /// take the mask, the plain multiply and the rejection branches.
    #[test]
    fn draws_equal_dcroxides_port_of_crypto_rand() {
        let key = [0x5a; 32];
        let mut ours = Chacha(dcroxide_crypto::rand::Prng::from_seed(key));
        let mut theirs = dcroxide_crypto::rand::Prng::from_seed(key);
        let bounds = [1usize, 2, 3, 5, 7, 8, 100, 1 << 20, (1 << 33) + 1];
        for round in 0..2000 {
            let n = bounds[round % bounds.len()];
            assert_eq!(
                int_n(&mut ours, n).unwrap(),
                theirs.int_n(n),
                "round {round}"
            );
        }
        for len in 0..40 {
            let mut a: Vec<u32> = (0..len).collect();
            let mut b = a.clone();
            shuffle(&mut ours, &mut a).unwrap();
            theirs.shuffle(&mut b);
            assert_eq!(a, b, "len {len}");
        }
        // The two streams are at the same place.
        assert_eq!(uint64(&mut ours).unwrap(), theirs.uint64());
    }

    #[test]
    fn a_failing_source_is_a_crypto_error() {
        let mut s = Bytes { bytes: &[], at: 0 };
        let err = uint64(&mut s).unwrap_err();
        assert!(err.is_kind(Kind::Crypto), "{err}");
    }
}
