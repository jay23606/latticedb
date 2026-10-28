//! State-based CRDTs and the laws they must satisfy: merge is commutative, associative and
//! idempotent. Text sequences arrive later (Phase 1 ADR decides build or adopt).

use lattice_abi::Hlc;
use std::collections::{BTreeMap, BTreeSet};

/// A state-based CRDT.
pub trait Crdt: Clone + PartialEq {
    /// Merges `other` into `self`. Must be commutative, associative and idempotent.
    fn merge(&mut self, other: &Self);
}

/// Last-writer-wins register ordered by hybrid logical clock; ties are impossible because the
/// timestamp includes the node id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwwRegister<T> {
    value: Option<(Hlc, T)>,
}

impl<T> Default for LwwRegister<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<T: Clone + Ord> LwwRegister<T> {
    /// Writes a value at `at`; an older write than the current one is ignored.
    pub fn set(&mut self, at: Hlc, value: T) {
        // Equal timestamps (only possible with forged input) fall back to the value, so merge
        // stays commutative.
        let newer = self
            .value
            .as_ref()
            .is_none_or(|(t, v)| at > *t || (at == *t && value > *v));
        if newer {
            self.value = Some((at, value));
        }
    }

    /// The current value.
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref().map(|(_, v)| v)
    }
}

impl<T: Clone + Ord> Crdt for LwwRegister<T> {
    fn merge(&mut self, other: &Self) {
        if let Some((t, v)) = &other.value {
            self.set(*t, v.clone());
        }
    }
}

/// Observed-remove set: an element is present when some add tag has not been removed. A remove
/// only cancels the adds it has seen, so a concurrent add wins ("add wins").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrSet<T: Ord> {
    adds: BTreeMap<T, BTreeSet<Hlc>>,
    removed: BTreeSet<Hlc>,
}

impl<T: Ord + Clone> OrSet<T> {
    /// Adds `value` with a unique tag.
    pub fn add(&mut self, value: T, tag: Hlc) {
        self.adds.entry(value).or_default().insert(tag);
    }

    /// Removes `value`, cancelling every add tag currently visible.
    pub fn remove(&mut self, value: &T) {
        if let Some(tags) = self.adds.get(value) {
            self.removed.extend(tags.iter().copied());
        }
    }

    /// Whether `value` is present.
    pub fn contains(&self, value: &T) -> bool {
        self.adds
            .get(value)
            .is_some_and(|t| t.iter().any(|x| !self.removed.contains(x)))
    }

    /// The present elements in order.
    pub fn elements(&self) -> Vec<T> {
        self.adds
            .keys()
            .filter(|k| self.contains(k))
            .cloned()
            .collect()
    }
}

impl<T: Ord + Clone> Crdt for OrSet<T> {
    fn merge(&mut self, other: &Self) {
        for (k, tags) in &other.adds {
            self.adds
                .entry(k.clone())
                .or_default()
                .extend(tags.iter().copied());
        }
        self.removed.extend(other.removed.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn arb_hlc() -> impl Strategy<Value = Hlc> {
        (0u64..50, 0u32..3, 0u32..3).prop_map(|(w, c, n)| Hlc {
            wall_ms: w,
            counter: c,
            node: n,
        })
    }

    fn arb_lww() -> impl Strategy<Value = LwwRegister<u8>> {
        proptest::collection::vec((arb_hlc(), any::<u8>()), 0..4).prop_map(|ops| {
            let mut r = LwwRegister::default();
            for (t, v) in ops {
                r.set(t, v);
            }
            r
        })
    }

    #[derive(Debug, Clone)]
    enum SetOp {
        Add(u8, Hlc),
        Remove(u8),
    }

    fn arb_orset() -> impl Strategy<Value = OrSet<u8>> {
        proptest::collection::vec(
            prop_oneof![
                (0u8..4, arb_hlc()).prop_map(|(v, t)| SetOp::Add(v, t)),
                (0u8..4).prop_map(SetOp::Remove)
            ],
            0..8,
        )
        .prop_map(|ops| {
            let mut s = OrSet::default();
            for op in ops {
                match op {
                    SetOp::Add(v, t) => s.add(v, t),
                    SetOp::Remove(v) => s.remove(&v),
                }
            }
            s
        })
    }

    fn merged<C: Crdt>(a: &C, b: &C) -> C {
        let mut m = a.clone();
        m.merge(b);
        m
    }

    proptest! {
        #[test]
        fn lww_merge_is_commutative_associative_idempotent(a in arb_lww(), b in arb_lww(), c in arb_lww()) {
            prop_assert_eq!(merged(&a, &b), merged(&b, &a));
            prop_assert_eq!(merged(&merged(&a, &b), &c), merged(&a, &merged(&b, &c)));
            prop_assert_eq!(merged(&a, &a), a);
        }

        #[test]
        fn orset_merge_is_commutative_associative_idempotent(a in arb_orset(), b in arb_orset(), c in arb_orset()) {
            prop_assert_eq!(merged(&a, &b), merged(&b, &a));
            prop_assert_eq!(merged(&merged(&a, &b), &c), merged(&a, &merged(&b, &c)));
            prop_assert_eq!(merged(&a, &a), a);
        }

        #[test]
        fn replicas_converge_in_any_delivery_order(a in arb_orset(), b in arb_orset(), c in arb_orset()) {
            let mut r1 = a.clone(); r1.merge(&b); r1.merge(&c);
            let mut r2 = c.clone(); r2.merge(&a); r2.merge(&b);
            let mut r3 = b.clone(); r3.merge(&c); r3.merge(&a); r3.merge(&a);
            prop_assert_eq!(r1.elements(), r2.elements());
            prop_assert_eq!(r2.elements(), r3.elements());
        }
    }

    #[test]
    fn add_wins_over_a_concurrent_remove() {
        let t1 = Hlc {
            wall_ms: 1,
            counter: 0,
            node: 1,
        };
        let t2 = Hlc {
            wall_ms: 1,
            counter: 0,
            node: 2,
        };
        let mut base = OrSet::default();
        base.add("x", t1);
        let mut a = base.clone();
        a.remove(&"x");
        let mut b = base.clone();
        b.add("x", t2);
        a.merge(&b);
        assert!(a.contains(&"x"));
    }

    #[test]
    fn lww_newest_write_wins() {
        let mut r = LwwRegister::default();
        r.set(
            Hlc {
                wall_ms: 5,
                counter: 0,
                node: 1,
            },
            "new",
        );
        r.set(
            Hlc {
                wall_ms: 3,
                counter: 0,
                node: 1,
            },
            "old",
        );
        assert_eq!(r.get(), Some(&"new"));
    }
}
