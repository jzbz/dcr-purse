// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Multisig outputs: `AddMultisigOut`, `SpendMultisigOut`
//! (`txmined.go:1711-1865`), `GetMultisigOutput` and
//! `UnspentMultisigCreditsForAddress` (`txmined.go:2852-2927`), and the
//! marks `Rollback` and `RemoveUnconfirmed` write (`setMultisigOutUnmined`,
//! `setMultisigOutUnSpent`, `txdb.go:1904-1922`).
//!
//! The record is `purse-store`'s [`MultisigOutRow`], written through
//! [`WriteTx::put_multisig_out`], which keeps `multisig_unspent` (keyed by
//! the script's HASH160, then the outpoint) holding exactly the unspent
//! outputs, as Go keeps `mu` beside `ms`. Go's 135-byte value also holds
//! the transaction hash, always the key's, and the stale spender bytes
//! QK-0055 describes, which no reader shows while the output is unspent;
//! neither is stored. A spend or an unspend clears the stake-tree flag, as
//! Go's do (QK-0054, reproduced; pending owner). QK-0734's fix marks a
//! revalidated transaction's records mined and spent again (`mark_mined`,
//! `mark_spent`; QK-0789, pending owner).

use dcroxide_txscript::stdaddr::Address;
use dcroxide_txscript::stdscript::{
    ScriptType, determine_script_type, extract_multi_sig_script_details_v0,
};
use dcroxide_wire::OutPoint;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    AddrByHash, Hash32, Hash160, MultisigBlock, MultisigOutRow, MultisigOuts, MultisigSpender,
    MultisigUnspent, MultisigUnspentKey, Outpoint, TableId, WriteTx,
};

use super::record::{BlockMeta, TxRecord, canonical, display};
use super::store::{TxRead, TxStore, store_err};
use crate::addrmgr::redeem_script_for_hash160;

/// A P2SH multisig output the wallet tracks (dcrwallet's `MultisigOut`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultisigOut {
    /// The output; its tree is always the regular tree, as Go's
    /// `fetchMultisigOut` sets it.
    pub outpoint: OutPoint,
    /// The tree the record holds: 1 for an output in the stake tree, until
    /// a spend or an unspend clears it (QK-0054).
    pub tree: i8,
    /// HASH160 of the redeem script.
    pub script_hash: Hash160,
    /// Signatures required.
    pub m: u8,
    /// Public keys in the script.
    pub n: u8,
    /// The transaction's hash (the outpoint's).
    pub tx_hash: Hash32,
    /// The mining block's hash; all zero while unmined.
    pub block_hash: Hash32,
    /// The mining block's height (Go's `uint32`); 0 while unmined.
    pub block_height: u32,
    /// Amount in atoms.
    pub amount: i64,
    /// The spending input, while spent (Go's `Spent`, `SpentBy` and
    /// `SpentByIndex`; the last two are read only while `Spent` is set,
    /// `wallet/multisig.go:78-83`).
    pub spender: Option<MultisigSpender>,
}

/// A redeemable P2SH multisig credit (dcrwallet's `MultisigCredit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultisigCredit {
    /// The output, with the tree the record holds.
    pub outpoint: OutPoint,
    /// HASH160 of the redeem script.
    pub script_hash: Hash160,
    /// The redeem script.
    pub ms_script: Vec<u8>,
    /// Signatures required.
    pub m: u8,
    /// Public keys in the script.
    pub n: u8,
    /// Amount in atoms.
    pub amount: i64,
}

/// `txrules.StakeSubScriptType` (`wallet/txrules/rules.go:19-35`) as far
/// as `AddMultisigOut` needs it: whether `st` is P2SH, the stake-tagged and
/// treasury-generated forms included, and whether it is one of those.
fn p2sh_class(st: ScriptType) -> (bool, bool) {
    use ScriptType as S;
    match st {
        S::ScriptHash => (true, false),
        S::StakeSubmissionScriptHash
        | S::StakeChangeScriptHash
        | S::StakeGenScriptHash
        | S::StakeRevocationScriptHash
        | S::TreasuryGenScriptHash => (true, true),
        _ => (false, false),
    }
}

/// `getScriptHashFromP2SHScript` (`txmined.go:1712-1736`) of a script
/// `determine_script_type` classed P2SH (possibly stake-tagged): every such
/// script ends `OP_HASH160 OP_DATA_20 <hash> OP_EQUAL`, the tag, if any,
/// before it, so the push the tokenizer finds after the first `OP_HASH160`
/// is the 20 bytes before the last.
fn p2sh_script_hash(script: &[u8]) -> Option<[u8; 20]> {
    let end = script.len().checked_sub(1)?;
    let start = end.checked_sub(20)?;
    script.get(start..end)?.try_into().ok()
}

/// The block a multisig record names for `block` (Go's dummy block of the
/// zero hash at height 0 when there is none). The store cannot hold a block
/// height beside the zero hash, which Go writes for a block meta of the
/// zero hash and another height; no caller passes such a block (ADR-0012
/// Rule 4; PARITY.md, `purse-udb`), and it is refused.
///
/// # Errors
///
/// As described: `Invalid`.
fn record_block(block: Option<&BlockMeta>) -> Result<Option<MultisigBlock>, AnyError> {
    let Some(b) = block else {
        return Ok(None);
    };
    let height = b.block.height.cast_unsigned();
    match MultisigBlock::new(b.block.hash, height) {
        Some(m) => Ok(Some(m)),
        None if height == 0 => Ok(None),
        None => Err(e!(
            Kind::Invalid,
            format!(
                "multisig output block of the zero hash at height {}: not representable",
                b.block.height
            )
        )
        .into()),
    }
}

/// Go's `redeemScriptForHash160` through the transaction's reader.
fn redeem_script(r: &mut (impl TxRead + ?Sized), hash: &[u8; 20]) -> Result<Vec<u8>, AnyError> {
    let row = r.get::<AddrByHash>(&Hash160(*hash)).map_err(store_err)?;
    redeem_script_for_hash160(row, hash)
}

/// Go's text for an outpoint with no multisig record.
fn no_output(op: &OutPoint) -> AnyError {
    e!(
        Kind::NotExist,
        format!("no multisig output for outpoint {op}")
    )
    .into()
}

/// Whether the store holds any multisig record: `Rollback` and
/// `RemoveUnconfirmed` look a record up for every credit output or input
/// only when it does (one emptiness test per operation, where Go makes one
/// lookup per output or input; a wallet without multisig outputs makes no
/// lookup at all).
pub(crate) fn any_multisig(r: &mut (impl TxRead + ?Sized)) -> Result<bool, AnyError> {
    Ok(!r.table_is_empty(TableId::MultisigOuts).map_err(store_err)?)
}

/// `setMultisigOutUnmined` on the record of `op`, if any (`Rollback`'s
/// credit outputs): its block cleared, whichever block it names, as Go's
/// is (QK-0789: a record naming a block the `Rollback` does not detach is
/// cleared too).
pub(crate) fn mark_unmined(tx: &mut WriteTx<'_>, op: &Outpoint) -> Result<(), AnyError> {
    let Some(mut row) = tx.get::<MultisigOuts>(op).map_err(store_err)? else {
        return Ok(());
    };
    if row.block.is_some() {
        row.block = None;
        tx.put_multisig_out(op, &row).map_err(store_err)?;
    }
    Ok(())
}

/// `setMultisigOutUnSpent` and `putMultisigOutUS` on the record of `op`,
/// if any (`Rollback`'s restored debits, every input of a transaction
/// `RemoveUnconfirmed` removes, whichever transaction spent the output):
/// its spender and its stake-tree flag cleared (QK-0054), its
/// `multisig_unspent` entry written.
pub(crate) fn mark_unspent(tx: &mut WriteTx<'_>, op: &Outpoint) -> Result<(), AnyError> {
    let Some(mut row) = tx.get::<MultisigOuts>(op).map_err(store_err)? else {
        return Ok(());
    };
    if row.spender.is_some() || row.stake_tree {
        row.spender = None;
        row.stake_tree = false;
        tx.put_multisig_out(op, &row).map_err(store_err)?;
    }
    Ok(())
}

/// QK-0734's fix (`mined.rs`, `merge_unmined_copy`; QK-0789, pending
/// owner): the record of `op`, if any and unmined, marked mined in the
/// block (`hash`, `height`), as `AddMultisigOut` with that block marks an
/// unmined record.
pub(crate) fn mark_mined(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    hash: &Hash32,
    height: u32,
) -> Result<(), AnyError> {
    let Some(mut row) = tx.get::<MultisigOuts>(op).map_err(store_err)? else {
        return Ok(());
    };
    if row.block.is_none()
        && let Some(b) = MultisigBlock::new(*hash, height)
    {
        row.block = Some(b);
        tx.put_multisig_out(op, &row).map_err(store_err)?;
    }
    Ok(())
}

/// QK-0734's fix (`mined.rs`, `merge_unmined_copy`; QK-0789, pending
/// owner): the record of `op`, if any and unspent, marked spent by input
/// `input` of `hash`, as `SpendMultisigOut` marks it (its stake-tree flag
/// cleared, QK-0054, and its `multisig_unspent` entry deleted). A record
/// another input spends is left as it is.
pub(crate) fn mark_spent(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    hash: &Hash32,
    input: u32,
) -> Result<(), AnyError> {
    let Some(mut row) = tx.get::<MultisigOuts>(op).map_err(store_err)? else {
        return Ok(());
    };
    if row.spender.is_none() {
        row.spender = Some(MultisigSpender { hash: *hash, input });
        row.stake_tree = false;
        tx.put_multisig_out(op, &row).map_err(store_err)?;
    }
    Ok(())
}

impl TxStore {
    /// `AddMultisigOut`: records output `index` of `rec`, a P2SH output of
    /// a redeem script the address manager holds as an imported script with
    /// a valid m-of-n multisig script, as unspent, mined in `block` or
    /// unmined. An output already recorded unmined is marked mined in
    /// `block` (nothing else is checked), and left as it is without a
    /// block; one recorded mined is refused with a block and, without one,
    /// written again as unmined and unspent (QK-0097, reproduced).
    ///
    /// # Errors
    ///
    /// Go's, in Go's order: no output `index`: `Invalid` "transaction
    /// output does not exist"; already mined, with a block: `Invalid`
    /// "multisig credit is mined"; not P2SH: `Invalid` "multisig output
    /// must be P2SH"; the address manager's redeem-script lookup, as it
    /// returns it (`NotExist`, or `Invalid` for an address that is not an
    /// imported script); a script version other than 0: `IO`; a redeem
    /// script that is not m-of-n multisig: `IO` "invalid m-of-n multisig
    /// script". A block of the zero hash at a height other than 0, which
    /// the store cannot hold: `Invalid`, nothing written. The store's.
    pub fn add_multisig_out(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        block: Option<&BlockMeta>,
        index: u32,
    ) -> Result<(), AnyError> {
        let Some(out) = usize::try_from(index)
            .ok()
            .and_then(|i| rec.msg_tx().tx_out.get(i))
        else {
            return Err(e!(Kind::Invalid, "transaction output does not exist").into());
        };
        let op = Outpoint {
            hash: *rec.hash(),
            index,
        };
        let existing = tx.get::<MultisigOuts>(&op).map_err(store_err)?;
        match (existing, block) {
            (Some(old), Some(_)) => {
                if old.block.is_some() {
                    return Err(e!(Kind::Invalid, "multisig credit is mined").into());
                }
                // Go rewrites the record with the block; the zero hash at
                // height 0 leaves it as it was.
                if let Some(b) = record_block(block)? {
                    let row = MultisigOutRow {
                        block: Some(b),
                        ..old
                    };
                    tx.put_multisig_out(&op, &row).map_err(store_err)?;
                }
                return Ok(());
            }
            (Some(old), None) if old.block.is_none() => return Ok(()),
            _ => {}
        }
        let (is_p2sh, stake_tree) = p2sh_class(determine_script_type(out.version, &out.pk_script));
        let script_hash = match p2sh_script_hash(&out.pk_script) {
            Some(h) if is_p2sh => h,
            _ => return Err(e!(Kind::Invalid, "multisig output must be P2SH").into()),
        };
        let script = redeem_script(tx, &script_hash)?;
        // Unreachable: a script of another version is not P2SH.
        if out.version != 0 {
            return Err(e!(Kind::Io, "only version 0 scripts are supported").into());
        }
        let details = extract_multi_sig_script_details_v0(&script, false);
        if !details.valid {
            return Err(e!(Kind::Io, "invalid m-of-n multisig script").into());
        }
        let block = record_block(block)?;
        // Go's uint8 conversions: a valid script has at most 16 keys.
        let row = MultisigOutRow {
            script_hash: Hash160(script_hash),
            m: u8::try_from(details.required_sigs & 0xff).unwrap_or(u8::MAX),
            n: u8::try_from(details.num_pub_keys & 0xff).unwrap_or(u8::MAX),
            stake_tree,
            block,
            amount: out.value,
            spender: None,
        };
        tx.put_multisig_out(&op, &row).map_err(store_err)
    }

    /// `SpendMultisigOut`: marks the multisig output `op` spent by input
    /// `spend_index` of `spend_hash` and removes it from the unspent
    /// outputs; the stake-tree flag is cleared, as Go's spend clears it
    /// (QK-0054). Spending it again by the same input changes nothing.
    ///
    /// # Errors
    ///
    /// Go's: no record: `NotExist` `no multisig output for outpoint
    /// <hash>:<index>`; spent by another input: `DoubleSpend` `outpoint
    /// <hash>:<index> spent by <hash>`; an unspent record without its
    /// `multisig_unspent` entry, which the store's writer never leaves:
    /// `IO` "missing unspent multisig record". The store's.
    pub fn spend_multisig_out(
        &self,
        tx: &mut WriteTx<'_>,
        op: &OutPoint,
        spend_hash: &Hash32,
        spend_index: u32,
    ) -> Result<(), AnyError> {
        let key = canonical(op);
        let Some(mut row) = tx.get::<MultisigOuts>(&key).map_err(store_err)? else {
            return Err(no_output(op));
        };
        if let Some(s) = row.spender {
            if s.hash == *spend_hash && s.input == spend_index {
                return Ok(());
            }
            return Err(e!(
                Kind::DoubleSpend,
                format!("outpoint {op} spent by {}", display(&s.hash))
            )
            .into());
        }
        let us = MultisigUnspentKey {
            script_hash: row.script_hash,
            outpoint: key,
        };
        if !tx.contains::<MultisigUnspent>(&us).map_err(store_err)? {
            return Err(e!(Kind::Io, "missing unspent multisig record").into());
        }
        row.spender = Some(MultisigSpender {
            hash: *spend_hash,
            input: spend_index,
        });
        row.stake_tree = false;
        tx.put_multisig_out(&key, &row).map_err(store_err)
    }

    /// `GetMultisigOutput`: the multisig record of `op`.
    ///
    /// # Errors
    ///
    /// No record: `NotExist` `no multisig output for outpoint
    /// <hash>:<index>`. The store's.
    pub fn get_multisig_output(
        &self,
        r: &mut (impl TxRead + ?Sized),
        op: &OutPoint,
    ) -> Result<MultisigOut, AnyError> {
        let key = canonical(op);
        let Some(row) = r.get::<MultisigOuts>(&key).map_err(store_err)? else {
            return Err(no_output(op));
        };
        let (block_hash, block_height) = row
            .block
            .map_or((Hash32::default(), 0), |b| (b.hash(), b.height()));
        Ok(MultisigOut {
            outpoint: OutPoint {
                hash: op.hash,
                index: op.index,
                tree: 0,
            },
            tree: i8::from(row.stake_tree),
            script_hash: row.script_hash,
            m: row.m,
            n: row.n,
            tx_hash: key.hash,
            block_hash,
            block_height,
            amount: row.amount,
            spender: row.spender,
        })
    }

    /// `UnspentMultisigCreditsForAddress`: the unspent multisig outputs
    /// paying the P2SH address `addr`, in outpoint order, as Go's walk of
    /// the whole `mu` bucket keeps them: here one prefix walk of
    /// `multisig_unspent`, which is keyed by the script hash first, and one
    /// redeem-script lookup, where Go makes one per output (the same
    /// script, and so the same answer or error).
    ///
    /// # Errors
    ///
    /// `addr` not a version-0 P2SH address: `Invalid` "address must be
    /// P2SH". The redeem-script lookup's, when an output is listed. The
    /// store's.
    pub fn unspent_multisig_credits_for_address(
        &self,
        r: &mut (impl TxRead + ?Sized),
        addr: &Address,
    ) -> Result<Vec<MultisigCredit>, AnyError> {
        let Address::ScriptHashV0 { hash, .. } = addr else {
            return Err(e!(Kind::Invalid, "address must be P2SH").into());
        };
        let lo = MultisigUnspentKey {
            script_hash: Hash160(*hash),
            outpoint: Outpoint {
                hash: Hash32::default(),
                index: 0,
            },
        };
        let hi = MultisigUnspentKey {
            script_hash: Hash160(*hash),
            outpoint: Outpoint {
                hash: Hash32([0xff; 32]),
                index: u32::MAX,
            },
        };
        let keys = r
            .keys::<MultisigUnspent>(
                std::ops::Bound::Included(&lo),
                std::ops::Bound::Included(&hi),
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
        let mut script: Option<Vec<u8>> = None;
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let Some(row) = r.get::<MultisigOuts>(&k.outpoint).map_err(store_err)? else {
                return Err(e!(Kind::Io, "missing multisig credit").into());
            };
            if row.script_hash.0 != *hash {
                continue;
            }
            let ms_script = match &script {
                Some(s) => s.clone(),
                None => {
                    let s = redeem_script(r, hash)?;
                    script = Some(s.clone());
                    s
                }
            };
            out.push(MultisigCredit {
                outpoint: OutPoint {
                    hash: dcroxide_chainhash::Hash(k.outpoint.hash.0),
                    index: k.outpoint.index,
                    tree: i8::from(row.stake_tree),
                },
                script_hash: row.script_hash,
                ms_script,
                m: row.m,
                n: row.n,
                amount: row.amount,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use dcroxide_txscript::stdscript::determine_script_type;

    use super::{p2sh_class, p2sh_script_hash};

    /// The script hash is the push after the first `OP_HASH160`, for the
    /// plain, stake-tagged and treasury forms, and each is classed P2SH.
    #[test]
    fn p2sh_script_hashes_follow_the_tokenizer() {
        let mut plain = vec![0xa9, 0x14];
        plain.extend((1..=20).collect::<Vec<u8>>());
        plain.push(0x87);
        for tag in [
            None,
            Some(0xba),
            Some(0xbb),
            Some(0xbc),
            Some(0xbd),
            Some(0xc3),
        ] {
            let mut s: Vec<u8> = tag.into_iter().collect();
            s.extend_from_slice(&plain);
            let (p2sh, stake) = p2sh_class(determine_script_type(0, &s));
            assert!(p2sh, "{tag:?}");
            assert_eq!(stake, tag.is_some(), "{tag:?}");
            let h = p2sh_script_hash(&s).expect("hash");
            assert_eq!(h.to_vec(), (1..=20).collect::<Vec<u8>>());
        }
        // P2PKH is not P2SH; a short script has no hash.
        let mut p2pkh = vec![0x76, 0xa9, 0x14];
        p2pkh.extend([7; 20]);
        p2pkh.extend([0x88, 0xac]);
        assert!(!p2sh_class(determine_script_type(0, &p2pkh)).0);
        assert!(p2sh_class(determine_script_type(1, &plain)) == (false, false));
        assert_eq!(p2sh_script_hash(&[0x87; 20]), None);
    }
}
