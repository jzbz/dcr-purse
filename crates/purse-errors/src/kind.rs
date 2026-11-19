// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The class of an error (Go's `errors.Kind`).

use std::fmt;

/// The text Go's `Kind.String` returns for a value outside the defined
/// kinds.
pub const UNKNOWN_KIND_TEXT: &str = "unknown error kind";

/// Describes the class of error (Go's `errors.Kind`).
///
/// The numeric values match Go's constants (`Other` is 0, `Deployment` 20).
/// Go's `Kind` is an `int`, so values outside that range exist there and
/// render as [`UNKNOWN_KIND_TEXT`]; no dcrwallet code creates one, and here
/// they cannot be represented ([`Kind::describe_code`] gives their text).
///
/// A `Kind` is also usable as an error value on its own, as in Go: it
/// implements [`std::error::Error`], and [`AnyError::Kind`](crate::AnyError)
/// holds one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Kind {
    /// Unclassified error; does not appear in error strings.
    #[default]
    Other = 0,
    /// Error is known to be a result of a wallet bug.
    Bug = 1,
    /// Invalid operation.
    Invalid = 2,
    /// Permission denied.
    Permission = 3,
    /// I/O error (Go's `IO`).
    Io = 4,
    /// Item already exists.
    Exist = 5,
    /// Item does not exist.
    NotExist = 6,
    /// Invalid encoding.
    Encoding = 7,
    /// Encryption or decryption error.
    Crypto = 8,
    /// Wallet is locked.
    Locked = 9,
    /// Invalid passphrase.
    Passphrase = 10,
    /// Invalid seed.
    Seed = 11,
    /// Missing private keys.
    WatchingOnly = 12,
    /// Insufficient balance to create a transaction (perhaps due to UTXO
    /// selection requirements).
    InsufficientBalance = 13,
    /// Transaction scripts do not execute (usually due to missing
    /// signatures).
    ScriptFailure = 14,
    /// Transaction rejected by wallet policy.
    Policy = 15,
    /// Consensus violation.
    Consensus = 16,
    /// Transaction is a double spend.
    DoubleSpend = 17,
    /// Protocol violation.
    Protocol = 18,
    /// Decred network is unreachable due to lack of peers or dcrd RPC
    /// connections.
    NoPeers = 19,
    /// Inactive consensus deployment.
    Deployment = 20,
}

impl Kind {
    /// Every kind, in numeric order.
    pub const ALL: [Kind; 21] = [
        Kind::Other,
        Kind::Bug,
        Kind::Invalid,
        Kind::Permission,
        Kind::Io,
        Kind::Exist,
        Kind::NotExist,
        Kind::Encoding,
        Kind::Crypto,
        Kind::Locked,
        Kind::Passphrase,
        Kind::Seed,
        Kind::WatchingOnly,
        Kind::InsufficientBalance,
        Kind::ScriptFailure,
        Kind::Policy,
        Kind::Consensus,
        Kind::DoubleSpend,
        Kind::Protocol,
        Kind::NoPeers,
        Kind::Deployment,
    ];

    /// The Go constant's numeric value.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The kind with Go numeric value `code`, if it is one of the defined
    /// kinds.
    pub fn from_code(code: i64) -> Option<Kind> {
        usize::try_from(code)
            .ok()
            .and_then(|i| Kind::ALL.get(i).copied())
    }

    /// The fixed text of the kind (Go's `Kind.String` and `Kind.Error`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Other => "unclassified error",
            Kind::Bug => "internal wallet error",
            Kind::Invalid => "invalid operation",
            Kind::Permission => "permission denied",
            Kind::Io => "I/O error",
            Kind::Exist => "item already exists",
            Kind::NotExist => "item does not exist",
            Kind::Encoding => "invalid encoding",
            Kind::Crypto => "encryption/decryption error",
            Kind::Locked => "wallet locked",
            Kind::Passphrase => "invalid passphrase",
            Kind::Seed => "invalid seed",
            Kind::WatchingOnly => "watching only wallet",
            Kind::InsufficientBalance => "insufficient balance",
            Kind::ScriptFailure => "transaction script fails to execute",
            Kind::Policy => "policy violation",
            Kind::Consensus => "consensus violation",
            Kind::DoubleSpend => "double spend",
            Kind::Protocol => "protocol violation",
            Kind::NoPeers => "Decred network is unreachable",
            Kind::Deployment => "inactive deployment",
        }
    }

    /// The text Go's `Kind(code).String()` returns for any integer: the
    /// kind's text for a defined kind and [`UNKNOWN_KIND_TEXT`] otherwise.
    pub fn describe_code(code: i64) -> &'static str {
        Kind::from_code(code).map_or(UNKNOWN_KIND_TEXT, Kind::as_str)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for Kind {}
