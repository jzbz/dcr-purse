// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A strict parser of the logical wallet export, format `purse-export 2`
//! (`docs/adr/0001-logical-export-format.md`).
//!
//! dcr-purse keeps its own store and never reads dcrwallet's `wallet.db`
//! (ADR-0013), so the two wallets share no bytes on disk. A Go wallet and a
//! Rust wallet that reached their state by the same operations (operation
//! scripts, ADR-0002) or by restoring the same seed against the same chain
//! are compared instead through a canonical text form of a wallet that
//! dcrwallet (through the Go oracle `tools/oracle/cmd/walletexport`) and,
//! later, `purse-udb` both produce. This crate holds the Rust half that
//! exists before `purse-udb` does: the grammar, the section registry, the
//! enumerated values, record order, masking, the comparison profiles and
//! the digests, checked strictly enough that a file it accepts is in
//! canonical form, and the conformance tests that run the Go exporter on
//! databases Go generated.
//!
//! [`parse`] rejects anything that is not exactly what the format allows:
//! a byte outside printable ASCII, a CR, a trailing space, a section or
//! record type out of place, a field out of order or of the wrong type, a
//! non-canonical value, an enumerated value outside its field's set,
//! records out of order, two records of one type with equal keys, metas
//! that contradict the sections, and digests that do not match the
//! records. It keeps every record in memory. [`validate`] applies the same
//! checks to a reader line by line and keeps only the previous record of
//! the open section, so its memory does not grow with the file.
//!
//! ```
//! let text = std::fs::read_to_string(concat!(
//!     env!("CARGO_MANIFEST_DIR"),
//!     "/tests/data/full-simnet.pe"
//! ))
//! .unwrap();
//! let export = purse_export_check::parse(&text).unwrap();
//! assert_eq!(export.meta("network"), Some("simnet"));
//! assert!(export.section("accounts").unwrap().records.len() > 1);
//! assert_eq!(export.digest(purse_export_check::Profile::MASKED), export.msha256);
//! ```

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;
use std::io::BufRead;

use sha2::{Digest, Sha256};

/// The number in the magic line.
pub const FORMAT_VERSION: u32 = 2;

/// The first line of every export.
pub const MAGIC: &str = "purse-export 2";

/// The header metas, in the order they appear.
pub const METAS: [&str; 7] = [
    "network",
    "chain",
    "decrypt",
    "derived",
    "privcheck",
    "origin",
    "exporter",
];

/// The metas the eof digests cover, in file order (ADR-0001 sections 4
/// and 5).
pub const COMPARED_METAS: [&str; 5] = ["network", "chain", "decrypt", "derived", "privcheck"];

/// One section of the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionSpec {
    /// The section name.
    pub name: &'static str,
    /// The section version.
    pub version: u32,
    /// Whether the eof digests cover the section (false only for
    /// `[residue]`, ADR-0001 section 4).
    pub compared: bool,
    /// The record type declarations (the `=type` lines without their
    /// prefix), in order.
    pub types: &'static [&'static str],
}

const fn spec(name: &'static str, version: u32, types: &'static [&'static str]) -> SectionSpec {
    SectionSpec {
        name,
        version,
        compared: true,
        types,
    }
}

/// The section registry of format 2, in file order. It must equal the Go
/// exporter's registry, which a test checks.
pub const REGISTRY: &[SectionSpec] = &[
    spec(
        "wallet",
        1,
        &[
            "wallet 0 watchonly:b? lastaccount:u? lastimported:u? mgrcreated:u?~^ txcreated:i?~^ minedbal:i?^",
        ],
    ),
    spec(
        "masterkeys",
        1,
        &["masterkey 1 which:e salt:x~ digest:x~ n:u r:u p:u"],
    ),
    spec("cryptokeys", 1, &["cryptokey 1 which:e sealed:x~"]),
    spec(
        "cointypes",
        1,
        &[
            "cointype 1 which:e encpub:x?~ encpriv:x?~ xpub:s?",
            "slip0044acct0 0 encpub:x~ encpriv:x~ nextext:u nextint:u name:s xpub:s?",
        ],
    ),
    spec(
        "accounts",
        1,
        &[
            "account 1 num:u type:u name:s? idname:s? encpub:x~ encpriv:x~ kdfsalt:x?~ kdftime:u? kdfmem:u? kdfthreads:u?~ extused:u? intused:u? extret:u? intret:u? xpub:s?",
        ],
    ),
    spec("accountnames", 1, &["nameindex 1 name:s num:u?"]),
    spec(
        "addresses",
        1,
        &[
            "address 1 key:x id:x? addr:s? type:e account:u idxacct:u? inset:b branch:u? index:u? encpub:x?~ encpriv:x?~ enchash:x?~ script:x? pubkey:x? added:u~^",
        ],
    ),
    spec(
        "chain",
        2,
        &[
            "tip 0 hash:h",
            "processed 0 hash:h",
            "havecfilters 0 raw:x?",
            "birth 0 set:b fromheight:b? fromtime:b? hash:h? height:u? time:i?@",
        ],
    ),
    spec(
        "blocks",
        2,
        &[
            "blockseg 1 seg:u n:u sha256:x",
            "block 1 height:u hash:h time:u votebits:u inv:b txs:L",
        ],
    ),
    spec(
        "headers",
        1,
        &[
            "hdrseg 1 seg:u n:u sha256:x",
            "header 1 height:u hash:h raw:x",
            "hdrmissing 1 height:u hash:h",
            "sideheader 1 hash:h raw:x",
        ],
    ),
    spec(
        "cfilters",
        1,
        &[
            "cfseg 1 seg:u n:u sha256:x",
            "cfilter 1 height:u hash:h key:x filter:x",
            "cfmissing 1 height:u hash:h",
            "sidecfilter 1 hash:h key:x filter:x",
        ],
    ),
    spec(
        "txs",
        2,
        &[
            "tx 3 hash:h height:u block:h received:i@ raw:x",
            "utx 1 hash:h received:i@ raw:x",
        ],
    ),
    spec("unpublished", 1, &["unpublished 1 hash:h"]),
    spec(
        "credits",
        1,
        &[
            "credit 5 tx:h height:u block:h index:u inv:b amount:i change:b optag:u coinbase:b expiry:b spent:b sptx:h? spheight:u? spblock:h? spindex:u? stype:u?^ scriptoff:u? scriptlen:u? account:u? utxo:b",
        ],
    ),
    spec(
        "debits",
        1,
        &[
            "debit 5 tx:h height:u block:h input:u inv:b amount:i crtx:h crheight:u crblock:h crindex:u",
        ],
    ),
    spec(
        "ucredits",
        1,
        &[
            "ucredit 2 tx:h index:u amount:i change:b optag:u coinbase:b expiry:b spentbit:b stype:u? scriptoff:u? scriptlen:u? account:u?",
        ],
    ),
    spec("uinputs", 1, &["uinput 2 tx:h index:u spender:h"]),
    spec("tickets", 1, &["ticket 1 hash:h picked:i?^"]),
    spec(
        "commitments",
        1,
        &["commitment 2 tx:h index:u amount:i account:u? unspent:b unminedspent:b?"],
    ),
    spec(
        "multisig",
        1,
        &[
            "multisig 2 tx:h index:u scripthash:x m:u n:u spent:b stake:b block:h height:u amount:i sptx:h? spindex:u? txhash:h utxo:b",
        ],
    ),
    spec(
        "agendas",
        1,
        &[
            "agenda 2 version:u agenda:s choice:s",
            "ticketagenda 3 ticket:h version:u agenda:s choice:s",
        ],
    ),
    spec(
        "treasury",
        1,
        &[
            "tspendpolicy 1 tspend:h vote:u",
            "vsptspendpolicy 2 ticket:h tspend:h vote:u",
            "treasurypolicy 1 key:x vote:u",
            "vsptreasurypolicy 2 ticket:h key:x vote:u",
        ],
    ),
    spec(
        "vsp",
        2,
        &[
            "vspfuture 1 offset:u host:s?",
            "vsphost 1 host:s pubkey:x idhost:s? future:u?",
            "vspticket 1 ticket:h feetx:h status:u host:s pubkey:x? future:u?",
        ],
    ),
    spec("orphans", 2, &["orphan 3 index:e ref:s val:s"]),
    spec("bad", 2, &["bad 3 path:s key:x reason:e value:x?"]),
    spec("unknown", 1, &["unknown 2 path:s key:x value:x?"]),
    spec(
        "derived",
        1,
        &[
            "coin 0 type:u?",
            "tipheight 0 height:i",
            "missingcf 0 v:b",
            "utxocount 0 n:u",
            "acctprops 1 num:u name:s type:u extused:u intused:u extret:u intret:u importedkeys:u encrypted:b",
            "balance 2 num:u minconf:i immcoinbase:i immstakegen:i lockedtickets:i spendable:i total:i votingauth:i unconfirmed:i",
            "unspentticket 1 hash:h",
            "failed 1 item:e",
        ],
    ),
    spec(
        "privcheck",
        2,
        &[
            "mkeyok 0 ok:e",
            "ctok 1 which:e ok:e",
            "slip0044acct0ok 0 ok:e",
            "acctok 1 num:u ok:e via:e",
            "addrok 1 key:x ok:e",
        ],
    ),
    SectionSpec {
        name: "residue",
        version: 1,
        compared: false,
        types: &["residue 1 path:s entries:u sha256:x"],
    },
];

const VERDICTS: &[&str] = &["yes", "no", "nopass", "error"];

/// The only values each field of type `e` may take, keyed `type.field`
/// (ADR-0001 section 3, "Enumerated values"). It must equal the Go
/// exporter's table, which a test checks.
pub const ENUMS: &[(&str, &[&str])] = &[
    ("masterkey.which", &["pub", "priv"]),
    ("cryptokey.which", &["pub", "priv"]),
    ("cointype.which", &["legacy", "slip0044"]),
    ("address.type", &["chain", "key", "script"]),
    (
        "orphan.index",
        &[
            "addracct",
            "acctaddrs",
            "acctvars",
            "acctidname",
            "utxo",
            "multisigutxo",
            "cmtunspent",
            "vsphostid",
        ],
    ),
    (
        "bad.reason",
        &[
            "len",
            "key",
            "layout",
            "type",
            "value",
            "missing",
            "trail",
            "counter",
            "decrypt",
            "unreached",
            "api",
        ],
    ),
    (
        "failed.item",
        &[
            "coin",
            "tipheight",
            "missingcf",
            "utxocount",
            "acctprops",
            "balance",
            "unspentticket",
        ],
    ),
    ("mkeyok.ok", VERDICTS),
    ("ctok.which", &["legacy", "slip0044"]),
    ("ctok.ok", VERDICTS),
    ("slip0044acct0ok.ok", VERDICTS),
    ("acctok.ok", VERDICTS),
    ("acctok.via", &["wallet", "account"]),
    ("addrok.ok", VERDICTS),
];

/// The fields whose values come from decrypting with the public
/// passphrase; with `@decrypt=none` they are all `-`.
const PLAINTEXT_FIELDS: &[(&str, &str)] = &[
    ("cointype", "xpub"),
    ("slip0044acct0", "xpub"),
    ("account", "xpub"),
    ("address", "id"),
    ("address", "addr"),
    ("address", "pubkey"),
];

/// The allowed values of field `field` of record type `rtype`, when it is
/// of type `e`.
pub fn enum_values(rtype: &str, field: &str) -> Option<&'static [&'static str]> {
    ENUMS
        .iter()
        .find(|(k, _)| {
            k.split_once('.')
                .is_some_and(|(t, f)| t == rtype && f == field)
        })
        .map(|(_, v)| *v)
}

/// The value type of a field (ADR-0001 section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// `u`: unsigned decimal.
    Unsigned,
    /// `i`: signed decimal.
    Signed,
    /// `b`: `0` or `1`.
    Bool,
    /// `h`: a chain hash, 64 lower-case hex digits in display order.
    Hash,
    /// `x`: bytes as lower-case hex in stored order, `_` when empty.
    Bytes,
    /// `s`: a quoted byte string with `\xHH` escapes.
    Str,
    /// `L`: comma-separated hashes, `_` when empty.
    HashList,
    /// `e`: an enumerated token, `[a-z0-9]+`, one of its field's values.
    Enum,
}

impl ValueType {
    fn from_char(c: char) -> Option<Self> {
        Some(match c {
            'u' => ValueType::Unsigned,
            'i' => ValueType::Signed,
            'b' => ValueType::Bool,
            'h' => ValueType::Hash,
            'x' => ValueType::Bytes,
            's' => ValueType::Str,
            'L' => ValueType::HashList,
            'e' => ValueType::Enum,
            _ => return None,
        })
    }
}

/// One field of a record type declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDecl {
    /// The field name.
    pub name: String,
    /// The value type.
    pub ty: ValueType,
    /// `?`: the value may be `-` (absent).
    pub optional: bool,
    /// `~`: derived from randomness, the host or udb's own clock reading;
    /// masked by the masked profiles.
    pub volatile: bool,
    /// `@`: a clock value the caller injects; masked only by the
    /// masked-clock profile.
    pub clock: bool,
    /// `^`: written but never read by dcrwallet at the pin.
    pub write_only: bool,
}

/// A record type declaration (an `=type` line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordType {
    /// The record type name.
    pub name: String,
    /// How many leading fields form the key (0: at most one record).
    pub keys: usize,
    /// The fields, in order.
    pub fields: Vec<FieldDecl>,
}

impl RecordType {
    /// Parses a declaration such as `account 1 num:u name:s?` (without the
    /// `=type ` prefix).
    pub fn parse(decl: &str) -> Result<Self, String> {
        let mut parts = decl.split(' ');
        let name = parts.next().filter(|n| is_name(n)).ok_or("bad type name")?;
        let keys: usize = parts
            .next()
            .filter(|k| is_canonical_unsigned(k))
            .and_then(|k| k.parse().ok())
            .ok_or("bad key count")?;
        let mut fields = Vec::new();
        for part in parts {
            let (fname, spec) = part.split_once(':').ok_or("field without a type")?;
            if !is_name(fname) {
                return Err(format!("bad field name {fname:?}"));
            }
            let mut chars = spec.chars();
            let ty = chars
                .next()
                .and_then(ValueType::from_char)
                .ok_or_else(|| format!("bad type of field {fname}"))?;
            let mut flags = chars.as_str();
            let mut take = |c: char| match flags.strip_prefix(c) {
                Some(rest) => {
                    flags = rest;
                    true
                }
                None => false,
            };
            // Flags appear at most once, in the order ? ~ @ ^, and ~ and @
            // exclude each other.
            let optional = take('?');
            let volatile = take('~');
            let clock = !volatile && take('@');
            let write_only = take('^');
            if !flags.is_empty() {
                return Err(format!("bad flags {spec:?} on field {fname}"));
            }
            fields.push(FieldDecl {
                name: fname.to_owned(),
                ty,
                optional,
                volatile,
                clock,
                write_only,
            });
        }
        if keys > fields.len() {
            return Err("key count exceeds the field count".to_owned());
        }
        if fields[..keys]
            .iter()
            .any(|f| f.optional || f.volatile || f.clock)
        {
            return Err("a key field may not be optional, volatile or a clock value".to_owned());
        }
        Ok(RecordType {
            name: name.to_owned(),
            keys,
            fields,
        })
    }

    /// The index of field `name`.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }
}

/// One record: its type (an index into its section's types) and its
/// rendered values, one per declared field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Index of the record's type in [`Section::types`].
    pub ty: usize,
    /// The rendered values, in field order.
    pub values: Vec<String>,
}

/// One section of an export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The section name.
    pub name: String,
    /// The section version.
    pub version: u32,
    /// Whether the eof digests cover the section.
    pub compared: bool,
    /// Its record types, in declaration (and file) order.
    pub types: Vec<RecordType>,
    /// Its records, in file order (empty in a [`Summary`]).
    pub records: Vec<Record>,
    /// The number of record lines.
    pub count: usize,
    /// SHA-256 of the record lines (lower-case hex).
    pub sha256: String,
    /// SHA-256 of the masked record lines (lower-case hex).
    pub msha256: String,
}

impl Section {
    /// The records of type `name`.
    pub fn records_of<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Record> + use<'a> {
        let idx = self.types.iter().position(|t| t.name == name);
        self.records.iter().filter(move |r| Some(r.ty) == idx)
    }

    /// The value of field `field` of `record` (a record of this section).
    pub fn value<'a>(&self, record: &'a Record, field: &str) -> Option<&'a str> {
        let i = self.types.get(record.ty)?.field_index(field)?;
        record.values.get(i).map(String::as_str)
    }

    /// The record lines, masked (profile masked) or not.
    pub fn lines(&self, masked: bool) -> Vec<String> {
        self.records
            .iter()
            .map(|r| render(&self.types[r.ty], &r.values, masked))
            .collect()
    }

    /// SHA-256 of the record lines under `profile`.
    pub fn digest(&self, profile: Profile) -> String {
        let mut h = Sha256::new();
        for r in &self.records {
            h.update(render_with(&self.types[r.ty], &r.values, profile).as_bytes());
            h.update(b"\n");
        }
        hex_digest(h)
    }
}

/// A parsed and fully validated export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// The header comments (text after `# `).
    pub comments: Vec<String>,
    /// The header metas, in order.
    pub metas: Vec<(String, String)>,
    /// Every section of the registry, in order.
    pub sections: Vec<Section>,
    /// The eof digest (exact profile).
    pub sha256: String,
    /// The eof digest under the masked profile.
    pub msha256: String,
}

/// What [`validate`] returns: an [`Export`] whose sections hold no
/// records, and the eof digest of each profile it was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The export, without records.
    pub export: Export,
    /// The eof digest of each requested profile, in request order.
    pub digests: Vec<(Profile, String)>,
}

impl Export {
    /// The section named `name`.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// The value of meta `name`.
    pub fn meta(&self, name: &str) -> Option<&str> {
        self.metas
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The eof digest under `profile` (ADR-0001 sections 4 and 8): over
    /// the compared metas and the digests of the compared sections. For
    /// [`Profile::EXACT`] and [`Profile::MASKED`] it equals the `=eof`
    /// line's digests.
    pub fn digest(&self, profile: Profile) -> String {
        let mut h = eof_hasher(&self.metas);
        for s in self.sections.iter().filter(|s| s.compared) {
            h.update(format!("{} {}\n", s.name, s.digest(profile)).as_bytes());
        }
        hex_digest(h)
    }

    /// Checks what makes the masked profiles valid (ADR-0001 section 8):
    /// the private passphrase was supplied, every `[privcheck]` verdict is
    /// `yes`, and every sealed private field present has its verdict
    /// (the master key and crypto key through `mkeyok`, each coin-type
    /// private key through `ctok`, the parked SLIP0044 account 0 through
    /// `slip0044acct0ok`, each account's sealed key through `acctok`, each
    /// imported private key through `addrok`).
    pub fn check_sealed_secrets(&self) -> Result<(), String> {
        if self.meta("privcheck") != Some("1") {
            return Err("no [privcheck]: the private passphrase was not supplied".to_owned());
        }
        let pc = self.section("privcheck").ok_or("no [privcheck]")?;
        for r in &pc.records {
            if pc.value(r, "ok") != Some("yes") {
                return Err(format!(
                    "verdict not yes: {}",
                    render(&pc.types[r.ty], &r.values, false)
                ));
            }
        }
        let has = |ty: &str, field: &str, value: &str| {
            pc.records_of(ty)
                .any(|r| field.is_empty() || pc.value(r, field) == Some(value))
        };
        let sec = |n: &str| self.section(n).ok_or_else(|| format!("no [{n}]"));
        if sec("masterkeys")?
            .records
            .iter()
            .any(|r| r.values[0] == "priv")
            && !has("mkeyok", "", "")
        {
            return Err("the private master key has no verdict".to_owned());
        }
        let ct = sec("cointypes")?;
        for r in ct.records_of("cointype") {
            let which = ct.value(r, "which").unwrap_or_default();
            if ct.value(r, "encpriv") != Some("-") && !has("ctok", "which", which) {
                return Err(format!("the {which} coin-type private key has no verdict"));
            }
        }
        if ct.records_of("slip0044acct0").next().is_some() && !has("slip0044acct0ok", "", "") {
            return Err("the parked SLIP0044 account 0 key has no verdict".to_owned());
        }
        let accts = sec("accounts")?;
        for r in &accts.records {
            let num = accts.value(r, "num").unwrap_or_default();
            if accts.value(r, "encpriv") != Some("_") && !has("acctok", "num", num) {
                return Err(format!("account {num}'s private key has no verdict"));
            }
        }
        let addrs = sec("addresses")?;
        for r in &addrs.records {
            let key = addrs.value(r, "key").unwrap_or_default();
            if addrs.value(r, "type") == Some("key")
                && !matches!(addrs.value(r, "encpriv"), Some("_" | "-"))
                && !has("addrok", "key", key)
            {
                return Err(format!("imported key {key}'s private key has no verdict"));
            }
        }
        Ok(())
    }
}

/// Which fields a comparison masks (ADR-0001 section 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mask {
    /// No masking (profile exact).
    None,
    /// `~` fields (profile masked; the `msha256` digests).
    Volatile,
    /// `~` and `@` fields (profile masked-clock, for wallets whose clock
    /// the scenario did not fix).
    VolatileAndClock,
}

/// A comparison profile (ADR-0001 section 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Profile {
    /// What is masked.
    pub mask: Mask,
    /// Whether `^` (write-only) fields are projected out.
    pub observable: bool,
}

impl Profile {
    /// Unmasked lines.
    pub const EXACT: Profile = Profile {
        mask: Mask::None,
        observable: false,
    };
    /// `~` fields masked.
    pub const MASKED: Profile = Profile {
        mask: Mask::Volatile,
        observable: false,
    };
    /// `~` and `@` fields masked.
    pub const MASKED_CLOCK: Profile = Profile {
        mask: Mask::VolatileAndClock,
        observable: false,
    };

    /// The same profile with every `^` field removed.
    pub const fn observable(self) -> Profile {
        Profile {
            mask: self.mask,
            observable: true,
        }
    }
}

/// Why an export was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The 1-based line number (0 for the file as a whole).
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_canonical_unsigned(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
}

/// Checks that `v` is the canonical rendering of a value of type `ty`
/// (absent values are handled by the caller; the value set of an `e` field
/// is checked by the parser, see [`enum_values`]).
pub fn check_value(ty: ValueType, v: &str) -> Result<(), String> {
    let ok = match ty {
        ValueType::Unsigned => is_canonical_unsigned(v) && v.parse::<u64>().is_ok(),
        ValueType::Signed => {
            let digits = v.strip_prefix('-').unwrap_or(v);
            is_canonical_unsigned(digits) && v != "-0" && v.parse::<i64>().is_ok()
        }
        ValueType::Bool => v == "0" || v == "1",
        ValueType::Hash => v.len() == 64 && is_lower_hex(v),
        ValueType::Bytes => {
            v == "_" || (!v.is_empty() && v.len().is_multiple_of(2) && is_lower_hex(v))
        }
        ValueType::HashList => v == "_" || v.split(',').all(|h| h.len() == 64 && is_lower_hex(h)),
        ValueType::Enum => is_name(v),
        ValueType::Str => decode_str(v).is_some(),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{v:?} is not a canonical value of type {ty:?}"))
    }
}

/// Decodes a canonical `s` value to its bytes: `None` unless `v` is quoted,
/// every byte in 0x21-0x7e other than `"` and `\` stands for itself, and
/// every other byte (only those) is written `\xHH` in lower-case hex.
pub fn decode_str(v: &str) -> Option<Vec<u8>> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&c) = bytes.get(i) {
        if c == b'\\' {
            let esc = bytes.get(i..i.checked_add(4)?)?;
            if esc[1] != b'x' {
                return None;
            }
            let hex = std::str::from_utf8(&esc[2..4]).ok()?;
            if !is_lower_hex(hex) {
                return None;
            }
            let b = u8::from_str_radix(hex, 16).ok()?;
            if literal(b) {
                return None; // not canonical
            }
            out.push(b);
            i = i.checked_add(4)?;
        } else {
            if !literal(c) {
                return None;
            }
            out.push(c);
            i = i.checked_add(1)?;
        }
    }
    Some(out)
}

fn literal(b: u8) -> bool {
    (0x21..=0x7e).contains(&b) && b != b'"' && b != b'\\'
}

/// Encodes bytes as a canonical `s` value.
pub fn encode_str(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().saturating_add(2));
    s.push('"');
    for &c in b {
        if literal(c) {
            s.push(char::from(c));
        } else {
            s.push_str(&format!("\\x{c:02x}"));
        }
    }
    s.push('"');
    s
}

/// Decodes lower-case hex (`_` is the empty string).
pub fn decode_hex(v: &str) -> Option<Vec<u8>> {
    if v == "_" {
        return Some(Vec::new());
    }
    if !v.len().is_multiple_of(2) || !is_lower_hex(v) {
        return None;
    }
    (0..v.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(v.get(i..i.checked_add(2)?)?, 16).ok())
        .collect()
}

/// Decodes an `h` value to the hash's stored (internal) byte order.
pub fn decode_hash(v: &str) -> Option<[u8; 32]> {
    let b = decode_hex(v)?;
    let mut h: [u8; 32] = b.try_into().ok()?;
    h.reverse();
    Some(h)
}

/// Orders two rendered values of one field (ADR-0001 section 7): numbers
/// numerically, `s` by its bytes, anything else by its rendering; `_`
/// (empty) sorts before and `-` (absent) after every other value.
pub fn compare_values(ty: ValueType, a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    match (a, b) {
        ("-", _) => return Ordering::Greater,
        (_, "-") => return Ordering::Less,
        ("_", _) => return Ordering::Less,
        (_, "_") => return Ordering::Greater,
        _ => {}
    }
    match ty {
        ValueType::Unsigned | ValueType::Bool => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
        ValueType::Signed => {
            let x: i64 = a.parse().unwrap_or_default();
            let y: i64 = b.parse().unwrap_or_default();
            x.cmp(&y)
        }
        ValueType::Str => decode_str(a).cmp(&decode_str(b)),
        _ => a.cmp(b),
    }
}

/// Renders a record line, masked (profile masked) or not.
pub fn render(ty: &RecordType, values: &[String], masked: bool) -> String {
    render_with(
        ty,
        values,
        if masked {
            Profile::MASKED
        } else {
            Profile::EXACT
        },
    )
}

/// Renders a record line under `profile`: a masked `x` value becomes `*`
/// and its byte length, any other masked value `*`; `-` and `_` are never
/// masked; an observable profile leaves out every `^` field.
pub fn render_with(ty: &RecordType, values: &[String], profile: Profile) -> String {
    let mut line = ty.name.clone();
    for (f, v) in ty.fields.iter().zip(values) {
        if profile.observable && f.write_only {
            continue;
        }
        let masked = match profile.mask {
            Mask::None => false,
            Mask::Volatile => f.volatile,
            Mask::VolatileAndClock => f.volatile || f.clock,
        };
        line.push(' ');
        line.push_str(&f.name);
        line.push('=');
        if masked && v != "-" && v != "_" {
            if f.ty == ValueType::Bytes {
                line.push_str(&format!("*{}", v.len() / 2));
            } else {
                line.push('*');
            }
        } else {
            line.push_str(v);
        }
    }
    line
}

fn hex_digest(h: Sha256) -> String {
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn eof_hasher(metas: &[(String, String)]) -> Sha256 {
    let mut h = Sha256::new();
    for want in COMPARED_METAS {
        if let Some((k, v)) = metas.iter().find(|(k, _)| k == want) {
            h.update(format!("@{k}={v}\n").as_bytes());
        }
    }
    h
}

/// Parses and validates an export held in memory, keeping every record
/// (see the crate documentation for what is checked).
pub fn parse(text: &str) -> Result<Export, ParseError> {
    run(text.as_bytes(), true, &[]).map(|s| s.export)
}

/// Validates an export read line by line, keeping no records, and returns
/// its summary with the eof digest of each profile in `profiles`. Memory
/// is bounded by the longest line, not by the file.
pub fn validate<R: BufRead>(reader: R, profiles: &[Profile]) -> Result<Summary, ParseError> {
    run(reader, false, profiles)
}

fn err(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}

/// Reads lines, checking the lexical rules every line obeys.
struct Lines<R> {
    reader: R,
    n: usize,
    buf: Vec<u8>,
    peeked: Option<Option<(usize, String)>>,
}

impl<R: BufRead> Lines<R> {
    fn read(&mut self) -> Result<Option<(usize, String)>, ParseError> {
        self.buf.clear();
        let got = self
            .reader
            .read_until(b'\n', &mut self.buf)
            .map_err(|e| err(0, format!("read: {e}")))?;
        if got == 0 {
            if self.n == 0 {
                return Err(err(0, "the file does not end with LF"));
            }
            return Ok(None);
        }
        self.n = self.n.saturating_add(1);
        let n = self.n;
        if self.buf.pop() != Some(b'\n') {
            return Err(err(0, "the file does not end with LF"));
        }
        if self.buf.is_empty() {
            return Err(err(n, "empty line"));
        }
        if !self.buf.iter().all(|b| (0x20..=0x7e).contains(b)) {
            return Err(err(n, "a byte outside printable ASCII (or a CR)"));
        }
        if self.buf.ends_with(b" ") {
            return Err(err(n, "trailing space"));
        }
        let line = String::from_utf8(std::mem::take(&mut self.buf))
            .map_err(|_| err(n, "a byte outside printable ASCII"))?;
        Ok(Some((n, line)))
    }

    fn next(&mut self) -> Result<Option<(usize, String)>, ParseError> {
        match self.peeked.take() {
            Some(p) => Ok(p),
            None => self.read(),
        }
    }

    fn peek(&mut self) -> Result<Option<&(usize, String)>, ParseError> {
        if self.peeked.is_none() {
            let l = self.read()?;
            self.peeked = Some(l);
        }
        Ok(self.peeked.as_ref().and_then(Option::as_ref))
    }
}

/// The metas a section's records are checked against.
struct MetaRules {
    chain_full: bool,
    decrypt: bool,
    derived: bool,
    privcheck: bool,
}

fn check_meta_rules(
    rules: &MetaRules,
    section: &str,
    decl: &RecordType,
    values: &[String],
) -> Result<(), String> {
    let name = decl.name.as_str();
    if section == "derived" && !rules.derived {
        return Err("a [derived] record with @derived=0".to_owned());
    }
    if section == "privcheck" && !rules.privcheck {
        return Err("a [privcheck] record with @privcheck=0".to_owned());
    }
    if !rules.decrypt {
        for (t, f) in PLAINTEXT_FIELDS {
            if *t == name
                && let Some(i) = decl.field_index(f)
                && values[i] != "-"
            {
                return Err(format!("{name}.{f} has a value with @decrypt=none"));
            }
        }
    }
    let digest_only = ["blockseg", "hdrseg", "cfseg"].contains(&name);
    if rules.chain_full && digest_only {
        return Err(format!("a {name} record with @chain=full"));
    }
    if !rules.chain_full {
        let folded = match name {
            "header" | "cfilter" => true,
            "block" => {
                let v = |f: &str| decl.field_index(f).map(|i| values[i].as_str());
                v("txs") == Some("_") && v("inv") == Some("0")
            }
            _ => false,
        };
        if folded {
            return Err(format!(
                "a {name} record that digest mode folds into a segment, with @chain=digest"
            ));
        }
    }
    Ok(())
}

fn run<R: BufRead>(reader: R, keep: bool, profiles: &[Profile]) -> Result<Summary, ParseError> {
    let mut lines = Lines {
        reader,
        n: 0,
        buf: Vec::new(),
        peeked: None,
    };
    let need = |l: Option<(usize, String)>, what: &str| l.ok_or_else(|| err(0, what.to_owned()));

    let (n, l) = need(lines.next()?, "empty file")?;
    if l != MAGIC {
        return Err(err(n, format!("the first line is not {MAGIC:?}")));
    }
    let mut comments = Vec::new();
    while let Some((_, l)) = lines.peek()?.filter(|(_, l)| l.starts_with('#')) {
        comments.push(l.strip_prefix("# ").unwrap_or(&l[1..]).to_owned());
        lines.next()?;
    }
    let mut metas = Vec::new();
    for want in METAS {
        let (n, l) = need(lines.next()?, "missing metas")?;
        let (name, value) = l
            .strip_prefix('@')
            .and_then(|m| m.split_once('='))
            .ok_or_else(|| err(n, format!("expected meta @{want}")))?;
        if name != want {
            return Err(err(n, format!("expected meta @{want}, found @{name}")));
        }
        let allowed: &[&str] = match name {
            "network" => &["mainnet", "testnet3", "simnet", "regnet"],
            "chain" => &["full", "digest"],
            "decrypt" => &["none", "public"],
            "derived" | "privcheck" => &["0", "1"],
            _ => &[],
        };
        if value.is_empty()
            || value.contains(' ')
            || (!allowed.is_empty() && !allowed.contains(&value))
        {
            return Err(err(n, format!("bad value {value:?} for @{name}")));
        }
        metas.push((name.to_owned(), value.to_owned()));
    }
    let meta = |k: &str| {
        metas
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    };
    let rules = MetaRules {
        chain_full: meta("chain") == "full",
        decrypt: meta("decrypt") == "public",
        derived: meta("derived") == "1",
        privcheck: meta("privcheck") == "1",
    };
    if (rules.derived || rules.privcheck) && !rules.decrypt {
        return Err(err(
            0,
            "@derived=1 and @privcheck=1 need @decrypt=public".to_owned(),
        ));
    }
    if rules.privcheck && !rules.derived {
        return Err(err(0, "@privcheck=1 needs @derived=1".to_owned()));
    }

    let mut sections = Vec::new();
    let mut eof_raw = eof_hasher(&metas);
    let mut eof_masked = eof_hasher(&metas);
    let mut eof_profiles: Vec<Sha256> = profiles.iter().map(|_| eof_hasher(&metas)).collect();
    for spec in REGISTRY {
        let (name, version) = (spec.name, spec.version);
        let (n, l) = need(lines.next()?, &format!("missing section {name}"))?;
        if l != format!("=section {name} {version}") {
            return Err(err(
                n,
                format!("expected \"=section {name} {version}\", found {l:?}"),
            ));
        }
        let mut types = Vec::new();
        for decl in spec.types {
            let (n, l) = need(lines.next()?, &format!("missing types of {name}"))?;
            if l.strip_prefix("=type ") != Some(decl) {
                return Err(err(n, format!("expected \"=type {decl}\", found {l:?}")));
            }
            types.push(RecordType::parse(decl).map_err(|e| err(n, e))?);
        }
        let mut records: Vec<Record> = Vec::new();
        let mut prev: Option<Record> = None;
        let mut first: Option<usize> = None;
        let mut count = 0usize;
        let mut raw = Sha256::new();
        let mut masked = Sha256::new();
        let mut per_profile: Vec<Sha256> = profiles.iter().map(|_| Sha256::new()).collect();
        let end = loop {
            let (n, l) = need(lines.next()?, &format!("section {name} has no =end"))?;
            if l.starts_with("=end ") {
                break (n, l);
            }
            let mut tokens = l.split(' ');
            let rtype = tokens.next().unwrap_or_default();
            let ty = types
                .iter()
                .position(|t| t.name == rtype)
                .ok_or_else(|| err(n, format!("no record type {rtype:?} in section {name}")))?;
            let decl = &types[ty];
            let mut values = Vec::with_capacity(decl.fields.len());
            for f in &decl.fields {
                let tok = tokens
                    .next()
                    .ok_or_else(|| err(n, format!("{rtype}: missing field {}", f.name)))?;
                let (fname, v) = tok
                    .split_once('=')
                    .ok_or_else(|| err(n, format!("{rtype}: bad field {tok:?}")))?;
                if fname != f.name {
                    return Err(err(
                        n,
                        format!("{rtype}: expected field {}, found {fname}", f.name),
                    ));
                }
                if v == "-" {
                    if !f.optional {
                        return Err(err(n, format!("{rtype}.{} may not be absent", f.name)));
                    }
                } else {
                    check_value(f.ty, v).map_err(|e| err(n, format!("{rtype}.{}: {e}", f.name)))?;
                    if f.ty == ValueType::Enum
                        && !enum_values(rtype, &f.name).is_some_and(|a| a.contains(&v))
                    {
                        return Err(err(
                            n,
                            format!("{rtype}.{}: {v:?} is not one of the field's values", f.name),
                        ));
                    }
                }
                values.push(v.to_owned());
            }
            if tokens.next().is_some() {
                return Err(err(n, format!("{rtype}: extra fields")));
            }
            check_meta_rules(&rules, name, decl, &values).map_err(|e| err(n, e))?;
            if let Some(p) = &prev {
                match p.ty.cmp(&ty) {
                    Ordering::Greater => {
                        return Err(err(n, format!("record type {rtype} out of order")));
                    }
                    Ordering::Equal => {
                        if decl.keys == 0 {
                            return Err(err(n, format!("more than one {rtype} record")));
                        }
                        let mut ord = Ordering::Equal;
                        for (k, f) in decl.fields[..decl.keys].iter().enumerate() {
                            ord = compare_values(f.ty, &p.values[k], &values[k]);
                            if ord != Ordering::Equal {
                                break;
                            }
                        }
                        match ord {
                            Ordering::Equal => {
                                return Err(err(n, format!("{rtype}: duplicate key")));
                            }
                            Ordering::Greater => {
                                return Err(err(n, format!("{rtype} record out of order")));
                            }
                            Ordering::Less => {}
                        }
                    }
                    Ordering::Less => {}
                }
            }
            raw.update(l.as_bytes());
            raw.update(b"\n");
            masked.update(render_with(decl, &values, Profile::MASKED).as_bytes());
            masked.update(b"\n");
            for (h, p) in per_profile.iter_mut().zip(profiles) {
                h.update(render_with(decl, &values, *p).as_bytes());
                h.update(b"\n");
            }
            count = count.saturating_add(1);
            first.get_or_insert(ty);
            let rec = Record { ty, values };
            if keep {
                records.push(rec.clone());
            }
            prev = Some(rec);
        };
        let (n, l) = end;
        let (sha, msha) = (hex_digest(raw), hex_digest(masked));
        let want = format!("=end {name} records={count} sha256={sha} msha256={msha}");
        if l != want {
            return Err(err(n, format!("expected {want:?}, found {l:?}")));
        }
        // mkeyok is [privcheck]'s first declared type, so when present it
        // is the first record.
        if name == "privcheck" && rules.privcheck && first != Some(0) {
            return Err(err(n, "@privcheck=1 without an mkeyok record".to_owned()));
        }
        if spec.compared {
            eof_raw.update(format!("{name} {sha}\n").as_bytes());
            eof_masked.update(format!("{name} {msha}\n").as_bytes());
            for (eh, h) in eof_profiles.iter_mut().zip(per_profile) {
                eh.update(format!("{name} {}\n", hex_digest(h)).as_bytes());
            }
        }
        sections.push(Section {
            name: name.to_owned(),
            version,
            compared: spec.compared,
            types,
            records,
            count,
            sha256: sha,
            msha256: msha,
        });
    }
    let (sha, msha) = (hex_digest(eof_raw), hex_digest(eof_masked));
    let (n, l) = need(lines.next()?, "missing =eof")?;
    let want = format!(
        "=eof sections={} sha256={sha} msha256={msha}",
        REGISTRY.len()
    );
    if l != want {
        return Err(err(n, format!("expected {want:?}, found {l:?}")));
    }
    if let Some((n, _)) = lines.next()? {
        return Err(err(n, "content after =eof".to_owned()));
    }
    Ok(Summary {
        export: Export {
            comments,
            metas,
            sections,
            sha256: sha,
            msha256: msha,
        },
        digests: profiles
            .iter()
            .copied()
            .zip(eof_profiles.into_iter().map(hex_digest))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_declarations_parse() {
        for s in REGISTRY {
            assert!(is_name(s.name));
            for d in s.types {
                RecordType::parse(d).unwrap_or_else(|e| panic!("{d}: {e}"));
            }
        }
    }

    #[test]
    fn every_enum_field_has_values_and_every_value_list_a_field() {
        let mut fields = Vec::new();
        for s in REGISTRY {
            for d in s.types {
                let t = RecordType::parse(d).expect("decl");
                for f in t.fields.iter().filter(|f| f.ty == ValueType::Enum) {
                    let vals = enum_values(&t.name, &f.name)
                        .unwrap_or_else(|| panic!("no values for {}.{}", t.name, f.name));
                    assert!(vals.iter().all(|v| is_name(v) && *v != "-"));
                    fields.push(format!("{}.{}", t.name, f.name));
                }
            }
        }
        for (k, _) in ENUMS {
            assert!(fields.contains(&(*k).to_owned()), "{k} is not an e field");
        }
    }

    #[test]
    fn flags() {
        let t = RecordType::parse("k 1 a:u b:i?@ c:x?~^ d:u@^").expect("decl");
        assert!(t.fields[1].optional && t.fields[1].clock && !t.fields[1].volatile);
        assert!(t.fields[2].volatile && t.fields[2].write_only && !t.fields[2].clock);
        assert!(t.fields[3].clock && t.fields[3].write_only);
        for bad in [
            "k 1 a:u b:i~@",
            "k 1 a:u b:i@~",
            "k 1 a:u b:i^?",
            "k 1 a:u@",
            "k 1 a:u b:i??",
        ] {
            assert!(RecordType::parse(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn values_are_canonical() {
        use ValueType::*;
        for (ty, v) in [
            (Unsigned, "0"),
            (Unsigned, "18446744073709551615"),
            (Signed, "-1"),
            (Signed, "-9223372036854775808"),
            (Bool, "1"),
            (Bytes, "_"),
            (Bytes, "00ff"),
            (Str, "\"\""),
            (Str, "\"a\\x20b\\x22\\x5c\""),
            (Enum, "slip0044"),
            (HashList, "_"),
        ] {
            assert_eq!(check_value(ty, v), Ok(()), "{ty:?} {v}");
        }
        for (ty, v) in [
            (Unsigned, "01"),
            (Unsigned, "-1"),
            (Unsigned, "18446744073709551616"),
            (Signed, "-0"),
            (Signed, "+1"),
            (Bool, "2"),
            (Bytes, ""),
            (Bytes, "0"),
            (Bytes, "00FF"),
            (Str, "a"),
            (Str, "\"\\x61\""),
            (Str, "\"\\X20\""),
            (Str, "\"\\x2\""),
            (Enum, "A"),
            (Enum, "addr-account"),
            (Hash, "00"),
            (HashList, ""),
        ] {
            assert!(check_value(ty, v).is_err(), "{ty:?} {v} accepted");
        }
    }

    #[test]
    fn strings_round_trip() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_str(&encode_str(&all)), Some(all));
    }

    #[test]
    fn order_rules() {
        use ValueType::*;
        assert_eq!(compare_values(Unsigned, "9", "10"), Ordering::Less);
        assert_eq!(compare_values(Signed, "-2", "-10"), Ordering::Greater);
        assert_eq!(compare_values(Bytes, "_", "00"), Ordering::Less);
        assert_eq!(compare_values(Bytes, "00", "0000"), Ordering::Less);
        assert_eq!(compare_values(Unsigned, "-", "0"), Ordering::Greater);
        assert_eq!(compare_values(Str, "\"\\x20\"", "\"!\""), Ordering::Less);
    }

    #[test]
    fn masking_and_profiles() {
        let t = RecordType::parse("k 1 a:u b:x~ c:i?@ d:x?~ e:u^").expect("decl");
        let v: Vec<String> = ["1", "0011", "5", "_", "7"].map(String::from).to_vec();
        assert_eq!(render(&t, &v, true), "k a=1 b=*2 c=5 d=_ e=7");
        assert_eq!(render(&t, &v, false), "k a=1 b=0011 c=5 d=_ e=7");
        assert_eq!(
            render_with(&t, &v, Profile::MASKED_CLOCK),
            "k a=1 b=*2 c=* d=_ e=7"
        );
        assert_eq!(
            render_with(&t, &v, Profile::EXACT.observable()),
            "k a=1 b=0011 c=5 d=_"
        );
    }
}
