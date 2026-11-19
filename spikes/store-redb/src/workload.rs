//! The workload: a deterministic wallet history generated from a seed, and
//! its file format, which the Go baseline reads too.
//!
//! # File format (`PURSEWL1`, little-endian)
//!
//! ```text
//! magic    "PURSEWL1"
//! u32      accounts
//! u32      address ranges; each: u32 account, u32 branch, u32 count,
//!          count x 20-byte HASH160 (children 0..count-1 of that branch)
//! u32      operations; each starts with a u8 kind:
//!   1, 3   attach a block (1: history sync, 3: at the tip): u8 commit,
//!          180-byte header, u32 transactions, each u32 length + bytes
//!   2      an unmined transaction: u8 unpublished, u32 length + bytes
//!   4      a reorganisation: u32 height; `Rollback` from that height to the
//!          tip, in the same write transaction as the blocks that follow
//! ```
//!
//! An operation with `commit` set ends a write transaction; an unmined
//! insert is always a write transaction of its own. Blocks during history
//! sync are batched (dcrwallet's `ChainSwitch` attaches a whole batch in one
//! `walletdb.Update`); blocks at the tip are one transaction each.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use dcroxide_chainhash::Hash;
use dcroxide_wire::{BlockHeader, MsgTx, OutPoint, TxIn, TxOut, TxSerializeType};
use purse_hdkeychain::{ExtendedKey, HdVersions};
use purse_testutil::SplitMix64;

use crate::chain::{
    self, COINBASE_MATURITY, LEGACY_COIN_TYPE, OP_SSGEN, OP_SSTX, OP_SSTXCHANGE, SIMNET_GENESIS,
    TICKET_MATURITY,
};

const HARDENED: u32 = 0x8000_0000;

/// What to generate.
#[derive(Clone, Debug)]
pub struct Params {
    /// PRNG seed; the wallet seed is derived from it.
    pub seed: u64,
    /// Target number of wallet transactions (votes included).
    pub txs: u64,
    /// Chain length in blocks after genesis.
    pub blocks: u32,
    /// Number of BIP0044 accounts.
    pub accounts: u32,
    /// The last `tail_blocks` blocks are attached one per transaction, and
    /// most of their transactions are first inserted unmined.
    pub tail_blocks: u32,
    /// Blocks per write transaction before the tail.
    pub batch_blocks: u32,
    /// Transactions inserted unmined after the last block and never mined
    /// (about a third of the sends among them unpublished).
    pub final_unmined: u32,
}

impl Params {
    /// The named presets: `tiny` (tests), `small`, `10k`, `100k`, `1m`.
    pub fn preset(name: &str) -> Option<Params> {
        let (txs, blocks, accounts, tail, batch, unmined) = match name {
            "tiny" => (600, 300, 3, 100, 50, 12),
            "small" => (2_000, 5_000, 4, 300, 500, 20),
            "10k" => (10_000, 50_000, 5, 2_000, 2_000, 30),
            "100k" => (100_000, 100_000, 8, 2_000, 2_000, 60),
            "1m" => (1_000_000, 200_000, 10, 2_000, 2_000, 200),
            _ => return None,
        };
        Some(Params {
            seed: 0x5eed_0003,
            txs,
            blocks,
            accounts,
            tail_blocks: tail,
            batch_blocks: batch,
            final_unmined: unmined,
        })
    }
}

/// Addresses to record before ingest: children `0..hashes.len()` of one
/// account branch.
#[derive(Clone, Debug)]
pub struct AddrRange {
    /// Account number.
    pub account: u32,
    /// 0 external, 1 internal.
    pub branch: u32,
    /// HASH160 of each child's public key.
    pub hashes: Vec<[u8; 20]>,
}

/// Kinds of operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    /// A block attached during history sync.
    SyncBlock = 1,
    /// An unmined transaction.
    Unmined = 2,
    /// A block attached at the tip.
    TipBlock = 3,
    /// `Rollback` to a height, followed in the same write transaction by
    /// the blocks of the new chain (dcrwallet's `ChainSwitch`,
    /// wallet/chainntfns.go:91).
    Reorg = 4,
}

/// One operation.
#[derive(Clone, Debug)]
pub struct Op {
    /// The kind.
    pub kind: OpKind,
    /// Whether the write transaction ends with this operation.
    pub commit: bool,
    /// For an unmined insert: the unpublished flag.
    pub unpublished: bool,
    /// For a block: the serialized header.
    pub header: Option<[u8; 180]>,
    /// For a reorganisation: the first height rolled back.
    pub rollback_to: u32,
    /// The serialized transactions.
    pub txs: Vec<Vec<u8>>,
}

/// A generated history.
#[derive(Clone, Debug)]
pub struct Workload {
    /// The wallet seed the addresses derive from.
    pub wallet_seed: [u8; 32],
    /// Number of accounts.
    pub accounts: u32,
    /// Addresses to record before ingest.
    pub ranges: Vec<AddrRange>,
    /// The operations, in order.
    pub ops: Vec<Op>,
}

/// Counts describing a generated workload.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Transactions by kind.
    pub receives: u64,
    /// Sends with change.
    pub sends: u64,
    /// Consolidations.
    pub consolidations: u64,
    /// Ticket purchases.
    pub tickets: u64,
    /// Votes.
    pub votes: u64,
    /// Coinbases paying the wallet.
    pub coinbases: u64,
    /// Transactions first inserted unmined.
    pub unmined_first: u64,
    /// Transactions still unmined at the end.
    pub unmined_at_end: u64,
    /// Addresses recorded.
    pub addresses: u64,
    /// Sends whose change goes to another account.
    pub cross_account: u64,
    /// Sends without change (they only debit their account).
    pub no_change: u64,
    /// Blocks that disapprove their parent.
    pub disapprovals: u64,
    /// Regular transactions of disapproved blocks (coinbases aside).
    pub invalidated: u64,
    /// Of those, mined again in a later block.
    pub remined: u64,
    /// Of those, never mined again.
    pub dropped: u64,
    /// Reorganisations.
    pub reorgs: u64,
    /// Transactions a reorganisation left unmined for a block.
    pub delayed: u64,
}

/// The wallet seed a workload seed gives.
pub fn wallet_seed(seed: u64) -> [u8; 32] {
    let mut rng = SplitMix64(seed ^ 0x77a1_1e75_eed0_0000);
    let mut s = [0; 32];
    rng.fill(&mut s);
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UtxoKind {
    Regular,
    Coinbase,
    StakeGen,
    TicketChange,
}

#[derive(Clone, Debug)]
struct Utxo {
    hash: [u8; 32],
    index: u32,
    tree: i8,
    value: i64,
    height: u32,
    kind: UtxoKind,
}

#[derive(Clone, Debug)]
struct Ticket {
    hash: [u8; 32],
    height: u32,
    price: i64,
    commit_hash: [u8; 20],
    commit_amount: i64,
    account: u32,
}

struct Account {
    branches: [ExtendedKey; 2],
    derived: [Vec<[u8; 20]>; 2],
    next: [u32; 2],
    used_ext: Vec<u32>,
}

/// A transaction whose outputs become wallet UTXOs once it is mined.
struct Built {
    tx: MsgTx,
    outputs: Vec<(u32, Utxo)>,
    /// The wallet outputs it spends, as (account, output).
    inputs: Vec<(u32, Utxo)>,
    ticket: Option<Ticket>,
}

struct Pending {
    bytes: Vec<u8>,
    mine_at: u32,
    outputs: Vec<(u32, Utxo)>,
    inputs: Vec<(u32, Utxo)>,
}

/// A transaction as it went into a block, kept for the last blocks so a
/// later disapproval or reorganisation can act on it.
#[derive(Clone)]
struct Mined {
    bytes: Vec<u8>,
    hash: [u8; 32],
    outputs: Vec<(u32, Utxo)>,
    inputs: Vec<(u32, Utxo)>,
    /// In the regular tree (stake invalidation acts on it).
    regular: bool,
    coinbase: bool,
}

impl Mined {
    fn of(bytes: Vec<u8>, outputs: Vec<(u32, Utxo)>, inputs: Vec<(u32, Utxo)>) -> Mined {
        let (tx, _) = MsgTx::from_bytes(&bytes).expect("own transaction");
        Mined {
            hash: tx.tx_hash().0,
            regular: chain::tx_type(&tx) == chain::TxType::Regular,
            coinbase: chain::is_coinbase(&tx),
            bytes,
            outputs,
            inputs,
        }
    }
}

/// The chain events the tail exercises, one every [`SCENARIO_SPACING`]
/// blocks, in this rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    /// The block disapproves its parent; each regular transaction of the
    /// parent is mined again in this block, mined again one or two blocks
    /// later, or never (its inputs then become spendable again).
    Disapprove,
    /// The block disapproves its parent and mines all of the parent's
    /// regular transactions again; the next block replaces it with a block
    /// that approves the parent (stake validation after a rollback).
    Revalidate,
    /// A reorganisation of the given depth: the last blocks are rolled back
    /// and replaced by blocks carrying the same transactions (coinbases
    /// dropped), sometimes with the last block's regular transactions left
    /// unmined until the next block.
    Reorg(u32),
}

/// Blocks between the starts of two tail scenarios.
const SCENARIO_SPACING: u32 = 8;
const ROTATION: [Scenario; 7] = [
    Scenario::Disapprove,
    Scenario::Revalidate,
    Scenario::Reorg(1),
    Scenario::Disapprove,
    Scenario::Reorg(2),
    Scenario::Revalidate,
    Scenario::Reorg(3),
];

struct Gen {
    rng: SplitMix64,
    p: Params,
    accts: Vec<Account>,
    utxos: Vec<Vec<Utxo>>,
    tickets: Vec<Ticket>,
    pending: Vec<Pending>,
    stats: Stats,
}

impl Gen {
    fn unit(&mut self) -> f64 {
        (self.rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.rng.below(n as u64)).expect("fits")
    }

    fn hash32(&mut self) -> [u8; 32] {
        let mut h = [0; 32];
        self.rng.fill(&mut h);
        h
    }

    fn hash20(&mut self) -> [u8; 20] {
        let mut h = [0; 20];
        self.rng.fill(&mut h);
        h
    }

    /// A Zipf-like account choice: account `i` with weight `1/(i+1)`.
    fn pick_account(&mut self) -> u32 {
        let n = self.p.accounts;
        let total: f64 = (1..=n).map(|i| 1.0 / f64::from(i)).sum();
        let mut x = self.unit() * total;
        for i in 0..n {
            x -= 1.0 / f64::from(i + 1);
            if x <= 0.0 {
                return i;
            }
        }
        n - 1
    }

    fn address(&mut self, account: u32, branch: u32, index: u32) -> [u8; 20] {
        let a = &mut self.accts[account as usize];
        let b = branch as usize;
        while a.derived[b].len() <= index as usize {
            let i = u32::try_from(a.derived[b].len()).expect("index");
            let child = a.branches[b].child(i).expect("child derivation");
            let h = chain::hash160(child.serialized_pub_key());
            a.derived[b].push(h);
        }
        a.derived[b][index as usize]
    }

    fn fresh(&mut self, account: u32, branch: u32) -> [u8; 20] {
        let idx = self.accts[account as usize].next[branch as usize];
        self.accts[account as usize].next[branch as usize] = idx + 1;
        if branch == 0 {
            self.accts[account as usize].used_ext.push(idx);
        }
        self.address(account, branch, idx)
    }

    fn receive_address(&mut self, account: u32) -> [u8; 20] {
        let used = self.accts[account as usize].used_ext.len();
        if used > 0 && self.unit() < 0.4 {
            let j = self.below(used);
            let idx = self.accts[account as usize].used_ext[j];
            self.address(account, 0, idx)
        } else {
            self.fresh(account, 0)
        }
    }

    fn spendable(u: &Utxo, height: u32) -> bool {
        let confs = i64::from(height) - i64::from(u.height);
        match u.kind {
            UtxoKind::Regular => confs >= 1,
            UtxoKind::Coinbase | UtxoKind::StakeGen => confs + 1 > i64::from(COINBASE_MATURITY),
            UtxoKind::TicketChange => confs >= 1,
        }
    }

    /// Removes and returns up to `n` spendable UTXOs of `account`.
    fn take_utxos(&mut self, account: u32, n: usize, height: u32, min_value: i64) -> Vec<Utxo> {
        let mut out = Vec::new();
        let mut tries = 0;
        while out.len() < n && tries < n * 8 + 8 {
            tries += 1;
            let list_len = self.utxos[account as usize].len();
            if list_len == 0 {
                break;
            }
            let i = self.below(list_len);
            let u = &self.utxos[account as usize][i];
            if Self::spendable(u, height) && u.value >= min_value {
                out.push(self.utxos[account as usize].swap_remove(i));
            }
        }
        out
    }

    /// Removes every pooled output of transaction `hash`.
    fn drop_outputs(&mut self, hash: &[u8; 32]) {
        for list in &mut self.utxos {
            list.retain(|u| u.hash != *hash);
        }
    }

    /// Moves every pooled output of transaction `hash` to `height`.
    fn move_outputs(&mut self, hash: &[u8; 32], height: u32) {
        for list in &mut self.utxos {
            for u in list.iter_mut().filter(|u| u.hash == *hash) {
                u.height = height;
            }
        }
    }

    fn sig_script(&mut self) -> Vec<u8> {
        // The shape of a P2PKH signature script: a 72-byte push of a DER
        // signature with its hash type, then a 33-byte compressed key.
        let mut s = Vec::with_capacity(107);
        s.push(0x48);
        s.extend_from_slice(&self.rng.bytes(72));
        s.push(0x21);
        s.extend_from_slice(&self.rng.bytes(33));
        s
    }

    fn input(&mut self, hash: [u8; 32], index: u32, tree: i8, value: i64, height: u32) -> TxIn {
        TxIn {
            previous_out_point: OutPoint {
                hash: Hash(hash),
                index,
                tree,
            },
            sequence: u32::MAX,
            value_in: value,
            block_height: height,
            block_index: self.rng.below(20) as u32,
            signature_script: self.sig_script(),
        }
    }

    fn new_tx() -> MsgTx {
        MsgTx {
            ser_type: TxSerializeType::Full,
            version: 1,
            tx_in: Vec::new(),
            tx_out: Vec::new(),
            lock_time: 0,
            expiry: 0,
        }
    }

    fn amount(&mut self) -> i64 {
        // Log-uniform between 0.001 and 1000 coins.
        let e = 5.0 + self.unit() * 6.0;
        10f64.powf(e) as i64
    }

    fn receive(&mut self, height: u32) -> Built {
        let account = self.pick_account();
        let mut tx = Self::new_tx();
        let value = self.amount();
        let h = self.hash32();
        let idx = self.rng.below(4) as u32;
        let inp = self.input(h, idx, 0, value + 100_000, height.saturating_sub(10));
        tx.tx_in.push(inp);
        let to = self.receive_address(account);
        let foreign = self.hash20();
        let mut outputs = Vec::new();
        let wallet_out = TxOut {
            value,
            version: 0,
            pk_script: chain::p2pkh(&to),
        };
        let change = TxOut {
            value: 100_000 - 2_500,
            version: 0,
            pk_script: chain::p2pkh(&foreign),
        };
        let wallet_index = if self.unit() < 0.5 {
            tx.tx_out.push(wallet_out);
            tx.tx_out.push(change);
            0
        } else {
            tx.tx_out.push(change);
            tx.tx_out.push(wallet_out);
            1
        };
        outputs.push((account, wallet_index, value, 0i8, UtxoKind::Regular));
        self.stats.receives += 1;
        Self::finish(tx, outputs, Vec::new(), None)
    }

    fn finish(
        tx: MsgTx,
        outs: Vec<(u32, u32, i64, i8, UtxoKind)>,
        inputs: Vec<(u32, Utxo)>,
        ticket: Option<Ticket>,
    ) -> Built {
        let hash = tx.tx_hash().0;
        let outputs = outs
            .into_iter()
            .map(|(account, index, value, tree, kind)| {
                (
                    account,
                    Utxo {
                        hash,
                        index,
                        tree,
                        value,
                        height: 0,
                        kind,
                    },
                )
            })
            .collect();
        let ticket = ticket.map(|mut t| {
            t.hash = hash;
            t
        });
        Built {
            tx,
            outputs,
            inputs,
            ticket,
        }
    }

    /// A send from one account: change to the same account usually, to
    /// another account sometimes, and sometimes no change at all, so that
    /// transactions which only debit an account are common.
    fn send(&mut self, height: u32, consolidate: bool) -> Option<Built> {
        let account = self.pick_account();
        let n = if consolidate {
            5 + self.below(16)
        } else {
            match self.below(10) {
                0..=6 => 1,
                7..=8 => 2,
                _ => 3,
            }
        };
        let ins = self.take_utxos(account, n, height, 20_000);
        if ins.is_empty() || (consolidate && ins.len() < 5) {
            self.utxos[account as usize].extend(ins);
            return None;
        }
        let mut tx = Self::new_tx();
        let mut total = 0i64;
        for u in &ins {
            let inp = self.input(u.hash, u.index, u.tree, u.value, u.height);
            tx.tx_in.push(inp);
            total += u.value;
        }
        let fee = 2_000 + 1_500 * ins.len() as i64;
        let mut outputs = Vec::new();
        if consolidate {
            let to = self.fresh(account, 1);
            tx.tx_out.push(TxOut {
                value: total - fee,
                version: 0,
                pk_script: chain::p2pkh(&to),
            });
            outputs.push((account, 0, total - fee, 0, UtxoKind::Regular));
            self.stats.consolidations += 1;
        } else {
            let shape = self.unit();
            let (no_change, other) = (shape < 0.04, (0.04..0.10).contains(&shape));
            let pay = if no_change {
                total - fee
            } else {
                ((total - fee) as f64 * (0.2 + 0.75 * self.unit())) as i64
            };
            let change = total - fee - pay;
            let payee = self.hash20();
            let pay_out = TxOut {
                value: pay,
                version: 0,
                pk_script: chain::p2pkh(&payee),
            };
            if change >= 6_000 {
                let to_account = if other && self.p.accounts > 1 {
                    (account + 1 + self.below(self.p.accounts as usize - 1) as u32)
                        % self.p.accounts
                } else {
                    account
                };
                if to_account != account {
                    self.stats.cross_account += 1;
                }
                let to = self.fresh(to_account, 1);
                let change_out = TxOut {
                    value: change,
                    version: 0,
                    pk_script: chain::p2pkh(&to),
                };
                let ci = if self.unit() < 0.5 {
                    tx.tx_out.push(change_out);
                    tx.tx_out.push(pay_out);
                    0
                } else {
                    tx.tx_out.push(pay_out);
                    tx.tx_out.push(change_out);
                    1
                };
                outputs.push((to_account, ci, change, 0, UtxoKind::Regular));
            } else {
                tx.tx_out.push(pay_out);
                self.stats.no_change += 1;
            }
            self.stats.sends += 1;
        }
        let inputs = ins.into_iter().map(|u| (account, u)).collect();
        Some(Self::finish(tx, outputs, inputs, None))
    }

    fn ticket(&mut self, height: u32, force_change: bool) -> Option<Built> {
        let account = self.pick_account();
        let price = 50_000_000 + (self.unit() * 150_000_000.0) as i64;
        let fee = 3_000;
        let min = if force_change {
            price + fee + 10_000
        } else {
            price + fee
        };
        let ins = self.take_utxos(account, 1, height, min);
        let u = ins.first()?.clone();
        if u.kind != UtxoKind::Regular {
            self.utxos[account as usize].push(u);
            return None;
        }
        let mut tx = Self::new_tx();
        let inp = self.input(u.hash, u.index, u.tree, u.value, u.height);
        tx.tx_in.push(inp);
        let vote_addr = self.fresh(account, 1);
        let commit_addr = self.fresh(account, 1);
        let change = u.value - price - fee;
        tx.tx_out.push(TxOut {
            value: price,
            version: 0,
            pk_script: chain::tagged_p2pkh(OP_SSTX, &vote_addr),
        });
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: chain::commitment(&commit_addr, price + fee),
        });
        let mut outputs = vec![(account, 0, price, 1i8, UtxoKind::Regular)];
        // Ticket change: half the tickets keep it (exercising the ticket
        // change category); the rest have a zero-value change output, as
        // dcrwallet's own split-funded tickets do.
        let (change_value, change_addr) = if change >= 10_000 && (force_change || self.unit() < 0.5)
        {
            (change, self.fresh(account, 1))
        } else {
            (0, self.hash20())
        };
        tx.tx_out.push(TxOut {
            value: change_value,
            version: 0,
            pk_script: chain::tagged_p2pkh(OP_SSTXCHANGE, &change_addr),
        });
        if change_value > 0 {
            outputs.push((account, 2, change_value, 1, UtxoKind::TicketChange));
        }
        self.stats.tickets += 1;
        let t = Ticket {
            hash: [0; 32],
            height,
            price,
            commit_hash: commit_addr,
            commit_amount: price + fee,
            account,
        };
        // Output 0 is a voting-authority credit, not spendable; it is not
        // tracked as a UTXO by the generator (the vote spends it).
        outputs.remove(0);
        Some(Self::finish(tx, outputs, vec![(account, u)], Some(t)))
    }

    fn vote(&mut self, t: &Ticket, height: u32, prev: &[u8; 32]) -> Built {
        let subsidy = 3_000_000;
        let mut tx = Self::new_tx();
        tx.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: Hash([0; 32]),
                index: u32::MAX,
                tree: 0,
            },
            sequence: u32::MAX,
            value_in: subsidy,
            block_height: 0,
            block_index: u32::MAX,
            signature_script: vec![0x00, 0x00],
        });
        let inp = self.input(t.hash, 0, 1, t.price, t.height);
        tx.tx_in.push(inp);
        let mut reference = Vec::with_capacity(36);
        reference.extend_from_slice(prev);
        reference.extend_from_slice(&(height - 1).to_le_bytes());
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: chain::nulldata(&reference),
        });
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: chain::nulldata(&[0x01, 0x00]),
        });
        let reward = t.commit_amount + subsidy;
        tx.tx_out.push(TxOut {
            value: reward,
            version: 0,
            pk_script: chain::tagged_p2pkh(OP_SSGEN, &t.commit_hash),
        });
        self.stats.votes += 1;
        Self::finish(
            tx,
            vec![(t.account, 2, reward, 1, UtxoKind::StakeGen)],
            Vec::new(),
            None,
        )
    }

    fn coinbase(&mut self, height: u32) -> Built {
        let account = self.pick_account();
        let value = 1_000_000_000 + (self.unit() * 1e9) as i64;
        let mut tx = Self::new_tx();
        tx.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: Hash([0; 32]),
                index: u32::MAX,
                tree: 0,
            },
            sequence: u32::MAX,
            value_in: value,
            block_height: 0,
            block_index: u32::MAX,
            // The height and a nonce, so that a block replacing this one in
            // a reorganisation could carry a different coinbase.
            signature_script: [
                height.to_le_bytes(),
                (self.rng.next_u64() as u32).to_le_bytes(),
            ]
            .concat(),
        });
        let to = self.fresh(account, 0);
        tx.tx_out.push(TxOut {
            value,
            version: 0,
            pk_script: chain::p2pkh(&to),
        });
        self.stats.coinbases += 1;
        Self::finish(
            tx,
            vec![(account, 0, value, 0, UtxoKind::Coinbase)],
            Vec::new(),
            None,
        )
    }

    /// Votes one mature ticket, if any.
    fn vote_any(&mut self, height: u32, prev: &[u8; 32]) -> Option<Built> {
        let i = self
            .tickets
            .iter()
            .position(|t| height > t.height + TICKET_MATURITY + 1)?;
        let t = self.tickets.swap_remove(i);
        Some(self.vote(&t, height, prev))
    }

    fn poisson(&mut self, lambda: f64) -> u64 {
        if lambda <= 0.0 {
            return 0;
        }
        // Knuth's method, split for large means.
        let mut n = 0u64;
        let mut lam = lambda;
        while lam > 0.0 {
            let step = lam.min(30.0);
            lam -= step;
            let l = (-step).exp();
            let mut p = 1.0;
            loop {
                p *= self.unit();
                if p <= l {
                    break;
                }
                n += 1;
            }
        }
        n
    }
}

fn header(prev: [u8; 32], height: u32, approve: bool, rng: &mut SplitMix64) -> BlockHeader {
    let mut h = [0u8; 64];
    rng.fill(&mut h);
    BlockHeader {
        version: 1,
        prev_block: Hash(prev),
        merkle_root: Hash(h[..32].try_into().expect("32")),
        stake_root: Hash(h[32..].try_into().expect("32")),
        vote_bits: u16::from(approve),
        final_state: [0; 6],
        voters: 5,
        fresh_stake: 0,
        revocations: 0,
        pool_size: 64,
        bits: 0x207f_ffff,
        sbits: 100_000_000,
        height,
        size: 1_000,
        timestamp: 1_700_000_000 + height * 300,
        nonce: rng.next_u64() as u32,
        extra_data: [0; 32],
        stake_version: 0,
    }
}

/// Generates the workload `p` describes.
///
/// Before the tail, blocks arrive in batches during history sync. In the
/// tail, each block is its own write transaction, most non-stake
/// transactions are first seen unmined, and every eight
/// blocks one of the scenarios runs: stake invalidation of the parent,
/// its reversal after a rollback, and reorganisations of one to three
/// blocks. The last blocks carry outputs on both sides of every maturity
/// boundary at the final tip: coinbases at depths 1, 15, 16 and 17, votes at
/// depths 1 and 17, ticket change at depths 1 and 2.
pub fn generate(p: &Params) -> (Workload, Stats) {
    let wseed = wallet_seed(p.seed);
    let master = ExtendedKey::new_master(&wseed, &HdVersions::SIMNET).expect("master key");
    let coin = master
        .child(44 + HARDENED)
        .and_then(|k| k.child(LEGACY_COIN_TYPE + HARDENED))
        .expect("coin type key");
    let accts = (0..p.accounts)
        .map(|a| {
            let acct = coin.child(a + HARDENED).expect("account key").neuter();
            Account {
                branches: [
                    acct.child(0).expect("branch"),
                    acct.child(1).expect("branch"),
                ],
                derived: [Vec::new(), Vec::new()],
                next: [0, 0],
                used_ext: Vec::new(),
            }
        })
        .collect();
    let mut g = Gen {
        rng: SplitMix64(p.seed),
        p: p.clone(),
        accts,
        utxos: vec![Vec::new(); p.accounts as usize],
        tickets: Vec::new(),
        pending: Vec::new(),
        stats: Stats::default(),
    };
    let mut hrng = SplitMix64(p.seed ^ 0x4ead_e500);
    let mut ops = Vec::new();
    let mut prev = SIMNET_GENESIS;
    let mut generated = 0u64;
    let tail_start = p.blocks.saturating_sub(p.tail_blocks) + 1;
    // Block hashes and contents of the tail, for scenarios.
    let mut hashes: BTreeMap<u32, [u8; 32]> = BTreeMap::new();
    let mut recent: BTreeMap<u32, (BlockHeader, Vec<Mined>)> = BTreeMap::new();
    let mut fate = 0usize;
    // Scenario heights: from 8 blocks into the tail to 24 before its end.
    let mut schedule: BTreeMap<u32, Scenario> = BTreeMap::new();
    let mut s = tail_start + 8;
    let mut k = 0usize;
    while s + 24 <= p.blocks {
        let sc = ROTATION[k % ROTATION.len()];
        schedule.insert(s, sc);
        if sc == Scenario::Revalidate {
            schedule.insert(s + 1, Scenario::Reorg(1));
        }
        s += SCENARIO_SPACING;
        k += 1;
    }
    let mut revalidate_set: Vec<Mined> = Vec::new();
    for height in 1..=p.blocks {
        let in_tail = height >= tail_start;
        let mut block: Vec<Mined> = Vec::new();
        let mut new_utxos: Vec<(u32, Utxo)> = Vec::new();
        let mut returned: Vec<(u32, Utxo)> = Vec::new();
        let mut approve = true;
        let mut reorg_ops: Vec<Op> = Vec::new();

        match schedule.get(&height).copied() {
            Some(Scenario::Disapprove) | Some(Scenario::Revalidate) => {
                let all = schedule.get(&height) == Some(&Scenario::Revalidate);
                approve = false;
                g.stats.disapprovals += 1;
                let parent = recent
                    .get(&(height - 1))
                    .map(|b| b.1.clone())
                    .unwrap_or_default();
                for t in parent.into_iter().filter(|t| t.regular) {
                    if t.coinbase {
                        // Lost with the regular tree; for a revalidation it
                        // comes back, and it is immature either way.
                        if !all {
                            g.drop_outputs(&t.hash);
                        }
                        continue;
                    }
                    g.drop_outputs(&t.hash);
                    g.stats.invalidated += 1;
                    // Fates in rotation, so every disapproval with a few
                    // transactions exercises each: mined again here, mined
                    // again later, or never.
                    fate += 1;
                    let x = [0.0, 0.6, 0.0, 0.9][fate % 4];
                    if all || x < 0.5 {
                        g.stats.remined += 1;
                        new_utxos.extend(t.outputs.iter().cloned());
                        if all {
                            revalidate_set.push(t.clone());
                        }
                        block.push(t);
                    } else if x < 0.75 {
                        g.stats.remined += 1;
                        let mine_at = height + 1 + g.rng.below(2) as u32;
                        g.pending.push(Pending {
                            bytes: t.bytes,
                            mine_at,
                            outputs: t.outputs,
                            inputs: t.inputs,
                        });
                    } else {
                        g.stats.dropped += 1;
                        returned.extend(t.inputs);
                    }
                }
            }
            Some(Scenario::Reorg(depth)) => {
                g.stats.reorgs += 1;
                let fork = height - depth;
                reorg_ops.push(Op {
                    kind: OpKind::Reorg,
                    commit: false,
                    unpublished: false,
                    header: None,
                    rollback_to: fork,
                    txs: Vec::new(),
                });
                let revalidating = !revalidate_set.is_empty();
                let delay = !revalidating && g.unit() < 0.5;
                let mut chain_prev = hashes[&(fork - 1)];
                let mut delayed = Vec::new();
                for hh in fork..height {
                    let (_, txs) = recent.remove(&hh).expect("recent block");
                    let mut keep = Vec::new();
                    for t in txs {
                        if t.coinbase {
                            g.drop_outputs(&t.hash);
                            continue;
                        }
                        if revalidate_set.iter().any(|r| r.hash == t.hash) {
                            // Rolled back to unmined; the parent's
                            // validation makes it mined there again.
                            g.move_outputs(&t.hash, hh - 1);
                            continue;
                        }
                        if delay && hh + 1 == height && t.regular {
                            delayed.push(t);
                            continue;
                        }
                        keep.push(t);
                    }
                    let hd = header(chain_prev, hh, true, &mut hrng);
                    reorg_ops.push(Op {
                        kind: OpKind::TipBlock,
                        commit: false,
                        unpublished: false,
                        header: Some(hd.serialize()),
                        rollback_to: 0,
                        txs: keep.iter().map(|t| t.bytes.clone()).collect(),
                    });
                    chain_prev = hd.block_hash().0;
                    hashes.insert(hh, chain_prev);
                    recent.insert(hh, (hd, keep));
                }
                revalidate_set.clear();
                for t in delayed {
                    g.stats.delayed += 1;
                    g.move_outputs(&t.hash, height);
                    block.push(t);
                }
                prev = chain_prev;
            }
            None => {}
        }

        // Previously unmined (or invalidated) transactions mined here.
        let mut k = 0;
        while k < g.pending.len() {
            if g.pending[k].mine_at == height {
                let pd = g.pending.remove(k);
                new_utxos.extend(pd.outputs.iter().cloned());
                block.push(Mined::of(pd.bytes, pd.outputs, pd.inputs));
            } else {
                k += 1;
            }
        }

        // Votes by mature tickets.
        let mut i = 0;
        while i < g.tickets.len() {
            let mature = height > g.tickets[i].height + TICKET_MATURITY + 1;
            if mature && g.unit() < 1.0 / 40.0 {
                let t = g.tickets.swap_remove(i);
                let b = g.vote(&t, height, &prev);
                new_utxos.extend(b.outputs.iter().cloned());
                block.push(Mined::of(b.tx.serialize(), b.outputs, b.inputs));
                generated += 1;
            } else {
                i += 1;
            }
        }

        // Outputs on both sides of each maturity boundary at the final tip,
        // and regular transactions (a receive and a send) in every block a
        // scenario acts on.
        let depth = p.blocks - height + 1;
        let mut forced = Vec::new();
        if schedule.contains_key(&(height + 1)) {
            forced.push(g.receive(height));
            if let Some(b) = (0..8).find_map(|_| g.send(height, false)) {
                forced.push(b);
            }
        }
        if matches!(depth, 1 | 15 | 16 | 17) {
            forced.push(g.coinbase(height));
        }
        if matches!(depth, 1 | 17)
            && let Some(b) = g.vote_any(height, &prev)
        {
            forced.push(b);
        }
        if matches!(depth, 1 | 2)
            && let Some(b) = (0..20).find_map(|_| g.ticket(height, true))
        {
            forced.push(b);
        }
        for b in forced {
            generated += 1;
            new_utxos.extend(b.outputs.iter().cloned());
            if let Some(t) = b.ticket.clone() {
                g.tickets.push(t);
            }
            block.push(Mined::of(b.tx.serialize(), b.outputs, b.inputs));
        }

        // New transactions.
        let remaining_blocks = u64::from(p.blocks - height + 1);
        let lambda = p.txs.saturating_sub(generated) as f64 / remaining_blocks as f64;
        let n = g.poisson(lambda);
        let mut unmined_ops = Vec::new();
        for _ in 0..n {
            let x = g.unit();
            let built = if x < 0.35 {
                g.send(height, false)
            } else if x < 0.36 {
                g.send(height, true)
            } else if x < 0.48 {
                g.ticket(height, false)
            } else if x < 0.485 {
                Some(g.coinbase(height))
            } else {
                None
            };
            let built = built.unwrap_or_else(|| g.receive(height));
            generated += 1;
            let is_stake = built.ticket.is_some() || chain::is_coinbase(&built.tx);
            let bytes = built.tx.serialize();
            if in_tail && !is_stake && g.unit() < 0.9 {
                // Seen unmined first; mined one to three blocks later, or
                // never if that is past the end of the chain.
                let mine_at = height + 1 + g.rng.below(3) as u32;
                let never = mine_at > p.blocks;
                let unpublished = never && g.unit() < 0.3;
                g.stats.unmined_first += 1;
                if never {
                    g.stats.unmined_at_end += 1;
                }
                unmined_ops.push(Op {
                    kind: OpKind::Unmined,
                    commit: true,
                    unpublished,
                    header: None,
                    rollback_to: 0,
                    txs: vec![bytes.clone()],
                });
                g.pending.push(Pending {
                    bytes,
                    mine_at,
                    outputs: built.outputs,
                    inputs: built.inputs,
                });
            } else {
                new_utxos.extend(built.outputs.iter().cloned());
                if let Some(t) = built.ticket {
                    g.tickets.push(t);
                }
                block.push(Mined::of(bytes, built.outputs, built.inputs));
            }
        }

        let hd = header(prev, height, approve, &mut g.rng);
        let hb = hd.serialize();
        prev = hd.block_hash().0;
        let kind = if in_tail {
            OpKind::TipBlock
        } else {
            OpKind::SyncBlock
        };
        let commit = in_tail || height % p.batch_blocks == 0 || height + 1 == tail_start;
        // A transaction seen unmined at this height can only be mined in a
        // later block; it reaches the store before this block does only if
        // it spends nothing this block creates, which holds because new
        // outputs become spendable from the next height on. A
        // reorganisation goes after them, with the new chain's blocks and
        // this block in one write transaction, as `ChainSwitch` writes it.
        ops.extend(unmined_ops);
        ops.extend(reorg_ops);
        ops.push(Op {
            kind,
            commit,
            unpublished: false,
            header: Some(hb),
            rollback_to: 0,
            txs: block.iter().map(|t| t.bytes.clone()).collect(),
        });
        // New outputs take this height; the inputs of a transaction the
        // disapproval dropped come back with theirs.
        for (account, mut u) in new_utxos.into_iter().chain(returned) {
            if u.height == 0 {
                u.height = height;
            }
            g.utxos[account as usize].push(u);
        }
        if height + 1 >= tail_start {
            hashes.insert(height, prev);
        }
        if in_tail {
            recent.insert(height, (hd, block));
            let old: Vec<u32> = recent
                .range(..height.saturating_sub(8))
                .map(|(h, _)| *h)
                .collect();
            for h in old {
                recent.remove(&h);
            }
        }
    }
    // Transactions seen after the last block and never mined.
    let height = p.blocks + 1;
    for _ in 0..p.final_unmined {
        let send = g.unit() < 0.6;
        let built = if send { g.send(height, false) } else { None };
        let is_send = built.is_some();
        let built = built.unwrap_or_else(|| g.receive(height));
        let unpublished = is_send && g.unit() < 0.35;
        g.stats.unmined_first += 1;
        g.stats.unmined_at_end += 1;
        ops.push(Op {
            kind: OpKind::Unmined,
            commit: true,
            unpublished,
            header: None,
            rollback_to: 0,
            txs: vec![built.tx.serialize()],
        });
    }

    // The workload records every derived address plus the gap beyond the
    // last one used, as the wallet's address sync would.
    let mut ranges = Vec::new();
    for a in 0..p.accounts {
        for b in 0..2u32 {
            let want = g.accts[a as usize].next[b as usize] + chain::GAP_LIMIT + 1;
            for i in 0..want {
                g.address(a, b, i);
            }
            let hashes = g.accts[a as usize].derived[b as usize][..want as usize].to_vec();
            g.stats.addresses += hashes.len() as u64;
            ranges.push(AddrRange {
                account: a,
                branch: b,
                hashes,
            });
        }
    }
    let w = Workload {
        wallet_seed: wseed,
        accounts: p.accounts,
        ranges,
        ops,
    };
    (w, g.stats)
}

/// Writes `w` to `path`.
pub fn write(w: &Workload, path: &Path) -> io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    f.write_all(b"PURSEWL1")?;
    f.write_all(&w.accounts.to_le_bytes())?;
    f.write_all(&u32_len(w.ranges.len()).to_le_bytes())?;
    for r in &w.ranges {
        f.write_all(&r.account.to_le_bytes())?;
        f.write_all(&r.branch.to_le_bytes())?;
        f.write_all(&u32_len(r.hashes.len()).to_le_bytes())?;
        for h in &r.hashes {
            f.write_all(h)?;
        }
    }
    f.write_all(&u32_len(w.ops.len()).to_le_bytes())?;
    for op in &w.ops {
        f.write_all(&[op.kind as u8])?;
        match op.kind {
            OpKind::SyncBlock | OpKind::TipBlock => {
                f.write_all(&[u8::from(op.commit)])?;
                f.write_all(op.header.as_ref().expect("block header"))?;
                f.write_all(&u32_len(op.txs.len()).to_le_bytes())?;
            }
            OpKind::Unmined => {
                f.write_all(&[u8::from(op.unpublished)])?;
            }
            OpKind::Reorg => {
                f.write_all(&op.rollback_to.to_le_bytes())?;
            }
        }
        for tx in &op.txs {
            f.write_all(&u32_len(tx.len()).to_le_bytes())?;
            f.write_all(tx)?;
        }
    }
    f.flush()
}

fn u32_len(n: usize) -> u32 {
    u32::try_from(n).expect("length fits in u32")
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_bytes(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let n = read_u32(r)? as usize;
    let mut b = vec![0; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// Reads a workload file. The wallet seed is not in the file and comes back
/// zeroed.
pub fn read(path: &Path) -> io::Result<Workload> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut magic = [0; 8];
    f.read_exact(&mut magic)?;
    if &magic != b"PURSEWL1" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a workload"));
    }
    let accounts = read_u32(&mut f)?;
    let nranges = read_u32(&mut f)?;
    let mut ranges = Vec::new();
    for _ in 0..nranges {
        let account = read_u32(&mut f)?;
        let branch = read_u32(&mut f)?;
        let n = read_u32(&mut f)?;
        let mut hashes = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut h = [0; 20];
            f.read_exact(&mut h)?;
            hashes.push(h);
        }
        ranges.push(AddrRange {
            account,
            branch,
            hashes,
        });
    }
    let nops = read_u32(&mut f)?;
    let mut ops = Vec::with_capacity(nops as usize);
    for _ in 0..nops {
        let kind = match read_u8(&mut f)? {
            1 => OpKind::SyncBlock,
            2 => OpKind::Unmined,
            3 => OpKind::TipBlock,
            4 => OpKind::Reorg,
            k => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad op kind {k}"),
                ));
            }
        };
        let op = match kind {
            OpKind::SyncBlock | OpKind::TipBlock => {
                let commit = read_u8(&mut f)? != 0;
                let mut h = [0; 180];
                f.read_exact(&mut h)?;
                let n = read_u32(&mut f)?;
                let mut txs = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    txs.push(read_bytes(&mut f)?);
                }
                Op {
                    kind,
                    commit,
                    unpublished: false,
                    header: Some(h),
                    rollback_to: 0,
                    txs,
                }
            }
            OpKind::Reorg => Op {
                kind,
                commit: false,
                unpublished: false,
                header: None,
                rollback_to: read_u32(&mut f)?,
                txs: Vec::new(),
            },
            OpKind::Unmined => {
                let unpublished = read_u8(&mut f)? != 0;
                Op {
                    kind,
                    commit: true,
                    unpublished,
                    header: None,
                    rollback_to: 0,
                    txs: vec![read_bytes(&mut f)?],
                }
            }
        };
        ops.push(op);
    }
    Ok(Workload {
        wallet_seed: [0; 32],
        accounts,
        ranges,
        ops,
    })
}

/// The write transactions of a workload: index ranges into `ops`, each
/// ending at an operation marked commit (or at the last operation).
pub fn txn_groups(ops: &[Op]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, op) in ops.iter().enumerate() {
        if op.commit || i + 1 == ops.len() {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    out
}
