// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Corrupt and hostile files: random byte damage and targeted structural
//! mutations of the frozen fixture, and hand-built files for paths no
//! generated file reaches. The reader must return `Ok` or `Err`, never
//! panic, and every operation must terminate (a watchdog aborts the run if
//! one case takes more than 10 s). Every targeted case asserts its exact
//! outcome.
//!
//! Cases that Go bbolt can read without panicking are also compared with it
//! when the oracle is available: the outcome (error kind, txid, dump) must
//! be bbolt's.

// Test code indexing a known-good fixture at fixed offsets; an overflow
// here is a test bug and panics loudly in debug builds.
#![allow(clippy::arithmetic_side_effects)]

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{Synth, Watchdog, fix_checksum};
use purse_boltread::{Corruption, Db, Error, OpenOptions, Source, dump};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{Oracle, SplitMix64, oracle_or_skip};

const PS: usize = 4096;

fn get_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn get_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn get_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}
fn set_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn set_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn set_u64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// Meta field offsets within the meta structure.
const M_MAGIC: usize = 0;
const M_VERSION: usize = 4;
const M_PAGE_SIZE: usize = 8;
const M_ROOT: usize = 16;
const M_FREELIST: usize = 32;
const M_PGID: usize = 40;
const M_TXID: usize = 48;
const M_CHECKSUM: usize = 56;

fn meta_off(page: usize) -> usize {
    page * PS + 16
}

/// A reachable page of the (valid) fixture.
#[derive(Clone, Debug)]
struct PageInfo {
    id: u64,
    flags: u16,
    count: u16,
    overflow: u32,
    /// The branch page that points here, if any.
    parent: Option<u64>,
    /// The bucket the page belongs to (index into `Layout::buckets`).
    bucket: usize,
}

impl PageInfo {
    fn off(&self) -> usize {
        self.id as usize * PS
    }
    fn end(&self) -> u64 {
        self.id + 1 + u64::from(self.overflow)
    }
}

/// A bucket of the fixture.
#[derive(Clone, Debug)]
struct BucketInfo {
    path: Vec<Vec<u8>>,
    inline: bool,
    /// File offset of the leaf element header (0 for the root bucket).
    header: usize,
    /// File offset of the value.
    value: usize,
    /// The page holding the value.
    holder: u64,
}

struct Layout {
    newest: usize,
    /// Reachable pages in the order the reader's whole-tree walk visits
    /// them.
    pages: Vec<PageInfo>,
    buckets: Vec<BucketInfo>,
}

/// Walks the valid fixture from its newest meta (no checks: the file is
/// known good), in the walk order of the reader's dump.
fn layout(b: &[u8]) -> Layout {
    fn visit(b: &[u8], id: u64, parent: Option<u64>, bucket: usize, lay: &mut Layout) {
        let off = id as usize * PS;
        let p = PageInfo {
            id,
            flags: get_u16(b, off + 8),
            count: get_u16(b, off + 10),
            overflow: get_u32(b, off + 12),
            parent,
            bucket,
        };
        lay.pages.push(p.clone());
        for i in 0..usize::from(p.count) {
            let h = off + 16 + 16 * i;
            if p.flags == 0x01 {
                visit(b, get_u64(b, h + 8), Some(id), bucket, lay);
            } else if get_u32(b, h) & 1 != 0 {
                let pos = get_u32(b, h + 4) as usize;
                let ksize = get_u32(b, h + 8) as usize;
                let value = h + pos + ksize;
                let broot = get_u64(b, value);
                let mut path = lay.buckets[bucket].path.clone();
                path.push(b[h + pos..h + pos + ksize].to_vec());
                lay.buckets.push(BucketInfo {
                    path,
                    inline: broot == 0,
                    header: h,
                    value,
                    holder: id,
                });
                if broot != 0 {
                    let idx = lay.buckets.len() - 1;
                    visit(b, broot, None, idx, lay);
                }
            }
        }
    }
    let t0 = get_u64(b, meta_off(0) + M_TXID);
    let t1 = get_u64(b, meta_off(1) + M_TXID);
    let newest = usize::from(t1 > t0);
    let root = get_u64(b, meta_off(newest) + M_ROOT);
    let mut lay = Layout {
        newest,
        pages: Vec::new(),
        buckets: vec![BucketInfo {
            path: Vec::new(),
            inline: false,
            header: 0,
            value: 0,
            holder: root,
        }],
    };
    visit(b, root, None, 0, &mut lay);
    lay
}

/// What a run of the reader over some bytes produced.
#[derive(Debug)]
struct Outcome {
    /// The open's error, or the meta page in use.
    open: Result<u64, Error>,
    /// The dump's digest or error (when the open succeeded).
    dump: Option<Result<String, Error>>,
}

/// Opens `bytes` and exercises every read path with bounded effort.
fn exercise(bytes: Vec<u8>, rng: &mut SplitMix64) -> Outcome {
    let db = match common::open_bytes(bytes) {
        Ok(db) => db,
        Err(e) => {
            return Outcome {
                open: Err(e),
                dump: None,
            };
        }
    };
    let dumped = dump(&db, &mut std::io::sink()).map(|s| s.sha256_hex());
    let _ = db.root().stats();
    let _ = db.freelist();
    let _ = db.verify_unchanged();
    // Cursors over the root and the first buckets reachable from it.
    let mut queue = vec![db.root()];
    let mut seen = 0;
    while let Some(b) = queue.pop() {
        seen += 1;
        if seen > 12 {
            break;
        }
        let mut keys = Vec::new();
        for item in b.iter().take(2000) {
            let Ok(item) = item else { break };
            if item.is_bucket()
                && queue.len() < 12
                && let Ok(Some(child)) = b.bucket(item.key())
            {
                queue.push(child);
            }
            keys.push(item.key().to_vec());
        }
        let mut c = b.cursor();
        for _ in 0..40 {
            let key = if keys.is_empty() || rng.below(3) == 0 {
                let n = rng.below(20) as usize;
                rng.bytes(n)
            } else {
                keys[rng.below(keys.len() as u64) as usize].clone()
            };
            let _ = match rng.below(7) {
                0 => c.first(),
                1 => c.last(),
                2 | 3 => c.next(),
                4 => c.prev(),
                5 => c.seek(&key),
                _ => {
                    let _ = b.bucket(&key);
                    b.get(&key).map(|_| None)
                }
            };
        }
    }
    Outcome {
        open: Ok(db.meta_page()),
        dump: Some(dumped),
    }
}

/// Runs `exercise` and turns a panic into a failure naming the case.
fn must_not_panic(what: &str, bytes: Vec<u8>, rng: &mut SplitMix64) -> Outcome {
    match catch_unwind(AssertUnwindSafe(|| exercise(bytes, rng))) {
        Ok(r) => r,
        Err(_) => panic!("reader panicked on {what}"),
    }
}

/// Random damage: a few thousand deterministic iterations over the frozen
/// fixtures and one upstream fixture.
#[test]
fn random_damage_never_panics() {
    let wd = Watchdog::new("random_damage_never_panics");
    let frozen = common::frozen();
    let deep = common::fixture_bytes("frozen-deep-4k.db");
    let up = common::fixture_bytes("upstream/v24.db.gz");
    let mut rng = SplitMix64::from_env("corrupt-random");
    let mut stats = [0u64; 3]; // open failed, dump failed, dump ok
    // PURSE_DAMAGE_SCALE multiplies the iteration counts for a longer soak.
    let scale: usize = std::env::var("PURSE_DAMAGE_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    for (name, base, iters) in [
        ("frozen-4k", &frozen, 2000 * scale),
        ("frozen-deep-4k", &deep, 1000 * scale),
        ("v24", &up, 1000 * scale),
    ] {
        let lay = layout(base);
        let pgid = get_u64(base, meta_off(lay.newest) + M_PGID) as usize;
        let used = (pgid * PS).min(base.len());
        for iter in 0..iters {
            let mut m = base.clone();
            let n = 1 + rng.below(4);
            let mut desc = Vec::new();
            for _ in 0..n {
                // Half the time hit a header: a page header or element
                // header of a reachable page, or a meta.
                let pos = if rng.below(2) == 0 {
                    rng.below(used as u64) as usize
                } else if rng.below(4) == 0 {
                    meta_off(rng.below(2) as usize) + rng.below(64) as usize
                } else {
                    let p = &lay.pages[rng.below(lay.pages.len() as u64) as usize];
                    let span = 16 + 16 * usize::from(p.count);
                    p.off() + rng.below(span as u64) as usize
                };
                match rng.below(5) {
                    0 => m[pos] ^= 1 << rng.below(8),
                    1 => m[pos] = rng.below(256) as u8,
                    2 => m[pos] = if rng.below(2) == 0 { 0 } else { 0xff },
                    3 if pos + 4 <= m.len() => {
                        let v = rng.next_u64() as u32;
                        set_u32(&mut m, pos, v);
                    }
                    _ if pos + 8 <= m.len() => {
                        let v = rng.next_u64() >> rng.below(64);
                        set_u64(&mut m, pos, v);
                    }
                    _ => m[pos] = !m[pos],
                }
                desc.push(pos);
            }
            let what = format!("{name} iteration {iter} (offsets {desc:?})");
            wd.case(&what);
            let out = must_not_panic(&what, m, &mut rng);
            match (&out.open, &out.dump) {
                (Err(_), _) => stats[0] += 1,
                (_, Some(Err(_))) => stats[1] += 1,
                _ => stats[2] += 1,
            }
        }
    }
    eprintln!(
        "random damage: {} iterations; open failed {}, dump failed {}, dump ok {}",
        stats.iter().sum::<u64>(),
        stats[0],
        stats[1],
        stats[2]
    );
}

/// What a targeted case must produce.
enum Expect {
    /// The open fails with an error the predicate accepts.
    OpenErr(fn(&Error) -> bool),
    /// The open succeeds and the dump has this digest.
    Digest(String),
    /// The open succeeds and the dump fails with this corruption on this
    /// page.
    Corrupt(u64, Corruption),
}

/// One targeted mutation and its exact outcome.
struct Case {
    name: String,
    bytes: Vec<u8>,
    expect: Expect,
    /// Whether Go bbolt reads the file without panicking or faulting, so
    /// the outcome can be compared with it.
    go: bool,
    /// Whether `Db::freelist` must fail.
    freelist_err: bool,
}

fn check(case: &Case, out: &Outcome) -> bool {
    match (&case.expect, &out.open, &out.dump) {
        (Expect::OpenErr(f), Err(e), _) => f(e),
        (Expect::Digest(d), Ok(_), Some(Ok(got))) => got == d,
        (
            Expect::Corrupt(page, detail),
            Ok(_),
            Some(Err(Error::Corrupt { page: p, detail: d })),
        ) => p == page && d == detail,
        _ => false,
    }
}

/// The frozen fixture's ids, offsets and digests the cases are built from.
struct Fixture {
    base: Vec<u8>,
    lay: Layout,
    base_digest: String,
    older_digest: String,
}

fn fixture() -> Fixture {
    let base = common::frozen();
    let lay = layout(&base);
    assert_eq!(lay.newest, 1, "frozen fixture's newest meta is page 1");
    Fixture {
        base,
        lay,
        base_digest: common::recorded_digest("frozen-4k.db"),
        older_digest: common::recorded_digest("frozen-4k.db@meta0"),
    }
}

fn oob_at(end_page: u64, file_pages: u64) -> Corruption {
    Corruption::PageOutOfBounds {
        end_page,
        file_pages,
    }
}

/// The reader's first out-of-bounds page when the fixture is cut to
/// `file_pages` pages, in walk order.
fn first_oob(lay: &Layout, file_pages: u64) -> Option<&PageInfo> {
    lay.pages.iter().find(|p| p.end() > file_pages)
}

#[allow(clippy::too_many_lines)]
fn targeted_cases() -> Vec<Case> {
    let fx = fixture();
    let (base, lay) = (&fx.base, &fx.lay);
    let mut cases: Vec<Case> = Vec::new();
    let mut add = |name: &str, f: &dyn Fn(&mut Vec<u8>), expect: Expect, go: bool| {
        let mut b = base.clone();
        f(&mut b);
        cases.push(Case {
            name: name.to_owned(),
            bytes: b,
            expect,
            go,
            freelist_err: false,
        });
    };
    let base_d = || Expect::Digest(fx.base_digest.clone());
    let older_d = || Expect::Digest(fx.older_digest.clone());
    let cor = |page: u64, detail: Corruption| Expect::Corrupt(page, detail);

    // Meta pages. Page 1 is the newer (txid 5), page 0 the older (txid 4).
    for (field, val, kind) in [
        (M_MAGIC, 0xdead_beef_u32, "magic"),
        (M_VERSION, 3, "version"),
        (M_CHECKSUM, 1, "checksum"),
    ] {
        let write = move |b: &mut Vec<u8>, page: usize| {
            if field == M_CHECKSUM {
                b[meta_off(page) + field] ^= 1;
            } else {
                set_u32(b, meta_off(page) + field, val);
            }
        };
        add(
            &format!("bad {kind} on the newer meta"),
            &|b| write(b, 1),
            older_d(),
            true,
        );
        add(
            &format!("bad {kind} on the older meta"),
            &|b| write(b, 0),
            base_d(),
            true,
        );
        let both: fn(&Error) -> bool = match kind {
            "magic" => |e| matches!(e, Error::Invalid),
            "version" => |e| matches!(e, Error::VersionMismatch),
            _ => |e| matches!(e, Error::Checksum),
        };
        add(
            &format!("bad {kind} on both metas"),
            &|b| {
                write(b, 0);
                write(b, 1);
            },
            Expect::OpenErr(both),
            true,
        );
    }
    add(
        "meta 0 bad version, meta 1 bad magic: meta 0's error",
        &|b| {
            set_u32(b, meta_off(0) + M_VERSION, 9);
            set_u32(b, meta_off(1) + M_MAGIC, 0);
        },
        Expect::OpenErr(|e| matches!(e, Error::VersionMismatch)),
        true,
    );
    add(
        "both metas valid with equal txid 4: meta 0 wins",
        &|b| {
            set_u64(b, meta_off(1) + M_TXID, 4);
            fix_checksum(b, 1, PS);
        },
        older_d(),
        true,
    );
    add(
        "both metas valid with equal txid 5: meta 0 wins",
        &|b| {
            set_u64(b, meta_off(0) + M_TXID, 5);
            fix_checksum(b, 0, PS);
        },
        older_d(),
        true,
    );
    // Page size (meta 0 decides it while meta 0 validates).
    for ps in [0u32, 512, 1000, 4097, 3 << 12] {
        add(
            &format!("meta 0 page size {ps}"),
            &|b| {
                set_u32(b, meta_off(0) + M_PAGE_SIZE, ps);
                fix_checksum(b, 0, PS);
            },
            Expect::OpenErr(|e| matches!(e, Error::UnsupportedPageSize(_))),
            false,
        );
    }
    for ps in [1u32 << 31, u32::MAX, 1 << 20] {
        add(
            &format!("meta 0 page size {ps:#x}: file too small"),
            &|b| {
                set_u32(b, meta_off(0) + M_PAGE_SIZE, ps);
                fix_checksum(b, 0, PS);
            },
            Expect::OpenErr(|e| matches!(e, Error::FileTooSmall { .. })),
            true,
        );
    }
    let meta0_root = get_u64(base, meta_off(0) + M_ROOT);
    add(
        "meta 0 page size 2048 (a valid size, the wrong one)",
        &|b| {
            set_u32(b, meta_off(0) + M_PAGE_SIZE, 2048);
            fix_checksum(b, 0, PS);
        },
        // Read with 2048-byte pages, the root's header is whatever lies at
        // that offset; its overflow field runs past the end of the file.
        cor(
            meta0_root,
            oob_at(
                meta0_root + 1 + u64::from(get_u32(base, meta0_root as usize * 2048 + 12)),
                128,
            ),
        ),
        false,
    );
    add(
        "meta 1 page size 8192, checksum fixed: bbolt ignores it",
        &|b| {
            set_u32(b, meta_off(1) + M_PAGE_SIZE, 8192);
            fix_checksum(b, 1, PS);
        },
        base_d(),
        true,
    );
    add(
        "page size 0 on both metas",
        &|b| {
            for p in 0..2 {
                set_u32(b, meta_off(p) + M_PAGE_SIZE, 0);
                fix_checksum(b, p, PS);
            }
        },
        Expect::OpenErr(|e| matches!(e, Error::UnsupportedPageSize(0))),
        false,
    );
    // Root pointers in the newest meta (checksum fixed, so it stays in use).
    let fl = get_u64(base, meta_off(1) + M_FREELIST);
    let oob = |end_page: u64| oob_at(end_page, 64);
    for (root, name, detail) in [
        (1u64 << 40, "beyond EOF", oob((1 << 40) + 1)),
        (u64::MAX, "u64::MAX", oob(u64::MAX)),
        (
            u64::MAX / PS as u64,
            "offset overflow",
            oob(u64::MAX / PS as u64 + 1),
        ),
        (
            1,
            "a meta page",
            Corruption::UnexpectedPageType { flags: 4 },
        ),
        (
            fl,
            "the freelist page",
            Corruption::UnexpectedPageType { flags: 0x10 },
        ),
        (0, "0 (inline root)", Corruption::InlineRootBucket),
    ] {
        add(
            &format!("root page {name}"),
            &|b| {
                set_u64(b, meta_off(1) + M_ROOT, root);
                fix_checksum(b, 1, PS);
            },
            cor(root, detail),
            false,
        );
    }
    // Truncation.
    for len in [
        0usize,
        1,
        79,
        80,
        1024,
        2048,
        2049,
        4095,
        4096,
        4097,
        8191,
        8192,
        12288,
        58 * PS - 1,
        58 * PS,
        59 * PS,
        base.len() / 3,
    ] {
        let file_pages = (len / PS) as u64;
        let (expect, go) = match len {
            0 => (Expect::OpenErr(|e| matches!(e, Error::EmptyFile)), false),
            1..=2048 => (Expect::OpenErr(|e| matches!(e, Error::Invalid)), true),
            2049..=8191 => (
                Expect::OpenErr(|e| matches!(e, Error::FileTooSmall { .. })),
                true,
            ),
            _ => match first_oob(lay, file_pages) {
                Some(p) => (
                    cor(
                        p.id,
                        Corruption::PageOutOfBounds {
                            end_page: p.end(),
                            file_pages,
                        },
                    ),
                    false,
                ),
                None => (base_d(), true),
            },
        };
        add(
            &format!("truncated to {len} bytes"),
            &|b| b.truncate(len),
            expect,
            go,
        );
    }

    // Pages of the tree.
    let branches: Vec<&PageInfo> = lay.pages.iter().filter(|p| p.flags == 1).collect();
    let leaves: Vec<&PageInfo> = lay.pages.iter().filter(|p| p.flags == 2).collect();
    let overflow_leaf = leaves
        .iter()
        .find(|p| p.overflow > 0)
        .copied()
        .expect("overflow leaf");
    let deep_branch = branches
        .iter()
        .find(|p| p.parent.is_some())
        .copied()
        .expect("branch below a branch");
    let root_branch = deep_branch.parent.expect("parent");
    // A leaf of plain pairs, so that changing its keys leaves the
    // bucket structure alone.
    let leaf = leaves
        .iter()
        .find(|p| {
            p.overflow == 0
                && p.count >= 2
                && (0..usize::from(p.count)).all(|i| get_u32(base, p.off() + 16 + 16 * i) & 1 == 0)
        })
        .copied()
        .expect("leaf of plain pairs");
    let lo = leaf.off();
    let bo = deep_branch.off();

    add(
        "overflow count huge on a leaf",
        &|b| set_u32(b, overflow_leaf.off() + 12, u32::MAX),
        cor(
            overflow_leaf.id,
            oob(overflow_leaf.id + 1 + u64::from(u32::MAX)),
        ),
        false,
    );
    add(
        "overflow count huge on a branch",
        &|b| set_u32(b, bo + 12, 0x7fff_ffff),
        cor(deep_branch.id, oob(deep_branch.id + 0x8000_0000)),
        false,
    );
    add(
        "element count 0xffff on a leaf",
        &|b| set_u16(b, lo + 10, 0xffff),
        cor(leaf.id, Corruption::ElementCountOutOfPage { count: 0xffff }),
        false,
    );
    add(
        "element count 0xffff on a branch",
        &|b| set_u16(b, bo + 10, 0xffff),
        cor(
            deep_branch.id,
            Corruption::ElementCountOutOfPage { count: 0xffff },
        ),
        false,
    );
    add(
        "element count 0 on a branch",
        &|b| set_u16(b, bo + 10, 0),
        cor(deep_branch.id, Corruption::EmptyBranch),
        false,
    );
    for (field, v, name) in [
        (4usize, 0xffff_fff0u32, "pos"),
        (8, 0x7fff_ffff, "ksize"),
        (12, u32::MAX, "vsize"),
    ] {
        add(
            &format!("leaf element {name} out of page"),
            &|b| set_u32(b, lo + 16 + 16 + field, v),
            cor(leaf.id, Corruption::ElementOutOfPage { index: 1 }),
            false,
        );
    }
    add(
        "branch element pos out of page",
        &|b| set_u32(b, bo + 16, 0xffff_0000),
        cor(deep_branch.id, Corruption::ElementOutOfPage { index: 0 }),
        false,
    );
    add(
        "leaf element with a zero-length key (bbolt reads it)",
        &|b| set_u32(b, lo + 16 + 8, 0),
        Expect::Digest(ZERO_KEY_DIGEST.to_owned()),
        true,
    );
    add(
        "branch element pgid beyond EOF",
        &|b| set_u64(b, bo + 16 + 8, 1 << 33),
        cor(1 << 33, oob((1 << 33) + 1)),
        false,
    );
    add(
        "branch pointing at itself",
        &|b| set_u64(b, bo + 16 + 8, deep_branch.id),
        cor(
            deep_branch.id,
            Corruption::PageReferencedTwice {
                pgid: deep_branch.id,
            },
        ),
        false,
    );
    add(
        "branch pointing at its parent",
        &|b| set_u64(b, bo + 16 + 8, root_branch),
        cor(
            root_branch,
            Corruption::PageReferencedTwice { pgid: root_branch },
        ),
        false,
    );
    let shared = get_u64(base, bo + 16 + 8);
    add(
        "two branch elements sharing a child",
        &|b| set_u64(b, bo + 32 + 8, shared),
        cor(shared, Corruption::PageReferencedTwice { pgid: shared }),
        false,
    );
    add(
        "branch pointing at the meta page",
        &|b| set_u64(b, bo + 16 + 8, 0),
        cor(0, Corruption::UnexpectedPageType { flags: 4 }),
        false,
    );
    // Read as a branch, the leaf's first element names the page whose id
    // is its ksize (low word) and vsize (high word).
    let as_branch_child =
        u64::from(get_u32(base, lo + 16 + 8)) | (u64::from(get_u32(base, lo + 16 + 12)) << 32);
    add(
        "leaf flagged as branch",
        &|b| set_u16(b, lo + 8, 1),
        cor(as_branch_child, oob(as_branch_child + 1)),
        false,
    );
    // Read as a leaf, a branch element's `pos` is the flags word and its
    // page id the value size (high word) behind a key of the low word's
    // length: the first odd `pos` makes a "bucket" with a short value.
    let odd = (0..usize::from(deep_branch.count))
        .map(|i| bo + 16 + 16 * i)
        .find(|&h| get_u32(base, h) & 1 == 1)
        .expect("a branch element with an odd pos");
    add(
        "branch flagged as leaf",
        &|b| set_u16(b, bo + 8, 2),
        cor(
            deep_branch.id,
            Corruption::BucketValueTooShort {
                len: (get_u64(base, odd + 8) >> 32) as usize,
            },
        ),
        false,
    );
    for flags in [0u16, 3, 0x20, 4, 0x10, 0xffff] {
        add(
            &format!("leaf flags {flags:#x}"),
            &|b| set_u16(b, lo + 8, flags),
            cor(leaf.id, Corruption::UnexpectedPageType { flags }),
            false,
        );
    }
    add(
        "page header id mismatch",
        &|b| set_u64(b, lo, leaf.id + 1),
        cor(leaf.id, Corruption::PageIdMismatch { found: leaf.id + 1 }),
        false,
    );
    // Overlapping pages (Visited::mark). (a) A page visited later lies
    // inside an earlier page's range, made by widening the earlier one.
    let visited_before = |i: usize, lo_id: u64, hi_id: u64| {
        lay.pages[..i]
            .iter()
            .any(|q| q.id <= hi_id && q.end() > lo_id)
    };
    let (a, later) = lay
        .pages
        .windows(2)
        .enumerate()
        .find(|(i, w)| w[1].id >= w[0].end() && !visited_before(*i, w[0].end(), w[1].id))
        .map(|(_, w)| (w[0].clone(), w[1].clone()))
        .expect("pages in increasing order");
    add(
        "a later page inside an earlier page's overflow range",
        &|b| set_u32(b, a.off() + 12, (later.id - a.id) as u32),
        cor(later.id, Corruption::PageReferencedTwice { pgid: later.id }),
        false,
    );
    // (b) A page visited later whose widened range covers an earlier page.
    let (earlier, c) = lay
        .pages
        .windows(2)
        .find(|w| w[1].id < w[0].id)
        .map(|w| (w[0].clone(), w[1].clone()))
        .expect("pages in decreasing order");
    add(
        "a later page's overflow range covering an earlier page",
        &|b| set_u32(b, c.off() + 12, (earlier.id - c.id) as u32),
        cor(c.id, Corruption::PageReferencedTwice { pgid: earlier.id }),
        false,
    );
    // Overflow read exactly to the end of the file, and one page past it.
    let last = lay
        .pages
        .iter()
        .max_by_key(|p| p.end())
        .expect("pages")
        .clone();
    add(
        "overflow widened by one page, still inside the file",
        &|b| set_u32(b, last.off() + 12, last.overflow + 1),
        base_d(),
        true,
    );
    add(
        "overflow widened by one page past the end of the file",
        &|b| {
            set_u32(b, last.off() + 12, last.overflow + 1);
            b.truncate(last.end() as usize * PS);
        },
        cor(
            last.id,
            Corruption::PageOutOfBounds {
                end_page: last.end() + 1,
                file_pages: last.end(),
            },
        ),
        false,
    );

    // Buckets.
    let inline = lay
        .buckets
        .iter()
        .find(|e| e.inline && get_u16(base, e.value + 16 + 10) > 0)
        .cloned()
        .expect("non-empty inline bucket");
    let regular = lay.buckets[1..]
        .iter()
        .find(|e| !e.inline)
        .cloned()
        .expect("regular bucket");
    add(
        "bucket value shorter than its header",
        &|b| set_u32(b, regular.header + 12, 3),
        cor(regular.holder, Corruption::BucketValueTooShort { len: 3 }),
        false,
    );
    add(
        "empty bucket value",
        &|b| set_u32(b, regular.header + 12, 0),
        cor(regular.holder, Corruption::BucketValueTooShort { len: 0 }),
        false,
    );
    add(
        "inline value too short for its page",
        &|b| set_u32(b, inline.header + 12, 20),
        cor(inline.holder, Corruption::BucketValueTooShort { len: 20 }),
        false,
    );
    add(
        "inline page count huge",
        &|b| set_u16(b, inline.value + 16 + 10, 0x7fff),
        cor(
            inline.holder,
            Corruption::ElementCountOutOfPage { count: 0x7fff },
        ),
        false,
    );
    add(
        "inline page flagged as branch",
        &|b| set_u16(b, inline.value + 16 + 8, 1),
        cor(inline.holder, Corruption::InlinePageNotLeaf { flags: 1 }),
        false,
    );
    add(
        "inline page with extra flag bits (bbolt reads it as a leaf)",
        &|b| set_u16(b, inline.value + 16 + 8, 0x12),
        base_d(),
        true,
    );
    add(
        "inline element out of its value",
        &|b| set_u32(b, inline.value + 16 + 16 + 4, 4000),
        cor(inline.holder, Corruption::ElementOutOfPage { index: 0 }),
        false,
    );
    // An inline bucket's element whose value is shorter than a bucket
    // header, flagged as a nested bucket.
    let (iholder, ih, ivsize) = lay
        .buckets
        .iter()
        .filter(|e| e.inline)
        .flat_map(|e| {
            let n = usize::from(get_u16(base, e.value + 16 + 10));
            (0..n).map(move |i| (e.holder, e.value + 16 + 16 + 16 * i))
        })
        .map(|(h, eh)| (h, eh, get_u32(base, eh + 12) as usize))
        .find(|&(_, _, v)| v < 16)
        .expect("inline element with a short value");
    add(
        "inline element flagged as a bucket with a short value",
        &|b| b[ih] |= 1,
        cor(iholder, Corruption::BucketValueTooShort { len: ivsize }),
        false,
    );
    add(
        "regular bucket root pointing at the page holding it",
        &|b| set_u64(b, regular.value, regular.holder),
        cor(
            regular.holder,
            Corruption::PageReferencedTwice {
                pgid: regular.holder,
            },
        ),
        false,
    );
    add(
        "bucket element losing its bucket flag (dumped as a pair)",
        &|b| b[regular.header] &= !1,
        Expect::Digest(UNFLAGGED_DIGEST.to_owned()),
        true,
    );

    // The freelist (not read by the dump).
    let flo = fl as usize * PS;
    for (name, f) in [
        (
            "freelist beyond EOF",
            (|b: &mut Vec<u8>| {
                set_u64(b, meta_off(1) + M_FREELIST, 1 << 30);
                fix_checksum(b, 1, PS);
            }) as fn(&mut Vec<u8>),
        ),
        ("freelist page not a freelist", |b| {
            let at = get_u64(b, meta_off(1) + M_FREELIST) as usize * PS + 8;
            set_u16(b, at, 2);
        }),
    ] {
        add(name, &f, base_d(), false);
    }
    add(
        "freelist count escape with a huge count",
        &|b| {
            set_u16(b, flo + 10, 0xffff);
            set_u64(b, flo + 16, u64::MAX / 2);
        },
        base_d(),
        false,
    );
    for c in &mut cases {
        c.freelist_err = c.name.starts_with("freelist");
    }
    cases
}

/// Digests of the targeted cases whose dump succeeds with changed content;
/// `targeted_cases_match_go` checks each against Go.
const ZERO_KEY_DIGEST: &str = "568646ef354dfdf8f21a655f3575978e342b3792a71b9bc721ebc0c4984ce04a";
const UNFLAGGED_DIGEST: &str = "db2fa4748b00f67223ec594fcc7b31cbd84a4a2148ac639df61832d468757160";

#[test]
fn targeted_mutations() {
    let wd = Watchdog::new("targeted_mutations");
    let mut rng = SplitMix64::from_env("corrupt-targeted");
    let cases = targeted_cases();
    let n = cases.len();
    let mut failures = Vec::new();
    for case in &cases {
        wd.case(&case.name);
        let out = must_not_panic(&case.name, case.bytes.clone(), &mut rng);
        if !check(case, &out) {
            failures.push(format!("{}: {out:?}", case.name));
        }
        if case.freelist_err {
            let db = common::open_bytes(case.bytes.clone()).expect("opens");
            assert!(db.freelist().is_err(), "{}", case.name);
        }
    }
    assert!(
        failures.is_empty(),
        "unexpected outcomes:\n{}",
        failures.join("\n")
    );
    eprintln!("targeted mutations: {n} cases");
}

/// The Go kind of a Rust open error.
fn kind_of(e: &Error) -> &'static str {
    match e {
        Error::Invalid => "invalid",
        Error::VersionMismatch => "version",
        Error::Checksum => "checksum",
        Error::FileTooSmall { .. } => "toosmall",
        _ => "other",
    }
}

/// Compares the reader with Go on a file: the open's error kind, or the
/// txid and dump. Returns Go's dump result.
fn compare_with_go(oracle: &mut Oracle, path: &Path, name: &str, preload: bool) -> Option<Value> {
    let go = oracle.try_call("dump", json!({"path": path, "preload_freelist": preload}));
    let rust = common::open_beside_go(path).and_then(|db| {
        let s = dump(&db, &mut std::io::sink())?;
        Ok((db.meta().txid, s.sha256_hex()))
    });
    match (&go, &rust) {
        (Ok(g), Ok((txid, sha))) => {
            assert_eq!(g["txid"].as_u64(), Some(*txid), "{name}: txid");
            assert_eq!(g["sha256"].as_str(), Some(sha.as_str()), "{name}: dump");
        }
        (Err(g), Err(r)) => {
            assert_eq!(
                g.kind.as_deref(),
                Some(kind_of(r)),
                "{name}: go {g:?}, rust {r}"
            );
        }
        _ => panic!("{name}: go {go:?}, rust {rust:?}"),
    }
    go.ok()
}

/// Every targeted case Go can read: same outcome as bbolt; plus random
/// damage confined to the meta pages.
#[test]
fn targeted_cases_match_go() {
    let Some(mut oracle) = oracle_or_skip("boltdump") else {
        return;
    };
    let wd = Watchdog::new("targeted_cases_match_go");
    let dir = common::scratch("corrupt-go");
    let path = dir.join("case.db");
    let mut n = 0;
    for case in targeted_cases().into_iter().filter(|c| c.go) {
        wd.case(&case.name);
        std::fs::write(&path, &case.bytes).expect("write");
        compare_with_go(&mut oracle, &path, &case.name, false);
        n += 1;
    }
    let base = common::frozen();
    let mut rng = SplitMix64::from_env("corrupt-meta-go");
    for iter in 0..400 {
        let name = format!("meta damage {iter}");
        wd.case(&name);
        let mut m = base.clone();
        let pages: &[usize] = match iter % 3 {
            0 => &[0],
            1 => &[1],
            _ => &[0, 1],
        };
        for &p in pages {
            for _ in 0..1 + rng.below(3) {
                let pos = p * PS + rng.below(96) as usize;
                m[pos] ^= 1 << rng.below(8);
            }
        }
        // The txid of the undamaged meta can be changed with its checksum
        // fixed, flipping which meta is newer.
        if iter % 7 == 0 && pages.len() == 1 {
            let p = 1 - pages[0];
            set_u64(&mut m, meta_off(p) + M_TXID, rng.below(8));
            fix_checksum(&mut m, p, PS);
        }
        std::fs::write(&path, &m).expect("write");
        compare_with_go(&mut oracle, &path, &name, false);
        n += 1;
    }
    eprintln!("compared with Go: {n} cases");
}

/// A source counting the bytes read from it.
struct Counting {
    data: Vec<u8>,
    read: AtomicU64,
}

impl Source for &'static Counting {
    fn size(&self) -> std::io::Result<u64> {
        Ok(self.data.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        self.read.fetch_add(buf.len() as u64, Ordering::Relaxed);
        let at = offset as usize;
        buf.copy_from_slice(
            self.data
                .get(at..at + buf.len())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?,
        );
        Ok(())
    }
}

/// A cursor running into a cycle stops before reading a page twice, even
/// when no page fits in the cache: a self-referencing bucket root, and two
/// pages pointing at each other.
#[test]
fn cursor_cycles_fail_without_rereading() {
    let wd = Watchdog::new("cursor_cycles_fail_without_rereading");
    let fx = fixture();
    let lay = &fx.lay;
    // A bucket whose root is a branch over branches.
    let rootb = lay
        .pages
        .iter()
        .find(|p| {
            p.flags == 1
                && p.parent.is_none()
                && lay
                    .pages
                    .iter()
                    .any(|q| q.parent == Some(p.id) && q.flags == 1)
        })
        .expect("bucket rooted at a branch over branches")
        .clone();
    let child = lay
        .pages
        .iter()
        .find(|q| q.parent == Some(rootb.id) && q.flags == 1)
        .expect("child branch")
        .clone();
    let path = lay.buckets[rootb.bucket].path.clone();
    let point_all = |b: &mut Vec<u8>, from: &PageInfo, to: u64| {
        let n = usize::from(get_u16(b, from.off() + 10));
        for i in 0..n {
            set_u64(b, from.off() + 16 + 16 * i + 8, to);
        }
    };
    let mut selfref = fx.base.clone();
    point_all(&mut selfref, &rootb, rootb.id);
    let mut pingpong = fx.base.clone();
    point_all(&mut pingpong, &rootb, child.id);
    point_all(&mut pingpong, &child, rootb.id);
    for (name, bytes, cycle_at) in [
        ("self-referencing root", selfref, rootb.id),
        ("ping-pong", pingpong, rootb.id),
    ] {
        let src: &'static Counting = Box::leak(Box::new(Counting {
            data: bytes,
            read: AtomicU64::new(0),
        }));
        // A cache smaller than one page: nothing is ever cached.
        let db = Db::from_source(src, &OpenOptions::new().cache_bytes(1024)).expect("open");
        let b = db.bucket_path(&path).expect("lookup").expect("bucket");
        let want = Corruption::Cycle { pgid: cycle_at };
        let mut c = b.cursor();
        type Op = fn(
            &mut purse_boltread::Cursor<'_>,
        ) -> purse_boltread::Result<Option<purse_boltread::Item>>;
        let ops: [(&str, Op); 5] = [
            ("first", |c| c.first()),
            ("next", |c| c.next()),
            ("prev", |c| c.prev()),
            ("last", |c| c.last()),
            ("seek", |c| c.seek(b"\x80")),
        ];
        for (op, f) in ops {
            wd.case(&format!("{name}: {op}"));
            let before = src.read.load(Ordering::Relaxed);
            let r = f(&mut c);
            let read = src.read.load(Ordering::Relaxed) - before;
            match r {
                Err(Error::Corrupt { detail, .. }) => assert_eq!(detail, want, "{name}: {op}"),
                other => panic!("{name}: {op}: {other:?}"),
            }
            // At most the pages of one root-to-cycle path.
            assert!(read <= 4 * PS as u64, "{name}: {op} read {read} bytes");
        }
        // The dump reports the page reached twice.
        assert!(matches!(
            dump(&db, &mut std::io::sink()),
            Err(Error::Corrupt {
                detail: Corruption::PageReferencedTwice { .. },
                ..
            })
        ));
    }
}

/// Iteration over a page graph with a shared child stops with an error
/// instead of repeating the child's entries; a shared empty leaf is caught
/// by the empty-leaf check.
#[test]
fn shared_pages_stop_iteration() {
    let wd = Watchdog::new("shared_pages_stop_iteration");
    let fx = fixture();
    let lay = &fx.lay;
    // A branch over at least three leaves on the leftmost path of its
    // bucket, so that First reaches it.
    let leftmost = |p: &PageInfo| {
        let mut cur = p.clone();
        while let Some(parent) = cur.parent {
            let pp = lay
                .pages
                .iter()
                .find(|q| q.id == parent)
                .expect("parent")
                .clone();
            if get_u64(&fx.base, pp.off() + 16 + 8) != cur.id {
                return false;
            }
            cur = pp;
        }
        true
    };
    let br = lay
        .pages
        .iter()
        .find(|p| {
            p.flags == 1
                && p.count >= 3
                && leftmost(p)
                && lay
                    .pages
                    .iter()
                    .filter(|q| q.parent == Some(p.id))
                    .all(|q| q.flags == 2)
        })
        .expect("leftmost branch over leaves")
        .clone();
    let first_child = get_u64(&fx.base, br.off() + 16 + 8);
    let path = lay.buckets[br.bucket].path.clone();
    let mut shared = fx.base.clone();
    for i in 0..usize::from(br.count) {
        set_u64(&mut shared, br.off() + 16 + 16 * i + 8, first_child);
    }
    wd.case("shared child");
    let db = common::open_bytes(shared.clone()).expect("open");
    let b = db.bucket_path(&path).expect("lookup").expect("bucket");
    let mut n = 0;
    let mut err = None;
    for item in b.iter() {
        match item {
            Ok(_) => n += 1,
            Err(e) => err = Some(e),
        }
        assert!(n < 10_000, "iteration does not stop");
    }
    assert!(
        matches!(
            err,
            Some(Error::Corrupt {
                detail: Corruption::KeysOutOfOrder,
                ..
            })
        ),
        "{err:?}"
    );
    // Backwards too.
    let mut c = b.cursor();
    let mut r = c.last();
    let mut steps = 0;
    while let Ok(Some(_)) = r {
        r = c.prev();
        steps += 1;
        assert!(steps < 10_000);
    }
    assert!(
        matches!(
            r,
            Err(Error::Corrupt {
                detail: Corruption::KeysOutOfOrder,
                ..
            })
        ),
        "{r:?}"
    );

    // The shared child emptied: Next skips the same empty leaf twice.
    wd.case("shared empty leaf");
    let mut empty = shared;
    set_u16(&mut empty, first_child as usize * PS + 10, 0);
    let db = common::open_bytes(empty).expect("open");
    let b = db.bucket_path(&path).expect("lookup").expect("bucket");
    let r = b.cursor().first();
    assert!(
        matches!(r, Err(Error::Corrupt { detail: Corruption::TooManyEmptyLeaves, page }) if page == first_child),
        "{r:?}"
    );
}

/// A logical page as large as a sparse file's apparent length is refused
/// before anything is allocated.
#[test]
fn huge_pages_in_sparse_files() {
    let _wd = Watchdog::new("huge_pages_in_sparse_files");
    let fx = fixture();
    let leaf = fx
        .lay
        .pages
        .iter()
        .find(|p| p.flags == 2)
        .expect("leaf")
        .clone();
    let dir = common::scratch("corrupt-sparse");
    let path = dir.join("sparse.db");
    // Unix filesystems keep the extended file sparse; NTFS would allocate
    // it, so Windows gets a smaller one.
    #[cfg(not(windows))]
    let (file_len, overflow) = (64u64 << 30, 1u32 << 20);
    #[cfg(windows)]
    let (file_len, overflow) = (1u64 << 30, 1u32 << 17);
    let mut b = fx.base.clone();
    set_u32(&mut b, leaf.off() + 12, overflow);
    std::fs::write(&path, &b).expect("write");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|f| f.set_len(file_len))
        .expect("set_len (sparse)");
    let bytes = (u64::from(overflow) + 1) * PS as u64;
    for (max, opts) in [
        (256u64 << 20, OpenOptions::new()),
        (1 << 20, OpenOptions::new().max_page_bytes(1 << 20)),
    ] {
        let db = Db::open_with(&path, &opts).expect("open");
        let e = dump(&db, &mut std::io::sink()).expect_err("page too large");
        assert!(
            matches!(&e, Error::Corrupt { page, detail: Corruption::PageTooLarge { bytes: n, max: m } }
                if *page == leaf.id && *n == bytes && *m == max),
            "{e}"
        );
    }
    // Pages within the limit still read: the unmodified fixture, sparse.
    std::fs::write(&path, &fx.base).expect("write");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|f| f.set_len(file_len))
        .expect("set_len");
    let db = Db::open(&path).expect("open");
    assert_eq!(
        dump(&db, &mut std::io::sink()).expect("dump").sha256_hex(),
        fx.base_digest
    );
    drop(db);
    let _ = std::fs::remove_file(&path);
}

fn key(i: usize) -> Vec<u8> {
    format!("key-{i:04}").into_bytes()
}

/// A tree of `depth` levels: a chain of single-element branch pages over
/// one leaf.
fn chain(depth: usize) -> Synth {
    let mut s = Synth::new(PS, 2);
    for d in 0..depth - 1 {
        s.branch(2 + d as u64, &[(key(0), 3 + d as u64)]);
    }
    s.leaf(1 + depth as u64, &[(key(0), b"v".to_vec())]);
    s
}

/// The empty-leaf layout: a root branch over five leaves of three pairs,
/// the second and the last leaf emptied.
fn empty_leaves() -> (Synth, Vec<Vec<u8>>) {
    let mut s = Synth::new(PS, 2);
    let mut children = Vec::new();
    let mut live = Vec::new();
    for l in 0..5usize {
        let items: Vec<(Vec<u8>, Vec<u8>)> =
            (0..3).map(|j| (key(l * 3 + j), vec![l as u8; 3])).collect();
        let id = 3 + l as u64;
        if l == 1 || l == 4 {
            s.leaf(id, &[]);
        } else {
            live.extend(items.iter().map(|(k, _)| k.clone()));
            s.leaf(id, &items);
        }
        children.push((key(l * 3), id));
    }
    s.branch(2, &children);
    (s, live)
}

/// The hand-built files: their outcomes, and Go's where Go can read them.
#[test]
fn synthetic_files() {
    let wd = Watchdog::new("synthetic_files");
    let dir = common::scratch("corrupt-synthetic");
    let mut oracle = oracle_or_skip("boltdump");

    // Tree depth: MAX_TREE_DEPTH levels read, one more is refused (bbolt
    // reads both; the limit is this reader's).
    for depth in [
        purse_boltread::MAX_TREE_DEPTH,
        purse_boltread::MAX_TREE_DEPTH + 1,
    ] {
        wd.case(&format!("chain of {depth}"));
        let s = chain(depth);
        let path = dir.join(format!("chain-{depth}.db"));
        s.write(&path);
        let db = common::open_beside_go(&path).expect("open");
        let r = dump(&db, &mut std::io::sink());
        let mut c = db.root().cursor();
        let first = c.first();
        if depth <= purse_boltread::MAX_TREE_DEPTH {
            assert_eq!(r.expect("dump").entries, 1);
            assert_eq!(first.expect("first").expect("item").key(), key(0));
            assert_eq!(
                db.root().get(&key(0)).expect("get").as_deref(),
                Some(&b"v"[..])
            );
            if let Some(o) = oracle.as_mut() {
                compare_with_go(o, &path, "deepest chain", false);
            }
        } else {
            for res in [r.map(|_| ()), first.map(|_| ())] {
                assert!(
                    matches!(
                        res,
                        Err(Error::Corrupt {
                            detail: Corruption::TreeTooDeep,
                            ..
                        })
                    ),
                    "{res:?}"
                );
            }
            if let Some(o) = oracle.as_mut() {
                let g = o.call("dump", json!({"path": path}));
                assert_eq!(
                    g["entries"].as_u64(),
                    Some(1),
                    "bbolt reads the deeper chain"
                );
            }
        }
    }

    // Empty middle and last leaves under a branch.
    wd.case("empty leaves");
    let (s, live) = empty_leaves();
    let path = dir.join("empty-leaves.db");
    s.write(&path);
    let db = common::open_beside_go(&path).expect("open");
    let fwd: Vec<Vec<u8>> = db
        .root()
        .iter()
        .map(|i| i.expect("item").key().to_vec())
        .collect();
    assert_eq!(fwd, live);
    let mut c = db.root().cursor();
    let k = |r: purse_boltread::Result<Option<purse_boltread::Item>>| {
        r.expect("op").map(|i| i.key().to_vec())
    };
    assert_eq!(k(c.last()), Some(key(11)));
    // Backwards: leaf 3, then bbolt's quirk of returning nothing when
    // stepping onto the empty leaf 1, then leaf 0, then the start.
    let mut back = Vec::new();
    for _ in 0..8 {
        back.push(k(c.prev()));
    }
    let want: Vec<Option<Vec<u8>>> = vec![
        Some(key(10)),
        Some(key(9)),
        Some(key(8)),
        Some(key(7)),
        Some(key(6)),
        None,
        Some(key(2)),
        Some(key(1)),
    ];
    assert_eq!(back, want);
    assert_eq!(k(c.prev()), Some(key(0)));
    assert_eq!(k(c.prev()), None);
    assert_eq!(k(c.next()), Some(key(1)));
    assert_eq!(k(c.seek(&key(3))), Some(key(6)));
    assert_eq!(k(c.seek(&key(13))), None);
    assert_eq!(k(c.prev()), Some(key(11)));
    if let Some(o) = oracle.as_mut() {
        compare_with_go(o, &path, "empty leaves", false);
        let mut ops = vec![json!({"op": "first"})];
        ops.extend((0..16).map(|_| json!({"op": "next"})));
        ops.push(json!({"op": "last"}));
        ops.extend((0..16).map(|_| json!({"op": "prev"})));
        ops.extend((0..16).map(|_| json!({"op": "next"})));
        for i in [0usize, 3, 4, 5, 12, 13, 14, 20] {
            ops.push(json!({"op": "seek", "key": purse_testutil::hex(&key(i))}));
            ops.push(json!({"op": "prev"}));
            ops.push(json!({"op": "next"}));
            ops.push(json!({"op": "next"}));
        }
        let go = o.call("cursor", json!({"path": path, "bucket": [], "ops": ops}));
        let mut c = db.root().cursor();
        for (op, want) in ops.iter().zip(go.as_array().expect("array")) {
            let got = match op["op"].as_str() {
                Some("first") => c.first(),
                Some("last") => c.last(),
                Some("next") => c.next(),
                Some("prev") => c.prev(),
                _ => c.seek(&purse_testutil::unhex(op["key"].as_str().expect("key"))),
            };
            assert_eq!(&common::item_json(&got.expect("op")), want, "{op}");
        }
    }

    // The meta-1 probe finds 16 MiB pages and no larger.
    for (shift, opens) in [(24u32, true), (25, false)] {
        wd.case(&format!("probe {shift}"));
        let ps = 1usize << shift;
        let mut s = Synth::new(ps, 2);
        s.leaf(2, &[]);
        s.break_meta0 = true;
        let path = dir.join(format!("probe-{shift}.db"));
        s.write(&path);
        let r = common::open_beside_go(&path);
        if opens {
            let db = r.expect("16 MiB pages open via meta 1");
            assert_eq!((db.page_size(), db.meta_page()), (ps as u32, 1));
            assert_eq!(dump(&db, &mut std::io::sink()).expect("dump").entries, 0);
        } else {
            assert!(matches!(r, Err(Error::Checksum)), "{r:?}");
        }
        if let Some(o) = oracle.as_mut() {
            compare_with_go(o, &path, &format!("probe {shift}"), false);
        }
        let _ = std::fs::remove_file(&path);
    }

    // A freelist with the 0xFFFF count escape.
    wd.case("freelist escape");
    let mut s = Synth::new(PS, 2);
    s.leaf(2, &[(key(0), b"v".to_vec())]);
    let mut ids: Vec<u64> = (0..70_000u64)
        .map(|i| 1000 + (i * 7919) % 70_000 * 2)
        .collect();
    s.freelist_page(3, &ids);
    let path = dir.join("freelist-escape.db");
    s.write(&path);
    let db = common::open_beside_go(&path).expect("open");
    ids.sort_unstable();
    assert_eq!(db.freelist().expect("freelist").expect("synced"), ids);
    if let Some(o) = oracle.as_mut() {
        let g = compare_with_go(o, &path, "freelist escape", true).expect("go dump");
        assert_eq!(g["free_page_n"].as_u64(), Some(70_000));
    }
}
