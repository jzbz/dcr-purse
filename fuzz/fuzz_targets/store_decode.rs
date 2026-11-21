// SPDX-License-Identifier: ISC
//! Row decoder fuzz target: every key and value type of the wallet store.
//!
//! A row read from the file may be damaged, so a decoder must return `Ok`
//! or `Err` for any bytes, never panic. Encodings are canonical: bytes that
//! decode re-encode to the same bytes.
//!
//! The first byte picks the type; the rest is the row.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_store::{
    AccountKey, AccountName, AccountNumber, AccountRow, AcctHistoryKey, AddrAcctShaKey,
    AddrPathKey, AddrRow, BalanceHeightKey, BalanceKey, BalanceRow, BirthState, BlockRow,
    CFilterRow, Codec, CoinTypeKeys, CommitmentRow, CommitmentUnspentRow, CreditKey, CreditRow,
    DebitRow, Empty, Hash32, Hash160, HeaderBytes, Height, HistoryKey, MasterKeyParams, MetaKey,
    Outpoint, SealedKey, Slip0044Acct0Row, Tip, TxMinedRow, TxRecKey, TxUnminedRow,
    UnminedCreditRow, UnpublishedFlag, UnspentRow, WalletKey,
};
use purse_store::{
    AgendaChoice, AgendaKey, MultisigOutRow, MultisigUnspentKey, PiKey, TicketAgendaKey,
    TreasuryVote, VspFeeStatusKey, VspHost, VspHostId, VspPubKeyRow, VspTSpendKey, VspTicketRow,
    VspTreasuryKey,
};

fn check<T: Codec>(b: &[u8]) {
    if let Ok(x) = T::decode(b) {
        assert_eq!(x.encode().as_ref(), b, "decoded bytes are not canonical");
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&which, row)) = data.split_first() else {
        return;
    };
    match which % 54 {
        0 => check::<Hash32>(row),
        1 => check::<Hash160>(row),
        2 => check::<Height>(row),
        3 => check::<AccountNumber>(row),
        4 => check::<AccountKey>(row),
        5 => check::<AccountName>(row),
        6 => check::<AddrPathKey>(row),
        7 => check::<AddrAcctShaKey>(row),
        8 => check::<TxRecKey>(row),
        9 => check::<HistoryKey>(row),
        10 => check::<AcctHistoryKey>(row),
        11 => check::<CreditKey>(row),
        12 => check::<Outpoint>(row),
        13 => check::<BalanceKey>(row),
        14 => check::<BalanceHeightKey>(row),
        15 => check::<MetaKey>(row),
        16 => check::<AccountRow>(row),
        17 => check::<AddrRow>(row),
        18 => check::<HeaderBytes>(row),
        19 => check::<BlockRow>(row),
        20 => check::<CFilterRow>(row),
        21 => check::<TxMinedRow>(row),
        22 => check::<TxUnminedRow>(row),
        23 => check::<Empty>(row),
        24 => check::<UnpublishedFlag>(row),
        25 => check::<CreditRow>(row),
        26 => check::<UnspentRow>(row),
        27 => check::<UnminedCreditRow>(row),
        28 => check::<DebitRow>(row),
        29 => check::<CommitmentRow>(row),
        30 => check::<CommitmentUnspentRow>(row),
        31 => check::<BalanceRow>(row),
        32 => check::<WalletKey>(row),
        33 => check::<bool>(row),
        34 => check::<BirthState>(row),
        35 => check::<MasterKeyParams>(row),
        36 => check::<SealedKey>(row),
        37 => check::<CoinTypeKeys>(row),
        38 => check::<Slip0044Acct0Row>(row),
        39 => check::<MultisigUnspentKey>(row),
        40 => check::<VspHostId>(row),
        41 => check::<VspHost>(row),
        42 => check::<VspFeeStatusKey>(row),
        43 => check::<PiKey>(row),
        44 => check::<VspTSpendKey>(row),
        45 => check::<VspTreasuryKey>(row),
        46 => check::<AgendaKey>(row),
        47 => check::<TicketAgendaKey>(row),
        48 => check::<MultisigOutRow>(row),
        49 => check::<VspTicketRow>(row),
        50 => check::<VspPubKeyRow>(row),
        51 => check::<TreasuryVote>(row),
        52 => check::<AgendaChoice>(row),
        _ => check::<Tip>(row),
    }
});
