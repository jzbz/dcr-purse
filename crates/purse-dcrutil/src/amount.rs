// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2013, 2014 The btcsuite developers
// Copyright (c) 2015-2024 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrutil's `amount.go` and `const.go`: [`Amount`], [`AmountUnit`],
//! [`new_amount`] and the monetary constants.
//!
//! # Conversions that depend on the CPU in Go
//!
//! [`new_amount`] and [`Amount::mul_f64`] compute `round(x * y)`, where
//! `round` adds or subtracts 0.5 and converts to `int64` by truncation
//! (`amount.go:61-66`, `:88`, `:121`). Go's answer depends on the
//! architecture the wallet was built for, in two ways:
//!
//! - **Fused multiply-add.** The Go specification lets a compiler fuse
//!   `x*y + z` into one operation with a single rounding, also across an
//!   inlined call. Where Go does so here, `x*y ± 0.5` is rounded once (the
//!   sign test still uses the rounded product) instead of rounding the
//!   product and then the sum, and for inputs whose product is inexact and
//!   lands near an atom boundary the two give different atoms, in range.
//! - **Out-of-range conversion.** Go leaves converting a `float64` outside
//!   `int64`'s range (or NaN) implementation-defined, and each
//!   architecture's conversion gives its own answer.
//!
//! dcrwallet's release bundles are built for amd64 (macOS, FreeBSD, Linux,
//! OpenBSD, Windows), arm64 (macOS, Linux, OpenBSD, Windows), 386 (Linux,
//! Windows), arm (Linux, `GOARM=7`) and riscv64 (Linux). With Go 1.27.4 and
//! dcrutil v4.0.3 they compute `round(x * y)` as follows. The instructions
//! are read from Go's assembly output (`go build -gcflags=-S`) for each
//! target; the conversion results are what the instruction set or the Go
//! runtime source defines unless the entry says they were run.
//!
//! - **amd64 at GOAMD64 v1 or v2** (v1 is the default, and what the
//!   releases use): the product and the sum are rounded separately
//!   (`MULSD`, then `ADDSD`/`SUBSD`); `CVTTSD2SQ` gives `MinInt64` for
//!   NaN and for every value outside `[-2^63, 2^63)`. This is the build
//!   the oracle tests run, so these answers were run.
//! - **amd64 at GOAMD64 v3 or v4**: Go fuses the non-negative branch
//!   (`VFMADD231SD` for `x*y + 0.5`) and rounds the negative one
//!   separately (`SUBSD`); the conversion is still `CVTTSD2SQ`. Such a
//!   build is not modelled, and the oracle tests refuse it. dcrwallet's
//!   releases are not built that way.
//! - **arm64**: both branches fused (`FMADDD`/`FNMSUBD`); `FCVTZS`
//!   saturates to `MinInt64`/`MaxInt64` and gives 0 for NaN. The oracle
//!   tests model this build and check the model against Go wherever they
//!   run on arm64.
//! - **riscv64**: both branches fused (`FMADDD`/`FMSUBD`); `FCVT.L.D`
//!   (which Go issues with round-toward-zero) saturates to
//!   `MinInt64`/`MaxInt64` and gives `MaxInt64` for NaN, as the RISC-V
//!   instruction set defines. Not run.
//! - **386 and arm**: nothing fused (`MULSD` then `ADDSD`/`SUBSD` on 386,
//!   `MULD` then `ADDD`/`SUBD` on arm), so in range these agree with
//!   amd64. The conversion is the runtime's software
//!   `runtime.float64toint64` (`runtime/vlrt.go`, `_d2v`): a value below
//!   2^64 in magnitude is truncated and wrapped modulo 2^64, so `2^63`
//!   gives `MinInt64` and `NewAmount(1e11)` gives
//!   `-8446744073709551616`; from 2^64 up, and for ±Inf and NaN, the
//!   result comes from `uint32(d)`, which differs between the two. On 386
//!   that is the x87 64-bit integer indefinite cut to 32 bits, so the
//!   answer is 0: `NewAmount(±1e300)`, `NewAmount(±1e20)`,
//!   `MulF64(NaN)` and `MulF64(1e300)` are all 0 (a 386 build of
//!   dcrutil v4.0.3 was run for these). On arm it is `VCVT.U32.F64`,
//!   which saturates (and gives 0 for NaN), so a positive value gives
//!   `-4294967296` (`0xffffffff00000000`) and a negative one or NaN gives
//!   0: `NewAmount(1e300)` is `-4294967296` there. The arm answers follow
//!   from the runtime source, its arm assembly and the Arm instruction
//!   set; they were not run, and they are not 386's.
//!
//! dcr-purse's observable behaviour must not depend on the architecture,
//! so this crate **follows amd64 Go (GOAMD64 v1/v2) on every target**, as
//! spec 18 recommends for its decision D4 (spec 17 lists the same choice
//! as open question 4; QUIRKS QK-0512 records the overflow): the product
//! and the sum are rounded separately, as Rust always does, and an
//! out-of-range or NaN result of the rounding is `i64::MIN`, never Rust's
//! saturating `as`. Only a dcrwallet built for amd64 at GOAMD64 v1 or v2
//! agrees with dcr-purse on every input. One built for arm64 or riscv64
//! can disagree in range (fused rounding) and out of range; one built for
//! 386 or arm agrees in range and disagrees out of range and for NaN.
//!
//! [`Amount::to_unit`] divides by Go's `math.Pow10` (a table product, not
//! always the correctly rounded power of ten), ported with its tables. Its
//! one NaN, 0/0 for a zero amount in a unit below 10^-331 coins, is
//! amd64's (`0xfff8000000000000`, sign bit set; 386's SSE2 division gives
//! the same); arm64's is `0x7ff8000000000000`, and arm's and riscv64's
//! default NaNs have that value too by their instruction sets (not run).
//! No text output shows the difference.

use std::fmt;

use crate::gomath::{AMD64_DEFAULT_NAN, go_amd64_round, pow10};

/// `AtomsPerCent` (`const.go:9`): atoms in one coin cent.
pub const ATOMS_PER_CENT: i64 = 1_000_000;

/// `AtomsPerCoin` (`const.go:12`): atoms in one coin.
pub const ATOMS_PER_COIN: i64 = 100_000_000;

/// `MaxAmount` (`const.go:15`): 21 million coins in atoms, the largest
/// transaction amount allowed.
///
/// In Go the three constants are untyped floating-point constants, so a Go
/// caller that passes `MaxAmount` itself to `fmt`'s `%v` prints
/// `2.1e+15`, not the integer; code reproducing such a message must spell
/// it that way.
pub const MAX_AMOUNT: i64 = 2_100_000_000_000_000;

/// A unit for expressing an [`Amount`] (Go's `AmountUnit`, an `int`): the
/// exponent of ten from coins to the unit.
///
/// Any value is a unit, as in Go; the named ones are the associated
/// constants.
///
/// Go's `int`, and so `AmountUnit`, is 64 bits on amd64, arm64 and
/// riscv64 but 32 bits on 386 and arm, two of the architectures dcrwallet
/// is released for. There a unit is an `int32`, and the `u + 8` of
/// `ToUnit` and the `-(u + 8)` of `Format` and `String` wrap at 32 bits.
/// This crate uses 64 bits on every target, 64-bit Go's width. No daemon
/// caller passes a unit where the width matters: dcrwallet formats in
/// [`AmountUnit::COIN`] only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AmountUnit(pub i64);

impl AmountUnit {
    /// `AmountMegaCoin` (6): `MDCR`.
    pub const MEGA_COIN: AmountUnit = AmountUnit(6);
    /// `AmountKiloCoin` (3): `kDCR`.
    pub const KILO_COIN: AmountUnit = AmountUnit(3);
    /// `AmountCoin` (0): `DCR`.
    pub const COIN: AmountUnit = AmountUnit(0);
    /// `AmountMilliCoin` (-3): `mDCR`.
    pub const MILLI_COIN: AmountUnit = AmountUnit(-3);
    /// `AmountMicroCoin` (-6): `μDCR` (U+03BC GREEK SMALL LETTER MU).
    pub const MICRO_COIN: AmountUnit = AmountUnit(-6);
    /// `AmountAtom` (-8): `Atom`.
    pub const ATOM: AmountUnit = AmountUnit(-8);

    /// The SI-prefixed name of a named unit, or `None` for any other value.
    fn name(self) -> Option<&'static str> {
        match self.0 {
            6 => Some("MDCR"),
            3 => Some("kDCR"),
            0 => Some("DCR"),
            -3 => Some("mDCR"),
            -6 => Some("\u{3bc}DCR"),
            -8 => Some("Atom"),
            _ => None,
        }
    }
}

/// Go's `AmountUnit.String()` (`amount.go:34-51`): the unit's name, or
/// `1e<N> DCR` for any other unit.
impl fmt::Display for AmountUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "1e{} DCR", self.0),
        }
    }
}

/// A monetary amount in atoms, 1e-8 of a coin (Go's `Amount`, an `int64`).
///
/// Go converts an integer count of atoms with a plain conversion,
/// `Amount(n)`; here that is `Amount(n)` as well. The derived ordering is
/// the numeric one, which is what Go's `AmountSorter` sorts by: amounts
/// that compare equal are identical, so any sort gives the same slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Amount(pub i64);

/// The error [`new_amount`] returns for NaN and ±Inf: `invalid coin amount`.
///
/// Go builds a new `errors.New` value on every call (`amount.go:85`), so Go
/// callers can only match it by its text (spec 24, Q-29).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InvalidCoinAmount;

impl fmt::Display for InvalidCoinAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid coin amount")
    }
}

impl std::error::Error for InvalidCoinAmount {}

/// Go's `NewAmount` (`amount.go:76-89`): converts an amount in coins to
/// atoms.
///
/// NaN and ±Inf fail with [`InvalidCoinAmount`]. Any other value is
/// `round(f * 1e8)`: the product rounded to a `float64`, then 0.5 subtracted
/// if the product is negative and added otherwise, rounded again, then
/// truncated toward zero. There is no range check: amounts beyond the money
/// supply are returned as they are, and a result outside `int64`'s range,
/// including a product that overflows to infinity, is `i64::MIN`, as on
/// amd64 (see the [module documentation](self) for why, and for what Go on
/// arm64, riscv64, 386 and arm does instead).
pub fn new_amount(f: f64) -> Result<Amount, InvalidCoinAmount> {
    if f.is_nan() || f.is_infinite() {
        return Err(InvalidCoinAmount);
    }
    Ok(Amount(go_amd64_round(f * 1e8)))
}

impl Amount {
    /// Go's `ToUnit` (`amount.go:93-95`): `float64(a) / math.Pow10(u + 8)`,
    /// with `u + 8` wrapping at 64 bits as 64-bit Go's `int` arithmetic
    /// does (see [`AmountUnit`] for 32-bit Go).
    ///
    /// For units above 10^300 coins the divisor is `+Inf` and the result a
    /// signed zero; below 10^-331 coins it is 0, so the result is ±Inf, or
    /// NaN for a zero amount (amd64's NaN, see the module documentation).
    pub fn to_unit(self, u: AmountUnit) -> f64 {
        // The conversion rounds to nearest, ties to even, as Go's does.
        let q = self.0 as f64 / pow10(u.0.wrapping_add(8));
        if q.is_nan() { AMD64_DEFAULT_NAN } else { q }
    }

    /// Go's `ToCoin` (`amount.go:98-100`): [`Amount::to_unit`] with
    /// [`AmountUnit::COIN`], one division by 1e8. dcrwallet emits this
    /// value as a JSON number in its RPC results.
    pub fn to_coin(self) -> f64 {
        self.to_unit(AmountUnit::COIN)
    }

    /// Go's `Format` (`amount.go:106-109`): `strconv.FormatFloat(a.ToUnit(u),
    /// 'f', -(u + 8), 64)`, a space, and the unit's name.
    ///
    /// For the atom and every larger unit the precision is negative, so the
    /// number is the shortest decimal that parses back to the quotient, in
    /// positional form (`1 DCR`, `0.5 DCR`, `0.00000001 DCR`,
    /// `-92233720368.54776 DCR`); for the atom itself it is 0 places, the
    /// exact integer value of the `float64` (`9223372036854775808 Atom` for
    /// `i64::MAX`). Smaller units give `-(u + 8)` places, rounded half to
    /// even on the exact binary value as Go does. NaN and ±Inf are `NaN`,
    /// `+Inf` and `-Inf`.
    ///
    /// # Difference from Go
    ///
    /// Go's `FormatFloat` reserves `max(prec + 4, 24)` bytes (Go `int`
    /// arithmetic, wrapping) before it looks at the value. Where that is
    /// more than Go can allocate, units below about -2^48 and
    /// `i64::MAX - 2 ..= i64::MAX` (where `u + 8` wraps to a large negative
    /// number), `Format` panics (`makeslice: cap out of range`); for units
    /// somewhat above -2^48 it can exhaust memory instead. In all those
    /// cases the quotient is ±Inf or NaN, so this function returns the text
    /// Go returns whenever its allocation succeeds, and never panics. No
    /// dcrwallet code formats in such a unit: the daemon uses only
    /// [`AmountUnit::COIN`], through `String`.
    pub fn format(self, u: AmountUnit) -> String {
        // The number's own buffer takes the unit, so the text is not copied
        // into a third allocation (purse-bench `dcrutil.amount_string`).
        let mut text = self.number(u);
        text.push(' ');
        push_unit(&mut text, u);
        text
    }

    /// The number part of [`Amount::format`], before the space and the
    /// unit.
    fn number(self, u: AmountUnit) -> String {
        let q = self.to_unit(u);
        let prec = u.0.wrapping_add(8).wrapping_neg();
        if q.is_nan() {
            "NaN".to_owned()
        } else if q.is_infinite() {
            if q > 0.0 { "+Inf" } else { "-Inf" }.to_owned()
        } else if prec < 0 {
            dcroxide_dcrjson::gojson::format_float_f(q)
        } else {
            // A finite quotient needs a divisor of at least 10^-323, so
            // prec = -(u + 8) <= 323 here.
            debug_assert!(prec <= MAX_FINITE_PREC);
            let places = usize::try_from(prec.min(MAX_FINITE_PREC)).unwrap_or(0);
            // Rust's fixed-precision formatting is exact and rounds half
            // to even, as Go's bigFtoa does.
            format!("{q:.places$}")
        }
    }

    /// Go's `MulF64` (`amount.go:120-122`): `round(float64(a) * f)`, with
    /// the rounding of [`new_amount`]. NaN, an infinite `f` and every
    /// result outside `int64`'s range give `i64::MIN`, as on amd64 (Go on
    /// arm64 and riscv64 fuses the multiplication with the rounding
    /// addition and saturates, and Go on 386 and arm wraps or gives 0 or
    /// `-2^32`; see the module documentation).
    pub fn mul_f64(self, f: f64) -> Amount {
        Amount(go_amd64_round(self.0 as f64 * f))
    }
}

/// The most decimal places a finite quotient can be formatted with: the
/// divisor is at least `math.Pow10(-323)`.
const MAX_FINITE_PREC: i64 = 323;

/// Appends `u` as [`AmountUnit`]'s `Display` writes it.
fn push_unit(text: &mut String, u: AmountUnit) {
    match u.name() {
        Some(name) => text.push_str(name),
        None => {
            use fmt::Write as _;
            // Writing to a String cannot fail.
            let _ = write!(text, "{u}");
        }
    }
}

/// Go's `Amount.String()` (`amount.go:112-114`): [`Amount::format`] in
/// coins, such as `0.001 DCR`. dcrwallet puts this text in log lines and
/// some error messages.
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The parts go to the formatter as they are, without first being
        // joined into a String of their own. " DCR" is the space and
        // AmountUnit::COIN's name.
        f.write_str(&self.number(AmountUnit::COIN))?;
        f.write_str(" DCR")
    }
}
