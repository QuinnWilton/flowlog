//! Row budgets (`.limitsize`): the rows each limited relation holds as an
//! engine derives them, counted by every worker into one [`Limits`] the
//! engine's host also reads, and the [`LimitExceeded`] it reports once a
//! relation grows past its budget.
//!
//! A relation's count is the sum of the weights of every change its
//! binding has emitted, so it is the relation's rows so far: inside a loop,
//! each iteration's changes, which sum to the rows derived until then.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crate::diff::Append;
use crate::diff::Mutable;
use crate::diff::Static;

// =============================================================================
// Rows
// =============================================================================

/// A weight as the rows it adds to a relation's count.
pub trait Rows {
    /// The rows this weight adds: one for a presence, its signed count for
    /// a signed weight.
    fn rows(&self) -> i64;
}

impl Rows for Static {
    #[inline]
    fn rows(&self) -> i64 {
        1
    }
}

impl Rows for Append {
    #[inline]
    fn rows(&self) -> i64 {
        1
    }
}

impl Rows for Mutable {
    #[inline]
    fn rows(&self) -> i64 {
        i64::from(*self)
    }
}

// =============================================================================
// Limits
// =============================================================================

/// The program's row budgets and the rows each limited relation holds,
/// shared by every worker and the host.
///
/// Workers count without synchronizing with each other: a count and the
/// stop it causes are seen by the other workers soon, not at once, as a
/// worker checks between steps. The host reads them once every worker has
/// returned from the commit.
#[derive(Clone, Debug)]
pub struct Limits(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    budgets: Vec<Budget>,
    /// Each budget's relation's rows so far, by the budget's position.
    held: Vec<AtomicI64>,
    /// The position of the first budget a relation outgrew, or
    /// [`Shared::NONE`].
    exceeded: AtomicUsize,
}

#[derive(Debug)]
struct Budget {
    relation: &'static str,
    rows: u64,
}

impl Shared {
    const NONE: usize = usize::MAX;
}

impl Limits {
    /// Limits holding each relation of `budgets` to its rows, each relation
    /// counted at its position there.
    #[must_use]
    pub fn new(budgets: &[(&'static str, u64)]) -> Self {
        Self(Arc::new(Shared {
            budgets: budgets
                .iter()
                .map(|&(relation, rows)| Budget { relation, rows })
                .collect(),
            held: budgets.iter().map(|_| AtomicI64::new(0)).collect(),
            exceeded: AtomicUsize::new(Shared::NONE),
        }))
    }

    /// Adds `updates`' weights to the rows of the relation at `index`, and
    /// records the stop when they grow past its budget. A stop, once
    /// recorded, is never replaced: the first relation to grow past its
    /// budget is the one reported.
    ///
    /// # Panics
    ///
    /// Panics if `index` is not a position of the budgets the limits were
    /// made with: generated code counts only the relations it made them of.
    pub fn count<D, T, R: Rows>(&self, index: usize, updates: &[(D, T, R)]) {
        let rows: i64 = updates.iter().map(|(_, _, diff)| diff.rows()).sum();
        if rows == 0 {
            return;
        }
        let held = self.0.held[index].fetch_add(rows, Ordering::Relaxed) + rows;
        if u64::try_from(held).is_ok_and(|held| held > self.0.budgets[index].rows) {
            // Only the first stop is kept; a later one failing to replace
            // it is the point.
            let _ = self.0.exceeded.compare_exchange(
                Shared::NONE,
                index,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }

    /// Returns `true` if some relation has grown past its budget: the
    /// check a worker makes between steps, so it costs one load.
    #[must_use]
    #[inline]
    pub fn is_exceeded(&self) -> bool {
        self.0.exceeded.load(Ordering::Relaxed) != Shared::NONE
    }

    /// The first relation to grow past its budget, with the rows it held
    /// when read, or `None` while every relation is within its budget.
    #[must_use]
    pub fn exceeded(&self) -> Option<LimitExceeded> {
        let index = self.0.exceeded.load(Ordering::Relaxed);
        let budget = self.0.budgets.get(index)?;
        Some(LimitExceeded {
            relation: budget.relation,
            rows: self.0.held[index].load(Ordering::Relaxed),
            limit: budget.rows,
        })
    }
}

// =============================================================================
// LimitExceeded
// =============================================================================

/// A relation grew past the rows its `.limitsize` holds it to, and the
/// engine stopped the commit deriving it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitExceeded {
    /// The relation, as the program spells it.
    pub relation: &'static str,
    /// The rows it held when the engine stopped, past `limit`: a commit
    /// stops as soon as the workers see the count, not at the row that
    /// crossed it.
    pub rows: i64,
    /// The rows its `.limitsize` holds it to.
    pub limit: u64,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "relation `{}` outgrew its limit of {} rows (`.limitsize`), with {} and growing",
            self.relation, self.limit, self.rows
        )
    }
}

impl std::error::Error for LimitExceeded {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// Rows of a relation's changes, at no particular time.
    fn changes<R: Copy>(weights: &[R]) -> Vec<((), (), R)> {
        weights.iter().map(|&weight| ((), (), weight)).collect()
    }

    /// A presence adds one row whatever it is; a signed weight adds its
    /// count, so a retraction takes a row away.
    // Cases: weight, rows.
    #[rstest]
    #[case::static_presence(Static.rows(), 1)]
    #[case::append_presence(Append.rows(), 1)]
    #[case::insertion(2_i32.rows(), 2)]
    #[case::retraction((-1_i32).rows(), -1)]
    fn a_weight_adds_its_rows(#[case] found: i64, #[case] rows: i64) {
        assert_eq!(found, rows);
    }

    #[test]
    fn rows_within_the_budget_stop_nothing() {
        let limits = Limits::new(&[("Reach", 3)]);
        limits.count(0, &changes(&[1_i32, 1, 1]));
        assert!(!limits.is_exceeded());
        assert_eq!(limits.exceeded(), None);
    }

    /// A budget is the most rows a relation may hold: one more stops it,
    /// counted across every worker's share.
    #[test]
    fn a_row_past_the_budget_stops_the_commit() {
        let limits = Limits::new(&[("Reach", 3)]);
        let worker = limits.clone();
        limits.count(0, &changes(&[Static, Static]));
        worker.count(0, &changes(&[Static, Static]));
        assert!(limits.is_exceeded());
        assert_eq!(
            limits.exceeded(),
            Some(LimitExceeded {
                relation: "Reach",
                rows: 4,
                limit: 3
            })
        );
    }

    /// Retractions count against the rows, so a relation that shrank back
    /// within its budget before growing again is held to the rows it has.
    #[test]
    fn retractions_take_rows_back() {
        let limits = Limits::new(&[("Reach", 3)]);
        limits.count(0, &changes(&[1_i32, 1, 1, -1, -1]));
        limits.count(0, &changes(&[1_i32, 1]));
        assert!(!limits.is_exceeded());
    }

    /// The first relation past its budget is the one reported, with the
    /// rows it holds when read.
    #[test]
    fn the_first_relation_past_its_budget_is_reported() {
        let limits = Limits::new(&[("A", 1), ("B", 1)]);
        limits.count(1, &changes(&[1_i32, 1]));
        limits.count(0, &changes(&[1_i32, 1]));
        limits.count(1, &changes(&[1_i32]));
        assert_eq!(
            limits.exceeded(),
            Some(LimitExceeded {
                relation: "B",
                rows: 3,
                limit: 1
            })
        );
    }

    #[test]
    fn no_budgets_are_never_exceeded() {
        let limits = Limits::new(&[]);
        assert!(!limits.is_exceeded());
        assert_eq!(limits.exceeded(), None);
    }

    #[test]
    fn a_stop_names_the_relation_its_rows_and_its_limit() {
        let exceeded = LimitExceeded {
            relation: "Reach",
            rows: 12,
            limit: 10,
        };
        assert_eq!(
            exceeded.to_string(),
            "relation `Reach` outgrew its limit of 10 rows (`.limitsize`), with 12 and growing"
        );
    }
}
