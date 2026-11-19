// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2013, 2014 The btcsuite developers
// Copyright (c) 2015-2021 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrd's `dcrutil/v4@v4.0.3/amount_test.go` (`TestAmountCreation`,
//! `TestAmountUnitConversions`, `TestAmountMulF64`, `TestAmountSorter`) and
//! the amount examples of `example_test.go` (`ExampleAmount`,
//! `ExampleNewAmount`, `ExampleAmount_unitConversions`), with all their
//! vectors.

use purse_dcrutil::{ATOMS_PER_COIN, Amount, AmountUnit, MAX_AMOUNT, new_amount};

#[test]
#[allow(
    clippy::excessive_precision,
    reason = "the upstream vectors' literals, kept as written"
)]
fn amount_creation() {
    struct Case {
        name: &'static str,
        amount: f64,
        valid: bool,
        expected: i64,
    }
    let tests = [
        // Positive tests.
        Case {
            name: "zero",
            amount: 0.0,
            valid: true,
            expected: 0,
        },
        Case {
            name: "max producable",
            amount: 21e6,
            valid: true,
            expected: MAX_AMOUNT,
        },
        Case {
            name: "min producable",
            amount: -21e6,
            valid: true,
            expected: -MAX_AMOUNT,
        },
        Case {
            name: "exceeds max producable",
            amount: 21e6 + 1e-8,
            valid: true,
            expected: MAX_AMOUNT + 1,
        },
        Case {
            name: "exceeds min producable",
            amount: -21e6 - 1e-8,
            valid: true,
            expected: -MAX_AMOUNT - 1,
        },
        Case {
            name: "one hundred",
            amount: 100.0,
            valid: true,
            expected: 100 * ATOMS_PER_COIN,
        },
        Case {
            name: "fraction",
            amount: 0.01234567,
            valid: true,
            expected: 1234567,
        },
        Case {
            name: "rounding up",
            amount: 54.999999999999943157,
            valid: true,
            expected: 55 * ATOMS_PER_COIN,
        },
        Case {
            name: "rounding down",
            amount: 55.000000000000056843,
            valid: true,
            expected: 55 * ATOMS_PER_COIN,
        },
        // Negative tests.
        Case {
            name: "not-a-number",
            amount: f64::NAN,
            valid: false,
            expected: 0,
        },
        Case {
            name: "-infinity",
            amount: f64::NEG_INFINITY,
            valid: false,
            expected: 0,
        },
        Case {
            name: "+infinity",
            amount: f64::INFINITY,
            valid: false,
            expected: 0,
        },
    ];
    for test in tests {
        match (new_amount(test.amount), test.valid) {
            (Ok(a), true) => assert_eq!(a, Amount(test.expected), "{}", test.name),
            (Err(e), false) => assert_eq!(e.to_string(), "invalid coin amount", "{}", test.name),
            (Ok(a), false) => panic!("{}: succeeded with {a} when it should fail", test.name),
            (Err(e), true) => panic!("{}: failed with {e}", test.name),
        }
    }
}

#[test]
fn amount_unit_conversions() {
    struct Case {
        name: &'static str,
        amount: i64,
        unit: AmountUnit,
        converted: f64,
        s: &'static str,
    }
    let tests = [
        Case {
            name: "MDCR",
            amount: MAX_AMOUNT,
            unit: AmountUnit::MEGA_COIN,
            converted: 21.0,
            s: "21 MDCR",
        },
        Case {
            name: "kDCR",
            amount: 44433322211100,
            unit: AmountUnit::KILO_COIN,
            converted: 444.33322211100,
            s: "444.333222111 kDCR",
        },
        Case {
            name: "Coin",
            amount: 44433322211100,
            unit: AmountUnit::COIN,
            converted: 444333.22211100,
            s: "444333.222111 DCR",
        },
        Case {
            name: "mDCR",
            amount: 44433322211100,
            unit: AmountUnit::MILLI_COIN,
            converted: 444333222.11100,
            s: "444333222.111 mDCR",
        },
        Case {
            name: "μDCR",
            amount: 44433322211100,
            unit: AmountUnit::MICRO_COIN,
            converted: 444333222111.00,
            s: "444333222111 μDCR",
        },
        Case {
            name: "atom",
            amount: 44433322211100,
            unit: AmountUnit::ATOM,
            converted: 44433322211100.0,
            s: "44433322211100 Atom",
        },
        Case {
            name: "non-standard unit",
            amount: 44433322211100,
            unit: AmountUnit(-1),
            converted: 4443332.2211100,
            s: "4443332.22111 1e-1 DCR",
        },
    ];
    for test in tests {
        let a = Amount(test.amount);
        assert_eq!(a.to_unit(test.unit), test.converted, "{}", test.name);
        assert_eq!(a.format(test.unit), test.s, "{}", test.name);
        // ToCoin is ToUnit(AmountCoin), String is Format(AmountCoin).
        assert_eq!(
            a.to_unit(AmountUnit::COIN).to_bits(),
            a.to_coin().to_bits(),
            "{}",
            test.name
        );
        assert_eq!(a.format(AmountUnit::COIN), a.to_string(), "{}", test.name);
    }
}

#[test]
fn amount_mul_f64() {
    let tests: [(&str, i64, f64, i64); 15] = [
        ("Multiply 0.1 DCR by 2", 100e5 as i64, 2.0, 200e5 as i64),
        ("Multiply 0.2 DCR by 0.02", 200e5 as i64, 1.02, 204e5 as i64),
        ("Multiply 0.1 DCR by -2", 100e5 as i64, -2.0, -200e5 as i64),
        (
            "Multiply 0.2 DCR by -0.02",
            200e5 as i64,
            -1.02,
            -204e5 as i64,
        ),
        ("Multiply -0.1 DCR by 2", -100e5 as i64, 2.0, -200e5 as i64),
        (
            "Multiply -0.2 DCR by 0.02",
            -200e5 as i64,
            1.02,
            -204e5 as i64,
        ),
        ("Multiply -0.1 DCR by -2", -100e5 as i64, -2.0, 200e5 as i64),
        (
            "Multiply -0.2 DCR by -0.02",
            -200e5 as i64,
            -1.02,
            204e5 as i64,
        ),
        ("Round down", 49, 0.01, 0),
        ("Round up", 50, 0.01, 1),
        ("Multiply by 0.", 1e8 as i64, 0.0, 0),
        ("Multiply 1 by 0.5.", 1, 0.5, 1),
        ("Multiply 100 by 66%.", 100, 0.66, 66),
        ("Multiply 100 by 66.6%.", 100, 0.666, 67),
        ("Multiply 100 by 2/3.", 100, 2.0 / 3.0, 67),
    ];
    for (name, amt, mul, res) in tests {
        assert_eq!(Amount(amt).mul_f64(mul), Amount(res), "{name}");
    }
}

#[test]
fn amount_sorter() {
    // Go sorts with sort.Sort(AmountSorter(...)); Amount's Ord is the same
    // order, and equal amounts are indistinguishable.
    let tests: [(&str, Vec<i64>, Vec<i64>); 4] = [
        ("Sort zero length slice of Amounts", vec![], vec![]),
        ("Sort 1-element slice of Amounts", vec![7], vec![7]),
        ("Sort 2-element slice of Amounts", vec![7, 5], vec![5, 7]),
        (
            "Sort 6-element slice of Amounts",
            vec![0, 9e8 as i64, 4e6 as i64, 4e6 as i64, 3, 9e12 as i64],
            vec![0, 3, 4e6 as i64, 4e6 as i64, 9e8 as i64, 9e12 as i64],
        ),
    ];
    for (name, as_, want) in tests {
        let mut result: Vec<Amount> = as_.into_iter().map(Amount).collect();
        result.sort();
        let want: Vec<Amount> = want.into_iter().map(Amount).collect();
        assert_eq!(result, want, "{name}");
    }
}

#[test]
fn example_amount() {
    let lines = [
        format!("Zero Atom: {}", Amount(0)),
        format!("100,000,000 Atoms: {}", Amount(1e8 as i64)),
        format!("100,000 Atoms: {}", Amount(1e5 as i64)),
    ];
    assert_eq!(
        lines,
        [
            "Zero Atom: 0 DCR",
            "100,000,000 Atoms: 1 DCR",
            "100,000 Atoms: 0.001 DCR"
        ]
    );
}

#[test]
fn example_new_amount() {
    let show = |f: f64| match new_amount(f) {
        Ok(a) => a.to_string(),
        Err(e) => e.to_string(),
    };
    assert_eq!(show(1.0), "1 DCR");
    assert_eq!(show(0.01234567), "0.01234567 DCR");
    assert_eq!(show(0.0), "0 DCR");
    assert_eq!(show(f64::NAN), "invalid coin amount");
}

#[test]
fn example_amount_unit_conversions() {
    let amount = Amount(44433322211100);
    let lines = [
        format!("Atom to kCoin: {}", amount.format(AmountUnit::KILO_COIN)),
        format!("Atom to Coin: {amount}"),
        format!(
            "Atom to MilliCoin: {}",
            amount.format(AmountUnit::MILLI_COIN)
        ),
        format!(
            "Atom to MicroCoin: {}",
            amount.format(AmountUnit::MICRO_COIN)
        ),
        format!("Atom to Atom: {}", amount.format(AmountUnit::ATOM)),
    ];
    assert_eq!(
        lines,
        [
            "Atom to kCoin: 444.333222111 kDCR",
            "Atom to Coin: 444333.222111 DCR",
            "Atom to MilliCoin: 444333222.111 mDCR",
            "Atom to MicroCoin: 444333222111 μDCR",
            "Atom to Atom: 44433322211100 Atom",
        ]
    );
}
