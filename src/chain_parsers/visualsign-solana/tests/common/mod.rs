//! Shared `#[derive(Arbitrary)]` model types for IDL-based property tests.
//!
//! These types mirror the IDL type system from `solana_parser::solana::structs`
//! and are deliberately coupled to it: every `Arb*` type has a `From` impl that
//! converts to the corresponding `solana_parser` type, so a compiler error is
//! produced if the two type systems ever diverge.
//!
//! Every type derives `proptest::arbitrary::Arbitrary` (via `proptest-derive`)
//! so that tests can write `any::<ArbIdl>()` instead of composing manual
//! proptest strategy functions.
//!
//! Each type exposes a `to_json()` / `to_json_string()` method that produces
//! the JSON consumed by `decode_idl_data`, and a `From` impl that produces the
//! equivalent `solana_parser` type directly (no JSON roundtrip).

#![allow(dead_code)]

use proptest::prelude::*;
use proptest_derive::Arbitrary;
use solana_parser::solana::structs::IdlType;

// ── Primitive types ───────────────────────────────────────────────────────────

/// All IDL primitive types in their Anchor JSON wire format.
///
/// `From<&ArbPrimitive> for IdlType` provides the explicit compile-time
/// coupling to `solana_parser::solana::structs::IdlType`.
#[derive(Debug, Clone, Arbitrary)]
pub enum ArbPrimitive {
    Bool,
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    PublicKey,
    String,
    Bytes,
}

impl ArbPrimitive {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Bool => serde_json::json!("bool"),
            Self::U8 => serde_json::json!("u8"),
            Self::U16 => serde_json::json!("u16"),
            Self::U32 => serde_json::json!("u32"),
            Self::U64 => serde_json::json!("u64"),
            Self::U128 => serde_json::json!("u128"),
            Self::I8 => serde_json::json!("i8"),
            Self::I16 => serde_json::json!("i16"),
            Self::I32 => serde_json::json!("i32"),
            Self::I64 => serde_json::json!("i64"),
            Self::I128 => serde_json::json!("i128"),
            Self::F32 => serde_json::json!("f32"),
            Self::F64 => serde_json::json!("f64"),
            Self::PublicKey => serde_json::json!("publicKey"),
            Self::String => serde_json::json!("string"),
            Self::Bytes => serde_json::json!("bytes"),
        }
    }
}

/// Converts an `ArbPrimitive` to the equivalent `solana_parser::solana::structs::IdlType`.
///
/// This `From` impl is the **compile-time dependency anchor**: if `IdlType` gains
/// or loses a primitive variant in `solana_parser`, this match will fail to
/// compile, alerting test authors to update `ArbPrimitive` accordingly.
impl From<&ArbPrimitive> for IdlType {
    fn from(p: &ArbPrimitive) -> IdlType {
        match p {
            ArbPrimitive::Bool => IdlType::Bool,
            ArbPrimitive::U8 => IdlType::U8,
            ArbPrimitive::U16 => IdlType::U16,
            ArbPrimitive::U32 => IdlType::U32,
            ArbPrimitive::U64 => IdlType::U64,
            ArbPrimitive::U128 => IdlType::U128,
            ArbPrimitive::I8 => IdlType::I8,
            ArbPrimitive::I16 => IdlType::I16,
            ArbPrimitive::I32 => IdlType::I32,
            ArbPrimitive::I64 => IdlType::I64,
            ArbPrimitive::I128 => IdlType::I128,
            ArbPrimitive::F32 => IdlType::F32,
            ArbPrimitive::F64 => IdlType::F64,
            ArbPrimitive::PublicKey => IdlType::PublicKey,
            ArbPrimitive::String => IdlType::String,
            ArbPrimitive::Bytes => IdlType::Bytes,
        }
    }
}

// ── Container / composite types ───────────────────────────────────────────────

/// IDL field type: a primitive or a single-level container wrapping a
/// primitive.
///
/// Weights: 4 Primitive : 1 Vec : 1 Option : 1 Array : 1 VecOption :
///          1 OptionVec — matching the original manual `arb_idl_type()`.
///
/// `From<&ArbIdlType> for IdlType` provides the explicit compile-time
/// coupling to `solana_parser::solana::structs::IdlType`.
#[derive(Debug, Clone, Arbitrary)]
pub enum ArbIdlType {
    #[proptest(weight = 4)]
    Primitive(ArbPrimitive),
    #[proptest(weight = 1)]
    Vec(ArbPrimitive),
    #[proptest(weight = 1)]
    Option(ArbPrimitive),
    /// Fixed-size array; length is generated in the range 1–4.
    #[proptest(weight = 1)]
    Array(ArbPrimitive, #[proptest(strategy = "1usize..=4")] usize),
    /// `Vec<Option<T>>`
    #[proptest(weight = 1)]
    VecOption(ArbPrimitive),
    /// `Option<Vec<T>>`
    #[proptest(weight = 1)]
    OptionVec(ArbPrimitive),
}

impl ArbIdlType {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Primitive(p) => p.to_json(),
            Self::Vec(p) => serde_json::json!({ "vec": p.to_json() }),
            Self::Option(p) => serde_json::json!({ "option": p.to_json() }),
            Self::Array(p, n) => serde_json::json!({ "array": [p.to_json(), n] }),
            Self::VecOption(p) => serde_json::json!({ "vec": { "option": p.to_json() } }),
            Self::OptionVec(p) => serde_json::json!({ "option": { "vec": p.to_json() } }),
        }
    }
}

/// Converts an `ArbIdlType` to the equivalent `solana_parser::solana::structs::IdlType`.
///
/// This `From` impl is the **compile-time dependency anchor** for container
/// types: if `IdlType`'s container variants change in `solana_parser`, this
/// match will fail to compile.
impl From<&ArbIdlType> for IdlType {
    fn from(t: &ArbIdlType) -> IdlType {
        match t {
            ArbIdlType::Primitive(p) => IdlType::from(p),
            ArbIdlType::Vec(p) => IdlType::Vec(Box::new(IdlType::from(p))),
            ArbIdlType::Option(p) => IdlType::Option(Box::new(IdlType::from(p))),
            ArbIdlType::Array(p, n) => IdlType::Array(Box::new(IdlType::from(p)), *n),
            ArbIdlType::VecOption(p) => {
                IdlType::Vec(Box::new(IdlType::Option(Box::new(IdlType::from(p)))))
            }
            ArbIdlType::OptionVec(p) => {
                IdlType::Option(Box::new(IdlType::Vec(Box::new(IdlType::from(p)))))
            }
        }
    }
}

// ── Instruction model ─────────────────────────────────────────────────────────

/// A single instruction argument: a name and an IDL type.
#[derive(Debug, Clone, Arbitrary)]
pub struct ArbIdlArg {
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub name: String,
    pub ty: ArbIdlType,
}

impl ArbIdlArg {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "name": self.name, "type": self.ty.to_json() })
    }
}

/// A single IDL instruction: a name and 0–20 arguments.
#[derive(Debug, Clone, Arbitrary)]
pub struct ArbIdlInstruction {
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub name: String,
    #[proptest(strategy = "prop::collection::vec(any::<ArbIdlArg>(), 0..=20)")]
    pub args: Vec<ArbIdlArg>,
}

impl ArbIdlInstruction {
    pub fn to_json(&self) -> serde_json::Value {
        let args: Vec<_> = self.args.iter().map(ArbIdlArg::to_json).collect();
        serde_json::json!({ "name": self.name, "accounts": [], "args": args })
    }
}

// ── Top-level IDL ─────────────────────────────────────────────────────────────

/// A full IDL with 1–16 randomly-structured instructions and no defined types.
#[derive(Debug, Arbitrary)]
pub struct ArbIdl {
    #[proptest(strategy = "prop::collection::vec(any::<ArbIdlInstruction>(), 1..=16)")]
    pub instructions: Vec<ArbIdlInstruction>,
}

impl ArbIdl {
    pub fn to_json_string(&self) -> String {
        let insts: Vec<_> = self
            .instructions
            .iter()
            .map(ArbIdlInstruction::to_json)
            .collect();
        serde_json::json!({ "instructions": insts, "types": [] }).to_string()
    }
}

// ── Defined-struct IDL ────────────────────────────────────────────────────────

/// A struct field with a primitive type (no nested defined types, to avoid
/// unbounded recursion in `arb_bytes_for_type`).
#[derive(Debug, Clone, Arbitrary)]
pub struct ArbStructField {
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub name: String,
    pub ty: ArbPrimitive,
}

/// An IDL where one instruction references a defined struct from the `types`
/// array.  Used to exercise the `Defined` type-resolution path.
#[derive(Debug, Arbitrary)]
pub struct ArbDefinedStructIdl {
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub struct_name: String,
    #[proptest(strategy = "prop::collection::vec(any::<ArbStructField>(), 1..=8)")]
    pub fields: Vec<ArbStructField>,
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub inst_name: String,
    /// Extra instructions using primitive args (not the defined type).
    #[proptest(strategy = "prop::collection::vec(any::<ArbIdlInstruction>(), 0..=4)")]
    pub extra_insts: Vec<ArbIdlInstruction>,
}

impl ArbDefinedStructIdl {
    pub fn to_json_string(&self) -> String {
        let field_jsons: Vec<_> = self
            .fields
            .iter()
            .map(|f| serde_json::json!({ "name": f.name, "type": f.ty.to_json() }))
            .collect();
        let main_inst = serde_json::json!({
            "name": self.inst_name,
            "accounts": [],
            "args": [{ "name": "data", "type": { "defined": self.struct_name } }]
        });
        let mut all_insts: Vec<_> = self
            .extra_insts
            .iter()
            .map(ArbIdlInstruction::to_json)
            .collect();
        all_insts.push(main_inst);
        serde_json::json!({
            "instructions": all_insts,
            "types": [{
                "name": self.struct_name,
                "type": { "kind": "struct", "fields": field_jsons }
            }]
        })
        .to_string()
    }
}

// ── Vec-arg IDL ───────────────────────────────────────────────────────────────

/// An IDL with exactly one instruction that has a single `Vec<T>` argument.
/// Used to stress-test the SizeGuard against large length-prefix claims.
#[derive(Debug, Arbitrary)]
pub struct ArbVecArgIdl {
    #[proptest(regex = "[a-z][a-z0-9]{1,15}")]
    pub inst_name: String,
    pub elem_type: ArbIdlType,
}

impl ArbVecArgIdl {
    pub fn to_json_string(&self) -> String {
        serde_json::json!({
            "instructions": [{
                "name": self.inst_name,
                "accounts": [],
                "args": [{ "name": "data", "type": { "vec": self.elem_type.to_json() } }]
            }],
            "types": []
        })
        .to_string()
    }
}

// ── Conversion unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arb_primitive_from_covers_all_idl_type_primitives() {
        assert!(matches!(IdlType::from(&ArbPrimitive::Bool), IdlType::Bool));
        assert!(matches!(IdlType::from(&ArbPrimitive::U8), IdlType::U8));
        assert!(matches!(IdlType::from(&ArbPrimitive::U16), IdlType::U16));
        assert!(matches!(IdlType::from(&ArbPrimitive::U32), IdlType::U32));
        assert!(matches!(IdlType::from(&ArbPrimitive::U64), IdlType::U64));
        assert!(matches!(IdlType::from(&ArbPrimitive::U128), IdlType::U128));
        assert!(matches!(IdlType::from(&ArbPrimitive::I8), IdlType::I8));
        assert!(matches!(IdlType::from(&ArbPrimitive::I16), IdlType::I16));
        assert!(matches!(IdlType::from(&ArbPrimitive::I32), IdlType::I32));
        assert!(matches!(IdlType::from(&ArbPrimitive::I64), IdlType::I64));
        assert!(matches!(IdlType::from(&ArbPrimitive::I128), IdlType::I128));
        assert!(matches!(IdlType::from(&ArbPrimitive::F32), IdlType::F32));
        assert!(matches!(IdlType::from(&ArbPrimitive::F64), IdlType::F64));
        assert!(matches!(
            IdlType::from(&ArbPrimitive::PublicKey),
            IdlType::PublicKey
        ));
        assert!(matches!(
            IdlType::from(&ArbPrimitive::String),
            IdlType::String
        ));
        assert!(matches!(
            IdlType::from(&ArbPrimitive::Bytes),
            IdlType::Bytes
        ));
    }

    #[test]
    fn arb_idl_type_from_covers_all_container_variants() {
        let p = ArbPrimitive::U64;

        assert!(matches!(
            IdlType::from(&ArbIdlType::Primitive(p.clone())),
            IdlType::U64
        ));
        assert!(matches!(
            IdlType::from(&ArbIdlType::Vec(p.clone())),
            IdlType::Vec(inner) if matches!(*inner, IdlType::U64)
        ));
        assert!(matches!(
            IdlType::from(&ArbIdlType::Option(p.clone())),
            IdlType::Option(inner) if matches!(*inner, IdlType::U64)
        ));
        assert!(matches!(
            IdlType::from(&ArbIdlType::Array(p.clone(), 3)),
            IdlType::Array(inner, 3) if matches!(*inner, IdlType::U64)
        ));
        assert!(matches!(
            IdlType::from(&ArbIdlType::VecOption(p.clone())),
            IdlType::Vec(outer) if matches!(*outer, IdlType::Option(ref inner) if matches!(**inner, IdlType::U64))
        ));
        assert!(matches!(
            IdlType::from(&ArbIdlType::OptionVec(p.clone())),
            IdlType::Option(outer) if matches!(*outer, IdlType::Vec(ref inner) if matches!(**inner, IdlType::U64))
        ));
    }
}
