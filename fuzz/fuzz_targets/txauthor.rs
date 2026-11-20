// SPDX-License-Identifier: ISC
//! purse-txauthor fuzz target: the authoring loop
//! (`new_unsigned_transaction`) on arbitrary outputs, fee rates, size
//! limits, input sources and change sources, and the signing loop
//! (`add_all_input_scripts`) on arbitrary inputs, previous scripts,
//! existing signature scripts, keys and redeem scripts.
//!
//! Amounts, rates and sizes range over all of Go's `int64`, where the
//! loop's sums wrap, and nothing may panic or fail to end: where a retry
//! of dcrwallet's loop would ask for the target it just asked for, which
//! it does forever while the source answers alike (QK-0744, QK-0753),
//! where dcrd's signature-script merge recurses until the stack overflows
//! (QK-0746) and where its signer indexes an empty address list (QK-0745),
//! the crate returns an error. The crate keeps Go's endless loop for a
//! source whose answers make the targets cycle with a longer period; mode
//! 1's source can do that, and the loop then ends only because that
//! source's input runs out. Input past its end reads as zeros, and no rare
//! branch is taken on a zero, so that short inputs reach the common
//! paths. The first byte picks the mode:
//!
//! - 0: authoring from an input source of the oracle's cumulative shape
//!   (UTXOs added until the target is met, or all at once, with a
//!   constant added to the reported amount; rarely with an output total
//!   just below `i64::MAX` and a first UTXO just above it with a large
//!   signature script, the shape whose input fee wraps the retry). Besides
//!   the checks of mode 1, the source is asked at most two times more than
//!   it has UTXOs.
//! - 1: authoring from an adversarial source that answers each call
//!   afresh from the input: any amount, any inputs and signature-script
//!   sizes, or an error, until the input runs out. Every call and the
//!   result are checked against the loop's definition (spec 10 §4.5,
//!   with the endless-loop divergence): the first target is the output
//!   total plus the fee for one P2PKH input; each retry follows an answer
//!   that met its target but not its own fee, asks for the output total
//!   plus that fee, and never repeats the target before it; the last
//!   answer decides the outcome (the source's error wrapped, insufficient
//!   balance, the size limit, the change source's error, the change
//!   script limit, or a transaction). A transaction carries the last
//!   answer's inputs and scripts, pays the required fee exactly when it
//!   has change (positive and not dust) and otherwise keeps a remainder
//!   that is zero or dust, and its signed-size estimate exceeds the
//!   unsigned serialization by exactly the declared signature scripts
//!   when the sizes are honest; it is then within the size limit unless
//!   the change output's estimated size wrapped, which Go's check passes
//!   too.
//! - 2: authoring with sane values from P2PKH UTXOs of keys made from
//!   the input, then signing with those keys: the signing must succeed,
//!   every input must verify in dcroxide's script engine, the signed size
//!   must not exceed the estimate (by exactly the bytes the signatures
//!   came under their worst case), and the fee paid must cover the fee
//!   for the signed size.
//! - 3: signing an arbitrary transaction: previous scripts of these
//!   shapes: P2PKH, P2PK, bare multisig, P2SH of any of them, P2SH of
//!   P2SH, stake-tagged P2PKH and P2SH (OP_SSTX, OP_SSGEN, OP_SSRTX,
//!   OP_SSTXCHANGE, OP_TGEN), treasury add, null data, and arbitrary bytes;
//!   keys of any length and signature type, redeem scripts that may not
//!   parse, existing signature scripts, lookup errors, and a script count
//!   that may not match the inputs. Nothing but the signature scripts may
//!   change, the length mismatch must leave the transaction untouched,
//!   and signing is deterministic. With the input's honest flag set, all
//!   scripts are standard over known keys with empty signature scripts,
//!   and every input must then sign and verify. Random scripts reach the
//!   shapes of QK-0746 and QK-0745 too seldom for a short run (a crate
//!   without the QK-0746 guard survived 60-second runs), so mode 3 also
//!   builds each directly, each from one mode-3 input in 16, after up to
//!   two honest P2PKH inputs: a pay-to-script-hash previous script whose
//!   redeem script pays to the hash of a standard script, both known, with
//!   a nonempty existing signature script that parses, which must fail
//!   with `NESTED_SCRIPT_HASH_MERGE`; and a pay-to-pubkey script whose
//!   compressed key is not on the curve, as the previous script or as its
//!   redeem script, which must fail with `INVALID_PUBKEY_SCRIPT`. Either
//!   way the honest inputs before it are signed and nothing else changes.

#![no_main]

use dcroxide_chaincfg::Params;
use dcroxide_chainhash::Hash;
use dcroxide_dcrec::{edwards, secp256k1};
use dcroxide_txscript::sign::SignatureType;
use dcroxide_txscript::stdaddr::{self, Address};
use dcroxide_txscript::{Engine, ScriptFlags};
use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut, TxSerializeType};
use libfuzzer_sys::fuzz_target;
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_errors::{AnyError, Kind};
use purse_txauthor::{
    ChangeSource, GENERATED_TX_VERSION, INVALID_PUBKEY_SCRIPT, InputDetail, InputSource,
    NESTED_SCRIPT_HASH_MERGE, SecretsSource, add_all_input_scripts, new_unsigned_transaction,
};
use purse_txrules::txsizes::REDEEM_P2PKH_SIG_SCRIPT_SIZE;
use purse_txrules::{estimate_serialize_size, fee_for_serialize_size, is_dust_amount};

const SEP: &str = ":: ";
const OP: &str = "txauthor.NewUnsignedTransaction";

/// The input as a stream that reads zeros once it runs out.
struct Bytes<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bytes { data, pos: 0 }
    }

    fn empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn u8(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        b
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.u8()).collect()
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()])
    }

    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.take(8).try_into().expect("8 bytes"))
    }

    /// 32 bytes expanded from two input bytes by SplitMix64: key seeds,
    /// which need not cost the input more.
    fn seed(&mut self) -> [u8; 32] {
        let mut state = u64::from(self.u16());
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            chunk.copy_from_slice(&(z ^ (z >> 31)).to_le_bytes());
        }
        out
    }

    fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }

    /// An amount, biased towards the values where Go's sums wrap and the
    /// relay rules' limits lie.
    fn amount(&mut self) -> i64 {
        match self.u8() % 10 {
            0 => i64::from(self.u16()),
            1 => i64::from(self.u32()),
            2 => self.i64(),
            3 => i64::MAX - i64::from(self.u16()),
            4 => i64::MIN + i64::from(self.u16()),
            5 => MAX_AMOUNT - i64::from(self.u16()),
            6 => -i64::from(self.u16()),
            7 => i64::from(self.u32()) * 10_000,
            8 => (i64::MAX >> 1) + i64::from(self.u16()),
            _ => 100_000_000 * i64::from(self.u8()),
        }
    }

    /// A signature-script or change-script size.
    fn size(&mut self) -> i64 {
        match self.u8() % 8 {
            0 | 1 => REDEEM_P2PKH_SIG_SCRIPT_SIZE,
            2 => i64::from(self.u8()),
            3 => i64::from(self.u16()),
            4 => self.i64(),
            // The one size whose output cost wraps to 0 (QK-0741).
            5 => -184,
            6 => -i64::from(self.u8()),
            _ => 25,
        }
    }

    fn rate(&mut self) -> i64 {
        match self.u8() % 8 {
            0 => 0,
            1 => 1,
            2 => 1_000,
            3 => 10_000,
            4 => 100_000,
            5 => i64::from(self.u32()),
            6 => self.i64(),
            _ => -i64::from(self.u16()),
        }
    }

    fn max_size(&mut self) -> i64 {
        match self.u8() % 6 {
            0 | 1 => 100_000,
            2 => 1_000_000,
            3 => i64::from(self.u16()),
            4 => self.i64(),
            _ => i64::MAX,
        }
    }

    fn kind(&mut self) -> Kind {
        [
            Kind::Locked,
            Kind::NotExist,
            Kind::Invalid,
            Kind::InsufficientBalance,
            Kind::Io,
        ][usize::from(self.u8() % 5)]
    }
}

fn source_error(text: &str, kind: Kind) -> AnyError {
    purse_errors::e!(kind, text.to_owned()).into()
}

fn tx_in(i: usize, value: i64, sig: Vec<u8>) -> TxIn {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&(i as u64).to_le_bytes());
    TxIn {
        previous_out_point: OutPoint {
            hash: Hash(h),
            index: i as u32,
            tree: (i % 2) as i8,
        },
        sequence: 0xffff_ffff,
        value_in: value,
        block_height: 0,
        block_index: 0xffff_ffff,
        signature_script: sig,
    }
}

fn outputs(b: &mut Bytes<'_>) -> Vec<TxOut> {
    let n = usize::from(b.u8() % 9);
    (0..n)
        .map(|i| TxOut {
            value: b.amount(),
            version: if b.u8() % 8 == 0 { b.u16() } else { 0 },
            pk_script: vec![i as u8; usize::from(b.u8() % 48)],
        })
        .collect()
}

/// The change source: a declared size, a script whose length may differ
/// from it and may pass the 2048-byte limit, or an error.
struct Change {
    size: i64,
    script: Vec<u8>,
    version: u16,
    error: Option<Kind>,
    script_calls: usize,
}

impl Change {
    fn from(b: &mut Bytes<'_>) -> Change {
        let size = b.size();
        let len = match b.u8() % 4 {
            0 => usize::try_from(size).unwrap_or(0).min(4096),
            1 => 2048 + usize::from(b.u8() % 3),
            _ => usize::from(b.u8() % 40),
        };
        Change {
            size,
            script: vec![0x76; len],
            version: if b.u8() % 8 == 0 { b.u16() } else { 0 },
            error: if b.u8() % 16 == 1 {
                Some(b.kind())
            } else {
                None
            },
            script_calls: 0,
        }
    }
}

impl ChangeSource for Change {
    fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError> {
        self.script_calls += 1;
        match self.error {
            Some(kind) => Err(source_error("change source failed", kind)),
            None => Ok((self.script.clone(), self.version)),
        }
    }
    fn script_size(&self) -> i64 {
        self.size
    }
}

/// Records every target asked for and every answer given.
struct Recorded<S> {
    inner: S,
    calls: Vec<(i64, Result<InputDetail, String>)>,
    /// The most calls the crate may make on this source; one more is a
    /// loop that does not end.
    cap: usize,
}

/// Far beyond any call count the crate can reach on the cumulative
/// sources of modes 0 and 2, which are asked at most two times more than
/// they have UTXOs; a cut here is a hang.
const MAX_CALLS: usize = 10_000;

impl<S> Recorded<S> {
    fn new(inner: S, cap: usize) -> Self {
        Recorded {
            inner,
            calls: Vec::new(),
            cap,
        }
    }
}

impl<S: FnMut(i64) -> Result<InputDetail, AnyError>> InputSource for Recorded<S> {
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError> {
        assert!(self.calls.len() < self.cap, "the loop does not end");
        let r = (self.inner)(target.0);
        self.calls.push((
            target.0,
            r.as_ref().map(Clone::clone).map_err(|e| e.render(SEP)),
        ));
        r
    }
}

fn sum(outputs: &[TxOut]) -> i64 {
    outputs.iter().fold(0i64, |s, o| s.wrapping_add(o.value))
}

/// The loop's definition (spec 10 §4.5, Go `author.go:90-166`, with the
/// endless-loop divergence), checked against the calls the crate made and
/// the result it returned.
fn check_authoring(
    outs: &[TxOut],
    rate: i64,
    max: i64,
    change: &Change,
    calls: &[(i64, Result<InputDetail, String>)],
    result: &Result<purse_txauthor::AuthoredTx, purse_errors::Error>,
) {
    let rate_a = Amount(rate);
    let total = sum(outs);
    let cs = change.size;
    let fee_of = |sizes: &[i64], cs: i64| {
        fee_for_serialize_size(rate_a, estimate_serialize_size(sizes, outs, cs)).0
    };
    let (first, _) = calls.first().expect("the source is always asked");
    assert_eq!(
        *first,
        total.wrapping_add(fee_of(&[REDEEM_P2PKH_SIG_SCRIPT_SIZE], cs))
    );
    let last = calls.len() - 1;
    // Every call but the last was a retry.
    for (k, (target, answer)) in calls[..last].iter().enumerate() {
        let d = answer.as_ref().expect("an error ends the loop");
        assert!(d.amount.0 >= *target, "a short answer ends the loop");
        let required = fee_of(&d.redeem_script_sizes, cs);
        assert!((0..=MAX_AMOUNT).contains(&required));
        assert!(
            d.amount.0.wrapping_sub(total) < required,
            "a retry without cause"
        );
        let next = calls[k + 1].0;
        assert_eq!(next, total.wrapping_add(required));
        assert_ne!(
            next, *target,
            "a target asked twice in a row (QK-0744, QK-0753)"
        );
    }
    let (target, answer) = &calls[last];
    let render = |r: &Result<_, purse_errors::Error>| match r {
        Ok(_) => "Ok".to_owned(),
        Err(e) => e.render(SEP),
    };
    let d = match answer {
        Err(text) => {
            assert_eq!(render(result), format!("{OP}: {text}"));
            assert_eq!(change.script_calls, 0);
            return;
        }
        Ok(d) => d,
    };
    let insufficient = format!("{OP}: insufficient balance");
    if d.amount.0 < *target {
        assert_eq!(render(result), insufficient);
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.is_kind(Kind::InsufficientBalance))
        );
        return;
    }
    let required = fee_of(&d.redeem_script_sizes, cs);
    let remaining = d.amount.0.wrapping_sub(total);
    if remaining < required {
        // The retry would repeat the target: Go's endless loop, refused.
        assert_eq!(total.wrapping_add(required), *target);
        assert_eq!(render(result), insufficient);
        assert_eq!(change.script_calls, 0);
        return;
    }
    let est = estimate_serialize_size(&d.redeem_script_sizes, outs, cs);
    if est > max {
        assert_eq!(
            render(result),
            "invalid operation: signed tx size exceeds allowed maximum"
        );
        assert_eq!(change.script_calls, 0);
        return;
    }
    // remaining >= required >= 0: no wrap.
    let change_amount = remaining - required;
    let wants_change = change_amount != 0 && !is_dust_amount(Amount(change_amount), cs, rate_a);
    if wants_change {
        assert_eq!(change.script_calls, 1);
        if let Some(kind) = change.error {
            let want = source_error("change source failed", kind).render(SEP);
            assert_eq!(render(result), format!("{OP}: {want}"));
            assert!(result.as_ref().is_err_and(|e| e.is_kind(kind)));
            return;
        }
        if change.script.len() > 2048 {
            assert_eq!(
                render(result),
                "invalid operation: script size exceed maximum bytes pushable to the stack"
            );
            return;
        }
    } else {
        assert_eq!(change.script_calls, 0);
    }
    let atx = result.as_ref().expect("a transaction");
    assert_eq!(atx.tx.ser_type, TxSerializeType::Full);
    assert_eq!(atx.tx.version, GENERATED_TX_VERSION);
    assert_eq!((atx.tx.lock_time, atx.tx.expiry), (0, 0));
    assert_eq!(atx.tx.tx_in, d.inputs);
    assert_eq!(atx.prev_scripts, d.scripts);
    assert_eq!(atx.total_input, d.amount);
    assert_eq!(atx.tx.tx_out[..outs.len()], outs[..]);
    // The fee paid: the input total less every output, change included.
    let paid = atx.total_input.0.wrapping_sub(sum(&atx.tx.tx_out));
    if wants_change {
        assert_eq!(atx.change_index, outs.len() as i64);
        assert_eq!(atx.tx.tx_out.len(), outs.len() + 1);
        let c = &atx.tx.tx_out[outs.len()];
        assert!(c.value > 0 && c.value == change_amount);
        assert!(!is_dust_amount(Amount(c.value), cs, rate_a));
        assert_eq!((&c.pk_script, c.version), (&change.script, change.version));
        assert_eq!(paid, required, "change leaves exactly the fee");
        assert_eq!(atx.estimated_signed_serialize_size, est);
    } else {
        assert_eq!(atx.change_index, -1);
        assert_eq!(atx.tx.tx_out, outs);
        assert_eq!(paid, remaining, "without change the remainder is fee");
        assert!(change_amount == 0 || is_dust_amount(Amount(change_amount), cs, rate_a));
        assert_eq!(
            atx.estimated_signed_serialize_size,
            estimate_serialize_size(&d.redeem_script_sizes, outs, 0)
        );
    }
    // The estimate against the real serialization: with honest sizes it
    // exceeds the unsigned transaction by exactly the signature scripts
    // still to come (each input's empty script costs one varint byte).
    let honest_sizes = d
        .redeem_script_sizes
        .iter()
        .all(|&s| (0..1 << 20).contains(&s));
    let honest_change = !wants_change || (cs > 0 && change.script.len() as i64 == cs);
    let sane_outputs = outs.len() < 0xfd && d.inputs.len() < 0xfd;
    if honest_sizes && honest_change && sane_outputs {
        let to_come: i64 = d
            .redeem_script_sizes
            .iter()
            .map(|&s| varint_size(s as u64) + s - 1)
            .sum();
        assert_eq!(
            atx.estimated_signed_serialize_size,
            atx.tx.serialize_size() as i64 + to_come,
            "estimate and serialization disagree"
        );
        // The limit was checked against the estimate with the change
        // output (`est`), whether or not one was added. Here every term but
        // the change output's is small, so in exact arithmetic that
        // estimate is the reported one plus the change output's cost; it
        // is the reported estimate's upper bound, and the limit holds for
        // the reported estimate wherever `est` did not wrap. A change size
        // within about 19 of i64::MAX wraps the change output's cost, and
        // so `est`, negative: Go's check (author.go, the loop's
        // `maxSignedSize > maxTxSize`) passes then, and without change Go
        // reports the estimate without it, which may exceed the limit.
        let reported = i128::from(atx.estimated_signed_serialize_size);
        let exact = if wants_change {
            reported
        } else {
            reported + change_cost(cs, outs.len())
        };
        assert!(reported <= exact);
        if exact == i128::from(est) {
            assert!(atx.estimated_signed_serialize_size <= max);
        } else {
            assert!(!wants_change && est < 0, "only the change cost wraps");
        }
    }
}

/// The exact cost of a change output of declared size `cs` in the
/// estimate over `outputs` other outputs (fewer than 0xfd): nothing for a
/// size that is not positive, else the output (value, version, length
/// varint, script) and the growth of the output count's varint.
fn change_cost(cs: i64, outputs: usize) -> i128 {
    if cs <= 0 {
        return 0;
    }
    let count = i128::from(varint_size(outputs as u64 + 1) - varint_size(outputs as u64));
    8 + 2 + i128::from(varint_size(cs as u64)) + i128::from(cs) + count
}

fn varint_size(n: u64) -> i64 {
    match n {
        0..0xfd => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Mode 0: the oracle's cumulative source shape.
fn author_cumulative(b: &mut Bytes<'_>) {
    let mut outs = outputs(b);
    // Rarely, the shape of QK-0744 with a source that answers: one more
    // output brings the total within 65,535 atoms of i64::MAX, and a first
    // UTXO (below) covers it with a signature script of up to 4,203
    // bytes, whose fee may wrap the retry although the remaining amount is
    // not negative. Random draws seldom land there.
    let near_wrap = b.u8() % 16 == 1;
    if near_wrap {
        let gap = i64::from(b.u16());
        outs.push(TxOut {
            value: (i64::MAX - gap).wrapping_sub(sum(&outs)),
            version: 0,
            pk_script: vec![0; 25],
        });
    }
    let total = sum(&outs);
    let rate = b.rate();
    let max = b.max_size();
    let mut change = Change::from(b);
    let mut utxos: Vec<(i64, i64, usize)> = Vec::new();
    if near_wrap {
        utxos.push((
            total.wrapping_add(i64::from(b.u16())),
            REDEEM_P2PKH_SIG_SCRIPT_SIZE + i64::from(b.u16() % 4096),
            25,
        ));
    }
    for _ in 0..b.u8() % 13 {
        utxos.push((b.amount(), b.size(), usize::from(b.u8() % 40)));
    }
    let n = utxos.len();
    let all = b.u8() % 4 == 0;
    let delta = if b.u8() % 4 == 0 { b.amount() } else { 0 };
    let mut detail = InputDetail::default();
    let mut have = 0i64;
    let mut next = 0usize;
    let source = |target: i64| -> Result<InputDetail, AnyError> {
        while next < utxos.len() && (all || have < target) {
            let (value, size, pk_len) = utxos[next];
            detail.inputs.push(tx_in(next, value, Vec::new()));
            detail.scripts.push(vec![next as u8; pk_len]);
            detail.redeem_script_sizes.push(size);
            have = have.wrapping_add(value);
            next += 1;
        }
        let mut d = detail.clone();
        d.amount = Amount(have.wrapping_add(delta));
        Ok(d)
    };
    let mut src = Recorded::new(source, MAX_CALLS);
    let result = new_unsigned_transaction(outs.clone(), Amount(rate), &mut src, &mut change, max);
    assert!(
        src.calls.len() <= n + 2,
        "{} calls for {n} UTXOs",
        src.calls.len()
    );
    check_authoring(&outs, rate, max, &change, &src.calls, &result);
}

/// Mode 1: a source that answers each call afresh from the input.
fn author_adversarial(b: &mut Bytes<'_>) {
    let outs = outputs(b);
    let rate = b.rate();
    let max = b.max_size();
    let mut change = Change::from(b);
    let rest = &b.data[b.pos.min(b.data.len())..];
    // Every answer reads at least one byte, and the call after the last
    // byte fails: a source whose answers cycle the targets, which the
    // crate keeps asking as Go does, still ends.
    let cap = rest.len() + 1;
    let mut s = Bytes::new(rest);
    let source = |_target: i64| -> Result<InputDetail, AnyError> {
        if s.empty() {
            return Err(purse_errors::new("input source: exhausted"));
        }
        let tag = s.u8();
        if tag % 8 == 0 {
            return Err(source_error("input source failed", s.kind()));
        }
        let n = usize::from(s.u8() % 7);
        let mut d = InputDetail {
            amount: Amount(s.amount()),
            ..InputDetail::default()
        };
        for i in 0..n {
            d.inputs.push(tx_in(i, s.amount(), Vec::new()));
            d.scripts.push(vec![i as u8; usize::from(s.u8() % 30)]);
            d.redeem_script_sizes.push(s.size());
        }
        Ok(d)
    };
    let mut src = Recorded::new(source, cap);
    let result = new_unsigned_transaction(outs.clone(), Amount(rate), &mut src, &mut change, max);
    check_authoring(&outs, rate, max, &change, &src.calls, &result);
}

// ---------------------------------------------------------------------
// Keys and scripts.
// ---------------------------------------------------------------------

#[derive(Clone)]
struct Key {
    priv_bytes: Vec<u8>,
    sig_type: SignatureType,
    pub_key: Vec<u8>,
    compressed: bool,
}

/// A key of `kind` (0 ECDSA compressed, 1 ECDSA uncompressed, 2 Ed25519,
/// 3 Schnorr) from 32 input bytes.
fn key(kind: u8, seed: [u8; 32]) -> Key {
    if kind % 4 == 2 {
        let pub_key = edwards::SecretKey::from_seed(seed).public_key().serialize();
        let mut priv_bytes = seed.to_vec();
        priv_bytes.extend_from_slice(&pub_key);
        return Key {
            priv_bytes,
            sig_type: SignatureType::Ed25519,
            pub_key: pub_key.to_vec(),
            compressed: true,
        };
    }
    let mut bytes = seed;
    let k = loop {
        match secp256k1::PrivateKey::from_bytes(&bytes) {
            Some(k) => break k,
            // Zero, or at least the group order (whose first byte is
            // 0xff).
            None => bytes[0] = if bytes[0] == 0 { 1 } else { bytes[0] >> 1 },
        }
    };
    let pk = k.public_key();
    let compressed = kind % 4 != 1;
    Key {
        priv_bytes: bytes.to_vec(),
        sig_type: if kind % 4 == 3 {
            SignatureType::SchnorrSecp256k1
        } else {
            SignatureType::EcdsaSecp256k1
        },
        pub_key: if compressed {
            pk.serialize_compressed().to_vec()
        } else {
            pk.serialize_uncompressed().to_vec()
        },
        compressed,
    }
}

fn pkh_address(k: &Key, p: &Params) -> Address {
    let h = stdaddr::hash160(&k.pub_key);
    match k.sig_type {
        SignatureType::EcdsaSecp256k1 => {
            stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&h, p)
        }
        SignatureType::Ed25519 => stdaddr::new_address_pub_key_hash_ed25519_v0(&h, p),
        SignatureType::SchnorrSecp256k1 => {
            stdaddr::new_address_pub_key_hash_schnorr_secp256k1_v0(&h, p)
        }
    }
    .expect("a 20-byte hash")
}

fn push(script: &mut Vec<u8>, data: &[u8]) {
    match data.len() {
        0 => script.push(0),
        n @ 1..=75 => script.push(n as u8),
        n @ 76..=255 => script.extend_from_slice(&[0x4c, n as u8]),
        n @ 256..=65535 => {
            script.push(0x4d);
            script.extend_from_slice(&(n as u16).to_le_bytes());
        }
        n => {
            script.push(0x4e);
            script.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
    script.extend_from_slice(data);
}

/// The pay-to-pubkey script of `k`: `<pk> OP_CHECKSIG`, or
/// `<pk> <type> OP_CHECKSIGALT` for Ed25519 and Schnorr.
fn p2pk(k: &Key) -> Vec<u8> {
    let mut s = Vec::new();
    push(&mut s, &k.pub_key);
    match k.sig_type {
        SignatureType::EcdsaSecp256k1 => s.push(0xac),
        SignatureType::Ed25519 => s.extend_from_slice(&[0x51, 0xbe]),
        SignatureType::SchnorrSecp256k1 => s.extend_from_slice(&[0x52, 0xbe]),
    }
    s
}

fn p2sh(redeem: &[u8]) -> Vec<u8> {
    let mut s = vec![0xa9, 0x14];
    s.extend_from_slice(&stdaddr::hash160(redeem));
    s.push(0x87);
    s
}

/// m-of-n bare multisig over the compressed ECDSA keys among `keys` (the
/// standard form takes no other).
fn multisig(keys: &[Key], m: u8) -> Option<Vec<u8>> {
    let pks: Vec<&Key> = keys
        .iter()
        .filter(|k| k.sig_type == SignatureType::EcdsaSecp256k1 && k.compressed)
        .collect();
    if pks.is_empty() {
        return None;
    }
    let n = pks.len() as u8;
    let m = 1 + m % n;
    let mut s = vec![0x50 + m];
    for k in &pks {
        push(&mut s, &k.pub_key);
    }
    s.extend_from_slice(&[0x50 + n, 0xae]);
    Some(s)
}

/// Stake-tagging opcodes: OP_SSTX, OP_SSGEN, OP_SSRTX, OP_SSTXCHANGE,
/// OP_TGEN.
const STAKE_TAGS: [u8; 5] = [0xba, 0xbb, 0xbc, 0xbd, 0xc3];

/// A standard script over `keys` (honest), or any script.
fn script(
    b: &mut Bytes<'_>,
    p: &Params,
    keys: &[Key],
    redeems: &[Vec<u8>],
    honest: bool,
) -> Vec<u8> {
    let k = &keys[usize::from(b.u8()) % keys.len()];
    let kinds = if honest { 4 } else { 10 };
    match b.u8() % kinds {
        0 => pkh_address(k, p).payment_script().1,
        1 => p2pk(k),
        2 => multisig(keys, b.u8()).unwrap_or_else(|| pkh_address(k, p).payment_script().1),
        3 => match redeems.get(usize::from(b.u8()) % redeems.len().max(1)) {
            Some(r) => p2sh(r),
            None => pkh_address(k, p).payment_script().1,
        },
        4 => {
            let mut s = vec![STAKE_TAGS[usize::from(b.u8() % 5)]];
            if b.bool() {
                s.extend(pkh_address(k, p).payment_script().1);
            } else {
                s.extend(p2sh(redeems.first().map_or(&[][..], Vec::as_slice)));
            }
            s
        }
        // A pay-to-pubkey script whose key need not parse (QK-0745).
        5 => {
            let mut s = Vec::new();
            let mut pk = vec![2 + (b.u8() & 1)];
            pk.extend_from_slice(&b.seed());
            push(&mut s, &pk);
            s.push(0xac);
            s
        }
        6 => {
            let mut s = vec![0x6a];
            let len = usize::from(b.u8() % 40);
            let d = b.take(len);
            push(&mut s, &d);
            s
        }
        // Treasury add: a lone OP_TADD, standard but unsignable.
        7 => vec![0xc1],
        _ => {
            let n = usize::from(b.u8() % 48);
            b.take(n)
        }
    }
}

/// The secrets source: honest keys and redeem scripts by address, then
/// whatever the input says for other addresses.
struct Secrets {
    params: Params,
    keys: Vec<(Address, Key)>,
    scripts: Vec<(Address, Vec<u8>)>,
    /// Answers for unknown addresses, picked by a hash of the address.
    odd_keys: Vec<Result<(Vec<u8>, SignatureType, bool), Kind>>,
    odd_scripts: Vec<Result<Vec<u8>, Kind>>,
}

fn addr_hash(a: &Address) -> usize {
    a.encode().bytes().fold(0usize, |h, c| {
        h.wrapping_mul(31).wrapping_add(usize::from(c))
    })
}

impl SecretsSource for Secrets {
    fn chain_params(&self) -> &Params {
        &self.params
    }
    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError> {
        if let Some((_, k)) = self.keys.iter().find(|(a, _)| a == addr) {
            return Ok((k.priv_bytes.clone(), k.sig_type, k.compressed));
        }
        if self.odd_keys.is_empty() {
            return Err(source_error("no key", Kind::NotExist));
        }
        let i = addr_hash(addr) % self.odd_keys.len();
        self.odd_keys[i]
            .clone()
            .map_err(|kind| source_error("key lookup failed", kind))
    }
    fn get_script(&mut self, addr: &Address) -> Result<Vec<u8>, AnyError> {
        if let Some((_, s)) = self.scripts.iter().find(|(a, _)| a == addr) {
            return Ok(s.clone());
        }
        if self.odd_scripts.is_empty() {
            return Err(source_error("no script", Kind::NotExist));
        }
        let i = addr_hash(addr) % self.odd_scripts.len();
        self.odd_scripts[i]
            .clone()
            .map_err(|kind| source_error("script lookup failed", kind))
    }
}

fn params(b: &mut Bytes<'_>) -> Params {
    match b.u8() % 4 {
        0 => dcroxide_chaincfg::mainnet_params(),
        1 => dcroxide_chaincfg::testnet3_params(),
        2 => dcroxide_chaincfg::simnet_params(),
        _ => dcroxide_chaincfg::regnet_params(),
    }
}

fn keys(b: &mut Bytes<'_>, n: usize, kinds: u8) -> Vec<Key> {
    (0..n)
        .map(|_| {
            let kind = b.u8() % kinds;
            key(kind, b.seed())
        })
        .collect()
}

/// Every address a key answers for: its P2PKH address and its P2PK
/// address, which for ECDSA is the compressed key's whichever form the
/// script holds (stdscript re-serializes it).
fn key_entries(keys: &[Key], p: &Params) -> Vec<(Address, Key)> {
    let mut out = Vec::new();
    for k in keys {
        out.push((pkh_address(k, p), k.clone()));
        let pk = match k.sig_type {
            SignatureType::EcdsaSecp256k1 => {
                secp256k1::PublicKey::parse(&k.pub_key).ok().map(|pk| {
                    stdaddr::new_address_pub_key_ecdsa_secp256k1_v0(pk.serialize_compressed(), p)
                })
            }
            SignatureType::Ed25519 => {
                stdaddr::new_address_pub_key_ed25519_v0_raw(&k.pub_key, p).ok()
            }
            SignatureType::SchnorrSecp256k1 => {
                stdaddr::new_address_pub_key_schnorr_secp256k1_v0_raw(&k.pub_key, p).ok()
            }
        };
        out.extend(pk.map(|a| (a, k.clone())));
    }
    out
}

fn flags() -> ScriptFlags {
    ScriptFlags::DISCOURAGE_UPGRADABLE_NOPS
        .union(ScriptFlags::VERIFY_CLEAN_STACK)
        .union(ScriptFlags::VERIFY_CHECK_LOCK_TIME_VERIFY)
        .union(ScriptFlags::VERIFY_CHECK_SEQUENCE_VERIFY)
        .union(ScriptFlags::VERIFY_TREASURY)
}

fn verify(tx: &MsgTx, prev: &[Vec<u8>]) {
    for (i, script) in prev.iter().enumerate() {
        let mut engine = Engine::new(script, tx, i, flags(), 0).expect("engine");
        engine
            .execute()
            .unwrap_or_else(|e| panic!("input {i} does not verify: {e:?}"));
    }
}

/// Mode 2: author with sane values from P2PKH UTXOs, sign, verify.
fn author_and_sign(b: &mut Bytes<'_>) {
    let p = params(b);
    // Keys whose P2PKH signature script fits REDEEM_P2PKH_SIG_SCRIPT_SIZE:
    // compressed ECDSA, Ed25519, Schnorr.
    let n_keys = 1 + usize::from(b.u8() % 3);
    let pool: Vec<Key> = keys(b, n_keys, 4)
        .into_iter()
        .map(|k| {
            if k.compressed {
                k
            } else {
                key(0, k.priv_bytes[..32].try_into().unwrap())
            }
        })
        .collect();
    let n_out = usize::from(b.u8() % 4);
    let outs: Vec<TxOut> = (0..n_out)
        .map(|_| TxOut {
            value: i64::from(b.u32()) * 100,
            version: 0,
            pk_script: pkh_address(&pool[0], &p).payment_script().1,
        })
        .collect();
    let rate = i64::from(b.u16()) * 10;
    let n = 1 + usize::from(b.u8() % 4);
    let utxos: Vec<(i64, usize)> = (0..n)
        .map(|_| (i64::from(b.u32()) * 1_000, usize::from(b.u8()) % pool.len()))
        .collect();
    let change_script = pkh_address(&pool[pool.len() - 1], &p).payment_script().1;
    let mut change = Change {
        size: change_script.len() as i64,
        script: change_script,
        version: 0,
        error: None,
        script_calls: 0,
    };
    let mut detail = InputDetail::default();
    let mut next = 0usize;
    let mut have = 0i64;
    let source = |target: i64| -> Result<InputDetail, AnyError> {
        while next < utxos.len() && have < target {
            let (value, k) = utxos[next];
            detail.inputs.push(tx_in(next, value, Vec::new()));
            detail
                .scripts
                .push(pkh_address(&pool[k], &p).payment_script().1);
            detail
                .redeem_script_sizes
                .push(REDEEM_P2PKH_SIG_SCRIPT_SIZE);
            have += value;
            next += 1;
        }
        let mut d = detail.clone();
        d.amount = Amount(have);
        Ok(d)
    };
    let mut src = Recorded::new(source, MAX_CALLS);
    let result =
        new_unsigned_transaction(outs.clone(), Amount(rate), &mut src, &mut change, 100_000);
    check_authoring(&outs, rate, 100_000, &change, &src.calls, &result);
    let Ok(mut atx) = result else {
        return;
    };
    let mut secrets = Secrets {
        params: p.clone(),
        keys: key_entries(&pool, &p),
        scripts: Vec::new(),
        odd_keys: Vec::new(),
        odd_scripts: Vec::new(),
    };
    atx.add_all_input_scripts(&mut secrets)
        .expect("honest inputs sign");
    verify(&atx.tx, &atx.prev_scripts);
    let signed = atx.tx.serialize_size() as i64;
    let under: i64 = atx
        .tx
        .tx_in
        .iter()
        .map(|i| REDEEM_P2PKH_SIG_SCRIPT_SIZE - i.signature_script.len() as i64)
        .sum();
    assert!(under >= 0, "a signature script above the worst case");
    assert_eq!(atx.estimated_signed_serialize_size, signed + under);
    let paid = atx.total_input.0 - sum(&atx.tx.tx_out);
    assert!(
        paid >= fee_for_serialize_size(Amount(rate), signed).0,
        "underpaid"
    );
}

/// The secrets source of the built shapes: the pool's keys and the given
/// redeem scripts by their pay-to-script-hash addresses, nothing else.
fn known(p: &Params, pool: &[Key], redeems: &[Vec<u8>]) -> Secrets {
    Secrets {
        params: p.clone(),
        keys: key_entries(pool, p),
        scripts: redeems
            .iter()
            .map(|r| {
                let a = stdaddr::new_address_script_hash_v0(r, p).expect("a script hash");
                (a, r.clone())
            })
            .collect(),
        odd_keys: Vec::new(),
        odd_scripts: Vec::new(),
    }
}

/// A transaction whose first `before` inputs spend honest P2PKH outputs
/// of the pool with empty signature scripts, followed by one input with
/// signature script `sig` spending `last`, and the previous scripts.
fn ahead_of(
    b: &mut Bytes<'_>,
    p: &Params,
    pool: &[Key],
    sig: Vec<u8>,
    last: Vec<u8>,
) -> (MsgTx, Vec<Vec<u8>>) {
    let before = usize::from(b.u8() % 3);
    let mut tx = MsgTx::default();
    let mut prev = Vec::new();
    for i in 0..before {
        let k = &pool[usize::from(b.u8()) % pool.len()];
        tx.tx_in.push(tx_in(i, b.amount(), Vec::new()));
        prev.push(pkh_address(k, p).payment_script().1);
    }
    tx.tx_in.push(tx_in(before, b.amount(), sig));
    prev.push(last);
    tx.tx_out.push(TxOut {
        value: 1,
        version: 0,
        pk_script: vec![0x6a],
    });
    (tx, prev)
}

/// Signs `tx` and checks that it fails with exactly `text`, having
/// changed nothing but the signature scripts of the inputs before the
/// last, which sign.
fn assert_refused(tx: &mut MsgTx, prev: &[Vec<u8>], secrets: &mut Secrets, text: &str) {
    let before = tx.clone();
    let r = add_all_input_scripts(tx, prev, secrets);
    assert_eq!(r.map_err(|e| e.render(SEP)), Err(text.to_owned()));
    let last = before.tx_in.len() - 1;
    assert_eq!(tx.tx_in[last], before.tx_in[last]);
    assert_eq!(tx.tx_out, before.tx_out);
    for i in 0..last {
        assert!(!tx.tx_in[i].signature_script.is_empty(), "input {i} signs");
    }
}

/// Mode 3's rare shape of QK-0746, which random scripts reach about once
/// in 10^4 to 10^5 inputs: a previous script paying to the hash of a
/// redeem script that itself pays to the hash of a standard script, both
/// known to the secrets source, and an existing signature script that is
/// nonempty and parses. dcrd's merge recurses on the new signature
/// script's final push, the inner pay-to-script-hash script, until the
/// stack overflows; the crate must refuse it with
/// [`NESTED_SCRIPT_HASH_MERGE`] (and a crate without its guard overflows
/// the stack here).
fn nested_script_hash_merge(b: &mut Bytes<'_>) {
    let p = params(b);
    let n_keys = 1 + usize::from(b.u8() % 3);
    let pool = keys(b, n_keys, 4);
    let inner = script(b, &p, &pool, &[], true);
    let outer = p2sh(&inner);
    // Pushes always parse; one of them may be a redeem script.
    let mut sig = Vec::new();
    for _ in 0..1 + b.u8() % 3 {
        match b.u8() % 4 {
            0 => sig.push(0x51),
            1 => push(&mut sig, &inner),
            2 => push(&mut sig, &outer),
            _ => {
                let len = usize::from(b.u8() % 40);
                let d = b.take(len);
                push(&mut sig, &d);
            }
        }
    }
    let (mut tx, prev) = ahead_of(b, &p, &pool, sig, p2sh(&outer));
    let mut secrets = known(&p, &pool, &[inner, outer]);
    assert_refused(&mut tx, &prev, &mut secrets, NESTED_SCRIPT_HASH_MERGE);
}

/// Mode 3's rare shape of QK-0745: a pay-to-pubkey script, ECDSA or
/// Schnorr, whose compressed key has an x coordinate of at least the field
/// prime, so that it classifies as pay-to-pubkey but yields no address;
/// dcrd's signer indexes the empty address list and panics. It is the
/// previous script or, behind a pay-to-script-hash previous script, the
/// redeem script; the crate must refuse it with [`INVALID_PUBKEY_SCRIPT`].
fn unparsable_pay_to_pubkey(b: &mut Bytes<'_>) {
    let p = params(b);
    let n_keys = 1 + usize::from(b.u8() % 3);
    let pool = keys(b, n_keys, 4);
    // 2^256 - 2^32 <= x < 2^256, above the prime 2^256 - 2^32 - 977.
    let mut pk = vec![2 + (b.u8() & 1)];
    pk.extend_from_slice(&[0xff; 28]);
    pk.extend(b.take(4));
    let mut bad = Vec::new();
    push(&mut bad, &pk);
    if b.bool() {
        bad.push(0xac);
    } else {
        bad.extend_from_slice(&[0x52, 0xbe]);
    }
    let sig = if b.bool() {
        Vec::new()
    } else {
        let len = usize::from(b.u8() % 40);
        b.take(len)
    };
    let (last, redeems) = if b.bool() {
        (bad, Vec::new())
    } else {
        (p2sh(&bad), vec![bad])
    };
    let (mut tx, prev) = ahead_of(b, &p, &pool, sig, last);
    let mut secrets = known(&p, &pool, &redeems);
    assert_refused(&mut tx, &prev, &mut secrets, INVALID_PUBKEY_SCRIPT);
}

/// Mode 3: sign an arbitrary transaction, or, rarely, one of the shapes
/// where Go dies.
fn sign_arbitrary(b: &mut Bytes<'_>) {
    match b.u8() % 16 {
        1 => return nested_script_hash_merge(b),
        2 => return unparsable_pay_to_pubkey(b),
        _ => {}
    }
    let p = params(b);
    let honest = b.u8() % 4 == 1;
    let n_keys = 1 + usize::from(b.u8() % 3);
    let pool = keys(b, n_keys, 4);
    // Redeem scripts: standard over the pool, and, unless honest, P2SH of
    // one of them (the nested shape of QK-0746) or anything at all.
    let mut redeems: Vec<Vec<u8>> = Vec::new();
    for _ in 0..1 + b.u8() % 3 {
        let r = script(b, &p, &pool, &[], true);
        redeems.push(r);
    }
    if !honest {
        for _ in 0..b.u8() % 3 {
            let r = script(b, &p, &pool, &redeems.clone(), false);
            redeems.push(r);
        }
    }
    let n = usize::from(b.u8() % 5);
    let mut tx = MsgTx {
        version: b.u16(),
        lock_time: b.u32(),
        expiry: b.u32(),
        ..MsgTx::default()
    };
    let mut prev = Vec::new();
    for i in 0..n {
        let sig = if honest {
            Vec::new()
        } else {
            match b.u8() % 4 {
                0 | 1 => Vec::new(),
                2 => {
                    // Ends in a push of a redeem script, as a P2SH
                    // signature script does.
                    let mut s = vec![0x51];
                    let r = &redeems[usize::from(b.u8()) % redeems.len()];
                    push(&mut s, r);
                    s
                }
                _ => {
                    let len = usize::from(b.u8() % 40);
                    b.take(len)
                }
            }
        };
        tx.tx_in.push(tx_in(i, b.amount(), sig));
        prev.push(script(b, &p, &pool, &redeems, honest));
    }
    tx.tx_out.push(TxOut {
        value: 1,
        version: 0,
        pk_script: vec![0x6a],
    });
    if !honest && b.u8() % 16 == 1 {
        if b.bool() || prev.is_empty() {
            prev.push(Vec::new());
        } else {
            prev.pop();
        }
    }
    let mut scripts: Vec<(Address, Vec<u8>)> = redeems
        .iter()
        .filter_map(|r| Some((stdaddr::new_address_script_hash_v0(r, &p).ok()?, r.clone())))
        .collect();
    let mut odd_keys = Vec::new();
    let mut odd_scripts = Vec::new();
    if !honest {
        if b.u8() % 4 == 1 {
            scripts.clear();
        }
        for _ in 0..b.u8() % 4 {
            odd_keys.push(if b.u8() % 4 == 0 {
                Err(b.kind())
            } else {
                let len = match b.u8() % 4 {
                    0 => 32,
                    1 => 64,
                    _ => usize::from(b.u8() % 70),
                };
                let st = [
                    SignatureType::EcdsaSecp256k1,
                    SignatureType::Ed25519,
                    SignatureType::SchnorrSecp256k1,
                ][usize::from(b.u8() % 3)];
                Ok((b.take(len), st, b.bool()))
            });
        }
        for _ in 0..b.u8() % 3 {
            odd_scripts.push(if b.u8() % 4 == 0 {
                Err(b.kind())
            } else {
                Ok(script(b, &p, &pool, &redeems, false))
            });
        }
    }
    let make = || Secrets {
        params: p.clone(),
        keys: key_entries(&pool, &p),
        scripts: scripts.clone(),
        odd_keys: odd_keys.clone(),
        odd_scripts: odd_scripts.clone(),
    };
    let before = tx.clone();
    let mut tx2 = tx.clone();
    let r1 = add_all_input_scripts(&mut tx, &prev, &mut make());
    let r2 = add_all_input_scripts(&mut tx2, &prev, &mut make());
    assert_eq!(tx, tx2, "signing is deterministic");
    assert_eq!(
        r1.as_ref().map_err(|e| e.render(SEP)),
        r2.as_ref().map_err(|e| e.render(SEP))
    );
    // Only signature scripts change.
    assert_eq!(
        (tx.version, tx.lock_time, tx.expiry, &tx.tx_out, tx.ser_type),
        (
            before.version,
            before.lock_time,
            before.expiry,
            &before.tx_out,
            before.ser_type
        )
    );
    assert_eq!(tx.tx_in.len(), before.tx_in.len());
    for (a, b) in tx.tx_in.iter().zip(&before.tx_in) {
        assert_eq!(
            TxIn {
                signature_script: Vec::new(),
                ..a.clone()
            },
            TxIn {
                signature_script: Vec::new(),
                ..b.clone()
            }
        );
    }
    if prev.len() != before.tx_in.len() {
        assert_eq!(
            r1.map_err(|e| e.render(SEP)),
            Err("tx.TxIn and prevPkScripts slices must have equal length".to_owned())
        );
        assert_eq!(tx, before);
        return;
    }
    if honest {
        r1.unwrap_or_else(|e| panic!("honest inputs must sign: {}", e.render(SEP)));
        verify(&tx, &prev);
    }
}

fuzz_target!(|data: &[u8]| {
    let mut b = Bytes::new(data);
    match b.u8() % 4 {
        0 => author_cumulative(&mut b),
        1 => author_adversarial(&mut b),
        2 => author_and_sign(&mut b),
        _ => sign_arbitrary(&mut b),
    }
});
