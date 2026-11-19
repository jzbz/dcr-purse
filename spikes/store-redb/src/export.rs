//! The canonical export both sides write for the correctness cross-check.
//! The Go side's `export` command (tools/oracle/cmd/storebench) writes the
//! same lines from dcrwallet's own udb; the two files must be identical.

use purse_testutil::{SplitMix64, hex};
use redb::ReadableTable as _;
use sha2::{Digest as _, Sha256};

use crate::store::{BoltCursor, R, Result};

/// A range query, as both sides take it.
#[derive(Clone, Debug)]
pub struct RangeSpec {
    /// Label.
    pub name: String,
    /// `RangeTransactions` begin.
    pub begin: i32,
    /// `RangeTransactions` end.
    pub end: i32,
    /// Stop after this many transactions (0: no limit).
    pub limit: usize,
}

/// A per-account history query.
#[derive(Clone, Debug)]
pub struct ByAccountSpec {
    /// Label.
    pub name: String,
    /// Account.
    pub account: u32,
    /// How many transactions.
    pub limit: usize,
}

/// What the export covers beyond the fixed sections.
#[derive(Clone, Debug, Default)]
pub struct ExportSpec {
    /// Confirmation depths for the balance section.
    pub minconfs: Vec<i32>,
    /// Cursor probe keys (hex).
    pub probes: Vec<String>,
    /// randomUTXO scan keys (hex).
    pub scans: Vec<String>,
    /// Range queries.
    pub ranges: Vec<RangeSpec>,
    /// Per-account history queries.
    pub by_account: Vec<ByAccountSpec>,
    /// Accounts whose eligible outputs are selected (`0..n`).
    pub select_accounts: u32,
    /// List every selected outpoint, not only the count, total and digest.
    pub select_detail: bool,
}

/// Selection targets the export uses: 0 (the full eligible list, no random
/// phase) and one no balance reaches (every eligible output, whatever the
/// random draws), so the result does not depend on randomness.
pub const SELECT_TARGETS: [i64; 2] = [0, i64::MAX];
/// Confirmation depths the selection section uses.
pub const SELECT_MINCONFS: [i32; 3] = [0, 1, 6];

fn dash(e: &Option<(Vec<u8>, Vec<u8>)>) -> String {
    match e {
        Some((k, _)) => hex(k),
        None => "-".to_owned(),
    }
}

/// Writes the export lines for the store `r` reads.
pub fn export(r: &R, spec: &ExportSpec) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let (tip, tip_hash) = r.tip()?;
    lines.push(format!("tip {tip} {}", hex(&tip_hash)));
    lines.push(format!("keyn {}", r.unspent_n()?));
    for (a, v) in r.account_list()? {
        lines.push(format!(
            "account {a} {} {} {} {}",
            v.last_used_ext, v.last_used_int, v.last_ret_ext, v.last_ret_int
        ));
    }
    for &mc in &spec.minconfs {
        for (a, b) in r.account_balances(mc, tip)? {
            lines.push(format!(
                "balance {mc} {a} {} {} {} {} {} {} {}",
                b.spendable,
                b.immature_coinbase,
                b.immature_stake,
                b.locked,
                b.voting_authority,
                b.unconfirmed,
                b.total
            ));
        }
    }
    let utxos = r.unspent_outputs()?;
    lines.push(format!("unspent {}", utxos.len()));
    for u in &utxos {
        lines.push(format!(
            "utxo {} {} {} {} {} {} {}",
            hex(&u.outpoint[..32]),
            u32::from_be_bytes(u.outpoint[32..].try_into().expect("4 bytes")),
            u.tree,
            u.height,
            u.amount,
            u.coinbase,
            hex(&u.script)
        ));
    }
    for p in &spec.probes {
        let k = purse_testutil::unhex(p);
        let mut c = BoltCursor::new(r.unspent()?);
        let mut fwd = vec![dash(&c.seek(&k)?)];
        for i in 0..5 {
            let e = if i < 3 { c.next()? } else { c.prev()? };
            fwd.push(dash(&e));
        }
        c.seek(&k)?;
        let mut back = Vec::new();
        for i in 0..5 {
            let e = if i < 3 { c.prev()? } else { c.next()? };
            back.push(dash(&e));
        }
        lines.push(format!("probe {p} {} | {}", fwd.join(" "), back.join(" ")));
    }
    let tip_i = tip as i32;
    let mut skip = |k: &[u8], v: &[u8]| -> Result<bool> {
        if r.unmined_inputs()?.get(k)?.is_some() || v.len() < 4 {
            return Ok(true);
        }
        let h = u32::from_be_bytes(v[..4].try_into().expect("4 bytes")) as i32;
        Ok(!crate::chain::confirmed(1, h, tip_i))
    };
    for s in &spec.scans {
        let k = purse_testutil::unhex(s);
        let next = r.scan_unspent(&k, false, &mut skip)?;
        let prev = r.scan_unspent(&k, true, &mut skip)?;
        let join = |v: &[[u8; 36]]| v.iter().map(|k| hex(k)).collect::<Vec<_>>().join(",");
        lines.push(format!(
            "scan {s} next {} prev {}",
            join(&next),
            join(&prev)
        ));
    }
    for rs in &spec.ranges {
        lines.push(format!(
            "range {} {} {} {}",
            rs.name, rs.begin, rs.end, rs.limit
        ));
        let mut n = 0usize;
        r.range_transactions(rs.begin, rs.end, |details| {
            lines.push(format!("batch {}", details.len()));
            for d in details {
                let mut s = format!("tx {} {}", hex(&d.hash), d.height);
                for (i, a, spent, change) in &d.credits {
                    s.push_str(&format!(" c{i}:{a}:{spent}:{change}"));
                }
                for (i, a) in &d.debits {
                    s.push_str(&format!(" d{i}:{a}"));
                }
                lines.push(s);
            }
            n += details.len();
            rs.limit > 0 && n >= rs.limit
        })?;
    }
    for b in &spec.by_account {
        let hashes = r.by_account(b.account, b.limit)?;
        let parts: Vec<String> = hashes.iter().map(|h| hex(h)).collect();
        lines.push(format!(
            "by_account {} {} {} {}",
            b.name,
            b.account,
            b.limit,
            parts.join(",")
        ));
    }
    for (h, hash) in r.invalid_blocks()? {
        lines.push(format!("invalid {h} {}", hex(&hash)));
    }
    for (hash, h, i, amount, spent) in r.invalidated_credits()? {
        lines.push(format!(
            "inv_credit {} {h} {i} {amount} {spent}",
            hex(&hash)
        ));
    }
    for (hash, h, i, amount, op) in r.invalidated_debits()? {
        lines.push(format!(
            "inv_debit {} {h} {i} {amount} {} {}",
            hex(&hash),
            hex(&op[..32]),
            u32::from_be_bytes(op[32..].try_into().expect("4 bytes"))
        ));
    }
    let mut walk = Vec::new();
    r.for_each_unspent_outpoint(|k, tree| walk.push((k.to_vec(), tree)))?;
    lines.push(format!("foreach {}", walk.len()));
    for (k, tree) in &walk {
        lines.push(format!(
            "fe {} {} {tree}",
            hex(&k[..32]),
            u32::from_be_bytes(k[32..36].try_into().expect("4 bytes"))
        ));
    }
    for acct in 0..spec.select_accounts {
        for mc in SELECT_MINCONFS {
            for target in SELECT_TARGETS {
                let mut src =
                    r.input_source(acct, mc, tip_i, SplitMix64(0x5e1ec7 ^ u64::from(acct)))?;
                let d = src.select(target)?;
                let mut sel: Vec<String> = d
                    .inputs
                    .iter()
                    .map(|(op, tree, amount)| {
                        format!(
                            "{} {} {tree} {amount}",
                            hex(&op[..32]),
                            u32::from_be_bytes(op[32..].try_into().expect("4 bytes"))
                        )
                    })
                    .collect();
                sel.sort();
                let digest = Sha256::digest(sel.join("\n").as_bytes());
                lines.push(format!(
                    "select {acct} {mc} {target} {} {} {}",
                    sel.len(),
                    d.amount,
                    hex(&digest)
                ));
                if spec.select_detail {
                    lines.extend(sel.into_iter().map(|l| format!("sel {l}")));
                }
            }
        }
    }
    for (a, _) in r.account_list()? {
        let addrs = r.account_addresses(a)?;
        let mut h = Sha256::new();
        for x in &addrs {
            h.update(x);
        }
        lines.push(format!("addrs {a} {} {}", addrs.len(), hex(&h.finalize())));
    }
    Ok(lines)
}
