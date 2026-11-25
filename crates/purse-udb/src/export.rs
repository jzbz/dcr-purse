// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The Rust producer of the logical wallet export, format `purse-export 2`
//! (ADR-0001 section 11, "Rust").
//!
//! dcr-purse's store shares no bytes with dcrwallet's `wallet.db`, so a Go
//! wallet and a Rust wallet that reached their state by the same
//! operations are compared through this text (ADR-0001, ADR-0002). This
//! producer writes the full format: the magic line, the seven metas, all
//! 29 sections of the registry in order with their declarations, record
//! counts and digests, and the eof line, so that `purse-export-check`
//! parses it as strictly as it parses Go's.
//!
//! # What this slice fills
//!
//! The address manager's sections: `[wallet]`, `[masterkeys]`,
//! `[cryptokeys]`, `[cointypes]`, `[accounts]`, `[accountnames]`,
//! `[addresses]`, the per-account address set entries that fold into no
//! row of their account (`[orphans]`, index `acctaddrs`: the entries an
//! overwritten address leaves, QK-0757), the address manager's records of
//! `[derived]` (`coin`, `acctprops`) and `[privcheck]`; and the first
//! slice of the transaction store's ([`crate::txstore`]): `[chain]`,
//! `[blocks]`, `[headers]`, `[cfilters]` (full and digest modes),
//! `[txs]`, `[unpublished]`, `[uinputs]`, `[tickets]`, the records of
//! `[derived]` that read it (`tipheight`, `missingcf`, `utxocount`); the
//! credit slice's: `[credits]`, `[debits]`, `[ucredits]` and `[derived]`'s
//! `balance` (minconf 0 and 1) and `unspentticket`; and the commitment and
//! multisig slice's: `[commitments]` (each commitment with its unspent
//! entry folded in) and `[multisig]` (each record, `utxo` folding its
//! `multisig_unspent` entry, which the store keeps for exactly the unspent
//! records; `txhash` the key's hash, which Go's value repeats)
//! ([`COVERED`]). `[agendas]`, `[treasury]` and `[vsp]` are written empty.
//!
//! The producer never omits state silently (ADR-0001 principle P1): if the
//! store holds a row this slice does not export (an aux record, an
//! `addr_by_path` entry), [`export`] fails and names the table, rather than
//! writing an export that looks complete.
//! The balance sums and `acct_history` are derived from the exported rows
//! (ADR-0003 Decision item 4) and are not exported themselves.
//!
//! The ticket set's value, which dcrwallet writes as −1 and never reads
//! (QK-0056), is not kept; `ticket.picked` is written `-1`, as every
//! dcrwallet store holds it.
//!
//! What it writes for fields the store does not keep: the write-only
//! `wallet.mgrcreated`, `wallet.txcreated` and `wallet.minedbal` are `-`
//! (ADR-0003, addendum "Schema version 1 as built"), and the write-only
//! `address.added`, which is not optional in the format, is `0`: the store
//! keeps no address add time, as it keeps no `mgrcreated`. Comparisons
//! with Go therefore use an observable profile (ADR-0001 section 11).
//! `[residue]`, `[unknown]` and `[bad]` are always empty.

use purse_errors::{AnyError, Kind, e};
use purse_snacl::{Limits, OsRandom};
use purse_store::wallet::{
    CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, CryptoKeyPub, LastAccount, MasterKeyPriv,
    MasterKeyPub, Slip0044Account0, WatchOnly,
};
use std::collections::BTreeMap;

use std::ops::Bound;

use purse_store::{
    AccountKey, AccountNames, Accounts, AddrAcctShaKey, AddrByAcctSha, AddrByHash, AddrRow,
    CFiltersDetached, CoinTypeKeys, Commitments, CommitmentsUnspent, Credits, CreditsInvalidated,
    Debits, DebitsInvalidated, Hash32, HeadersDetached, History, HistoryKey, MultisigOuts, ReadTx,
    TableId, Tickets, TxMined, TxUnmined, UnminedCredits, UnminedInputs, Unpublished, Unspent,
};
use sha2::{Digest, Sha256};

use crate::addrmgr::{
    AccountKeys, Address, ChainParams, IMPORTED_ADDR_ACCOUNT, ImportedData, MAX_ACCOUNT_NUM,
    ManagedAddress, Manager, StoreRead,
};
use crate::txstore::TxStore;

/// The magic line of format 2.
pub const MAGIC: &str = "purse-export 2";

/// One section of the registry: its name, version and record type
/// declarations, exactly as the `=section` and `=type` lines carry them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// The section name.
    pub name: &'static str,
    /// The section version.
    pub version: u32,
    /// The `=type` declarations without their prefix, in order.
    pub types: &'static [&'static str],
}

/// The sections this slice's producer fills, or writes empty because the
/// store can hold none of their rows; every other one is written empty
/// without being compared. (`[derived]` only with the records of
/// [`COVERED_DERIVED`].)
pub const COVERED: [&str; 23] = [
    "wallet",
    "masterkeys",
    "cryptokeys",
    "cointypes",
    "accounts",
    "accountnames",
    "addresses",
    "chain",
    "blocks",
    "headers",
    "cfilters",
    "txs",
    "unpublished",
    "credits",
    "debits",
    "ucredits",
    "uinputs",
    "tickets",
    "commitments",
    "multisig",
    "orphans",
    "derived",
    "privcheck",
];

/// The record types of `[derived]` this slice writes.
pub const COVERED_DERIVED: [&str; 8] = [
    "coin",
    "tipheight",
    "missingcf",
    "utxocount",
    "acctprops",
    "balance",
    "unspentticket",
    "failed",
];

/// The registry of format 2, in file order (ADR-0001 section 6). A test
/// checks it against `purse-export-check`'s.
pub const REGISTRY: &[Section] = &[
    Section {
        name: "wallet",
        version: 1,
        types: &[
            "wallet 0 watchonly:b? lastaccount:u? lastimported:u? mgrcreated:u?~^ txcreated:i?~^ minedbal:i?^",
        ],
    },
    Section {
        name: "masterkeys",
        version: 1,
        types: &["masterkey 1 which:e salt:x~ digest:x~ n:u r:u p:u"],
    },
    Section {
        name: "cryptokeys",
        version: 1,
        types: &["cryptokey 1 which:e sealed:x~"],
    },
    Section {
        name: "cointypes",
        version: 1,
        types: &[
            "cointype 1 which:e encpub:x?~ encpriv:x?~ xpub:s?",
            "slip0044acct0 0 encpub:x~ encpriv:x~ nextext:u nextint:u name:s xpub:s?",
        ],
    },
    Section {
        name: "accounts",
        version: 1,
        types: &[
            "account 1 num:u type:u name:s? idname:s? encpub:x~ encpriv:x~ kdfsalt:x?~ kdftime:u? kdfmem:u? kdfthreads:u?~ extused:u? intused:u? extret:u? intret:u? xpub:s?",
        ],
    },
    Section {
        name: "accountnames",
        version: 1,
        types: &["nameindex 1 name:s num:u?"],
    },
    Section {
        name: "addresses",
        version: 1,
        types: &[
            "address 1 key:x id:x? addr:s? type:e account:u idxacct:u? inset:b branch:u? index:u? encpub:x?~ encpriv:x?~ enchash:x?~ script:x? pubkey:x? added:u~^",
        ],
    },
    Section {
        name: "chain",
        version: 2,
        types: &[
            "tip 0 hash:h",
            "processed 0 hash:h",
            "havecfilters 0 raw:x?",
            "birth 0 set:b fromheight:b? fromtime:b? hash:h? height:u? time:i?@",
        ],
    },
    Section {
        name: "blocks",
        version: 2,
        types: &[
            "blockseg 1 seg:u n:u sha256:x",
            "block 1 height:u hash:h time:u votebits:u inv:b txs:L",
        ],
    },
    Section {
        name: "headers",
        version: 1,
        types: &[
            "hdrseg 1 seg:u n:u sha256:x",
            "header 1 height:u hash:h raw:x",
            "hdrmissing 1 height:u hash:h",
            "sideheader 1 hash:h raw:x",
        ],
    },
    Section {
        name: "cfilters",
        version: 1,
        types: &[
            "cfseg 1 seg:u n:u sha256:x",
            "cfilter 1 height:u hash:h key:x filter:x",
            "cfmissing 1 height:u hash:h",
            "sidecfilter 1 hash:h key:x filter:x",
        ],
    },
    Section {
        name: "txs",
        version: 2,
        types: &[
            "tx 3 hash:h height:u block:h received:i@ raw:x",
            "utx 1 hash:h received:i@ raw:x",
        ],
    },
    Section {
        name: "unpublished",
        version: 1,
        types: &["unpublished 1 hash:h"],
    },
    Section {
        name: "credits",
        version: 1,
        types: &[
            "credit 5 tx:h height:u block:h index:u inv:b amount:i change:b optag:u coinbase:b expiry:b spent:b sptx:h? spheight:u? spblock:h? spindex:u? stype:u?^ scriptoff:u? scriptlen:u? account:u? utxo:b",
        ],
    },
    Section {
        name: "debits",
        version: 1,
        types: &[
            "debit 5 tx:h height:u block:h input:u inv:b amount:i crtx:h crheight:u crblock:h crindex:u",
        ],
    },
    Section {
        name: "ucredits",
        version: 1,
        types: &[
            "ucredit 2 tx:h index:u amount:i change:b optag:u coinbase:b expiry:b spentbit:b stype:u? scriptoff:u? scriptlen:u? account:u?",
        ],
    },
    Section {
        name: "uinputs",
        version: 1,
        types: &["uinput 2 tx:h index:u spender:h"],
    },
    Section {
        name: "tickets",
        version: 1,
        types: &["ticket 1 hash:h picked:i?^"],
    },
    Section {
        name: "commitments",
        version: 1,
        types: &["commitment 2 tx:h index:u amount:i account:u? unspent:b unminedspent:b?"],
    },
    Section {
        name: "multisig",
        version: 1,
        types: &[
            "multisig 2 tx:h index:u scripthash:x m:u n:u spent:b stake:b block:h height:u amount:i sptx:h? spindex:u? txhash:h utxo:b",
        ],
    },
    Section {
        name: "agendas",
        version: 1,
        types: &[
            "agenda 2 version:u agenda:s choice:s",
            "ticketagenda 3 ticket:h version:u agenda:s choice:s",
        ],
    },
    Section {
        name: "treasury",
        version: 1,
        types: &[
            "tspendpolicy 1 tspend:h vote:u",
            "vsptspendpolicy 2 ticket:h tspend:h vote:u",
            "treasurypolicy 1 key:x vote:u",
            "vsptreasurypolicy 2 ticket:h key:x vote:u",
        ],
    },
    Section {
        name: "vsp",
        version: 2,
        types: &[
            "vspfuture 1 offset:u host:s?",
            "vsphost 1 host:s pubkey:x idhost:s? future:u?",
            "vspticket 1 ticket:h feetx:h status:u host:s pubkey:x? future:u?",
        ],
    },
    Section {
        name: "orphans",
        version: 2,
        types: &["orphan 3 index:e ref:s val:s"],
    },
    Section {
        name: "bad",
        version: 2,
        types: &["bad 3 path:s key:x reason:e value:x?"],
    },
    Section {
        name: "unknown",
        version: 1,
        types: &["unknown 2 path:s key:x value:x?"],
    },
    Section {
        name: "derived",
        version: 1,
        types: &[
            "coin 0 type:u?",
            "tipheight 0 height:i",
            "missingcf 0 v:b",
            "utxocount 0 n:u",
            "acctprops 1 num:u name:s type:u extused:u intused:u extret:u intret:u importedkeys:u encrypted:b",
            "balance 2 num:u minconf:i immcoinbase:i immstakegen:i lockedtickets:i spendable:i total:i votingauth:i unconfirmed:i",
            "unspentticket 1 hash:h",
            "failed 1 item:e",
        ],
    },
    Section {
        name: "privcheck",
        version: 2,
        types: &[
            "mkeyok 0 ok:e",
            "ctok 1 which:e ok:e",
            "slip0044acct0ok 0 ok:e",
            "acctok 1 num:u ok:e via:e",
            "addrok 1 key:x ok:e",
        ],
    },
    Section {
        name: "residue",
        version: 1,
        types: &["residue 1 path:s entries:u sha256:x"],
    },
];

/// What [`export`] is asked to produce.
///
/// Its [`Debug`](std::fmt::Debug) shows only whether each passphrase is
/// given, never its bytes, as the manager's and the key types' do.
#[derive(Clone, Copy)]
pub struct ExportOptions<'a> {
    /// The public passphrase: with it the manager is opened, plaintext
    /// fields are filled and `[derived]` is computed (`@decrypt=public`,
    /// `@derived=1`); without it, `@decrypt=none`.
    pub pub_pass: Option<&'a [u8]>,
    /// The private passphrase: with it (and the public one) `[privcheck]`
    /// is computed (`@privcheck=1`).
    pub priv_pass: Option<&'a [u8]>,
    /// The passphrases of accounts with their own (Go's exporter's
    /// `AcctPass`), by account number: with `[privcheck]`, each such
    /// account listed here is unlocked with its passphrase and checked
    /// (`via=account`); one not listed is `nopass`.
    pub acct_pass: &'a [(u32, &'a [u8])],
    /// The `@exporter` meta: the producing program.
    pub exporter: &'a str,
    /// The resource bounds of the key derivations.
    pub limits: Limits,
    /// The chain sections' digest mode (`@chain=digest`, ADR-0001 section
    /// 6.10): blocks without wallet state, headers and filters as one
    /// digest per 4,096 heights. `false` writes every record
    /// (`@chain=full`).
    pub digest: bool,
}

impl std::fmt::Debug for ExportOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let given = |p: Option<&[u8]>| if p.is_some() { "<given>" } else { "<none>" };
        f.debug_struct("ExportOptions")
            .field("pub_pass", &format_args!("{}", given(self.pub_pass)))
            .field("priv_pass", &format_args!("{}", given(self.priv_pass)))
            .field(
                "acct_pass",
                &format_args!("<{} given>", self.acct_pass.len()),
            )
            .field("exporter", &self.exporter)
            .field("limits", &self.limits)
            .field("digest", &self.digest)
            .finish()
    }
}

/// Renders a `u` value.
fn vu(v: impl Into<u64>) -> String {
    v.into().to_string()
}

/// Renders a `b` value.
fn vb(v: bool) -> String {
    if v { "1" } else { "0" }.to_owned()
}

/// Renders an `x` value: lower-case hex, `_` for no bytes.
fn vx(b: &[u8]) -> String {
    if b.is_empty() {
        return "_".to_owned();
    }
    let mut s = String::with_capacity(b.len().saturating_mul(2));
    for byte in b {
        s.push(char::from(HEX[usize::from(byte >> 4)]));
        s.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    s
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Renders an `s` value: bytes 0x21-0x7E other than `"` and `\` as
/// themselves, every other byte as `\xHH`.
fn vs(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().saturating_add(2));
    s.push('"');
    for &c in b {
        if (0x21..=0x7e).contains(&c) && c != b'"' && c != b'\\' {
            s.push(char::from(c));
        } else {
            s.push_str("\\x");
            s.push(char::from(HEX[usize::from(c >> 4)]));
            s.push(char::from(HEX[usize::from(c & 0x0f)]));
        }
    }
    s.push('"');
    s
}

/// The absent value of a `?` field.
const ABSENT: &str = "-";

/// One field of a declaration, as far as rendering needs it.
struct Field<'a> {
    name: &'a str,
    bytes: bool,
    volatile: bool,
}

/// The fields of a declaration (`name keys field:type[flags] ...`).
fn fields(decl: &str) -> (&str, Vec<Field<'_>>) {
    let mut parts = decl.split(' ');
    let name = parts.next().unwrap_or_default();
    let _keys = parts.next();
    let fields = parts
        .map(|p| {
            let (fname, spec) = p.split_once(':').unwrap_or((p, ""));
            Field {
                name: fname,
                bytes: spec.starts_with('x'),
                volatile: spec.contains('~'),
            }
        })
        .collect();
    (name, fields)
}

/// A record: its type's index in the section's declarations and its
/// rendered values in declaration order.
type Record = (usize, Vec<String>);

/// Writes the text, hashing as it goes.
struct Writer {
    out: String,
    eof: Sha256,
    eof_masked: Sha256,
}

impl Writer {
    fn line(&mut self, l: &str) {
        self.out.push_str(l);
        self.out.push('\n');
    }

    fn section(&mut self, spec: &Section, records: &[Record]) {
        self.line(&format!("=section {} {}", spec.name, spec.version));
        let decls: Vec<_> = spec.types.iter().map(|d| fields(d)).collect();
        for d in spec.types {
            self.line(&format!("=type {d}"));
        }
        let mut sha = Sha256::new();
        let mut msha = Sha256::new();
        for (ty, values) in records {
            let (rname, fs) = &decls[*ty];
            let mut line = (*rname).to_owned();
            let mut masked = (*rname).to_owned();
            for (f, v) in fs.iter().zip(values) {
                line.push_str(&format!(" {}={v}", f.name));
                let m = if f.volatile && v != ABSENT && v != "_" {
                    if f.bytes {
                        format!("*{}", v.len() / 2)
                    } else {
                        "*".to_owned()
                    }
                } else {
                    v.clone()
                };
                masked.push_str(&format!(" {}={m}", f.name));
            }
            sha.update(line.as_bytes());
            sha.update(b"\n");
            msha.update(masked.as_bytes());
            msha.update(b"\n");
            self.line(&line);
        }
        let sha = vx(&sha.finalize());
        let msha = vx(&msha.finalize());
        self.line(&format!(
            "=end {} records={} sha256={sha} msha256={msha}",
            spec.name,
            records.len()
        ));
        if spec.name != "residue" {
            self.eof.update(format!("{} {sha}\n", spec.name).as_bytes());
            self.eof_masked
                .update(format!("{} {msha}\n", spec.name).as_bytes());
        }
    }
}

/// `export` failed because the store holds what this slice cannot export.
fn not_exported(what: &str) -> AnyError {
    e!(
        Kind::Invalid,
        format!("logical export: {what} is not exported by this version of purse-udb")
    )
    .into()
}

fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

/// The tables whose rows this slice exports or reads.
const EXPORTED_TABLES: [TableId; 31] = [
    TableId::Meta,
    TableId::Wallet,
    TableId::Accounts,
    TableId::AccountNames,
    TableId::AddrByHash,
    TableId::AddrByAcctSha,
    TableId::Headers,
    TableId::Blocks,
    TableId::BlockByHash,
    TableId::CFilters,
    TableId::HeadersDetached,
    TableId::CFiltersDetached,
    TableId::TxMined,
    TableId::TxUnmined,
    TableId::Unpublished,
    TableId::History,
    TableId::UnminedInputs,
    TableId::Tickets,
    TableId::Credits,
    TableId::CreditsInvalidated,
    TableId::Unspent,
    TableId::UnminedCredits,
    TableId::Debits,
    TableId::DebitsInvalidated,
    TableId::Balances,
    TableId::BalanceHeights,
    TableId::AcctHistory,
    TableId::Commitments,
    TableId::CommitmentsUnspent,
    TableId::MultisigOuts,
    TableId::MultisigUnspent,
];

/// Writes the logical export of the wallet in `tx` (one snapshot,
/// ADR-0001 principle P7).
///
/// With the public passphrase, the producer opens a manager of its own on
/// `tx` (as Go's exporter opens its own `udb.Manager` on a snapshot), so
/// the caller's manager and its lock state are untouched; with the private
/// passphrase too, that manager is unlocked for `[privcheck]` and dropped,
/// which wipes its keys.
///
/// # Errors
///
/// The store holds rows this slice does not export (see the module
/// documentation): `invalid operation: logical export: <table> is not
/// exported by this version of purse-udb`. Opening the manager with the
/// public passphrase: [`Manager::open`]'s errors. The store's.
pub fn export(
    tx: &ReadTx,
    chain: &ChainParams,
    opts: &ExportOptions<'_>,
) -> Result<String, AnyError> {
    let mut r = tx;
    for id in TableId::ALL {
        if EXPORTED_TABLES.contains(id) {
            continue;
        }
        if r.table_len(*id).map_err(store_err)? != 0 {
            return Err(not_exported(&format!("table {}", id.name())));
        }
    }

    let mgr = match opts.pub_pass {
        Some(p) => Some(Manager::open(
            &mut r,
            chain,
            p,
            Box::new(OsRandom),
            opts.limits,
        )?),
        None => None,
    };
    let derived = mgr.is_some();
    let privcheck = derived && opts.priv_pass.is_some();

    let mut w = Writer {
        out: String::new(),
        eof: Sha256::new(),
        eof_masked: Sha256::new(),
    };
    w.line(MAGIC);
    let metas = [
        ("network", chain.name.to_owned()),
        (
            "chain",
            if opts.digest { "digest" } else { "full" }.to_owned(),
        ),
        (
            "decrypt",
            if derived { "public" } else { "none" }.to_owned(),
        ),
        ("derived", vb(derived)),
        ("privcheck", vb(privcheck)),
    ];
    for (k, v) in &metas {
        let l = format!("@{k}={v}");
        w.eof.update(format!("{l}\n").as_bytes());
        w.eof_masked.update(format!("{l}\n").as_bytes());
        w.line(&l);
    }
    w.line(&format!(
        "@origin=purse-udb/{}",
        purse_store::SCHEMA_VERSION
    ));
    w.line(&format!("@exporter={}", opts.exporter));

    let accounts = account_rows(&mut r)?;
    let plain = |sealed: &[u8]| -> String {
        match &mgr {
            Some(m) if !sealed.is_empty() => m
                .decrypt_public(sealed)
                .ok()
                .and_then(|p| {
                    std::str::from_utf8(p.as_bytes())
                        .ok()
                        .map(|s| vs(s.as_bytes()))
                })
                .unwrap_or_else(|| ABSENT.to_owned()),
            _ => ABSENT.to_owned(),
        }
    };

    let addrs = address_records(&mut r, mgr.as_ref(), &accounts)?;
    let mut txstore = txstore_records(tx, opts.digest)?;

    for spec in REGISTRY {
        let records = match spec.name {
            "wallet" => wallet_records(&mut r)?,
            "masterkeys" => master_key_records(&mut r)?,
            "cryptokeys" => crypto_key_records(&mut r)?,
            "cointypes" => coin_type_records(&mut r, &plain)?,
            "accounts" => account_records(&accounts, &plain),
            "accountnames" => name_records(tx)?,
            "addresses" => addrs.records.clone(),
            "orphans" => addrs.orphans.clone(),
            "derived" => match &mgr {
                Some(m) => derived_records(&mut r, m, chain, &accounts)?,
                None => Vec::new(),
            },
            name @ ("chain" | "blocks" | "headers" | "cfilters" | "txs" | "unpublished"
            | "credits" | "debits" | "ucredits" | "uinputs" | "tickets" | "commitments"
            | "multisig") => txstore.remove(name).unwrap_or_default(),
            "privcheck" => match (&mgr, opts.priv_pass) {
                (Some(m), Some(pass)) if privcheck => {
                    privcheck_records(&mut r, m, pass, opts.acct_pass, &accounts, &addrs.priv_keys)?
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        w.section(spec, &records);
    }
    let sha = vx(&w.eof.clone().finalize());
    let msha = vx(&w.eof_masked.clone().finalize());
    w.line(&format!(
        "=eof sections={} sha256={sha} msha256={msha}",
        REGISTRY.len()
    ));
    Ok(w.out)
}

/// Every account row, with its decoded key material, in numeric order.
fn account_rows(
    r: &mut &ReadTx,
) -> Result<Vec<(u32, purse_store::AccountRow, AccountKeys)>, AnyError> {
    let mut nums = r.account_numbers().map_err(store_err)?;
    nums.sort_unstable();
    let mut out = Vec::with_capacity(nums.len());
    for n in nums {
        let row = r
            .get::<Accounts>(&AccountKey(n))
            .map_err(store_err)?
            .ok_or_else(|| not_exported("a vanished account row"))?;
        let keys = AccountKeys::decode(&row.keys)
            .map_err(|_| not_exported(&format!("the malformed key material of account {n}")))?;
        out.push((n, row, keys));
    }
    Ok(out)
}

fn wallet_records(r: &mut &ReadTx) -> Result<Vec<Record>, AnyError> {
    let watch = r.wallet_row::<WatchOnly>().map_err(store_err)?;
    let last = r.wallet_row::<LastAccount>().map_err(store_err)?;
    let imported = r
        .wallet_row::<purse_store::wallet::LastImportedAccount>()
        .map_err(store_err)?;
    let imported = match imported {
        None => vu(IMPORTED_ADDR_ACCOUNT),
        Some(a) if a.0 > MAX_ACCOUNT_NUM => vu(a.0),
        Some(_) => {
            return Err(not_exported(
                "a last imported account below the imported range",
            ));
        }
    };
    Ok(vec![(
        0,
        vec![
            watch.map_or_else(|| ABSENT.to_owned(), vb),
            last.map_or_else(|| ABSENT.to_owned(), |a| vu(a.0)),
            imported,
            ABSENT.to_owned(),
            ABSENT.to_owned(),
            ABSENT.to_owned(),
        ],
    )])
}

fn master_key_records(r: &mut &ReadTx) -> Result<Vec<Record>, AnyError> {
    let mut out = Vec::new();
    for (which, row) in [
        ("priv", r.wallet_row::<MasterKeyPriv>().map_err(store_err)?),
        ("pub", r.wallet_row::<MasterKeyPub>().map_err(store_err)?),
    ] {
        if let Some(p) = row {
            out.push((
                0,
                vec![
                    which.to_owned(),
                    vx(&p.salt),
                    vx(&p.digest),
                    vu(p.n),
                    vu(p.r),
                    vu(p.p),
                ],
            ));
        }
    }
    Ok(out)
}

fn crypto_key_records(r: &mut &ReadTx) -> Result<Vec<Record>, AnyError> {
    let mut out = Vec::new();
    for (which, row) in [
        ("priv", r.wallet_row::<CryptoKeyPriv>().map_err(store_err)?),
        ("pub", r.wallet_row::<CryptoKeyPub>().map_err(store_err)?),
    ] {
        if let Some(k) = row {
            out.push((0, vec![which.to_owned(), vx(&k.0)]));
        }
    }
    Ok(out)
}

fn opt_x(v: Option<&Vec<u8>>) -> String {
    v.map_or_else(|| ABSENT.to_owned(), |b| vx(b))
}

fn coin_type_records(
    r: &mut &ReadTx,
    plain: &dyn Fn(&[u8]) -> String,
) -> Result<Vec<Record>, AnyError> {
    let mut out = Vec::new();
    let rows: [(&str, Option<CoinTypeKeys>); 2] = [
        (
            "legacy",
            r.wallet_row::<CoinTypeLegacy>().map_err(store_err)?,
        ),
        (
            "slip0044",
            r.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?,
        ),
    ];
    for (which, row) in rows {
        let Some(k) = row else { continue };
        if k.enc_pub.is_none() && k.enc_priv.is_none() {
            continue;
        }
        let xpub = k
            .enc_pub
            .as_deref()
            .map_or_else(|| ABSENT.to_owned(), plain);
        out.push((
            0,
            vec![
                which.to_owned(),
                opt_x(k.enc_pub.as_ref()),
                opt_x(k.enc_priv.as_ref()),
                xpub,
            ],
        ));
    }
    if let Some(a) = r.wallet_row::<Slip0044Account0>().map_err(store_err)? {
        out.push((
            1,
            vec![
                vx(&a.enc_pub),
                vx(&a.enc_priv),
                vu(a.next_external),
                vu(a.next_internal),
                vs(&a.name),
                plain(&a.enc_pub),
            ],
        ));
    }
    Ok(out)
}

fn account_records(
    accounts: &[(u32, purse_store::AccountRow, AccountKeys)],
    plain: &dyn Fn(&[u8]) -> String,
) -> Vec<Record> {
    accounts
        .iter()
        .map(|(n, row, keys)| {
            let kdf = keys.kdf.as_ref();
            (
                0,
                vec![
                    vu(*n),
                    vu(row.account_type),
                    vs(&row.name),
                    vs(&row.name),
                    vx(&keys.enc_pub),
                    vx(&keys.enc_priv),
                    kdf.map_or_else(|| ABSENT.to_owned(), |k| vx(&k.salt)),
                    kdf.map_or_else(|| ABSENT.to_owned(), |k| vu(k.time)),
                    kdf.map_or_else(|| ABSENT.to_owned(), |k| vu(k.memory)),
                    kdf.map_or_else(|| ABSENT.to_owned(), |k| vu(k.threads)),
                    vu(row.last_used[0]),
                    vu(row.last_used[1]),
                    vu(row.last_returned[0]),
                    vu(row.last_returned[1]),
                    plain(&keys.enc_pub),
                ],
            )
        })
        .collect()
}

fn name_records(tx: &ReadTx) -> Result<Vec<Record>, AnyError> {
    tx.iter::<AccountNames>()
        .map_err(store_err)?
        .map(|row| {
            let (name, num) = row.map_err(store_err)?;
            Ok((0, vec![vs(&name.0), vu(num.0)]))
        })
        .collect()
}

/// What the API answers for an address row (Go's exporter's `addrInfo`):
/// `AddrHash`, `Address().String()` and, for an imported key, `PubKey`.
struct AddrInfo {
    id: [u8; 20],
    addr: String,
    pub_key: Option<Vec<u8>>,
    address: Address,
}

/// An imported key `[privcheck]` checks: its bucket key and, when the API
/// reached it, its public key and address.
type PrivKeyAddr = ([u8; 32], Option<(Vec<u8>, Address)>);

/// `[addresses]`, `[orphans]` and the imported keys `[privcheck]` checks.
struct AddressSections {
    records: Vec<Record>,
    orphans: Vec<Record>,
    /// Bucket key and API answer of each imported key with a sealed
    /// private key, in key order.
    priv_keys: Vec<PrivKeyAddr>,
}

/// `[addresses]` (ADR-0001 section 6.7) and the `acctaddrs` orphans, as
/// Go's exporter writes them: one record per address row in the order of
/// its dcrwallet bucket key, SHA-256 of the HASH160; `id`, `addr` and
/// `pubkey` from `Manager::for_each_account_address` over every account,
/// indexed by the SHA-256 of the answer's HASH160 (`-` for a row no answer
/// reaches, which Go also reports in `[bad]`); `idxacct` from the row's
/// account (the store's flat index is the row itself); `inset` from the
/// row's account's set.
fn address_records(
    r: &mut &ReadTx,
    mgr: Option<&Manager>,
    accounts: &[(u32, purse_store::AccountRow, AccountKeys)],
) -> Result<AddressSections, AnyError> {
    let mut api: BTreeMap<[u8; 32], AddrInfo> = BTreeMap::new();
    if let Some(m) = mgr {
        for (n, _, _) in accounts {
            // An error stops this account's walk, as in Go, which then
            // writes a [bad] record "api" (not exported here).
            let _ = m.for_each_account_address(r, *n, |ma| {
                let id = ma.addr_hash();
                let pub_key = match &ma {
                    ManagedAddress::PubKey {
                        pub_key,
                        imported: true,
                        ..
                    } => Some(pub_key.clone()),
                    _ => None,
                };
                api.insert(
                    Sha256::digest(id).into(),
                    AddrInfo {
                        id,
                        addr: ma.address().encode(),
                        pub_key,
                        address: ma.address().clone(),
                    },
                );
                Ok(())
            });
        }
    }

    let mut rows: Vec<([u8; 32], purse_store::Hash160, AddrRow)> = r
        .rows_with_prefix::<AddrByHash>(&[])
        .map_err(store_err)?
        .into_iter()
        .map(|(h, row)| (Sha256::digest(h.0).into(), h, row))
        .collect();
    rows.sort_unstable_by_key(|a| a.0);

    let mut out = AddressSections {
        records: Vec::with_capacity(rows.len()),
        orphans: Vec::new(),
        priv_keys: Vec::new(),
    };
    let absent = || ABSENT.to_owned();
    for (key, h, row) in &rows {
        let account = row.account();
        let (typ, branch, index) = match row {
            AddrRow::Derived { branch, index, .. } => ("chain", vu(*branch), vu(*index)),
            AddrRow::Imported { .. } => ("", absent(), absent()),
        };
        let (mut encpub, mut encpriv, mut enchash, mut script) =
            (absent(), absent(), absent(), absent());
        let typ = match row {
            AddrRow::Derived { .. } => typ,
            AddrRow::Imported { data, .. } => match ImportedData::decode(data)
                .map_err(|_| not_exported("a malformed imported address row"))?
            {
                ImportedData::Key { enc_pub, enc_priv } => {
                    encpub = vx(&enc_pub);
                    encpriv = vx(&enc_priv);
                    if !enc_priv.is_empty() {
                        let info = api
                            .get(key)
                            .and_then(|i| i.pub_key.clone().map(|p| (p, i.address.clone())));
                        out.priv_keys.push((*key, info));
                    }
                    "key"
                }
                ImportedData::Script {
                    enc_hash,
                    script: s,
                } => {
                    enchash = vx(&enc_hash);
                    script = vx(&s);
                    "script"
                }
            },
        };
        let info = api.get(key);
        let (id, addr) = info.map_or_else(
            || (absent(), absent()),
            |i| (vx(&i.id), vs(i.addr.as_bytes())),
        );
        let pub_key = match (typ, info.and_then(|i| i.pub_key.as_ref())) {
            ("key", Some(p)) => vx(p),
            _ => absent(),
        };
        let inset = r
            .get::<AddrByAcctSha>(&AddrAcctShaKey::for_address(account, h))
            .map_err(store_err)?
            .is_some();
        out.records.push((
            0,
            vec![
                vx(key),
                id,
                addr,
                typ.to_owned(),
                vu(account),
                vu(account),
                vb(inset),
                branch,
                index,
                encpub,
                encpriv,
                enchash,
                script,
                pub_key,
                // The add time, which the store does not keep (write-only
                // in dcrwallet; projected out by the observable profile).
                "0".to_owned(),
            ],
        ));
    }

    // Set entries that fold into no row of their account.
    let mut orphans: Vec<String> = Vec::new();
    for (k, h) in r
        .rows_with_prefix::<AddrByAcctSha>(&[])
        .map_err(store_err)?
    {
        let row = r.get::<AddrByHash>(&h).map_err(store_err)?;
        if row.is_none_or(|row| row.account() != k.account) {
            orphans.push(format!("{}:{}", k.account, vx(&k.sha)));
        }
    }
    orphans.sort_unstable();
    out.orphans = orphans
        .into_iter()
        .map(|o| (0, vec!["acctaddrs".to_owned(), vs(o.as_bytes()), vs(b"")]))
        .collect();
    Ok(out)
}

fn derived_records(
    r: &mut &ReadTx,
    m: &Manager,
    chain: &ChainParams,
    accounts: &[(u32, purse_store::AccountRow, AccountKeys)],
) -> Result<Vec<Record>, AnyError> {
    // Type indexes in `[derived]`'s declarations.
    const COIN: usize = 0;
    const TIPHEIGHT: usize = 1;
    const MISSINGCF: usize = 2;
    const UTXOCOUNT: usize = 3;
    const ACCTPROPS: usize = 4;
    const BALANCE: usize = 5;
    const UNSPENTTICKET: usize = 6;
    const FAILED: usize = 7;
    let mut out = vec![(
        COIN,
        vec![m.coin_type(r).map_or_else(|_| ABSENT.to_owned(), vu)],
    )];
    let mut failed_items = Vec::new();
    let store = TxStore::new(chain)?;
    let tip = store.main_chain_tip(r).ok();
    match tip {
        Some((_, h)) => out.push((TIPHEIGHT, vec![h.to_string()])),
        None => failed_items.push("tipheight"),
    }
    out.push((
        MISSINGCF,
        vec![vb(store.is_missing_main_chain_cfilters(r)?)],
    ));
    out.push((UTXOCOUNT, vec![vu(store.unspent_output_count(r)?)]));
    let mut failed = false;
    for (n, _, _) in accounts {
        match m.account_properties(r, *n) {
            Ok(p) => out.push((
                ACCTPROPS,
                vec![
                    vu(*n),
                    vs(&p.account_name),
                    vu(p.account_type),
                    vu(p.last_used_external_index),
                    vu(p.last_used_internal_index),
                    vu(p.last_returned_external_index),
                    vu(p.last_returned_internal_index),
                    vu(p.imported_key_count),
                    vb(p.account_encrypted),
                ],
            )),
            Err(_) => failed = true,
        }
    }
    if failed {
        failed_items.push("acctprops");
    }
    // `AccountBalances` at minconf 0 and 1, in (account, minconf) order.
    let mut balances = Vec::new();
    let mut balance_failed = false;
    for minconf in [0, 1] {
        match store.account_balances(r, minconf) {
            Ok(bs) => {
                for b in bs {
                    balances.push(((b.account, minconf), b));
                }
            }
            Err(_) => balance_failed = true,
        }
    }
    balances.sort_by_key(|(k, _)| *k);
    for ((account, minconf), b) in balances {
        out.push((
            BALANCE,
            vec![
                vu(account),
                minconf.to_string(),
                b.immature_coinbase_rewards.to_string(),
                b.immature_stake_generation.to_string(),
                b.locked_by_tickets.to_string(),
                b.spendable.to_string(),
                b.total.to_string(),
                b.voting_authority.to_string(),
                b.unconfirmed.to_string(),
            ],
        ));
    }
    if balance_failed {
        failed_items.push("balance");
    }
    // `UnspentTickets` at the tip, immature included.
    if let Some((_, h)) = tip {
        match store.unspent_tickets(r, h, true) {
            Ok(hs) => {
                let mut hs: Vec<String> = hs.iter().map(vh).collect();
                hs.sort();
                out.extend(hs.into_iter().map(|h| (UNSPENTTICKET, vec![h])));
            }
            Err(_) => failed_items.push("unspentticket"),
        }
    }
    failed_items.sort_unstable();
    for item in failed_items {
        out.push((FAILED, vec![item.to_owned()]));
    }
    Ok(out)
}

/// Renders a hash stored in internal byte order in display order (`h`).
fn vh(h: &Hash32) -> String {
    let mut r = h.0;
    r.reverse();
    vx(&r)
}

/// The height span of one chain digest segment (ADR-0001 section 6.10).
const SEG_SIZE: u32 = 4096;

/// The digest records of one record type, one per segment of heights.
struct Segmenter {
    ty: usize,
    cur: Option<u32>,
    n: u64,
    sha: Sha256,
    out: Vec<Record>,
}

impl Segmenter {
    fn new(ty: usize) -> Segmenter {
        Segmenter {
            ty,
            cur: None,
            n: 0,
            sha: Sha256::new(),
            out: Vec::new(),
        }
    }

    fn add(&mut self, height: u32, item: &[u8]) {
        let seg = height / SEG_SIZE;
        if self.cur != Some(seg) {
            self.flush();
            self.cur = Some(seg);
            self.sha = Sha256::new();
        }
        self.sha.update(item);
        self.n = self.n.saturating_add(1);
    }

    fn flush(&mut self) {
        if let Some(seg) = self.cur
            && self.n > 0
        {
            let sum = std::mem::take(&mut self.sha).finalize();
            self.out
                .push((self.ty, vec![vu(seg), vu(self.n), vx(&sum)]));
        }
        self.n = 0;
    }

    fn finish(mut self) -> Vec<Record> {
        self.flush();
        self.out
    }
}

/// A `u32` as 4 big-endian bytes, `usize` lengths included (a stored value
/// is below 2^31 bytes).
fn be32(v: usize) -> [u8; 4] {
    u32::try_from(v).unwrap_or(u32::MAX).to_be_bytes()
}

/// The transaction store's sections, by name: `[chain]`, `[blocks]`,
/// `[headers]`, `[cfilters]`, `[txs]`, `[unpublished]`, `[credits]`,
/// `[debits]`, `[ucredits]`, `[uinputs]`, `[tickets]` (ADR-0001 sections 6.9 to 6.16), each record in the order
/// of section 7: dcrwallet's bucket rows as Go's exporter decodes them
/// (`tools/oracle/internal/walletexport/txstore.go`), rendered from the
/// rows ADR-0003 keeps.
fn txstore_records(
    tx: &ReadTx,
    digest: bool,
) -> Result<BTreeMap<&'static str, Vec<Record>>, AnyError> {
    let mut out: BTreeMap<&'static str, Vec<Record>> = BTreeMap::new();

    // [chain]
    let zero = Hash32::default();
    let tip = tx.tip().map_err(store_err)?;
    let mut chain = vec![
        (0, vec![vh(&tip.map_or(zero, |t| t.hash))]),
        (
            1,
            vec![vh(&tx
                .processed_marker()
                .map_err(store_err)?
                .unwrap_or(zero))],
        ),
        (
            2,
            vec![if tx.have_cfilters().map_err(store_err)? {
                "01".to_owned()
            } else {
                ABSENT.to_owned()
            }],
        ),
    ];
    chain.push((
        3,
        match tx.birth_state().map_err(store_err)? {
            Some(b) => vec![
                vb(true),
                vb(b.set_from_height),
                vb(b.set_from_time),
                vh(&b.hash),
                vu(b.height),
                b.time.to_string(),
            ],
            None => {
                let mut v = vec![vb(false)];
                v.extend(std::iter::repeat_n(ABSENT.to_owned(), 5));
                v
            }
        },
    ));
    out.insert("chain", chain);

    // [blocks], [headers], [cfilters]: one walk of the main chain.
    let mut blocks = Vec::new();
    let mut headers = Vec::new();
    let mut hdr_missing = Vec::new();
    let mut cfilters = Vec::new();
    let mut cf_missing = Vec::new();
    let mut block_seg = Segmenter::new(0);
    let mut hdr_seg = Segmenter::new(0);
    let mut cf_seg = Segmenter::new(0);
    let mut main = std::collections::BTreeSet::new();
    let mut block_of_height = BTreeMap::new();
    for row in tx.blocks_in(..).map_err(store_err)? {
        let (h, b) = row.map_err(store_err)?;
        let height = h.0;
        main.insert(b.hash);
        block_of_height.insert(height, b.hash);
        if digest && b.wallet_txs == 0 && !b.invalid {
            let mut item = Vec::with_capacity(46);
            item.extend_from_slice(&height.to_be_bytes());
            item.extend_from_slice(&b.hash.0);
            item.extend_from_slice(&b.time.to_be_bytes());
            item.extend_from_slice(&b.vote_bits.to_be_bytes());
            block_seg.add(height, &item);
        } else {
            let lo = HistoryKey {
                height,
                position: 0,
            };
            let hi = HistoryKey {
                height,
                position: u32::MAX,
            };
            let mut txs = Vec::new();
            for r in tx
                .range::<History>(Bound::Included(&lo), Bound::Included(&hi))
                .map_err(store_err)?
            {
                txs.push(vh(&r.map_err(store_err)?.1));
            }
            let txs = if txs.is_empty() {
                "_".to_owned()
            } else {
                txs.join(",")
            };
            let time = u64::try_from(b.time)
                .map_err(|_| not_exported(&format!("the negative time of block {height}")))?;
            blocks.push((
                1,
                vec![
                    vu(height),
                    vh(&b.hash),
                    vu(time),
                    vu(b.vote_bits),
                    vb(b.invalid),
                    txs,
                ],
            ));
        }
        match tx.header_at(height).map_err(store_err)? {
            None => hdr_missing.push((2, vec![vu(height), vh(&b.hash)])),
            Some(raw) if digest => {
                let mut item = Vec::with_capacity(220);
                item.extend_from_slice(&height.to_be_bytes());
                item.extend_from_slice(&b.hash.0);
                item.extend_from_slice(&be32(raw.0.len()));
                item.extend_from_slice(&raw.0);
                hdr_seg.add(height, &item);
            }
            Some(raw) => headers.push((1, vec![vu(height), vh(&b.hash), vx(&raw.0)])),
        }
        match tx.cfilter_at(height).map_err(store_err)? {
            None => cf_missing.push((2, vec![vu(height), vh(&b.hash)])),
            Some(f) if digest => {
                let mut item = Vec::with_capacity(f.filter.len().saturating_add(56));
                item.extend_from_slice(&height.to_be_bytes());
                item.extend_from_slice(&b.hash.0);
                item.extend_from_slice(&f.key);
                item.extend_from_slice(&be32(f.filter.len()));
                item.extend_from_slice(&f.filter);
                cf_seg.add(height, &item);
            }
            Some(f) => cfilters.push((1, vec![vu(height), vh(&b.hash), vx(&f.key), vx(&f.filter)])),
        }
    }
    let mut v = block_seg.finish();
    v.extend(blocks);
    out.insert("blocks", v);

    let mut side_headers = Vec::new();
    for r in tx.iter::<HeadersDetached>().map_err(store_err)? {
        let (hash, raw) = r.map_err(store_err)?;
        side_headers.push((vh(&hash), vx(&raw.0)));
    }
    side_headers.sort();
    let mut v = hdr_seg.finish();
    v.extend(headers);
    v.extend(hdr_missing);
    v.extend(side_headers.into_iter().map(|(h, raw)| (3, vec![h, raw])));
    out.insert("headers", v);

    let mut side_filters = Vec::new();
    for r in tx.iter::<CFiltersDetached>().map_err(store_err)? {
        let (hash, f) = r.map_err(store_err)?;
        side_filters.push((vh(&hash), vx(&f.key), vx(&f.filter)));
    }
    side_filters.sort();
    let mut v = cf_seg.finish();
    v.extend(cfilters);
    v.extend(cf_missing);
    v.extend(side_filters.into_iter().map(|(h, k, f)| (3, vec![h, k, f])));
    out.insert("cfilters", v);

    // [txs]: mined records by (hash, height, block), then unmined by hash.
    let mut mined = Vec::new();
    for r in tx.iter::<TxMined>().map_err(store_err)? {
        let (k, row) = r.map_err(store_err)?;
        let block = block_of_height.get(&k.height).ok_or_else(|| {
            not_exported(&format!(
                "a mined transaction at height {} above the main chain",
                k.height
            ))
        })?;
        mined.push((
            (vh(&k.hash), k.height, vh(block)),
            row.received.to_string(),
            vx(&row.tx),
        ));
    }
    mined.sort();
    let mut v: Vec<Record> = mined
        .into_iter()
        .map(|((hash, height, block), received, raw)| {
            (0, vec![hash, vu(height), block, received, raw])
        })
        .collect();
    let mut unmined = Vec::new();
    for r in tx.iter::<TxUnmined>().map_err(store_err)? {
        let (hash, row) = r.map_err(store_err)?;
        unmined.push((vh(&hash), row.received.to_string(), vx(&row.tx)));
    }
    unmined.sort();
    v.extend(
        unmined
            .into_iter()
            .map(|(h, rc, raw)| (1, vec![h, rc, raw])),
    );
    out.insert("txs", v);

    let mut unpublished = Vec::new();
    for r in tx.iter::<Unpublished>().map_err(store_err)? {
        unpublished.push(vh(&r.map_err(store_err)?.0));
    }
    unpublished.sort();
    out.insert(
        "unpublished",
        unpublished.into_iter().map(|h| (0, vec![h])).collect(),
    );

    let mut uinputs = Vec::new();
    for r in tx.iter::<UnminedInputs>().map_err(store_err)? {
        let (op, spender) = r.map_err(store_err)?;
        uinputs.push(((vh(&op.hash), op.index), vh(&spender)));
    }
    uinputs.sort();
    out.insert(
        "uinputs",
        uinputs
            .into_iter()
            .map(|((h, i), s)| (0, vec![h, vu(i), s]))
            .collect(),
    );

    credit_records(tx, &block_of_height, &mut out)?;

    let mut tickets = Vec::new();
    for r in tx.iter::<Tickets>().map_err(store_err)? {
        tickets.push(vh(&r.map_err(store_err)?.0));
    }
    tickets.sort();
    out.insert(
        "tickets",
        tickets
            .into_iter()
            .map(|h| (0, vec![h, "-1".to_owned()]))
            .collect(),
    );

    // [commitments]: by outpoint (display hash, then index), the unspent
    // entry folded in (`unminedspent` absent without one).
    let mut commitments = Vec::new();
    for r in tx.iter::<Commitments>().map_err(store_err)? {
        let (op, c) = r.map_err(store_err)?;
        let (unspent, ums) = match tx.get::<CommitmentsUnspent>(&op).map_err(store_err)? {
            Some(u) => (true, vb(u.unmined_spent)),
            None => (false, ABSENT.to_owned()),
        };
        commitments.push((
            (vh(&op.hash), op.index),
            vec![c.amount.to_string(), vu(c.account), vb(unspent), ums],
        ));
    }
    commitments.sort();
    out.insert(
        "commitments",
        commitments
            .into_iter()
            .map(|((h, i), rest)| {
                let mut v = vec![h, vu(i)];
                v.extend(rest);
                (0, v)
            })
            .collect(),
    );

    // [multisig]: by outpoint; the spender only while spent, as Go's
    // exporter writes it; the stored transaction hash is the key's.
    let mut multisig = Vec::new();
    for r in tx.iter::<MultisigOuts>().map_err(store_err)? {
        let (op, m) = r.map_err(store_err)?;
        let (block, height) = m
            .block
            .map_or((Hash32::default(), 0), |b| (b.hash(), b.height()));
        let (sptx, spindex) = match m.spender {
            Some(s) => (vh(&s.hash), vu(s.input)),
            None => (ABSENT.to_owned(), ABSENT.to_owned()),
        };
        multisig.push((
            (vh(&op.hash), op.index),
            vec![
                vx(&m.script_hash.0),
                vu(m.m),
                vu(m.n),
                vb(m.spender.is_some()),
                vb(m.stake_tree),
                vh(&block),
                vu(height),
                m.amount.to_string(),
                sptx,
                spindex,
                vh(&op.hash),
                vb(m.spender.is_none()),
            ],
        ));
    }
    multisig.sort();
    out.insert(
        "multisig",
        multisig
            .into_iter()
            .map(|((h, i), rest)| {
                let mut v = vec![h, vu(i)];
                v.extend(rest);
                (0, v)
            })
            .collect(),
    );
    Ok(out)
}

fn privcheck_records(
    r: &mut &ReadTx,
    m: &Manager,
    pass: &[u8],
    acct_pass: &[(u32, &[u8])],
    accounts: &[(u32, purse_store::AccountRow, AccountKeys)],
    priv_keys: &[PrivKeyAddr],
) -> Result<Vec<Record>, AnyError> {
    const MKEYOK: usize = 0;
    const CTOK: usize = 1;
    const SLIP0044ACCT0OK: usize = 2;
    const ACCTOK: usize = 3;
    const ADDROK: usize = 4;
    let mut out = Vec::new();
    let unlocked = match m.unlock(pass) {
        Ok(()) => {
            out.push((MKEYOK, vec!["yes".to_owned()]));
            true
        }
        Err(err) if err.is_kind(Kind::Passphrase) => {
            out.push((MKEYOK, vec!["no".to_owned()]));
            false
        }
        Err(_) => {
            out.push((MKEYOK, vec!["error".to_owned()]));
            false
        }
    };
    let chain = *m.chain_params();
    let sealed_pair = |enc_pub: &[u8], enc_priv: &[u8]| -> String {
        if !unlocked {
            return "nopass".to_owned();
        }
        let verdict = || -> Option<bool> {
            let p = m.decrypt_public(enc_pub).ok()?;
            let k = m.decrypt_private(enc_priv).ok()?;
            let k = std::str::from_utf8(k.as_bytes()).ok()?;
            let k = purse_hdkeychain::ExtendedKey::new_key_from_string(k, &chain.hd).ok()?;
            Some(k.neuter().string().as_bytes() == p.as_bytes())
        };
        match verdict() {
            Some(true) => "yes",
            Some(false) => "no",
            None => "error",
        }
        .to_owned()
    };
    let rows: [(&str, Option<CoinTypeKeys>); 2] = [
        (
            "legacy",
            r.wallet_row::<CoinTypeLegacy>().map_err(store_err)?,
        ),
        (
            "slip0044",
            r.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?,
        ),
    ];
    for (which, row) in rows {
        let Some(k) = row else { continue };
        let Some(enc_priv) = &k.enc_priv else {
            continue;
        };
        let enc_pub = k.enc_pub.as_deref().unwrap_or_default();
        out.push((CTOK, vec![which.to_owned(), sealed_pair(enc_pub, enc_priv)]));
    }
    if let Some(a) = r.wallet_row::<Slip0044Account0>().map_err(store_err)? {
        out.push((SLIP0044ACCT0OK, vec![sealed_pair(&a.enc_pub, &a.enc_priv)]));
    }
    for (n, _, keys) in accounts {
        if keys.enc_priv.is_empty() {
            continue;
        }
        // As Go's exporter: an account with its own passphrase is unlocked
        // with the one given for it (a failure is `no`), checked, and
        // locked again; one without is checked while the wallet is
        // unlocked.
        let own = keys.kdf.is_some();
        let via = if own { "account" } else { "wallet" };
        let ready = if own {
            match acct_pass.iter().find(|(a, _)| a == n) {
                Some((_, p)) => {
                    if m.unlock_account(r, *n, p).is_err() {
                        out.push((ACCTOK, vec![vu(*n), "no".to_owned(), via.to_owned()]));
                        continue;
                    }
                    true
                }
                None => false,
            }
        } else {
            unlocked
        };
        let verdict = if ready {
            match (
                m.account_extended_priv_key(r, *n),
                m.account_extended_pub_key(r, *n),
            ) {
                (Ok(xpriv), Ok(xpub)) => {
                    if xpriv.neuter().string() == xpub.string() {
                        "yes"
                    } else {
                        "no"
                    }
                }
                _ => "error",
            }
        } else {
            "nopass"
        };
        out.push((ACCTOK, vec![vu(*n), verdict.to_owned(), via.to_owned()]));
        if own && ready {
            let _ = m.lock_account(r, *n);
        }
    }
    // Each imported key with a sealed private key: does the private key
    // open, and is its public key the one the API reports?
    for (key, info) in priv_keys {
        let verdict = match info {
            Some((pub_key, address)) if unlocked => match m.private_key(r, address) {
                Ok(k) => {
                    let p = k.public_point();
                    let compressed = p.serialize_compressed();
                    let uncompressed = match p {
                        dcroxide_dcrec::secp256k1::PublicPoint::Key(k) => {
                            Some(k.serialize_uncompressed())
                        }
                        dcroxide_dcrec::secp256k1::PublicPoint::Infinity => None,
                    };
                    if compressed[..] == pub_key[..]
                        || uncompressed.is_some_and(|u| u[..] == pub_key[..])
                    {
                        "yes"
                    } else {
                        "no"
                    }
                }
                Err(_) => "error",
            },
            _ => "nopass",
        };
        out.push((ADDROK, vec![vx(key), verdict.to_owned()]));
    }
    // The manager is dropped by the caller, which wipes what it unlocked;
    // locking first keeps the order of Go's exporter.
    let _ = m.lock();
    Ok(out)
}

/// The hash of the main-chain block at `height`, which every credit and
/// debit key's height names (Go's keys hold the block hash itself).
fn block_hash_at(
    blocks: &BTreeMap<u32, Hash32>,
    height: u32,
    what: &str,
) -> Result<String, AnyError> {
    blocks.get(&height).map(vh).ok_or_else(|| {
        not_exported(&format!(
            "{what} at height {height}, where no main-chain block stands,"
        ))
    })
}

/// `[credits]`, `[debits]` and `[ucredits]` (ADR-0001 sections 6.13 to
/// 6.15), as Go's exporter decodes `c` and `ic`, `d` and `id`, and `mc`:
/// records ordered by (transaction, height, block, index, invalidated), a
/// credit's `utxo` set when the unspent output of its outpoint names its
/// height. The store's credits always carry their script location and
/// account (dcrwallet writes the account bit on every credit, QK-0078), and
/// an unmined credit never has the spent bit set.
fn credit_records(
    tx: &ReadTx,
    blocks: &BTreeMap<u32, Hash32>,
    out: &mut BTreeMap<&'static str, Vec<Record>>,
) -> Result<(), AnyError> {
    type Key = (String, u32, String, u32, bool);
    let mut credits: Vec<(Key, Vec<String>)> = Vec::new();
    for inv in [false, true] {
        let rows: Vec<(purse_store::CreditKey, purse_store::CreditRow)> = if inv {
            tx.iter::<CreditsInvalidated>()
                .map_err(store_err)?
                .collect::<Result<_, _>>()
                .map_err(store_err)?
        } else {
            tx.iter::<Credits>()
                .map_err(store_err)?
                .collect::<Result<_, _>>()
                .map_err(store_err)?
        };
        for (k, c) in rows {
            let block = block_hash_at(blocks, k.height, "a credit")?;
            let (sptx, spheight, spblock, spindex) = match c.spender {
                Some(s) => (
                    vh(&s.hash),
                    vu(s.height),
                    block_hash_at(blocks, s.height, "a credit's spender")?,
                    vu(s.input),
                ),
                None => (
                    ABSENT.to_owned(),
                    ABSENT.to_owned(),
                    ABSENT.to_owned(),
                    ABSENT.to_owned(),
                ),
            };
            let utxo = !inv
                && tx
                    .get::<Unspent>(&k.outpoint())
                    .map_err(store_err)?
                    .is_some_and(|u| u.height == k.height);
            let key = (vh(&k.hash), k.height, block.clone(), k.index, inv);
            credits.push((
                key,
                vec![
                    vh(&k.hash),
                    vu(k.height),
                    block,
                    vu(k.index),
                    vb(inv),
                    c.amount.to_string(),
                    vb(c.flags.change),
                    vu(c.flags.tag as u8),
                    vb(c.flags.coinbase),
                    vb(c.flags.expiry),
                    vb(c.spender.is_some()),
                    sptx,
                    spheight,
                    spblock,
                    spindex,
                    vu(c.script.class),
                    vu(c.script.offset),
                    vu(c.script.len),
                    vu(c.account),
                    vb(utxo),
                ],
            ));
        }
    }
    credits.sort_by(|a, b| a.0.cmp(&b.0));
    out.insert(
        "credits",
        credits.into_iter().map(|(_, v)| (0, v)).collect(),
    );

    let mut debits: Vec<(Key, Vec<String>)> = Vec::new();
    for inv in [false, true] {
        let rows: Vec<(purse_store::CreditKey, purse_store::DebitRow)> = if inv {
            tx.iter::<DebitsInvalidated>()
                .map_err(store_err)?
                .collect::<Result<_, _>>()
                .map_err(store_err)?
        } else {
            tx.iter::<Debits>()
                .map_err(store_err)?
                .collect::<Result<_, _>>()
                .map_err(store_err)?
        };
        for (k, d) in rows {
            let block = block_hash_at(blocks, k.height, "a debit")?;
            let crblock = block_hash_at(blocks, d.credit.height, "a debit's credit")?;
            let key = (vh(&k.hash), k.height, block.clone(), k.index, inv);
            debits.push((
                key,
                vec![
                    vh(&k.hash),
                    vu(k.height),
                    block,
                    vu(k.index),
                    vb(inv),
                    d.amount.to_string(),
                    vh(&d.credit.hash),
                    vu(d.credit.height),
                    crblock,
                    vu(d.credit.index),
                ],
            ));
        }
    }
    debits.sort_by(|a, b| a.0.cmp(&b.0));
    out.insert("debits", debits.into_iter().map(|(_, v)| (0, v)).collect());

    let mut ucredits: Vec<((String, u32), Vec<String>)> = Vec::new();
    for r in tx.iter::<UnminedCredits>().map_err(store_err)? {
        let (op, m) = r.map_err(store_err)?;
        ucredits.push((
            (vh(&op.hash), op.index),
            vec![
                vh(&op.hash),
                vu(op.index),
                m.amount.to_string(),
                vb(m.flags.change),
                vu(m.flags.tag as u8),
                vb(m.flags.coinbase),
                vb(m.flags.expiry),
                vb(false),
                vu(m.script.class),
                vu(m.script.offset),
                vu(m.script.len),
                vu(m.account),
            ],
        ));
    }
    ucredits.sort_by(|a, b| a.0.cmp(&b.0));
    out.insert(
        "ucredits",
        ucredits.into_iter().map(|(_, v)| (0, v)).collect(),
    );
    Ok(())
}
