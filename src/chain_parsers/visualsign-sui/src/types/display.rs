//! Move type strings in the two forms the parser emits.
//!
//! `sui_sdk_types::TypeTag`'s own `Display` writes full-length addresses, which
//! is neither form below, so these are written out explicitly.

use std::fmt::Write as _;

use sui_sdk_types::TypeTag;

/// Move's short form, as `move_core_types::TypeTag`'s `Display` writes it:
/// `0x2::coin::Coin<0x2::sui::SUI, u64>` (leading zeros trimmed, `", "` between
/// type parameters).
#[must_use]
pub fn type_tag_to_string(tag: &TypeTag) -> String {
    let mut out = String::new();
    write_type_tag(&mut out, tag, false);
    out
}

/// Move's canonical form with a `0x` prefix, as `TypeInput::to_canonical_string(true)`
/// writes it: full 64-digit addresses and `","` between type parameters.
#[must_use]
pub fn type_tag_to_canonical_string(tag: &TypeTag) -> String {
    let mut out = String::new();
    write_type_tag(&mut out, tag, true);
    out
}

fn write_type_tag(out: &mut String, tag: &TypeTag, canonical: bool) {
    match tag {
        TypeTag::U8 => out.push_str("u8"),
        TypeTag::U16 => out.push_str("u16"),
        TypeTag::U32 => out.push_str("u32"),
        TypeTag::U64 => out.push_str("u64"),
        TypeTag::U128 => out.push_str("u128"),
        TypeTag::U256 => out.push_str("u256"),
        TypeTag::Bool => out.push_str("bool"),
        TypeTag::Address => out.push_str("address"),
        TypeTag::Signer => out.push_str("signer"),
        TypeTag::Vector(inner) => {
            out.push_str("vector<");
            write_type_tag(out, inner, canonical);
            out.push('>');
        }
        TypeTag::Struct(tag) => {
            let address = tag.address().into_inner();
            let hex = hex::encode(address);
            let address = if canonical {
                hex.as_str()
            } else {
                match hex.trim_start_matches('0') {
                    "" => "0",
                    trimmed => trimmed,
                }
            };
            let _ = write!(out, "0x{address}::{}::{}", tag.module(), tag.name());
            if let Some((first, rest)) = tag.type_params().split_first() {
                out.push('<');
                write_type_tag(out, first, canonical);
                for param in rest {
                    out.push_str(if canonical { "," } else { ", " });
                    write_type_tag(out, param, canonical);
                }
                out.push('>');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_canonical_forms() {
        let tag: TypeTag = "0x2::coin::Coin<0x2::sui::SUI, vector<u64>>"
            .parse()
            .unwrap();
        assert_eq!(
            type_tag_to_string(&tag),
            "0x2::coin::Coin<0x2::sui::SUI, vector<u64>>"
        );
        let two = format!("0x{:0>64}", "2");
        assert_eq!(
            type_tag_to_canonical_string(&tag),
            format!("{two}::coin::Coin<{two}::sui::SUI,vector<u64>>")
        );
        let zero: TypeTag = "0x0::m::T".parse().unwrap();
        assert_eq!(type_tag_to_string(&zero), "0x0::m::T");
    }
}
