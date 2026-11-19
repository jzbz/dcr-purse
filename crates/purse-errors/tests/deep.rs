// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Chains a million levels deep are built, rendered, matched and dropped on
//! a thread with a 256 KiB stack, which any recursion per level would
//! overflow (aborting the process). Go's growable stacks handle such chains,
//! so the port must too.

use purse_errors::{AnyError, Error, Kind, Op, e, join, new};

const DEPTH: usize = 1_000_000;

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("deep-chain thread panicked");
}

#[test]
fn deep_error_chain() {
    on_small_stack(|| {
        let bottom = new("bottom");
        let mut err = Error::from_parts(Op::new("leaf"), Kind::Seed, Some(bottom.clone()));
        for i in 0..DEPTH {
            let op = if i.is_multiple_of(2) {
                Op::new("a")
            } else {
                Op::new("b")
            };
            err = Error::from_parts(op, Kind::Other, Some(err.into()));
        }
        let text = err.render(":: ");
        assert!(text.starts_with("b:: a:: b:: "));
        assert!(text.ends_with("a:: leaf: invalid seed: bottom"));
        assert_eq!(err.as_kind(), Some(Kind::Seed));

        let any = AnyError::from(err);
        assert!(any.is_kind(Kind::Seed));
        assert!(any.is(&bottom));
        assert!(!any.is(&new("bottom")));
        assert!(any.matches(&Error::from_parts(
            Op::new("leaf"),
            Kind::Other,
            Some(new("bottom")),
        )));
        assert!(!any.matches(&Error::from_parts(Op::new("c"), Kind::Other, None)));
        assert!(any.find_foreign::<std::fmt::Error>().is_none());
        assert_eq!(
            any.as_error().map(|e| e.op.as_str().to_owned()).as_deref(),
            Some("b")
        );
        drop(any);
    });
}

#[test]
fn deep_wrapped_chain() {
    on_small_stack(|| {
        let text = new("bottom");
        let mut w = AnyError::from(e!(Op::new("x"), Kind::Io, text.clone()));
        for _ in 0..DEPTH {
            w = AnyError::wrap("w", [w]);
        }
        assert_eq!(w.render(":: "), "w");
        assert!(w.is(&text));
        assert!(w.is_kind(Kind::Io));
        assert!(!w.is_kind(Kind::Seed));
        assert_eq!(w.as_kind(), Some(Kind::Io));
        assert!(w.matches(&e!(Op::new("x"))));
        drop(w);
    });
}

#[test]
fn deep_nested_join() {
    on_small_stack(|| {
        let mut j = AnyError::from(e!(Op::new("x"), Kind::Io));
        for _ in 0..DEPTH {
            j = join([Some(j), Some(new("s"))]).expect("two errors");
        }
        let text = j.render(":: ");
        assert!(text.starts_with("x: I/O error\ns\ns\n"));
        assert_eq!(text.lines().count(), DEPTH.saturating_add(1));
        assert!(j.is_kind(Kind::Io));
        assert_eq!(j.as_kind(), Some(Kind::Io));
        assert!(!j.is(&new("s")));
        drop(j);
    });
}

#[test]
fn deep_chain_with_shared_parts_drops() {
    on_small_stack(|| {
        let mut err = AnyError::from(e!(Op::new("leaf"), Kind::Io));
        let mut kept = Vec::new();
        for i in 0..DEPTH {
            err = AnyError::from(Error::from_parts(Op::new("a"), Kind::Other, Some(err)));
            if i.is_multiple_of(100_000) {
                kept.push(err.clone());
            }
        }
        drop(err);
        // The shared levels survive and are still whole.
        assert_eq!(kept.last().map(AnyError::as_kind), Some(Some(Kind::Io)));
        drop(kept);
    });
}
