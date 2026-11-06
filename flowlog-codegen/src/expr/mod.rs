//! Expressions inside operator closures, from a rule's arguments and filters:
//!
//! - [`param`]: the closure's parameters.
//! - [`projection`]: the key and value tuples it emits.
//! - [`compare`] and [`constraint`]: the predicates it filters on.
//! - [`guard`]: the test that its partial functions' arguments lie in
//!   their domains, ahead of those predicates.
//! - [`aggregation`]: an aggregation's kind and the split and merge around
//!   its column.
//! - [`term`]: the terms all of the above are built from.

pub(crate) mod aggregation;
pub(crate) mod compare;
pub(crate) mod constraint;
pub(crate) mod guard;
pub(crate) mod param;
pub(crate) mod projection;
pub(crate) mod term;
