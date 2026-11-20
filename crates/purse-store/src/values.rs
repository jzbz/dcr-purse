// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Value types: what each table holds, field by field.
//!
//! Integers are big-endian. Amounts are signed atoms (`i64`), as dcrwallet's
//! `dcrutil.Amount`. Every layout is fixed by the encoding written next to
//! the type; a change to one is a schema version step.

use crate::codec::{ArrayWriter, Codec, DecodeError, Reader};
use crate::keys::{CreditKey, Hash32};

/// What an unspent output counts as before a balance query applies its
/// depth rules (ADR-0003 Decision item 4). The class is fixed when the
/// output is written; `balances` and `balance_heights` sum by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum BalanceClass {
    /// Mined, non-stake (or treasury-generated) tag, coinbase bit clear.
    MinedRegular = 0,
    /// Mined, non-stake tag, coinbase bit set.
    MinedCoinbase = 1,
    /// Mined vote or revocation output.
    MinedStakeGen = 2,
    /// Mined ticket change.
    MinedTicketChange = 3,
    /// Ticket submission output, mined or unmined.
    VotingAuthority = 4,
    /// Unmined non-stake, coinbase bit clear, published.
    UnminedRegularPublished = 5,
    /// Unmined non-stake, coinbase bit clear, unpublished.
    UnminedRegularUnpublished = 6,
    /// Unmined non-stake, coinbase bit set, published.
    UnminedCoinbasePublished = 7,
    /// Unmined non-stake, coinbase bit set, unpublished.
    UnminedCoinbaseUnpublished = 8,
    /// Unmined vote or revocation output.
    UnminedStakeGen = 9,
    /// Unmined ticket change.
    UnminedTicketChange = 10,
    /// Unmined treasury-generated output (counts toward no category).
    UnminedTreasuryGen = 11,
    /// A wallet ticket commitment not spent by an unmined vote.
    Locked = 12,
}

impl BalanceClass {
    /// Every class, in code order.
    pub const ALL: [BalanceClass; 13] = [
        BalanceClass::MinedRegular,
        BalanceClass::MinedCoinbase,
        BalanceClass::MinedStakeGen,
        BalanceClass::MinedTicketChange,
        BalanceClass::VotingAuthority,
        BalanceClass::UnminedRegularPublished,
        BalanceClass::UnminedRegularUnpublished,
        BalanceClass::UnminedCoinbasePublished,
        BalanceClass::UnminedCoinbaseUnpublished,
        BalanceClass::UnminedStakeGen,
        BalanceClass::UnminedTicketChange,
        BalanceClass::UnminedTreasuryGen,
        BalanceClass::Locked,
    ];

    /// The class with code `code`.
    pub fn from_u8(code: u8) -> Option<BalanceClass> {
        BalanceClass::ALL.get(usize::from(code)).copied()
    }

    /// Whether the class keeps per-height sums in `balance_heights`: the
    /// mined classes, whose category depends on the depth a query asks for.
    pub fn is_windowed(self) -> bool {
        self <= BalanceClass::MinedTicketChange
    }
}

/// The opcode tag of an output's script, as a credit records it: the stake
/// opcode that prefixes the script, or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OutputTag {
    /// No stake tag (`OP_NOP10`, dcrwallet's `opNonstake`).
    #[default]
    NonStake = 0,
    /// `OP_SSTX`: ticket submission.
    Ticket = 1,
    /// `OP_SSGEN`: vote.
    Vote = 2,
    /// `OP_SSRTX`: revocation.
    Revocation = 3,
    /// `OP_SSTXCHANGE`: ticket change.
    TicketChange = 4,
    /// `OP_TGEN`: treasury-generated.
    TreasuryGen = 7,
}

impl OutputTag {
    fn from_code(code: u8) -> Option<OutputTag> {
        Some(match code {
            0 => OutputTag::NonStake,
            1 => OutputTag::Ticket,
            2 => OutputTag::Vote,
            3 => OutputTag::Revocation,
            4 => OutputTag::TicketChange,
            7 => OutputTag::TreasuryGen,
            _ => return None,
        })
    }
}

/// The flags of a credit, unspent output or unmined credit, in one byte:
/// bit 0 change, bit 1 spent (credits only, derived from the spender),
/// bits 2–4 the [`OutputTag`] code, bit 5 coinbase, bit 6 expiry; bit 7 is
/// zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CreditFlags {
    /// The output pays an internal-branch (change) address.
    pub change: bool,
    /// The output's script tag.
    pub tag: OutputTag,
    /// The output belongs to a coinbase transaction (or a treasury base).
    pub coinbase: bool,
    /// The transaction has a nonzero expiry.
    pub expiry: bool,
}

const F_CHANGE: u8 = 1;
const F_SPENT: u8 = 1 << 1;
const F_COINBASE: u8 = 1 << 5;
const F_EXPIRY: u8 = 1 << 6;
const F_RESERVED: u8 = 1 << 7;

impl CreditFlags {
    fn byte(self, spent: bool) -> u8 {
        let mut b = (self.tag as u8) << 2;
        if self.change {
            b |= F_CHANGE;
        }
        if spent {
            b |= F_SPENT;
        }
        if self.coinbase {
            b |= F_COINBASE;
        }
        if self.expiry {
            b |= F_EXPIRY;
        }
        b
    }

    /// Splits a flags byte into flags and the spent bit.
    fn parse(r: &Reader<'_>, b: u8) -> Result<(CreditFlags, bool), DecodeError> {
        if b & F_RESERVED != 0 {
            return Err(r.field("reserved flag bit set"));
        }
        let tag =
            OutputTag::from_code((b >> 2) & 7).ok_or_else(|| r.field("unknown output tag"))?;
        Ok((
            CreditFlags {
                change: b & F_CHANGE != 0,
                tag,
                coinbase: b & F_COINBASE != 0,
                expiry: b & F_EXPIRY != 0,
            },
            b & F_SPENT != 0,
        ))
    }
}

/// Where an output's script lies in its serialized transaction, and its
/// class. The script itself stays in the transaction record (ADR-0003
/// Decision item 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ScriptLoc {
    /// Offset of the script in the serialized transaction.
    pub offset: u32,
    /// Length of the script.
    pub len: u16,
    /// The script's class, a code the transaction store logic defines
    /// (the spike used 1 for P2PKH, possibly stake-tagged, and 0 for any
    /// other script).
    pub class: u8,
}

/// `accounts` value: an account's type, address counters, name and key
/// material.
///
/// Layout: type u8 ‖ last used external ‖ last used internal ‖ last
/// returned external ‖ last returned internal (u32 BE each, `u32::MAX`:
/// none yet) ‖ name length u32 BE ‖ name ‖ keys (the rest).
///
/// A name longer than `u32::MAX` bytes cannot be stored: its row would
/// exceed redb's value limit and the put fails before anything is written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccountRow {
    /// The account type, a code the address manager logic defines.
    pub account_type: u8,
    /// Last used child per branch (external, internal).
    pub last_used: [u32; 2],
    /// Last returned child per branch (external, internal).
    pub last_returned: [u32; 2],
    /// The account's name.
    pub name: Vec<u8>,
    /// The account's key material as the address manager seals it
    /// (extended public key, encrypted extended private key, per-account
    /// passphrase parameters); opaque to the store.
    pub keys: Vec<u8>,
}

impl Codec for AccountRow {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(
            21usize
                .saturating_add(self.name.len())
                .saturating_add(self.keys.len()),
        );
        v.push(self.account_type);
        for x in self.last_used.iter().chain(&self.last_returned) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        let name_len = u32::try_from(self.name.len()).unwrap_or(u32::MAX);
        v.extend_from_slice(&name_len.to_be_bytes());
        v.extend_from_slice(&self.name);
        v.extend_from_slice(&self.keys);
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("account row", bytes);
        let account_type = r.u8()?;
        let last_used = [r.u32_be()?, r.u32_be()?];
        let last_returned = [r.u32_be()?, r.u32_be()?];
        let name_len = usize::try_from(r.u32_be()?).map_err(|_| r.field("name length"))?;
        let name = r.bytes(name_len)?.to_vec();
        let keys = r.rest().to_vec();
        Ok(AccountRow {
            account_type,
            last_used,
            last_returned,
            name,
            keys,
        })
    }
}

/// `addr_by_hash` value: who owns an address.
///
/// Layout: tag u8, then for tag 0 account u32 BE ‖ branch u8 ‖ index u32 BE,
/// for tag 1 account u32 BE ‖ data (the rest).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AddrRow {
    /// A BIP0044 address at (account, branch, index).
    Derived {
        /// Account number.
        account: u32,
        /// Branch.
        branch: u8,
        /// Child index.
        index: u32,
    },
    /// An imported key or script, which has no derivation path; listed by
    /// `addr_by_acct_sha` under its account (ADR-0003 Decision item 2).
    Imported {
        /// Account number (the imported account, 0x7FFFFFFF, for keys and
        /// scripts imported into it).
        account: u32,
        /// What the address manager keeps for it (public key or script,
        /// sealed private key); opaque to the store.
        data: Vec<u8>,
    },
}

impl AddrRow {
    /// The owning account.
    pub fn account(&self) -> u32 {
        match self {
            AddrRow::Derived { account, .. } | AddrRow::Imported { account, .. } => *account,
        }
    }
}

impl Codec for AddrRow {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        match self {
            AddrRow::Derived {
                account,
                branch,
                index,
            } => {
                let mut v = Vec::with_capacity(10);
                v.push(0);
                v.extend_from_slice(&account.to_be_bytes());
                v.push(*branch);
                v.extend_from_slice(&index.to_be_bytes());
                v
            }
            AddrRow::Imported { account, data } => {
                let mut v = Vec::with_capacity(data.len().saturating_add(5));
                v.push(1);
                v.extend_from_slice(&account.to_be_bytes());
                v.extend_from_slice(data);
                v
            }
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("address row", bytes);
        match r.u8()? {
            0 => {
                let row = AddrRow::Derived {
                    account: r.u32_be()?,
                    branch: r.u8()?,
                    index: r.u32_be()?,
                };
                r.finish()?;
                Ok(row)
            }
            1 => Ok(AddrRow::Imported {
                account: r.u32_be()?,
                data: r.rest().to_vec(),
            }),
            _ => Err(r.field("unknown address kind")),
        }
    }
}

/// `headers` and `headers_detached` value: a serialized 180-byte block
/// header.
///
/// The store reads two fields of it, at their offsets in Decred's wire
/// serialization (all little-endian): the previous block's hash (bytes 4 to
/// 36) and the height (bytes 128 to 132), which dcrwallet reads at the same
/// offsets (`extractBlockHeaderParentHash`, `wallet/udb/txmined.go:545-548`,
/// and `extractBlockHeaderHeight`, `:568-571`). It does not hash the
/// header: a block's hash is the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HeaderBytes(pub [u8; 180]);

impl HeaderBytes {
    /// Offset of the previous block's hash.
    pub const PREV_BLOCK_OFFSET: usize = 4;
    /// Offset of the height, u32 little-endian.
    pub const HEIGHT_OFFSET: usize = 128;

    /// The height the header states.
    pub fn height(&self) -> u32 {
        let h = &self.0;
        u32::from_le_bytes([h[128], h[129], h[130], h[131]])
    }

    /// The hash of the previous block, in internal byte order.
    pub fn prev_block(&self) -> Hash32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(&self.0[4..36]);
        Hash32(p)
    }
}

impl Codec for HeaderBytes {
    type Bytes = [u8; 180];

    fn encode(&self) -> [u8; 180] {
        self.0
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("block header", bytes);
        let h = r.array()?;
        r.finish()?;
        Ok(HeaderBytes(h))
    }
}

/// `blocks` value: a main-chain block's record.
///
/// Layout (47 bytes): hash ‖ time i64 BE ‖ vote bits u16 BE ‖ invalid u8
/// (0 or 1) ‖ wallet transactions u32 BE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockRow {
    /// Block hash.
    pub hash: Hash32,
    /// Header timestamp (Unix seconds).
    pub time: i64,
    /// Vote bits of the header.
    pub vote_bits: u16,
    /// The block's regular tree was disapproved by its child.
    pub invalid: bool,
    /// Number of wallet transactions the block mined.
    pub wallet_txs: u32,
}

impl Codec for BlockRow {
    type Bytes = [u8; 47];

    fn encode(&self) -> [u8; 47] {
        ArrayWriter::new()
            .put(&self.hash.0)
            .put(&self.time.to_be_bytes())
            .put(&self.vote_bits.to_be_bytes())
            .put(&[u8::from(self.invalid)])
            .put(&self.wallet_txs.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("block record", bytes);
        let hash = Hash32(r.array()?);
        let time = r.i64_be()?;
        let vote_bits = r.u16_be()?;
        let invalid = match r.u8()? {
            0 => false,
            1 => true,
            _ => return Err(r.field("invalid flag not 0 or 1")),
        };
        let wallet_txs = r.u32_be()?;
        r.finish()?;
        Ok(BlockRow {
            hash,
            time,
            vote_bits,
            invalid,
            wallet_txs,
        })
    }
}

/// `cfilters` value: a block's version-2 committed filter.
///
/// Layout: key (16) ‖ filter bytes (the rest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CFilterRow {
    /// The filter key (the first 16 bytes of the block's merkle root).
    pub key: [u8; 16],
    /// The serialized filter.
    pub filter: Vec<u8>,
}

impl CFilterRow {
    /// The length of the row's encoding (`None`: past `usize`), which
    /// bbolt's value limit is checked against.
    pub(crate) fn encoded_len(&self) -> Option<usize> {
        self.key.len().checked_add(self.filter.len())
    }
}

impl Codec for CFilterRow {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.filter.len().saturating_add(16));
        v.extend_from_slice(&self.key);
        v.extend_from_slice(&self.filter);
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("cfilter", bytes);
        Ok(CFilterRow {
            key: r.array()?,
            filter: r.rest().to_vec(),
        })
    }
}

/// `tx_mined` value: a mined transaction record.
///
/// Layout: position in block u32 BE ‖ received time i64 BE ‖ serialized
/// transaction (the rest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TxMinedRow {
    /// Position among the block's wallet transactions (the `history` key's).
    pub position: u32,
    /// When the wallet first saw the transaction (Unix seconds).
    pub received: i64,
    /// The serialized transaction.
    pub tx: Vec<u8>,
}

impl Codec for TxMinedRow {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.tx.len().saturating_add(12));
        v.extend_from_slice(&self.position.to_be_bytes());
        v.extend_from_slice(&self.received.to_be_bytes());
        v.extend_from_slice(&self.tx);
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("mined transaction", bytes);
        Ok(TxMinedRow {
            position: r.u32_be()?,
            received: r.i64_be()?,
            tx: r.rest().to_vec(),
        })
    }
}

/// `tx_unmined` value: an unmined transaction record.
///
/// Layout: received time i64 BE ‖ serialized transaction (the rest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TxUnminedRow {
    /// When the wallet first saw the transaction (Unix seconds).
    pub received: i64,
    /// The serialized transaction.
    pub tx: Vec<u8>,
}

impl Codec for TxUnminedRow {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.tx.len().saturating_add(8));
        v.extend_from_slice(&self.received.to_be_bytes());
        v.extend_from_slice(&self.tx);
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("unmined transaction", bytes);
        Ok(TxUnminedRow {
            received: r.i64_be()?,
            tx: r.rest().to_vec(),
        })
    }
}

/// A value of no content: `tickets` rows say only that the key exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Empty;

impl Codec for Empty {
    type Bytes = [u8; 0];

    fn encode(&self) -> [u8; 0] {
        []
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Reader::new("empty value", bytes).finish()?;
        Ok(Empty)
    }
}

/// `unpublished` value: one byte, 1. dcrwallet's `up` rows say only that
/// the transaction has not been published.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct UnpublishedFlag;

impl Codec for UnpublishedFlag {
    type Bytes = [u8; 1];

    fn encode(&self) -> [u8; 1] {
        [1]
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("unpublished flag", bytes);
        if r.u8()? != 1 {
            return Err(r.field("not 1"));
        }
        r.finish()?;
        Ok(UnpublishedFlag)
    }
}

/// The transaction input that spends a credit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Spender {
    /// Spending transaction's hash.
    pub hash: Hash32,
    /// Height of the block that mined it.
    pub height: u32,
    /// Input index.
    pub input: u32,
}

/// `credits` and `credits_invalidated` value: a mined credit, kept after it
/// is spent (ADR-0003 Open point 8).
///
/// Layout (60 bytes): amount i64 BE ‖ flags ‖ account u32 BE ‖ script
/// offset u32 BE ‖ script length u16 BE ‖ script class ‖ spender hash ‖
/// spender height u32 BE ‖ spender input u32 BE. The flags' spent bit is
/// set exactly when there is a spender; otherwise the spender bytes are
/// zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CreditRow {
    /// Amount in atoms.
    pub amount: i64,
    /// Flags (the spent bit is `spender.is_some()`).
    pub flags: CreditFlags,
    /// Owning account.
    pub account: u32,
    /// The output's script.
    pub script: ScriptLoc,
    /// The mined input that spends it, if any.
    pub spender: Option<Spender>,
}

impl Codec for CreditRow {
    type Bytes = [u8; 60];

    fn encode(&self) -> [u8; 60] {
        let sp = self.spender.unwrap_or_default();
        ArrayWriter::new()
            .put(&self.amount.to_be_bytes())
            .put(&[self.flags.byte(self.spender.is_some())])
            .put(&self.account.to_be_bytes())
            .put(&self.script.offset.to_be_bytes())
            .put(&self.script.len.to_be_bytes())
            .put(&[self.script.class])
            .put(&sp.hash.0)
            .put(&sp.height.to_be_bytes())
            .put(&sp.input.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("credit", bytes);
        let amount = r.i64_be()?;
        let fb = r.u8()?;
        let account = r.u32_be()?;
        let script = ScriptLoc {
            offset: r.u32_be()?,
            len: r.u16_be()?,
            class: r.u8()?,
        };
        let sp = Spender {
            hash: Hash32(r.array()?),
            height: r.u32_be()?,
            input: r.u32_be()?,
        };
        r.finish()?;
        let r = Reader::new("credit", bytes);
        let (flags, spent) = CreditFlags::parse(&r, fb)?;
        let spender = if spent {
            Some(sp)
        } else if sp == Spender::default() {
            None
        } else {
            return Err(r.field("spender set on an unspent credit"));
        };
        Ok(CreditRow {
            amount,
            flags,
            account,
            script,
            spender,
        })
    }
}

/// `unspent` value: what selection and balances need of an unspent mined
/// output, so that they read one row per output (ADR-0003 Decision item 2).
///
/// Layout (25 bytes): height u32 BE ‖ account u32 BE ‖ amount i64 BE ‖
/// flags (spent bit clear) ‖ balance class ‖ script class ‖ script offset
/// u32 BE ‖ script length u16 BE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnspentRow {
    /// Height of the block that mined the output.
    pub height: u32,
    /// Owning account.
    pub account: u32,
    /// Amount in atoms.
    pub amount: i64,
    /// Flags.
    pub flags: CreditFlags,
    /// Balance class.
    pub class: BalanceClass,
    /// The output's script.
    pub script: ScriptLoc,
}

impl Codec for UnspentRow {
    type Bytes = [u8; 25];

    fn encode(&self) -> [u8; 25] {
        ArrayWriter::new()
            .put(&self.height.to_be_bytes())
            .put(&self.account.to_be_bytes())
            .put(&self.amount.to_be_bytes())
            .put(&[self.flags.byte(false), self.class as u8, self.script.class])
            .put(&self.script.offset.to_be_bytes())
            .put(&self.script.len.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("unspent output", bytes);
        let height = r.u32_be()?;
        let account = r.u32_be()?;
        let amount = r.i64_be()?;
        let fb = r.u8()?;
        let class = BalanceClass::from_u8(r.u8()?).ok_or_else(|| r.field("unknown class"))?;
        let class_code = r.u8()?;
        let offset = r.u32_be()?;
        let len = r.u16_be()?;
        let (flags, spent) = CreditFlags::parse(&r, fb)?;
        if spent {
            return Err(r.field("spent flag set"));
        }
        r.finish()?;
        Ok(UnspentRow {
            height,
            account,
            amount,
            flags,
            class,
            script: ScriptLoc {
                offset,
                len,
                class: class_code,
            },
        })
    }
}

/// `unmined_credits` value: an output of an unmined transaction paying the
/// wallet (dcrwallet's `mc`).
///
/// Layout (21 bytes): amount i64 BE ‖ flags (spent bit clear) ‖ account
/// u32 BE ‖ balance class ‖ script class ‖ script offset u32 BE ‖ script
/// length u16 BE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnminedCreditRow {
    /// Amount in atoms.
    pub amount: i64,
    /// Flags.
    pub flags: CreditFlags,
    /// Owning account.
    pub account: u32,
    /// Balance class.
    pub class: BalanceClass,
    /// The output's script.
    pub script: ScriptLoc,
}

impl Codec for UnminedCreditRow {
    type Bytes = [u8; 21];

    fn encode(&self) -> [u8; 21] {
        ArrayWriter::new()
            .put(&self.amount.to_be_bytes())
            .put(&[self.flags.byte(false)])
            .put(&self.account.to_be_bytes())
            .put(&[self.class as u8, self.script.class])
            .put(&self.script.offset.to_be_bytes())
            .put(&self.script.len.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("unmined credit", bytes);
        let amount = r.i64_be()?;
        let fb = r.u8()?;
        let account = r.u32_be()?;
        let class = BalanceClass::from_u8(r.u8()?).ok_or_else(|| r.field("unknown class"))?;
        let class_code = r.u8()?;
        let offset = r.u32_be()?;
        let len = r.u16_be()?;
        let (flags, spent) = CreditFlags::parse(&r, fb)?;
        if spent {
            return Err(r.field("spent flag set"));
        }
        r.finish()?;
        Ok(UnminedCreditRow {
            amount,
            flags,
            account,
            class,
            script: ScriptLoc {
                offset,
                len,
                class: class_code,
            },
        })
    }
}

/// `debits` and `debits_invalidated` value: what a wallet transaction's
/// input spent.
///
/// Layout (48 bytes): amount i64 BE ‖ the spent credit's key (40).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DebitRow {
    /// Amount of the spent credit.
    pub amount: i64,
    /// The spent credit.
    pub credit: CreditKey,
}

impl Codec for DebitRow {
    type Bytes = [u8; 48];

    fn encode(&self) -> [u8; 48] {
        ArrayWriter::new()
            .put(&self.amount.to_be_bytes())
            .put(&self.credit.encode())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("debit", bytes);
        let amount = r.i64_be()?;
        let credit = CreditKey {
            hash: Hash32(r.array()?),
            height: r.u32_be()?,
            index: r.u32_be()?,
        };
        r.finish()?;
        Ok(DebitRow { amount, credit })
    }
}

/// `commitments` value: a ticket commitment paying the wallet (dcrwallet's
/// `cmt`).
///
/// Layout (12 bytes): amount i64 BE ‖ account u32 BE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CommitmentRow {
    /// Committed amount in atoms.
    pub amount: i64,
    /// Account of the commitment address.
    pub account: u32,
}

impl Codec for CommitmentRow {
    type Bytes = [u8; 12];

    fn encode(&self) -> [u8; 12] {
        ArrayWriter::new()
            .put(&self.amount.to_be_bytes())
            .put(&self.account.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("commitment", bytes);
        let row = CommitmentRow {
            amount: r.i64_be()?,
            account: r.u32_be()?,
        };
        r.finish()?;
        Ok(row)
    }
}

/// `commitments_unspent` value: one flags byte, bit 0 set when an unmined
/// vote or revocation spends the commitment (dcrwallet's `cmu`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CommitmentUnspentRow {
    /// Spent by an unmined vote or revocation.
    pub unmined_spent: bool,
}

impl Codec for CommitmentUnspentRow {
    type Bytes = [u8; 1];

    fn encode(&self) -> [u8; 1] {
        [u8::from(self.unmined_spent)]
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("unspent commitment", bytes);
        let b = r.u8()?;
        r.finish()?;
        match b {
            0 => Ok(CommitmentUnspentRow {
                unmined_spent: false,
            }),
            1 => Ok(CommitmentUnspentRow {
                unmined_spent: true,
            }),
            _ => Err(DecodeError::field(
                "unspent commitment",
                "unknown flag bits",
            )),
        }
    }
}

/// `balances` and `balance_heights` value: a running sum and count.
///
/// Layout (16 bytes): sum i64 BE ‖ count i64 BE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BalanceRow {
    /// Sum of the amounts, in atoms.
    pub sum: i64,
    /// Number of outputs summed.
    pub count: i64,
}

impl Codec for BalanceRow {
    type Bytes = [u8; 16];

    fn encode(&self) -> [u8; 16] {
        ArrayWriter::new()
            .put(&self.sum.to_be_bytes())
            .put(&self.count.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("balance", bytes);
        let row = BalanceRow {
            sum: r.i64_be()?,
            count: r.i64_be()?,
        };
        r.finish()?;
        Ok(row)
    }
}

/// The `tip` meta row: the main chain tip.
///
/// Layout (36 bytes): height u32 BE ‖ block hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tip {
    /// Tip height.
    pub height: u32,
    /// Tip block hash.
    pub hash: Hash32,
}

impl Codec for Tip {
    type Bytes = [u8; 36];

    fn encode(&self) -> [u8; 36] {
        ArrayWriter::new()
            .put(&self.height.to_be_bytes())
            .put(&self.hash.0)
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("tip", bytes);
        let tip = Tip {
            height: r.u32_be()?,
            hash: Hash32(r.array()?),
        };
        r.finish()?;
        Ok(tip)
    }
}

/// A flag: one byte, 0 or 1. The value of the `watch_only` wallet row and
/// the `have_cfilters` meta row.
impl Codec for bool {
    type Bytes = [u8; 1];

    fn encode(&self) -> [u8; 1] {
        [u8::from(*self)]
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("flag", bytes);
        let b = r.u8()?;
        r.finish()?;
        match b {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::field("flag", "not 0 or 1")),
        }
    }
}

/// The `birth` meta row: the wallet's birthday, as dcrwallet's
/// `BirthdayState` (`wallet/udb/txmined.go:323-333`).
///
/// Layout (45 bytes, dcrwallet's `birthstate` layout): options u8 (bit 0:
/// set from the height, bit 1: set from the time, no other bit) ‖ block
/// hash ‖ height u32 BE ‖ time i64 BE (Unix seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BirthState {
    /// The birthday block is still to be found from `height`.
    pub set_from_height: bool,
    /// The birthday block is still to be found from `time`.
    pub set_from_time: bool,
    /// The birthday block's hash, once found.
    pub hash: Hash32,
    /// The birthday block's height.
    pub height: u32,
    /// The birthday (Unix seconds).
    pub time: i64,
}

impl Codec for BirthState {
    type Bytes = [u8; 45];

    fn encode(&self) -> [u8; 45] {
        let options = u8::from(self.set_from_height) | if self.set_from_time { 0b10 } else { 0 };
        ArrayWriter::new()
            .put(&[options])
            .put(&self.hash.0)
            .put(&self.height.to_be_bytes())
            .put(&self.time.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("birth state", bytes);
        let options = r.u8()?;
        if options & !0b11 != 0 {
            return Err(r.field("unknown option bits"));
        }
        let state = BirthState {
            set_from_height: options & 1 != 0,
            set_from_time: options & 0b10 != 0,
            hash: Hash32(r.array()?),
            height: r.u32_be()?,
            time: r.i64_be()?,
        };
        r.finish()?;
        Ok(state)
    }
}

/// The `master_pub` and `master_priv` wallet rows: the parameters of a
/// master key derived from a passphrase with scrypt (dcrwallet's `mpub` and
/// `mpriv`, snacl's `Marshal`, `wallet/udb/addressdb.go:318-341`).
///
/// Layout (88 bytes): salt (32) ‖ SHA-256 digest of the derived key (32) ‖
/// N u64 BE ‖ R u64 BE ‖ P u64 BE. (snacl's own encoding has the three
/// parameters little-endian; this layout is the store's.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MasterKeyParams {
    /// The scrypt salt.
    pub salt: [u8; 32],
    /// SHA-256 of the derived key, which checks a passphrase.
    pub digest: [u8; 32],
    /// scrypt's N.
    pub n: u64,
    /// scrypt's r.
    pub r: u64,
    /// scrypt's p.
    pub p: u64,
}

impl Codec for MasterKeyParams {
    type Bytes = [u8; 88];

    fn encode(&self) -> [u8; 88] {
        ArrayWriter::new()
            .put(&self.salt)
            .put(&self.digest)
            .put(&self.n.to_be_bytes())
            .put(&self.r.to_be_bytes())
            .put(&self.p.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("master key parameters", bytes);
        let params = MasterKeyParams {
            salt: r.array()?,
            digest: r.array()?,
            n: r.u64_be()?,
            r: r.u64_be()?,
            p: r.u64_be()?,
        };
        r.finish()?;
        Ok(params)
    }
}

/// The `crypto_pub` and `crypto_priv` wallet rows: a crypto key sealed by
/// its master key (dcrwallet's `cpub` and `cpriv`, snacl boxes,
/// `wallet/udb/addressdb.go:443-466`), opaque to the store.
///
/// Layout: the sealed bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SealedKey(pub Vec<u8>);

impl Codec for SealedKey {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(SealedKey(bytes.to_vec()))
    }
}

/// The `cointype_legacy` and `cointype_slip0044` wallet rows: a coin type's
/// extended keys, each sealed by its crypto key and either possibly absent
/// (dcrwallet's `ctpub`/`ctpriv` and `ctpub-slip0044`/`ctpriv-slip0044`,
/// `wallet/udb/addressdb.go:367-441`).
///
/// Layout: presence u8 (bit 0: public key, bit 1: private key, no other
/// bit) ‖ when public: its length u32 BE ‖ public ‖ when private: private
/// (the rest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CoinTypeKeys {
    /// The sealed extended public key.
    pub enc_pub: Option<Vec<u8>>,
    /// The sealed extended private key (absent in a watching-only wallet).
    pub enc_priv: Option<Vec<u8>>,
}

impl Codec for CoinTypeKeys {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let pub_len = self.enc_pub.as_ref().map_or(0, Vec::len);
        let priv_len = self.enc_priv.as_ref().map_or(0, Vec::len);
        let mut v = Vec::with_capacity(5usize.saturating_add(pub_len).saturating_add(priv_len));
        v.push(u8::from(self.enc_pub.is_some()) | if self.enc_priv.is_some() { 0b10 } else { 0 });
        if let Some(p) = &self.enc_pub {
            let len = u32::try_from(p.len()).unwrap_or(u32::MAX);
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(p);
        }
        if let Some(p) = &self.enc_priv {
            v.extend_from_slice(p);
        }
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("coin type keys", bytes);
        let present = r.u8()?;
        if present & !0b11 != 0 {
            return Err(r.field("unknown presence bits"));
        }
        let enc_pub = if present & 1 != 0 {
            let len = usize::try_from(r.u32_be()?).map_err(|_| r.field("public key length"))?;
            Some(r.bytes(len)?.to_vec())
        } else {
            None
        };
        let enc_priv = if present & 0b10 != 0 {
            Some(r.rest().to_vec())
        } else {
            r.finish()?;
            None
        };
        Ok(CoinTypeKeys { enc_pub, enc_priv })
    }
}

/// The `slip0044_acct0` wallet row: account 0 under the SLIP0044 coin type,
/// parked by wallet creation until the wallet moves to that coin type
/// (dcrwallet's `slip0044acct0`, a version-1 account row,
/// `wallet/udb/addressdb.go:555-608`, `addressmanager.go:2961-3007`).
///
/// Layout: next external index u32 BE ‖ next internal index u32 BE ‖
/// public key length u32 BE ‖ sealed public key ‖ private key length u32 BE
/// ‖ sealed private key ‖ name (the rest). A key longer than `u32::MAX`
/// bytes cannot be stored, as for [`AccountRow`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Slip0044Acct0Row {
    /// The sealed extended public key.
    pub enc_pub: Vec<u8>,
    /// The sealed extended private key.
    pub enc_priv: Vec<u8>,
    /// The version-1 row's next external index, unconverted.
    pub next_external: u32,
    /// The version-1 row's next internal index, unconverted.
    pub next_internal: u32,
    /// The account's name.
    pub name: Vec<u8>,
}

impl Codec for Slip0044Acct0Row {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(
            16usize
                .saturating_add(self.enc_pub.len())
                .saturating_add(self.enc_priv.len())
                .saturating_add(self.name.len()),
        );
        v.extend_from_slice(&self.next_external.to_be_bytes());
        v.extend_from_slice(&self.next_internal.to_be_bytes());
        for k in [&self.enc_pub, &self.enc_priv] {
            let len = u32::try_from(k.len()).unwrap_or(u32::MAX);
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(k);
        }
        v.extend_from_slice(&self.name);
        v
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("slip0044 account 0 row", bytes);
        let next_external = r.u32_be()?;
        let next_internal = r.u32_be()?;
        let pub_len = usize::try_from(r.u32_be()?).map_err(|_| r.field("public key length"))?;
        let enc_pub = r.bytes(pub_len)?.to_vec();
        let priv_len = usize::try_from(r.u32_be()?).map_err(|_| r.field("private key length"))?;
        let enc_priv = r.bytes(priv_len)?.to_vec();
        let name = r.rest().to_vec();
        Ok(Slip0044Acct0Row {
            enc_pub,
            enc_priv,
            next_external,
            next_internal,
            name,
        })
    }
}

/// A raw value: the `meta` and `wallet` tables' rows, whose types differ
/// per row and are read through the transactions' typed accessors.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RawValue(pub Vec<u8>);

impl Codec for RawValue {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(RawValue(bytes.to_vec()))
    }
}
