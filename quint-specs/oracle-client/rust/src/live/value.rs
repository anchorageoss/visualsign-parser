//! Conversion of Rust values into the ITF JSON dialect the oracle parses.
//!
//! `itf::Value` is the canonical representation; its serde serialization is
//! exactly what the daemon deserializes, so this module owns no encoding of
//! its own — only the bridge from Rust types onto it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use itf::Value;

/// Conversion into the logged ITF value dialect.
pub trait ToLogged {
    fn to_logged(&self) -> Value;
}

impl ToLogged for Value {
    fn to_logged(&self) -> Value {
        self.clone()
    }
}

impl<T: ToLogged + ?Sized> ToLogged for &T {
    fn to_logged(&self) -> Value {
        (**self).to_logged()
    }
}

impl ToLogged for bool {
    fn to_logged(&self) -> Value {
        Value::Bool(*self)
    }
}

impl ToLogged for str {
    fn to_logged(&self) -> Value {
        Value::String(self.to_string())
    }
}

impl ToLogged for String {
    fn to_logged(&self) -> Value {
        Value::String(self.clone())
    }
}

macro_rules! impl_to_logged_small_int {
    ($($ty:ty),*) => {$(
        impl ToLogged for $ty {
            fn to_logged(&self) -> Value {
                Value::Number(i64::from(*self))
            }
        }
    )*};
}

impl_to_logged_small_int!(i8, i16, i32, i64, u8, u16, u32);

macro_rules! impl_to_logged_wide_int {
    ($($ty:ty),*) => {$(
        impl ToLogged for $ty {
            fn to_logged(&self) -> Value {
                match i64::try_from(*self) {
                    Ok(n) => Value::Number(n),
                    Err(_) => Value::BigInt(itf::value::BigInt::new(*self)),
                }
            }
        }
    )*};
}

impl_to_logged_wide_int!(isize, usize, u64, i128, u128);

// A Rust tuple maps straight onto the Quint tuple, so it needs no constructor
// as long as every component converts. Arity 2..=12: Quint has no 1-tuple, and
// 12 is where std's own tuple impls stop; `tuple()` covers anything wider.
macro_rules! impl_to_logged_tuple {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($name: ToLogged),+> ToLogged for ($($name,)+) {
            fn to_logged(&self) -> Value {
                let ($($name,)+) = self;
                tuple([$($name.to_logged()),+])
            }
        }
    };
}

impl_to_logged_tuple!(A, B);
impl_to_logged_tuple!(A, B, C);
impl_to_logged_tuple!(A, B, C, D);
impl_to_logged_tuple!(A, B, C, D, E);
impl_to_logged_tuple!(A, B, C, D, E, F);
impl_to_logged_tuple!(A, B, C, D, E, F, G);
impl_to_logged_tuple!(A, B, C, D, E, F, G, H);
impl_to_logged_tuple!(A, B, C, D, E, F, G, H, I);
impl_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

impl<T: ToLogged> ToLogged for [T] {
    fn to_logged(&self) -> Value {
        Value::List(self.iter().map(ToLogged::to_logged).collect())
    }
}

impl<T: ToLogged> ToLogged for Vec<T> {
    fn to_logged(&self) -> Value {
        self.as_slice().to_logged()
    }
}

impl<T: ToLogged> ToLogged for BTreeSet<T> {
    fn to_logged(&self) -> Value {
        Value::Set(self.iter().map(ToLogged::to_logged).collect())
    }
}

impl<K: ToLogged, V: ToLogged> ToLogged for BTreeMap<K, V> {
    fn to_logged(&self) -> Value {
        Value::Map(
            self.iter()
                .map(|(k, v)| (k.to_logged(), v.to_logged()))
                .collect(),
        )
    }
}

// A hash collection's iteration order does not reach the wire: `itf::Set` and
// `itf::Map` sort on collect, whatever order they are fed.
impl<T: ToLogged, S> ToLogged for HashSet<T, S> {
    fn to_logged(&self) -> Value {
        Value::Set(self.iter().map(ToLogged::to_logged).collect())
    }
}

impl<K: ToLogged, V: ToLogged, S> ToLogged for HashMap<K, V, S> {
    fn to_logged(&self) -> Value {
        Value::Map(
            self.iter()
                .map(|(k, v)| (k.to_logged(), v.to_logged()))
                .collect(),
        )
    }
}

/// A record: a JSON object, the form a Quint record takes.
///
/// The automatic conversions cover no Rust type that maps onto a record —
/// build one here, usually from a domain type's [`ToLogged`] impl:
///
/// ```
/// # use quint_oracle::{record, ToLogged, Value};
/// # struct Account { owner: String, balance: u64 }
/// impl ToLogged for Account {
///     fn to_logged(&self) -> Value {
///         record([
///             ("owner", self.owner.to_logged()),
///             ("balance", self.balance.to_logged()),
///         ])
///     }
/// }
/// ```
///
/// A free function rather than a constructor because [`Value`] is `itf::Value`,
/// a foreign type.
pub fn record<N, I>(fields: I) -> Value
where
    N: Into<String>,
    I: IntoIterator<Item = (N, Value)>,
{
    Value::Record(fields.into_iter().map(|(n, v)| (n.into(), v)).collect())
}

/// A tuple: `{"#tup": […]}`.
///
/// Rust tuples up to arity 12 convert on their own, so this is for the rest:
/// a wider tuple, or items assembled at runtime. Like [`record`], a free
/// function over a foreign type.
pub fn tuple<I: IntoIterator<Item = Value>>(items: I) -> Value {
    Value::Tuple(items.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rendering below is pinned against the dialect the daemon parses
    /// (`crates/oracle/src/logged_value.rs` in the quint repo): ints within
    /// i64 must be bare numbers (the spec side serializes them that way and
    /// matching is JSON equality), wider ints take the `#bigint` form, and
    /// collections take the `#set`/`#map` forms.
    fn json(value: Value) -> serde_json::Value {
        serde_json::to_value(value).expect("itf values always serialize")
    }

    #[test]
    fn scalars_render_bare() {
        assert_eq!(json(true.to_logged()), serde_json::json!(true));
        assert_eq!(json(42u8.to_logged()), serde_json::json!(42));
        assert_eq!(json((-7i64).to_logged()), serde_json::json!(-7));
        assert_eq!(json("acc1".to_logged()), serde_json::json!("acc1"));
        assert_eq!(
            json(String::from("acc1").to_logged()),
            serde_json::json!("acc1")
        );
    }

    #[test]
    fn ints_within_i64_stay_bare_numbers_even_from_wide_types() {
        assert_eq!(json(5u64.to_logged()), serde_json::json!(5));
        assert_eq!(json(5u128.to_logged()), serde_json::json!(5));
        assert_eq!(json((-5i128).to_logged()), serde_json::json!(-5));
        assert_eq!(
            json(i64::MAX.to_logged()),
            serde_json::json!(9223372036854775807i64)
        );
    }

    #[test]
    fn ints_beyond_i64_take_the_bigint_form() {
        assert_eq!(
            json(u64::MAX.to_logged()),
            serde_json::json!({"#bigint": "18446744073709551615"})
        );
        assert_eq!(
            json((i128::from(i64::MIN) - 1).to_logged()),
            serde_json::json!({"#bigint": "-9223372036854775809"})
        );
    }

    #[test]
    fn collections_take_the_itf_forms() {
        assert_eq!(
            json(vec![1, 2, 3].to_logged()),
            serde_json::json!([1, 2, 3])
        );
        assert_eq!(
            json(BTreeSet::from(["a", "b"]).to_logged()),
            serde_json::json!({"#set": ["a", "b"]})
        );
        assert_eq!(
            json(BTreeMap::from([("a", 1), ("b", 2)]).to_logged()),
            serde_json::json!({"#map": [["a", 1], ["b", 2]]})
        );
    }

    #[test]
    fn tuples_render_as_tuples() {
        assert_eq!(
            json((1, "a").to_logged()),
            serde_json::json!({"#tup": [1, "a"]})
        );
        // Nesting and the wide arities go through the same impl set.
        assert_eq!(
            json((true, (2u64, vec![3])).to_logged()),
            serde_json::json!({"#tup": [true, {"#tup": [2, [3]]}]})
        );
        assert_eq!(
            json((1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12).to_logged()),
            serde_json::json!({"#tup": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]})
        );
    }

    /// A tuple of components is the same wire form as the hand-built one, so
    /// tuple keys and set elements need no constructor either.
    #[test]
    fn derived_tuples_match_the_hand_built_form() {
        assert_eq!(
            json((1, "a").to_logged()),
            json(tuple([1.to_logged(), "a".to_logged()]))
        );

        let map = BTreeMap::from([((1, 2).to_logged(), "pair".to_logged())]);
        assert_eq!(
            json(map.to_logged()),
            serde_json::json!({"#map": [[{"#tup": [1, 2]}, "pair"]]})
        );
    }

    #[test]
    fn hand_built_forms_render_as_written() {
        assert_eq!(
            json(tuple([1.to_logged(), "a".to_logged()])),
            serde_json::json!({"#tup": [1, "a"]})
        );
        assert_eq!(
            json(record([
                ("owner", "alice".to_logged()),
                ("balance", 900.to_logged()),
            ])),
            serde_json::json!({"owner": "alice", "balance": 900})
        );
        assert_eq!(
            json(record(Vec::<(String, Value)>::new())),
            serde_json::json!({})
        );
        assert_eq!(json(tuple([])), serde_json::json!({"#tup": []}));
    }

    #[test]
    fn hash_collections_convert_like_their_ordered_twins() {
        assert_eq!(
            json(HashSet::from(["b", "a"]).to_logged()),
            json(BTreeSet::from(["a", "b"]).to_logged())
        );
        assert_eq!(
            json(HashMap::from([("b", 2), ("a", 1)]).to_logged()),
            json(BTreeMap::from([("a", 1), ("b", 2)]).to_logged())
        );
    }

    /// Heterogeneous sets and maps need no constructor: `Value` is itself
    /// `ToLogged` and `Ord`, so the existing collection impls take them.
    #[test]
    fn heterogeneous_collections_compose_from_values() {
        let set = BTreeSet::from([1.to_logged(), "a".to_logged()]);
        assert_eq!(json(set.to_logged()), serde_json::json!({"#set": [1, "a"]}));

        let map = BTreeMap::from([
            (tuple([1.to_logged(), 2.to_logged()]), "pair".to_logged()),
            (3.to_logged(), "three".to_logged()),
        ]);
        assert_eq!(
            json(map.to_logged()),
            serde_json::json!({"#map": [[3, "three"], [{"#tup": [1, 2]}, "pair"]]})
        );
    }

    #[test]
    fn references_and_identity_convert() {
        let v = 7.to_logged();
        assert_eq!(json(v.to_logged()), serde_json::json!(7));
        let s = "x";
        assert_eq!(json((&s).to_logged()), serde_json::json!("x"));
    }
}
