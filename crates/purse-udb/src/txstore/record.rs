// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Transaction records (`TxRecord`, `NewTxRecord`, `NewTxRecordFromMsgTx`,
//! `wallet/udb/txmined.go:95-141`) and the block descriptions the queries
//! return.

use std::sync::OnceLock;

use dcroxide_chainhash::Hash;
use dcroxide_stake::TxType;
use dcroxide_wire::{MsgTx, OutPoint};
use purse_errors::{AnyError, Kind, e};
use purse_store::{Hash32, HeaderBytes, Outpoint};

/// Go's zero `time.Time` as Unix seconds: the time of an unmined
/// transaction's block (`TxDetails.Block.Time`), and the birthday time
/// dcrwallet's upgrade 26 writes (QK-0114).
pub const ZERO_TIME: i64 = -62_135_596_800;

/// A transaction the store records: dcrwallet's `TxRecord`.
///
/// The serialized bytes are kept exactly as the caller gave them, bytes
/// after the transaction's end included, as Go keeps the caller's slice;
/// the hash and the stake type are computed once, when the record is made,
/// and cannot be changed afterwards (Go's fields are writable, QK-0058).
///
/// A record read back from the store keeps no bytes, as Go's
/// `readRawTxRecord` leaves `SerializedTx` nil: [`TxRecord::serialized`]
/// serializes its transaction on the first call and keeps the result, so
/// the readers that never ask (every query but the caller's own) pay no
/// serialization. Equality compares the serialized bytes, filled or not.
#[derive(Clone)]
pub struct TxRecord {
    msg_tx: MsgTx,
    hash: Hash32,
    received: i64,
    serialized: OnceLock<Vec<u8>>,
    tx_type: TxType,
    unpublished: bool,
}

impl PartialEq for TxRecord {
    fn eq(&self, other: &TxRecord) -> bool {
        self.msg_tx == other.msg_tx
            && self.hash == other.hash
            && self.received == other.received
            && self.tx_type == other.tx_type
            && self.unpublished == other.unpublished
            && self.serialized() == other.serialized()
    }
}

impl Eq for TxRecord {}

impl std::fmt::Debug for TxRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TxRecord")
            .field("msg_tx", &self.msg_tx)
            .field("hash", &self.hash)
            .field("received", &self.received)
            .field("serialized", &self.serialized())
            .field("tx_type", &self.tx_type)
            .field("unpublished", &self.unpublished)
            .finish()
    }
}

impl TxRecord {
    /// `NewTxRecord`: a record of the serialized transaction `bytes`,
    /// received at `received` (Unix seconds).
    ///
    /// # Errors
    ///
    /// The bytes do not begin with a transaction: the wire decoder's error,
    /// unwrapped (kind `Other`), as Go returns `MsgTx.Deserialize`'s.
    pub fn new(bytes: &[u8], received: i64) -> Result<TxRecord, AnyError> {
        let (msg_tx, _) = MsgTx::from_bytes(bytes).map_err(AnyError::foreign)?;
        Ok(TxRecord::with_bytes(msg_tx, bytes.to_vec(), received))
    }

    /// `NewTxRecordFromMsgTx`: a record of `msg_tx`, serialized as its
    /// serialization type says.
    ///
    /// # Errors
    ///
    /// An input with a negative tree, which Go's `MsgTx.Serialize` refuses:
    /// the wire encoder's error (kind `Other`).
    pub fn from_msg_tx(msg_tx: MsgTx, received: i64) -> Result<TxRecord, AnyError> {
        let mut bytes = Vec::with_capacity(msg_tx.serialize_size());
        msg_tx.encode_into(&mut bytes).map_err(AnyError::foreign)?;
        Ok(TxRecord::with_bytes(msg_tx, bytes, received))
    }

    fn with_bytes(msg_tx: MsgTx, serialized: Vec<u8>, received: i64) -> TxRecord {
        let hash = Hash32(*msg_tx.tx_hash().as_bytes());
        let tx_type = dcroxide_stake::determine_tx_type(&msg_tx);
        TxRecord {
            msg_tx,
            hash,
            received,
            serialized: OnceLock::from(serialized),
            tx_type,
            unpublished: false,
        }
    }

    /// A record read back from a stored value: Go's `readRawTxRecord`,
    /// which decodes the transaction, computes its type and keeps no
    /// serialized bytes (`SerializedTx` is nil). The bytes are made only
    /// if [`TxRecord::serialized`] is called.
    pub(crate) fn stored(hash: Hash32, tx: &[u8], received: i64) -> Result<TxRecord, AnyError> {
        let (msg_tx, _) = MsgTx::from_bytes(tx).map_err(|err| io(AnyError::foreign(err)))?;
        let tx_type = dcroxide_stake::determine_tx_type(&msg_tx);
        Ok(TxRecord {
            msg_tx,
            hash,
            received,
            serialized: OnceLock::new(),
            tx_type,
            unpublished: false,
        })
    }

    /// The decoded transaction, the record consumed (no copy).
    pub(crate) fn into_msg_tx(self) -> MsgTx {
        self.msg_tx
    }

    /// Marks the record unpublished (Go's `TxRecord.Unpublished`), as a
    /// caller does before `InsertMemPoolTx` for a transaction it has not
    /// relayed.
    #[must_use]
    pub fn with_unpublished(mut self, unpublished: bool) -> TxRecord {
        self.unpublished = unpublished;
        self
    }

    /// The decoded transaction.
    pub fn msg_tx(&self) -> &MsgTx {
        &self.msg_tx
    }

    /// The transaction hash, in internal byte order.
    pub fn hash(&self) -> &Hash32 {
        &self.hash
    }

    /// When the transaction was received, in Unix seconds.
    pub fn received(&self) -> i64 {
        self.received
    }

    /// The serialized transaction: the caller's bytes for a record made by
    /// [`TxRecord::new`] or [`TxRecord::from_msg_tx`], and, for a record a
    /// query read back, the decoded transaction serialized again, on the
    /// first call (Go's record read back has nil bytes).
    pub fn serialized(&self) -> &[u8] {
        self.serialized.get_or_init(|| self.msg_tx.serialize())
    }

    /// The stake type (`stake.DetermineTxType`).
    pub fn tx_type(&self) -> TxType {
        self.tx_type
    }

    /// Whether the record is marked unpublished.
    pub fn unpublished(&self) -> bool {
        self.unpublished
    }
}

/// A block's hash and height (dcrwallet's `Block`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// The block hash, in internal byte order.
    pub hash: Hash32,
    /// The block height; -1 for an unmined transaction's block.
    pub height: i32,
}

/// A block and the header fields dcrwallet keeps with it (`BlockMeta`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockMeta {
    /// The block.
    pub block: Block,
    /// The block's timestamp, in Unix seconds.
    pub time: i64,
    /// The header's vote bits.
    pub vote_bits: u16,
}

impl BlockMeta {
    /// Go's `blockMetaFromHeader` (`txmined.go:595-604`).
    pub fn from_header(hash: &Hash32, header: &HeaderBytes) -> BlockMeta {
        BlockMeta {
            block: Block {
                hash: *hash,
                height: header_height(header),
            },
            time: header_time(header),
            vote_bits: header_vote_bits(header),
        }
    }

    /// The block of an unmined transaction's details: no hash, height -1,
    /// Go's zero time.
    pub fn unmined() -> BlockMeta {
        BlockMeta {
            block: Block {
                hash: Hash32::default(),
                height: -1,
            },
            time: ZERO_TIME,
            vote_bits: 0,
        }
    }
}

/// A credit as `TxDetails` lists it (`CreditRecord`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreditRecord {
    /// The output index.
    pub index: u32,
    /// The output's amount, in atoms.
    pub amount: i64,
    /// Spent by a mined transaction, or by an unmined one.
    pub spent: bool,
    /// Pays an internal-branch address.
    pub change: bool,
    /// The stake opcode tag (`OP_NOP10` for none); 0 for an unmined
    /// credit, whose iterator Go leaves it unset (QK-0083).
    pub opcode: u8,
    /// From a coinbase; false for an unmined credit (QK-0083).
    pub coinbase: bool,
    /// The transaction has an expiry.
    pub expiry: bool,
}

/// A debit as `TxDetails` lists it (`DebitRecord`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebitRecord {
    /// The input index.
    pub index: u32,
    /// The amount of the credit it spends, in atoms.
    pub amount: i64,
}

/// Details of a recorded transaction (`TxDetails`): the record, its block,
/// its credits and its debits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxDetails {
    /// The record (never marked unpublished: Go's readers leave the flag
    /// unset).
    pub record: TxRecord,
    /// Its block: the mined block (with its time, and vote bits 0, as Go
    /// leaves them), or [`BlockMeta::unmined`].
    pub block: BlockMeta,
    /// The credits, in output order.
    pub credits: Vec<CreditRecord>,
    /// The debits, in input order: for a mined record those recorded, for
    /// an unmined one each input spending an unspent output or an unmined
    /// credit (Go records no debit for an unmined transaction).
    pub debits: Vec<DebitRecord>,
}

impl TxDetails {
    /// `TxDetails.Height`: the block height, -1 when unmined.
    pub fn height(&self) -> i32 {
        self.block.block.height
    }
}

/// An output the UTXO queries give (dcrwallet's `Credit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    /// The output, with the tree Go derives from its stake tag (stake for
    /// any tag but `opNonstake`).
    pub outpoint: OutPoint,
    /// Its block: the mined block's hash and height, with Go's zero time
    /// and vote bits 0 (Go leaves them unset); for an unmined output,
    /// [`BlockMeta::unmined`].
    pub block: BlockMeta,
    /// The amount, in atoms.
    pub amount: i64,
    /// The output's script.
    pub pk_script: Vec<u8>,
    /// When the wallet first saw the transaction (Unix seconds).
    pub received: i64,
    /// From a coinbase; false for an unmined output, whose credit Go reads
    /// no coinbase flag from.
    pub from_coinbase: bool,
    /// The transaction has an expiry.
    pub has_expiry: bool,
}

/// A ticket the store knows, as `TicketIterator.Next` yields it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// The ticket's record.
    pub record: TxRecord,
    /// Its block (height -1 and no hash when unmined).
    pub block: Block,
    /// The hash of the unmined transaction spending output 0, or zero.
    pub spender: Hash32,
}

/// `TicketDetails`: a ticket and the transaction spending it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketDetails {
    /// The ticket.
    pub ticket: TxDetails,
    /// The spender's details, when one is recorded.
    pub spender: Option<TxDetails>,
}

/// The height a header states, as Go's `int32` (`extractBlockHeaderHeight`).
#[allow(
    clippy::cast_possible_wrap,
    reason = "Go reads the field as an int32 (txmined.go:568-571)"
)]
pub(crate) fn header_height(h: &HeaderBytes) -> i32 {
    h.height() as i32
}

/// The header's timestamp (`extractBlockHeaderUnixTime`).
pub(crate) fn header_time(h: &HeaderBytes) -> i64 {
    i64::from(u32::from_le_bytes([h.0[136], h.0[137], h.0[138], h.0[139]]))
}

/// The header's vote bits (`extractBlockHeaderVoteBits`).
pub(crate) fn header_vote_bits(h: &HeaderBytes) -> u16 {
    u16::from_le_bytes([h.0[100], h.0[101]])
}

/// The header's merkle root (`extractBlockHeaderMerkleRoot`).
pub(crate) fn header_merkle_root(h: &HeaderBytes) -> [u8; 32] {
    let mut m = [0u8; 32];
    m.copy_from_slice(&h.0[36..68]);
    m
}

/// A hash in dcrd's display form (`chainhash.Hash.String`).
pub(crate) fn display(h: &Hash32) -> String {
    Hash(h.0).to_string()
}

/// The canonical outpoint of a wire outpoint (`canonicalOutPoint`).
pub(crate) fn canonical(op: &OutPoint) -> Outpoint {
    Outpoint {
        hash: Hash32(op.hash.0),
        index: op.index,
    }
}

/// `errors.E(errors.IO, err)`.
pub(crate) fn io(err: AnyError) -> AnyError {
    e!(Kind::Io, err).into()
}

#[cfg(test)]
mod tests {
    use dcroxide_wire::{MsgTx, TxOut};

    use super::TxRecord;

    /// A record read back keeps no bytes until they are asked for, as Go's
    /// `readRawTxRecord` keeps none, and then has the transaction's
    /// serialization.
    #[test]
    fn a_stored_record_serializes_only_when_asked() {
        let m = MsgTx {
            tx_out: vec![TxOut {
                value: 7,
                version: 0,
                pk_script: vec![0x6a],
            }],
            ..MsgTx::default()
        };
        let made = TxRecord::from_msg_tx(m, 5).expect("record");
        let stored = TxRecord::stored(*made.hash(), made.serialized(), 5).expect("stored");
        assert!(stored.serialized.get().is_none());
        assert_eq!(stored.tx_type(), made.tx_type());
        assert_eq!(stored.serialized(), made.serialized());
        assert!(stored.serialized.get().is_some());
        let again = TxRecord::stored(*made.hash(), made.serialized(), 5).expect("stored");
        assert_eq!(again, made, "equality compares the bytes, filled or not");
    }
}
