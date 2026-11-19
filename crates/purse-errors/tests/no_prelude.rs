// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The exported macros expand to fully qualified paths: they work from
//! another crate with no prelude and nothing imported.

#![no_implicit_prelude]

extern crate purse_errors;

#[test]
fn macros_need_no_prelude() {
    let op = purse_errors::opf!("wallet.Foo({})", 1);
    let op2 = purse_errors::opf!("wallet.Bar",);
    let text = purse_errors::errorf!("bad {}", 2);
    let text2 = purse_errors::errorf!("plain",);
    let e1 = purse_errors::e!(op, purse_errors::Kind::Io, text);
    let e2 = purse_errors::e!(op2, e1,);
    let e3 = purse_errors::with_stack!(purse_errors::Kind::Seed, text2,);
    let e4 = purse_errors::with_stack!(purse_errors::Op::new("x"));
    ::std::assert_eq!(
        e2.render(":: "),
        "wallet.Bar: I/O error:: wallet.Foo(1): bad 2"
    );
    ::std::assert_eq!(e3.render(":: "), "invalid seed: plain");
    ::std::assert!(e3.stack().is_some() && e4.stack().is_some());
}
