//! Shared `#[derive(Arbitrary)]` model types for IDL-based property tests.
//!
//! These types mirror the JSON shape expected by `decode_idl_data` / the Anchor
//! IDL format.  Every type derives `proptest::arbitrary::Arbitrary` (via
//! `proptest-derive`) so that tests can write `any::<ArbIdl>()` instead of
//! composing manual proptest strategy functions.
//!
//! Each type exposes a `to_json()` / `to_json_string()` method that produces
//! the JSON consumed by `decode_idl_data`.

#![allow(dead_code)]

use proptest::prelude::*;
use proptest_derive::Arbitrary;

// ── Primitive types ───────────────────────────────────────────────────────────

/// All IDL primitive types in their Anchor JSON wire format.
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

// ── Container / composite types ───────────────────────────────────────────────

/// IDL field type: a primitive or a single-level container wrapping a
/// primitive.
///
/// Weights: 4 Primitive : 1 Vec : 1 Option : 1 Array : 1 VecOption :
///          1 OptionVec — matching the original manual `arb_idl_type()`.
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
        let insts: Vec<_> = self.instructions.iter().map(ArbIdlInstruction::to_json).collect();
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
        let mut all_insts: Vec<_> =
            self.extra_insts.iter().map(ArbIdlInstruction::to_json).collect();
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
