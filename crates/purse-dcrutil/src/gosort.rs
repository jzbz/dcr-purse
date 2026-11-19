// Copyright (c) 2026 The dcr-purse developers
// Copyright 2009, 2022 The Go Authors. All rights reserved.
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file. This port of Go's sort package is also under Go's
// BSD-style license; see the NOTICE file.

//! Go's `sort.Sort` and `sort.IsSorted` (`sort/sort.go`,
//! `sort/zsortinterface.go`, unchanged from Go 1.19 to the reference Go
//! 1.27.4), ported step for step so that the order it leaves elements that
//! compare equal in is Go's.
//!
//! `sort.Sort` is pattern-defeating quicksort: not stable, but
//! deterministic. Its only randomness, the pattern breaker's xorshift, is
//! seeded with the slice length. Given the same input and the same `less`,
//! this port performs the same comparisons and swaps in the same order as
//! Go and leaves the same permutation, including the quirks of Go's code
//! (`partialInsertionSort` shifts down to index 1, not to the start of the
//! sub-slice; `pdqsort` looks at the element before the sub-slice).
//!
//! [`crate::txsort`] is built on it. It is public for other ports of Go
//! code that sorts with `sort.Sort` and whose ties are observable (dcrwallet
//! sorts `listunspent`'s credits this way, spec 18 Q-R6): Rust's own sorts
//! order equal elements differently.
//!
//! `less` must be a strict weak order, as Go requires of `Less`; with any
//! other function the result is some permutation of the input (never a
//! panic or an out-of-bounds access), Go's own for the same function.

/// The slice and its ordering, as Go's `sort.Interface`.
struct Data<'a, T, F> {
    s: &'a mut [T],
    less: F,
}

impl<T, F: FnMut(&T, &T) -> bool> Data<'_, T, F> {
    fn less(&mut self, i: usize, j: usize) -> bool {
        (self.less)(&self.s[i], &self.s[j])
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.s.swap(i, j);
    }
}

/// Go's `bits.Len(uint(n))`: the number of bits needed to represent `n`.
fn bits_len(n: usize) -> usize {
    // At most usize::BITS, which fits any usize.
    usize::try_from(usize::BITS.saturating_sub(n.leading_zeros())).unwrap_or(usize::MAX)
}

/// Go's `sort.Sort(data)` over a slice, with `less` as `data.Less` and
/// `data.Swap` swapping two elements: the same calls to `less`, in the same
/// order, and the same final arrangement as Go.
pub fn sort_by<T>(s: &mut [T], less: impl FnMut(&T, &T) -> bool) {
    let n = s.len();
    if n <= 1 {
        return;
    }
    let limit = bits_len(n);
    pdqsort(&mut Data { s, less }, 0, n, limit);
}

/// Go's `sort.IsSorted(data)`: no element is less than the one before it,
/// checked from the end, as Go does.
pub fn is_sorted_by<T>(s: &[T], mut less: impl FnMut(&T, &T) -> bool) -> bool {
    s.windows(2).rev().all(|w| !less(&w[1], &w[0]))
}

/// Go's `sortedHint`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}

/// `insertionSort`: sorts `s[a..b]`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a < i < b <= len and j > a >= 0 under the loop conditions, as in Go"
)]
fn insertion_sort<T, F: FnMut(&T, &T) -> bool>(d: &mut Data<'_, T, F>, a: usize, b: usize) {
    for i in a + 1..b {
        let mut j = i;
        while j > a && d.less(j, j - 1) {
            d.swap(j, j - 1);
            j -= 1;
        }
    }
}

/// `siftDown`: the heap property on `s[lo..hi]`, rooted at offset `first`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "indices stay below the sub-slice length, which is below isize::MAX, as in Go"
)]
fn sift_down<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    lo: usize,
    hi: usize,
    first: usize,
) {
    let mut root = lo;
    loop {
        let mut child = 2 * root + 1;
        if child >= hi {
            break;
        }
        if child + 1 < hi && d.less(first + child, first + child + 1) {
            child += 1;
        }
        if !d.less(first + root, first + child) {
            return;
        }
        d.swap(first + root, first + child);
        root = child;
    }
}

/// `heapSort`: sorts `s[a..b]`, which holds more than 12 elements.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "b > a + 12, so hi - 1 >= 12; the loops count down to 0 inclusive, as in Go"
)]
fn heap_sort<T, F: FnMut(&T, &T) -> bool>(d: &mut Data<'_, T, F>, a: usize, b: usize) {
    #[cfg(test)]
    tests::HEAP_SORTS.with(|c| c.set(c.get() + 1));
    let first = a;
    let lo = 0;
    let hi = b - a;
    for i in (0..=(hi - 1) / 2).rev() {
        sift_down(d, i, hi, first);
    }
    for i in (0..hi).rev() {
        d.swap(first, first + i);
        sift_down(d, lo, i, first);
    }
}

/// `pdqsort`: sorts `s[a..b]`; `limit` bad pivots are allowed before
/// falling back to heapsort.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a <= mid < b <= len and limit > 0 where decremented, as in Go"
)]
fn pdqsort<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    mut a: usize,
    mut b: usize,
    mut limit: usize,
) {
    const MAX_INSERTION: usize = 12;
    let mut was_balanced = true;
    let mut was_partitioned = true;
    loop {
        let length = b - a;
        if length <= MAX_INSERTION {
            insertion_sort(d, a, b);
            return;
        }
        if limit == 0 {
            heap_sort(d, a, b);
            return;
        }
        if !was_balanced {
            break_patterns(d, a, b);
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(d, a, b);
        if hint == Hint::Decreasing {
            reverse_range(d, a, b);
            pivot = (b - 1) - (pivot - a);
            hint = Hint::Increasing;
        }
        if was_balanced
            && was_partitioned
            && hint == Hint::Increasing
            && partial_insertion_sort(d, a, b)
        {
            return;
        }
        if a > 0 && !d.less(a - 1, pivot) {
            a = partition_equal(d, a, b, pivot);
            continue;
        }
        let (mid, already_partitioned) = partition(d, a, b, pivot);
        was_partitioned = already_partitioned;
        let (left_len, right_len) = (mid - a, b - mid);
        let balance_threshold = length / 8;
        if left_len < right_len {
            was_balanced = left_len >= balance_threshold;
            pdqsort(d, a, mid, limit);
            a = mid + 1;
        } else {
            was_balanced = right_len >= balance_threshold;
            pdqsort(d, mid + 1, b, limit);
            b = mid;
        }
    }
}

/// `partition`: one quicksort partition of `s[a..b]` around `s[pivot]`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a < i and a <= j < b throughout: j only decreases while i <= j, as in Go"
)]
fn partition<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
    pivot: usize,
) -> (usize, bool) {
    d.swap(a, pivot);
    let (mut i, mut j) = (a + 1, b - 1);
    while i <= j && d.less(i, a) {
        i += 1;
    }
    while i <= j && !d.less(j, a) {
        j -= 1;
    }
    if i > j {
        d.swap(j, a);
        return (j, true);
    }
    d.swap(i, j);
    i += 1;
    j -= 1;
    loop {
        while i <= j && d.less(i, a) {
            i += 1;
        }
        while i <= j && !d.less(j, a) {
            j -= 1;
        }
        if i > j {
            break;
        }
        d.swap(i, j);
        i += 1;
        j -= 1;
    }
    d.swap(j, a);
    (j, false)
}

/// `partitionEqual`: `s[a..b]` into elements equal to `s[pivot]` followed
/// by greater ones; returns the start of the greater ones.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a < i and a <= j < b throughout: j only decreases while i <= j, as in Go"
)]
fn partition_equal<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
    pivot: usize,
) -> usize {
    d.swap(a, pivot);
    let (mut i, mut j) = (a + 1, b - 1);
    loop {
        while i <= j && !d.less(a, i) {
            i += 1;
        }
        while i <= j && d.less(a, j) {
            j -= 1;
        }
        if i > j {
            break;
        }
        d.swap(i, j);
        i += 1;
        j -= 1;
    }
    i
}

/// `partialInsertionSort`: shifts at most five out-of-order pairs; true if
/// `s[a..b]` ends up sorted.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a < i <= b, and the shifting loops keep 1 <= j < b, as in Go"
)]
fn partial_insertion_sort<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
) -> bool {
    const MAX_STEPS: usize = 5;
    const SHORTEST_SHIFTING: usize = 50;
    let mut i = a + 1;
    for _ in 0..MAX_STEPS {
        while i < b && !d.less(i, i - 1) {
            i += 1;
        }
        if i == b {
            return true;
        }
        if b - a < SHORTEST_SHIFTING {
            return false;
        }
        d.swap(i, i - 1);
        // Shift the smaller one to the left. Go's bound is j >= 1, not
        // j > a, and this keeps it.
        if i - a >= 2 {
            let mut j = i - 1;
            while j >= 1 {
                if !d.less(j, j - 1) {
                    break;
                }
                d.swap(j, j - 1);
                j -= 1;
            }
        }
        // Shift the greater one to the right.
        if b - i >= 2 {
            let mut j = i + 1;
            while j < b {
                if !d.less(j, j - 1) {
                    break;
                }
                d.swap(j, j - 1);
                j += 1;
            }
        }
    }
    false
}

/// `breakPatterns`: three swaps driven by a xorshift seeded with the
/// length.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "length >= 8 so idx stays in a..b; other < 2 * length before the one subtraction, \
              as in Go"
)]
fn break_patterns<T, F: FnMut(&T, &T) -> bool>(d: &mut Data<'_, T, F>, a: usize, b: usize) {
    let length = b - a;
    if length >= 8 {
        // xorshift(length) and nextPowerOfTwo(length).
        let mut random = length as u64;
        let modulus = 1usize.checked_shl(u32::try_from(bits_len(length)).unwrap_or(u32::MAX));
        let mask = modulus.map_or(usize::MAX, |m| m - 1);
        let start = a + (length / 4) * 2 - 1;
        for idx in start..=start + 2 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            // Go's uint(random.Next()): the platform's word.
            let mut other = (random as usize) & mask;
            if other >= length {
                other -= length;
            }
            d.swap(idx, a + other);
        }
    }
}

/// `choosePivot`: a static pivot below 8 elements, the median of three
/// below 50, Tukey's ninther from 50; the hint says whether the samples
/// looked increasing or decreasing.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "l >= 8 where medians are taken, so i - 1 >= a and k + 1 < b, as in Go"
)]
fn choose_pivot<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
) -> (usize, Hint) {
    const SHORTEST_NINTHER: usize = 50;
    const MAX_SWAPS: usize = 4 * 3;
    let l = b - a;
    let mut swaps = 0usize;
    let mut i = a + l / 4;
    let mut j = a + l / 4 * 2;
    let mut k = a + l / 4 * 3;
    if l >= 8 {
        if l >= SHORTEST_NINTHER {
            i = median(d, i - 1, i, i + 1, &mut swaps);
            j = median(d, j - 1, j, j + 1, &mut swaps);
            k = median(d, k - 1, k, k + 1, &mut swaps);
        }
        j = median(d, i, j, k, &mut swaps);
    }
    match swaps {
        0 => (j, Hint::Increasing),
        MAX_SWAPS => (j, Hint::Decreasing),
        _ => (j, Hint::Unknown),
    }
}

/// `order2`.
fn order2<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
    swaps: &mut usize,
) -> (usize, usize) {
    if d.less(b, a) {
        *swaps = swaps.saturating_add(1);
        (b, a)
    } else {
        (a, b)
    }
}

/// `median`: the index of the median of `s[a]`, `s[b]`, `s[c]`.
fn median<T, F: FnMut(&T, &T) -> bool>(
    d: &mut Data<'_, T, F>,
    a: usize,
    b: usize,
    c: usize,
    swaps: &mut usize,
) -> usize {
    let (a, b) = order2(d, a, b, swaps);
    let (b, _c) = order2(d, b, c, swaps);
    let (_a, b) = order2(d, a, b, swaps);
    b
}

/// `reverseRange`: reverses `s[a..b]`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "a <= i < j < b while the loop runs, as in Go"
)]
fn reverse_range<T, F: FnMut(&T, &T) -> bool>(d: &mut Data<'_, T, F>, a: usize, b: usize) {
    let (mut i, mut j) = (a, b - 1);
    while i < j {
        d.swap(i, j);
        i += 1;
        j -= 1;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: counters and index arithmetic on small, bounded values"
    )]

    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// How many times this thread fell back to heapsort.
        pub(super) static HEAP_SORTS: Cell<usize> = const { Cell::new(0) };
    }

    /// The adversary of Go's own `TestAdversary` (`sort/sort_test.go`),
    /// McIlroy's, whose pivot guess is a position, run against this port;
    /// returns the input it built (the integration tests build the same
    /// inputs and compare Go's order on them). Its first guess is position
    /// n/4 rather than Go's 0, so that the first pivot choice does not look
    /// increasing: from Go's start, the partial insertion sort fixes the
    /// order and the adversary never forces a bad pivot (Go's test only
    /// bounds the comparison count; its comment about degenerating to
    /// heapsort predates pdqsort).
    fn adversary(n: usize) -> Vec<u32> {
        let gas = n.saturating_sub(1) as u32;
        let cells: Vec<(Cell<u32>, u32)> = (0..n).map(|i| (Cell::new(gas), i as u32)).collect();
        let mut s = cells;
        let base = s.as_ptr() as usize;
        let size = std::mem::size_of::<(Cell<u32>, u32)>();
        let pos = |e: &(Cell<u32>, u32)| (e as *const (Cell<u32>, u32) as usize - base) / size;
        let state = Cell::new((0u32, n / 4));
        sort_by(&mut s, |x, y| {
            let (mut nsolid, mut candidate) = state.get();
            if x.0.get() == gas && y.0.get() == gas {
                let freeze = if pos(x) == candidate { x } else { y };
                freeze.0.set(nsolid);
                nsolid += 1;
            }
            if x.0.get() == gas {
                candidate = pos(x);
            } else if y.0.get() == gas {
                candidate = pos(y);
            }
            state.set((nsolid, candidate));
            x.0.get() < y.0.get()
        });
        let mut keys = vec![0u32; n];
        for (v, orig) in &s {
            keys[*orig as usize] = v.get();
        }
        keys
    }

    #[test]
    fn the_adversary_reaches_the_heapsort_fallback() {
        for n in [100usize, 257, 1000] {
            let keys = adversary(n);
            let before = HEAP_SORTS.with(Cell::get);
            let mut v = keys.clone();
            sort_by(&mut v, |x, y| x < y);
            assert!(HEAP_SORTS.with(Cell::get) > before, "n = {n}");
            let mut want = keys;
            want.sort_unstable();
            assert_eq!(v, want);
        }
    }

    #[test]
    fn sorts_like_a_sort() {
        let mut rng = purse_testutil::SplitMix64::from_env("dcrutil-gosort");
        for n in 0..400usize {
            let mut v: Vec<u64> = (0..n).map(|_| rng.below(1 + n as u64 / 3)).collect();
            if n % 5 == 0 {
                v.sort_unstable();
            }
            if n % 7 == 0 {
                v.reverse();
            }
            let mut want = v.clone();
            want.sort_unstable();
            sort_by(&mut v, |x, y| x < y);
            assert_eq!(v, want);
            assert!(is_sorted_by(&v, |x, y| x < y));
        }
    }
}
