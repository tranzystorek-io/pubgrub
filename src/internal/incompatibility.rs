// SPDX-License-Identifier: MPL-2.0

//! An incompatibility is a set of terms for different packages
//! that should never be satisfied all together.

use std::collections::HashSet as Set;
use std::fmt;

use crate::internal::arena::{Arena, Id};
use crate::internal::small_map::SmallMap;
use crate::package::Package;
use crate::range::Range;
use crate::report::{DefaultStringReporter, DerivationTree, Derived, External};
use crate::term::{self, Term};
use crate::version::Version;

/// An incompatibility is a set of terms for different packages
/// that should never be satisfied all together.
/// An incompatibility usually originates from a package dependency.
/// For example, if package A at version 1 depends on package B
/// at version 2, you can never have both terms `A = 1`
/// and `not B = 2` satisfied at the same time in a partial solution.
/// This would mean that we found a solution with package A at version 1
/// but not with package B at version 2.
/// Yet A at version 1 depends on B at version 2 so this is not possible.
/// Therefore, the set `{ A = 1, not B = 2 }` is an incompatibility,
/// defined from dependencies of A at version 1.
///
/// Incompatibilities can also be derived from two other incompatibilities
/// during conflict resolution. More about all this in
/// [PubGrub documentation](https://github.com/dart-lang/pub/blob/master/doc/solver.md#incompatibility).
#[derive(Debug, Clone)]
pub struct Incompatibility<P: Package, V: Version> {
    package_terms: SmallMap<P, Term<V>>,
    kind: Kind<P, V>,
}

/// Type alias of unique identifiers for incompatibilities.
pub type IncompId<P, V> = Id<Incompatibility<P, V>>;

#[derive(Debug, Clone)]
enum Kind<P: Package, V: Version> {
    /// Initial incompatibility aiming at picking the root package for the first decision.
    NotRoot,
    /// There are no versions in the given range for this package.
    NoVersions,
    /// Dependencies of the package are unavailable for versions in that range.
    UnavailableDependencies,
    /// Incompatibility coming from the dependencies of a given package.
    FromDependency,
    /// Derived from two causes. Stores cause ids.
    DerivedFrom(IncompId<P, V>, IncompId<P, V>),
}

/// A type alias for a pair of [Package] and a corresponding [Term].
pub type PackageTerm<P, V> = (P, Term<V>);

/// A Relation describes how a set of terms can be compared to an incompatibility.
/// Typically, the set of terms comes from the partial solution.
#[derive(Eq, PartialEq)]
pub enum Relation<P: Package, V: Version> {
    /// We say that a set of terms S satisfies an incompatibility I
    /// if S satisfies every term in I.
    Satisfied,
    /// We say that S contradicts I
    /// if S contradicts at least one term in I.
    Contradicted(PackageTerm<P, V>),
    /// If S satisfies all but one of I's terms and is inconclusive for the remaining term,
    /// we say S "almost satisfies" I and we call the remaining term the "unsatisfied term".
    AlmostSatisfied(P),
    /// Otherwise, we say that their relation is inconclusive.
    Inconclusive,
}

impl<P: Package, V: Version> Incompatibility<P, V> {
    /// Create the initial "not Root" incompatibility.
    pub fn not_root(package: P, version: V) -> Self {
        Self {
            package_terms: SmallMap::One([(package, Term::Negative(Range::exact(version)))]),
            kind: Kind::NotRoot,
        }
    }

    /// Create an incompatibility to remember
    /// that a given range does not contain any version.
    pub fn no_versions(package: P, term: Term<V>) -> Self {
        assert!(term.is_positive(), "No version should have a positive term");
        Self {
            package_terms: SmallMap::One([(package, term)]),
            kind: Kind::NoVersions,
        }
    }

    /// Create an incompatibility to remember
    /// that a package version is not selectable
    /// because its list of dependencies is unavailable.
    pub fn unavailable_dependencies(package: P, version: V) -> Self {
        let range = Range::exact(version);
        Self {
            package_terms: SmallMap::One([(package, Term::Positive(range))]),
            kind: Kind::UnavailableDependencies,
        }
    }

    /// Build an incompatibility from a given dependency.
    pub fn from_dependency(package: P, version: V, dep: (&P, &Range<V>)) -> Self {
        let range1 = Range::exact(version.clone());
        let (p2, range2) = dep;
        Self {
            package_terms: SmallMap::Two([
                (package.clone(), Term::Positive(range1)),
                (p2.clone(), Term::Negative(range2.clone())),
            ]),
            kind: Kind::FromDependency,
        }
    }

    /// Add this incompatibility into the set of all incompatibilities.
    ///
    /// Pub collapses identical dependencies from adjacent package versions
    /// into individual incompatibilities.
    /// This substantially reduces the total number of incompatibilities
    /// and makes it much easier for Pub to reason about multiple versions of packages at once.
    ///
    /// For example, rather than representing
    /// foo 1.0.0 depends on bar ^1.0.0 and
    /// foo 1.1.0 depends on bar ^1.0.0
    /// as two separate incompatibilities,
    /// they are collapsed together into the single incompatibility {foo ^1.0.0, not bar ^1.0.0}
    /// (provided that no other version of foo exists between 1.0.0 and 2.0.0).
    /// We could collapse them into { foo (1.0.0 ∪ 1.1.0), not bar ^1.0.0 }
    /// without having to check the existence of other versions though.
    /// And it would even keep the same [Kind]: [FromDependencyOf](Kind::FromDependencyOf) foo.
    ///
    /// Here we do the simple stupid thing of just growing the Vec.
    /// TODO: improve this.
    /// It may not be trivial since those incompatibilities
    /// may already have derived others.
    /// Maybe this should not be pursued.
    pub fn merge_into(id: Id<Self>, incompatibilities: &mut Vec<Id<Self>>) {
        incompatibilities.push(id);
    }

    /// Prior cause of two incompatibilities using the rule of resolution.
    pub fn prior_cause(
        incompat: Id<Self>,
        satisfier_cause: Id<Self>,
        package: &P,
        incompatibility_store: &Arena<Self>,
    ) -> Self {
        let kind = Kind::DerivedFrom(incompat, satisfier_cause);
        let mut package_terms = incompatibility_store[incompat].package_terms.clone();
        let t1 = package_terms.remove(package).unwrap();
        let satisfier_cause_terms = &incompatibility_store[satisfier_cause].package_terms;
        package_terms.merge(
            satisfier_cause_terms.iter().filter(|(p, _)| p != &package),
            |t1, t2| Some(t1.intersection(t2)),
        );
        let term = t1.union(satisfier_cause_terms.get(package).unwrap());
        if term != Term::any() {
            package_terms.insert(package.clone(), term);
        }
        Self {
            package_terms,
            kind,
        }
    }

    /// CF definition of Relation enum.
    pub fn relation(&self, mut terms: impl FnMut(&P) -> Option<Term<V>>) -> Relation<P, V> {
        let mut relation = Relation::Satisfied;
        for (package, incompat_term) in self.package_terms.iter() {
            match terms(package).map(|term| incompat_term.relation_with(&term)) {
                Some(term::Relation::Satisfied) => {}
                Some(term::Relation::Contradicted) => {
                    return Relation::Contradicted((package.clone(), incompat_term.clone()));
                }
                None | Some(term::Relation::Inconclusive) => {
                    // If a package is not present, the intersection is the same as [Term::any].
                    // According to the rules of satisfactions, the relation would be inconclusive.
                    // It could also be satisfied if the incompatibility term was also [Term::any],
                    // but we systematically remove those from incompatibilities
                    // so we're safe on that front.
                    if relation == Relation::Satisfied {
                        relation = Relation::AlmostSatisfied(package.clone());
                    } else {
                        relation = Relation::Inconclusive;
                    }
                }
            }
        }
        relation
    }

    /// Check if an incompatibility should mark the end of the algorithm
    /// because it satisfies the root package.
    pub fn is_terminal(&self, root_package: &P, root_version: &V) -> bool {
        if self.package_terms.len() == 0 {
            true
        } else if self.package_terms.len() > 1 {
            false
        } else {
            let (package, term) = self.package_terms.iter().next().unwrap();
            (package == root_package) && term.contains(&root_version)
        }
    }

    /// Get the term related to a given package (if it exists).
    pub fn get(&self, package: &P) -> Option<&Term<V>> {
        self.package_terms.get(package)
    }

    /// Iterate over packages.
    pub fn iter(&self) -> impl Iterator<Item = (&P, &Term<V>)> {
        self.package_terms.iter()
    }

    // The number of packages.
    pub fn len(&self) -> usize {
        self.package_terms.len()
    }

    // Reporting ###############################################################

    /// Retrieve parent causes if of type DerivedFrom.
    pub fn causes(&self) -> Option<(Id<Self>, Id<Self>)> {
        match self.kind {
            Kind::DerivedFrom(id1, id2) => Some((id1, id2)),
            _ => None,
        }
    }

    /// Build a derivation tree for error reporting.
    pub fn build_derivation_tree(
        self_id: Id<Self>,
        shared_ids: &Set<Id<Self>>,
        store: &Arena<Self>,
        root_package: &P,
        root_version: &V,
    ) -> DerivationTree<P, V> {
        match &store[self_id].kind {
            Kind::DerivedFrom(id1, id2) => {
                let cause1 = Self::build_derivation_tree(
                    *id1,
                    shared_ids,
                    store,
                    root_package,
                    root_version,
                );
                let cause2 = Self::build_derivation_tree(
                    *id2,
                    shared_ids,
                    store,
                    root_package,
                    root_version,
                );
                let derived = Derived {
                    terms: store[self_id].package_terms.as_map(),
                    shared_id: shared_ids.get(&self_id).map(|id| id.into_raw()),
                    cause1: Box::new(cause1),
                    cause2: Box::new(cause2),
                };
                DerivationTree::Derived(derived)
            }
            Kind::NotRoot => DerivationTree::External(External::NotRoot(
                root_package.clone(),
                root_version.clone(),
            )),
            Kind::NoVersions => match &store[self_id].package_terms {
                SmallMap::One([(package, Term::Positive(range))]) => {
                    DerivationTree::External(External::NoVersions(package.clone(), range.clone()))
                }
                _ => unreachable!("NoVersions with wrong shape"),
            },
            Kind::UnavailableDependencies => match &store[self_id].package_terms {
                SmallMap::One([(package, Term::Positive(range))]) => DerivationTree::External(
                    External::UnavailableDependencies(package.clone(), range.clone()),
                ),
                _ => unreachable!("UnavailableDependencies with wrong shape"),
            },
            Kind::FromDependency => match &store[self_id].package_terms {
                SmallMap::Two(
                    [(package, Term::Positive(range)), (dep_package, Term::Negative(dep_range))],
                ) => DerivationTree::External(External::FromDependencyOf(
                    package.clone(),
                    range.clone(),
                    dep_package.clone(),
                    dep_range.clone(),
                )),
                _ => unreachable!("FromDependency with wrong shape"),
            },
        }
    }
}

impl<P: Package, V: Version> fmt::Display for Incompatibility<P, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            DefaultStringReporter::string_terms(&self.package_terms.as_map())
        )
    }
}

// TESTS #######################################################################

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::term::tests::strategy as term_strat;
    use crate::type_aliases::Map;
    use proptest::prelude::*;

    proptest! {

        /// For any three different packages p1, p2 and p3,
        /// for any three terms t1, t2 and t3,
        /// if we have the two following incompatibilities:
        ///    { p1: t1, p2: not t2 }
        ///    { p2: t2, p3: t3 }
        /// the rule of resolution says that we can deduce the following incompatibility:
        ///    { p1: t1, p3: t3 }
        #[test]
        fn rule_of_resolution(t1 in term_strat(), t2 in term_strat(), t3 in term_strat()) {
            let mut store = Arena::new();
            let i1 = store.alloc(Incompatibility {
                package_terms: SmallMap::Two([("p1", t1.clone()), ("p2", t2.negate())]),
                kind: Kind::UnavailableDependencies
            });

            let i2 = store.alloc(Incompatibility {
                package_terms: SmallMap::Two([("p2", t2), ("p3", t3.clone())]),
                kind: Kind::UnavailableDependencies
            });

            let mut i3 = Map::default();
            i3.insert("p1", t1);
            i3.insert("p3", t3);

            let i_resolution = Incompatibility::prior_cause(i1, i2, &"p2", &store);
            assert_eq!(i_resolution.package_terms.as_map(), i3);
        }

    }
}
