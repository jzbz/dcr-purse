//! The prototype store: dcr-purse's own wallet schema on redb, for the
//! transaction store's and address manager's hot paths.
//!
//! Every table maps raw bytes to raw bytes, so key order is byte order, as
//! in bbolt, and every encoding is spelled out here. Integers in keys are
//! big-endian unless the order a client observes is little-endian
//! (accounts). The schema, table by table, with the dcrwallet access path
//! each one serves, is in the module-level tables below and in ADR-0003.
//!
//! What the prototype covers: accounts and their counters, addresses by
//! HASH160, by (account, branch, index) and in dcrwallet's per-account
//! order, blocks and headers, mined and unmined transactions, credits,
//! debits, the unspent index in dcrwallet's key order, unmined credits and
//! inputs, tickets and commitments, incrementally maintained balances, the
//! history indexes, and the three chain operations that move rows between
//! those tables: stake invalidation of the parent block (`stakeInvalidate`),
//! its reversal (`stakeValidate`) and `Rollback`. What it does not:
//! double-spend removal (`removeDoubleSpends`, `RemoveUnconfirmed`),
//! multisig, imported keys and scripts, revocations, `SetPublished`, and a
//! regular transaction of a disapproved block spending a credit of the same
//! block; these fail loudly where the workload could reach them.
//!
//! # Keys after stake invalidation
//!
//! dcrwallet keys mined transaction records, credits and debits by
//! (transaction hash, block), never by hash alone (spec 03), because one
//! hash can have two mined records at once: a regular transaction of a
//! block its child disapproves keeps its record under that block, with its
//! credits and debits moved to the invalidated buckets `ic` and `id`
//! (`stakeInvalidate`, wallet/udb/txmined.go:767), and the same transaction
//! mined again in a later block gets a second record there
//! (`InsertMinedTx`, txmined.go:1213). `Rollback` deletes the records of
//! detached blocks (txmined.go:1874), so at any moment every record belongs
//! to a main-chain block and (hash, height) identifies it. The prototype
//! therefore keys `tx_mined`, `credits`, `debits` and their invalidated
//! counterparts by (hash, height); `unspent` and `unmined_credits` stay
//! keyed by outpoint, in dcrwallet's key bytes, and can hold the same
//! outpoint at once (see [`W::stake_validate`]).

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Bound;
use std::path::Path;

use dcroxide_wire::MsgTx;
use redb::{
    Database, Durability, ReadOnlyTable, ReadTransaction, ReadableDatabase as _, ReadableTable,
    ReadableTableMetadata as _, Table, TableDefinition, TableHandle as _, WriteTransaction,
};
use sha2::{Digest as _, Sha256};

use crate::chain::{
    self, OP_NONSTAKE, OP_SSGEN, OP_SSRTX, OP_SSTX, OP_SSTXCHANGE, OP_TGEN, ScriptKind, TxType,
};

/// Key and value type of every table.
pub type K = &'static [u8];
type Def = TableDefinition<'static, K, K>;

macro_rules! tables {
    ($($(#[$m:meta])* $name:ident = $s:literal;)*) => {
        $($(#[$m])* pub const $name: Def = TableDefinition::new($s);)*
        /// Every table, in declaration order: whole-store copies and the
        /// digest walk this list, so a table added above cannot be missed.
        pub const ALL_TABLES: &[Def] = &[$($name),*];
    };
}

tables! {
    /// `name` -> value: `version` (u32 BE), `tip` (height u32 BE ‖ hash
    /// 32), `unspent_n` (u64 BE: the number of `UNSPENT` keys, bbolt's
    /// `KeyN`), `last_op` (u64 BE: workload operations applied),
    /// `processed` (hash 32).
    META = "meta";
    /// account u32 **LE** -> last used external ‖ last used internal ‖
    /// last returned external ‖ last returned internal (u32 BE each) ‖
    /// name. Little-endian keys iterate in dcrwallet's account order.
    ACCOUNTS = "accounts";
    /// name -> account u32 BE.
    ACCOUNT_NAMES = "account_names";
    /// HASH160 (20) -> account u32 BE ‖ branch u8 ‖ index u32 BE.
    /// `Manager.Address` / `AddrAccount` by script.
    ADDR_BY_HASH = "addr_by_hash";
    /// account u32 BE ‖ branch u8 ‖ index u32 BE -> HASH160. Address sync
    /// without deriving keys; a BIP0044 account's addresses in derivation
    /// order.
    ADDR_BY_PATH = "addr_by_path";
    /// account u32 **LE** ‖ SHA-256(HASH160) (32) -> HASH160: dcrwallet's
    /// `addracctidx` nested bucket per account (wallet/udb/addressdb.go:907,
    /// keyed by `uint32ToBytes`, little-endian, :269), so a walk of one
    /// account's prefix is `ForEachAccountAddress` order. A client sees that
    /// order for the imported account (`getaddressesbyaccount "imported"`).
    ADDR_BY_ACCT_SHA = "addr_by_acct_sha";
    /// height u32 BE -> serialized header (180). Main chain only; dcrwallet
    /// keeps detached headers too (its `h` bucket is keyed by hash and never
    /// pruned), which the prototype does not.
    HEADERS = "headers";
    /// height u32 BE -> hash 32 ‖ time i64 BE ‖ vote bits u16 BE ‖
    /// invalid u8 ‖ wallet transactions u32 BE. `RangeBlocks`, tip.
    BLOCKS = "blocks";
    /// block hash -> height u32 BE. `GetBlockMetaForHash`.
    BLOCK_BY_HASH = "block_by_hash";
    /// height u32 BE -> filter key (16) ‖ filter bytes.
    CFILTERS = "cfilters";
    /// tx hash ‖ height u32 BE -> position in block u32 BE ‖ received
    /// i64 BE ‖ serialized tx. dcrwallet's `t` (keyed by hash and block).
    /// `TxDetails`, `Tx` read the highest height of a hash.
    TX_MINED = "tx_mined";
    /// tx hash -> received i64 BE ‖ serialized tx. dcrwallet's `m`.
    TX_UNMINED = "tx_unmined";
    /// tx hash -> 0x01. dcrwallet's `up`.
    UNPUBLISHED = "unpublished";
    /// height u32 BE ‖ position u32 BE -> tx hash. `RangeTransactions`
    /// over wallet transactions only, never over empty blocks; the record is
    /// `TX_MINED` at (hash, this height).
    HISTORY = "history";
    /// account u32 BE ‖ height u32 BE ‖ position u32 BE -> tx hash.
    /// Per-account history (no dcrwallet equivalent): the mined records
    /// with a live credit to, or a live debit from, the account.
    ACCT_HISTORY = "acct_history";
    /// credit key (tx hash ‖ height u32 BE ‖ index u32 BE) -> amount i64
    /// BE ‖ flags ‖ account u32 BE ‖ script offset u32 BE ‖ script length
    /// u16 BE ‖ script kind ‖ spender hash 32 ‖ spender height u32 BE ‖
    /// spender input u32 BE. Mined credits, kept after they are spent
    /// (dcrwallet's `c`).
    CREDITS = "credits";
    /// credit key -> credit value: credits of a regular transaction whose
    /// block was disapproved (dcrwallet's `ic`).
    CREDITS_INVALIDATED = "credits_invalidated";
    /// outpoint -> height u32 BE ‖ account u32 BE ‖ amount i64 BE ‖ flags
    /// ‖ balance class ‖ script kind ‖ script offset u32 BE ‖ script
    /// length u16 BE. Exactly the key set of dcrwallet's `u`, in its key
    /// order, with what selection and balances need in the value; the
    /// credit is `CREDITS` at (hash, height, index).
    UNSPENT = "unspent";
    /// outpoint -> amount i64 BE ‖ flags ‖ account u32 BE ‖ balance class ‖
    /// script kind ‖ script offset u32 BE ‖ script length u16 BE.
    /// dcrwallet's `mc`.
    UNMINED_CREDITS = "unmined_credits";
    /// outpoint -> spender tx hash. dcrwallet's `mi`.
    UNMINED_INPUTS = "unmined_inputs";
    /// debit key (tx hash ‖ height u32 BE ‖ input u32 BE) -> amount i64
    /// BE ‖ the spent credit's key (40). Mined debits (dcrwallet's `d`).
    DEBITS = "debits";
    /// debit key -> debit value: debits of a regular transaction whose
    /// block was disapproved (dcrwallet's `id`).
    DEBITS_INVALIDATED = "debits_invalidated";
    /// ticket hash -> empty. dcrwallet's `tix`.
    TICKETS = "tickets";
    /// outpoint -> amount i64 BE ‖ account u32 BE. dcrwallet's `cmt`.
    COMMITMENTS = "commitments";
    /// outpoint -> flags (bit 0: spent by an unmined vote). `cmu`.
    COMMITMENTS_UNSPENT = "commitments_unspent";
    /// account u32 BE ‖ class u8 -> sum i64 BE ‖ count i64 BE.
    BALANCES = "balances";
    /// account u32 BE ‖ class u8 ‖ height u32 BE -> sum ‖ count, for the
    /// classes whose category depends on depth.
    BALANCE_HEIGHTS = "balance_heights";
}

/// Schema version this prototype writes. Version 2 keys mined records,
/// credits and debits by (hash, height) and adds the invalidated tables and
/// `addr_by_acct_sha`.
pub const SCHEMA_VERSION: u32 = 2;

/// Balance classes: what an unspent output counts as, before the depth
/// rules of a query are applied.
pub mod class {
    /// Mined, non-stake (or treasury-gen) tag, coinbase bit clear.
    pub const M_REG: u8 = 0;
    /// Mined, non-stake (or treasury-gen) tag, coinbase bit set.
    pub const M_CB: u8 = 1;
    /// Mined vote or revocation output.
    pub const M_STAKEGEN: u8 = 2;
    /// Mined ticket change.
    pub const M_TCHANGE: u8 = 3;
    /// Ticket submission output, mined or unmined.
    pub const VOTEAUTH: u8 = 4;
    /// Unmined non-stake, coinbase bit clear, published.
    pub const U_REG_PUB: u8 = 5;
    /// Unmined non-stake, coinbase bit clear, unpublished.
    pub const U_REG_UNPUB: u8 = 6;
    /// Unmined non-stake, coinbase bit set, published.
    pub const U_CB_PUB: u8 = 7;
    /// Unmined non-stake, coinbase bit set, unpublished.
    pub const U_CB_UNPUB: u8 = 8;
    /// Unmined vote or revocation output.
    pub const U_STAKEGEN: u8 = 9;
    /// Unmined ticket change.
    pub const U_TCHANGE: u8 = 10;
    /// Unmined treasury-gen output (counts toward nothing).
    pub const U_TGEN: u8 = 11;
    /// Wallet ticket commitment not spent by an unmined vote.
    pub const LOCKED: u8 = 12;

    /// Whether the class keeps per-height sums.
    pub fn windowed(c: u8) -> bool {
        c <= M_TCHANGE
    }
}

/// Store errors.
#[derive(Debug)]
pub enum Error {
    /// The engine failed.
    Db(redb::Error),
    /// The data or the request is not what the prototype supports.
    Invalid(String),
    /// The file's schema version is missing or newer than this build
    /// writes; the store refuses to open it.
    Version {
        /// The version recorded in the file (`None`: no version row).
        found: Option<u32>,
        /// The newest version this build reads.
        supported: u32,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Db(e) => write!(f, "redb: {e}"),
            Error::Invalid(s) => f.write_str(s),
            Error::Version {
                found: Some(v),
                supported,
            } => write!(
                f,
                "store schema version {v} is newer than this build's {supported}"
            ),
            Error::Version {
                found: None,
                supported,
            } => write!(
                f,
                "store has no schema version (this build reads up to {supported})"
            ),
        }
    }
}

impl std::error::Error for Error {}

macro_rules! from_redb {
    ($($t:ty),*) => {$(
        impl From<$t> for Error {
            fn from(e: $t) -> Self {
                Error::Db(e.into())
            }
        }
    )*};
}
from_redb!(
    redb::Error,
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError,
    redb::SetDurabilityError
);

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

fn invalid<T>(s: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(s.into()))
}

// ---------------------------------------------------------------------------
// Encodings

/// A 36-byte outpoint key: tx hash ‖ index u32 BE (dcrwallet's canonical
/// outpoint).
pub fn outpoint(hash: &[u8; 32], index: u32) -> [u8; 36] {
    let mut k = [0; 36];
    k[..32].copy_from_slice(hash);
    k[32..].copy_from_slice(&index.to_be_bytes());
    k
}

/// A 36-byte mined-record key: tx hash ‖ height u32 BE.
pub fn rec_key(hash: &[u8; 32], height: u32) -> [u8; 36] {
    let mut k = [0; 36];
    k[..32].copy_from_slice(hash);
    k[32..].copy_from_slice(&height.to_be_bytes());
    k
}

/// A 40-byte credit or debit key: tx hash ‖ height u32 BE ‖ index u32 BE.
pub fn cd_key(hash: &[u8; 32], height: u32, index: u32) -> [u8; 40] {
    let mut k = [0; 40];
    k[..32].copy_from_slice(hash);
    k[32..36].copy_from_slice(&height.to_be_bytes());
    k[36..].copy_from_slice(&index.to_be_bytes());
    k
}

/// The outpoint a credit key names.
fn cd_outpoint(k: &[u8]) -> [u8; 36] {
    let mut o = [0; 36];
    o[..32].copy_from_slice(&k[..32]);
    o[32..].copy_from_slice(&k[36..40]);
    o
}

fn hash32(b: &[u8]) -> [u8; 32] {
    b[..32].try_into().expect("32 bytes")
}

/// SHA-256 of a HASH160: the key dcrwallet's address buckets use
/// (wallet/udb/addressdb.go:1343).
pub fn addr_sha(h: &[u8; 20]) -> [u8; 32] {
    Sha256::digest(h).into()
}

/// A decoded credit value (`CREDITS`, `CREDITS_INVALIDATED`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreditVal {
    /// Amount.
    pub amount: i64,
    /// Flags.
    pub flags: u8,
    /// Account.
    pub account: u32,
    /// Script offset in the serialized transaction.
    pub script_off: u32,
    /// Script length.
    pub script_len: u16,
    /// 1: P2PKH (possibly stake-tagged), 0: other.
    pub script_kind: u8,
    /// Spender (hash, height, input) when spent by a mined transaction.
    pub spender: Option<([u8; 32], u32, u32)>,
}

const CREDIT_LEN: usize = 60;

impl CreditVal {
    fn encode(&self) -> [u8; CREDIT_LEN] {
        let mut v = [0; CREDIT_LEN];
        v[0..8].copy_from_slice(&self.amount.to_be_bytes());
        v[8] = self.flags;
        v[9..13].copy_from_slice(&self.account.to_be_bytes());
        v[13..17].copy_from_slice(&self.script_off.to_be_bytes());
        v[17..19].copy_from_slice(&self.script_len.to_be_bytes());
        v[19] = self.script_kind;
        if let Some((h, height, i)) = self.spender {
            v[20..52].copy_from_slice(&h);
            v[52..56].copy_from_slice(&height.to_be_bytes());
            v[56..60].copy_from_slice(&i.to_be_bytes());
        }
        v
    }

    /// Decodes a value (`None` if malformed).
    pub fn decode(v: &[u8]) -> Option<CreditVal> {
        (v.len() == CREDIT_LEN).then(|| CreditVal {
            amount: be64(v, 0),
            flags: v[8],
            account: be32(v, 9),
            script_off: be32(v, 13),
            script_len: be16(v, 17),
            script_kind: v[19],
            spender: (v[8] & F_SPENT != 0).then(|| (hash32(&v[20..52]), be32(v, 52), be32(v, 56))),
        })
    }

    /// The `UNSPENT` value for this credit, mined at `height`.
    fn unspent(&self, height: u32) -> UnspentVal {
        UnspentVal {
            height,
            account: self.account,
            amount: self.amount,
            flags: self.flags & !F_SPENT,
            class: mined_class(flags_tag(self.flags), self.flags & F_COINBASE != 0),
            script_kind: self.script_kind,
            script_off: self.script_off,
            script_len: self.script_len,
        }
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn be64(b: &[u8], at: usize) -> i64 {
    i64::from_be_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(b[at..at + 2].try_into().expect("2 bytes"))
}

fn tag_code(tag: u8) -> u8 {
    match tag {
        0xb9..=0xbd => tag - 0xb9,
        OP_TGEN => 7,
        _ => 0,
    }
}

/// The opcode tag stored in a flags byte.
pub fn flags_tag(flags: u8) -> u8 {
    let code = (flags >> 2) & 7;
    if code == 7 { OP_TGEN } else { 0xb9 + code }
}

const F_CHANGE: u8 = 1;
const F_SPENT: u8 = 2;
const F_COINBASE: u8 = 1 << 5;
const F_EXPIRY: u8 = 1 << 6;

fn mined_class(tag: u8, coinbase: bool) -> u8 {
    match tag {
        OP_SSTX => class::VOTEAUTH,
        OP_SSGEN | OP_SSRTX => class::M_STAKEGEN,
        OP_SSTXCHANGE => class::M_TCHANGE,
        _ if coinbase => class::M_CB,
        _ => class::M_REG,
    }
}

fn unmined_class(tag: u8, coinbase: bool, unpublished: bool) -> u8 {
    match tag {
        OP_SSTX => class::VOTEAUTH,
        OP_SSGEN | OP_SSRTX => class::U_STAKEGEN,
        OP_SSTXCHANGE => class::U_TCHANGE,
        OP_TGEN => class::U_TGEN,
        _ => match (coinbase, unpublished) {
            (false, false) => class::U_REG_PUB,
            (false, true) => class::U_REG_UNPUB,
            (true, false) => class::U_CB_PUB,
            (true, true) => class::U_CB_UNPUB,
        },
    }
}

/// A decoded `UNSPENT` value.
#[derive(Clone, Copy, Debug)]
pub struct UnspentVal {
    /// Block height.
    pub height: u32,
    /// Owning account.
    pub account: u32,
    /// Amount in atoms.
    pub amount: i64,
    /// Credit flags.
    pub flags: u8,
    /// Balance class.
    pub class: u8,
    /// 1: P2PKH (possibly stake-tagged), 0: other.
    pub script_kind: u8,
    /// Script offset within the serialized transaction.
    pub script_off: u32,
    /// Script length.
    pub script_len: u16,
}

impl UnspentVal {
    fn encode(&self) -> [u8; 25] {
        let mut v = [0; 25];
        v[0..4].copy_from_slice(&self.height.to_be_bytes());
        v[4..8].copy_from_slice(&self.account.to_be_bytes());
        v[8..16].copy_from_slice(&self.amount.to_be_bytes());
        v[16] = self.flags;
        v[17] = self.class;
        v[18] = self.script_kind;
        v[19..23].copy_from_slice(&self.script_off.to_be_bytes());
        v[23..25].copy_from_slice(&self.script_len.to_be_bytes());
        v
    }

    /// Decodes a value (`None` if malformed).
    pub fn decode(v: &[u8]) -> Option<UnspentVal> {
        (v.len() == 25).then(|| UnspentVal {
            height: be32(v, 0),
            account: be32(v, 4),
            amount: be64(v, 8),
            flags: v[16],
            class: v[17],
            script_kind: v[18],
            script_off: be32(v, 19),
            script_len: be16(v, 23),
        })
    }
}

/// A decoded `UNMINED_CREDITS` value.
#[derive(Clone, Copy, Debug)]
pub struct McVal {
    /// Amount.
    pub amount: i64,
    /// Flags.
    pub flags: u8,
    /// Account.
    pub account: u32,
    /// Balance class.
    pub class: u8,
    /// Script kind.
    pub script_kind: u8,
    /// Script offset.
    pub script_off: u32,
    /// Script length.
    pub script_len: u16,
}

impl McVal {
    fn encode(&self) -> [u8; 21] {
        let mut v = [0; 21];
        v[0..8].copy_from_slice(&self.amount.to_be_bytes());
        v[8] = self.flags;
        v[9..13].copy_from_slice(&self.account.to_be_bytes());
        v[13] = self.class;
        v[14] = self.script_kind;
        v[15..19].copy_from_slice(&self.script_off.to_be_bytes());
        v[19..21].copy_from_slice(&self.script_len.to_be_bytes());
        v
    }

    /// Decodes a value.
    pub fn decode(v: &[u8]) -> Option<McVal> {
        (v.len() == 21).then(|| McVal {
            amount: be64(v, 0),
            flags: v[8],
            account: be32(v, 9),
            class: v[13],
            script_kind: v[14],
            script_off: be32(v, 15),
            script_len: be16(v, 19),
        })
    }
}

fn acct_hist_key(account: u32, height: u32, seq: u32) -> [u8; 12] {
    let mut k = [0; 12];
    k[0..4].copy_from_slice(&account.to_be_bytes());
    k[4..8].copy_from_slice(&height.to_be_bytes());
    k[8..12].copy_from_slice(&seq.to_be_bytes());
    k
}

fn hist_key(height: u32, seq: u32) -> [u8; 8] {
    let mut k = [0; 8];
    k[0..4].copy_from_slice(&height.to_be_bytes());
    k[4..8].copy_from_slice(&seq.to_be_bytes());
    k
}

fn path_key(account: u32, branch: u8, index: u32) -> [u8; 9] {
    let mut k = [0; 9];
    k[0..4].copy_from_slice(&account.to_be_bytes());
    k[4] = branch;
    k[5..9].copy_from_slice(&index.to_be_bytes());
    k
}

fn bal_key(account: u32, class: u8) -> [u8; 5] {
    let mut k = [0; 5];
    k[0..4].copy_from_slice(&account.to_be_bytes());
    k[4] = class;
    k
}

fn bal_height_key(account: u32, class: u8, height: u32) -> [u8; 9] {
    let mut k = [0; 9];
    k[0..5].copy_from_slice(&bal_key(account, class));
    k[5..9].copy_from_slice(&height.to_be_bytes());
    k
}

fn get<T: ReadableTable<K, K>>(t: &T, k: &[u8]) -> Result<Option<Vec<u8>>> {
    Ok(t.get(k)?.map(|g| g.value().to_vec()))
}

fn has<T: ReadableTable<K, K>>(t: &T, k: &[u8]) -> Result<bool> {
    Ok(t.get(k)?.is_some())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The database

/// The prototype store.
pub struct Store {
    /// The engine handle.
    pub db: Database,
}

/// Account counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcctVars {
    /// Last used external child (`u32::MAX`: none).
    pub last_used_ext: u32,
    /// Last used internal child.
    pub last_used_int: u32,
    /// Last returned external child.
    pub last_ret_ext: u32,
    /// Last returned internal child.
    pub last_ret_int: u32,
}

impl AcctVars {
    fn decode(v: &[u8]) -> AcctVars {
        AcctVars {
            last_used_ext: be32(v, 0),
            last_used_int: be32(v, 4),
            last_ret_ext: be32(v, 8),
            last_ret_int: be32(v, 12),
        }
    }

    fn encode(&self, name: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(16 + name.len());
        for x in [
            self.last_used_ext,
            self.last_used_int,
            self.last_ret_ext,
            self.last_ret_int,
        ] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.extend_from_slice(name);
        v
    }
}

impl Store {
    /// Creates a store at `path` (which must not exist) with the schema's
    /// tables and the genesis block as tip.
    pub fn create(path: &Path) -> Result<Store> {
        if path.exists() {
            return invalid(format!("{} exists", path.display()));
        }
        Store::init(Database::create(path)?)
    }

    fn init(db: Database) -> Result<Store> {
        let txn = db.begin_write()?;
        {
            let mut w = W::open(&txn)?;
            w.meta.insert(
                b"version".as_slice(),
                SCHEMA_VERSION.to_be_bytes().as_slice(),
            )?;
            w.tip_height = 0;
            w.tip_hash = chain::SIMNET_GENESIS;
            let mut rec = Vec::with_capacity(47);
            rec.extend_from_slice(&chain::SIMNET_GENESIS);
            rec.extend_from_slice(&0i64.to_be_bytes());
            rec.extend_from_slice(&0u16.to_be_bytes());
            rec.push(0);
            rec.extend_from_slice(&0u32.to_be_bytes());
            w.blocks
                .insert(0u32.to_be_bytes().as_slice(), rec.as_slice())?;
            w.block_by_hash.insert(
                chain::SIMNET_GENESIS.as_slice(),
                0u32.to_be_bytes().as_slice(),
            )?;
            w.flush_meta()?;
        }
        txn.commit()?;
        Ok(Store { db })
    }

    /// Opens an existing store, refusing a file whose schema version is
    /// missing or newer than [`SCHEMA_VERSION`] (an older one would be
    /// migrated here, one transaction per step; the spike has none).
    pub fn open(path: &Path) -> Result<Store> {
        let db = Database::open(path)?;
        let st = Store { db };
        st.check_version()?;
        Ok(st)
    }

    /// Wraps an already opened database (another storage backend), with
    /// the same version check as [`Store::open`].
    pub fn from_db(db: Database) -> Result<Store> {
        let st = Store { db };
        st.check_version()?;
        Ok(st)
    }

    /// Creates a store on an already created, empty database.
    pub fn create_on(db: Database) -> Result<Store> {
        Store::init(db)
    }

    fn check_version(&self) -> Result<()> {
        let txn = self.db.begin_read()?;
        let found = match txn.open_table(META) {
            Ok(t) => get(&t, b"version")?.and_then(|v| v.get(..4).map(|b| be32(b, 0))),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(e.into()),
        };
        match found {
            Some(v) if v <= SCHEMA_VERSION => Ok(()),
            found => Err(Error::Version {
                found,
                supported: SCHEMA_VERSION,
            }),
        }
    }

    /// Creates the accounts and records the given addresses, in one
    /// transaction.
    pub fn setup(&self, accounts: u32, ranges: &[crate::workload::AddrRange]) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut w = W::open(&txn)?;
            for a in 0..accounts {
                let name = if a == 0 {
                    "default".to_owned()
                } else {
                    format!("acct-{a}")
                };
                let vars = AcctVars {
                    last_used_ext: u32::MAX,
                    last_used_int: u32::MAX,
                    last_ret_ext: u32::MAX,
                    last_ret_int: u32::MAX,
                };
                w.accounts.insert(
                    a.to_le_bytes().as_slice(),
                    vars.encode(name.as_bytes()).as_slice(),
                )?;
                w.account_names
                    .insert(name.as_bytes(), a.to_be_bytes().as_slice())?;
            }
            for r in ranges {
                for (i, h) in r.hashes.iter().enumerate() {
                    let i = u32::try_from(i).expect("index");
                    w.put_address(r.account, r.branch as u8, i, h)?;
                }
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Begins a read transaction with every table open.
    pub fn read(&self) -> Result<R> {
        R::open(self.db.begin_read()?)
    }

    /// Applies one write transaction's worth of operations.
    pub fn apply(&self, ops: &[ParsedOp], counters: &mut Counters) -> Result<()> {
        self.apply_with(ops, counters, Durability::Immediate)
    }

    /// [`Store::apply`] with an explicit durability for the commit.
    pub fn apply_with(
        &self,
        ops: &[ParsedOp],
        counters: &mut Counters,
        durability: Durability,
    ) -> Result<()> {
        let mut txn = self.db.begin_write()?;
        txn.set_durability(durability)?;
        {
            let mut w = W::open(&txn)?;
            for op in ops {
                w.apply(op, counters)?;
            }
            w.flush_meta()?;
        }
        txn.commit()?;
        Ok(())
    }
}

/// An operation with its transactions parsed (outside the timed region,
/// as the Go side parses them before ingest).
#[derive(Clone, Debug)]
pub struct ParsedOp {
    /// The workload kind.
    pub kind: crate::workload::OpKind,
    /// Whether the write transaction ends here.
    pub commit: bool,
    /// The unpublished flag of an unmined insert.
    pub unpublished: bool,
    /// The header of a block.
    pub header: Option<[u8; 180]>,
    /// For a reorganisation: the first height rolled back.
    pub rollback_to: u32,
    /// The transactions.
    pub txs: Vec<MsgTx>,
}

impl ParsedOp {
    /// Parses a workload operation.
    pub fn parse(op: &crate::workload::Op) -> Result<ParsedOp> {
        let mut txs = Vec::with_capacity(op.txs.len());
        for b in &op.txs {
            let (tx, _) = MsgTx::from_bytes(b)
                .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))?;
            txs.push(tx);
        }
        Ok(ParsedOp {
            kind: op.kind,
            commit: op.commit,
            unpublished: op.unpublished,
            header: op.header,
            rollback_to: op.rollback_to,
            txs,
        })
    }
}

/// What ingest counted.
#[derive(Clone, Debug, Default)]
pub struct Counters {
    /// Address lookups by script.
    pub lookups: u64,
    /// Lookups that found no wallet address.
    pub lookup_misses: u64,
    /// Credits added.
    pub credits: u64,
    /// Ticket commitments added.
    pub commitments: u64,
    /// Address syncs whose target child was not recorded (would derive).
    pub sync_misses: u64,
}

/// A transaction being recorded.
struct Rec<'a> {
    tx: &'a MsgTx,
    hash: [u8; 32],
    bytes: Vec<u8>,
    ty: TxType,
    coinbase: bool,
    locs: Vec<usize>,
    unpublished: bool,
}

impl<'a> Rec<'a> {
    fn new(tx: &'a MsgTx, unpublished: bool) -> Rec<'a> {
        Rec {
            tx,
            hash: tx.tx_hash().0,
            bytes: tx.serialize(),
            ty: chain::tx_type(tx),
            coinbase: chain::is_coinbase(tx),
            locs: chain::pk_script_locs(tx),
            unpublished,
        }
    }
}

// ---------------------------------------------------------------------------
// Write side

/// Every table of a write transaction, opened once.
pub struct W<'t> {
    meta: Table<'t, K, K>,
    accounts: Table<'t, K, K>,
    account_names: Table<'t, K, K>,
    addr_by_hash: Table<'t, K, K>,
    addr_by_path: Table<'t, K, K>,
    addr_by_acct_sha: Table<'t, K, K>,
    headers: Table<'t, K, K>,
    blocks: Table<'t, K, K>,
    block_by_hash: Table<'t, K, K>,
    cfilters: Table<'t, K, K>,
    tx_mined: Table<'t, K, K>,
    tx_unmined: Table<'t, K, K>,
    unpublished: Table<'t, K, K>,
    history: Table<'t, K, K>,
    acct_history: Table<'t, K, K>,
    credits: Table<'t, K, K>,
    credits_invalidated: Table<'t, K, K>,
    unspent: Table<'t, K, K>,
    unmined_credits: Table<'t, K, K>,
    unmined_inputs: Table<'t, K, K>,
    debits: Table<'t, K, K>,
    debits_invalidated: Table<'t, K, K>,
    tickets: Table<'t, K, K>,
    commitments: Table<'t, K, K>,
    commitments_unspent: Table<'t, K, K>,
    balances: Table<'t, K, K>,
    balance_heights: Table<'t, K, K>,
    tip_height: u32,
    tip_hash: [u8; 32],
    unspent_n: u64,
    last_op: u64,
    processed: Option<[u8; 32]>,
}

impl<'t> W<'t> {
    /// Opens every table in `txn`.
    pub fn open(txn: &'t WriteTransaction) -> Result<W<'t>> {
        let meta = txn.open_table(META)?;
        let (tip_height, tip_hash) = match get(&meta, b"tip")? {
            Some(v) => (be32(&v, 0), v[4..36].try_into().expect("hash")),
            None => (0, [0; 32]),
        };
        let unspent_n = get(&meta, b"unspent_n")?.map_or(0, |v| be64(&v, 0) as u64);
        let last_op = get(&meta, b"last_op")?.map_or(0, |v| be64(&v, 0) as u64);
        Ok(W {
            meta,
            accounts: txn.open_table(ACCOUNTS)?,
            account_names: txn.open_table(ACCOUNT_NAMES)?,
            addr_by_hash: txn.open_table(ADDR_BY_HASH)?,
            addr_by_path: txn.open_table(ADDR_BY_PATH)?,
            addr_by_acct_sha: txn.open_table(ADDR_BY_ACCT_SHA)?,
            headers: txn.open_table(HEADERS)?,
            blocks: txn.open_table(BLOCKS)?,
            block_by_hash: txn.open_table(BLOCK_BY_HASH)?,
            cfilters: txn.open_table(CFILTERS)?,
            tx_mined: txn.open_table(TX_MINED)?,
            tx_unmined: txn.open_table(TX_UNMINED)?,
            unpublished: txn.open_table(UNPUBLISHED)?,
            history: txn.open_table(HISTORY)?,
            acct_history: txn.open_table(ACCT_HISTORY)?,
            credits: txn.open_table(CREDITS)?,
            credits_invalidated: txn.open_table(CREDITS_INVALIDATED)?,
            unspent: txn.open_table(UNSPENT)?,
            unmined_credits: txn.open_table(UNMINED_CREDITS)?,
            unmined_inputs: txn.open_table(UNMINED_INPUTS)?,
            debits: txn.open_table(DEBITS)?,
            debits_invalidated: txn.open_table(DEBITS_INVALIDATED)?,
            tickets: txn.open_table(TICKETS)?,
            commitments: txn.open_table(COMMITMENTS)?,
            commitments_unspent: txn.open_table(COMMITMENTS_UNSPENT)?,
            balances: txn.open_table(BALANCES)?,
            balance_heights: txn.open_table(BALANCE_HEIGHTS)?,
            tip_height,
            tip_hash,
            unspent_n,
            last_op,
            processed: None,
        })
    }

    /// Writes the cached meta values back.
    pub fn flush_meta(&mut self) -> Result<()> {
        let mut tip = [0; 36];
        tip[..4].copy_from_slice(&self.tip_height.to_be_bytes());
        tip[4..].copy_from_slice(&self.tip_hash);
        self.meta.insert(b"tip".as_slice(), tip.as_slice())?;
        self.meta.insert(
            b"unspent_n".as_slice(),
            (self.unspent_n as i64).to_be_bytes().as_slice(),
        )?;
        self.meta.insert(
            b"last_op".as_slice(),
            (self.last_op as i64).to_be_bytes().as_slice(),
        )?;
        if let Some(p) = self.processed {
            self.meta.insert(b"processed".as_slice(), p.as_slice())?;
        }
        Ok(())
    }

    fn put_address(&mut self, account: u32, branch: u8, index: u32, h: &[u8; 20]) -> Result<()> {
        let mut v = [0; 9];
        v.copy_from_slice(&path_key(account, branch, index));
        self.addr_by_hash.insert(h.as_slice(), v.as_slice())?;
        self.addr_by_path
            .insert(path_key(account, branch, index).as_slice(), h.as_slice())?;
        let mut k = [0; 36];
        k[..4].copy_from_slice(&account.to_le_bytes());
        k[4..].copy_from_slice(&addr_sha(h));
        self.addr_by_acct_sha.insert(k.as_slice(), h.as_slice())?;
        Ok(())
    }

    /// Records the next external address of `account` (the bookkeeping
    /// half of next-address derivation): stores the child if it is not
    /// recorded yet.
    pub fn record_address(
        &mut self,
        account: u32,
        branch: u8,
        index: u32,
        h: &[u8; 20],
    ) -> Result<bool> {
        if has(&self.addr_by_path, &path_key(account, branch, index))? {
            return Ok(false);
        }
        self.put_address(account, branch, index, h)?;
        Ok(true)
    }

    /// Applies one operation.
    pub fn apply(&mut self, op: &ParsedOp, c: &mut Counters) -> Result<()> {
        match op.kind {
            crate::workload::OpKind::Unmined => {
                let rec = Rec::new(&op.txs[0], op.unpublished);
                self.process_tx(&rec, None, c)?;
            }
            crate::workload::OpKind::Reorg => {
                self.rollback(op.rollback_to)?;
            }
            _ => self.attach_block(op.header.as_ref().expect("header"), &op.txs, c, None)?,
        }
        self.last_op += 1;
        Ok(())
    }

    // -- balance bookkeeping ------------------------------------------------

    fn bal_delta(
        &mut self,
        account: u32,
        class: u8,
        height: u32,
        amount: i64,
        sign: i64,
    ) -> Result<()> {
        fn bump(t: &mut Table<'_, K, K>, k: &[u8], amount: i64, sign: i64) -> Result<()> {
            let (mut sum, mut n) = match t.get(k)? {
                Some(g) => (be64(g.value(), 0), be64(g.value(), 8)),
                None => (0, 0),
            };
            sum = sum.wrapping_add(sign.wrapping_mul(amount));
            n = n.wrapping_add(sign);
            if n == 0 {
                if sum != 0 {
                    return invalid("balance sum nonzero with no outputs");
                }
                t.remove(k)?;
            } else {
                let mut v = [0; 16];
                v[..8].copy_from_slice(&sum.to_be_bytes());
                v[8..].copy_from_slice(&n.to_be_bytes());
                t.insert(k, v.as_slice())?;
            }
            Ok(())
        }
        bump(&mut self.balances, &bal_key(account, class), amount, sign)?;
        if class::windowed(class) {
            bump(
                &mut self.balance_heights,
                &bal_height_key(account, class, height),
                amount,
                sign,
            )?;
        }
        Ok(())
    }

    fn put_unspent(&mut self, op: &[u8; 36], v: &UnspentVal) -> Result<()> {
        let old = self
            .unspent
            .insert(op.as_slice(), v.encode().as_slice())?
            .map(|g| g.value().to_vec());
        let spent_unmined = has(&self.unmined_inputs, op)?;
        match old.as_deref().and_then(UnspentVal::decode) {
            Some(o) if !spent_unmined => {
                self.bal_delta(o.account, o.class, o.height, o.amount, -1)?;
            }
            Some(_) => {}
            None => self.unspent_n += 1,
        }
        if !spent_unmined {
            self.bal_delta(v.account, v.class, v.height, v.amount, 1)?;
        }
        Ok(())
    }

    fn del_unspent(&mut self, op: &[u8; 36]) -> Result<()> {
        let old = self
            .unspent
            .remove(op.as_slice())?
            .map(|g| g.value().to_vec());
        if let Some(o) = old.as_deref().and_then(UnspentVal::decode) {
            self.unspent_n -= 1;
            if !has(&self.unmined_inputs, op)? {
                self.bal_delta(o.account, o.class, o.height, o.amount, -1)?;
            }
        }
        Ok(())
    }

    fn put_mc(&mut self, op: &[u8; 36], v: &McVal) -> Result<()> {
        let old = self
            .unmined_credits
            .insert(op.as_slice(), v.encode().as_slice())?
            .map(|g| g.value().to_vec());
        let spent_unmined = has(&self.unmined_inputs, op)?;
        if let Some(o) = old.as_deref().and_then(McVal::decode)
            && !spent_unmined
        {
            self.bal_delta(o.account, o.class, 0, o.amount, -1)?;
        }
        if !spent_unmined {
            self.bal_delta(v.account, v.class, 0, v.amount, 1)?;
        }
        Ok(())
    }

    fn del_mc(&mut self, op: &[u8; 36]) -> Result<Option<McVal>> {
        let old = self
            .unmined_credits
            .remove(op.as_slice())?
            .map(|g| g.value().to_vec());
        let old = old.as_deref().and_then(McVal::decode);
        if let Some(o) = old
            && !has(&self.unmined_inputs, op)?
        {
            self.bal_delta(o.account, o.class, 0, o.amount, -1)?;
        }
        Ok(old)
    }

    fn put_mi(&mut self, op: &[u8; 36], spender: &[u8; 32]) -> Result<()> {
        let existed = self
            .unmined_inputs
            .insert(op.as_slice(), spender.as_slice())?
            .is_some();
        if !existed {
            if let Some(u) = get(&self.unspent, op)?
                .as_deref()
                .and_then(UnspentVal::decode)
            {
                self.bal_delta(u.account, u.class, u.height, u.amount, -1)?;
            }
            if let Some(m) = get(&self.unmined_credits, op)?
                .as_deref()
                .and_then(McVal::decode)
            {
                self.bal_delta(m.account, m.class, 0, m.amount, -1)?;
            }
        }
        Ok(())
    }

    fn del_mi(&mut self, op: &[u8; 36]) -> Result<()> {
        if self.unmined_inputs.remove(op.as_slice())?.is_some() {
            if let Some(u) = get(&self.unspent, op)?
                .as_deref()
                .and_then(UnspentVal::decode)
            {
                self.bal_delta(u.account, u.class, u.height, u.amount, 1)?;
            }
            if let Some(m) = get(&self.unmined_credits, op)?
                .as_deref()
                .and_then(McVal::decode)
            {
                self.bal_delta(m.account, m.class, 0, m.amount, 1)?;
            }
        }
        Ok(())
    }

    fn commitment(&self, op: &[u8; 36]) -> Result<Option<(i64, u32)>> {
        Ok(get(&self.commitments, op)?.map(|v| (be64(&v, 0), be32(&v, 8))))
    }

    fn put_cmu(&mut self, op: &[u8; 36], flags: u8) -> Result<()> {
        let old = self
            .commitments_unspent
            .insert(op.as_slice(), [flags].as_slice())?
            .map(|g| g.value()[0]);
        let Some((amount, account)) = self.commitment(op)? else {
            return invalid("unspent commitment without commitment");
        };
        if old == Some(0) {
            self.bal_delta(account, class::LOCKED, 0, amount, -1)?;
        }
        if flags == 0 {
            self.bal_delta(account, class::LOCKED, 0, amount, 1)?;
        }
        Ok(())
    }

    fn del_cmu(&mut self, op: &[u8; 36]) -> Result<()> {
        let old = self
            .commitments_unspent
            .remove(op.as_slice())?
            .map(|g| g.value()[0]);
        if old == Some(0) {
            let Some((amount, account)) = self.commitment(op)? else {
                return invalid("unspent commitment without commitment");
            };
            self.bal_delta(account, class::LOCKED, 0, amount, -1)?;
        }
        Ok(())
    }

    // -- chain --------------------------------------------------------------

    /// `ExtendMainChain` alone, with an explicit filter (16-byte key and
    /// the filter's bytes): the header-ingest measurement.
    pub fn attach_header(&mut self, header: &[u8; 180], filter: &[u8]) -> Result<()> {
        self.attach_block(header, &[], &mut Counters::default(), Some(filter))
    }

    /// `ExtendMainChain` plus the wallet's per-transaction processing:
    /// validates or invalidates the parent's regular tree by this block's
    /// vote bits first (txmined.go:234-241), then records the block and its
    /// transactions.
    fn attach_block(
        &mut self,
        header: &[u8; 180],
        txs: &[MsgTx],
        c: &mut Counters,
        filter: Option<&[u8]>,
    ) -> Result<()> {
        let prev: [u8; 32] = header[4..36].try_into().expect("hash");
        let vote_bits = u16::from_le_bytes([header[100], header[101]]);
        let height = u32::from_le_bytes(header[128..132].try_into().expect("u32"));
        let time = u32::from_le_bytes(header[136..140].try_into().expect("u32"));
        if height < 1 {
            return invalid("block 0 cannot be added");
        }
        if prev != self.tip_hash || height != self.tip_height + 1 {
            return invalid(format!("block {height} is not a child of the tip"));
        }
        if vote_bits & 1 != 0 {
            self.stake_validate(self.tip_height)?;
        } else {
            self.stake_invalidate(self.tip_height)?;
        }
        let hash = dcroxide_crypto::blake256::sum256(header);
        let hk = height.to_be_bytes();
        self.headers.insert(hk.as_slice(), header.as_slice())?;
        let mut rec = Vec::with_capacity(47);
        rec.extend_from_slice(&hash);
        rec.extend_from_slice(&i64::from(time).to_be_bytes());
        rec.extend_from_slice(&vote_bits.to_be_bytes());
        rec.push(0);
        rec.extend_from_slice(&0u32.to_be_bytes());
        self.blocks.insert(hk.as_slice(), rec.as_slice())?;
        self.block_by_hash.insert(hash.as_slice(), hk.as_slice())?;
        // The committed filter: its 16-byte key and the filter bytes; in
        // the workload the empty filter the Go side stores too.
        self.cfilters
            .insert(hk.as_slice(), filter.unwrap_or(&header[36..52]))?;
        self.tip_height = height;
        self.tip_hash = hash;
        for tx in txs {
            let rec = Rec::new(tx, false);
            self.process_tx(&rec, Some(height), c)?;
        }
        self.processed = Some(hash);
        Ok(())
    }

    fn block_row(&self, height: u32) -> Result<Vec<u8>> {
        match get(&self.blocks, &height.to_be_bytes())? {
            Some(b) => Ok(b),
            None => invalid(format!("missing block record for height {height}")),
        }
    }

    fn block_ntx(&self, height: u32) -> Result<u32> {
        Ok(be32(&self.block_row(height)?, 43))
    }

    fn block_invalid(&self, height: u32) -> Result<bool> {
        Ok(self.block_row(height)?[42] != 0)
    }

    /// The transactions of a block in the order dcrwallet's block record
    /// lists them: (position, hash).
    fn block_txs(&self, height: u32) -> Result<Vec<(u32, [u8; 32])>> {
        let lo = hist_key(height, 0);
        let hi = hist_key(height, u32::MAX);
        let mut out = Vec::new();
        for e in self.history.range(lo.as_slice()..=hi.as_slice())? {
            let (k, v) = e?;
            out.push((be32(k.value(), 4), hash32(v.value())));
        }
        Ok(out)
    }

    fn append_block(&mut self, height: u32, hash: &[u8; 32]) -> Result<u32> {
        let hk = height.to_be_bytes();
        let mut b = self.block_row(height)?;
        let seq = be32(&b, 43);
        b[43..47].copy_from_slice(&(seq + 1).to_be_bytes());
        self.blocks.insert(hk.as_slice(), b.as_slice())?;
        self.history
            .insert(hist_key(height, seq).as_slice(), hash.as_slice())?;
        Ok(seq)
    }

    fn acct_history(&mut self, account: u32, height: u32, seq: u32, hash: &[u8; 32]) -> Result<()> {
        self.acct_history.insert(
            acct_hist_key(account, height, seq).as_slice(),
            hash.as_slice(),
        )?;
        Ok(())
    }

    /// The accounts a mined record touches: the owners of its live credits
    /// and of the credits its live debits spend.
    fn acct_set(&self, hash: &[u8; 32], height: u32) -> Result<BTreeSet<u32>> {
        let lo = cd_key(hash, height, 0);
        let hi = cd_key(hash, height, u32::MAX);
        let mut s = BTreeSet::new();
        for e in self.credits.range(lo.as_slice()..=hi.as_slice())? {
            let (_, v) = e?;
            s.insert(be32(v.value(), 9));
        }
        for e in self.debits.range(lo.as_slice()..=hi.as_slice())? {
            let (_, v) = e?;
            let Some(cv) = get(&self.credits, &v.value()[8..48])? else {
                return invalid("debit of a missing credit");
            };
            s.insert(be32(&cv, 9));
        }
        Ok(s)
    }

    fn set_acct_history(
        &mut self,
        height: u32,
        seq: u32,
        hash: &[u8; 32],
        before: &BTreeSet<u32>,
        after: &BTreeSet<u32>,
    ) -> Result<()> {
        for a in before.difference(after) {
            self.acct_history
                .remove(acct_hist_key(*a, height, seq).as_slice())?;
        }
        for a in after.difference(before) {
            self.acct_history(*a, height, seq, hash)?;
        }
        Ok(())
    }

    /// `processTransactionRecord`: insert, then credits and commitments
    /// for every output paying a wallet address, then commitment
    /// redemption for a vote.
    fn process_tx(&mut self, rec: &Rec<'_>, height: Option<u32>, c: &mut Counters) -> Result<()> {
        let seq = match height {
            None => {
                if !self.insert_mempool(rec)? {
                    return Ok(());
                }
                None
            }
            Some(h) => Some((h, self.insert_mined(rec, h)?)),
        };
        for (i, out) in rec.tx.tx_out.iter().enumerate() {
            let i = u32::try_from(i).expect("index");
            let kind = chain::classify(&out.pk_script);
            let commit = rec.ty == TxType::Ticket && i % 2 == 1;
            let hash = if commit {
                match chain::decode_commitment(&out.pk_script) {
                    Some((h, _)) => h,
                    None => continue,
                }
            } else {
                match kind {
                    ScriptKind::NonStandard | ScriptKind::NullData => continue,
                    ScriptKind::PubKeyHash { hash, .. } => {
                        if out.value == 0 {
                            continue;
                        }
                        hash
                    }
                }
            };
            c.lookups += 1;
            let Some(a) = get(&self.addr_by_hash, &hash)? else {
                c.lookup_misses += 1;
                continue;
            };
            let (account, branch, index) = (be32(&a, 0), a[4], be32(&a, 5));
            if commit {
                self.add_ticket_commitment(rec, i, account)?;
                c.commitments += 1;
            } else {
                self.add_credit(rec, seq, i, branch == 1, account)?;
                c.credits += 1;
            }
            self.mark_used(account, branch, index, c)?;
        }
        if rec.ty == TxType::Vote {
            self.redeem_ticket_commitments(rec, height.is_some())?;
        }
        Ok(())
    }

    /// `markUsedAddress`: the account's last used (and, if lower, last
    /// returned) child, then the gap check. The check finds the child in
    /// `ADDR_BY_PATH` rather than deriving it as dcrwallet does.
    fn mark_used(&mut self, account: u32, branch: u8, index: u32, c: &mut Counters) -> Result<()> {
        let key = account.to_le_bytes();
        let Some(v) = get(&self.accounts, &key)? else {
            return invalid(format!("no account {account}"));
        };
        let mut vars = AcctVars::decode(&v);
        let (used, ret) = if branch == 0 {
            (&mut vars.last_used_ext, &mut vars.last_ret_ext)
        } else {
            (&mut vars.last_used_int, &mut vars.last_ret_int)
        };
        if index.wrapping_add(1) > used.wrapping_add(1) {
            *used = index;
            if ret.wrapping_add(1) < index.wrapping_add(1) {
                *ret = index;
            }
            let updated = vars.encode(&v[16..]);
            self.accounts.insert(key.as_slice(), updated.as_slice())?;
        }
        let last_used = if branch == 0 {
            vars.last_used_ext
        } else {
            vars.last_used_int
        };
        let target = (0x7fff_ffffu32).min(last_used.wrapping_add(chain::GAP_LIMIT));
        if !has(&self.addr_by_path, &path_key(account, branch, target))? {
            c.sync_misses += 1;
        }
        Ok(())
    }

    /// The highest-height mined record of a hash (`latestTxRecord`).
    fn latest_mined(&self, hash: &[u8; 32]) -> Result<Option<u32>> {
        let lo = rec_key(hash, 0);
        let hi = rec_key(hash, u32::MAX);
        let mut r = self.tx_mined.range(lo.as_slice()..=hi.as_slice())?;
        Ok(match r.next_back() {
            Some(e) => Some(be32(e?.0.value(), 32)),
            None => None,
        })
    }

    /// `InsertMinedTx` (no double-spend removal). Returns the
    /// transaction's position in its block. Into a block already marked
    /// invalid, a regular transaction's debits go to
    /// `DEBITS_INVALIDATED` and spend nothing (txmined.go:1275-1289).
    fn insert_mined(&mut self, rec: &Rec<'_>, height: u32) -> Result<u32> {
        let seq = self.block_ntx(height)?;
        let invalidated = rec.ty == TxType::Regular && self.block_invalid(height)?;
        for (i, input) in rec.tx.tx_in.iter().enumerate() {
            let prev = &input.previous_out_point;
            let op = outpoint(&prev.hash.0, prev.index);
            let Some(u) = get(&self.unspent, &op)?
                .as_deref()
                .and_then(UnspentVal::decode)
            else {
                continue;
            };
            let i = u32::try_from(i).expect("index");
            let ck = cd_key(&prev.hash.0, u.height, prev.index);
            let dk = cd_key(&rec.hash, height, i);
            let mut d = [0; 48];
            d[..8].copy_from_slice(&u.amount.to_be_bytes());
            d[8..].copy_from_slice(&ck);
            if invalidated {
                self.debits_invalidated
                    .insert(dk.as_slice(), d.as_slice())?;
                continue;
            }
            self.spend_credit(&ck, &rec.hash, height, i)?;
            self.debits.insert(dk.as_slice(), d.as_slice())?;
            self.acct_history(u.account, height, seq, &rec.hash)?;
            self.del_unspent(&op)?;
        }
        if let Some(v) = get(&self.tx_mined, &rec_key(&rec.hash, height))? {
            return Ok(be32(&v, 0));
        }
        if rec.ty == TxType::Ticket && !has(&self.tickets, &rec.hash)? {
            self.tickets.insert(rec.hash.as_slice(), [].as_slice())?;
        }
        if let Some(uv) = get(&self.tx_unmined, &rec.hash)? {
            if invalidated {
                return invalid(
                    "an unmined transaction mined into a disapproved block (dcrwallet panics here)",
                );
            }
            self.move_mined(rec, height, &uv)?;
            return Ok(seq);
        }
        for input in &rec.tx.tx_in {
            let prev = &input.previous_out_point;
            if has(&self.unmined_inputs, &outpoint(&prev.hash.0, prev.index))? {
                return invalid("double-spend removal is not implemented in the prototype");
            }
        }
        let got = self.append_block(height, &rec.hash)?;
        debug_assert_eq!(got, seq);
        let mut v = Vec::with_capacity(12 + rec.bytes.len());
        v.extend_from_slice(&seq.to_be_bytes());
        v.extend_from_slice(&now().to_be_bytes());
        v.extend_from_slice(&rec.bytes);
        self.tx_mined
            .insert(rec_key(&rec.hash, height).as_slice(), v.as_slice())?;
        Ok(seq)
    }

    fn credit(&self, ck: &[u8]) -> Result<Option<CreditVal>> {
        match get(&self.credits, ck)? {
            None => Ok(None),
            Some(v) => match CreditVal::decode(&v) {
                Some(c) => Ok(Some(c)),
                None => invalid("bad credit value"),
            },
        }
    }

    fn put_credit(&mut self, ck: &[u8; 40], c: &CreditVal) -> Result<()> {
        self.credits.insert(ck.as_slice(), c.encode().as_slice())?;
        Ok(())
    }

    /// `spendCredit`.
    fn spend_credit(
        &mut self,
        ck: &[u8; 40],
        spender: &[u8; 32],
        height: u32,
        input: u32,
    ) -> Result<()> {
        let Some(mut c) = self.credit(ck)? else {
            return invalid("missing credit for unspent output");
        };
        c.flags |= F_SPENT;
        c.spender = Some((*spender, height, input));
        self.put_credit(ck, &c)
    }

    /// `unspendRawCredit`: the credit marked unspent, or `None` when it no
    /// longer exists (which dcrwallet also tolerates).
    fn unspend_credit(&mut self, ck: &[u8; 40]) -> Result<Option<CreditVal>> {
        let Some(mut c) = self.credit(ck)? else {
            return Ok(None);
        };
        c.flags &= !F_SPENT;
        c.spender = None;
        self.put_credit(ck, &c)?;
        Ok(Some(c))
    }

    /// `moveMinedTx`: the unmined record becomes the mined one, its unmined
    /// credits become credits and unspent outputs.
    fn move_mined(&mut self, rec: &Rec<'_>, height: u32, uv: &[u8]) -> Result<()> {
        let seq = self.append_block(height, &rec.hash)?;
        let mut v = Vec::with_capacity(4 + uv.len());
        v.extend_from_slice(&seq.to_be_bytes());
        v.extend_from_slice(uv);
        self.tx_mined
            .insert(rec_key(&rec.hash, height).as_slice(), v.as_slice())?;
        for input in &rec.tx.tx_in {
            let prev = &input.previous_out_point;
            self.del_mi(&outpoint(&prev.hash.0, prev.index))?;
        }
        for i in 0..rec.tx.tx_out.len() {
            let i = u32::try_from(i).expect("index");
            let op = outpoint(&rec.hash, i);
            let Some(m) = self.del_mc(&op)? else {
                continue;
            };
            // dcrwallet writes the moved credit without the expiry flag.
            let c = CreditVal {
                amount: m.amount,
                flags: m.flags & !F_EXPIRY,
                account: m.account,
                script_off: m.script_off,
                script_len: m.script_len,
                script_kind: m.script_kind,
                spender: None,
            };
            self.put_credit(&cd_key(&rec.hash, height, i), &c)?;
            self.put_unspent(&op, &c.unspent(height))?;
            self.acct_history(m.account, height, seq, &rec.hash)?;
        }
        self.unpublished.remove(rec.hash.as_slice())?;
        self.tx_unmined.remove(rec.hash.as_slice())?;
        Ok(())
    }

    /// `InsertMemPoolTx` (no conflict handling). Returns false when the
    /// transaction has a mined record, as dcrwallet then skips its credits
    /// (txunmined.go:26, which looks for any record of the hash, one in a
    /// disapproved block included).
    fn insert_mempool(&mut self, rec: &Rec<'_>) -> Result<bool> {
        if self.latest_mined(&rec.hash)?.is_some() {
            return Ok(false);
        }
        if has(&self.tx_unmined, &rec.hash)? {
            return Ok(true);
        }
        for input in &rec.tx.tx_in {
            let prev = &input.previous_out_point;
            if has(&self.unmined_inputs, &outpoint(&prev.hash.0, prev.index))? {
                return invalid("unmined double spends are not implemented in the prototype");
            }
        }
        let mut v = Vec::with_capacity(8 + rec.bytes.len());
        v.extend_from_slice(&now().to_be_bytes());
        v.extend_from_slice(&rec.bytes);
        self.tx_unmined.insert(rec.hash.as_slice(), v.as_slice())?;
        if rec.unpublished {
            self.unpublished
                .insert(rec.hash.as_slice(), [1u8].as_slice())?;
        }
        for (i, input) in rec.tx.tx_in.iter().enumerate() {
            if i == 0 && rec.ty == TxType::Vote {
                continue;
            }
            let prev = &input.previous_out_point;
            self.put_mi(&outpoint(&prev.hash.0, prev.index), &rec.hash)?;
        }
        if rec.ty == TxType::Ticket && !has(&self.tickets, &rec.hash)? {
            self.tickets.insert(rec.hash.as_slice(), [].as_slice())?;
        }
        Ok(true)
    }

    /// `AddCredit`. Into a disapproved block, a regular transaction's
    /// credit goes to `CREDITS_INVALIDATED` with no unspent output
    /// (txmined.go:1399-1427).
    fn add_credit(
        &mut self,
        rec: &Rec<'_>,
        mined: Option<(u32, u32)>,
        index: u32,
        change: bool,
        account: u32,
    ) -> Result<bool> {
        let out = &rec.tx.tx_out[index as usize];
        let (tag, script_kind) = match chain::classify(&out.pk_script) {
            ScriptKind::PubKeyHash { tag, .. } => (tag, 1),
            _ => (OP_NONSTAKE, 0),
        };
        let mut flags = tag_code(tag) << 2;
        if change {
            flags |= F_CHANGE;
        }
        if rec.coinbase {
            flags |= F_COINBASE;
        }
        if rec.tx.expiry != 0 {
            flags |= F_EXPIRY;
        }
        let script_off = u32::try_from(rec.locs[index as usize]).expect("offset");
        let script_len = u16::try_from(out.pk_script.len()).expect("script length");
        let op = outpoint(&rec.hash, index);
        match mined {
            None => {
                if !has(&self.tx_unmined, &rec.hash)? {
                    return invalid("credit for an unmined tx that is not recorded");
                }
                if has(&self.unmined_credits, &op)? {
                    return Ok(false);
                }
                let unpublished = has(&self.unpublished, &rec.hash)?;
                let m = McVal {
                    amount: out.value,
                    flags,
                    account,
                    class: unmined_class(tag, rec.coinbase, unpublished),
                    script_kind,
                    script_off,
                    script_len,
                };
                self.put_mc(&op, &m)?;
            }
            Some((height, seq)) => {
                let ck = cd_key(&rec.hash, height, index);
                let c = CreditVal {
                    amount: out.value,
                    flags,
                    account,
                    script_off,
                    script_len,
                    script_kind,
                    spender: None,
                };
                if rec.ty == TxType::Regular && self.block_invalid(height)? {
                    self.credits_invalidated
                        .insert(ck.as_slice(), c.encode().as_slice())?;
                    return Ok(true);
                }
                if has(&self.credits, &ck)? {
                    return Ok(false);
                }
                self.put_credit(&ck, &c)?;
                self.put_unspent(&op, &c.unspent(height))?;
                self.acct_history(account, height, seq, &rec.hash)?;
            }
        }
        Ok(true)
    }

    /// `AddTicketCommitment`.
    fn add_ticket_commitment(&mut self, rec: &Rec<'_>, index: u32, account: u32) -> Result<()> {
        let op = outpoint(&rec.hash, index);
        if has(&self.commitments, &op)? {
            return Ok(());
        }
        let Some((_, amount)) = chain::decode_commitment(&rec.tx.tx_out[index as usize].pk_script)
        else {
            return invalid("bad commitment script");
        };
        let mut v = [0; 12];
        v[..8].copy_from_slice(&amount.to_be_bytes());
        v[8..].copy_from_slice(&account.to_be_bytes());
        self.commitments.insert(op.as_slice(), v.as_slice())?;
        self.put_cmu(&op, 0)
    }

    /// The commitment outpoints of the ticket a vote spends.
    fn vote_commitments(tx: &MsgTx) -> Vec<[u8; 36]> {
        let ticket = tx.tx_in[1].previous_out_point.hash.0;
        let outs = u32::try_from(tx.tx_out.len()).expect("outputs");
        let count = (outs - 2) * 2 + 1;
        (1..count)
            .step_by(2)
            .map(|i| outpoint(&ticket, i))
            .collect()
    }

    /// `RedeemTicketCommitments` for a vote.
    fn redeem_ticket_commitments(&mut self, rec: &Rec<'_>, mined: bool) -> Result<()> {
        for op in Self::vote_commitments(rec.tx) {
            if !has(&self.commitments, &op)? {
                continue;
            }
            if mined {
                self.del_cmu(&op)?;
            } else {
                self.put_cmu(&op, 1)?;
            }
        }
        Ok(())
    }

    /// The regular transactions of the block at `height`, with their
    /// positions and decoded records.
    fn regular_txs(&self, height: u32) -> Result<Vec<(u32, [u8; 32], MsgTx)>> {
        let mut out = Vec::new();
        for (seq, hash) in self.block_txs(height)? {
            let Some(rv) = get(&self.tx_mined, &rec_key(&hash, height))? else {
                return invalid(format!("missing transaction record at height {height}"));
            };
            let (tx, _) = MsgTx::from_bytes(&rv[12..])
                .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))?;
            if chain::tx_type(&tx) == TxType::Regular {
                out.push((seq, hash, tx));
            }
        }
        Ok(out)
    }

    fn set_invalid_flag(&mut self, height: u32, invalid_flag: bool) -> Result<bool> {
        let mut b = self.block_row(height)?;
        if (b[42] != 0) == invalid_flag {
            return Ok(false);
        }
        b[42] = u8::from(invalid_flag);
        self.blocks
            .insert(height.to_be_bytes().as_slice(), b.as_slice())?;
        Ok(true)
    }

    /// `stakeInvalidate` (txmined.go:767): marks the block invalid and, for
    /// each of its regular transactions, moves its credits to
    /// `CREDITS_INVALIDATED` (removing their unspent outputs) and its
    /// debits to `DEBITS_INVALIDATED`, marking the credits they spent
    /// unspent again and restoring their unspent outputs. The record stays
    /// under this block, and history keeps listing it; per-account history
    /// drops the accounts it no longer touches.
    pub fn stake_invalidate(&mut self, height: u32) -> Result<()> {
        if !self.set_invalid_flag(height, true)? {
            return Ok(());
        }
        for (seq, hash, tx) in self.regular_txs(height)? {
            let before = self.acct_set(&hash, height)?;
            for i in 0..tx.tx_out.len() {
                let i = u32::try_from(i).expect("index");
                let ck = cd_key(&hash, height, i);
                let Some(v) = self
                    .credits
                    .remove(ck.as_slice())?
                    .map(|g| g.value().to_vec())
                else {
                    continue;
                };
                self.credits_invalidated
                    .insert(ck.as_slice(), v.as_slice())?;
                self.del_unspent(&outpoint(&hash, i))?;
            }
            for i in 0..tx.tx_in.len() {
                let i = u32::try_from(i).expect("index");
                let dk = cd_key(&hash, height, i);
                let Some(dv) = self
                    .debits
                    .remove(dk.as_slice())?
                    .map(|g| g.value().to_vec())
                else {
                    continue;
                };
                let ck: [u8; 40] = dv[8..48].try_into().expect("credit key");
                let Some(c) = self.unspend_credit(&ck)? else {
                    // dcrwallet restores an unspent entry for a credit it
                    // has just moved to `ic` (txmined.go:857-878).
                    return invalid(
                        "a disapproved block spends a credit it also creates; not implemented",
                    );
                };
                self.debits_invalidated
                    .insert(dk.as_slice(), dv.as_slice())?;
                self.put_unspent(&cd_outpoint(&ck), &c.unspent(be32(&ck, 32)))?;
            }
            let after = self.acct_set(&hash, height)?;
            self.set_acct_history(height, seq, &hash, &before, &after)?;
        }
        Ok(())
    }

    /// `stakeValidate` (txmined.go:631): the reverse of
    /// [`W::stake_invalidate`], run when a block approving a parent marked
    /// invalid is attached (after a reorganisation replaced the
    /// disapproving child). Like dcrwallet it writes an unspent output for
    /// every credit it moves back, even one whose transaction a rollback
    /// has meanwhile put back among the unmined (with its own unmined
    /// credit at the same outpoint): both tables then hold the outpoint,
    /// and both count in the balance, as in dcrwallet.
    pub fn stake_validate(&mut self, height: u32) -> Result<()> {
        if !self.set_invalid_flag(height, false)? {
            return Ok(());
        }
        for (seq, hash, tx) in self.regular_txs(height)? {
            let before = self.acct_set(&hash, height)?;
            for i in 0..tx.tx_out.len() {
                let i = u32::try_from(i).expect("index");
                let ck = cd_key(&hash, height, i);
                let Some(v) = self
                    .credits_invalidated
                    .remove(ck.as_slice())?
                    .map(|g| g.value().to_vec())
                else {
                    continue;
                };
                let Some(c) = CreditVal::decode(&v) else {
                    return invalid("bad invalidated credit");
                };
                self.credits.insert(ck.as_slice(), v.as_slice())?;
                self.put_unspent(&outpoint(&hash, i), &c.unspent(height))?;
            }
            for i in 0..tx.tx_in.len() {
                let i = u32::try_from(i).expect("index");
                let dk = cd_key(&hash, height, i);
                let Some(dv) = self
                    .debits_invalidated
                    .remove(dk.as_slice())?
                    .map(|g| g.value().to_vec())
                else {
                    continue;
                };
                let ck: [u8; 40] = dv[8..48].try_into().expect("credit key");
                if self.credit(&ck)?.is_none() {
                    // dcrwallet's spendCredit writes a zero-filled credit
                    // here (txmined.go:728, txdb.go:942).
                    return invalid("stake validation of a debit whose credit is gone");
                }
                self.spend_credit(&ck, &hash, height, i)?;
                self.del_unspent(&cd_outpoint(&ck))?;
                self.debits.insert(dk.as_slice(), dv.as_slice())?;
            }
            let after = self.acct_set(&hash, height)?;
            self.set_acct_history(height, seq, &hash, &before, &after)?;
        }
        Ok(())
    }

    /// `Rollback` (txmined.go:1874): removes every block from `height` to
    /// the tip, moving each non-coinbase transaction to the unmined set
    /// (its inputs to `UNMINED_INPUTS`, its debits undone, its credits to
    /// `UNMINED_CREDITS`), deleting coinbase credits outright, and marking
    /// a vote's commitments spent by an unmined vote. Like dcrwallet it
    /// does not validate the block at `height - 1`. Unlike dcrwallet, which
    /// leaves the invalidated credits and debits of a detached block in
    /// `ic` and `id` under that block's hash, where nothing reads them
    /// again, it deletes them, as it deletes the detached headers.
    pub fn rollback(&mut self, height: u32) -> Result<()> {
        if height == 0 {
            return invalid("cannot rollback the genesis block");
        }
        if height > self.tip_height {
            return invalid(format!("rollback to {height} above the tip"));
        }
        let mut accounts = Vec::new();
        for e in self.accounts.iter()? {
            let (k, _) = e?;
            accounts.push(u32::from_le_bytes(k.value().try_into().expect("4 bytes")));
        }
        let mut removed: HashMap<[u8; 40], CreditVal> = HashMap::new();
        let mut coinbase_ops = Vec::new();
        for h in (height..=self.tip_height).rev() {
            let txs = self.block_txs(h)?;
            for (_, hash) in &txs {
                let Some(rv) = self
                    .tx_mined
                    .remove(rec_key(hash, h).as_slice())?
                    .map(|g| g.value().to_vec())
                else {
                    return invalid(format!("missing transaction record at height {h}"));
                };
                let (tx, _) = MsgTx::from_bytes(&rv[12..])
                    .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))?;
                let ty = chain::tx_type(&tx);
                if chain::is_coinbase(&tx) {
                    for i in 0..tx.tx_out.len() {
                        let i = u32::try_from(i).expect("index");
                        let ck = cd_key(hash, h, i);
                        let Some(c) = self.credit(&ck)? else {
                            continue;
                        };
                        let op = outpoint(hash, i);
                        coinbase_ops.push(op);
                        if has(&self.unspent, &op)? {
                            self.del_unspent(&op)?;
                        }
                        self.credits.remove(ck.as_slice())?;
                        removed.insert(ck, c);
                    }
                    continue;
                }
                self.tx_unmined.insert(hash.as_slice(), &rv[4..])?;
                for (i, input) in tx.tx_in.iter().enumerate() {
                    if i == 0 && ty == TxType::Vote {
                        continue;
                    }
                    let prev = &input.previous_out_point;
                    let op = outpoint(&prev.hash.0, prev.index);
                    self.put_mi(&op, hash)?;
                    let i = u32::try_from(i).expect("index");
                    let dk = cd_key(hash, h, i);
                    let Some(dv) = self
                        .debits
                        .remove(dk.as_slice())?
                        .map(|g| g.value().to_vec())
                    else {
                        continue;
                    };
                    let ck: [u8; 40] = dv[8..48].try_into().expect("credit key");
                    if !has(&self.credits, &ck)? && !removed.contains_key(&ck) {
                        return invalid("rollback: debit of a missing credit");
                    }
                    let Some(c) = self.unspend_credit(&ck)? else {
                        continue;
                    };
                    self.put_unspent(&op, &c.unspent(be32(&ck, 32)))?;
                }
                let unpublished = has(&self.unpublished, hash)?;
                for i in 0..tx.tx_out.len() {
                    let i = u32::try_from(i).expect("index");
                    let ck = cd_key(hash, h, i);
                    let Some(c) = self.credit(&ck)? else {
                        continue;
                    };
                    self.credits.remove(ck.as_slice())?;
                    removed.insert(ck, c);
                    let op = outpoint(hash, i);
                    let m = McVal {
                        amount: c.amount,
                        flags: c.flags & !F_SPENT,
                        account: c.account,
                        class: unmined_class(
                            flags_tag(c.flags),
                            c.flags & F_COINBASE != 0,
                            unpublished,
                        ),
                        script_kind: c.script_kind,
                        script_off: c.script_off,
                        script_len: c.script_len,
                    };
                    self.put_mc(&op, &m)?;
                    if has(&self.unspent, &op)? {
                        self.del_unspent(&op)?;
                    }
                }
                if ty == TxType::Vote {
                    for op in Self::vote_commitments(&tx) {
                        if has(&self.commitments, &op)? {
                            self.put_cmu(&op, 1)?;
                        }
                    }
                }
            }
            for (seq, hash) in &txs {
                self.history.remove(hist_key(h, *seq).as_slice())?;
                let lo = cd_key(hash, h, 0);
                let hi = cd_key(hash, h, u32::MAX);
                self.credits_invalidated
                    .retain_in(lo.as_slice()..=hi.as_slice(), |_, _| false)?;
                self.debits_invalidated
                    .retain_in(lo.as_slice()..=hi.as_slice(), |_, _| false)?;
            }
            for a in &accounts {
                let lo = acct_hist_key(*a, h, 0);
                let hi = acct_hist_key(*a, h, u32::MAX);
                self.acct_history
                    .retain_in(lo.as_slice()..=hi.as_slice(), |_, _| false)?;
            }
            let b = self.block_row(h)?;
            let hk = h.to_be_bytes();
            self.blocks.remove(hk.as_slice())?;
            self.block_by_hash.remove(&b[..32])?;
            self.headers.remove(hk.as_slice())?;
            self.cfilters.remove(hk.as_slice())?;
        }
        for op in coinbase_ops {
            if has(&self.unmined_inputs, &op)? {
                return invalid("removing the unmined spenders of a coinbase is not implemented");
            }
        }
        let b = self.block_row(height - 1)?;
        self.tip_height = height - 1;
        self.tip_hash = hash32(&b);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Read side

macro_rules! lazy_table {
    ($($f:ident => $def:ident),*) => {
        /// A read transaction; each table is opened on first use, so a
        /// query pays only for the tables it touches.
        pub struct R {
            txn: ReadTransaction,
            $($f: OnceCell<ReadOnlyTable<K, K>>,)*
        }

        impl R {
            fn open(txn: ReadTransaction) -> Result<R> {
                Ok(R { txn, $($f: OnceCell::new(),)* })
            }
            $(
                #[doc = concat!("The `", stringify!($def), "` table.")]
                pub fn $f(&self) -> Result<&ReadOnlyTable<K, K>> {
                    if let Some(t) = self.$f.get() {
                        return Ok(t);
                    }
                    let t = self.txn.open_table($def)?;
                    Ok(self.$f.get_or_init(|| t))
                }
            )*
        }
    };
}

lazy_table!(
    meta => META,
    accounts => ACCOUNTS,
    account_names => ACCOUNT_NAMES,
    addr_by_hash => ADDR_BY_HASH,
    addr_by_path => ADDR_BY_PATH,
    addr_by_acct_sha => ADDR_BY_ACCT_SHA,
    headers => HEADERS,
    blocks => BLOCKS,
    block_by_hash => BLOCK_BY_HASH,
    cfilters => CFILTERS,
    tx_mined => TX_MINED,
    tx_unmined => TX_UNMINED,
    unpublished => UNPUBLISHED,
    history => HISTORY,
    acct_history => ACCT_HISTORY,
    credits => CREDITS,
    credits_invalidated => CREDITS_INVALIDATED,
    unspent => UNSPENT,
    unmined_credits => UNMINED_CREDITS,
    unmined_inputs => UNMINED_INPUTS,
    debits => DEBITS,
    debits_invalidated => DEBITS_INVALIDATED,
    tickets => TICKETS,
    commitments => COMMITMENTS,
    commitments_unspent => COMMITMENTS_UNSPENT,
    balances => BALANCES,
    balance_heights => BALANCE_HEIGHTS
);

/// Balances by category, as `udb.Balances` has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Balances {
    /// Spendable.
    pub spendable: i64,
    /// ImmatureCoinbaseRewards.
    pub immature_coinbase: i64,
    /// ImmatureStakeGeneration.
    pub immature_stake: i64,
    /// LockedByTickets.
    pub locked: i64,
    /// VotingAuthority.
    pub voting_authority: i64,
    /// Unconfirmed.
    pub unconfirmed: i64,
    /// Total.
    pub total: i64,
}

/// One transaction as `udb.TxDetails` reports it.
#[derive(Clone, Debug)]
pub struct TxDetails {
    /// Hash.
    pub hash: [u8; 32],
    /// Height, -1 for unmined.
    pub height: i32,
    /// The decoded transaction.
    pub tx: MsgTx,
    /// (index, amount, spent, change).
    pub credits: Vec<(u32, i64, bool, bool)>,
    /// (index, amount).
    pub debits: Vec<(u32, i64)>,
}

impl R {
    /// Copies every table of this snapshot into a new store at `dest`, in
    /// one write transaction: a consistent backup taken while the source
    /// stays open for writing (redb readers see the snapshot their
    /// transaction began with).
    pub fn copy_to(&self, dest: &Path) -> Result<()> {
        let db = Database::create(dest)?;
        let txn = db.begin_write()?;
        for def in ALL_TABLES {
            let src = self.txn.open_table(*def)?;
            let mut dst = txn.open_table(*def)?;
            for e in src.iter()? {
                let (k, v) = e?;
                dst.insert(k.value(), v.value())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// The tip: (height, hash).
    pub fn tip(&self) -> Result<(u32, [u8; 32])> {
        let v = get(self.meta()?, b"tip")?.ok_or_else(|| Error::Invalid("no tip".into()))?;
        Ok((be32(&v, 0), v[4..36].try_into().expect("hash")))
    }

    /// Workload operations applied.
    pub fn last_op(&self) -> Result<u64> {
        Ok(get(self.meta()?, b"last_op")?.map_or(0, |v| be64(&v, 0) as u64))
    }

    /// The maintained `UNSPENT` key count (bbolt's `KeyN`).
    pub fn unspent_n(&self) -> Result<u64> {
        Ok(get(self.meta()?, b"unspent_n")?.map_or(0, |v| be64(&v, 0) as u64))
    }

    /// Accounts in key order, with their counters.
    pub fn account_list(&self) -> Result<Vec<(u32, AcctVars)>> {
        let mut out = Vec::new();
        for e in self.accounts()?.iter()? {
            let (k, v) = e?;
            let n = u32::from_le_bytes(k.value().try_into().expect("4 bytes"));
            out.push((n, AcctVars::decode(v.value())));
        }
        Ok(out)
    }

    /// The (account, branch, index) of an address, by HASH160.
    pub fn address(&self, hash: &[u8; 20]) -> Result<Option<(u32, u8, u32)>> {
        Ok(get(self.addr_by_hash()?, hash)?.map(|a| (be32(&a, 0), a[4], be32(&a, 5))))
    }

    /// Whether `(account, branch, index)` is recorded.
    pub fn has_path(&self, account: u32, branch: u8, index: u32) -> Result<bool> {
        has(self.addr_by_path()?, &path_key(account, branch, index))
    }

    /// An account's addresses in `ForEachAccountAddress` order.
    pub fn account_addresses(&self, account: u32) -> Result<Vec<[u8; 20]>> {
        let mut lo = [0u8; 36];
        lo[..4].copy_from_slice(&account.to_le_bytes());
        let mut hi = [0xffu8; 36];
        hi[..4].copy_from_slice(&account.to_le_bytes());
        let mut out = Vec::new();
        for e in self
            .addr_by_acct_sha()?
            .range(lo.as_slice()..=hi.as_slice())?
        {
            let (_, v) = e?;
            out.push(v.value().try_into().expect("20 bytes"));
        }
        Ok(out)
    }

    /// The heights of the blocks marked invalid, with their hashes.
    pub fn invalid_blocks(&self) -> Result<Vec<(u32, [u8; 32])>> {
        let mut out = Vec::new();
        for e in self.blocks()?.iter()? {
            let (k, v) = e?;
            if v.value()[42] != 0 {
                out.push((be32(k.value(), 0), hash32(v.value())));
            }
        }
        Ok(out)
    }

    /// The invalidated credits: (hash, height, index, amount, spent).
    pub fn invalidated_credits(&self) -> Result<Vec<InvalidatedCredit>> {
        let mut out = Vec::new();
        for e in self.credits_invalidated()?.iter()? {
            let (k, v) = e?;
            let (k, v) = (k.value(), v.value());
            out.push((
                hash32(k),
                be32(k, 32),
                be32(k, 36),
                be64(v, 0),
                v[8] & F_SPENT != 0,
            ));
        }
        Ok(out)
    }

    /// The invalidated debits: (hash, height, input, amount, spent
    /// outpoint).
    pub fn invalidated_debits(&self) -> Result<Vec<InvalidatedDebit>> {
        let mut out = Vec::new();
        for e in self.debits_invalidated()?.iter()? {
            let (k, v) = e?;
            let (k, v) = (k.value(), v.value());
            out.push((
                hash32(k),
                be32(k, 32),
                be32(k, 36),
                be64(v, 0),
                cd_outpoint(&v[8..48]),
            ));
        }
        Ok(out)
    }

    /// The last recorded child of an account branch.
    pub fn last_path(&self, account: u32, branch: u8) -> Result<Option<u32>> {
        let lo = path_key(account, branch, 0);
        let hi = path_key(account, branch, u32::MAX);
        let mut r = self.addr_by_path()?.range(lo.as_slice()..=hi.as_slice())?;
        Ok(match r.next_back() {
            Some(e) => Some(be32(e?.0.value(), 5)),
            None => None,
        })
    }

    /// The sum of a depth-dependent class over heights `from..=tip`: the
    /// per-height rows inside the window, or, when the window spans more
    /// heights than lie below it, the class total minus the rows below it.
    /// Either way a query reads at most half the chain's heights' rows, and
    /// at the depths the wallet asks for (1 to 16 blocks) a handful.
    fn window(&self, account: u32, class: u8, from: i64, tip: i64, total: i64) -> Result<i64> {
        if from <= 0 {
            return Ok(total);
        }
        let from = u32::try_from(from).unwrap_or(u32::MAX);
        let sum_range = |lo: u32, hi: u32| -> Result<i64> {
            let lo = bal_height_key(account, class, lo);
            let hi = bal_height_key(account, class, hi);
            let mut sum = 0i64;
            for e in self
                .balance_heights()?
                .range(lo.as_slice()..=hi.as_slice())?
            {
                let (_, v) = e?;
                sum = sum.wrapping_add(be64(v.value(), 0));
            }
            Ok(sum)
        };
        if tip - i64::from(from) < i64::from(from) {
            sum_range(from, u32::MAX)
        } else {
            Ok(total.wrapping_sub(sum_range(0, from - 1)?))
        }
    }

    /// The balances of one account at `min_conf` with the tip at `tip`
    /// (`None` if the account owns nothing the full scan would visit),
    /// from the maintained sums: a handful of gets plus the per-height
    /// rows inside the depth windows.
    pub fn account_balance(
        &self,
        account: u32,
        min_conf: i32,
        tip: u32,
    ) -> Result<Option<Balances>> {
        let lo = bal_key(account, 0);
        let hi = bal_key(account, u8::MAX);
        let mut present = false;
        let mut b = Balances::default();
        let tip = i64::from(tip);
        let mut rows = Vec::new();
        for e in self.balances()?.range(lo.as_slice()..=hi.as_slice())? {
            let (k, v) = e?;
            rows.push((k.value()[4], be64(v.value(), 0), be64(v.value(), 8)));
        }
        let maturity = i64::from(chain::COINBASE_MATURITY);
        for (c, sum, n) in rows {
            if n != 0 {
                present = true;
            }
            match c {
                class::M_REG => {
                    let unconf = if min_conf <= 0 {
                        0
                    } else {
                        self.window(account, c, tip + 2 - i64::from(min_conf), tip, sum)?
                    };
                    b.spendable += sum - unconf;
                    b.total += sum;
                }
                class::M_CB => {
                    let immature = self.window(account, c, tip + 1 - maturity, tip, sum)?;
                    b.spendable += sum - immature;
                    b.immature_coinbase += immature;
                    b.total += sum;
                }
                class::M_STAKEGEN => {
                    let immature = self.window(account, c, tip + 1 - maturity, tip, sum)?;
                    b.spendable += sum - immature;
                    b.immature_stake += immature;
                    b.total += sum;
                }
                class::M_TCHANGE => {
                    let m = i64::from(chain::SSTX_CHANGE_MATURITY);
                    let immature = self.window(account, c, tip + 1 - m, tip, sum)?;
                    b.spendable += sum - immature;
                    b.total += sum;
                }
                class::VOTEAUTH => b.voting_authority += sum,
                class::U_REG_PUB => {
                    if min_conf == 0 {
                        b.spendable += sum;
                    } else {
                        b.unconfirmed += sum;
                    }
                    b.total += sum;
                }
                class::U_REG_UNPUB => {
                    b.unconfirmed += sum;
                    b.total += sum;
                }
                class::U_CB_PUB => {
                    if min_conf == 0 {
                        b.spendable += sum;
                    }
                    b.total += sum;
                }
                class::U_CB_UNPUB | class::U_TCHANGE => b.total += sum,
                class::U_STAKEGEN => {
                    b.immature_stake += sum;
                    b.total += sum;
                }
                class::U_TGEN => {}
                class::LOCKED => {
                    b.locked += sum;
                    b.total += sum;
                }
                _ => return invalid(format!("unknown balance class {c}")),
            }
        }
        Ok(present.then_some(b))
    }

    /// Every account's balances (the accounts present in `BALANCES`).
    pub fn account_balances(&self, min_conf: i32, tip: u32) -> Result<BTreeMap<u32, Balances>> {
        let mut accounts = Vec::new();
        for e in self.balances()?.iter()? {
            let (k, _) = e?;
            let a = be32(k.value(), 0);
            if accounts.last() != Some(&a) {
                accounts.push(a);
            }
        }
        let mut out = BTreeMap::new();
        for a in accounts {
            if let Some(b) = self.account_balance(a, min_conf, tip)? {
                out.insert(a, b);
            }
        }
        Ok(out)
    }

    /// The pkScript of a mined credit of the record at (hash, height).
    pub fn mined_script(&self, hash: &[u8], height: u32, off: u32, len: u16) -> Result<Vec<u8>> {
        let Some(g) = self
            .tx_mined()?
            .get(rec_key(&hash32(hash), height).as_slice())?
        else {
            return invalid("missing mined transaction for credit");
        };
        let start = 12 + off as usize;
        let v = g.value();
        v.get(start..start + len as usize)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| Error::Invalid("script out of range".into()))
    }

    /// The pkScript of an unmined credit.
    pub fn unmined_script(&self, hash: &[u8], off: u32, len: u16) -> Result<Vec<u8>> {
        let Some(g) = self.tx_unmined()?.get(hash)? else {
            return invalid("missing unmined transaction for credit");
        };
        let start = 8 + off as usize;
        let v = g.value();
        v.get(start..start + len as usize)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| Error::Invalid("script out of range".into()))
    }

    /// `UnspentOutputs`: every mined unspent output not spent by an
    /// unmined transaction, in key order, then every unmined credit not so
    /// spent and not from an unpublished transaction, in key order.
    pub fn unspent_outputs(&self) -> Result<Vec<Utxo>> {
        let mut out = Vec::new();
        for e in self.unspent()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            if has(self.unmined_inputs()?, k)? {
                continue;
            }
            let u = UnspentVal::decode(v.value())
                .ok_or_else(|| Error::Invalid("bad unspent value".into()))?;
            // outputCreditInfo (txmined.go:2244) prefers an unmined credit
            // at the same outpoint, which exists when stake validation
            // restored the output of a transaction a rollback had made
            // unmined: the entry is then reported as unmined, here and
            // again in the unmined walk below.
            if let Some(m) = get(self.unmined_credits()?, k)?
                .as_deref()
                .and_then(McVal::decode)
            {
                out.push(Utxo {
                    outpoint: k.try_into().expect("36 bytes"),
                    tree: i8::from(flags_tag(m.flags) != OP_NONSTAKE),
                    height: -1,
                    amount: m.amount,
                    coinbase: false,
                    script: self.unmined_script(&k[..32], m.script_off, m.script_len)?,
                });
                continue;
            }
            let tag = flags_tag(u.flags);
            out.push(Utxo {
                outpoint: k.try_into().expect("36 bytes"),
                tree: i8::from(tag != OP_NONSTAKE),
                height: u.height as i32,
                amount: u.amount,
                coinbase: u.flags & F_COINBASE != 0,
                script: self.mined_script(&k[..32], u.height, u.script_off, u.script_len)?,
            });
        }
        for e in self.unmined_credits()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            if has(self.unmined_inputs()?, k)? || has(self.unpublished()?, &k[..32])? {
                continue;
            }
            let m =
                McVal::decode(v.value()).ok_or_else(|| Error::Invalid("bad mc value".into()))?;
            let tag = flags_tag(m.flags);
            out.push(Utxo {
                outpoint: k.try_into().expect("36 bytes"),
                tree: i8::from(tag != OP_NONSTAKE),
                height: -1,
                amount: m.amount,
                coinbase: false,
                script: self.unmined_script(&k[..32], m.script_off, m.script_len)?,
            });
        }
        Ok(out)
    }

    /// `ForEachUnspentOutpoint`: the same walk, outpoints only.
    pub fn for_each_unspent_outpoint(&self, mut f: impl FnMut(&[u8], i8)) -> Result<()> {
        for e in self.unspent()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            if has(self.unmined_inputs()?, k)? {
                continue;
            }
            let tag = flags_tag(v.value()[16]);
            f(k, i8::from(tag != OP_NONSTAKE));
        }
        for e in self.unmined_credits()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            if has(self.unmined_inputs()?, k)? || has(self.unpublished()?, &k[..32])? {
                continue;
            }
            let tag = flags_tag(v.value()[8]);
            f(k, i8::from(tag != OP_NONSTAKE));
        }
        Ok(())
    }

    fn mined_details(&self, hash: &[u8; 32], height: u32) -> Result<TxDetails> {
        let Some(g) = self.tx_mined()?.get(rec_key(hash, height).as_slice())? else {
            return invalid("missing transaction");
        };
        let v = g.value();
        let (tx, _) = MsgTx::from_bytes(&v[12..])
            .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))?;
        drop(g);
        let lo = cd_key(hash, height, 0);
        let hi = cd_key(hash, height, u32::MAX);
        let mut credits = Vec::new();
        for e in self.credits()?.range(lo.as_slice()..=hi.as_slice())? {
            let (k, cv) = e?;
            let k = k.value();
            let cv = cv.value();
            let index = be32(k, 36);
            let spent = cv[8] & F_SPENT != 0 || has(self.unmined_inputs()?, &cd_outpoint(k))?;
            credits.push((index, be64(cv, 0), spent, cv[8] & F_CHANGE != 0));
        }
        let mut debits = Vec::new();
        for e in self.debits()?.range(lo.as_slice()..=hi.as_slice())? {
            let (k, dv) = e?;
            debits.push((be32(k.value(), 36), be64(dv.value(), 0)));
        }
        Ok(TxDetails {
            hash: *hash,
            height: height as i32,
            tx,
            credits,
            debits,
        })
    }

    /// The highest height at which `hash` has a mined record
    /// (`latestTxRecord`).
    pub fn latest_mined(&self, hash: &[u8; 32]) -> Result<Option<u32>> {
        let lo = rec_key(hash, 0);
        let hi = rec_key(hash, u32::MAX);
        let mut r = self.tx_mined()?.range(lo.as_slice()..=hi.as_slice())?;
        Ok(match r.next_back() {
            Some(e) => Some(be32(e?.0.value(), 32)),
            None => None,
        })
    }

    fn unmined_details(&self, hash: &[u8; 32], v: &[u8]) -> Result<TxDetails> {
        let (tx, _) = MsgTx::from_bytes(&v[8..])
            .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))?;
        let lo = outpoint(hash, 0);
        let hi = outpoint(hash, u32::MAX);
        let mut credits = Vec::new();
        for e in self
            .unmined_credits()?
            .range(lo.as_slice()..=hi.as_slice())?
        {
            let (k, mv) = e?;
            let k = k.value();
            let mv = mv.value();
            let spent = has(self.unmined_inputs()?, k)?;
            credits.push((be32(k, 32), be64(mv, 0), spent, mv[8] & F_CHANGE != 0));
        }
        let mut debits = Vec::new();
        for (i, input) in tx.tx_in.iter().enumerate() {
            let prev = &input.previous_out_point;
            let op = outpoint(&prev.hash.0, prev.index);
            let amount = if let Some(g) = self.unspent()?.get(op.as_slice())? {
                Some(be64(g.value(), 8))
            } else {
                self.unmined_credits()?
                    .get(op.as_slice())?
                    .map(|g| be64(g.value(), 0))
            };
            if let Some(a) = amount {
                debits.push((u32::try_from(i).expect("index"), a));
            }
        }
        Ok(TxDetails {
            hash: *hash,
            height: -1,
            tx,
            credits,
            debits,
        })
    }

    /// `TxDetails` for one transaction.
    pub fn tx_details(&self, hash: &[u8; 32]) -> Result<Option<TxDetails>> {
        if let Some(v) = get(self.tx_unmined()?, hash)? {
            return Ok(Some(self.unmined_details(hash, &v)?));
        }
        if let Some(h) = self.latest_mined(hash)? {
            return Ok(Some(self.mined_details(hash, h)?));
        }
        Ok(None)
    }

    fn range_unmined(&self, f: &mut impl FnMut(&[TxDetails]) -> bool) -> Result<bool> {
        let mut details = Vec::new();
        for e in self.tx_unmined()?.iter()? {
            let (k, v) = e?;
            let hash: [u8; 32] = k
                .value()
                .try_into()
                .map_err(|_| Error::Invalid("bad unmined key".into()))?;
            details.push(self.unmined_details(&hash, v.value())?);
        }
        if details.is_empty() {
            return Ok(false);
        }
        Ok(f(&details))
    }

    /// `RangeTransactions` with dcrwallet's argument semantics (spec 04
    /// section 5.16), walking `HISTORY`, which holds wallet transactions
    /// only, so a range over mostly empty blocks costs only what it
    /// returns.
    pub fn range_transactions(
        &self,
        begin: i32,
        end: i32,
        mut f: impl FnMut(&[TxDetails]) -> bool,
    ) -> Result<()> {
        let mut added_unmined = false;
        if begin < 0 {
            if self.range_unmined(&mut f)? {
                return Ok(());
            }
            added_unmined = true;
        }
        let b = if begin < 0 { 0x7fff_ffff } else { begin as u32 };
        let e = if end < 0 { 0x7fff_ffff } else { end as u32 };
        let mut brk = false;
        let mut group: Vec<TxDetails> = Vec::new();
        let mut group_height = None;
        let flush = |group: &mut Vec<TxDetails>, f: &mut dyn FnMut(&[TxDetails]) -> bool| {
            if !group.is_empty() && f(group) {
                return true;
            }
            group.clear();
            false
        };
        if b < e {
            let lo = hist_key(b, 0);
            let hi = hist_key(e, u32::MAX);
            for item in self.history()?.range(lo.as_slice()..=hi.as_slice())? {
                let (k, v) = item?;
                let h = be32(k.value(), 0);
                if group_height != Some(h) {
                    if flush(&mut group, &mut f) {
                        brk = true;
                        break;
                    }
                    group_height = Some(h);
                }
                let hash: [u8; 32] = v.value().try_into().expect("hash");
                group.push(self.mined_details(&hash, h)?);
            }
        } else {
            let lo = hist_key(e, 0);
            let hi = hist_key(b, u32::MAX);
            let mut hashes: Vec<([u8; 32], u32)> = Vec::new();
            for item in self.history()?.range(lo.as_slice()..=hi.as_slice())?.rev() {
                let (k, v) = item?;
                let h = be32(k.value(), 0);
                if group_height != Some(h) {
                    hashes.reverse();
                    for (hash, hh) in hashes.drain(..) {
                        group.push(self.mined_details(&hash, hh)?);
                    }
                    if flush(&mut group, &mut f) {
                        brk = true;
                        break;
                    }
                    group_height = Some(h);
                }
                hashes.push((v.value().try_into().expect("hash"), h));
            }
            if !brk {
                hashes.reverse();
                for (hash, hh) in hashes.drain(..) {
                    group.push(self.mined_details(&hash, hh)?);
                }
            }
        }
        if !brk && flush(&mut group, &mut f) {
            brk = true;
        }
        if !brk && !added_unmined && end < 0 {
            self.range_unmined(&mut f)?;
        }
        Ok(())
    }

    /// The most recent `limit` mined transactions crediting or debiting
    /// `account`, in the order a backward `RangeTransactions` walk finds
    /// them (blocks descending, positions ascending within a block).
    pub fn by_account(&self, account: u32, limit: usize) -> Result<Vec<[u8; 32]>> {
        let lo = acct_hist_key(account, 0, 0);
        let hi = acct_hist_key(account, u32::MAX, u32::MAX);
        let mut out = Vec::new();
        let mut block: Vec<[u8; 32]> = Vec::new();
        let mut cur = None;
        for item in self
            .acct_history()?
            .range(lo.as_slice()..=hi.as_slice())?
            .rev()
        {
            let (k, v) = item?;
            let h = be32(k.value(), 4);
            if cur != Some(h) {
                block.reverse();
                for x in block.drain(..) {
                    if out.len() < limit {
                        out.push(x);
                    }
                }
                if out.len() >= limit {
                    return Ok(out);
                }
                cur = Some(h);
            }
            block.push(v.value().try_into().expect("hash"));
        }
        block.reverse();
        for x in block {
            if out.len() < limit {
                out.push(x);
            }
        }
        Ok(out)
    }

    /// What a wallet output is, re-derived from the transaction and the
    /// address tables rather than from any stored copy.
    fn derive_out(&self, tx: &MsgTx, coinbase: bool, index: u32) -> Result<Option<Derived>> {
        let Some(out) = tx.tx_out.get(index as usize) else {
            return Ok(None);
        };
        let (tag, hash, kind) = match chain::classify(&out.pk_script) {
            ScriptKind::PubKeyHash { tag, hash } => (tag, hash, 1u8),
            _ => return Ok(None),
        };
        let Some((account, branch, _)) = self.address(&hash)? else {
            return Ok(None);
        };
        Ok(Some(Derived {
            amount: out.value,
            tag,
            coinbase,
            account,
            change: branch == 1,
            script_off: u32::try_from(chain::pk_script_locs(tx)[index as usize]).expect("offset"),
            script_len: u16::try_from(out.pk_script.len()).expect("length"),
            script_kind: kind,
        }))
    }

    /// The consistency check. Every denormalised value is re-derived from
    /// what it copies and compared:
    ///
    /// - each credit (live and invalidated) and unmined credit against its
    ///   transaction's output and the address it pays (amount, account,
    ///   change bit, stake tag, coinbase bit, script location and kind);
    /// - each `unspent` value against its credit, and its balance class
    ///   against the class the transaction's output implies;
    /// - each live credit's spent state against the debit that spends it,
    ///   and each debit against its transaction's input and its credit;
    /// - the invalidated rows against their block's invalid flag;
    /// - `history`, `acct_history` and the blocks' transaction counts
    ///   against the mined records, credits and debits;
    /// - the commitments against their tickets' outputs;
    /// - the three address tables against each other, the block tables
    ///   against each other and the headers, the tip against the blocks;
    /// - `balances`, `balance_heights` and the unspent count against a
    ///   recomputation from the rows checked above.
    ///
    /// Returns a description of the first difference.
    pub fn check_balances(&self) -> Result<Option<String>> {
        macro_rules! fail {
            ($($a:tt)*) => { return Ok(Some(format!($($a)*))) };
        }
        let parse = |v: &[u8]| -> Result<MsgTx> {
            MsgTx::from_bytes(v)
                .map(|(t, _)| t)
                .map_err(|e| Error::Invalid(format!("bad transaction: {e:?}")))
        };
        // Mined records, cached one at a time: every walk below is in hash
        // order, so consecutive rows share a record.
        let mut last: Option<([u8; 36], u32, MsgTx)> = None;
        let mut record = |hash: &[u8; 32], height: u32| -> Result<Option<(u32, MsgTx)>> {
            let k = rec_key(hash, height);
            if let Some((lk, seq, tx)) = &last
                && *lk == k
            {
                return Ok(Some((*seq, tx.clone())));
            }
            let Some(v) = get(self.tx_mined()?, &k)? else {
                return Ok(None);
            };
            let tx = parse(&v[12..])?;
            last = Some((k, be32(&v, 0), tx.clone()));
            Ok(Some((be32(&v, 0), tx)))
        };
        let block_invalid = |h: u32| -> Result<Option<bool>> {
            Ok(get(self.blocks()?, &h.to_be_bytes())?.map(|b| b[42] != 0))
        };
        let same = |c: &CreditVal, d: &Derived| {
            c.amount == d.amount
                && c.account == d.account
                && (c.flags & F_CHANGE != 0) == d.change
                && flags_tag(c.flags) == d.tag
                && (c.flags & F_COINBASE != 0) == d.coinbase
                && c.script_off == d.script_off
                && c.script_len == d.script_len
                && c.script_kind == d.script_kind
        };

        // Live credits.
        let mut want_acct: BTreeSet<[u8; 12]> = BTreeSet::new();
        for e in self.credits()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            let (hash, height, index) = (hash32(k), be32(k, 32), be32(k, 36));
            let Some(c) = CreditVal::decode(v.value()) else {
                fail!("bad credit value at {}", purse_testutil::hex(k));
            };
            let Some((seq, tx)) = record(&hash, height)? else {
                fail!("credit {} without a mined record", purse_testutil::hex(k));
            };
            let regular = chain::tx_type(&tx) == TxType::Regular;
            let Some(d) = self.derive_out(&tx, chain::is_coinbase(&tx), index)? else {
                fail!("credit {} pays no wallet address", purse_testutil::hex(k));
            };
            if !same(&c, &d) {
                fail!(
                    "credit {} differs from its output: {c:?} vs {d:?}",
                    purse_testutil::hex(k)
                );
            }
            if regular && block_invalid(height)? != Some(false) {
                fail!(
                    "live credit {} in a block marked invalid",
                    purse_testutil::hex(k)
                );
            }
            want_acct.insert(acct_hist_key(c.account, height, seq));
            let op = cd_outpoint(k);
            match c.spender {
                Some((sh, shh, si)) => {
                    let dv = get(self.debits()?, &cd_key(&sh, shh, si))?;
                    if dv.as_deref().map(|d| &d[8..48]) != Some(k) {
                        fail!("spent credit {} without its debit", purse_testutil::hex(k));
                    }
                }
                None => {
                    let want = c.unspent(height);
                    let Some(u) = get(self.unspent()?, &op)?
                        .as_deref()
                        .and_then(UnspentVal::decode)
                    else {
                        fail!(
                            "unspent credit {} has no unspent output",
                            purse_testutil::hex(k)
                        );
                    };
                    if u.encode() != want.encode() || u.class != mined_class(d.tag, d.coinbase) {
                        fail!(
                            "unspent output {} differs from its credit",
                            purse_testutil::hex(&op)
                        );
                    }
                }
            }
        }
        // Unspent outputs point at unspent credits.
        let mut n = 0u64;
        for e in self.unspent()?.iter()? {
            let (k, v) = e?;
            n += 1;
            let k = k.value();
            let Some(u) = UnspentVal::decode(v.value()) else {
                fail!("bad unspent value at {}", purse_testutil::hex(k));
            };
            let ck = cd_key(&hash32(k), u.height, be32(k, 32));
            match get(self.credits()?, &ck)?
                .as_deref()
                .and_then(CreditVal::decode)
            {
                Some(c) if c.spender.is_none() => {}
                _ => fail!(
                    "unspent output {} without an unspent credit",
                    purse_testutil::hex(k)
                ),
            }
        }
        // Invalidated credits.
        for e in self.credits_invalidated()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            let (hash, height, index) = (hash32(k), be32(k, 32), be32(k, 36));
            let Some(c) = CreditVal::decode(v.value()) else {
                fail!("bad invalidated credit at {}", purse_testutil::hex(k));
            };
            let Some((_, tx)) = record(&hash, height)? else {
                fail!(
                    "invalidated credit {} without a record",
                    purse_testutil::hex(k)
                );
            };
            if chain::tx_type(&tx) != TxType::Regular || block_invalid(height)? != Some(true) {
                fail!(
                    "invalidated credit {} outside a disapproved regular tree",
                    purse_testutil::hex(k)
                );
            }
            match self.derive_out(&tx, chain::is_coinbase(&tx), index)? {
                Some(d) if same(&c, &d) => {}
                d => fail!(
                    "invalidated credit {} differs from its output: {d:?}",
                    purse_testutil::hex(k)
                ),
            }
        }
        // Debits, live and invalidated.
        for (table, live) in [(self.debits()?, true), (self.debits_invalidated()?, false)] {
            for e in table.iter()? {
                let (k, v) = e?;
                let (k, v) = (k.value(), v.value());
                let (hash, height, input) = (hash32(k), be32(k, 32), be32(k, 36));
                let Some((seq, tx)) = record(&hash, height)? else {
                    fail!("debit {} without a mined record", purse_testutil::hex(k));
                };
                let Some(txin) = tx.tx_in.get(input as usize) else {
                    fail!("debit {} names a missing input", purse_testutil::hex(k));
                };
                let ck = &v[8..48];
                let prev = &txin.previous_out_point;
                if cd_outpoint(ck) != outpoint(&prev.hash.0, prev.index) {
                    fail!("debit {} names another outpoint", purse_testutil::hex(k));
                }
                let regular = chain::tx_type(&tx) == TxType::Regular;
                let credit = get(self.credits()?, ck)?
                    .as_deref()
                    .and_then(CreditVal::decode);
                if live {
                    if regular && block_invalid(height)? != Some(false) {
                        fail!(
                            "live debit {} in a block marked invalid",
                            purse_testutil::hex(k)
                        );
                    }
                    match credit {
                        Some(c)
                            if c.spender == Some((hash, height, input))
                                && c.amount == be64(v, 0) =>
                        {
                            want_acct.insert(acct_hist_key(c.account, height, seq));
                        }
                        _ => fail!("debit {} and its credit disagree", purse_testutil::hex(k)),
                    }
                } else {
                    if !regular || block_invalid(height)? != Some(true) {
                        fail!(
                            "invalidated debit {} outside a disapproved regular tree",
                            purse_testutil::hex(k)
                        );
                    }
                    if let Some(c) = credit
                        && c.amount != be64(v, 0)
                    {
                        fail!(
                            "invalidated debit {} amount differs from its credit",
                            purse_testutil::hex(k)
                        );
                    }
                }
            }
        }
        // Unmined credits and inputs.
        let mut last_unmined: Option<([u8; 32], MsgTx)> = None;
        for e in self.unmined_credits()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            let hash = hash32(k);
            let Some(m) = McVal::decode(v.value()) else {
                fail!("bad unmined credit at {}", purse_testutil::hex(k));
            };
            let tx = match &last_unmined {
                Some((h, tx)) if *h == hash => tx.clone(),
                _ => {
                    let Some(uv) = get(self.tx_unmined()?, &hash)? else {
                        fail!(
                            "unmined credit {} without an unmined record",
                            purse_testutil::hex(k)
                        );
                    };
                    let tx = parse(&uv[8..])?;
                    last_unmined = Some((hash, tx.clone()));
                    tx
                }
            };
            let Some(d) = self.derive_out(&tx, chain::is_coinbase(&tx), be32(k, 32))? else {
                fail!(
                    "unmined credit {} pays no wallet address",
                    purse_testutil::hex(k)
                );
            };
            let c = CreditVal {
                amount: m.amount,
                flags: m.flags,
                account: m.account,
                script_off: m.script_off,
                script_len: m.script_len,
                script_kind: m.script_kind,
                spender: None,
            };
            let unpublished = has(self.unpublished()?, &hash)?;
            if !same(&c, &d) || m.class != unmined_class(d.tag, d.coinbase, unpublished) {
                fail!(
                    "unmined credit {} differs from its output",
                    purse_testutil::hex(k)
                );
            }
        }
        for e in self.unmined_inputs()?.iter()? {
            let (k, v) = e?;
            let Some(uv) = get(self.tx_unmined()?, v.value())? else {
                fail!(
                    "unmined input {} names no unmined spender",
                    purse_testutil::hex(k.value())
                );
            };
            let tx = parse(&uv[8..])?;
            let spends = tx.tx_in.iter().any(|i| {
                outpoint(&i.previous_out_point.hash.0, i.previous_out_point.index) == k.value()
            });
            if !spends {
                fail!(
                    "unmined input {} is not spent by its spender",
                    purse_testutil::hex(k.value())
                );
            }
        }
        // History, per-account history, block counts.
        let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
        for e in self.tx_mined()?.iter()? {
            let (k, v) = e?;
            let (hash, height, seq) = (hash32(k.value()), be32(k.value(), 32), be32(v.value(), 0));
            if get(self.history()?, &hist_key(height, seq))?.as_deref() != Some(hash.as_slice()) {
                fail!(
                    "mined record {} at {height} not in history",
                    purse_testutil::hex(&hash)
                );
            }
            *counts.entry(height).or_default() += 1;
        }
        let history_rows = self.history()?.len()?;
        if history_rows != counts.values().map(|&c| u64::from(c)).sum::<u64>() {
            fail!(
                "history has {history_rows} rows for {} records",
                counts.values().sum::<u32>()
            );
        }
        let mut have_acct: BTreeSet<[u8; 12]> = BTreeSet::new();
        for e in self.acct_history()?.iter()? {
            let (k, _) = e?;
            have_acct.insert(k.value().try_into().expect("12 bytes"));
        }
        if have_acct != want_acct {
            let d = have_acct.symmetric_difference(&want_acct).next().copied();
            fail!(
                "per-account history differs at {}",
                d.map(|k| purse_testutil::hex(&k)).unwrap_or_default()
            );
        }
        // Blocks, headers, the tip.
        let (tip, tip_hash) = self.tip()?;
        let mut expect = 0u32;
        for e in self.blocks()?.iter()? {
            let (k, v) = e?;
            let (h, v) = (be32(k.value(), 0), v.value());
            if h != expect {
                fail!("block heights not contiguous at {h}");
            }
            expect += 1;
            if be32(v, 43) != counts.get(&h).copied().unwrap_or(0) {
                fail!("block {h} transaction count differs from its records");
            }
            if get(self.block_by_hash()?, &v[..32])?.as_deref() != Some(h.to_be_bytes().as_slice())
            {
                fail!("block {h} missing from block_by_hash");
            }
            if h > 0 {
                let Some(hd) = get(self.headers()?, &h.to_be_bytes())? else {
                    fail!("block {h} has no header");
                };
                if dcroxide_crypto::blake256::sum256(&hd) != v[..32]
                    || !has(self.cfilters()?, &h.to_be_bytes())?
                {
                    fail!("block {h} header or filter differs");
                }
            }
            if h == tip && v[..32] != tip_hash {
                fail!("tip hash differs from block {h}");
            }
        }
        if expect != tip + 1 || self.block_by_hash()?.len()? != u64::from(expect) {
            fail!("tip {tip} but {expect} block rows");
        }
        // Addresses.
        let paths = self.addr_by_path()?.len()?;
        if self.addr_by_hash()?.len()? != paths || self.addr_by_acct_sha()?.len()? != paths {
            fail!("address tables differ in size");
        }
        for e in self.addr_by_path()?.iter()? {
            let (k, v) = e?;
            let h: [u8; 20] = v.value().try_into().expect("20 bytes");
            if get(self.addr_by_hash()?, &h)?.as_deref() != Some(k.value()) {
                fail!("address {} differs between tables", purse_testutil::hex(&h));
            }
            let mut sk = [0; 36];
            sk[..4].copy_from_slice(&be32(k.value(), 0).to_le_bytes());
            sk[4..].copy_from_slice(&addr_sha(&h));
            if get(self.addr_by_acct_sha()?, &sk)?.as_deref() != Some(h.as_slice()) {
                fail!(
                    "address {} missing from the account order",
                    purse_testutil::hex(&h)
                );
            }
        }
        // Commitments against their tickets.
        for e in self.commitments()?.iter()? {
            let (k, v) = e?;
            let k = k.value();
            let ticket = hash32(k);
            let tx = match self.latest_mined(&ticket)? {
                Some(h) => record(&ticket, h)?.map(|(_, t)| t),
                None => match get(self.tx_unmined()?, &ticket)? {
                    Some(uv) => Some(parse(&uv[8..])?),
                    None => None,
                },
            };
            let Some(tx) = tx else {
                fail!("commitment {} without its ticket", purse_testutil::hex(k));
            };
            let derived = tx
                .tx_out
                .get(be32(k, 32) as usize)
                .and_then(|o| chain::decode_commitment(&o.pk_script));
            let Some((h, amount)) = derived else {
                fail!(
                    "commitment {} is not a commitment output",
                    purse_testutil::hex(k)
                );
            };
            let account = self.address(&h)?.map(|a| a.0);
            if amount != be64(v.value(), 0) || account != Some(be32(v.value(), 8)) {
                fail!(
                    "commitment {} differs from its ticket",
                    purse_testutil::hex(k)
                );
            }
        }
        // Sums.
        let mut want: BTreeMap<Vec<u8>, (i64, i64)> = BTreeMap::new();
        let mut want_h: BTreeMap<Vec<u8>, (i64, i64)> = BTreeMap::new();
        let add = |k: Vec<u8>, a: i64, m: &mut BTreeMap<Vec<u8>, (i64, i64)>| {
            let e = m.entry(k).or_insert((0, 0));
            e.0 += a;
            e.1 += 1;
        };
        for e in self.unspent()?.iter()? {
            let (k, v) = e?;
            if has(self.unmined_inputs()?, k.value())? {
                continue;
            }
            let u = UnspentVal::decode(v.value())
                .ok_or_else(|| Error::Invalid("bad unspent value".into()))?;
            add(bal_key(u.account, u.class).to_vec(), u.amount, &mut want);
            if class::windowed(u.class) {
                add(
                    bal_height_key(u.account, u.class, u.height).to_vec(),
                    u.amount,
                    &mut want_h,
                );
            }
        }
        for e in self.unmined_credits()?.iter()? {
            let (k, v) = e?;
            if has(self.unmined_inputs()?, k.value())? {
                continue;
            }
            let m =
                McVal::decode(v.value()).ok_or_else(|| Error::Invalid("bad mc value".into()))?;
            add(bal_key(m.account, m.class).to_vec(), m.amount, &mut want);
        }
        for e in self.commitments_unspent()?.iter()? {
            let (k, v) = e?;
            if v.value()[0] != 0 {
                continue;
            }
            let c = get(self.commitments()?, k.value())?
                .ok_or_else(|| Error::Invalid("commitment missing".into()))?;
            add(
                bal_key(be32(&c, 8), class::LOCKED).to_vec(),
                be64(&c, 0),
                &mut want,
            );
        }
        let read = |t: &ReadOnlyTable<K, K>| -> Result<BTreeMap<Vec<u8>, (i64, i64)>> {
            let mut m = BTreeMap::new();
            for e in t.iter()? {
                let (k, v) = e?;
                m.insert(k.value().to_vec(), (be64(v.value(), 0), be64(v.value(), 8)));
            }
            Ok(m)
        };
        let have = read(self.balances()?)?;
        let have_h = read(self.balance_heights()?)?;
        if have != want {
            fail!(
                "balances differ: maintained {} rows, recomputed {} rows; first difference {:?}",
                have.len(),
                want.len(),
                first_diff(&have, &want)
            );
        }
        if have_h != want_h {
            fail!(
                "per-height balances differ: {:?}",
                first_diff(&have_h, &want_h)
            );
        }
        let counted = self.unspent_n()?;
        if counted != n || self.unspent()?.len()? != n {
            fail!("unspent count {counted}, keys {n}");
        }
        Ok(None)
    }

    /// A digest of every table in [`ALL_TABLES`] (each pair hashed as
    /// length-prefixed key and value), for comparing a reopened store or a
    /// backup with a reference. The received time in `TX_MINED` and
    /// `TX_UNMINED` values, which differs between runs, is left out.
    pub fn digest(&self) -> Result<[u8; 32]> {
        let mut h = dcroxide_crypto::blake256::Blake256::new();
        for def in ALL_TABLES {
            let skip = match def.name() {
                "tx_mined" => 4..12,
                "tx_unmined" => 0..8,
                _ => 0..0,
            };
            h.update(def.name().as_bytes());
            let t = self.txn.open_table(*def)?;
            for e in t.iter()? {
                let (k, v) = e?;
                let v = v.value();
                let skip = if v.len() >= skip.end {
                    skip.clone()
                } else {
                    0..0
                };
                h.update(&(k.value().len() as u32).to_le_bytes());
                h.update(k.value());
                h.update(&((v.len() - skip.len()) as u32).to_le_bytes());
                h.update(&v[..skip.start]);
                h.update(&v[skip.end..]);
            }
            h.update(b"|");
        }
        Ok(h.finalize())
    }
}

/// A wallet output as its transaction and the address tables describe it.
#[derive(Clone, Copy, Debug)]
struct Derived {
    amount: i64,
    tag: u8,
    coinbase: bool,
    account: u32,
    change: bool,
    script_off: u32,
    script_len: u16,
    script_kind: u8,
}

/// An invalidated credit: (hash, height, index, amount, spent).
pub type InvalidatedCredit = ([u8; 32], u32, u32, i64, bool);
/// An invalidated debit: (hash, height, input, amount, spent outpoint).
pub type InvalidatedDebit = ([u8; 32], u32, u32, i64, [u8; 36]);

/// A key whose maintained and recomputed (sum, count) differ.
type SumDiff = (Vec<u8>, Option<(i64, i64)>, Option<(i64, i64)>);

fn first_diff(
    a: &BTreeMap<Vec<u8>, (i64, i64)>,
    b: &BTreeMap<Vec<u8>, (i64, i64)>,
) -> Option<SumDiff> {
    let keys: std::collections::BTreeSet<&Vec<u8>> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .find(|k| a.get(*k) != b.get(*k))
        .map(|k| (k.clone(), a.get(k).copied(), b.get(k).copied()))
}

/// One entry of [`R::unspent_outputs`].
#[derive(Clone, Debug)]
pub struct Utxo {
    /// The 36-byte outpoint key.
    pub outpoint: [u8; 36],
    /// The tree recomputed from the tag.
    pub tree: i8,
    /// Height, -1 for unmined.
    pub height: i32,
    /// Amount.
    pub amount: i64,
    /// Coinbase flag as `UnspentOutputs` reports it.
    pub coinbase: bool,
    /// The pkScript.
    pub script: Vec<u8>,
}

// ---------------------------------------------------------------------------
// bbolt cursor semantics

/// A cursor over a table with bbolt v1.3.12's semantics (spec 04 section
/// 5.1), built on redb's gap cursor: `seek` lands on the first key at or
/// after its argument, `prev` after a `seek` that found nothing returns
/// the last key, `next` at the end and `prev` at the start return `None`
/// (bbolt leaves its cursor on the last, respectively first, element, and
/// the modes below reproduce what the next step then returns).
pub struct BoltCursor<'a> {
    table: &'a ReadOnlyTable<K, K>,
    c: Option<redb::Cursor<'a, K, K>>,
    mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// On an element; redb's gap is just after it.
    After,
    /// On an element; redb's gap is just before it.
    Before,
    /// `seek` found nothing; the gap is at the end.
    EndSeek,
    /// `next` ran off the end; bbolt stays on the last element.
    EndNext,
    /// `prev` ran off the start; bbolt stays on the first element.
    Start,
}

/// A copied key/value pair.
pub type Kv = (Vec<u8>, Vec<u8>);

fn kv(e: Option<(redb::AccessGuard<'_, K>, redb::AccessGuard<'_, K>)>) -> Option<Kv> {
    e.map(|(k, v)| (k.value().to_vec(), v.value().to_vec()))
}

impl<'a> BoltCursor<'a> {
    /// A cursor over `table`; position it with [`BoltCursor::seek`].
    pub fn new(table: &'a ReadOnlyTable<K, K>) -> Self {
        BoltCursor {
            table,
            c: None,
            mode: Mode::EndSeek,
        }
    }

    /// `Seek`: the first key `>= k`.
    pub fn seek(&mut self, k: &[u8]) -> Result<Option<Kv>> {
        let mut c = self.table.lower_bound(Bound::Included(k))?;
        let e = kv(c.next()?);
        self.mode = if e.is_some() {
            Mode::After
        } else {
            Mode::EndSeek
        };
        self.c = Some(c);
        Ok(e)
    }

    /// `Next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Kv>> {
        let Some(c) = self.c.as_mut() else {
            return Ok(None);
        };
        let e = match self.mode {
            Mode::After => kv(c.next()?),
            Mode::Before => {
                c.next()?;
                kv(c.next()?)
            }
            Mode::Start => {
                if c.next()?.is_none() {
                    return Ok(None);
                }
                kv(c.next()?)
            }
            Mode::EndSeek | Mode::EndNext => return Ok(None),
        };
        self.mode = if e.is_some() {
            Mode::After
        } else {
            Mode::EndNext
        };
        Ok(e)
    }

    /// `Prev`.
    pub fn prev(&mut self) -> Result<Option<Kv>> {
        let Some(c) = self.c.as_mut() else {
            return Ok(None);
        };
        let e = match self.mode {
            Mode::After | Mode::EndNext => {
                if c.prev()?.is_none() {
                    self.mode = Mode::Start;
                    return Ok(None);
                }
                kv(c.prev()?)
            }
            Mode::Before | Mode::EndSeek => kv(c.prev()?),
            Mode::Start => return Ok(None),
        };
        self.mode = if e.is_some() {
            Mode::Before
        } else {
            Mode::Start
        };
        Ok(e)
    }
}

// ---------------------------------------------------------------------------
// Input selection

/// The randomness selection consumes.
pub trait Rand {
    /// Fills `b`.
    fn fill(&mut self, b: &mut [u8]);
    /// A value in `0..n`.
    fn int_n(&mut self, n: usize) -> usize;
}

impl Rand for purse_testutil::SplitMix64 {
    fn fill(&mut self, b: &mut [u8]) {
        purse_testutil::SplitMix64::fill(self, b);
    }
    fn int_n(&mut self, n: usize) -> usize {
        usize::try_from(self.below(n as u64)).expect("fits")
    }
}

/// `txauthor.InputDetail`.
#[derive(Clone, Debug, Default)]
pub struct InputDetail {
    /// Total selected.
    pub amount: i64,
    /// (outpoint, tree, amount) per input.
    pub inputs: Vec<([u8; 36], i8, i64)>,
    /// The pkScript of each input.
    pub scripts: Vec<Vec<u8>>,
    /// Redeem script size estimates.
    pub redeem_sizes: Vec<usize>,
}

/// A `randomUTXO` skip predicate over an unspent key and value.
pub type SkipFn<'a> = dyn FnMut(&[u8], &[u8]) -> Result<bool> + 'a;

/// `MakeInputSource`'s closure state.
pub struct InputSource<'a, G: Rand> {
    r: &'a R,
    account: u32,
    min_conf: i32,
    sync_height: i32,
    rng: G,
    num_unspent: u64,
    seen: HashSet<[u8; 36]>,
    rand_tries: u64,
    remaining: Option<Vec<([u8; 36], bool)>>,
    detail: InputDetail,
}

impl R {
    /// `MakeInputSource`. The unspent count comes from `META` instead of a
    /// walk of the whole index.
    pub fn input_source<G: Rand>(
        &self,
        account: u32,
        min_conf: i32,
        sync_height: i32,
        rng: G,
    ) -> Result<InputSource<'_, G>> {
        Ok(InputSource {
            r: self,
            account,
            min_conf,
            sync_height,
            rng,
            num_unspent: self.unspent_n()?,
            seen: HashSet::new(),
            rand_tries: 0,
            remaining: None,
            detail: InputDetail::default(),
        })
    }

    /// The scan half of `randomUTXO` in one direction from `seek`: the
    /// eligible keys of the first transaction met, as dcrwallet collects
    /// them before its random pick.
    pub fn scan_unspent(
        &self,
        seek: &[u8],
        backward: bool,
        skip: &mut SkipFn<'_>,
    ) -> Result<Vec<[u8; 36]>> {
        let mut c = BoltCursor::new(self.unspent()?);
        let mut e = c.seek(seek)?;
        if backward {
            e = c.prev()?;
        }
        let mut keys: Vec<[u8; 36]> = Vec::new();
        while let Some((k, v)) = e {
            if let Some(first) = keys.first()
                && first[..32] != k[..32]
            {
                break;
            }
            if !skip(&k, &v)? {
                keys.push(k.as_slice().try_into().expect("36 bytes"));
            }
            e = if backward { c.prev()? } else { c.next()? };
        }
        Ok(keys)
    }
}

impl<G: Rand> InputSource<'_, G> {
    fn skip(&self, k: &[u8], v: &[u8]) -> Result<bool> {
        if has(self.r.unmined_inputs()?, k)? {
            return Ok(true);
        }
        if v.len() < 4 {
            return Ok(true);
        }
        let h = be32(v, 0) as i32;
        if !chain::confirmed(self.min_conf, h, self.sync_height) {
            return Ok(true);
        }
        Ok(self.seen.contains(k))
    }

    /// `randomUTXO`.
    fn random_utxo(&mut self) -> Result<Option<[u8; 36]>> {
        let mut r = [0u8; 33];
        self.rng.fill(&mut r);
        let prev_first = r[32] & 1 == 1;
        let rand_key = &r[..32];
        let skip_state = |k: &[u8], v: &[u8]| -> Result<bool> { self.skip(k, v) };
        let mut skip = skip_state;
        let mut keys = self.r.scan_unspent(rand_key, prev_first, &mut skip)?;
        if keys.is_empty() {
            keys = self.r.scan_unspent(rand_key, !prev_first, &mut skip)?;
        }
        if keys.is_empty() {
            return Ok(None);
        }
        let i = self.rng.int_n(keys.len());
        Ok(Some(keys[i]))
    }

    /// `SelectInputs`.
    pub fn select(&mut self, target: i64) -> Result<InputDetail> {
        while self.detail.amount < target || target == 0 {
            let mut picked: Option<([u8; 36], bool)> = None;
            if self.min_conf != 0 && target != 0 && self.rand_tries < self.num_unspent / 2 {
                self.rand_tries += 1;
                if let Some(k) = self.random_utxo()? {
                    self.seen.insert(k);
                    picked = Some((k, false));
                }
            } else if self.remaining.is_none() {
                let mut rem = Vec::new();
                for e in self.r.unspent()?.iter()? {
                    let (k, v) = e?;
                    if self.skip(k.value(), v.value())? {
                        continue;
                    }
                    rem.push((k.value().try_into().expect("36 bytes"), false));
                }
                if self.min_conf == 0 {
                    for e in self.r.unmined_credits()?.iter()? {
                        let (k, v) = e?;
                        let k = k.value();
                        if self.seen.contains(k) || has(self.r.unpublished()?, &k[..32])? {
                            continue;
                        }
                        let tag = flags_tag(v.value()[8]);
                        if matches!(tag, OP_SSTX | OP_SSGEN | OP_SSTXCHANGE | OP_SSRTX | OP_TGEN)
                            || tag == 0xc1
                        {
                            continue;
                        }
                        rem.push((k.try_into().expect("36 bytes"), true));
                    }
                }
                // Fisher-Yates, as rand.ShuffleSlice; kept reversed so that
                // popping from the back takes them in shuffled order.
                for i in (1..rem.len()).rev() {
                    let j = self.rng.int_n(i + 1);
                    rem.swap(i, j);
                }
                rem.reverse();
                self.remaining = Some(rem);
            }
            let (k, unmined) = match picked {
                Some(p) => p,
                None => match self.remaining.as_mut() {
                    Some(rem) if !rem.is_empty() => rem.pop().expect("nonempty"),
                    _ => break,
                },
            };
            let (tree, amount, script_kind, script) = if !unmined {
                let Some(u) = get(self.r.unspent()?, &k)?
                    .as_deref()
                    .and_then(UnspentVal::decode)
                else {
                    return invalid("missing mined and unmined credit");
                };
                if u.account != self.account || u.amount == 0 {
                    continue;
                }
                let tag = flags_tag(u.flags);
                let h = u.height as i32;
                if tag == OP_SSTX || !chain::confirmed(self.min_conf, h, self.sync_height) {
                    continue;
                }
                let matured = chain::coinbase_matured(h, self.sync_height);
                if tag == OP_NONSTAKE && u.flags & F_COINBASE != 0 && !matured {
                    continue;
                }
                if matches!(tag, OP_SSGEN | OP_SSRTX | OP_TGEN | 0xc1) && !matured {
                    continue;
                }
                if tag == OP_SSTXCHANGE && !chain::ticket_change_matured(h, self.sync_height) {
                    continue;
                }
                let tree = i8::from(tag != OP_NONSTAKE);
                (
                    tree,
                    u.amount,
                    u.script_kind,
                    (u.height, u.script_off, u.script_len),
                )
            } else {
                let Some(m) = get(self.r.unmined_credits()?, &k)?
                    .as_deref()
                    .and_then(McVal::decode)
                else {
                    return invalid("missing mined and unmined credit");
                };
                if m.account != self.account {
                    continue;
                }
                let tree = i8::from(flags_tag(m.flags) != OP_NONSTAKE);
                (
                    tree,
                    m.amount,
                    m.script_kind,
                    (0, m.script_off, m.script_len),
                )
            };
            if script_kind != 1 {
                continue;
            }
            let script = if unmined {
                self.r.unmined_script(&k[..32], script.1, script.2)?
            } else {
                self.r
                    .mined_script(&k[..32], script.0, script.1, script.2)?
            };
            self.detail.amount += amount;
            self.detail.inputs.push((k, tree, amount));
            self.detail.scripts.push(script);
            self.detail.redeem_sizes.push(108);
        }
        Ok(self.detail.clone())
    }
}
