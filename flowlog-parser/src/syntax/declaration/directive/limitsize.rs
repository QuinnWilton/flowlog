//! `.limitsize R(n=N)`: hold a relation to a number of rows.

use educe::Educe;
use flowlog_common::Span;

use crate::Lexeme;
use crate::Node;
use crate::error::ParseError;

/// `.limitsize R(n=N)`: the target relation and the most rows it may
/// hold. An engine that honours it stops a solve as the relation grows
/// past them, as Soufflé's `.limitsize` does.
#[derive(Debug, Clone, Educe)]
#[educe(PartialEq, Eq)]
pub(crate) struct LimitSizeDirective {
    relation_name: String,
    rows: u64,
    #[educe(PartialEq(ignore))]
    span: Span,
}

impl LimitSizeDirective {
    /// Creates a directive from already-parsed parts.
    pub(crate) fn new(relation_name: String, rows: u64, span: Span) -> Self {
        Self {
            relation_name,
            rows,
            span,
        }
    }

    /// Canonical (lowercased) target relation name.
    #[must_use]
    pub(crate) fn relation_name(&self) -> &str {
        &self.relation_name
    }

    /// The most rows the relation may hold.
    #[must_use]
    pub(crate) fn rows(&self) -> u64 {
        self.rows
    }

    /// Span of the directive's target relation-name token.
    #[must_use]
    #[inline]
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    /// The relation's name as written, and its limit: what a component's
    /// body item keeps until the inliner qualifies the name.
    pub(crate) fn parse_parts(node: Node) -> Result<(String, u64, Span), ParseError> {
        let mut children = node.children();
        let name_node = children.next_any("relation name")?;
        let rows_node = children.next_any("row limit")?;
        let rows = rows_node
            .text()
            .parse::<u64>()
            .map_err(|_| ParseError::InvalidLimitSize {
                span: rows_node.span(),
                text: rows_node.text().to_string(),
            })?;
        Ok((name_node.text().to_string(), rows, name_node.span()))
    }
}

impl Lexeme for LimitSizeDirective {
    fn from_parsed_rule(node: Node) -> Result<Self, ParseError> {
        let (name, rows, span) = Self::parse_parts(node)?;
        Ok(Self {
            relation_name: name.to_lowercase(),
            rows,
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use flowlog_common::FileId;

    use super::*;
    use crate::Rule;
    use crate::test_harness::parse_pair;

    #[test]
    fn limitsize_lowercases_name_and_reads_the_limit() {
        let d = LimitSizeDirective::from_parsed_rule(Node::new(
            parse_pair(Rule::limitsize_directive, ".limitsize Edge(n=500000)"),
            FileId::new(0),
        ))
        .expect("limitsize_directive parses");
        assert_eq!(d.relation_name(), "edge");
        assert_eq!(d.rows(), 500_000);
    }
}
