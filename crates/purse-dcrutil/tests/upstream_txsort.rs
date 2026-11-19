// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2016 The btcsuite developers
// Copyright (c) 2017-2021 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrd's `dcrutil/v4@v4.0.3/txsort/txsort_test.go` (`TestSort`)
//! with its vectors and its testdata (copied unchanged to
//! `tests/data/txsort`).

use dcroxide_wire::MsgTx;
use purse_dcrutil::txsort;
use purse_testutil::unhex;

struct Case {
    name: &'static str,
    hex_file: &'static str,
    is_sorted: bool,
    unsorted_hash: &'static str,
    sorted_hash: &'static str,
}

const TESTS: &[Case] = &[
    Case {
        name: "block 100004 tx[4] - already sorted",
        hex_file: "tx100004-4.hex",
        is_sorted: true,
        unsorted_hash: "b0f540d02eb5cf030e14c08b3b0092648a87f4c6b141dcf34723815a1d01a975",
        sorted_hash: "b0f540d02eb5cf030e14c08b3b0092648a87f4c6b141dcf34723815a1d01a975",
    },
    Case {
        name: "block 101790 tx[3] - sorts inputs only, based on tree",
        hex_file: "tx101790-3.hex",
        is_sorted: false,
        unsorted_hash: "832f1df32817e2f1a1c655c658c2c93926a79b4f3ecc0e8a1f9e4447d990c69d",
        sorted_hash: "93d21689d1c25935afb2e3d0c9b0ee16ee4eebde98133248e0a916594e7244a5",
    },
    Case {
        name: "block 150007 tx[23] - sorts inputs only, based on hash",
        hex_file: "tx150007-23.hex",
        is_sorted: false,
        unsorted_hash: "06fce11b0670038e1e7ec5fd5b84ff1e4fc07bc373f48060f21b9ff79e851a26",
        sorted_hash: "48c8586124eb874ce5f9043acc3d5861f214e4bcc2949590cf8f8bd84743dee9",
    },
    Case {
        name: "block 108930 tx[1] - sorts inputs only, based on index",
        hex_file: "tx108930-1.hex",
        is_sorted: false,
        unsorted_hash: "08f620fbd2d7b5eb3d6c2940a859ce12968547f661be0ce24c4f116dce1f1f44",
        sorted_hash: "1071ce9732b58355616f815d08ff573a607f451bc1b722ab0ec5a094e376ea0a",
    },
    Case {
        name: "block 100082 tx[5] - sorts outputs only, based on amount",
        hex_file: "tx100082-5.hex",
        is_sorted: false,
        unsorted_hash: "765bb2f1fa79222ebc381b6e32a2996e73d5b0d01b443d871f04060def245dcd",
        sorted_hash: "dddbea422361700c93899ae99ca8c3a5860efe1861df439de2f820787d8a1622",
    },
    Case {
        // Tx manually modified to make the first output (output 0) have
        // script version 1.
        name: "modified block 150043 tx[14] - sorts outputs only, based on script version",
        hex_file: "tx150043-14m.hex",
        is_sorted: false,
        unsorted_hash: "b416cbe932c3419465de359b1fc1e07833fd2c49ddee36b61257d10046c98df4",
        sorted_hash: "ff198ee44011742f6c37580430bc13c05b2e91ea356ac36edfaaa037e245638d",
    },
    Case {
        name: "block 150043 tx[14] - sorts outputs only, based on output script",
        hex_file: "tx150043-14.hex",
        is_sorted: false,
        unsorted_hash: "aa4c208c85a9576954d11dd566009cdad98b1c65dcd63b4f6ca11c3e8dff6684",
        sorted_hash: "bb5f9b338c0244e51182b10f36b5ca4c6eeaa2eae98a7bfc106d1f108f154525",
    },
    Case {
        name: "block 150626 tx[24] - sorts outputs only, based on amount and output script",
        hex_file: "tx150626-24.hex",
        is_sorted: false,
        unsorted_hash: "60542bf8ff4acd9ddb28fd9a2df33d23dd2b70a9aaca1f3dd5fe737879205e56",
        sorted_hash: "63709777a56ce88b146d90e8341501ac3cf8d7a2e16c56c74cacd2ec3f961f97",
    },
    Case {
        name: "block 150002 tx[7] - sorts both inputs and outputs",
        hex_file: "tx150002-7.hex",
        is_sorted: false,
        unsorted_hash: "267d9700a090710127efa60c5523f56a0f268e6483737800d193109f5d76276b",
        sorted_hash: "0203d3e9c27ee42d670217685fa9ddb53990c4d0381a06de21ca37ec44087234",
    },
];

#[test]
fn sort() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/txsort");
    for test in TESTS {
        let text = std::fs::read_to_string(dir.join(test.hex_file))
            .unwrap_or_else(|e| panic!("{}: read {}: {e}", test.name, test.hex_file));
        let (mut tx, _) = MsgTx::from_bytes(&unhex(text.trim()))
            .unwrap_or_else(|e| panic!("{}: deserialize: {e:?}", test.name));

        assert_eq!(
            txsort::is_sorted(&tx),
            test.is_sorted,
            "{}: IsSorted",
            test.name
        );

        let sorted = txsort::sort(&tx);
        assert_eq!(
            sorted.tx_hash().to_string(),
            test.sorted_hash,
            "{}: Sort",
            test.name
        );

        // The original transaction is not modified.
        assert_eq!(
            tx.tx_hash().to_string(),
            test.unsorted_hash,
            "{}: unsorted",
            test.name
        );

        txsort::in_place_sort(&mut tx);
        assert_eq!(
            tx.tx_hash().to_string(),
            test.sorted_hash,
            "{}: InPlaceSort",
            test.name
        );
    }
}
