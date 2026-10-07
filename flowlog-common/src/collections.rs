//! Hash collections whose iteration order is a function of their contents.
//!
//! `std`'s maps and sets seed their hasher randomly per process, so the
//! order they iterate in, and with it every plan and every line of code
//! derived from iterating one, differs from one compilation of a program
//! to the next. The compiler's crates use these instead: the same program
//! plans and generates the same code every time, and a build is
//! reproducible.
//!
//! The hasher is FxHash: its seed is fixed, and it is several times faster
//! than SipHash over the planner's keys (expressions, atom signatures,
//! canonical forms), which it hashes whole and often. Planning argus's
//! largest program spent nearly half its time in SipHash.

use std::hash::BuildHasherDefault;

use rustc_hash::FxHasher;

/// The hasher of every map and set here: FxHash, whose seed is fixed.
pub type FixedState = BuildHasherDefault<FxHasher>;

/// A `std` hash map with a fixed hasher (`HashMap::default()`).
pub type HashMap<K, V> = std::collections::HashMap<K, V, FixedState>;

/// A `std` hash set with a fixed hasher (`HashSet::default()`).
pub type HashSet<T> = std::collections::HashSet<T, FixedState>;
