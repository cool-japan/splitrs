//! Fixed-width integer type modelling for the SMT equivalence oracle.
//!
//! Maps Rust primitive integer types onto QF_BV bit-widths, decodes
//! counterexample bit-patterns back into human-readable decimals honouring
//! two's-complement, and provides zero/sign-extension term builders (OxiZ has
//! no built-in extend helper, so we synthesise them from concat/extract).

use num_bigint::BigInt;
use oxiz::{TermId, TermManager};

/// A fixed-width Rust integer type lowered to a QF_BV bit-vector sort.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntType {
    /// Bit width of the integer (8, 16, 32, 64 or 128).
    pub width: u32,
    /// Whether the integer is signed (`iN`) or unsigned (`uN`).
    pub signed: bool,
}

/// Map a `syn::Type` to an [`IntType`] when it names a fixed-width Rust integer.
///
/// Supported: `i8`/`u8` … `i128`/`u128`, plus `isize`/`usize` modelled as
/// 64-bit (the common 64-bit target). Anything else — references, slices,
/// `String`, `f32`/`f64`, generics, paths with segments — returns `None` so
/// the caller can report the construct as Unsupported.
#[must_use]
pub fn int_type_of(ty: &syn::Type) -> Option<IntType> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    // Reject qualified paths (`<T as Trait>::Assoc`) and multi-segment paths
    // (`std::primitive::u32`) — only a bare single-segment ident is a primitive.
    if type_path.qself.is_some() {
        return None;
    }
    if type_path.path.segments.len() != 1 {
        return None;
    }
    let segment = type_path.path.segments.first()?;
    // A primitive int never carries generic arguments.
    if !matches!(segment.arguments, syn::PathArguments::None) {
        return None;
    }
    int_type_from_name(&segment.ident.to_string())
}

/// Map a primitive type name (e.g. `"u32"`) to an [`IntType`].
#[must_use]
pub fn int_type_from_name(name: &str) -> Option<IntType> {
    let (signed, width) = match name {
        "i8" => (true, 8),
        "u8" => (false, 8),
        "i16" => (true, 16),
        "u16" => (false, 16),
        "i32" => (true, 32),
        "u32" => (false, 32),
        "i64" => (true, 64),
        "u64" => (false, 64),
        "i128" => (true, 128),
        "u128" => (false, 128),
        // isize/usize modelled as 64-bit (the dominant 64-bit target).
        "isize" => (true, 64),
        "usize" => (false, 64),
        _ => return None,
    };
    Some(IntType { width, signed })
}

/// Render a model bit-pattern (`raw`, an unsigned magnitude in `[0, 2^width)`)
/// as a decimal string, honouring two's-complement for signed types.
#[must_use]
pub fn decode_bits(raw: &BigInt, ty: IntType) -> String {
    let modulus = BigInt::from(1u8) << ty.width;
    // Normalise into the unsigned range first (defensive: solver values are
    // already in range, but a stray sign or overflow bit would corrupt output).
    let mut unsigned = raw % &modulus;
    if unsigned.sign() == num_bigint::Sign::Minus {
        unsigned += &modulus;
    }
    if ty.signed {
        let half = BigInt::from(1u8) << (ty.width - 1);
        if unsigned >= half {
            // High bit set → negative value: subtract 2^width.
            return (unsigned - modulus).to_string();
        }
    }
    unsigned.to_string()
}

/// Zero-extend `term` from `from_w` bits to `to_w` bits by prepending a zero
/// block. `to_w` must be strictly greater than `from_w`.
pub fn zero_extend(tm: &mut TermManager, term: TermId, from_w: u32, to_w: u32) -> TermId {
    if to_w <= from_w {
        return term;
    }
    let pad_bits = to_w - from_w;
    let zero = tm.mk_bitvec(0i64, pad_bits);
    // concat(high, low): the zero pad becomes the most-significant bits.
    tm.try_mk_bv_concat(zero, term)
        .expect("zero-padding literal and term are both bit-vector sorted")
}

/// Sign-extend `term` from `from_w` bits to `to_w` bits by replicating the
/// sign bit. `to_w` must be strictly greater than `from_w`.
pub fn sign_extend(tm: &mut TermManager, term: TermId, from_w: u32, to_w: u32) -> TermId {
    if to_w <= from_w {
        return term;
    }
    let pad_bits = to_w - from_w;
    // Extract the sign bit (the most-significant bit of the source).
    let sign_bit = tm.mk_bv_extract(from_w - 1, from_w - 1, term);
    // Replicate it `pad_bits` times via repeated concat, then prepend.
    let mut replicated = sign_bit;
    for _ in 1..pad_bits {
        replicated = tm
            .try_mk_bv_concat(replicated, sign_bit)
            .expect("replicated sign bit and sign bit are both bit-vector sorted");
    }
    tm.try_mk_bv_concat(replicated, term)
        .expect("replicated sign bits and term are both bit-vector sorted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;

    fn parse_ty(src: &str) -> syn::Type {
        syn::parse_str(src).expect("type should parse")
    }

    #[test]
    fn maps_unsigned_widths() {
        assert_eq!(
            int_type_of(&parse_ty("u8")),
            Some(IntType {
                width: 8,
                signed: false
            })
        );
        assert_eq!(
            int_type_of(&parse_ty("u32")),
            Some(IntType {
                width: 32,
                signed: false
            })
        );
        assert_eq!(
            int_type_of(&parse_ty("u128")),
            Some(IntType {
                width: 128,
                signed: false
            })
        );
    }

    #[test]
    fn maps_signed_widths() {
        assert_eq!(
            int_type_of(&parse_ty("i8")),
            Some(IntType {
                width: 8,
                signed: true
            })
        );
        assert_eq!(
            int_type_of(&parse_ty("i64")),
            Some(IntType {
                width: 64,
                signed: true
            })
        );
    }

    #[test]
    fn maps_isize_usize_to_64() {
        assert_eq!(
            int_type_of(&parse_ty("usize")),
            Some(IntType {
                width: 64,
                signed: false
            })
        );
        assert_eq!(
            int_type_of(&parse_ty("isize")),
            Some(IntType {
                width: 64,
                signed: true
            })
        );
    }

    #[test]
    fn rejects_non_integer_types() {
        assert_eq!(int_type_of(&parse_ty("f32")), None);
        assert_eq!(int_type_of(&parse_ty("f64")), None);
        assert_eq!(int_type_of(&parse_ty("String")), None);
        assert_eq!(int_type_of(&parse_ty("&u32")), None);
        assert_eq!(int_type_of(&parse_ty("[u8]")), None);
        assert_eq!(int_type_of(&parse_ty("Vec<u8>")), None);
        assert_eq!(int_type_of(&parse_ty("std::primitive::u32")), None);
        assert_eq!(int_type_of(&parse_ty("bool")), None);
    }

    #[test]
    fn decodes_unsigned() {
        let u8t = IntType {
            width: 8,
            signed: false,
        };
        assert_eq!(decode_bits(&BigInt::from(0), u8t), "0");
        assert_eq!(decode_bits(&BigInt::from(255), u8t), "255");
        assert_eq!(decode_bits(&BigInt::from(128), u8t), "128");
    }

    #[test]
    fn decodes_signed_two_complement() {
        let i8t = IntType {
            width: 8,
            signed: true,
        };
        // 0xFF (255) -> -1
        assert_eq!(decode_bits(&BigInt::from(255), i8t), "-1");
        // 0x80 (128) -> -128
        assert_eq!(decode_bits(&BigInt::from(128), i8t), "-128");
        // 0x7F (127) -> 127 (largest positive)
        assert_eq!(decode_bits(&BigInt::from(127), i8t), "127");
        // 0x00 -> 0
        assert_eq!(decode_bits(&BigInt::from(0), i8t), "0");
    }

    #[test]
    fn decodes_signed_32bit() {
        let i32t = IntType {
            width: 32,
            signed: true,
        };
        // 0xFFFFFFFF -> -1
        let all_ones = (BigInt::from(1u8) << 32) - 1;
        assert_eq!(decode_bits(&all_ones, i32t), "-1");
        // 0x80000000 -> -2147483648
        let min = BigInt::from(1u8) << 31;
        assert_eq!(decode_bits(&min, i32t), "-2147483648");
    }
}
