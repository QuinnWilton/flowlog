//! Planner errors: user errors that need the strata, ICEs, and catalog
//! errors.
//!
//! The parser owns errors within one rule; the planner reports those that
//! depend on how rules form recursion. Catalog errors pass through the
//! planner unchanged, and invariant violations use [`PlanError::internal`].

use codespan_reporting::diagnostic::Diagnostic as CsDiagnostic;
use flowlog_common::BUG_URL;
use flowlog_common::Diagnostic;
use flowlog_common::FileId;
use flowlog_common::InternalError;
use flowlog_common::Span;
use flowlog_common::primary_label;
use flowlog_common::secondary_label;
use thiserror::Error;

use crate::catalog::CatalogError;

#[non_exhaustive]
#[derive(Debug, Error)]
pub(crate) enum PlanError {
    /// An aggregate reads `relation`, which its own recursive stratum
    /// derives, at the atom `read_span` covers. An aggregate over a changing
    /// input has no fixpoint to settle on; only a lattice column may merge
    /// values across recursion.
    #[error("aggregate reads `{relation}`, which its own recursion derives")]
    AggregateInRecursion {
        relation: String,
        read_span: Span,
        rule_span: Span,
    },

    /// A rule negates `relation`, which its own recursive stratum derives,
    /// at the atom `negation_span` covers. Negation is stratified only when
    /// the negated relation is complete before the rule runs; inside its
    /// own recursion it is still growing, so the loop has no fixpoint to
    /// settle on. `derived_span` is the rule that derives `relation`, when
    /// it isn't the negating rule itself.
    #[error("rule negates `{relation}`, which its own recursion derives")]
    NegationInRecursion {
        relation: String,
        negation_span: Span,
        rule_span: Span,
        derived_span: Option<Span>,
    },

    /// Catalog errors bubble through the planner unchanged.
    #[error(transparent)]
    Catalog(#[from] CatalogError),

    /// Planner/optimizer invariant violation. Rendered as a "please file a
    /// bug" ICE; optimizer sites share the `"planner"` stage label.
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl PlanError {
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal(InternalError::new("planner", detail, BUG_URL))
    }
}

impl Diagnostic for PlanError {
    fn to_diagnostic(&self) -> CsDiagnostic<FileId> {
        match self {
            PlanError::AggregateInRecursion {
                relation,
                read_span,
                rule_span,
            } => {
                let labels = [
                    primary_label(*read_span)
                        .map(|l| l.with_message(format!("`{relation}` changes every round"))),
                    secondary_label(*rule_span).map(|l| l.with_message("in this recursive rule")),
                ];
                CsDiagnostic::error()
                    .with_message(self.to_string())
                    .with_labels(labels.into_iter().flatten().collect())
                    .with_notes(vec![
                        "an aggregate may read only relations computed before the recursion; \
                         to keep the least or greatest value per key across the recursion, \
                         declare a lattice column instead: `.decl R(k: T, v: min<U>)`"
                            .into(),
                    ])
            }
            PlanError::NegationInRecursion {
                relation,
                negation_span,
                rule_span,
                derived_span,
            } => {
                let labels = [
                    primary_label(*negation_span).map(|l| {
                        l.with_message(format!("`{relation}` is still being derived here"))
                    }),
                    secondary_label(*rule_span).map(|l| l.with_message("in this recursive rule")),
                    derived_span.and_then(|span| {
                        secondary_label(span).map(|l| {
                            l.with_message(format!(
                                "which feeds `{relation}` in the same recursion"
                            ))
                        })
                    }),
                ];
                CsDiagnostic::error()
                    .with_message(self.to_string())
                    .with_labels(labels.into_iter().flatten().collect())
                    .with_notes(vec![
                        "a negation may read only relations computed before its recursion; \
                         negate in a rule after the recursion instead, or derive the negated \
                         relation without reading the recursive one"
                            .into(),
                    ])
            }
            PlanError::Catalog(e) => e.to_diagnostic(),
            PlanError::Internal(ie) => ie.to_diagnostic(),
        }
    }

    fn is_internal(&self) -> bool {
        match self {
            PlanError::AggregateInRecursion { .. } | PlanError::NegationInRecursion { .. } => false,
            PlanError::Internal(_) => true,
            PlanError::Catalog(e) => e.is_internal(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use flowlog_common::SourceMap;

    use super::*;

    #[test]
    fn internal_error_is_flagged_internal() {
        assert!(PlanError::internal("broken invariant").is_internal());
    }

    /// An aggregate in a recursive rule is the user's to fix, not an ICE.
    #[test]
    fn aggregate_in_recursion_is_not_internal() {
        let err = PlanError::AggregateInRecursion {
            relation: "R".into(),
            read_span: Span::DUMMY,
            rule_span: Span::DUMMY,
        };
        assert!(!err.is_internal());
    }

    /// The negated atom gets the primary label, the negating rule a
    /// secondary one, and the rule deriving the relation another.
    #[test]
    fn negation_in_recursion_labels_the_negation_and_both_rules() {
        let mut sm = SourceMap::new();
        let file = sm.add(
            "test.dl".into(),
            "A(x) :- E(x), !B(x).\nB(x) :- A(x).".into(),
        );
        let err = PlanError::NegationInRecursion {
            relation: "B".into(),
            negation_span: Span::new(file, 14, 19),
            rule_span: Span::new(file, 0, 20),
            derived_span: Some(Span::new(file, 21, 34)),
        };
        assert!(!err.is_internal());
        let labels = err.to_diagnostic().labels;
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].message, "`B` is still being derived here");
        assert_eq!(labels[1].message, "in this recursive rule");
        assert_eq!(labels[2].message, "which feeds `B` in the same recursion");
    }

    /// The recursive atom the aggregate reads gets the primary label and the
    /// enclosing rule the secondary one.
    #[test]
    fn aggregate_in_recursion_labels_the_read_and_the_rule() {
        let mut sm = SourceMap::new();
        let file = sm.add(
            "test.dl".into(),
            "R(x, c) :- R(x, _), c = count : R(_, _).".into(),
        );
        let err = PlanError::AggregateInRecursion {
            relation: "R".into(),
            read_span: Span::new(file, 32, 39),
            rule_span: Span::new(file, 0, 40),
        };
        let labels = err.to_diagnostic().labels;
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].message, "`R` changes every round");
        assert_eq!(labels[1].message, "in this recursive rule");
    }
}
