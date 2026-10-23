//! Hash collections whose iteration order is a function of their contents.
//!
//! `std`'s maps and sets seed their hasher randomly per process, so the
//! order they iterate in, and with it every plan and every line of code
//! derived from iterating one, differs from one compilation of a program
//! to the next. The compiler's crates use these instead: the same program
//! plans and generates the same code every time, and a build is
//! reproducible.

use std::collections::hash_map::DefaultHasher;
use std::hash::BuildHasherDefault;

/// The hasher of every map and set here: SipHash with fixed keys.
pub type FixedState = BuildHasherDefault<DefaultHasher>;

/// A `std` hash map with a fixed hasher (`HashMap::default()`).
pub type HashMap<K, V> = std::collections::HashMap<K, V, FixedState>;

/// A `std` hash set with a fixed hasher (`HashSet::default()`).
pub type HashSet<T> = std::collections::HashSet<T, FixedState>;
