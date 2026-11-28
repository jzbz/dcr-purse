// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The auxiliary records' operations of the Rust runner (VSP tickets,
//! treasury policies, agenda preferences), with Go's arguments and result
//! shapes (`tools/oracle/cmd/oprunner/ops_aux.go`), over
//! [`purse_udb::vsp`], [`purse_udb::treasury`] and [`purse_udb::agendas`].
//!
//! `udb.set_vsp_host` and `udb.set_vsp_pub_key` are not among them: they
//! are not ported (ADR-0003 R2; `purse_udb::vsp`, "Divergences"), and the
//! runner refuses them as it refuses every operation not ported.
//!
//! Two renderings follow Go's runner: listings, which Go returns as maps,
//! are sorted by key as `oprunner` sorts them (hashes in display order,
//! pi keys bytewise), and `GetVSPHost`'s missing-id text, where Go prints
//! a pointer and dcr-purse the id (QK-0123), has the id rewritten to
//! `<pointer>` as `oprunner`'s `canonPointer` rewrites the address.

use purse_errors::AnyError;
use purse_testutil::hex;
use purse_testutil::serde_json::{Value, json};
use purse_udb::vsp::VspTicket;
use purse_udb::{agendas, treasury, vsp};

use super::txops::hs;
use super::{Args, Cx, OpResult, Runner, fault};

/// This module's operations.
pub const AUX_OPS: &[&str] = &[
    "udb.set_vsp_ticket",
    "udb.delete_vsp_ticket",
    "udb.get_vsp_ticket",
    "udb.get_vsp_tickets_by_fee_status",
    "udb.get_vsp_host",
    "udb.get_vsp_pub_key",
    "udb.set_tspend_policy",
    "udb.set_vsp_tspend_policy",
    "udb.tspend_policy",
    "udb.vsp_tspend_policy",
    "udb.tspend_policies",
    "udb.vsp_tspend_policies",
    "udb.set_treasury_key_policy",
    "udb.set_vsp_treasury_key_policy",
    "udb.treasury_key_policy",
    "udb.vsp_treasury_key_policy",
    "udb.treasury_key_policies",
    "udb.vsp_treasury_key_policies",
    "udb.set_default_agenda_preference",
    "udb.set_ticket_agenda_preference",
    "udb.default_agenda_preference",
    "udb.ticket_agenda_preference",
];

/// The operations of this module that need a read-write transaction (Go's
/// `writeTx` kind).
pub const AUX_CHANGING: &[&str] = &[
    "udb.set_vsp_ticket",
    "udb.delete_vsp_ticket",
    "udb.set_tspend_policy",
    "udb.set_vsp_tspend_policy",
    "udb.set_treasury_key_policy",
    "udb.set_vsp_treasury_key_policy",
    "udb.set_default_agenda_preference",
    "udb.set_ticket_agenda_preference",
];

/// Whether `name` is one of this module's operations.
pub fn handles(name: &str) -> bool {
    AUX_OPS.contains(&name)
}

/// A JSON string of bytes that came from a JSON string (lossless here;
/// Go's encoder writes U+FFFD for invalid UTF-8 as this does).
fn text(b: &[u8]) -> Value {
    json!(String::from_utf8_lossy(b))
}

/// Go's `vspTicketValue`.
fn ticket_value(t: &VspTicket) -> Value {
    json!({
        "feehash": hs(&t.fee_hash), "feetxstatus": t.fee_tx_status,
        "vsphostid": t.vsp_host_id, "host": text(&t.host), "pubkey": hex(&t.pub_key),
    })
}

/// `canonPointer`: `no VSP host info for id <n>` with `<pointer>` for the
/// id, the kind kept.
fn canon_pointer(err: AnyError) -> AnyError {
    const MARKER: &str = "no VSP host info for id ";
    let text = err.render(purse_errors::DAEMON_SEPARATOR);
    let Some(i) = text.find(MARKER) else {
        return err;
    };
    let rest = &text[i.saturating_add(MARKER.len())..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    // The kind's own text ("item does not exist") comes back with the
    // kind; what follows it is the message.
    let message = format!("{MARKER}<pointer>{}", &rest[digits..]);
    match err.as_kind() {
        Some(k) => purse_errors::e!(k, message).into(),
        None => AnyError::new(format!("{}{message}", &text[..i])),
    }
}

impl Runner {
    fn policy_arg(a: &Args<'_>) -> Result<u8, String> {
        a.value("policy")?
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| fault(format!("{}: argument \"policy\" out of range", a.op)))
    }

    /// Runs one auxiliary record operation.
    pub(super) fn aux_op(&mut self, cx: &mut Cx<'_, '_>, a: &Args<'_>) -> Result<OpResult, String> {
        let ok_null = |r: Result<(), AnyError>| r.map(|()| Value::Null);
        Ok(match a.op {
            "udb.set_vsp_ticket" => {
                a.check(&[
                    "ticket",
                    "fee_hash",
                    "fee_tx_status",
                    "vsp_host_id",
                    "host",
                    "pubkey",
                ])?;
                let ticket = self.arg_hash(a, "ticket")?;
                let record = VspTicket {
                    fee_hash: self.arg_hash(a, "fee_hash")?,
                    fee_tx_status: a.u32("fee_tx_status")?,
                    // Read as Go reads it, then ignored by SetVSPTicket.
                    vsp_host_id: Self::opt_u32(a, "vsp_host_id", 0)?,
                    host: a.str("host")?.as_bytes().to_vec(),
                    pub_key: a.hex("pubkey")?,
                };
                self.update(cx, |tx| vsp::set_vsp_ticket(tx, &ticket, &record))?
                    .map(|id| json!({"vsphostid": id}))
            }
            "udb.delete_vsp_ticket" => {
                a.check(&["ticket"])?;
                let ticket = self.arg_hash(a, "ticket")?;
                ok_null(self.update(cx, |tx| vsp::delete_vsp_ticket(tx, &ticket))?)
            }
            "udb.get_vsp_ticket" => {
                a.check(&["ticket"])?;
                let ticket = self.arg_hash(a, "ticket")?;
                self.view(cx, |r| vsp::get_vsp_ticket(r, &ticket))?
                    .map(|t| ticket_value(&t))
            }
            "udb.get_vsp_tickets_by_fee_status" => {
                a.check(&["status"])?;
                let status = a.i64("status")?;
                if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&status) {
                    return Err(fault(format!("{}: argument \"status\" out of range", a.op)));
                }
                self.view(cx, |r| vsp::get_vsp_tickets_by_fee_status(r, status))?
                    .map(|mut list| {
                        list.sort_by_key(|(h, _)| hs(h));
                        Value::Array(
                            list.iter()
                                .map(|(h, t)| {
                                    let mut v = ticket_value(t);
                                    v["ticket"] = json!(hs(h));
                                    v
                                })
                                .collect(),
                        )
                    })
            }
            "udb.get_vsp_host" => {
                a.check(&["id"])?;
                let id = a.u32("id")?;
                self.view(cx, |r| vsp::get_vsp_host(r, id))?
                    .map(|h| json!(hex(&h)))
                    .map_err(canon_pointer)
            }
            "udb.get_vsp_pub_key" => {
                a.check(&["host"])?;
                let host = a.str("host")?.as_bytes().to_vec();
                self.view(cx, |r| vsp::get_vsp_pub_key(r, &host))?
                    .map(|(id, pk)| json!({"id": id, "pubkey": hex(&pk)}))
            }
            "udb.set_tspend_policy" => {
                a.check(&["tspend", "policy"])?;
                let h = self.arg_hash(a, "tspend")?;
                let p = Self::policy_arg(a)?;
                ok_null(self.update(cx, |tx| treasury::set_tspend_policy(tx, &h, p))?)
            }
            "udb.set_vsp_tspend_policy" => {
                a.check(&["ticket", "tspend", "policy"])?;
                let t = self.arg_hash(a, "ticket")?;
                let h = self.arg_hash(a, "tspend")?;
                let p = Self::policy_arg(a)?;
                ok_null(self.update(cx, |tx| treasury::set_vsp_tspend_policy(tx, &t, &h, p))?)
            }
            "udb.tspend_policy" => {
                a.check(&["tspend"])?;
                let h = self.arg_hash(a, "tspend")?;
                self.view(cx, |r| treasury::tspend_policy(r, &h))?
                    .map(|p| json!(p))
            }
            "udb.vsp_tspend_policy" => {
                a.check(&["ticket", "tspend"])?;
                let t = self.arg_hash(a, "ticket")?;
                let h = self.arg_hash(a, "tspend")?;
                self.view(cx, |r| treasury::vsp_tspend_policy(r, &t, &h))?
                    .map(|p| json!(p))
            }
            "udb.tspend_policies" => {
                a.check(&[])?;
                self.view(cx, |r| treasury::tspend_policies(r))?
                    .map(|mut list| {
                        list.sort_by_key(|(h, _)| hs(h));
                        Value::Array(
                            list.iter()
                                .map(|(h, p)| json!({"tspend": hs(h), "policy": p}))
                                .collect(),
                        )
                    })
            }
            "udb.vsp_tspend_policies" => {
                a.check(&[])?;
                self.view(cx, |r| treasury::vsp_tspend_policies(r))?
                    .map(|mut list| {
                        list.sort_by_key(|(k, _)| (hs(&k.ticket), hs(&k.tspend)));
                        Value::Array(
                            list.iter()
                                .map(|(k, p)| {
                                    json!({"ticket": hs(&k.ticket), "tspend": hs(&k.tspend),
                                           "policy": p})
                                })
                                .collect(),
                        )
                    })
            }
            "udb.set_treasury_key_policy" => {
                a.check(&["pikey", "policy"])?;
                let k = a.hex("pikey")?;
                let p = Self::policy_arg(a)?;
                ok_null(self.update(cx, |tx| treasury::set_treasury_key_policy(tx, &k, p))?)
            }
            "udb.set_vsp_treasury_key_policy" => {
                a.check(&["ticket", "pikey", "policy"])?;
                let t = self.arg_hash(a, "ticket")?;
                let k = a.hex("pikey")?;
                let p = Self::policy_arg(a)?;
                ok_null(self.update(cx, |tx| {
                    treasury::set_vsp_treasury_key_policy(tx, &t, &k, p)
                })?)
            }
            "udb.treasury_key_policy" => {
                a.check(&["pikey"])?;
                let k = a.hex("pikey")?;
                self.view(cx, |r| treasury::treasury_key_policy(r, &k))?
                    .map(|p| json!(p))
            }
            "udb.vsp_treasury_key_policy" => {
                a.check(&["ticket", "pikey"])?;
                let t = self.arg_hash(a, "ticket")?;
                let k = a.hex("pikey")?;
                self.view(cx, |r| treasury::vsp_treasury_key_policy(r, &t, &k))?
                    .map(|p| json!(p))
            }
            "udb.treasury_key_policies" => {
                a.check(&[])?;
                // Bytewise key order is Go's sort of its string keys.
                self.view(cx, |r| treasury::treasury_key_policies(r))?
                    .map(|list| {
                        Value::Array(
                            list.iter()
                                .map(|(k, p)| json!({"pikey": hex(k), "policy": p}))
                                .collect(),
                        )
                    })
            }
            "udb.vsp_treasury_key_policies" => {
                a.check(&[])?;
                self.view(cx, |r| treasury::vsp_treasury_key_policies(r))?
                    .map(|mut list| {
                        list.sort_by(|(x, _), (y, _)| {
                            (hs(&x.ticket), &x.pikey).cmp(&(hs(&y.ticket), &y.pikey))
                        });
                        Value::Array(
                            list.iter()
                                .map(|(k, p)| {
                                    json!({"ticket": hs(&k.ticket), "pikey": hex(&k.pikey),
                                           "policy": p})
                                })
                                .collect(),
                        )
                    })
            }
            "udb.set_default_agenda_preference" => {
                a.check(&["version", "agenda", "choice"])?;
                let v = a.u32("version")?;
                let agenda = a.str("agenda")?.as_bytes().to_vec();
                let choice = a.str("choice")?.as_bytes().to_vec();
                ok_null(self.update(cx, |tx| {
                    agendas::set_default_agenda_preference(tx, v, &agenda, &choice)
                })?)
            }
            "udb.set_ticket_agenda_preference" => {
                a.check(&["ticket", "version", "agenda", "choice"])?;
                let t = self.arg_hash(a, "ticket")?;
                let v = a.u32("version")?;
                let agenda = a.str("agenda")?.as_bytes().to_vec();
                let choice = a.str("choice")?.as_bytes().to_vec();
                ok_null(self.update(cx, |tx| {
                    agendas::set_ticket_agenda_preference(tx, &t, v, &agenda, &choice)
                })?)
            }
            "udb.default_agenda_preference" => {
                a.check(&["version", "agenda"])?;
                let v = a.u32("version")?;
                let agenda = a.str("agenda")?.as_bytes().to_vec();
                self.view(cx, |r| agendas::default_agenda_preference(r, v, &agenda))?
                    .map(|c| text(&c))
            }
            "udb.ticket_agenda_preference" => {
                a.check(&["ticket", "version", "agenda"])?;
                let t = self.arg_hash(a, "ticket")?;
                let v = a.u32("version")?;
                let agenda = a.str("agenda")?.as_bytes().to_vec();
                self.view(cx, |r| agendas::ticket_agenda_preference(r, &t, v, &agenda))?
                    .map(|c| text(&c))
            }
            other => return Err(fault(format!("{other}: not an auxiliary operation"))),
        })
    }
}
