// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A consistency checker of the chain tables written for the tests, apart
//! from the store's own `ReadTx::check_chain`: it reads the six tables and
//! the `tip` row as raw bytes, decodes keys and the few value fields it
//! needs by hand (a height is 4 bytes big-endian; a block row starts with
//! its 32-byte hash; a header states its previous block at bytes 4 to 36
//! and its height little-endian at 128 to 132), and checks both directions
//! of every correspondence:
//!
//! - main-chain heights in `blocks` run without a gap from the store's
//!   start (height 0, the genesis block) to the tip;
//! - every height in `blocks` has its header in `headers`, and every
//!   header's height has its block;
//! - every block's hash has its `block_by_hash` row naming its height, and
//!   every `block_by_hash` row names a height whose block has that hash;
//! - every header states its height and, above genesis, names the block
//!   below as its previous block;
//! - every filter in `cfilters` is at a height of `blocks`;
//! - the `tip` row is the last block, and absent with no main chain;
//! - no detached header or filter is of a block in `block_by_hash`, and
//!   every detached filter has its detached header.
//!
//! It works on any source of raw rows, so the model test runs it on its
//! model as well as on the store. On success it returns a [`ChainView`],
//! which the power-loss tests compare with the views before and after a
//! reorganisation.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use purse_store::{MetaKey, ReadTx, TableId};

/// A table's rows, raw, in key order.
pub type Rows = BTreeMap<Vec<u8>, Vec<u8>>;

/// What the chain tables hold, once found consistent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainView {
    /// The main chain's block hashes; the index is the height.
    pub main: Vec<[u8; 32]>,
    /// Heights whose main-chain block has its filter.
    pub filters: BTreeSet<u32>,
    /// Hashes with a detached header.
    pub detached: BTreeSet<[u8; 32]>,
    /// Hashes with a detached filter.
    pub detached_filters: BTreeSet<[u8; 32]>,
}

impl ChainView {
    /// The main-chain height of `hash`.
    pub fn height_of(&self, hash: &[u8; 32]) -> Option<u32> {
        self.main
            .iter()
            .position(|h| h == hash)
            .map(|p| u32::try_from(p).expect("height fits"))
    }
}

fn height_key(k: &[u8], table: &str) -> Result<u32, String> {
    let b: [u8; 4] = k
        .try_into()
        .map_err(|_| format!("{table}: a {}-byte height key", k.len()))?;
    Ok(u32::from_be_bytes(b))
}

fn hash_key(k: &[u8], table: &str) -> Result<[u8; 32], String> {
    k.try_into()
        .map_err(|_| format!("{table}: a {}-byte hash key", k.len()))
}

/// Checks the chain tables of a read transaction.
pub fn chain_view(tx: &ReadTx) -> Result<ChainView, String> {
    chain_view_of(|id| {
        tx.range_raw(id, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
            .map(|e| e.expect("entry"))
            .collect()
    })
}

/// Checks the chain tables of any source of raw rows: `rows(id)` returns
/// table `id`'s rows.
pub fn chain_view_of(rows: impl Fn(TableId) -> Rows) -> Result<ChainView, String> {
    let blocks = rows(TableId::Blocks);
    let headers = rows(TableId::Headers);
    let index = rows(TableId::BlockByHash);
    let cfilters = rows(TableId::CFilters);
    let headers_detached = rows(TableId::HeadersDetached);
    let cfilters_detached = rows(TableId::CFiltersDetached);
    let meta = rows(TableId::Meta);

    // Heights contiguous from the start (0) to the tip.
    let mut main: Vec<[u8; 32]> = Vec::with_capacity(blocks.len());
    for (k, v) in &blocks {
        let h = height_key(k, "blocks")?;
        if h as usize != main.len() {
            return Err(format!(
                "blocks: height {h} follows {} rows (the main chain starts at 0 and has no gap)",
                main.len()
            ));
        }
        if v.len() != 47 {
            return Err(format!("blocks: a {}-byte row at height {h}", v.len()));
        }
        main.push(v[..32].try_into().expect("32 bytes"));
    }

    // Headers: the same heights, each stating its height and parent.
    for (k, v) in &headers {
        let h = height_key(k, "headers")?;
        if h as usize >= main.len() {
            return Err(format!("headers: a header at height {h} with no block"));
        }
        if v.len() != 180 {
            return Err(format!("headers: a {}-byte header at height {h}", v.len()));
        }
        let stated = u32::from_le_bytes(v[128..132].try_into().expect("4 bytes"));
        if stated != h {
            return Err(format!("headers: the header at height {h} states {stated}"));
        }
        if h > 0 && v[4..36] != main[h as usize - 1] {
            return Err(format!(
                "headers: the header at height {h} does not name the block below"
            ));
        }
    }
    if headers.len() != main.len() {
        let missing = (0..main.len())
            .find(|&h| !headers.contains_key(&u32::try_from(h).expect("fits").to_be_bytes()[..]))
            .unwrap_or(main.len());
        return Err(format!("headers: no header at height {missing}"));
    }

    // The hash index, both ways.
    for (h, hash) in main.iter().enumerate() {
        let want = u32::try_from(h).expect("fits").to_be_bytes();
        match index.get(&hash[..]) {
            Some(v) if v[..] == want => {}
            Some(v) => {
                return Err(format!(
                    "block_by_hash: the block at height {h} is indexed at {}",
                    purse_testutil::hex(v)
                ));
            }
            None => return Err(format!("block_by_hash: the block at height {h} is missing")),
        }
    }
    for (k, v) in &index {
        let hash = hash_key(k, "block_by_hash")?;
        let h = height_key(v, "block_by_hash value")?;
        if main.get(h as usize) != Some(&hash) {
            return Err(format!(
                "block_by_hash: {} names height {h}, whose block is another or none",
                purse_testutil::hex(&hash[..6])
            ));
        }
    }

    // Filters only at main-chain heights.
    let mut filters = BTreeSet::new();
    for k in cfilters.keys() {
        let h = height_key(k, "cfilters")?;
        if h as usize >= main.len() {
            return Err(format!("cfilters: a filter at height {h} with no block"));
        }
        filters.insert(h);
    }

    // The tip row.
    let tip = meta.get(MetaKey::Tip.name());
    match (main.last(), tip) {
        (None, None) => {}
        (None, Some(_)) => return Err("meta: a tip row with no main chain".into()),
        (Some(_), None) => return Err("meta: no tip row".into()),
        (Some(last), Some(v)) => {
            let want_h = u32::try_from(main.len() - 1).expect("fits").to_be_bytes();
            if v.len() != 36 || v[..4] != want_h || v[4..] != last[..] {
                return Err("meta: the tip row is not the last block".into());
            }
        }
    }

    // Detached rows: never of a main-chain block; a filter has its header.
    let mut detached = BTreeSet::new();
    for k in headers_detached.keys() {
        let hash = hash_key(k, "headers_detached")?;
        if index.contains_key(k) {
            return Err(format!(
                "headers_detached: {} is in the main chain",
                purse_testutil::hex(&hash[..6])
            ));
        }
        detached.insert(hash);
    }
    let mut detached_filters = BTreeSet::new();
    for k in cfilters_detached.keys() {
        let hash = hash_key(k, "cfilters_detached")?;
        if index.contains_key(k) {
            return Err(format!(
                "cfilters_detached: {} is in the main chain",
                purse_testutil::hex(&hash[..6])
            ));
        }
        if !detached.contains(&hash) {
            return Err(format!(
                "cfilters_detached: {} has no detached header",
                purse_testutil::hex(&hash[..6])
            ));
        }
        detached_filters.insert(hash);
    }

    Ok(ChainView {
        main,
        filters,
        detached,
        detached_filters,
    })
}
