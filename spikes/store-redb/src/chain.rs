//! The few chain rules the spike needs: simnet parameters, the script forms
//! the generator writes and the ingest recognises, and transaction
//! classification for those forms.
//!
//! This is not a port of `txscript`/`stdscript` or of `stake`: it recognises
//! exactly the script and transaction shapes the generator produces, which
//! is enough to drive the store the way dcrwallet drives udb. The Go side
//! classifies the same bytes with dcrd's real code, so a disagreement shows
//! up in the cross-check.

use dcroxide_wire::MsgTx;

/// Simnet coinbase maturity (chaincfg v3.3.0).
pub const COINBASE_MATURITY: i32 = 16;
/// Simnet ticket change maturity.
pub const SSTX_CHANGE_MATURITY: i32 = 1;
/// Simnet ticket maturity.
pub const TICKET_MATURITY: u32 = 16;
/// Simnet legacy BIP0044 coin type, which a freshly created dcrwallet uses.
pub const LEGACY_COIN_TYPE: u32 = 115;
/// The wallet's default address gap limit.
pub const GAP_LIMIT: u32 = 20;
/// The simnet genesis block hash in internal byte order (displayed
/// 6bef82c6...d16b4f87). The cross-check compares it with the tip of a
/// freshly created Go wallet.
pub const SIMNET_GENESIS: [u8; 32] = [
    0x87, 0x4f, 0x6b, 0xd1, 0x59, 0x3a, 0xfe, 0x35, 0xd6, 0x0c, 0x09, 0x20, 0x28, 0x49, 0xf5, 0xc2,
    0x1a, 0x92, 0x72, 0x26, 0xb0, 0x5c, 0x25, 0xf7, 0x85, 0x95, 0x99, 0x45, 0xc6, 0x82, 0xef, 0x6b,
];

/// `OP_NOP10`, the tag dcrwallet stores for a non-stake credit.
pub const OP_NONSTAKE: u8 = 0xb9;
/// Ticket purchase output tag.
pub const OP_SSTX: u8 = 0xba;
/// Vote output tag.
pub const OP_SSGEN: u8 = 0xbb;
/// Revocation output tag.
pub const OP_SSRTX: u8 = 0xbc;
/// Ticket change output tag.
pub const OP_SSTXCHANGE: u8 = 0xbd;
/// Treasury generation output tag.
pub const OP_TGEN: u8 = 0xc3;
const OP_RETURN: u8 = 0x6a;

/// A pay-to-pubkey-hash script.
pub fn p2pkh(hash: &[u8; 20]) -> Vec<u8> {
    let mut s = Vec::with_capacity(25);
    s.extend_from_slice(&[0x76, 0xa9, 0x14]);
    s.extend_from_slice(hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

/// A stake-tagged pay-to-pubkey-hash script.
pub fn tagged_p2pkh(tag: u8, hash: &[u8; 20]) -> Vec<u8> {
    let mut s = Vec::with_capacity(26);
    s.push(tag);
    s.extend_from_slice(&p2pkh(hash));
    s
}

/// A ticket commitment output script (P2PKH commitment, dcrwallet's
/// default fee limits).
pub fn commitment(hash: &[u8; 20], amount: i64) -> Vec<u8> {
    let mut s = Vec::with_capacity(32);
    s.extend_from_slice(&[OP_RETURN, 0x1e]);
    s.extend_from_slice(hash);
    s.extend_from_slice(&(amount as u64).to_le_bytes());
    s.extend_from_slice(&[0x00, 0x58]);
    s
}

/// A null-data script carrying `data` with a single-byte push.
pub fn nulldata(data: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(data.len().saturating_add(2));
    s.push(OP_RETURN);
    s.push(u8::try_from(data.len()).expect("short push"));
    s.extend_from_slice(data);
    s
}

/// What the ingest makes of an output script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    /// P2PKH, or a stake-tagged P2PKH with its tag (`OP_NONSTAKE` for none).
    PubKeyHash { tag: u8, hash: [u8; 20] },
    /// A null-data script.
    NullData,
    /// Anything else (the generator writes nothing else).
    NonStandard,
}

/// Classifies an output script.
pub fn classify(script: &[u8]) -> ScriptKind {
    let is_p2pkh =
        |s: &[u8]| s.len() == 25 && s[..3] == [0x76, 0xa9, 0x14] && s[23..] == [0x88, 0xac];
    if is_p2pkh(script) {
        let mut hash = [0; 20];
        hash.copy_from_slice(&script[3..23]);
        return ScriptKind::PubKeyHash {
            tag: OP_NONSTAKE,
            hash,
        };
    }
    if script.len() == 26
        && matches!(
            script[0],
            OP_SSTX | OP_SSGEN | OP_SSRTX | OP_SSTXCHANGE | OP_TGEN
        )
        && is_p2pkh(&script[1..])
    {
        let mut hash = [0; 20];
        hash.copy_from_slice(&script[4..24]);
        return ScriptKind::PubKeyHash {
            tag: script[0],
            hash,
        };
    }
    if script.first() == Some(&OP_RETURN) {
        return ScriptKind::NullData;
    }
    ScriptKind::NonStandard
}

/// The HASH160 and amount of a P2PKH ticket commitment script.
pub fn decode_commitment(script: &[u8]) -> Option<([u8; 20], i64)> {
    if script.len() < 32 || script[0] != OP_RETURN || script[1] != 0x1e {
        return None;
    }
    let mut hash = [0; 20];
    hash.copy_from_slice(&script[2..22]);
    let mut amt = [0; 8];
    amt.copy_from_slice(&script[22..30]);
    // The high bit flags a P2SH commitment; it is not part of the amount.
    let amount = (u64::from_le_bytes(amt) & !(1 << 63)) as i64;
    Some((hash, amount))
}

/// The stake transaction types the spike distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxType {
    /// Anything that is not a ticket or a vote.
    Regular,
    /// A ticket purchase.
    Ticket,
    /// A vote.
    Vote,
}

fn is_null_outpoint(tx: &MsgTx, i: usize) -> bool {
    let op = &tx.tx_in[i].previous_out_point;
    op.index == u32::MAX && op.hash.0 == [0; 32] && op.tree == 0
}

/// Classifies a transaction the way `stake.DetermineTxType` would for the
/// shapes the generator writes.
pub fn tx_type(tx: &MsgTx) -> TxType {
    let outs = &tx.tx_out;
    let ticket = !outs.is_empty()
        && outs.len() == tx.tx_in.len().saturating_mul(2).saturating_add(1)
        && matches!(
            classify(&outs[0].pk_script),
            ScriptKind::PubKeyHash { tag: OP_SSTX, .. }
        )
        && outs.iter().enumerate().skip(1).all(|(i, o)| {
            if i % 2 == 0 {
                matches!(
                    classify(&o.pk_script),
                    ScriptKind::PubKeyHash {
                        tag: OP_SSTXCHANGE,
                        ..
                    }
                )
            } else {
                o.pk_script.len() >= 32 && o.pk_script[0] == OP_RETURN && o.pk_script[1] == 0x1e
            }
        });
    if ticket {
        return TxType::Ticket;
    }
    let vote = tx.tx_in.len() == 2
        && is_null_outpoint(tx, 0)
        && tx.tx_in[0].block_height == 0
        && tx.tx_in[0].block_index == u32::MAX
        && tx.tx_in[1].previous_out_point.index == 0
        && tx.tx_in[1].previous_out_point.tree == 1
        && outs.len() >= 3
        && outs[0].pk_script.len() == 38
        && outs[0].pk_script[..2] == [OP_RETURN, 0x24]
        && outs[1].pk_script.len() >= 4
        && outs[1].pk_script[0] == OP_RETURN
        && outs.iter().skip(2).all(|o| {
            matches!(
                classify(&o.pk_script),
                ScriptKind::PubKeyHash { tag: OP_SSGEN, .. }
            )
        });
    if vote {
        return TxType::Vote;
    }
    TxType::Regular
}

/// `IsEitherCoinBaseTx` for the shapes the generator writes: one input
/// spending the null outpoint.
pub fn is_coinbase(tx: &MsgTx) -> bool {
    tx.tx_in.len() == 1
        && tx.tx_in[0].previous_out_point.index == u32::MAX
        && tx.tx_in[0].previous_out_point.hash.0 == [0; 32]
}

fn varint_len(n: usize) -> usize {
    match n {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Offsets of each output script within the serialized transaction
/// (`MsgTx.PkScriptLocs`).
pub fn pk_script_locs(tx: &MsgTx) -> Vec<usize> {
    let mut off = 4usize
        .saturating_add(varint_len(tx.tx_in.len()))
        .saturating_add(tx.tx_in.len().saturating_mul(41))
        .saturating_add(varint_len(tx.tx_out.len()));
    let mut locs = Vec::with_capacity(tx.tx_out.len());
    for o in &tx.tx_out {
        off = off
            .saturating_add(10)
            .saturating_add(varint_len(o.pk_script.len()));
        locs.push(off);
        off = off.saturating_add(o.pk_script.len());
    }
    locs
}

/// HASH160 (RIPEMD-160 of BLAKE-256) of `data`.
pub fn hash160(data: &[u8]) -> [u8; 20] {
    dcroxide_crypto::ripemd160::sum160(&dcroxide_crypto::blake256::sum256(data))
}

/// `confirms` (txmined.go:2938).
pub fn confirms(tx_height: i32, cur_height: i32) -> i32 {
    if tx_height == -1 || tx_height > cur_height {
        0
    } else {
        cur_height.wrapping_sub(tx_height).wrapping_add(1)
    }
}

/// `confirmed` (txmined.go:2931).
pub fn confirmed(min_conf: i32, tx_height: i32, cur_height: i32) -> bool {
    confirms(tx_height, cur_height) >= min_conf
}

/// `coinbaseMatured` (txmined.go:2949).
pub fn coinbase_matured(tx_height: i32, cur_height: i32) -> bool {
    tx_height >= 0 && cur_height.wrapping_sub(tx_height).wrapping_add(1) > COINBASE_MATURITY
}

/// `ticketChangeMatured` (txmined.go:2956).
pub fn ticket_change_matured(tx_height: i32, cur_height: i32) -> bool {
    tx_height >= 0 && cur_height.wrapping_sub(tx_height).wrapping_add(1) > SSTX_CHANGE_MATURITY
}
