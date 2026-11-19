// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2022 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrd's `hdkeychain` v3.1.3 `extendedkey_test.go`, ported with all of its
//! vectors (the benchmarks are omitted). The vectors are upstream's.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: index arithmetic on small, bounded values"
)]

use purse_hdkeychain::{
    Error, ExtendedKey, HARDENED_KEY_START, HdVersions, NetworkParams, PubKeyErrorKind,
    RECOMMENDED_SEED_LEN, SeedError, generate_seed,
};
use purse_testutil::{SplitMix64, hex, unhex};

/// Mock network parameters, as upstream's `mockNetParams`.
struct MockNetParams {
    priv_key_id: [u8; 4],
    pub_key_id: [u8; 4],
}

impl NetworkParams for MockNetParams {
    fn hd_priv_key_version(&self) -> [u8; 4] {
        self.priv_key_id
    }

    fn hd_pub_key_version(&self) -> [u8; 4] {
        self.pub_key_id
    }
}

/// Mock mainnet parameters (they match Decred mainnet).
const MAIN: MockNetParams = MockNetParams {
    priv_key_id: [0x02, 0xfd, 0xa4, 0xe8], // starts with dprv
    pub_key_id: [0x02, 0xfd, 0xa9, 0x26],  // starts with dpub
};

/// Mock testnet parameters (they match Decred testnet version 3).
const TEST: MockNetParams = MockNetParams {
    priv_key_id: [0x04, 0x35, 0x83, 0x97], // starts with tprv
    pub_key_id: [0x04, 0x35, 0x87, 0xd1],  // starts with tpub
};

const HKS: u32 = 0x8000_0000;

// The master seeds for each of the test vectors in [BIP32].
//
// Note that the 3rd seed has been changed to ensure the condition it is
// intended to test applies with the modified hash function used in Decred.
// In particular, it results in hardened child 0 of the master key having a
// child key with leading zeroes.
const VEC1: &str = "000102030405060708090a0b0c0d0e0f";
const VEC2: &str = "fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a29f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542";
const VEC3: &str = "4b381541583be4423346c643850da4b320e46a87ae3d2a4e6da11eba819cd4acba45d239319ac14f863b8d5ab5a0d0c64d2e8a1e7d1457df2e5a3c5100005930";
const VEC4: &str = "3ddd5602285899a946114506157c7997e5444528f3003f6134712147db19b678";

struct Bip32 {
    name: &'static str,
    master: &'static str,
    path: &'static [u32],
    want_pub: &'static str,
    want_priv: &'static str,
    net: MockNetParams,
}

/// TestBIP0032Vectors: the [BIP32] vectors with Decred prefixes.
#[test]
fn bip0032_vectors() {
    let tests = [
        Bip32 {
            name: "test vector 1 chain m",
            master: VEC1,
            path: &[],
            want_pub: "dpubZ9169KDAEUnyoBhjjmT2VaEodr6pUTDoqCEAeqgbfr2JfkB88BbK77jbTYbcYXb2FVz7DKBdW4P618yd51MwF8DjKVopSbS7Lkgi6bowX5w",
            want_priv: "dprv3hCznBesA6jBtmoyVFPfyMSZ1qYZ3WdjdebquvkEfmRfxC9VFEFi2YDaJqHnx7uGe75eGSa3Mn3oHK11hBW7KZUrPxwbCPBmuCi1nwm182s",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m/0H",
            master: VEC1,
            path: &[HKS],
            want_pub: "dpubZCGVaKZBiMo7pMgLaZm1qmchjWenTeVcUdFQkTNsFGFEA6xs4EW8PKiqYqP7HBAitt9Hw16VQkQ1tjsZQSHNWFc6bEK6bLqrbco24FzBTY4",
            want_priv: "dprv3kUQDBztdyjKuwnaL3hfKYpT7W6X2huYH5d61YSWFBebSYwEBHAXJkCpQ7rvMAxPzKqxVCGLvBqWvGxXjAyMJsV1XwKkfnQCM9KctC8k8bk",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1",
            master: VEC1,
            path: &[HKS, 1],
            want_pub: "dpubZEDyZgdnFBMHxqNhfCUwBfAg1UmXHiTmB5jKtzbAZhF8PTzy2PwAicNdkg1CmW6TARxQeUbgC7nAQenJts4YoG3KMiqcjsjgeMvwLc43w6C",
            want_priv: "dprv3nRtCZ5VAoHW4RUwQgRafSNRPUDFrmsgyY71A5eoZceVfuyL9SbZe2rcbwDW2UwpkEniE4urffgbypegscNchPajWzy9QS4cRxF8QYXsZtq",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H",
            master: VEC1,
            path: &[HKS, 1, HKS + 2],
            want_pub: "dpubZGLz7gsJAWzUksvtw3opxx5eeLq5fRaUMDABA3bdUVfnGUk5fiS5Cc3kZGTjWtYr3jrEavQQnAF6jv2WCpZtFX4uFgifXqev6ED1TM9rTCB",
            want_priv: "dprv3pYtkZK168vgrU38gXkUSjHQ2LGpEUzQ9fXrR8fGUR59YviSnm6U82XjQYhpJEUPnVcC9bguJBQU5xVM4VFcDHu9BgScGPA6mQMH4bn5Cth",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H/2",
            master: VEC1,
            path: &[HKS, 1, HKS + 2, 2],
            want_pub: "dpubZHv6Cfp2XRSWHQXZBo1dLmVM421Zdkc4MePkyBXCLFttVkCmwZkxth4ZV9PzkFP3DtD5xcVq2CPSYpJMWMaoxu1ixz4GNZFVcE2xnHP6chJ",
            want_priv: "dprv3r7zqYFjT3NiNzdnwGxGpYh6S1TJCp1zA6mSEGaqLBJFnCB94cRMp7YYLR49aTZHZ7ya1CXwQJ6rodKeU9NgQTxkPSK7pzgZRgjYkQ7rgJh",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H/2/1000000000",
            master: VEC1,
            path: &[HKS, 1, HKS + 2, 2, 1000000000],
            want_pub: "dpubZL6d9amjfRy1zeoZM2zHDU7uoMvwPqtxHRQAiJjeEtQQWjP3retQV1qKJyzUd6ZJNgbJGXjtc5pdoBcTTYTLoxQzvV9JJCzCjB2eCWpRf8T",
            want_priv: "dprv3tJXnTDSb3uE6Euo6WvvhFKfBMNfxuJt5smqyPoHEoomoBMQyhYoQSKJAHWtWxmuqdUVb8q9J2NaTkF6rYm6XDrSotkJ55bM21fffa7VV97",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m",
            master: VEC2,
            path: &[],
            want_pub: "dpubZ9169KDAEUnynoD4qvXJwmxZt3FFA5UdWn1twnRReE9AxjCKJLNFY1uBoegbFmwzA4Du7yqnu8tLivhrCCH6P3DgBS1HH5vmf8MpNXvvYT9",
            want_priv: "dprv3hCznBesA6jBtPKJbQTxRZAKG2gyj8tZKEPaCsV4e9YYFBAgRP2eTSPAeu4r8dTMt9q51j2Vdt5zNqj7jbtovvocrP1qLj6WUTLF9xYQt4y",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m/0",
            master: VEC2,
            path: &[0],
            want_pub: "dpubZBA4RCkCybJFaNbqPuBiyfXY1rvmG1XTdCy1AY1U96dxkFqWc2i5KREMh7NYPpy7ZPMhdpFMAesex3JdFDfX4J5FEW3HjSacqEYPfwb9Cj7",
            want_priv: "dprv3jMy45BuuDETfxi59P8NTSjHPrNVq4wPRfLgRd57923L2hosj5NUEqiLYQ4i7fJtUpiXZLr2wUeToJY2Tm5sCpAJdajEHDmieVJiPQNXwu9",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m/0/2147483647H",
            master: VEC2,
            path: &[0, HKS + 2147483647],
            want_pub: "dpubZDUNkZEcCRCZEizDGL9sAQbZRKSnaxQLeqN9zpueeqCyq2VY7NUGMXASacsK96S8XzNjq3YgFgwLtj8MJBToW6To9U5zxuazEyh89bjR1xA",
            want_priv: "dprv3mgHPRgK838mLK6T1p6WeBoJoJtXA1pGTHjqFuyHekcM7UTuER8fGweRRsoLqSuHa98uskVPnJnfWZEBUC1AVmXnSCPDvUFKydXNnnPHTuQ",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m/0/2147483647H/1",
            master: VEC2,
            path: &[0, HKS + 2147483647, 1],
            want_pub: "dpubZF3wJh7SfggGg74QZW3EE9ei8uQSJEFgd62uyuK5iMgQzUNjpSnprgTpYz3d6Q3fXXtEEXQqpzWcP4LUVuXFsgA8JKt1Hot5kyUk4pPRhDz",
            want_priv: "dprv3oFqwZZ9bJcUmhAeJyyshvrTWtrAsHfcRYQbEzNiiH5nGvM6wVTDn6woQEz92b2EHTYZBtLi82jKEnxSouA3cVaW8YWBsw5c3f4mwAhA3d2",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m/0/2147483647H/1/2147483646H",
            master: VEC2,
            path: &[0, HKS + 2147483647, 1, HKS + 2147483646],
            want_pub: "dpubZH38NEg1CW19dGZs8NdaT4hDkz7wXPstio1mGpHSAXHpSGW3UnTrn25ERT1Mp8ae5GMoQHMbgQiPrChMXQMdx3UqS8YqFkT1pqait8fY92u",
            want_priv: "dprv3qF3177i87wMirg6sraDvqty8yZg6THpXFPSXuM5AShBiiUQbq8FhSZDGkYmBNR3RKfBrxzkKDBpsRFJfTnQfLsvpPPqRnakat6hHQA43X9",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 2 chain m/0/2147483647H/1/2147483646H/2",
            master: VEC2,
            path: &[0, HKS + 2147483647, 1, HKS + 2147483646, 2],
            want_pub: "dpubZJoBFoQJ35zvEBgsfhJBssnAp8TY5gvruzQFLmyxcqRb7enVtGfSkLo2CkAZJMpa6T2fx6fUtvTgXtUvSVgAZ56bEwGxQsToeZfFV8VadE1",
            want_priv: "dprv3s15tfqzxhw8Kmo7RBEqMeyvC7uGekLniSmvbs3bckpxQ6ks1KKqfmH144Jgh3PLxkyZRcS367kp7DrtUmnG16NpnsoNhxSXRgKbJJ7MUQR",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 3 chain m",
            master: VEC3,
            path: &[],
            want_pub: "dpubZ9169KDAEUnynpjjLH5vXBfmaBym5MB4HsyZ4ZbsXtJh8DyFVRP8ucKKgNTNSyUZZC93dnJX5eSZxeURPmzHpnBDBieisUbaMJteHNHJpPK",
            want_priv: "dprv3hCznBesA6jBtQqy5m2ZzxsWxBRVeQaz6LMEKefWXoi4QfwccU3Xq2oJXgCZvgKQtRXKdgvkyt35PMhyavDyy95dTc1xjbMGd7vJ61LQan8",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 3 chain m/0H",
            master: VEC3,
            path: &[HKS],
            want_pub: "dpubZB4aNJNxjimmDnmC3ZCo7zmSW9ue8i6fRMwpwyHzi15X7j2KHXWSDiPr4ymhbQWg6D3HAjSnqSG3PHDaLVAkUCuRU5ohX9y9z6KL768vYm6",
            want_priv: "dprv3jGV1ApffLhyKNsRo39SbmyBt9MNhmWbDpKWD4MdhvUtQAzgQaAq98spvFaUmYyNJSPGLJ9JzgVDNN9bQP62tUpLwVigWZMjXmojbERDmhk",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 3 chain m/0H/0H",
            master: VEC3,
            path: &[HKS, HKS],
            want_pub: "dpubZESpio8DE1w5E9h5cBwuER5dmfcqZzh19UTstEDJMTfTAbWPJQ8mCFKeqyVgTFL5n4VmrUgbidcH1FmJaPLEWcdyizsdHocfyJTn8Zw8tkB",
            want_priv: "dprv3nejMfZv9dsHKjoKMftYiCHP9f4a946vwvqZ9KGwMP4pT3UkRSoA7fodhGCqPVBqgC2wijajQYW9ZQKNMdZ7Ac2L5i7xqHcq4tPzcxzAxhf",
            net: MAIN,
        },
        Bip32 {
            name: "test vector 1 chain m - testnet",
            master: VEC1,
            path: &[],
            want_pub: "tpubVhnMyQmZAhoosedBTX7oacwyCNc5qtdEMoNHudUCW1R6WZTvqCZQoNJHSn4H11puwdk4qyDv2ET637EDap4r8HH3odjBC5nEjmnPcsDfLwm",
            want_priv: "tprvZUo1ZuEfLLFWfAYiMVaoDV1EeLmbSRuNzaSh7F4awft7dm8nHfFAFZyobWQyV8Qr26r8M2CmNw6nEb35HaECWFGy1vzx2ZGdyfBeaaHudoi",
            net: TEST,
        },
        Bip32 {
            name: "test vector 1 chain m/0H - testnet",
            master: VEC1,
            path: &[HKS],
            want_pub: "tpubVm3mQR7aeaowtpbnJKRnvpKsJ3A3q5u31EPY1FAU5Re1zvFfmFUE5aHXY4qmjfQcb1uFZf8mvvU1vi89vEzHPQfR5NETLqByzdthaYfQGja",
            want_priv: "tprvZY4QzuagpDFegLXKCHtnZgP8k1KZRdBBe1TwCrkrX67387vXDi9yXmy3gnz6tBTyNKcSZmu4wLtVsYzbKZhSVZH89uP7VxV4RboFfozTBMQ",
            net: TEST,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1 - testnet",
            master: VEC1,
            path: &[HKS, 1],
            want_pub: "tpubVo1FPnCBBQN83JJ9Nx9iGhsqa1Gnf9sBhgsT9nNmPrdvEHHmjQuGQrwKjuTsDzLLrZiNH8dxiHrASd2uQfmTgR6dqrkyVN5p3P2crvfgpEQ",
            want_priv: "tprvZa1tzGfHM2opppDgGvchuZw71ySJFh9LLTwrMPy9qX6wMUxdBsb1s4cqtcLgZVTQ8EZCJeYagpjaw6gkU16ht5Nr8y2WEc9UWQimC4Y6MFs",
            net: TEST,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H - testnet",
            master: VEC1,
            path: &[HKS, 1, HKS + 2],
            want_pub: "tpubVq8FwnRh6k1JqLrLeoUc3znpCsLM2rytspJJQqPEJf4a7J2tNjQAtrcSYVvPyNnjjscCDaShJLK6mtH6idGo8g8Djpe2HL13VFJgygvRmc9",
            want_priv: "tprvZc8uYGtoGNT1crmsYmwbgrr5eqVrdQG3WbNhcSyckKXbEVhjqC5vM4HxhDpzqEyyAVNgEBKdKLTT3EXQesyhPyhFoeVy6ZExqrpurCCRvrF",
            net: TEST,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H/2 - testnet",
            master: VEC1,
            path: &[HKS, 1, HKS + 2, 2],
            want_pub: "tpubVrhN2mNRTeTLMsSzuYgQRpCWcYWq1C1UtFXtDyJoARHgLZVaeaj4awdFUNrfCjcvv1y3bGY7YNTSanYx2AHir453T7yd83bd1F8eJgtdq3F",
            want_priv: "tprvZdi1dFqXdGu39PNXoX9Q4gFn4WgLbjHdX2cHRauBc5khTmAS73Qp39Jmd6BL7U4rw7k45nAfRT9qkuMi4Y6mb9ks1QNUfAmRW9DBY5g5ELT",
            net: TEST,
        },
        Bip32 {
            name: "test vector 1 chain m/0H/1/2H/2/1000000000 - testnet",
            master: VEC1,
            path: &[HKS, 1, HKS + 2, 2, 1000000000],
            want_pub: "tpubVtstygL8beyr57j14nf4JWq5MtSCmHJNp2YHy6XF53oCMYfrZfrWBGQ1JDT95aoC4pMFuBnB8Ftdq9s3yMAFh7UKQd4f3hLL8C8KipwSek6",
            want_priv: "tprvZftYaAoEmHRYrdeXxm83wNtLorbiMpaXSochAi7dWiGDUkLi28YFdU5XSxe53yHVDdEyfiTsKBRZR2HASwVBhueZRroeuFgD6U9JTC1mUyU",
            net: TEST,
        },
    ];

    for (i, test) in tests.iter().enumerate() {
        let seed = unhex(test.master);
        let mut ext_key = ExtendedKey::new_master(&seed, &test.net)
            .unwrap_or_else(|e| panic!("NewMaster #{i} ({}): {e}", test.name));
        for &child_num in test.path {
            ext_key = ext_key
                .child(child_num)
                .unwrap_or_else(|e| panic!("#{i} ({}): {e}", test.name));
        }

        assert_eq!(
            *ext_key.string(),
            test.want_priv,
            "Serialize #{i} ({})",
            test.name
        );

        let pub_key = ext_key.neuter();
        // Neutering a second time has no effect.
        assert_eq!(*pub_key.neuter().string(), *pub_key.string(), "#{i}");
        assert_eq!(
            *pub_key.string(),
            test.want_pub,
            "Neuter #{i} ({})",
            test.name
        );
    }
}

struct Derive {
    name: &'static str,
    master: &'static str,
    path: &'static [u32],
    want: &'static str,
}

const VEC1_PRIV: &str = "dprv3hCznBesA6jBucms1ZhyGeFfvJfBSwfs7ZFrxS8tdYzbjDZe2UwSaL7EbYo1qa88DmtyyG5cL9tdGxHkD89JmeZTbz5sVYU4Dgtijiio4Sc";
const VEC2_PRIV: &str = "dprv3hCznBesA6jBtPKJbQTxRZAKG2gyj8tZKEPaCsV4e9YYFBAgRP2eTSPAeu4r8dTMt9q51j2Vdt5zNqj7jbtovvocrP1qLj6WUTLF9xYQt4y";

/// TestPrivateDerivation: private keys derived from private keys.
#[test]
fn private_derivation() {
    let tests = [
        Derive {
            name: "test vector 1 chain m",
            master: VEC1_PRIV,
            path: &[],
            want: "dprv3hCznBesA6jBucms1ZhyGeFfvJfBSwfs7ZFrxS8tdYzbjDZe2UwSaL7EbYo1qa88DmtyyG5cL9tdGxHkD89JmeZTbz5sVYU4Dgtijiio4Sc",
        },
        Derive {
            name: "test vector 1 chain m/0",
            master: VEC1_PRIV,
            path: &[0],
            want: "dprv3jFfEhxvVxy6NJWopujhfg7syQL71xCRgNoGUpQTtjTpCwzigwtCwssQGbRQsby7PBs1Yp8Wu7isu396qeNof13EZuxbCTJVF1xkoFAQHWj",
        },
        Derive {
            name: "test vector 1 chain m/0/1",
            master: VEC1_PRIV,
            path: &[0, 1],
            want: "dprv3mWLns1v1fdLhxStaJHh3BqxmTi14RHHeWdNU6oU8sSkTDmAr54yK6La2APy3rAZr9ZJAdm5asTJaqBZ3vBYVSPHqyL8kbcCp5jgqfxBs4x",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2",
            master: VEC1_PRIV,
            path: &[0, 1, 2],
            want: "dprv3oDxSziXR1rQVWwWWBRKgCQU3vN6dnR3ekzHzvZRdgfVvrYSE35saJh8UdfSxCgtMn7pnbeMXWbyBbwxoncC9LMrnuH1AoJSB259c4XgmnN",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2/2",
            master: VEC1_PRIV,
            path: &[0, 1, 2, 2],
            want: "dprv3rYHNih25i8MqeRjhFq8mLnK4a3J63a9zkYTCFJd8kaJuNc5aDAAuG1XopkU7h93HvfbNvQaWdQLtwFmUEbDN3GCZ2Mxw6tq5ZSh8d1Chyw",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2/2/1000000000",
            master: VEC1_PRIV,
            path: &[0, 1, 2, 2, 1000000000],
            want: "dprv3tKkzgLFKaX2VcQTir9JeNHWxikiKokSJtdyj7sYoiDkU3np2rc3DGYPRVmDhb2FFaAk98fnqRotQYTVCRaoyAZiHaoyNoPCFeYA9pEshBT",
        },
        Derive {
            name: "test vector 2 chain m",
            master: VEC2_PRIV,
            path: &[],
            want: "dprv3hCznBesA6jBtPKJbQTxRZAKG2gyj8tZKEPaCsV4e9YYFBAgRP2eTSPAeu4r8dTMt9q51j2Vdt5zNqj7jbtovvocrP1qLj6WUTLF9xYQt4y",
        },
        Derive {
            name: "test vector 2 chain m/0",
            master: VEC2_PRIV,
            path: &[0],
            want: "dprv3jMy45BuuDETfxi59P8NTSjHPrNVq4wPRfLgRd57923L2hosj5NUEqiLYQ4i7fJtUpiXZLr2wUeToJY2Tm5sCpAJdajEHDmieVJiPQNXwu9",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647",
            master: VEC2_PRIV,
            path: &[0, 2147483647],
            want: "dprv3mgHPRgAnNboAb9edL4RPscKYrNLG77BhPvFe3eiTGPSiigDeXct3WeiZ2QqRrm9TiseBuYWGEG79xkBzazpBGfym1vRXjcEo5KUi4rbhZ1",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1",
            master: VEC2_PRIV,
            path: &[0, 2147483647, 1],
            want: "dprv3onqUUAjN1xQUAW1BzwiBa5wisDCE8hX8YcAEgVe1DPvbgHVLauDt2NsZPQX6tJs6ozcQSU9GdsffhueTbxTxFMPEMPxM2iHiooMz2oQHWS",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1/2147483646",
            master: VEC2_PRIV,
            path: &[0, 2147483647, 1, 2147483646],
            want: "dprv3r8NTJgGzAjY5cU7sLo5rhT8o2wdco2iqjks6nJoiDACTucrPMcsciccv9skwGMX69uRa8EaZofskV7YyzBDbVi6v4RXbJ4DyeZ6JpUgdUi",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1/2147483646/2",
            master: VEC2_PRIV,
            path: &[0, 2147483647, 1, 2147483646, 2],
            want: "dprv3sp4xvFP9mL9UUEddSZUxrtNnhe5UcHs5wrpxdZVEFCXoT4EpYeHZJjCDhvVEQFK2KfSHXFmew6MeBuvtrJfQv1BnkiSV7xxUji66uvWasp",
        },
        Derive {
            name: "Derived privkey with zero high byte m/0",
            master: "dprv3jFfEhxvVxy6NJWopujhfg7syQL71xCRgNoGUpQTtjTpCwzigwtCwssQGbRQsby7PBs1Yp8Wu7isu396qeNof13EZuxbCTJVF1xkoFAQHWj",
            path: &[0],
            want: "dprv3mWLns1v1fdLfeu5DKTA6NWQHLF6pFsPSwKCS6q4h4nkjm2DfuH5X2iDnW15jhHTGa3rzxSpvskuXugcbBcUUVWCETKKzjW7ja4V2jL4aw4",
        },
    ];

    for (i, test) in tests.iter().enumerate() {
        let mut ext_key = ExtendedKey::new_key_from_string(test.master, &MAIN)
            .unwrap_or_else(|e| panic!("NewKeyFromString #{i} ({}): {e}", test.name));
        for &child_num in test.path {
            ext_key = ext_key
                .child(child_num)
                .unwrap_or_else(|e| panic!("#{i} ({}): {e}", test.name));
        }
        assert_eq!(*ext_key.string(), test.want, "Child #{i} ({})", test.name);
    }
}

const VEC1_PUB: &str = "dpubZF8BRmciAzYoTjXZ3bbRWLVCwUKtTquact3Tr6ye77Rgmw76VyqMb9TB9KpfrvUYEM5d1Au4fQzE2BbtxRjwzGsqnWHmtQP9UV1kxZaqvb6";
const VEC2_PUB: &str = "dpubZF4LSCdF9YKZfNzTVYhz4RBxsjYXqms8AQnMBHXZ8GUKoRSigG7kQnKiJt5pzk93Q8FxcdVBEkQZruSXduGtWnkwXzGnjbSovQ97dCxqaXc";

/// TestPublicDerivation: public keys derived from public keys.
#[test]
fn public_derivation() {
    let tests = [
        Derive {
            name: "test vector 1 chain m",
            master: VEC1_PUB,
            path: &[],
            want: "dpubZF8BRmciAzYoTjXZ3bbRWLVCwUKtTquact3Tr6ye77Rgmw76VyqMb9TB9KpfrvUYEM5d1Au4fQzE2BbtxRjwzGsqnWHmtQP9UV1kxZaqvb6",
        },
        Derive {
            name: "test vector 1 chain m/0",
            master: VEC1_PUB,
            path: &[0],
            want: "dpubZHm6cmVU9pvfDCe3BY7iESzsEnV6xfi4DfoYvycnWLM9cryzKA84DqJ2CphYq6cfiEXgo9C3YLJA4ou81mavw9NDtNc3bLCWVqJz8Fx8qxB",
        },
        Derive {
            name: "test vector 1 chain m/0/1",
            master: VEC1_PUB,
            path: &[0, 1],
            want: "dpubZKtA6UTDuxeXV2PcYqoe68u7cgDhbTNbA4dUJoaAvfWzuCcRQCyG5S6dbpDZb2p3B5Y2XxLtD94Nemc8QRV4RspmvGwHvE2FZsfE5Pqpeor",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2",
            master: VEC1_PUB,
            path: &[0, 1, 2],
            want: "dpubZMwLXm5dRVEJRvJHU8gNV7RwHeXMRRUnYFD4f6C8uNFfqksD1FCDARTwNPsQB3Pg4LuoKXkZbPnE6woUyedwNYVPvZToT5x4Kt6rs4GKa9c",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2/2",
            master: VEC1_PUB,
            path: &[0, 1, 2, 2],
            want: "dpubZPfASfojwk6MhtAtkM6wPdQBr1ycVjoyqs3N51zR1keK6FcBhjBTtdW3Wn3kDLBZqgLnGozu8Gh3FV8GrFGpu3knmGVoF1Z6yGdqLU1Rz1S",
        },
        Derive {
            name: "test vector 1 chain m/0/1/2/2/1000000000",
            master: VEC1_PUB,
            path: &[0, 1, 2, 2, 1000000000],
            want: "dpubZR5Pf8cbUGikESevygwydenBaTsgcvoYnRSi7tygu23PxmVEG4GeMQj54oHFoPyRdt7Pg4sMad56yprQszbNyZVewaNEhDkn112C3mqB1fd",
        },
        Derive {
            name: "test vector 2 chain m",
            master: VEC2_PUB,
            path: &[],
            want: "dpubZF4LSCdF9YKZfNzTVYhz4RBxsjYXqms8AQnMBHXZ8GUKoRSigG7kQnKiJt5pzk93Q8FxcdVBEkQZruSXduGtWnkwXzGnjbSovQ97dCxqaXc",
        },
        Derive {
            name: "test vector 2 chain m/0",
            master: VEC2_PUB,
            path: &[0],
            want: "dpubZHJs2Z3PtHbbpaXQCi5wBKPhU8tC5ztBKUYBCYNGKk8eZ1EmBs3MhnLJbxHFMAahGnDnZT7qZxC7AXKP8PB6BDNUZgkG77moNMRmXyQ6s6s",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647",
            master: VEC2_PUB,
            path: &[0, 2147483647],
            want: "dpubZJgFEUcAZawGaLZdFEX6FfQBQVgU4bUC5qvDERUTD5dfcB2AQPnJ1dKp1R2DrAzC36BznZG43317s2oBJv3PuaZmA6HqmwMu6vNna4Gfumf",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1",
            master: VEC2_PUB,
            path: &[0, 2147483647, 1],
            want: "dpubZLbgtFNyjt3k2cJtg4a3dD2iXPKFTLgNKP8rLC1p5UE3AyfRHLTcYrZ6brg8eUmGvKRrXZ7A3XyVfwGxvYtjfz8514dUoJPkmSnBmC6qQK6",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1/2147483646",
            master: VEC2_PUB,
            path: &[0, 2147483647, 1, 2147483646],
            want: "dpubZNyWTupEG35S6d4uN93vWXpGxQuxtW9zuThQbnWpWTHwRCzxREqSSc9eDYivRGiZnEkEhPece5ciSoHtW6Khc729f6eAxjPnBgU38U9hgYw",
        },
        Derive {
            name: "test vector 2 chain m/0/2147483647/1/2147483646/2",
            master: VEC2_PUB,
            path: &[0, 2147483647, 1, 2147483646, 2],
            want: "dpubZRuRErXqhdJaZWD1AzXB6d5w2zw7UZ7ALxiS1gHbnQbVEohBzQzsVwGRzq97pmuE7ToA6DGn2QTH4DexxzdnMvkiYUpk8Nh2KEuYUM2RCeU",
        },
    ];

    for (i, test) in tests.iter().enumerate() {
        let mut ext_key = ExtendedKey::new_key_from_string(test.master, &MAIN)
            .unwrap_or_else(|e| panic!("NewKeyFromString #{i} ({}): {e}", test.name));
        for &child_num in test.path {
            ext_key = ext_key
                .child(child_num)
                .unwrap_or_else(|e| panic!("#{i} ({}): {e}", test.name));
        }
        assert_eq!(*ext_key.string(), test.want, "Child #{i} ({})", test.name);
    }
}

/// TestGenenerateSeed: valid and invalid lengths.
#[test]
fn generate_seed_lengths() {
    let tests: [(&str, u8, bool); 7] = [
        // Valid lengths.
        ("16 bytes", 16, true),
        ("17 bytes", 17, true),
        ("20 bytes", 20, true),
        ("32 bytes", 32, true),
        ("64 bytes", 64, true),
        // Invalid lengths.
        ("15 bytes", 15, false),
        ("65 bytes", 65, false),
    ];

    let mut rng = SplitMix64::from_env("hdkeychain-upstream-generate-seed");
    for (i, (name, length, ok)) in tests.into_iter().enumerate() {
        let r = generate_seed(length, |b: &mut [u8]| {
            rng.fill(b);
            Ok::<(), ()>(())
        });
        match r {
            Ok(seed) => {
                assert!(ok, "GenerateSeed #{i} ({name}): want error");
                assert_eq!(
                    seed.len(),
                    usize::from(length),
                    "GenerateSeed #{i} ({name})"
                );
            }
            Err(e) => {
                assert!(!ok, "GenerateSeed #{i} ({name}): unexpected error {e:?}");
                assert_eq!(e, SeedError::InvalidSeedLen, "GenerateSeed #{i} ({name})");
            }
        }
    }
}

/// TestExtendedKeyAPI: accessors of a private and a public key.
#[test]
fn extended_key_api() {
    struct Api {
        name: &'static str,
        ext_key: &'static str,
        is_private: bool,
        parent_fp: u32,
        priv_key: Result<&'static str, Error>,
        pub_key: &'static str,
    }
    let tests = [
        Api {
            name: "test vector 1 master node private",
            ext_key: "dprv3hCznBesA6jBteCheVrorP4Nix6oEWFwtk79FaLy1rPzedNGg9Jhy9y596C9uCPMeeKJMvtEGZRaxPKxGuCFgR4uo2EySsA29GsnXcrsby8",
            is_private: true,
            parent_fp: 0,
            priv_key: Ok("33a63922ea4e6686c9fc31daf136888297537f66c1aabe3363df06af0b8274c7"),
            pub_key: "039f2e1d7b50b8451911c64cf745f9ba16193b319212a64096e5679555449d8f37",
        },
        Api {
            name: "test vector 2 chain m/0/2147483647/1/2147483646/2",
            ext_key: "dpubZRuRErXqhdJaZWD1AzXB6d5w2zw7UZ7ALxiS1gHbnQbVEohBzQzsVwGRzq97pmuE7ToA6DGn2QTH4DexxzdnMvkiYUpk8Nh2KEuYUM2RCeU",
            is_private: false,
            parent_fp: 4220580796,
            priv_key: Err(Error::NotPrivExtKey),
            pub_key: "03dceb0b07698ec3d6ac08ae7297e7f5e63d7fda99d3fce1ded31d36badcdd4d36",
        },
    ];

    for (i, test) in tests.iter().enumerate() {
        let key = ExtendedKey::new_key_from_string(test.ext_key, &MAIN)
            .unwrap_or_else(|e| panic!("NewKeyFromString #{i} ({}): {e}", test.name));
        assert_eq!(
            key.is_private(),
            test.is_private,
            "IsPrivate #{i} ({})",
            test.name
        );
        assert_eq!(
            key.parent_fingerprint(),
            test.parent_fp,
            "ParentFingerprint #{i} ({})",
            test.name
        );
        assert_eq!(*key.string(), test.ext_key, "String #{i} ({})", test.name);
        assert_eq!(
            key.serialized_priv_key().map(hex),
            test.priv_key.clone().map(str::to_owned),
            "SerializedPrivKey #{i} ({})",
            test.name
        );
        assert_eq!(
            hex(key.serialized_pub_key()),
            test.pub_key,
            "SerializedPubKey #{i} ({})",
            test.name
        );
    }
}

/// TestErrors: negative tests.
#[test]
fn errors() {
    // Too few and too many seed bytes.
    assert_eq!(
        ExtendedKey::new_master(&[0u8; 15], &MAIN).err(),
        Some(Error::InvalidSeedLen)
    );
    assert_eq!(
        ExtendedKey::new_master(&[0u8; 65], &MAIN).err(),
        Some(Error::InvalidSeedLen)
    );

    // A new key neutered to a public extended key cannot derive a hardened
    // child.
    let mut rng = SplitMix64::from_env("hdkeychain-upstream-errors");
    let seed = generate_seed(RECOMMENDED_SEED_LEN, |b: &mut [u8]| {
        rng.fill(b);
        Ok::<(), ()>(())
    })
    .expect("seed");
    let ext_key = ExtendedKey::new_master(&seed, &MAIN).expect("master");
    let pub_key = ext_key.neuter();
    assert_eq!(
        pub_key.child(HARDENED_KEY_START).err(),
        Some(Error::DeriveHardFromPublic)
    );

    // NewKeyFromString failures.
    type IsWanted = fn(&Error) -> bool;
    let tests: [(&str, &str, IsWanted); 5] = [
        (
            "key length too long",
            "DNQvB3p9wsKRyaoxXrh2UyBby7RpMtvhe4uJszAE4Mc8qinPwdUkcpqoUZPFAtXf41ukuio6JsUTgKjPnCaAy99fSBHKJgeRmWCrcytLR1GZQqpnR5",
            |e| *e == Error::InvalidKeyLen,
        ),
        ("invalid key length", "dpub1234", |e| {
            *e == Error::InvalidKeyLen
        }),
        (
            "bad checksum",
            "dpubZF6AWaFizAuUcbkZSs8cP8Gxzr6Sg5tLYYM7gEjZMC5GDaSHB4rW4F51zkWyo9U19BnXhc99kkEiPg248bYin8m9b8mGss9nxV6N2QpU8vj",
            |e| *e == Error::BadChecksum,
        ),
        (
            "pubkey not on curve",
            "dpubZ9169KDAEUnyoTzA7pDGtXbxpji5LuUk8johUPVGY2CDsz6S7hahGNL6QkeYrUeAPnaJD1MBmrsUnErXScGZdjL6b2gjCRX1Z1GNhLdVCjv",
            |e| matches!(e, Error::PubKey(p) if p.kind() == PubKeyErrorKind::NotOnCurve),
        ),
        (
            "unsupported version",
            "4s9bfpYH9CkJboPNLFC4BhTENPrjfmKwUxesnqxHBjv585bCLzVdQKuKQ5TouA57FkdDskrR695Z5U2wWwDUUVWXPg7V57sLpc9dMgx74LsVZGEB",
            |e| *e == Error::WrongNetwork,
        ),
    ];
    for (i, (name, key, want)) in tests.iter().enumerate() {
        let err = ExtendedKey::new_key_from_string(key, &MAIN).expect_err(name);
        assert!(
            want(&err),
            "NewKeyFromString #{i} ({name}): mismatched error {err:?}"
        );
    }
}

/// TestZero: zeroing a key, from a seed and from a string, neutered or not.
#[test]
fn zero() {
    let tests = [
        (
            "test vector 1 chain m",
            "000102030405060708090a0b0c0d0e0f",
            "dprv3hCznBesA6jBtmoyVFPfyMSZ1qYZ3WdjdebquvkEfmRfxC9VFEFi2YDaJqHnx7uGe75eGSa3Mn3oHK11hBW7KZUrPxwbCPBmuCi1nwm182s",
        ),
        (
            "test vector 2 chain m",
            "fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a29f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542",
            "dprv3hCznBesA6jBtPKJbQTxRZAKG2gyj8tZKEPaCsV4e9YYFBAgRP2eTSPAeu4r8dTMt9q51j2Vdt5zNqj7jbtovvocrP1qLj6WUTLF9xYQt4y",
        ),
    ];

    let test_zeroed = |i: usize, name: &str, key: &ExtendedKey| {
        // Zeroing a key makes it no longer private.
        assert!(!key.is_private(), "IsPrivate #{i} ({name})");
        assert_eq!(
            key.parent_fingerprint(),
            0,
            "ParentFingerprint #{i} ({name})"
        );
        assert_eq!(*key.string(), "zeroed extended key", "String #{i} ({name})");
        assert_eq!(
            key.serialized_priv_key().err(),
            Some(Error::NotPrivExtKey),
            "SerializedPrivKey #{i} ({name})"
        );
        assert!(
            key.serialized_pub_key().is_empty(),
            "SerializedPubKey #{i} ({name})"
        );
    };

    for (i, (name, master, ext_key)) in tests.iter().enumerate() {
        // From a seed, with its neutered version.
        let mut key = ExtendedKey::new_master(&unhex(master), &MAIN).expect("master");
        let mut neutered = key.neuter();
        key.zero();
        test_zeroed(i, &format!("{name} from seed not neutered"), &key);
        neutered.zero();
        // Upstream checks `key` again here; both are checked.
        test_zeroed(i, &format!("{name} from seed neutered"), &key);
        test_zeroed(i, &format!("{name} from seed neutered"), &neutered);

        // Deserialized, with its neutered version.
        let mut key = ExtendedKey::new_key_from_string(ext_key, &MAIN).expect("parse");
        let mut neutered = key.neuter();
        key.zero();
        test_zeroed(i, &format!("{name} deserialized not neutered"), &key);
        neutered.zero();
        test_zeroed(i, &format!("{name} deserialized neutered"), &key);
        test_zeroed(i, &format!("{name} deserialized neutered"), &neutered);
    }
}

struct Vec4 {
    name: &'static str,
    master: &'static str,
    path: &'static [u32],
    want_pub: &'static str,
    want_priv: &'static str,
    want_priv_ser: &'static str,
    leading_zeros: usize,
    net: MockNetParams,
}

/// TestBIP0032Vector4: [BIP32] test vector 4 through `child_bip32_std`,
/// which keeps leading zeros, and the stripped length under `child`.
#[test]
fn bip0032_vector4() {
    let tests = [
        Vec4 {
            name: "test vector 4 chain m",
            master: VEC4,
            path: &[],
            want_pub: "dpubZ9169KDAEUnyovAoD3NuxWgVNvAWqC3eUFUrip8ZnT4emsEvAPCyhVgoymUF6p7gug7K6ewzEaUvy77tT7LStEjj56CaZxLRMSYjypMK1yC",
            want_priv: "dprv3hCznBesA6jBuWH2xXKZSHtEkucFQFTaGhrXyuCCnNU24KDHHRsNcvAnq4596P5sYKTSgqbjUe2UNVxid8J9oDyFsuSWb3L93Jrka3vPtYV",
            want_priv_ser: "12c0d59c7aa3a10973dbd3f478b65f2516627e3fe61e00c345be9a477ad2e215",
            leading_zeros: 0,
            net: MAIN,
        },
        Vec4 {
            name: "test vector 4 chain m/0H -- leading zeros in private key serialization",
            master: VEC4,
            path: &[HKS],
            want_pub: "dpubZBYwK8RvCX3KjyCPhRQ9BYdX1ZYTvcFf67sRtoZGPnEYMCVgVyF5uWToDwePapvd1GtLYHAWGpQWo9ffod84MPvwpHmvKeZnZZf47EJcUmT",
            want_priv: "dprv3jkqwzsd88yXqZJdSuLnfKqGPYzCVffataF79tcuPhdudeU3d1uUpvwn5Btmbai3BoVYhzbsEQ7tcH1XtEJ1BhzNSAtZfqv8BtSpdq5FEpj",
            want_priv_ser: "00d948e9261e41362a688b916f297121ba6bfb2274a3575ac0e456551dfd7f7e",
            leading_zeros: 1,
            net: MAIN,
        },
        Vec4 {
            name: "test vector 4 chain m/0H/1H -- completely different key",
            master: VEC4,
            path: &[HKS, HKS + 1],
            want_pub: "dpubZDVQT3iY5Bz6NtyCE8zfnpgwkN7AEUv5mr6H6vPVPRMupBbjTJojs7qRrvddW9adSJt6obdBgW6DsUAaFD8xRVQfVB4XfWCEYdTYRZ7updr",
            want_priv: "dprv3mhK5vAEzovJUV5RycwKGbth8MYtoYL1aJTxN1T8PLmH6da6aMU8nYKQiC5Zj7mC7pH4G8xPxGZVR51FG9Ssptzx1c39A1JxADS6zDrsLPr",
            want_priv_ser: "3a2086edd7d9df86c3487a5905a1712a9aa664bce8cc268141e07549eaa8661d",
            leading_zeros: 0,
            net: MAIN,
        },
    ];

    for (i, test) in tests.iter().enumerate() {
        let seed = unhex(test.master);
        let mut ext_key = ExtendedKey::new_master(&seed, &test.net).expect("master");
        for &child_num in test.path {
            ext_key = ext_key
                .child_bip32_std(child_num)
                .unwrap_or_else(|e| panic!("ChildBIP32Std #{i} ({}): {e}", test.name));
        }

        let privkey = ext_key.serialized_priv_key().expect("private");
        assert_eq!(privkey.len(), 32, "SerializedPrivKey #{i} ({})", test.name);
        assert_eq!(
            hex(privkey),
            test.want_priv_ser,
            "Serialize #{i} ({})",
            test.name
        );
        assert_eq!(
            *ext_key.string(),
            test.want_priv,
            "Serialize #{i} ({})",
            test.name
        );

        let pub_key = ext_key.neuter();
        assert_eq!(*pub_key.neuter().string(), *pub_key.string(), "Neuter #{i}");
        assert_eq!(
            *pub_key.string(),
            test.want_pub,
            "Neuter #{i} ({})",
            test.name
        );

        // A path that produces a private key with leading zeros has them
        // stripped under the legacy Decred derivation.
        if test.leading_zeros > 0 {
            let want_len = 32 - test.leading_zeros;
            let mut ext_key = ExtendedKey::new_master(&seed, &test.net).expect("master");
            for &child_num in test.path {
                ext_key = ext_key.child(child_num).expect("child");
            }
            let privkey = ext_key.serialized_priv_key().expect("private");
            assert_eq!(
                privkey.len(),
                want_len,
                "SerializedPrivKey #{i} ({})",
                test.name
            );
        }
    }
}

/// The networks' version constants are upstream's mock values and dcrd
/// chaincfg v3.3.0's (spec 02, section 4.12).
#[test]
fn network_versions() {
    assert_eq!(HdVersions::MAINNET.private, MAIN.priv_key_id);
    assert_eq!(HdVersions::MAINNET.public, MAIN.pub_key_id);
    assert_eq!(HdVersions::TESTNET3.private, TEST.priv_key_id);
    assert_eq!(HdVersions::TESTNET3.public, TEST.pub_key_id);
    assert_eq!(HdVersions::SIMNET.private, [0x04, 0x20, 0xb9, 0x03]);
    assert_eq!(HdVersions::SIMNET.public, [0x04, 0x20, 0xbd, 0x3d]);
    assert_eq!(HdVersions::REGNET.private, [0xea, 0xb4, 0x04, 0x48]);
    assert_eq!(HdVersions::REGNET.public, [0xea, 0xb4, 0xf9, 0x87]);
}
