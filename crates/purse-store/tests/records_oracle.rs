// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The VSP, treasury-policy and agenda-preference records against
//! dcrwallet's own `wallet/udb` through the `oprunner` oracle: generated
//! operation scripts of `udb.set_vsp_ticket`, `udb.get_vsp_ticket`,
//! `udb.delete_vsp_ticket`, `udb.get_vsp_tickets_by_fee_status`,
//! `udb.get_vsp_host`, `udb.get_vsp_pub_key`, the twelve treasury policy
//! operations and the four agenda preference operations (each in its own
//! transaction, as `oprunner` runs a top-level operation) run on Go's udb
//! over bbolt, and through a small interpreter here over the store's record
//! tables and operations (`purse_store::records`); every operation's result
//! or error must be equal.
//!
//! The interpreter stands where `purse-udb` will (Phase 3): it formats
//! udb's error texts and builds udb's result values from what the store
//! returns, and answers `GetVSPHost(0)` with the empty host that
//! dcrwallet's upgrade 23 seeds and the store does not keep (ADR-0003,
//! addendum "The remaining records, as built"). What the comparison holds
//! of the store itself: host ids given out as dcrwallet gives them (1, 2,
//! 3 in order of first use, a known host keeping its id and first public
//! key), every value read back as written, deletes and the "no policy"
//! value 0, a refused empty host or pi key leaving nothing, and the sets
//! the listings return (Go returns maps; `oprunner` sorts them, and so
//! does the interpreter). `udb.set_vsp_host` and `udb.set_vsp_pub_key`,
//! which write one of the two host tables without the other, are not
//! drawn: no caller at the pin uses them outside `SetVSPTicket`, and the
//! store keeps the two together. The multisig records are not reached:
//! `oprunner`'s multisig operations need transaction records and imported
//! scripts, which belong to the transaction store of Phase 3; their one
//! observable order is held against bbolt in `tests/conformance.rs`.
//!
//! `PURSE_TEST_SEED` seeds the scripts; `PURSE_STORE_RECORD_SCRIPTS` sets
//! their number (default 12).

#![allow(clippy::arithmetic_side_effects, reason = "test code: small counters")]

use std::path::PathBuf;

use purse_store::{
    AgendaChoice, AgendaKey, AgendaPrefs, Error, Hash32, OpenOptions, PiKey, Store, TSpendPolicies,
    TicketAgendaKey, TicketAgendaPrefs, TreasuryKeyPolicies, TreasuryVote, VspHost, VspHostId,
    VspHosts, VspPubKeys, VspTSpendKey, VspTSpendPolicies, VspTicketRow, VspTickets,
    VspTreasuryKey, VspTreasuryKeyPolicies,
};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, oracle_or_skip, unhex};

/// A hash in Go's display form (`chainhash.Hash.String()`, the internal
/// bytes reversed) and back.
fn display(h: &Hash32) -> String {
    let mut b = h.0;
    b.reverse();
    hex(&b)
}

fn internal(s: &str) -> Hash32 {
    let mut b: [u8; 32] = unhex(s).try_into().expect("32 bytes");
    b.reverse();
    Hash32(b)
}

/// Go's `%v` of a `*[]byte`: `&[104 116 ...]`.
fn go_bytes(b: &[u8]) -> String {
    let parts: Vec<String> = b.iter().map(u8::to_string).collect();
    format!("&[{}]", parts.join(" "))
}

fn err(kind: &str, text: String) -> Value {
    json!({"kind": kind, "text": text})
}

/// The error as `oprunner` renders it: dcrwallet's kind identifier and the
/// text.
fn store_err(e: &Error) -> Value {
    let kind = match format!("{:?}", e.kind()).as_str() {
        "Io" => "IO".to_owned(),
        k => k.to_owned(),
    };
    err(&kind, e.to_string())
}

/// One record operation's result: `{"ok": v}` or `{"err": {kind, text}}`.
type Outcome = Result<Value, Value>;

struct Interp {
    store: Store,
}

impl Interp {
    fn update<R>(
        &self,
        f: impl FnOnce(&mut purse_store::WriteTx<'_>) -> purse_store::Result<R>,
    ) -> Result<R, Value> {
        self.store.update(f).map_err(|e| store_err(&e))
    }

    fn vsp_ticket_value(&self, ticket: &Hash32, lookup: bool) -> Result<Option<Value>, Value> {
        let tx = self.store.read().map_err(|e| store_err(&e))?;
        let Some(row) = tx.get::<VspTickets>(ticket).map_err(|e| store_err(&e))? else {
            return Ok(None);
        };
        let (mut host, mut pubkey) = (String::new(), String::new());
        if lookup
            && let Some(h) = tx
                .get::<VspHosts>(&VspHostId(row.host_id))
                .map_err(|e| store_err(&e))?
            && !h.0.is_empty()
        {
            host = String::from_utf8(h.0.clone()).expect("ASCII hosts");
            if let Some(p) = tx.get::<VspPubKeys>(&h).map_err(|e| store_err(&e))? {
                pubkey = hex(&p.pubkey);
            }
        }
        Ok(Some(json!({
            "feehash": display(&row.fee_hash),
            "feetxstatus": row.fee_status,
            "vsphostid": row.host_id,
            "host": host,
            "pubkey": pubkey,
        })))
    }

    fn policy_of<K, T>(&self, key: &K) -> Outcome
    where
        T: purse_store::Table<Key = K, Value = TreasuryVote>,
    {
        let tx = self.store.read().map_err(|e| store_err(&e))?;
        Ok(json!(
            tx.get::<T>(key)
                .map_err(|e| store_err(&e))?
                .map_or(0, TreasuryVote::code)
        ))
    }

    fn run(&self, op: &Value) -> Outcome {
        let s = |k: &str| op[k].as_str().expect(k).to_owned();
        let h = |k: &str| internal(op[k].as_str().expect(k));
        let u = |k: &str| u32::try_from(op[k].as_u64().expect(k)).expect("u32");
        let policy = |k: &str| u8::try_from(op[k].as_u64().expect(k)).expect("u8");
        let name = op["op"].as_str().expect("op");
        match name {
            "udb.set_vsp_ticket" => {
                let (ticket, host, pubkey) = (h("ticket"), s("host"), unhex(&s("pubkey")));
                let row = VspTicketRow {
                    fee_hash: h("fee_hash"),
                    fee_status: u("fee_tx_status"),
                    host_id: 0,
                };
                let id = self.update(|tx| {
                    let host_id = tx.add_vsp_host(host.as_bytes(), &pubkey)?;
                    tx.put_vsp_ticket(&ticket, &VspTicketRow { host_id, ..row })?;
                    Ok(host_id)
                })?;
                Ok(json!({"vsphostid": id}))
            }
            "udb.delete_vsp_ticket" => {
                let t = h("ticket");
                self.update(|tx| tx.delete_vsp_ticket(&t))?;
                Ok(Value::Null)
            }
            "udb.get_vsp_ticket" => {
                let t = h("ticket");
                self.vsp_ticket_value(&t, true)?.ok_or_else(|| {
                    err(
                        "NotExist",
                        format!(
                            "item does not exist: no VSP info for ticket {}",
                            display(&t)
                        ),
                    )
                })
            }
            "udb.get_vsp_tickets_by_fee_status" => {
                let st = op["status"].as_i64().expect("status");
                let mut tickets = match u32::try_from(st) {
                    Ok(st) => self
                        .store
                        .read()
                        .and_then(|tx| tx.vsp_tickets_with_fee_status(st))
                        .map_err(|e| store_err(&e))?,
                    Err(_) => Vec::new(),
                };
                tickets.sort_by_key(display);
                let mut out = Vec::new();
                for t in tickets {
                    // Go's listing decodes the ticket rows alone: no host.
                    let mut v = self.vsp_ticket_value(&t, false)?.expect("indexed ticket");
                    v["ticket"] = json!(display(&t));
                    out.push(v);
                }
                Ok(Value::Array(out))
            }
            "udb.get_vsp_host" => {
                let id = u("id");
                if id == 0 {
                    // dcrwallet's upgrade 23 seeds id 0 with the empty host.
                    return Ok(json!(""));
                }
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                match tx
                    .get::<VspHosts>(&VspHostId(id))
                    .map_err(|e| store_err(&e))?
                {
                    Some(host) => Ok(json!(hex(&host.0))),
                    None => Err(err(
                        "NotExist",
                        "item does not exist: no VSP host info for id <pointer>".into(),
                    )),
                }
            }
            "udb.get_vsp_pub_key" => {
                let host = s("host");
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let row = if host.is_empty() {
                    None
                } else {
                    tx.get::<VspPubKeys>(&VspHost(host.clone().into_bytes()))
                        .map_err(|e| store_err(&e))?
                };
                match row {
                    Some(p) => Ok(json!({"id": p.host_id, "pubkey": hex(&p.pubkey)})),
                    None => Err(err(
                        "NotExist",
                        format!(
                            "item does not exist: no VSP pubkey info for host {}",
                            go_bytes(host.as_bytes())
                        ),
                    )),
                }
            }
            "udb.set_tspend_policy" => {
                let (k, p) = (h("tspend"), policy("policy"));
                self.set_policy::<TSpendPolicies>(&k, p)
            }
            "udb.tspend_policy" => self.policy_of::<_, TSpendPolicies>(&h("tspend")),
            "udb.tspend_policies" => {
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let mut rows: Vec<(Hash32, TreasuryVote)> = tx
                    .iter::<TSpendPolicies>()
                    .and_then(|i| i.collect())
                    .map_err(|e| store_err(&e))?;
                rows.sort_by_key(|(k, _)| display(k));
                Ok(Value::Array(
                    rows.iter()
                        .map(|(k, v)| json!({"tspend": display(k), "policy": v.code()}))
                        .collect(),
                ))
            }
            "udb.set_vsp_tspend_policy" => {
                let k = VspTSpendKey {
                    ticket: h("ticket"),
                    tspend: h("tspend"),
                };
                self.set_policy::<VspTSpendPolicies>(&k, policy("policy"))
            }
            "udb.vsp_tspend_policy" => self.policy_of::<_, VspTSpendPolicies>(&VspTSpendKey {
                ticket: h("ticket"),
                tspend: h("tspend"),
            }),
            "udb.vsp_tspend_policies" => {
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let mut rows: Vec<(VspTSpendKey, TreasuryVote)> = tx
                    .iter::<VspTSpendPolicies>()
                    .and_then(|i| i.collect())
                    .map_err(|e| store_err(&e))?;
                rows.sort_by_key(|(k, _)| (display(&k.ticket), display(&k.tspend)));
                Ok(Value::Array(
                    rows.iter()
                        .map(|(k, v)| {
                            json!({"ticket": display(&k.ticket), "tspend": display(&k.tspend),
                                   "policy": v.code()})
                        })
                        .collect(),
                ))
            }
            "udb.set_treasury_key_policy" => {
                let k = PiKey(unhex(&s("pikey")));
                self.set_policy::<TreasuryKeyPolicies>(&k, policy("policy"))
            }
            "udb.treasury_key_policy" => {
                self.policy_of::<_, TreasuryKeyPolicies>(&PiKey(unhex(&s("pikey"))))
            }
            "udb.treasury_key_policies" => {
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let rows: Vec<(PiKey, TreasuryVote)> = tx
                    .iter::<TreasuryKeyPolicies>()
                    .and_then(|i| i.collect())
                    .map_err(|e| store_err(&e))?;
                // Byte order of the keys, which is Go's sort of its
                // string keys.
                Ok(Value::Array(
                    rows.iter()
                        .map(|(k, v)| json!({"pikey": hex(&k.0), "policy": v.code()}))
                        .collect(),
                ))
            }
            "udb.set_vsp_treasury_key_policy" => {
                let k = VspTreasuryKey {
                    ticket: h("ticket"),
                    pikey: unhex(&s("pikey")),
                };
                self.set_policy::<VspTreasuryKeyPolicies>(&k, policy("policy"))
            }
            "udb.vsp_treasury_key_policy" => {
                self.policy_of::<_, VspTreasuryKeyPolicies>(&VspTreasuryKey {
                    ticket: h("ticket"),
                    pikey: unhex(&s("pikey")),
                })
            }
            "udb.vsp_treasury_key_policies" => {
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let mut rows: Vec<(VspTreasuryKey, TreasuryVote)> = tx
                    .iter::<VspTreasuryKeyPolicies>()
                    .and_then(|i| i.collect())
                    .map_err(|e| store_err(&e))?;
                rows.sort_by(|(a, _), (b, _)| {
                    (display(&a.ticket), &a.pikey).cmp(&(display(&b.ticket), &b.pikey))
                });
                Ok(Value::Array(
                    rows.iter()
                        .map(|(k, v)| {
                            json!({"ticket": display(&k.ticket), "pikey": hex(&k.pikey),
                                   "policy": v.code()})
                        })
                        .collect(),
                ))
            }
            "udb.set_default_agenda_preference" => {
                let k = AgendaKey {
                    version: u("version"),
                    agenda: s("agenda").into_bytes(),
                };
                let c = AgendaChoice(s("choice").into_bytes());
                self.update(|tx| tx.put::<AgendaPrefs>(&k, &c))?;
                Ok(Value::Null)
            }
            "udb.set_ticket_agenda_preference" => {
                let k = TicketAgendaKey {
                    ticket: h("ticket"),
                    version: u("version"),
                    agenda: s("agenda").into_bytes(),
                };
                let c = AgendaChoice(s("choice").into_bytes());
                self.update(|tx| tx.put::<TicketAgendaPrefs>(&k, &c))?;
                Ok(Value::Null)
            }
            "udb.default_agenda_preference" => {
                let k = AgendaKey {
                    version: u("version"),
                    agenda: s("agenda").into_bytes(),
                };
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let c = tx.get::<AgendaPrefs>(&k).map_err(|e| store_err(&e))?;
                Ok(json!(
                    String::from_utf8(c.map_or_else(Vec::new, |c| c.0)).expect("ASCII")
                ))
            }
            "udb.ticket_agenda_preference" => {
                let k = TicketAgendaKey {
                    ticket: h("ticket"),
                    version: u("version"),
                    agenda: s("agenda").into_bytes(),
                };
                let tx = self.store.read().map_err(|e| store_err(&e))?;
                let c = tx.get::<TicketAgendaPrefs>(&k).map_err(|e| store_err(&e))?;
                Ok(json!(
                    String::from_utf8(c.map_or_else(Vec::new, |c| c.0)).expect("ASCII")
                ))
            }
            other => panic!("operation {other} is not interpreted"),
        }
    }

    /// dcrwallet's setters: policy 0 deletes the key, any other byte is
    /// written (`wallet/udb/treasury.go`).
    fn set_policy<T>(&self, key: &T::Key, code: u8) -> Outcome
    where
        T: purse_store::Table<Value = TreasuryVote>,
    {
        self.update(|tx| match TreasuryVote::new(code) {
            None => tx.delete::<T>(key).map(drop),
            Some(v) => tx.put::<T>(key, &v),
        })?;
        Ok(Value::Null)
    }
}

/// A generated script over small sets of tickets, hosts, keys and agendas,
/// so that records are written again, read back, moved and deleted.
fn script(r: &mut SplitMix64, id: usize) -> Value {
    let hashes: Vec<String> = (0..5)
        .map(|_| {
            let mut b = [0u8; 32];
            r.fill(&mut b);
            display(&Hash32(b))
        })
        .collect();
    let zero = display(&Hash32::default());
    let hosts = [
        "https://a.example",
        "https://b.example",
        "https://c.example",
        "",
    ];
    let pikeys: Vec<String> = vec![
        hex(&[0x02; 33]),
        hex(&[0x03; 33]),
        hex(&[0x02, 0x01]),
        String::new(),
    ];
    let agendas = ["treasury", "maxblocksize", "", "autorevocations"];
    let choices = ["yes", "no", "abstain", ""];
    let pick = |r: &mut SplitMix64, n: usize| r.below(n as u64) as usize;
    let mut ops = Vec::new();
    for _ in 0..40 + r.below(40) {
        let hash = |r: &mut SplitMix64| hashes[pick(r, hashes.len())].clone();
        let policy = |r: &mut SplitMix64| match r.below(6) {
            0 => 0u64,
            1 => r.below(256),
            n => 1 + (n % 2),
        };
        let op = match r.below(22) {
            0..=3 => json!({
                "op": "udb.set_vsp_ticket", "ticket": hash(r),
                "fee_hash": if r.below(3) == 0 { zero.clone() } else { hash(r) },
                "fee_tx_status": r.below(4), "vsp_host_id": r.below(9),
                "host": hosts[if r.below(12) == 0 { 3 } else { pick(r, 3) }],
                "pubkey": hex(&[r.below(3) as u8 + 1; 33]),
            }),
            4 => json!({"op": "udb.delete_vsp_ticket", "ticket": hash(r)}),
            5 | 6 => json!({"op": "udb.get_vsp_ticket", "ticket": hash(r)}),
            7 => json!({"op": "udb.get_vsp_tickets_by_fee_status",
                        "status": r.below(5) as i64 - 1}),
            8 => json!({"op": "udb.get_vsp_host", "id": r.below(5)}),
            9 => json!({"op": "udb.get_vsp_pub_key", "host": hosts[pick(r, 4)]}),
            10 => json!({"op": "udb.set_tspend_policy", "tspend": hash(r), "policy": policy(r)}),
            11 => json!({"op": "udb.tspend_policy", "tspend": hash(r)}),
            12 => json!({"op": "udb.set_vsp_tspend_policy", "ticket": hash(r),
                         "tspend": hash(r), "policy": policy(r)}),
            13 => json!({"op": "udb.vsp_tspend_policy", "ticket": hash(r), "tspend": hash(r)}),
            14 => json!({"op": "udb.set_treasury_key_policy",
                         "pikey": pikeys[pick(r, 4)], "policy": policy(r)}),
            15 => json!({"op": "udb.treasury_key_policy", "pikey": pikeys[pick(r, 4)]}),
            16 => json!({"op": "udb.set_vsp_treasury_key_policy", "ticket": hash(r),
                         "pikey": pikeys[pick(r, 4)], "policy": policy(r)}),
            17 => json!({"op": "udb.vsp_treasury_key_policy", "ticket": hash(r),
                         "pikey": pikeys[pick(r, 4)]}),
            18 => {
                let l = [
                    "udb.tspend_policies",
                    "udb.vsp_tspend_policies",
                    "udb.treasury_key_policies",
                    "udb.vsp_treasury_key_policies",
                ][pick(r, 4)];
                json!({ "op": l })
            }
            19 => json!({"op": "udb.set_default_agenda_preference",
                         "version": 10 + r.below(2), "agenda": agendas[pick(r, 4)],
                         "choice": choices[pick(r, 4)]}),
            20 => json!({"op": "udb.set_ticket_agenda_preference", "ticket": hash(r),
                         "version": 10 + r.below(2), "agenda": agendas[pick(r, 4)],
                         "choice": choices[pick(r, 4)]}),
            _ => {
                if r.below(2) == 0 {
                    json!({"op": "udb.default_agenda_preference",
                           "version": 10 + r.below(2), "agenda": agendas[pick(r, 4)]})
                } else {
                    json!({"op": "udb.ticket_agenda_preference", "ticket": hash(r),
                           "version": 10 + r.below(2), "agenda": agendas[pick(r, 4)]})
                }
            }
        };
        ops.push(op);
    }
    // Every listing at the end.
    for l in [
        "udb.tspend_policies",
        "udb.vsp_tspend_policies",
        "udb.treasury_key_policies",
        "udb.vsp_treasury_key_policies",
    ] {
        ops.push(json!({"op": l}));
    }
    for st in 0..4 {
        ops.push(json!({"op": "udb.get_vsp_tickets_by_fee_status", "status": st}));
    }
    for t in &hashes {
        ops.push(json!({"op": "udb.get_vsp_ticket", "ticket": t}));
    }
    json!({
        "format": "purse-opscript 1",
        "name": format!("records-{id}"),
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "0505050505050505050505050505050505050505050505050505050505050505",
                  "clock": 1_794_990_000},
        "ops": ops,
    })
}

fn scratch(label: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-store-records-oracle")
        .join(format!("{label}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&p).expect("scratch dir");
    p
}

#[test]
fn record_operations_match_dcrwallets_udb() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_STORE_RECORD_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let mut r = SplitMix64::from_env("purse-store records oracle");
    let mut ops = 0;
    let mut errors = 0;
    for id in 0..count {
        let s = script(&mut r, id);
        let dir = scratch(&format!("s{id}"));
        let go = o.call(
            "run",
            json!({"script": &s, "dir": dir.join("go").to_str().expect("utf-8 path")}),
        );
        let results = go["results"].as_array().expect("results");
        let interp = Interp {
            store: OpenOptions::new()
                .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
                .expect("store"),
        };
        let script_ops = s["ops"].as_array().expect("ops");
        assert_eq!(results.len(), script_ops.len(), "{}", s["name"]);
        for (i, (op, want)) in script_ops.iter().zip(results).enumerate() {
            let got = match interp.run(op) {
                Ok(v) => json!({"ok": v}),
                Err(e) => {
                    errors += 1;
                    json!({"err": e})
                }
            };
            let want_cmp = match want.get("err") {
                Some(e) => json!({"err": e}),
                None => json!({"ok": want["ok"]}),
            };
            assert_eq!(
                got, want_cmp,
                "{} op {i} {op}: Rust (left) and Go (right) differ",
                s["name"]
            );
            ops += 1;
        }
        let tx = interp.store.read().expect("read");
        tx.check_records().expect("record tables consistent");
        drop(tx);
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!("{count} scripts, {ops} operations equal ({errors} of them errors)");
    assert!(errors > 0, "no refusal or miss was compared");
}
