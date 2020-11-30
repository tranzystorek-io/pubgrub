// SPDX-License-Identifier: MPL-2.0

//! Ranges are constraints defining sets of versions.
//!
//! Concretely, those constraints correspond to any set of versions
//! representable as the concatenation, union, and complement
//! of the ranges building blocks.
//!
//! Those building blocks are:
//!  - [none()](Range::none): the empty set
//!  - [any()](Range::any): the set of all possible versions
//!  - [exact(v)](Range::exact): the set containing only the version v
//!  - [higher_than(v)](Range::higher_than): the set defined by `v <= versions`
//!  - [strictly_lower_than(v)](Range::strictly_lower_than): the set defined by `versions < v`
//!  - [between(v1, v2)](Range::between): the set defined by `v1 <= versions < v2`

use std::cmp::Ordering;
use std::fmt;

pub trait RangeSet: fmt::Debug + fmt::Display + Clone + Eq {
    type VERSION: Clone + Ord + fmt::Debug + fmt::Display;

    /// The empty set.
    fn none() -> Self;

    /// Set of all possible versions.
    fn any() -> Self;

    /// Set containing exactly one version.
    fn exact(v: impl Into<Self::VERSION>) -> Self;

    /// Compute the complement set of versions.
    fn negate(&self) -> Self;

    /// Compute the intersection of two sets of versions.
    fn intersection(&self, other: &Self) -> Self;

    /// Check if a range contains a given version.
    fn contains(&self, version: &Self::VERSION) -> bool {
        let exact = Self::exact(version.clone());
        self.intersection(&exact) == exact
    }

    /// Compute the union of two sets of versions.
    fn union(&self, other: &Self) -> Self {
        (self.negate().intersection(&other.negate())).negate()
    }
}

/// A Range is a set of versions.
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Range<V: crate::version::RangeVersion> {
    segments: Vec<Interval<V>>,
}

type Interval<V> = (V, Option<V>);

impl<V: crate::version::RangeVersion> RangeSet for Range<V> {
    type VERSION = V;

    /// Empty set of versions.
    fn none() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Set of all possible versions.
    fn any() -> Self {
        Self::higher_than(V::lowest())
    }

    /// Set of all possible versions.
    fn exact(v: impl Into<V>) -> Self {
        let v = v.into();
        Self {
            segments: vec![(v.clone(), Some(v.bump()))],
        }
    }

    /// Check if a range contains a given version.
    fn contains(&self, version: &V) -> bool {
        for (v1, some_v2) in self.segments.iter() {
            match some_v2 {
                None => return v1 <= version,
                Some(v2) => {
                    if version < v1 {
                        return false;
                    } else if version < v2 {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Compute the complement set of versions.
    fn negate(&self) -> Self {
        match self.segments.as_slice().first() {
            None => Self::any(), // Complement of ∅  is *

            // First high bound is +∞
            Some((v, None)) => {
                // Complement of * is ∅
                if v == &V::lowest() {
                    Self::none()
                // Complement of "v <= _" is "_ < v"
                } else {
                    Self::strictly_lower_than(v.clone())
                }
            }

            // First high bound is not +∞
            Some((v1, Some(v2))) => {
                if v1 == &V::lowest() {
                    Self {
                        segments: Self::negate_segments(v2.clone(), &self.segments[1..]),
                    }
                } else {
                    Self {
                        segments: Self::negate_segments(V::lowest(), &self.segments),
                    }
                }
            }
        }
    }

    /// Compute the intersection of two sets of versions.
    fn intersection(&self, other: &Self) -> Self {
        Self {
            segments: Self::intersection_segments(&self.segments, &other.segments),
        }
    }
}

// Range building blocks.
impl<V: crate::version::RangeVersion> Range<V> {
    /// Set of all possible versions.
    pub fn any() -> Self {
        Self::higher_than(V::lowest())
    }

    /// Set of all versions higher or equal to some version.
    pub fn higher_than(v: impl Into<V>) -> Self {
        Self {
            segments: vec![(v.into(), None)],
        }
    }

    /// Set of all versions strictly lower than some version.
    pub fn strictly_lower_than(v: impl Into<V>) -> Self {
        let v = v.into();
        if v == V::lowest() {
            Self::none()
        } else {
            Self {
                segments: vec![(V::lowest(), Some(v))],
            }
        }
    }

    /// Set of all versions comprised between two given versions.
    /// The lower bound is included and the higher bound excluded.
    /// `v1 <= v < v2`.
    pub fn between(v1: impl Into<V>, v2: impl Into<V>) -> Self {
        let v1 = v1.into();
        let v2 = v2.into();
        if v1 < v2 {
            Self {
                segments: vec![(v1, Some(v2))],
            }
        } else {
            Self::none()
        }
    }
}

// Set operations.
impl<V: crate::version::RangeVersion> Range<V> {
    // Negate ##################################################################

    /// Compute the complement set of versions.
    pub fn negate(&self) -> Self {
        match self.segments.as_slice().first() {
            None => Self::any(), // Complement of ∅  is *

            // First high bound is +∞
            Some((v, None)) => {
                // Complement of * is ∅
                if v == &V::lowest() {
                    Self::none()
                // Complement of "v <= _" is "_ < v"
                } else {
                    Self::strictly_lower_than(v.clone())
                }
            }

            // First high bound is not +∞
            Some((v1, Some(v2))) => {
                if v1 == &V::lowest() {
                    Self {
                        segments: Self::negate_segments(v2.clone(), &self.segments[1..]),
                    }
                } else {
                    Self {
                        segments: Self::negate_segments(V::lowest(), &self.segments),
                    }
                }
            }
        }
    }

    /// Helper function performing the negation of intervals in segments.
    /// For example:
    ///    [ (v1, None) ] => [ (start, Some(v1)) ]
    ///    [ (v1, Some(v2)) ] => [ (start, Some(v1)), (v2, None) ]
    fn negate_segments(start: V, segments: &[Interval<V>]) -> Vec<Interval<V>> {
        let mut complement_segments = Vec::with_capacity(1 + segments.len());
        let mut start = Some(start);
        for (v1, some_v2) in segments.iter() {
            // start.unwrap() is fine because `segments` is not exposed,
            // and our usage guaranties that only the last segment may contain a None.
            complement_segments.push((start.unwrap(), Some(v1.to_owned())));
            start = some_v2.to_owned();
        }
        if let Some(last) = start {
            complement_segments.push((last, None));
        }
        complement_segments
    }

    /// Helper function performing intersection of two interval segments.
    fn intersection_segments(s1: &[Interval<V>], s2: &[Interval<V>]) -> Vec<Interval<V>> {
        let mut segments = Vec::with_capacity(s1.len().min(s2.len()));
        let mut left_iter = s1.iter();
        let mut right_iter = s2.iter();
        let mut left = left_iter.next();
        let mut right = right_iter.next();
        loop {
            match (left, right) {
                // Both left and right still contain a finite interval:
                (Some((l1, Some(l2))), Some((r1, Some(r2)))) => {
                    if l2 <= r1 {
                        // Intervals are disjoint, progress on the left.
                        left = left_iter.next();
                    } else if r2 <= l1 {
                        // Intervals are disjoint, progress on the right.
                        right = right_iter.next();
                    } else {
                        // Intervals are not disjoint.
                        let start = l1.max(r1).to_owned();
                        if l2 < r2 {
                            segments.push((start, Some(l2.to_owned())));
                            left = left_iter.next();
                        } else {
                            segments.push((start, Some(r2.to_owned())));
                            right = right_iter.next();
                        }
                    }
                }

                // Right contains an infinite interval:
                (Some((l1, Some(l2))), Some((r1, None))) => match l2.cmp(r1) {
                    Ordering::Less => {
                        left = left_iter.next();
                    }
                    Ordering::Equal => {
                        segments.extend(left_iter.cloned());
                        break;
                    }
                    Ordering::Greater => {
                        let start = l1.max(r1).to_owned();
                        segments.push((start, Some(l2.to_owned())));
                        segments.extend(left_iter.cloned());
                        break;
                    }
                },

                // Left contains an infinite interval:
                (Some((l1, None)), Some((r1, Some(r2)))) => match r2.cmp(l1) {
                    Ordering::Less => {
                        right = right_iter.next();
                    }
                    Ordering::Equal => {
                        segments.extend(right_iter.cloned());
                        break;
                    }
                    Ordering::Greater => {
                        let start = l1.max(r1).to_owned();
                        segments.push((start, Some(r2.to_owned())));
                        segments.extend(right_iter.cloned());
                        break;
                    }
                },

                // Both sides contain an infinite interval:
                (Some((l1, None)), Some((r1, None))) => {
                    let start = l1.max(r1).to_owned();
                    segments.push((start, None));
                    break;
                }

                // Left or right has ended.
                _ => {
                    break;
                }
            }
        }
        segments
    }
}

// Other useful functions.
impl<V: crate::version::RangeVersion> Range<V> {
    /// Return the lowest version in the range (if there is one).
    pub fn lowest_version(&self) -> Option<V> {
        self.segments
            .as_slice()
            .first()
            .map(|(start, _)| start)
            .cloned()
    }
}

// REPORT ######################################################################

impl<V: crate::version::RangeVersion> fmt::Display for Range<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.segments.as_slice() {
            [] => write!(f, "∅"),
            [(start, None)] if start == &V::lowest() => write!(f, "∗"),
            [(start, None)] => write!(f, "{} <= v", start),
            [(start, Some(end))] if end == &start.bump() => write!(f, "{}", start),
            [(start, Some(end))] if start == &V::lowest() => write!(f, "v < {}", end),
            [(start, Some(end))] => write!(f, "{} <= v < {}", start, end),
            more_than_one_interval => {
                let string_intervals: Vec<_> = more_than_one_interval
                    .iter()
                    .map(interval_to_string)
                    .collect();
                write!(f, "{}", string_intervals.join("  "))
            }
        }
    }
}

fn interval_to_string<V: crate::version::RangeVersion>(interval: &Interval<V>) -> String {
    match interval {
        (start, Some(end)) => format!("[ {}, {} [", start, end),
        (start, None) => format!("[ {}, ∞ [", start),
    }
}

// TESTS #######################################################################

#[cfg(test)]
pub mod tests {
    use proptest::prelude::*;

    use crate::version::NumberVersion;

    use super::*;

    pub fn strategy() -> impl Strategy<Value = Range<NumberVersion>> {
        prop::collection::vec(any::<u32>(), 0..10).prop_map(|mut vec| {
            vec.sort_unstable();
            vec.dedup();
            let mut pair_iter = vec.chunks_exact(2);
            let mut segments = Vec::with_capacity(vec.len() / 2 + 1);
            while let Some([v1, v2]) = pair_iter.next() {
                segments.push((NumberVersion(*v1), Some(NumberVersion(*v2))));
            }
            if let [v] = pair_iter.remainder() {
                segments.push((NumberVersion(*v), None));
            }
            Range { segments }
        })
    }

    fn version_strat() -> impl Strategy<Value = NumberVersion> {
        any::<u32>().prop_map(NumberVersion)
    }

    proptest! {

        // Testing negate ----------------------------------

        #[test]
        fn negate_is_different(range in strategy()) {
            assert_ne!(range.negate(), range);
        }

        #[test]
        fn double_negate_is_identity(range in strategy()) {
            assert_eq!(range.negate().negate(), range);
        }

        #[test]
        fn negate_contains_opposite(range in strategy(), version in version_strat()) {
            assert_ne!(range.contains(&version), range.negate().contains(&version));
        }

        // Testing intersection ----------------------------

        #[test]
        fn intersection_is_symmetric(r1 in strategy(), r2 in strategy()) {
            assert_eq!(r1.intersection(&r2), r2.intersection(&r1));
        }

        #[test]
        fn intersection_with_any_is_identity(range in strategy()) {
            assert_eq!(Range::any().intersection(&range), range);
        }

        #[test]
        fn intersection_with_none_is_none(range in strategy()) {
            assert_eq!(Range::none().intersection(&range), Range::none());
        }

        #[test]
        fn intersection_is_idempotent(r1 in strategy(), r2 in strategy()) {
            assert_eq!(r1.intersection(&r2).intersection(&r2), r1.intersection(&r2));
        }

        #[test]
        fn intersection_is_associative(r1 in strategy(), r2 in strategy(), r3 in strategy()) {
            assert_eq!(r1.intersection(&r2).intersection(&r3), r1.intersection(&r2.intersection(&r3)));
        }

        #[test]
        fn intesection_of_complements_is_none(range in strategy()) {
            assert_eq!(range.negate().intersection(&range), Range::none());
        }

        #[test]
        fn intesection_contains_both(r1 in strategy(), r2 in strategy(), version in version_strat()) {
            assert_eq!(r1.intersection(&r2).contains(&version), r1.contains(&version) && r2.contains(&version));
        }

        // Testing union -----------------------------------

        #[test]
        fn union_of_complements_is_any(range in strategy()) {
            assert_eq!(range.negate().union(&range), Range::any());
        }

        #[test]
        fn union_contains_either(r1 in strategy(), r2 in strategy(), version in version_strat()) {
            assert_eq!(r1.union(&r2).contains(&version), r1.contains(&version) || r2.contains(&version));
        }

        // Testing contains --------------------------------

        #[test]
        fn always_contains_exact(version in version_strat()) {
            assert!(Range::exact(version).contains(&version));
        }

        #[test]
        fn contains_negation(range in strategy(), version in version_strat()) {
            assert_ne!(range.contains(&version), range.negate().contains(&version));
        }

        #[test]
        fn contains_intersection(range in strategy(), version in version_strat()) {
            assert_eq!(range.contains(&version), range.intersection(&Range::exact(version)) != Range::none());
        }
    }
}
