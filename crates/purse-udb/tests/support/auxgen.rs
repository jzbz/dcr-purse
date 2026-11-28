// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Generated operation scripts for the auxiliary records (`generate_aux`):
//! VSP tickets, the four treasury policy kinds and agenda preferences,
//! every operation of [`AUX_OPS`](super::runner::auxops::AUX_OPS) drawn
//! over small per-script pools, so that writes land on the same keys again
//! and lookups hit and miss, with reopens, on all four networks.
//!
//! Each script draws a pool of three to seven ticket hashes (shared by the
//! VSP tickets, the per-ticket treasury policies and the per-ticket agenda
//! preferences), two to five treasury spend hashes and two to four pi keys
//! (33 bytes), and takes two to five hosts from a fixed list in which one
//! host is a prefix of another and one is not ASCII. Now and then an
//! argument is one Go refuses or answers specially: an empty host (bbolt's
//! `key required` from Go's public-key put, which dcr-purse returns before
//! writing, QK-0122), an empty or one-byte pi key, an empty agenda id or
//! choice, policy 0 (a delete), 3 or 255 (QK-0127), a fee status outside
//! dcrwallet's four (7, 2^31, 2^32 − 1), a host id passed in the record
//! (ignored by `SetVSPTicket`), `GetVSPHost` of id 0, of ids past the last
//! and of 2^32 − 1, and `GetVSPTicketsByFeeStatus` of -1, `i32::MIN`,
//! `i32::MAX` and 7. Keys over bbolt's size limit are left to
//! `auxiliary_records_at_the_key_limit_match_go`, for the size of their
//! strings. Every script ends with every listing, every fee status of
//! dcrwallet's four, every pool ticket's record and every pool host's
//! public key.
//!
//! `group_aux` puts runs of the operations into `update` and `view` groups
//! (a `view` group holds only readers), some members tolerating `NotExist`,
//! `Invalid` or `IO`, except `Invalid` on `udb.set_vsp_ticket`: Go has
//! written the host row and the counter when it refuses an empty host,
//! which a tolerating group would keep and the store cannot hold, and the
//! Rust runner refuses such a script (QK-0122;
//! `a_tolerated_vsp_ticket_refusal_is_refused_by_the_rust_runner`). A
//! tolerated `IO` refusal of `SetTicketAgendaPreference` leaves Go an
//! empty bucket for the ticket, which no reader and no export line shows.
//!
//! A script never faults: every argument is one both runners accept.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex};

use super::runner::auxops::AUX_CHANGING;

/// The operations a generated auxiliary script draws from (a name listed
/// twice is drawn twice as often).
pub const AUX_GEN_OPS: &[&str] = &[
    "udb.set_vsp_ticket",
    "udb.set_vsp_ticket",
    "udb.set_vsp_ticket",
    "udb.set_vsp_ticket",
    "udb.delete_vsp_ticket",
    "udb.get_vsp_ticket",
    "udb.get_vsp_ticket",
    "udb.get_vsp_tickets_by_fee_status",
    "udb.get_vsp_tickets_by_fee_status",
    "udb.get_vsp_host",
    "udb.get_vsp_pub_key",
    "udb.set_tspend_policy",
    "udb.set_tspend_policy",
    "udb.set_vsp_tspend_policy",
    "udb.set_vsp_tspend_policy",
    "udb.tspend_policy",
    "udb.vsp_tspend_policy",
    "udb.tspend_policies",
    "udb.vsp_tspend_policies",
    "udb.set_treasury_key_policy",
    "udb.set_treasury_key_policy",
    "udb.set_vsp_treasury_key_policy",
    "udb.set_vsp_treasury_key_policy",
    "udb.treasury_key_policy",
    "udb.vsp_treasury_key_policy",
    "udb.treasury_key_policies",
    "udb.vsp_treasury_key_policies",
    "udb.set_default_agenda_preference",
    "udb.set_default_agenda_preference",
    "udb.set_ticket_agenda_preference",
    "udb.set_ticket_agenda_preference",
    "udb.default_agenda_preference",
    "udb.ticket_agenda_preference",
    "rt.reopen",
];

/// The networks, by how often a script draws them.
const NETWORKS: [(&str, u64); 4] = [
    ("simnet", 70),
    ("regnet", 10),
    ("testnet3", 10),
    ("mainnet", 10),
];

/// The hosts a script takes its pool from: one is a prefix of another,
/// one differs only by a trailing slash, one is not ASCII.
const HOSTS: [&str; 7] = [
    "https://vsp-a.example",
    "https://vsp-a.example/",
    "https://vsp-b.example",
    "https://vsp-b.example.org",
    "https://vsp-\u{fc}.example",
    "vsp.c",
    "x",
];

/// Agenda ids: dcrd's, an empty one, a non-ASCII one.
const AGENDAS: [&str; 6] = [
    "treasury",
    "maxblocksize",
    "changesubsidysplit",
    "blake3pow",
    "",
    "agenda-\u{e9}",
];

/// Agenda choices: dcrd's, and an empty one.
const CHOICES: [&str; 4] = ["yes", "no", "abstain", ""];

/// Deployment versions.
const VERSIONS: [u32; 6] = [0, 1, 7, 10, 11, u32::MAX];

fn network(rng: &mut SplitMix64) -> &'static str {
    let total: u64 = NETWORKS.iter().map(|n| n.1).sum();
    let mut x = rng.below(total);
    for (name, weight) in NETWORKS {
        if x < weight {
            return name;
        }
        x -= weight;
    }
    "simnet"
}

struct Gen<'r> {
    rng: &'r mut SplitMix64,
    tickets: Vec<String>,
    tspends: Vec<String>,
    pikeys: Vec<String>,
    hosts: Vec<&'static str>,
    /// The distinct non-empty hosts a `SetVSPTicket` was drawn for: a
    /// bound on the ids given out, so that `GetVSPHost` hits and misses.
    hosts_used: Vec<&'static str>,
    costly: bool,
    reopens: usize,
}

impl Gen<'_> {
    fn below(&mut self, n: u64) -> u64 {
        self.rng.below(n)
    }

    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[usize::try_from(self.below(v.len() as u64)).expect("small")]
    }

    fn ticket(&mut self) -> String {
        let i = usize::try_from(self.below(self.tickets.len() as u64)).expect("small");
        self.tickets[i].clone()
    }

    fn tspend(&mut self) -> String {
        let i = usize::try_from(self.below(self.tspends.len() as u64)).expect("small");
        self.tspends[i].clone()
    }

    /// A pi key: one of the pool's, or rarely an empty or one-byte one.
    fn pikey(&mut self) -> String {
        match self.below(12) {
            0 => String::new(),
            1 => "aa".to_owned(),
            _ => {
                let i = usize::try_from(self.below(self.pikeys.len() as u64)).expect("small");
                self.pikeys[i].clone()
            }
        }
    }

    /// A host: one of the pool's, or rarely the empty host.
    fn host(&mut self) -> &'static str {
        if self.below(14) == 0 {
            return "";
        }
        let hosts = self.hosts.clone();
        self.pick(&hosts)
    }

    /// A public key: 33 bytes mostly, now and then empty or one byte.
    fn pubkey(&mut self) -> String {
        match self.below(10) {
            0 => String::new(),
            1 => "01".to_owned(),
            _ => {
                let mut k = self.rng.bytes(33);
                k[0] = 2 | (k[0] & 1);
                hex(&k)
            }
        }
    }

    fn fee_hash(&mut self) -> String {
        if self.below(6) == 0 {
            "00".repeat(32)
        } else {
            hex(&self.rng.bytes(32))
        }
    }

    fn fee_status(&mut self) -> u32 {
        if self.below(8) < 6 {
            u32::try_from(self.below(4)).expect("small")
        } else {
            self.pick(&[7, 0x8000_0000, u32::MAX])
        }
    }

    fn query_status(&mut self) -> i64 {
        if self.below(6) != 0 {
            i64::try_from(self.below(4)).expect("small")
        } else {
            self.pick(&[-1, 7, i64::from(i32::MIN), i64::from(i32::MAX)])
        }
    }

    fn policy(&mut self) -> u64 {
        self.pick(&[0, 0, 1, 1, 1, 2, 2, 2, 3, 255])
    }

    fn agenda(&mut self) -> (u32, &'static str) {
        (self.pick(&VERSIONS[..]), self.pick(&AGENDAS[..]))
    }

    fn op(&mut self, name: &str) -> Value {
        match name {
            "udb.set_vsp_ticket" => {
                let host = self.host();
                if !host.is_empty() && !self.hosts_used.contains(&host) {
                    self.hosts_used.push(host);
                }
                let mut v = json!({
                    "op": name, "ticket": self.ticket(), "fee_hash": self.fee_hash(),
                    "fee_tx_status": self.fee_status(), "host": host,
                    "pubkey": self.pubkey(),
                });
                if self.below(4) == 0 {
                    let n = self.hosts_used.len() as u64;
                    v["vsp_host_id"] = json!(match self.below(3) {
                        0 => u64::from(u32::MAX),
                        _ => self.below(n + 2),
                    });
                }
                v
            }
            "udb.delete_vsp_ticket" | "udb.get_vsp_ticket" => {
                json!({"op": name, "ticket": self.ticket()})
            }
            "udb.get_vsp_tickets_by_fee_status" => {
                json!({"op": name, "status": self.query_status()})
            }
            "udb.get_vsp_host" => {
                let n = self.hosts_used.len() as u64;
                let id = if self.below(8) == 0 {
                    u64::from(u32::MAX)
                } else {
                    self.below(n + 3)
                };
                json!({"op": name, "id": id})
            }
            "udb.get_vsp_pub_key" => json!({"op": name, "host": self.host()}),
            "udb.set_tspend_policy" => {
                json!({"op": name, "tspend": self.tspend(), "policy": self.policy()})
            }
            "udb.set_vsp_tspend_policy" => json!({
                "op": name, "ticket": self.ticket(), "tspend": self.tspend(),
                "policy": self.policy(),
            }),
            "udb.tspend_policy" => json!({"op": name, "tspend": self.tspend()}),
            "udb.vsp_tspend_policy" => {
                json!({"op": name, "ticket": self.ticket(), "tspend": self.tspend()})
            }
            "udb.set_treasury_key_policy" => {
                json!({"op": name, "pikey": self.pikey(), "policy": self.policy()})
            }
            "udb.set_vsp_treasury_key_policy" => json!({
                "op": name, "ticket": self.ticket(), "pikey": self.pikey(),
                "policy": self.policy(),
            }),
            "udb.treasury_key_policy" => json!({"op": name, "pikey": self.pikey()}),
            "udb.vsp_treasury_key_policy" => {
                json!({"op": name, "ticket": self.ticket(), "pikey": self.pikey()})
            }
            "udb.set_default_agenda_preference" => {
                let (version, agenda) = self.agenda();
                let choice = self.pick(&CHOICES[..]);
                json!({"op": name, "version": version, "agenda": agenda, "choice": choice})
            }
            "udb.set_ticket_agenda_preference" => {
                let (version, agenda) = self.agenda();
                let choice = self.pick(&CHOICES[..]);
                json!({"op": name, "ticket": self.ticket(), "version": version,
                       "agenda": agenda, "choice": choice})
            }
            "udb.default_agenda_preference" => {
                let (version, agenda) = self.agenda();
                json!({"op": name, "version": version, "agenda": agenda})
            }
            "udb.ticket_agenda_preference" => {
                let (version, agenda) = self.agenda();
                json!({"op": name, "ticket": self.ticket(), "version": version,
                       "agenda": agenda})
            }
            "rt.reopen" => {
                // A reopen on the other networks costs a key derivation of
                // their full Argon2 parameters: one per script.
                if self.costly && self.reopens >= 1 {
                    return json!({"op": "udb.tspend_policies"});
                }
                self.reopens += 1;
                json!({"op": name})
            }
            other => json!({"op": other}),
        }
    }
}

/// A generated auxiliary record script (module documentation), on
/// `network_override` or a drawn network.
pub fn generate_aux(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    let network = network_override.unwrap_or_else(|| network(rng));
    let seed = rng.bytes(32);
    let n = 30 + rng.below(60);
    let n_tickets = 3 + rng.below(5);
    let tickets = (0..n_tickets).map(|_| hex(&rng.bytes(32))).collect();
    let n_tspends = 2 + rng.below(4);
    let tspends = (0..n_tspends).map(|_| hex(&rng.bytes(32))).collect();
    let n_pikeys = 2 + rng.below(3);
    let pikeys = (0..n_pikeys)
        .map(|_| {
            let mut k = rng.bytes(33);
            k[0] = 2 | (k[0] & 1);
            hex(&k)
        })
        .collect();
    // Two to five hosts, without repeats, in drawn order.
    let mut all: Vec<&'static str> = HOSTS.to_vec();
    let n_hosts = 2 + usize::try_from(rng.below(4)).expect("small");
    let mut hosts = Vec::with_capacity(n_hosts);
    for _ in 0..n_hosts {
        let i = usize::try_from(rng.below(all.len() as u64)).expect("small");
        hosts.push(all.remove(i));
    }
    let mut g = Gen {
        rng,
        tickets,
        tspends,
        pikeys,
        hosts,
        hosts_used: Vec::new(),
        costly: network != "simnet",
        reopens: 0,
    };
    let mut ops = Vec::new();
    for _ in 0..n {
        let name = g.pick(AUX_GEN_OPS);
        ops.push(g.op(name));
    }
    // Every script ends with the queries over everything it wrote.
    for q in [
        "udb.tspend_policies",
        "udb.vsp_tspend_policies",
        "udb.treasury_key_policies",
        "udb.vsp_treasury_key_policies",
    ] {
        ops.push(json!({"op": q}));
    }
    for status in 0..4 {
        ops.push(json!({"op": "udb.get_vsp_tickets_by_fee_status", "status": status}));
    }
    for t in g.tickets.clone() {
        ops.push(json!({"op": "udb.get_vsp_ticket", "ticket": t}));
    }
    for h in g.hosts.clone() {
        ops.push(json!({"op": "udb.get_vsp_pub_key", "host": h}));
    }
    for id in 0..=g.hosts_used.len() + 1 {
        ops.push(json!({"op": "udb.get_vsp_host", "id": id}));
    }
    let rand = hex(&g.rng.bytes(32));
    json!({
        "format": "purse-opscript 1",
        "name": format!("aux generated {id}"),
        "setup": {
            "network": network,
            "seed": hex(&seed),
            "pubpass": "public",
            "privpass": "private",
            "rand": rand,
            "clock": 1_795_600_000,
        },
        "ops": ops,
    })
}

/// The kinds a generated group member may tolerate.
const TOLERABLE: [&str; 3] = ["NotExist", "Invalid", "IO"];

/// `generate_aux`'s script with runs of its operations put in `update` and
/// `view` groups (ADR-0002 section 2), some members tolerating kinds, never
/// `Invalid` on `udb.set_vsp_ticket` (module documentation). A reopen is
/// never a member.
pub fn group_aux(rng: &mut SplitMix64, mut script: Value) -> Value {
    let ops = script["ops"].as_array().cloned().unwrap_or_default();
    let groupable = |v: &Value| v["op"].as_str().unwrap_or("").starts_with("udb.");
    let mut out = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < ops.len() {
        if !groupable(&ops[i]) || rng.below(3) != 0 {
            out.push(ops[i].clone());
            i += 1;
            continue;
        }
        let want = 2 + usize::try_from(rng.below(5)).expect("small");
        let mut members = Vec::new();
        while i < ops.len() && members.len() < want && groupable(&ops[i]) {
            let mut m = ops[i].clone();
            if rng.below(2) == 0 {
                let partial = m["op"] == "udb.set_vsp_ticket";
                let kinds: Vec<&str> = TOLERABLE
                    .iter()
                    .copied()
                    .filter(|k| rng.below(2) == 0 && !(partial && *k == "Invalid"))
                    .collect();
                if !kinds.is_empty() {
                    m["tolerate"] = json!(kinds);
                }
            }
            members.push(m);
            i += 1;
        }
        let queries = members
            .iter()
            .all(|m| !AUX_CHANGING.contains(&m["op"].as_str().unwrap_or("")));
        let kind = if queries && rng.below(2) == 0 {
            "view"
        } else {
            "update"
        };
        out.push(json!({"op": kind, "ops": members}));
    }
    script["ops"] = Value::Array(out);
    script
}
