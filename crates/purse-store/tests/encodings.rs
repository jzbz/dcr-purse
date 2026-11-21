// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests of every key and value encoding: round trips both ways,
//! key order equal to byte order, the orders dcrwallet's clients observe,
//! and decoders that refuse every malformed input without panicking.
//!
//! Seeded from `PURSE_TEST_SEED` (purse-testutil); the seed is printed.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: generators and lengths"
)]

use std::cmp::Ordering;
use std::fmt::Debug;

use purse_store::{
    AccountKey, AccountName, AccountNumber, AccountRow, AcctHistoryKey, AddrAcctShaKey,
    AddrPathKey, AddrRow, BalanceClass, BalanceHeightKey, BalanceKey, BalanceRow, BirthState,
    BlockRow, CFilterRow, Codec, CoinTypeKeys, CommitmentRow, CommitmentUnspentRow, CreditFlags,
    CreditKey, CreditRow, DebitRow, Empty, Hash32, Hash160, HeaderBytes, Height, HistoryKey,
    MasterKeyParams, MetaKey, Outpoint, OutputTag, ScriptLoc, SealedKey, Slip0044Acct0Row, Spender,
    Tip, TxMinedRow, TxRecKey, TxUnminedRow, UnminedCreditRow, UnpublishedFlag, UnspentRow,
    WalletKey,
};
use purse_store::{
    AgendaChoice, AgendaKey, MultisigBlock, MultisigOutRow, MultisigSpender, MultisigUnspentKey,
    PiKey, TicketAgendaKey, TreasuryVote, VspFeeStatusKey, VspHost, VspHostId, VspPubKeyRow,
    VspTSpendKey, VspTicketRow, VspTreasuryKey,
};
use purse_testutil::SplitMix64;

const ROUNDS: usize = 2_000;

/// Small values often, so that equal prefixes and boundaries are common.
fn u32_of(r: &mut SplitMix64) -> u32 {
    match r.below(6) {
        0 => 0,
        1 => u32::MAX,
        2 => r.below(4) as u32,
        3 => 0x8000_0000 | r.below(4) as u32,
        4 => (r.below(4) as u32) << 8,
        _ => r.next_u64() as u32,
    }
}

fn i64_of(r: &mut SplitMix64) -> i64 {
    match r.below(4) {
        0 => 0,
        1 => i64::MIN,
        2 => i64::MAX,
        _ => r.next_u64() as i64,
    }
}

fn hash_of(r: &mut SplitMix64) -> Hash32 {
    let mut h = [0u8; 32];
    // Few distinct hashes, so that keys share hash prefixes.
    if r.below(2) == 0 {
        h[0] = r.below(3) as u8;
    } else {
        r.fill(&mut h);
    }
    Hash32(h)
}

fn bytes_of(r: &mut SplitMix64, max: u64) -> Vec<u8> {
    let n = r.below(max + 1) as usize;
    r.bytes(n)
}

fn tag_of(r: &mut SplitMix64) -> OutputTag {
    [
        OutputTag::NonStake,
        OutputTag::Ticket,
        OutputTag::Vote,
        OutputTag::Revocation,
        OutputTag::TicketChange,
        OutputTag::TreasuryGen,
    ][r.below(6) as usize]
}

fn flags_of(r: &mut SplitMix64) -> CreditFlags {
    CreditFlags {
        change: r.below(2) == 0,
        tag: tag_of(r),
        coinbase: r.below(2) == 0,
        expiry: r.below(2) == 0,
    }
}

fn class_of(r: &mut SplitMix64) -> BalanceClass {
    BalanceClass::ALL[r.below(BalanceClass::ALL.len() as u64) as usize]
}

fn script_of(r: &mut SplitMix64) -> ScriptLoc {
    ScriptLoc {
        offset: u32_of(r),
        len: r.next_u64() as u16,
        class: r.next_u64() as u8,
    }
}

/// `decode(encode(x)) == x`, and `encode(decode(b)) == b` for the encoding
/// and for every shorter and longer byte string that decodes.
fn round_trip<T: Codec + PartialEq + Debug>(x: &T, r: &mut SplitMix64) {
    let enc = x.encode();
    let enc = enc.as_ref();
    let back = T::decode(enc).unwrap_or_else(|e| panic!("{x:?} does not decode: {e}"));
    assert_eq!(&back, x);
    assert_eq!(back.encode().as_ref(), enc);
    // Truncations and extensions either fail or are themselves canonical.
    for cut in [0, enc.len() / 2, enc.len().saturating_sub(1)] {
        check_canonical::<T>(&enc[..cut]);
    }
    let mut longer = enc.to_vec();
    longer.push(r.next_u64() as u8);
    check_canonical::<T>(&longer);
}

fn check_canonical<T: Codec + Debug>(b: &[u8]) {
    if let Ok(x) = T::decode(b) {
        assert_eq!(x.encode().as_ref(), b, "{x:?} is not canonical");
    }
}

/// `a.cmp(b)` equals the byte order of the encodings.
fn order<T: Codec + Ord + Debug>(a: &T, b: &T) {
    assert_eq!(
        a.cmp(b),
        a.encode().as_ref().cmp(b.encode().as_ref()),
        "order of {a:?} and {b:?}"
    );
}

fn keys<T: Codec + Ord + PartialEq + Debug>(
    label: &str,
    mut gen_key: impl FnMut(&mut SplitMix64) -> T,
) {
    let mut r = SplitMix64::from_env(label);
    for _ in 0..ROUNDS {
        let a = gen_key(&mut r);
        let b = gen_key(&mut r);
        round_trip(&a, &mut r);
        order(&a, &b);
        order(&a, &a);
    }
}

fn values<T: Codec + PartialEq + Debug>(
    label: &str,
    mut gen_value: impl FnMut(&mut SplitMix64) -> T,
) {
    let mut r = SplitMix64::from_env(label);
    for _ in 0..ROUNDS {
        let v = gen_value(&mut r);
        round_trip(&v, &mut r);
    }
}

#[test]
fn key_round_trips_and_order() {
    keys("enc-hash32", hash_of);
    keys("enc-hash160", |r| {
        let mut h = [0u8; 20];
        r.fill(&mut h);
        Hash160(h)
    });
    keys("enc-height", |r| Height(u32_of(r)));
    keys("enc-account-number", |r| AccountNumber(u32_of(r)));
    keys("enc-account-key", |r| AccountKey(u32_of(r)));
    keys("enc-account-name", |r| AccountName(bytes_of(r, 8)));
    keys("enc-addr-path", |r| AddrPathKey {
        account: u32_of(r),
        branch: r.below(3) as u8,
        index: u32_of(r),
    });
    keys("enc-addr-acct-sha", |r| {
        let mut h = [0u8; 20];
        r.fill(&mut h);
        AddrAcctShaKey::for_address(u32_of(r), &Hash160(h))
    });
    keys("enc-tx-rec", |r| TxRecKey {
        hash: hash_of(r),
        height: u32_of(r),
    });
    keys("enc-history", |r| HistoryKey {
        height: u32_of(r),
        position: u32_of(r),
    });
    keys("enc-acct-history", |r| AcctHistoryKey {
        account: u32_of(r),
        height: u32_of(r),
        position: u32_of(r),
    });
    keys("enc-credit-key", |r| CreditKey {
        hash: hash_of(r),
        height: u32_of(r),
        index: u32_of(r),
    });
    keys("enc-outpoint", |r| Outpoint {
        hash: hash_of(r),
        index: u32_of(r),
    });
    keys("enc-balance-key", |r| BalanceKey {
        account: u32_of(r),
        class: class_of(r),
    });
    keys("enc-balance-height-key", |r| BalanceHeightKey {
        account: u32_of(r),
        class: class_of(r),
        height: u32_of(r),
    });
    keys("enc-meta-key", |r| {
        MetaKey::ALL[r.below(MetaKey::ALL.len() as u64) as usize]
    });
    keys("enc-wallet-key", |r| {
        WalletKey::ALL[r.below(WalletKey::ALL.len() as u64) as usize]
    });
    keys("enc-multisig-unspent-key", |r| MultisigUnspentKey {
        script_hash: Hash160([r.below(3) as u8; 20]),
        outpoint: Outpoint {
            hash: hash_of(r),
            index: u32_of(r),
        },
    });
    keys("enc-vsp-host-id", |r| VspHostId(u32_of(r)));
    keys("enc-vsp-host", |r| VspHost(bytes_of(r, 12)));
    keys("enc-vsp-fee-status-key", |r| VspFeeStatusKey {
        status: u32_of(r),
        ticket: hash_of(r),
    });
    keys("enc-pi-key", |r| PiKey(bytes_of(r, 40)));
    keys("enc-vsp-tspend-key", |r| VspTSpendKey {
        ticket: hash_of(r),
        tspend: hash_of(r),
    });
    keys("enc-vsp-treasury-key", |r| VspTreasuryKey {
        ticket: hash_of(r),
        pikey: bytes_of(r, 40),
    });
    keys("enc-agenda-key", |r| AgendaKey {
        version: u32_of(r),
        agenda: bytes_of(r, 12),
    });
    keys("enc-ticket-agenda-key", |r| TicketAgendaKey {
        ticket: hash_of(r),
        version: u32_of(r),
        agenda: bytes_of(r, 12),
    });
}

#[test]
fn value_round_trips() {
    values("enc-account-row", |r| AccountRow {
        account_type: r.next_u64() as u8,
        last_used: [u32_of(r), u32_of(r)],
        last_returned: [u32_of(r), u32_of(r)],
        name: bytes_of(r, 12),
        keys: bytes_of(r, 40),
    });
    values("enc-addr-row", |r| {
        if r.below(2) == 0 {
            AddrRow::Derived {
                account: u32_of(r),
                branch: r.next_u64() as u8,
                index: u32_of(r),
            }
        } else {
            AddrRow::Imported {
                account: u32_of(r),
                data: bytes_of(r, 40),
            }
        }
    });
    values("enc-header", |r| {
        let mut h = [0u8; 180];
        r.fill(&mut h);
        HeaderBytes(h)
    });
    values("enc-block-row", |r| BlockRow {
        hash: hash_of(r),
        time: i64_of(r),
        vote_bits: r.next_u64() as u16,
        invalid: r.below(2) == 0,
        wallet_txs: u32_of(r),
    });
    values("enc-cfilter", |r| {
        let mut key = [0u8; 16];
        r.fill(&mut key);
        CFilterRow {
            key,
            filter: bytes_of(r, 300),
        }
    });
    values("enc-tx-mined", |r| TxMinedRow {
        position: u32_of(r),
        received: i64_of(r),
        tx: bytes_of(r, 200),
    });
    values("enc-tx-unmined", |r| TxUnminedRow {
        received: i64_of(r),
        tx: bytes_of(r, 200),
    });
    values("enc-empty", |_| Empty);
    values("enc-unpublished", |_| UnpublishedFlag);
    values("enc-credit", |r| CreditRow {
        amount: i64_of(r),
        flags: flags_of(r),
        account: u32_of(r),
        script: script_of(r),
        spender: (r.below(2) == 0).then(|| Spender {
            hash: hash_of(r),
            height: u32_of(r),
            input: u32_of(r),
        }),
    });
    values("enc-unspent", |r| UnspentRow {
        height: u32_of(r),
        account: u32_of(r),
        amount: i64_of(r),
        flags: flags_of(r),
        class: class_of(r),
        script: script_of(r),
    });
    values("enc-unmined-credit", |r| UnminedCreditRow {
        amount: i64_of(r),
        flags: flags_of(r),
        account: u32_of(r),
        class: class_of(r),
        script: script_of(r),
    });
    values("enc-debit", |r| DebitRow {
        amount: i64_of(r),
        credit: CreditKey {
            hash: hash_of(r),
            height: u32_of(r),
            index: u32_of(r),
        },
    });
    values("enc-commitment", |r| CommitmentRow {
        amount: i64_of(r),
        account: u32_of(r),
    });
    values("enc-commitment-unspent", |r| CommitmentUnspentRow {
        unmined_spent: r.below(2) == 0,
    });
    values("enc-balance", |r| BalanceRow {
        sum: i64_of(r),
        count: i64_of(r),
    });
    values("enc-tip", |r| Tip {
        height: u32_of(r),
        hash: hash_of(r),
    });
    values("enc-flag", |r| r.below(2) == 0);
    values("enc-birth", |r| BirthState {
        set_from_height: r.below(2) == 0,
        set_from_time: r.below(2) == 0,
        hash: hash_of(r),
        height: u32_of(r),
        time: i64_of(r),
    });
    values("enc-master-key", |r| {
        let mut salt = [0u8; 32];
        let mut digest = [0u8; 32];
        r.fill(&mut salt);
        r.fill(&mut digest);
        MasterKeyParams {
            salt,
            digest,
            n: r.next_u64(),
            r: r.below(16),
            p: r.below(4),
        }
    });
    values("enc-sealed-key", |r| SealedKey(bytes_of(r, 100)));
    values("enc-coin-type-keys", |r| CoinTypeKeys {
        enc_pub: (r.below(3) != 0).then(|| bytes_of(r, 160)),
        enc_priv: (r.below(3) != 0).then(|| bytes_of(r, 160)),
    });
    values("enc-slip0044-acct0", |r| Slip0044Acct0Row {
        enc_pub: bytes_of(r, 160),
        enc_priv: bytes_of(r, 160),
        next_external: u32_of(r),
        next_internal: u32_of(r),
        name: bytes_of(r, 12),
    });
    values("enc-multisig-out", |r| {
        let mut sh = [0u8; 20];
        r.fill(&mut sh);
        MultisigOutRow {
            script_hash: Hash160(sh),
            m: r.next_u64() as u8,
            n: r.next_u64() as u8,
            stake_tree: r.below(2) == 0,
            // A drawn block hash is all zero one time in four: the
            // constructor turns it into no block, which must round-trip as
            // well (an all-zero hash is dcrwallet's "unmined").
            block: (r.below(2) == 0)
                .then(|| {
                    let hash = if r.below(4) == 0 {
                        Hash32::default()
                    } else {
                        hash_of(r)
                    };
                    MultisigBlock::new(hash, u32_of(r))
                })
                .flatten(),
            amount: i64_of(r),
            spender: (r.below(2) == 0).then(|| MultisigSpender {
                hash: hash_of(r),
                input: u32_of(r),
            }),
        }
    });
    values("enc-vsp-ticket", |r| VspTicketRow {
        fee_hash: hash_of(r),
        fee_status: u32_of(r),
        host_id: u32_of(r),
    });
    values("enc-vsp-host-value", |r| VspHost(bytes_of(r, 60)));
    values("enc-vsp-pubkey", |r| VspPubKeyRow {
        host_id: u32_of(r),
        pubkey: bytes_of(r, 40),
    });
    values("enc-treasury-vote", |r| {
        TreasuryVote::new(1 + r.below(255) as u8).expect("non-zero")
    });
    values("enc-agenda-choice", |r| AgendaChoice(bytes_of(r, 12)));
}

/// Random bytes into every decoder: no panic, and whatever decodes is
/// canonical. The fixed-size types also get random bytes of their exact
/// length, so field checks (flags, enumerations) are reached.
#[test]
fn decoders_refuse_garbage() {
    let mut r = SplitMix64::from_env("enc-garbage");
    for _ in 0..20_000 {
        let len = match r.below(3) {
            0 => r.below(70) as usize,
            1 => [
                1, 4, 5, 9, 12, 16, 21, 25, 36, 40, 45, 47, 48, 56, 60, 64, 88, 103,
            ][r.below(18) as usize],
            _ => r.below(400) as usize,
        };
        let b = r.bytes(len);
        check_canonical::<Hash32>(&b);
        check_canonical::<Hash160>(&b);
        check_canonical::<Height>(&b);
        check_canonical::<AccountNumber>(&b);
        check_canonical::<AccountKey>(&b);
        check_canonical::<AccountName>(&b);
        check_canonical::<AddrPathKey>(&b);
        check_canonical::<AddrAcctShaKey>(&b);
        check_canonical::<TxRecKey>(&b);
        check_canonical::<HistoryKey>(&b);
        check_canonical::<AcctHistoryKey>(&b);
        check_canonical::<CreditKey>(&b);
        check_canonical::<Outpoint>(&b);
        check_canonical::<BalanceKey>(&b);
        check_canonical::<BalanceHeightKey>(&b);
        check_canonical::<MetaKey>(&b);
        check_canonical::<AccountRow>(&b);
        check_canonical::<AddrRow>(&b);
        check_canonical::<HeaderBytes>(&b);
        check_canonical::<BlockRow>(&b);
        check_canonical::<CFilterRow>(&b);
        check_canonical::<TxMinedRow>(&b);
        check_canonical::<TxUnminedRow>(&b);
        check_canonical::<Empty>(&b);
        check_canonical::<UnpublishedFlag>(&b);
        check_canonical::<CreditRow>(&b);
        check_canonical::<UnspentRow>(&b);
        check_canonical::<UnminedCreditRow>(&b);
        check_canonical::<DebitRow>(&b);
        check_canonical::<CommitmentRow>(&b);
        check_canonical::<CommitmentUnspentRow>(&b);
        check_canonical::<BalanceRow>(&b);
        check_canonical::<Tip>(&b);
        check_canonical::<WalletKey>(&b);
        check_canonical::<bool>(&b);
        check_canonical::<BirthState>(&b);
        check_canonical::<MasterKeyParams>(&b);
        check_canonical::<SealedKey>(&b);
        check_canonical::<CoinTypeKeys>(&b);
        check_canonical::<Slip0044Acct0Row>(&b);
        check_canonical::<MultisigUnspentKey>(&b);
        check_canonical::<VspHostId>(&b);
        check_canonical::<VspHost>(&b);
        check_canonical::<VspFeeStatusKey>(&b);
        check_canonical::<PiKey>(&b);
        check_canonical::<VspTSpendKey>(&b);
        check_canonical::<VspTreasuryKey>(&b);
        check_canonical::<AgendaKey>(&b);
        check_canonical::<TicketAgendaKey>(&b);
        check_canonical::<MultisigOutRow>(&b);
        check_canonical::<VspTicketRow>(&b);
        check_canonical::<VspPubKeyRow>(&b);
        check_canonical::<TreasuryVote>(&b);
        check_canonical::<AgendaChoice>(&b);
    }
}

/// dcrwallet's account order, which a client sees: the byte order of the
/// little-endian account numbers (brief §6 Phase 2; spec 04 §5.1).
#[test]
fn accounts_iterate_in_little_endian_byte_order() {
    let mut accounts = vec![1u32, 256, 0x8000_0000, 0, 2, 0x7fff_ffff, 255, 65_536];
    accounts.sort_by_key(|a| AccountKey(*a));
    assert_eq!(
        accounts,
        [0, 0x8000_0000, 65_536, 256, 1, 2, 255, 0x7fff_ffff],
    );
    // The first four of the brief's example: 0, 0x80000000, 256, ..., 1.
    let pos = |a: u32| accounts.iter().position(|x| *x == a).unwrap();
    assert!(pos(0) < pos(0x8000_0000) && pos(0x8000_0000) < pos(256) && pos(256) < pos(1));
}

/// Outpoints order by raw hash bytes, then big-endian index: index 256
/// after index 1 within a hash, and any index of a smaller hash first.
#[test]
fn outpoints_order_by_raw_hash_then_big_endian_index() {
    let h = |b: u8| {
        let mut x = [0u8; 32];
        x[0] = b;
        Hash32(x)
    };
    let mut ops = [
        Outpoint {
            hash: h(1),
            index: 0,
        },
        Outpoint {
            hash: h(0),
            index: 256,
        },
        Outpoint {
            hash: h(0),
            index: 1,
        },
        Outpoint {
            hash: h(0),
            index: u32::MAX,
        },
    ];
    ops.sort_by_key(|o| o.encode());
    assert_eq!(
        ops.iter()
            .map(|o| (o.hash.0[0], o.index))
            .collect::<Vec<_>>(),
        [(0, 1), (0, 256), (0, u32::MAX), (1, 0)]
    );
    assert_eq!(ops[0].cmp(&ops[1]), Ordering::Less);
}

/// The per-account address key: an account's rows are contiguous and
/// ordered by the SHA-256 of their HASH160 within it.
#[test]
fn account_address_keys_group_by_little_endian_account() {
    let a = AddrAcctShaKey::for_address(0x7fff_ffff, &Hash160([1; 20]));
    assert_eq!(
        &a.encode()[..4],
        &AddrAcctShaKey::account_prefix(0x7fff_ffff)
    );
    assert_eq!(&a.encode()[..4], &[0xff, 0xff, 0xff, 0x7f]);
    use sha2::Digest as _;
    let want: [u8; 32] = sha2::Sha256::digest([1u8; 20]).into();
    assert_eq!(&a.encode()[4..], &want);
    // Account 1 (01 00 00 00) sorts before account 0x7fffffff
    // (ff ff ff 7f), whatever the digests.
    let b = AddrAcctShaKey::for_address(1, &Hash160([1; 20]));
    assert!(b < a);
}

/// Malformed fields are refused with dcrwallet-shaped texts.
#[test]
fn decode_errors_name_the_row() {
    let e = Outpoint::decode(&[0; 35]).unwrap_err();
    assert_eq!(e.to_string(), "outpoint len 35");
    let mut credit = CreditRow::default().encode();
    credit[8] |= 0x80;
    assert_eq!(
        CreditRow::decode(&credit).unwrap_err().to_string(),
        "credit: reserved flag bit set"
    );
    let mut credit = CreditRow::default().encode();
    credit[30] = 1; // a spender byte on an unspent credit
    assert_eq!(
        CreditRow::decode(&credit).unwrap_err().to_string(),
        "credit: spender set on an unspent credit"
    );
    let mut credit = CreditRow::default().encode();
    credit[8] = 5 << 2; // output tag code 5 is undefined
    assert_eq!(
        CreditRow::decode(&credit).unwrap_err().to_string(),
        "credit: unknown output tag"
    );
    let mut block = BlockRow::default().encode();
    block[42] = 2;
    assert_eq!(
        BlockRow::decode(&block).unwrap_err().to_string(),
        "block record: invalid flag not 0 or 1"
    );
    let mut unspent = UnspentRow {
        height: 1,
        account: 0,
        amount: 5,
        flags: CreditFlags::default(),
        class: BalanceClass::MinedRegular,
        script: ScriptLoc::default(),
    }
    .encode();
    unspent[16] |= 2;
    assert_eq!(
        UnspentRow::decode(&unspent).unwrap_err().to_string(),
        "unspent output: spent flag set"
    );
    unspent[16] &= !2;
    unspent[17] = 13;
    assert_eq!(
        UnspentRow::decode(&unspent).unwrap_err().to_string(),
        "unspent output: unknown class"
    );
    assert_eq!(
        MetaKey::decode(b"nope").unwrap_err().to_string(),
        "meta key: unknown name"
    );
}

/// The record values refuse what their layouts leave undefined, with
/// texts naming the row.
#[test]
fn record_decode_errors_name_the_row() {
    let row = MultisigOutRow {
        script_hash: Hash160([7; 20]),
        m: 1,
        n: 2,
        stake_tree: true,
        block: MultisigBlock::new(Hash32([9; 32]), 5),
        amount: 100,
        spender: None,
    };
    let enc = row.encode();
    assert_eq!(enc.len(), 103);
    assert_eq!(&enc[..20], &[7; 20]);
    assert_eq!(enc[20..23], [1, 2, 2], "m, n, flags (stake tree only)");
    let mut bad = enc;
    bad[22] |= 4;
    assert_eq!(
        MultisigOutRow::decode(&bad).unwrap_err().to_string(),
        "multisig output: unknown flag bits"
    );
    let mut bad = enc;
    bad[23..55].fill(0); // zero block hash: unmined, but height 5
    assert_eq!(
        MultisigOutRow::decode(&bad).unwrap_err().to_string(),
        "multisig output: height set on an unmined output"
    );
    let mut bad = enc;
    bad[70] = 1; // a spender byte while the spent bit is clear
    assert_eq!(
        MultisigOutRow::decode(&bad).unwrap_err().to_string(),
        "multisig output: spender set on an unspent output"
    );
    assert_eq!(
        MultisigOutRow::decode(&enc[..102]).unwrap_err().to_string(),
        "multisig output len 102"
    );
    // A block with an all-zero hash cannot be built: that hash is
    // dcrwallet's "unmined", and a row carrying it would not read back.
    assert_eq!(MultisigBlock::new(Hash32::default(), 0), None);
    assert_eq!(MultisigBlock::new(Hash32::default(), 7), None);
    let b = MultisigBlock::new(Hash32([9; 32]), 5).unwrap();
    assert_eq!((b.hash(), b.height()), (Hash32([9; 32]), 5));
    let unmined = MultisigOutRow { block: None, ..row };
    assert!(unmined.encode()[23..59].iter().all(|&x| x == 0));
    assert_eq!(MultisigOutRow::decode(&unmined.encode()).unwrap(), unmined);
    // Spent with a zero spender hash is a state dcrwallet can write.
    let spent = MultisigOutRow {
        spender: Some(MultisigSpender::default()),
        block: None,
        ..row
    };
    let e = spent.encode();
    assert_eq!(e[22], 3);
    assert_eq!(MultisigOutRow::decode(&e).unwrap(), spent);
    assert_eq!(TreasuryVote::new(0), None);
    assert_eq!(
        TreasuryVote::decode(&[0]).unwrap_err().to_string(),
        "treasury vote: 0 is no policy"
    );
    assert_eq!(TreasuryVote::decode(&[2]).unwrap(), TreasuryVote::NO);
    assert_eq!(TreasuryVote::decode(&[3]).unwrap().code(), 3);
    assert_eq!(
        VspTicketRow::decode(&[0; 36]).unwrap_err().to_string(),
        "vsp ticket len 36"
    );
}

/// The VSP ticket value is dcrwallet's 40-byte layout
/// (`serializeVSPTicket`, `wallet/udb/vsp.go:163-179`): fee hash, status
/// and host id big-endian.
#[test]
fn vsp_ticket_layout_is_dcrwallets() {
    let row = VspTicketRow {
        fee_hash: Hash32([0xaa; 32]),
        fee_status: 3,
        host_id: 0x0102_0304,
    };
    let mut want = vec![0xaa; 32];
    want.extend_from_slice(&[0, 0, 0, 3, 1, 2, 3, 4]);
    assert_eq!(row.encode().to_vec(), want);
    // The policy keys are dcrwallet's bytes too (treasury.go:49-65,
    // 167-183; agendas.go:23-28).
    let k = VspTSpendKey {
        ticket: Hash32([1; 32]),
        tspend: Hash32([2; 32]),
    };
    assert_eq!(&k.encode()[..32], &[1; 32]);
    assert_eq!(&k.encode()[32..], &[2; 32]);
    let a = AgendaKey {
        version: 10,
        agenda: b"treasury".to_vec(),
    };
    assert_eq!(a.encode(), b"\x00\x00\x00\x0atreasury".to_vec());
    let t = TicketAgendaKey {
        ticket: Hash32([3; 32]),
        version: 10,
        agenda: b"treasury".to_vec(),
    };
    assert_eq!(&t.encode()[..32], &[3; 32]);
    assert_eq!(&t.encode()[32..], a.encode().as_slice());
}

/// One script's `multisig_unspent` keys sort as dcrwallet's `mu` keys
/// (bare outpoints) of that script do: within a script hash the key order
/// is the outpoint order, whatever the other scripts' outpoints.
#[test]
fn multisig_unspent_keys_keep_mu_order_within_a_script() {
    let mut r = SplitMix64::from_env("enc-multisig-mu-order");
    for _ in 0..200 {
        let n = 1 + r.below(40) as usize;
        let entries: Vec<(u8, Outpoint)> = (0..n)
            .map(|_| {
                (
                    r.below(3) as u8,
                    Outpoint {
                        hash: hash_of(&mut r),
                        index: u32_of(&mut r),
                    },
                )
            })
            .collect();
        let mut mu: Vec<[u8; 36]> = entries.iter().map(|(_, o)| o.encode()).collect();
        mu.sort();
        mu.dedup();
        let mut ours: Vec<[u8; 56]> = entries
            .iter()
            .map(|(s, o)| {
                MultisigUnspentKey {
                    script_hash: Hash160([*s; 20]),
                    outpoint: *o,
                }
                .encode()
            })
            .collect();
        ours.sort();
        ours.dedup();
        for s in 0..3u8 {
            let want: Vec<[u8; 36]> = mu
                .iter()
                .filter(|k| entries.iter().any(|(es, o)| *es == s && o.encode() == **k))
                .copied()
                .collect();
            let got: Vec<[u8; 36]> = ours
                .iter()
                .filter(|k| k[..20] == [s; 20])
                .map(|k| k[20..].try_into().unwrap())
                .collect();
            assert_eq!(got, want, "script {s}");
        }
    }
}

/// The flags byte layout: bit 0 change, bit 1 spent, bits 2-4 tag, bit 5
/// coinbase, bit 6 expiry.
#[test]
fn credit_flag_bits() {
    let row = CreditRow {
        amount: 1,
        flags: CreditFlags {
            change: true,
            tag: OutputTag::TicketChange,
            coinbase: true,
            expiry: true,
        },
        account: 0,
        script: ScriptLoc::default(),
        spender: Some(Spender::default()),
    };
    assert_eq!(row.encode()[8], 1 | 2 | (4 << 2) | (1 << 5) | (1 << 6));
    let row = CreditRow {
        flags: CreditFlags {
            tag: OutputTag::TreasuryGen,
            ..CreditFlags::default()
        },
        spender: None,
        ..row
    };
    assert_eq!(row.encode()[8], 7 << 2);
}

#[test]
fn balance_classes() {
    for (i, c) in BalanceClass::ALL.iter().enumerate() {
        assert_eq!(*c as usize, i);
        assert_eq!(BalanceClass::from_u8(i as u8), Some(*c));
        assert_eq!(c.is_windowed(), i <= 3);
    }
    assert_eq!(BalanceClass::from_u8(13), None);
}

/// The two header fields the chain operations read are at the offsets of
/// dcrd's header serialization (`dcroxide-wire`, byte-compatible with
/// dcrd's `BlockHeader`): on random headers, `HeaderBytes::height` and
/// `HeaderBytes::prev_block` equal the decoded header's `height` and
/// `prev_block`, and a header serialized from those fields reads back.
#[test]
fn header_fields_are_read_at_the_wire_offsets() {
    let mut r = SplitMix64::from_env("purse-store header fields");
    for _ in 0..ROUNDS {
        let mut b = [0u8; 180];
        r.fill(&mut b);
        let (wire, n) = dcroxide_wire::BlockHeader::from_bytes(&b).expect("180 bytes decode");
        assert_eq!(n, 180);
        assert_eq!(wire.serialize(), b);
        let h = HeaderBytes(b);
        assert_eq!(h.height(), wire.height);
        assert_eq!(h.prev_block().0, wire.prev_block.0);
        let mut w = wire;
        w.height = u32_of(&mut r);
        let h = HeaderBytes(w.serialize());
        assert_eq!(h.height(), w.height);
        assert_eq!(h.prev_block().0, wire.prev_block.0);
    }
}
