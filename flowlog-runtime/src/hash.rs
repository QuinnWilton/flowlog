//! `hash(v1, ..., vn)`: a 64-bit id of a sequence of values, the same in
//! every engine, on every platform and in every run.
//!
//! The id is the first eight bytes, read big-endian, of the SHA-256 digest
//! of the values' encodings laid end to end. Each value encodes as a tag
//! byte followed by its contents:
//!
//! | Value | Encoding |
//! |-------|----------|
//! | integer of any width | `i`, then the value as a 128-bit big-endian two's complement |
//! | float of either width | `f`, then the value as an `f64`'s big-endian bits |
//! | string | `s`, then its UTF-8 length in bytes as a 64-bit big-endian, then the bytes |
//! | bool | `b`, then `1` for `true` or `0` for `false` |
//!
//! An encoding is decided by the value, not by its column's type: `1` hashes
//! alike as an `int8` and a `uint64`, and an `f32` as the `f64` it widens to
//! exactly. A float's `-0` encodes as `0`, and every NaN as the quiet NaN
//! `0x7FF8000000000000`, so values the engines hold equal hash equal. Every
//! encoding announces its own length, so two sequences encode alike only
//! when they are the same values in the same order.
//!
//! SHA-256 is chosen for its specification, not its strength: any
//! implementation of it, the BEAM's `:crypto` among them, computes the same
//! ids, and over 10 million distinct values two share an id with a
//! probability of about 3 × 10⁻⁶.

use ordered_float::OrderedFloat;
use sha2::Digest;
use sha2::Sha256;

use crate::intern::Symbol;
use crate::intern::resolve;

/// The bits a NaN of either width hashes as.
const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

/// A `hash` call's values, absorbed one at a time.
#[derive(Clone, Default)]
#[must_use]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorbs `value` by its own encoding ([`Field`]).
    pub fn field(self, value: impl Field) -> Self {
        value.absorb(self)
    }

    pub fn integer(mut self, value: i128) -> Self {
        self.0.update(b"i");
        self.0.update(value.to_be_bytes());
        self
    }

    pub fn float(mut self, value: f64) -> Self {
        let bits = match value {
            v if v.is_nan() => CANONICAL_NAN,
            // A float pattern matches by `==`, and `-0 == 0`, so this arm
            // takes both zeros.
            0.0 => 0,
            v => v.to_bits(),
        };
        self.0.update(b"f");
        self.0.update(bits.to_be_bytes());
        self
    }

    pub fn text(mut self, value: &str) -> Self {
        self.0.update(b"s");
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value.as_bytes());
        self
    }

    pub fn boolean(mut self, value: bool) -> Self {
        self.0.update([b'b', u8::from(value)]);
        self
    }

    /// The id of the values absorbed so far.
    #[must_use]
    pub fn finish(self) -> u64 {
        let digest = self.0.finalize();
        let mut head = [0; 8];
        head.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(head)
    }
}

/// A value `hash` accepts, absorbed by the encoding its kind gives it.
pub trait Field {
    fn absorb(self, hasher: Hasher) -> Hasher;
}

macro_rules! integer_field {
    ($($ty:ty),*) => {$(
        impl Field for $ty {
            #[inline]
            fn absorb(self, hasher: Hasher) -> Hasher {
                hasher.integer(i128::from(self))
            }
        }
    )*};
}

integer_field!(i8, i16, i32, i64, u8, u16, u32, u64);

impl Field for OrderedFloat<f32> {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.float(f64::from(self.0))
    }
}

impl Field for OrderedFloat<f64> {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.float(self.0)
    }
}

impl Field for f32 {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.float(f64::from(self))
    }
}

impl Field for f64 {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.float(self)
    }
}

impl Field for bool {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.boolean(self)
    }
}

impl Field for Symbol {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.text(resolve(self))
    }
}

impl Field for &str {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.text(self)
    }
}

impl Field for String {
    #[inline]
    fn absorb(self, hasher: Hasher) -> Hasher {
        hasher.text(&self)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::intern::intern;

    /// Ids computed on the BEAM, as
    /// `<<id::64, _::binary>> = :crypto.hash(:sha256, encoding)`: the
    /// encoding is the module's, not this implementation's.
    #[rstest]
    #[case::nothing(Hasher::new(), 16_406_829_232_824_261_652)]
    #[case::empty_text(Hasher::new().text(""), 8_559_766_167_582_867_106)]
    #[case::text(Hasher::new().text("hello"), 17_882_697_362_519_485_994)]
    #[case::one(Hasher::new().integer(1), 16_304_050_518_116_838_669)]
    #[case::minus_one(Hasher::new().integer(-1), 12_596_678_502_499_993_939)]
    #[case::largest_u64(Hasher::new().field(u64::MAX), 5_816_329_776_564_699_570)]
    #[case::one_and_a_half(Hasher::new().float(1.5), 12_208_416_065_220_454_938)]
    #[case::truth(Hasher::new().boolean(true), 1_151_421_812_079_010_336)]
    #[case::constructor(
        Hasher::new().text("Called").integer(7).text("ctx"),
        5_487_416_791_946_292_471
    )]
    fn ids_are_the_specified_ones(#[case] hasher: Hasher, #[case] id: u64) {
        assert_eq!(hasher.finish(), id);
    }

    #[test]
    fn an_integer_hashes_by_its_value_whatever_its_width() {
        let ids = [
            Hasher::new().field(5_i8).finish(),
            Hasher::new().field(5_u16).finish(),
            Hasher::new().field(5_i32).finish(),
            Hasher::new().field(5_u64).finish(),
        ];
        assert!(ids.iter().all(|&id| id == ids[0]));
    }

    #[test]
    fn an_f32_hashes_as_the_f64_it_widens_to() {
        assert_eq!(
            Hasher::new().field(OrderedFloat(0.1_f32)).finish(),
            Hasher::new().float(f64::from(0.1_f32)).finish()
        );
    }

    #[test]
    fn values_held_equal_hash_equal() {
        assert_eq!(
            Hasher::new().float(-0.0).finish(),
            Hasher::new().float(0.0).finish()
        );
        assert_eq!(
            Hasher::new().float(f64::NAN).finish(),
            Hasher::new()
                .float(-f64::from_bits(0x7FF0_0000_0000_0001))
                .finish()
        );
    }

    #[test]
    fn a_string_hashes_alike_interned_or_not() {
        let id = Hasher::new().field("x").finish();
        assert_eq!(Hasher::new().field(intern("x")).finish(), id);
        assert_eq!(Hasher::new().field(String::from("x")).finish(), id);
    }

    /// Lengths delimit strings, so no split of the same text across two
    /// strings hashes as another split of it.
    #[test]
    fn a_split_of_one_text_is_not_another_split() {
        assert_ne!(
            Hasher::new().text("ab").text("c").finish(),
            Hasher::new().text("a").text("bc").finish()
        );
    }

    /// Tags set kinds apart where their contents coincide: the integer
    /// `1` against the bool `true`, and `1` against the float `1.0`.
    #[test]
    fn kinds_are_told_apart() {
        let one = Hasher::new().integer(1).finish();
        assert_ne!(Hasher::new().boolean(true).finish(), one);
        assert_ne!(Hasher::new().float(1.0).finish(), one);
    }

    #[test]
    fn order_matters() {
        assert_ne!(
            Hasher::new().integer(1).integer(2).finish(),
            Hasher::new().integer(2).integer(1).finish()
        );
    }
}
