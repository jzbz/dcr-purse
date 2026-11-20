// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2026 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's transaction authoring, ported from the parity pin
//! (`wallet/txauthor`; spec 10 §2.3, §4.5, §4.6): the loop that picks inputs
//! from an abstract source until they pay for the outputs and for their own
//! fee, adds a change output unless the remainder is dust, randomizes the
//! change position, and signs every input.
//!
//! | Go | Rust |
//! |---|---|
//! | `InputDetail` | [`InputDetail`] |
//! | `InputSource` (a func type) | [`InputSource`] (any `FnMut(Amount) -> Result<InputDetail, AnyError>`) |
//! | `ChangeSource` | [`ChangeSource`] |
//! | `AuthoredTx` | [`AuthoredTx`] |
//! | `NewUnsignedTransaction` | [`new_unsigned_transaction`] |
//! | `RandomizeOutputPosition` | [`randomize_output_position`], [`randomize_output_position_with`] |
//! | `(*AuthoredTx).RandomizeChangePosition` | [`AuthoredTx::randomize_change_position`], [`AuthoredTx::randomize_change_position_with`] |
//! | `SecretsSource` | [`SecretsSource`] |
//! | `AddAllInputScripts`, `(*AuthoredTx).AddAllInputScripts` | [`add_all_input_scripts`], [`AuthoredTx::add_all_input_scripts`] |
//! | `generatedTxVersion` | [`GENERATED_TX_VERSION`] |
//! | the global `rand.Int32N` | `dcroxide_crypto::rand::int32n`, [`Prng::int32n`](dcroxide_crypto::rand::Prng::int32n) |
//!
//! # Integer semantics
//!
//! As in `purse-txrules`: amounts are [`Amount`](purse_dcrutil::Amount)
//! (int64 atoms) and sizes are Go `int`, `i64` here, and every sum and
//! difference wraps as Go's does, so an output total near `i64::MAX` gives
//! Go's targets, comparisons and errors.
//!
//! # Randomness
//!
//! Go draws the change position from dcrd's process-wide `crypto/rand`
//! generator. The plain forms draw from `dcroxide-crypto`'s port of that
//! generator, holding its lock for the whole bounded draw as Go holds it;
//! the `_with` forms draw from a caller's
//! [`Prng`](dcroxide_crypto::rand::Prng). The bounded draw is dcroxide's
//! port of Go's `Int32N`, one 64-bit draw per attempt, so the same stream
//! gives Go's positions (docs/specs/23 §5.6.2); the oracle differential
//! checks this against a copy of dcrwallet's `author.go`, verbatim but for
//! its import of the generator, drawing from the seeded stream that
//! `Prng::from_seed` also produces.
//!
//! # Divergences
//!
//! Each is tested and recorded in `docs/PARITY.md` with its QUIRKS entry:
//!
//! - [`new_unsigned_transaction`] returns its insufficient-balance error
//!   when a retry would ask the input source again for the target it was
//!   just asked for, which happens only after a wrapped sum, an output
//!   total within the fee of `i64::MAX` (QK-0744) or a source amount
//!   exceeding a wrapped output total by more than `i64::MAX` (QK-0753);
//!   Go asks for that target again, and forever while the source answers
//!   as before (ADR-0012 Rule 2(a); project owner, 2026-11-19). That stall
//!   is the only one a source whose answer settles can cause, the wallet's
//!   among them; a source whose answers make the targets cycle with a
//!   longer period keeps Go's endless loop.
//! - [`randomize_output_position_with`] returns an error of kind `Invalid`
//!   where Go panics: no outputs, or an index outside them, as for an
//!   authored transaction without change, which JSON-RPC `spendoutputs`
//!   randomizes unconditionally (ADR-0012 Rule 2(c); QK-0263, QK-0492).
//! - [`add_all_input_scripts`] returns an error where Go's signer panics on
//!   a pay-to-pubkey previous or redeem script whose public key does not
//!   parse (ADR-0012 Rule 2(a); QK-0745).
//! - [`add_all_input_scripts`] returns an error exactly where Go's
//!   signature-script merge recurses until the process dies of a stack
//!   overflow: a pay-to-script-hash input whose redeem script is itself
//!   pay-to-script-hash, plain or stake-tagged, whose new signature script
//!   is nonempty, parses and ends in a push of a plain pay-to-script-hash
//!   script, with an existing signature script that is nonempty and parses
//!   (ADR-0012 Rule 2(a); QK-0746).
//! - [`add_all_input_scripts`] refuses an ECDSA key that reduces to zero
//!   modulo the group order, with `invalid privkey`, where Go signs with it
//!   (ADR-0012 Rule 7; QK-0682). Every other secp256k1 key is read as Go's
//!   `PrivKeyFromBytes` reads it, which dcroxide's signer alone would not
//!   do.
//!
//! Not representable (ADR-0012 Rule 4): a nil input detail with a nil
//! error, a nil change source (`cmd/movefunds` passes one, QK-0263), a nil
//! secrets source (spec 10 quirk 56), and a signature type outside dcrec's
//! three. The loop moves the source's inputs and the caller's outputs into
//! the transaction where Go aliases the slices, which spec 10 quirk 18
//! shows unobservable.
//!
//! # Checks
//!
//! Besides the oracle differential, the frozen vectors, the ported
//! upstream tests and the property tests (`tests/`), the cargo-fuzz target
//! `txauthor` (`fuzz/fuzz_targets/txauthor.rs`) drives the authoring loop
//! with arbitrary values and adversarial input sources, checking every
//! call and the result against the loop's definition, and the signing loop
//! with arbitrary scripts, keys and lookups; on honest inputs it verifies
//! the signatures in dcroxide's script engine and the signed size against
//! the estimate. It also builds the two signing shapes where Go dies,
//! which random scripts reach too seldom for a short run, and checks the
//! crate's error for each: the nested pay-to-script-hash merge (QK-0746)
//! and the pay-to-pubkey key that does not parse (QK-0745).

mod author;
mod random;
mod sign;

pub use author::{
    AuthoredTx, ChangeSource, GENERATED_TX_VERSION, InputDetail, InputSource,
    new_unsigned_transaction,
};
pub use random::{randomize_output_position, randomize_output_position_with};
pub use sign::{
    INVALID_PUBKEY_SCRIPT, NESTED_SCRIPT_HASH_MERGE, SecretsSource, add_all_input_scripts,
};
