// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A consistency checker of the record tables written for the tests,
//! apart from the store's own `ReadTx::check_records`: it reads the six
//! indexed record tables as raw bytes and decodes the few fields it needs
//! by hand from the documented layouts (a multisig output starts with its
//! script's 20-byte HASH160 and has its flags byte at 22, bit 0 spent; a
//! VSP ticket has its fee status big-endian at bytes 32 to 36 and its host
//! id at 36 to 40; a public-key row starts with its host id), and checks
//! both directions of every correspondence:
//!
//! - every unspent multisig output has its `multisig_unspent` entry
//!   (script ‖ outpoint), and every entry names an unspent output of that
//!   script;
//! - every VSP ticket has its `vsp_fee_status` entry (status ‖ ticket), and
//!   every entry names a ticket of that status;
//! - every VSP host id names a host whose public-key row names that id,
//!   and every public-key row's id names that host; no id is 0 and no host
//!   empty;
//! - every ticket's host id is 0 or a host's.
//!
//! It works on any source of raw rows, so the model test runs it on its
//! model as well as on the store. On success it returns a [`RecordsView`].

use std::collections::BTreeMap;
use std::ops::Bound;

use purse_store::{ReadTx, TableId};

/// A table's rows, raw, in key order.
pub type Rows = BTreeMap<Vec<u8>, Vec<u8>>;

/// What the indexed record tables hold, once found consistent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordsView {
    /// Unspent multisig outpoints (36 bytes each) by script hash, in
    /// dcrwallet's `mu` order (the outpoints' byte order).
    pub unspent_multisig: BTreeMap<[u8; 20], Vec<Vec<u8>>>,
    /// Ticket hashes by fee status.
    pub by_status: BTreeMap<u32, Vec<[u8; 32]>>,
    /// Hosts by id.
    pub hosts: BTreeMap<u32, Vec<u8>>,
}

/// The view of the record tables a read transaction sees.
pub fn records_view(tx: &ReadTx) -> Result<RecordsView, String> {
    records_view_of(|id| {
        tx.range_raw(id, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
            .map(|e| e.expect("entry"))
            .collect()
    })
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b.try_into().expect("4 bytes"))
}

/// The view of the record tables `rows` returns, or the first
/// inconsistency found.
pub fn records_view_of(rows: impl Fn(TableId) -> Rows) -> Result<RecordsView, String> {
    let mut view = RecordsView::default();
    let outs = rows(TableId::MultisigOuts);
    let index = rows(TableId::MultisigUnspent);
    for (op, v) in &outs {
        if op.len() != 36 || v.len() != 103 {
            return Err(format!(
                "multisig row of key {} value {} bytes",
                op.len(),
                v.len()
            ));
        }
        if v[22] & 1 == 0 {
            let mut k = v[..20].to_vec();
            k.extend_from_slice(op);
            if !index.contains_key(&k) {
                return Err("an unspent multisig output has no index entry".into());
            }
        }
    }
    for k in index.keys() {
        if k.len() != 56 {
            return Err(format!("multisig_unspent key of {} bytes", k.len()));
        }
        let (sh, op) = k.split_at(20);
        match outs.get(op) {
            Some(v) if v[..20] == *sh && v[22] & 1 == 0 => {}
            _ => {
                return Err(
                    "a multisig_unspent entry names no unspent output of its script".into(),
                );
            }
        }
        view.unspent_multisig
            .entry(sh.try_into().expect("20 bytes"))
            .or_default()
            .push(op.to_vec());
    }

    let tickets = rows(TableId::VspTickets);
    let status = rows(TableId::VspFeeStatus);
    let hosts = rows(TableId::VspHosts);
    let pubkeys = rows(TableId::VspPubKeys);
    for (t, v) in &tickets {
        if t.len() != 32 || v.len() != 40 {
            return Err("a VSP ticket row of the wrong size".into());
        }
        let mut k = v[32..36].to_vec();
        k.extend_from_slice(t);
        if !status.contains_key(&k) {
            return Err("a VSP ticket has no fee status entry".into());
        }
        let host = be32(&v[36..40]);
        if host != 0 && !hosts.contains_key(&host.to_be_bytes()[..]) {
            return Err(format!("a VSP ticket names host id {host} with no host"));
        }
    }
    for k in status.keys() {
        if k.len() != 36 {
            return Err("a vsp_fee_status key of the wrong size".into());
        }
        let (st, t) = k.split_at(4);
        match tickets.get(t) {
            Some(v) if v[32..36] == *st => {}
            _ => return Err("a vsp_fee_status entry names no ticket of its status".into()),
        }
        view.by_status
            .entry(be32(st))
            .or_default()
            .push(t.try_into().expect("32 bytes"));
    }
    for (id, host) in &hosts {
        if id.len() != 4 || be32(id) == 0 || host.is_empty() {
            return Err("a VSP host row with a bad id or an empty host".into());
        }
        match pubkeys.get(host) {
            Some(p) if p.len() >= 4 && p[..4] == id[..] => {}
            _ => {
                return Err(format!(
                    "VSP host id {} has no public-key row naming it",
                    be32(id)
                ));
            }
        }
        view.hosts.insert(be32(id), host.clone());
    }
    for (host, p) in &pubkeys {
        if p.len() < 4 {
            return Err("a VSP public-key row shorter than its id".into());
        }
        match hosts.get(&p[..4]) {
            Some(h) if h == host => {}
            _ => return Err("a VSP public-key row's id names another host".into()),
        }
    }
    Ok(view)
}
