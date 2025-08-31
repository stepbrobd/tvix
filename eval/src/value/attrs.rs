//! This module implements Nix attribute sets, backed by Rust hash maps.
use std::borrow::Borrow;
use std::collections::hash_map;
use std::hash::Hash;
use std::iter::FromIterator;
use std::rc::Rc;

use itertools::Itertools as _;
use rustc_hash::FxHashMap;
use serde::de::{Deserializer, Error, Visitor};
use serde::Deserialize;

use super::string::NixString;
use super::thunk::ThunkSet;
use super::TotalDisplay;
use super::Value;
use crate::errors::ErrorKind;
use crate::CatchableErrorKind;

#[cfg(test)]
mod tests;

type AttrsRep = FxHashMap<NixString, Value>;

#[repr(transparent)]
#[derive(Clone, Debug, Default)]
pub struct NixAttrs(Rc<AttrsRep>);

impl From<AttrsRep> for NixAttrs {
    fn from(rep: AttrsRep) -> Self {
        NixAttrs(Rc::new(rep))
    }
}

impl<K, V> FromIterator<(K, V)> for NixAttrs
where
    NixString: From<K>,
    Value: From<V>,
{
    fn from_iter<T>(iter: T) -> NixAttrs
    where
        T: IntoIterator<Item = (K, V)>,
    {
        iter.into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect::<AttrsRep>()
            .into()
    }
}

impl TotalDisplay for NixAttrs {
    fn total_fmt(&self, f: &mut std::fmt::Formatter<'_>, set: &mut ThunkSet) -> std::fmt::Result {
        if let Some(Value::String(s)) = self.select_str("type") {
            if *s == "derivation" {
                write!(f, "«derivation ")?;
                if let Some(p) = self.select_str("drvPath") {
                    p.total_fmt(f, set)?;
                } else {
                    write!(f, "???")?;
                }
                return write!(f, "»");
            }
        }

        f.write_str("{ ")?;

        for (name, value) in self.iter_sorted() {
            write!(f, "{} = ", name.ident_str())?;
            value.total_fmt(f, set)?;
            f.write_str("; ")?;
        }

        f.write_str("}")
    }
}

impl<'de> Deserialize<'de> for NixAttrs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = NixAttrs;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a valid Nix attribute set")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut stack_array = Vec::with_capacity(map.size_hint().unwrap_or(0) * 2);

                while let Some((key, value)) = map.next_entry()? {
                    stack_array.push(key);
                    stack_array.push(value);
                }

                Ok(NixAttrs::construct(stack_array.len() / 2, stack_array)
                    .map_err(A::Error::custom)?
                    .expect("Catchable values are unreachable here"))
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }
}

impl NixAttrs {
    pub fn empty() -> Self {
        AttrsRep::default().into()
    }

    /// Compare two attribute sets by pointer equality, but returning `false`
    /// does not mean that the two attribute sets do not have equal content.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Return an attribute set containing the merge of the two
    /// provided sets. Keys from the `other` set have precedence.
    pub fn update(self, other: Self) -> Self {
        let mut out = Rc::unwrap_or_clone(self.0);
        for (key, value) in other {
            out.insert(key, value);
        }

        out.into()
    }

    /// Return the number of key-value entries in an attrset.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Select a value from an attribute set by key.
    pub fn select<Q>(&self, key: &Q) -> Option<&Value>
    where
        NixString: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.0.get(key)
    }

    /// Select a value from an attribute set by a key in string format. This is
    /// separated out to avoid unintended copies, as the NixString
    /// representation is not guaranteed to be valid UTF-8 and doesn't fit the
    /// usual `Borrow` trick.
    pub fn select_str(&self, key: &str) -> Option<&Value> {
        self.select(key.as_bytes())
    }

    /// Select a required value from an attribute set by key, return
    /// an `AttributeNotFound` error if it is missing.
    pub fn select_required(&self, key: &str) -> Result<&Value, ErrorKind> {
        self.0
            .get(key.as_bytes())
            .ok_or_else(|| ErrorKind::AttributeNotFound {
                name: key.to_string(),
            })
    }

    pub fn contains<Q>(&self, key: &Q) -> bool
    where
        NixString: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.0.contains_key(key)
    }

    /// Construct an iterator over all the key-value pairs in the attribute set.
    #[allow(clippy::needless_lifetimes)]
    pub fn iter<'a>(&'a self) -> Iter<KeyValue<'a>> {
        Iter(KeyValue::Map(self.0.iter()))
    }

    /// Construct an iterator over all the key-value pairs in lexicographic
    /// order of their keys.
    pub fn iter_sorted(&self) -> Iter<KeyValue<'_>> {
        let sorted = self.0.iter().sorted_by_key(|x| x.0);
        Iter(KeyValue::Sorted(sorted))
    }

    /// Same as [IntoIterator::into_iter], but marks call sites which rely on the
    /// iteration being lexicographic.
    pub fn into_iter_sorted(self) -> OwnedAttrsIterator {
        OwnedAttrsIterator(IntoIterRepr::Finite(
            self.0
                .as_ref()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .sorted_by(|(a, _), (b, _)| a.cmp(b)),
        ))
    }

    /// Construct an iterator over all the keys of the attribute set
    pub fn keys(&self) -> Keys<'_> {
        Keys(KeysInner::Map(self.0.keys()))
    }

    /// Same as [Self::keys], but marks call sites which rely on the
    /// iteration being lexicographic.
    pub fn keys_sorted(&self) -> Keys<'_> {
        Keys(KeysInner::Sorted(self.0.keys().sorted()))
    }

    /// Implement construction logic of an attribute set, to encapsulate
    /// logic about attribute set optimisations inside of this module.
    pub fn construct(
        count: usize,
        mut stack_slice: Vec<Value>,
    ) -> Result<Result<Self, CatchableErrorKind>, ErrorKind> {
        debug_assert!(
            stack_slice.len() == count * 2,
            "construct_attrs called with count == {}, but slice.len() == {}",
            count,
            stack_slice.len(),
        );

        let mut attrs_map = FxHashMap::with_capacity_and_hasher(count, rustc_hash::FxBuildHasher);

        for _ in 0..count {
            let value = stack_slice.pop().unwrap();
            let key = stack_slice.pop().unwrap();

            match key {
                Value::String(ks) => set_attr(&mut attrs_map, ks, value)?,

                Value::Null => {
                    // This is in fact valid, but leads to the value being
                    // ignored and nothing being set, i.e. `{ ${null} = 1; } =>
                    // { }`.
                    continue;
                }

                Value::Catchable(err) => return Ok(Err(*err)),

                other => return Err(ErrorKind::InvalidAttributeName(other)),
            }
        }

        Ok(Ok(attrs_map.into()))
    }

    /// Calculate the intersection of the attribute sets.
    /// The right side value is used when the keys match.
    pub(crate) fn intersect(&self, other: &Self) -> NixAttrs {
        let lhs = &self.0;
        let rhs = &other.0;

        let mut out = FxHashMap::with_capacity_and_hasher(
            std::cmp::min(lhs.len(), rhs.len()),
            rustc_hash::FxBuildHasher,
        );

        if lhs.len() < rhs.len() {
            for key in lhs.keys() {
                if let Some(val) = rhs.get(key) {
                    out.insert(key.clone(), val.clone());
                }
            }
        } else {
            for (key, val) in rhs.iter() {
                if lhs.contains_key(key) {
                    out.insert(key.clone(), val.clone());
                }
            }
        };

        out.into()
    }
}

impl IntoIterator for NixAttrs {
    type Item = (NixString, Value);
    type IntoIter = OwnedAttrsIterator;

    fn into_iter(self) -> Self::IntoIter {
        OwnedAttrsIterator(IntoIterRepr::Map(Rc::unwrap_or_clone(self.0).into_iter()))
    }
}

/// Set an attribute on an in-construction attribute set, while
/// checking against duplicate keys.
fn set_attr(map: &mut AttrsRep, key: NixString, value: Value) -> Result<(), ErrorKind> {
    match map.entry(key) {
        hash_map::Entry::Occupied(entry) => Err(ErrorKind::DuplicateAttrsKey {
            key: entry.key().to_string(),
        }),

        hash_map::Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
    }
}

/// Iterator representation over the keys *and* values of an attribute
/// set.
pub enum KeyValue<'a> {
    Map(hash_map::Iter<'a, NixString, Value>),
    Sorted(std::vec::IntoIter<(&'a NixString, &'a Value)>),
}

/// Iterator over a Nix attribute set.
// This wrapper type exists to make the inner "raw" iterator
// inaccessible.
#[repr(transparent)]
pub struct Iter<T>(T);

impl<'a> Iterator for Iter<KeyValue<'a>> {
    type Item = (&'a NixString, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            KeyValue::Map(inner) => inner.next(),
            KeyValue::Sorted(inner) => inner.next(),
        }
    }
}

impl ExactSizeIterator for Iter<KeyValue<'_>> {
    fn len(&self) -> usize {
        match &self.0 {
            KeyValue::Map(inner) => inner.len(),
            KeyValue::Sorted(inner) => inner.len(),
        }
    }
}

enum KeysInner<'a> {
    Map(hash_map::Keys<'a, NixString, Value>),
    Sorted(std::vec::IntoIter<&'a NixString>),
}

pub struct Keys<'a>(KeysInner<'a>);

impl<'a> Iterator for Keys<'a> {
    type Item = &'a NixString;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            KeysInner::Map(m) => m.next(),
            KeysInner::Sorted(v) => v.next(),
        }
    }
}

impl<'a> IntoIterator for &'a NixAttrs {
    type Item = (&'a NixString, &'a Value);

    type IntoIter = Iter<KeyValue<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl ExactSizeIterator for Keys<'_> {
    fn len(&self) -> usize {
        match &self.0 {
            KeysInner::Map(m) => m.len(),
            KeysInner::Sorted(v) => v.len(),
        }
    }
}

/// Internal representation of an owning attrset iterator
pub enum IntoIterRepr {
    Finite(std::vec::IntoIter<(NixString, Value)>),
    Map(hash_map::IntoIter<NixString, Value>),
}

/// Wrapper type which hides the internal implementation details from
/// users.
#[repr(transparent)]
pub struct OwnedAttrsIterator(IntoIterRepr);

impl Iterator for OwnedAttrsIterator {
    type Item = (NixString, Value);

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            IntoIterRepr::Finite(inner) => inner.next(),
            IntoIterRepr::Map(m) => m.next(),
        }
    }
}

impl ExactSizeIterator for OwnedAttrsIterator {
    fn len(&self) -> usize {
        match &self.0 {
            IntoIterRepr::Finite(inner) => inner.len(),
            IntoIterRepr::Map(inner) => inner.len(),
        }
    }
}

impl DoubleEndedIterator for OwnedAttrsIterator {
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            IntoIterRepr::Finite(inner) => inner.next_back(),
            // hashmaps have arbitary iteration order, so reversing it would not make a difference
            IntoIterRepr::Map(inner) => inner.next(),
        }
    }
}
