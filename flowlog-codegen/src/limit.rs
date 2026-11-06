//! Row budgets (`.limitsize`), when the engine honours them
//! (`Config::limitsize`): each limited relation's last binding counted
//! into the engine's `limits` as it is derived, and the step loop's stop
//! once one grows past its budget. The frontend binds `limits`, a
//! `flowlog_runtime::limit::Limits` made of [`limited_relations`] in order,
//! wherever the dataflow and the step loop are spliced.
//!
//! A relation's last binding unions every earlier one (its seed), so its
//! changes sum to the relation's rows; counting an earlier binding too
//! would count those rows twice. Inside a loop it is counted before the
//! loop's feedback, so a fixpoint stops while it grows rather than once it
//! leaves. A lattice relation in a static loop is the exception: its
//! binding there keeps every iteration's answer until the fold after the
//! loop, so each answer counts, and its count can pass the rows it ends
//! with.

use flowlog_common::collections::HashMap;
use flowlog_parser::Program;
use flowlog_parser::Relation;
use flowlog_planner::planner::StratumPlanner;
use flowlog_profiler::PlanGraph;
use flowlog_profiler::with_plan_graph;
use proc_macro2::Ident;
use proc_macro2::TokenStream;
use quote::quote;

use crate::Codegen;

/// The relations `program` holds to a number of rows, in declaration
/// order: the budgets of an engine's `limits`, by position.
#[must_use]
pub fn limited_relations(program: &Program) -> Vec<&Relation> {
    program
        .relations()
        .iter()
        .filter(|rel| rel.limitsize().is_some())
        .collect()
}

/// Where a limited relation is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Count {
    /// The relation's position among [`limited_relations`].
    index: usize,
    /// The stratum that binds the relation last.
    stratum: usize,
}

impl Codegen {
    /// Records where each limited relation is counted: at its binding in
    /// the last of `strata` to bind it. A relation no stratum binds holds
    /// only its input, which no budget counts. Records nothing when the
    /// engine does not honour budgets.
    pub(crate) fn seed_limits(&mut self, strata: &[StratumPlanner]) {
        self.limits = HashMap::default();
        if !self.config.limitsize_enabled() {
            return;
        }
        for (index, relation) in limited_relations(&self.program).iter().enumerate() {
            let fp = relation.fingerprint();
            let last = strata
                .iter()
                .rposition(|stratum| stratum.relation_bindings().contains_key(&fp));
            if let Some(stratum) = last {
                self.limits.insert(fp, Count { index, stratum });
            }
        }
    }

    /// Returns `rows`, the binding of relation `idb_fp` that `stratum`
    /// binds to `binding`, counted into `limits` when it is the relation's
    /// last binding and the relation is limited; `rows` unchanged
    /// otherwise.
    pub(crate) fn gen_limit_count(
        &self,
        idb_fp: u64,
        stratum: usize,
        rows: TokenStream,
        binding: &Ident,
        plan_graph: &mut Option<PlanGraph>,
    ) -> TokenStream {
        let Some(count) = self.limits.get(&idb_fp) else {
            return rows;
        };
        if count.stratum != stratum {
            return rows;
        }
        with_plan_graph(plan_graph, |plan_graph| {
            plan_graph.limit_operator(self.display_name(idb_fp), binding.to_string());
        });
        let index = count.index;
        // A batch at a time, so workers add to the shared count once per
        // batch rather than once per row.
        quote! {{
            let limits = limits.clone();
            (#rows).inspect_batch(move |_time, updates| limits.count(#index, updates))
        }}
    }

    /// Returns the step loop's condition to keep stepping while no relation
    /// has outgrown its budget, or `None` when nothing is counted.
    pub(crate) fn gen_limit_stop(&self) -> Option<TokenStream> {
        (!self.limits.is_empty()).then(|| quote! { && !limits.is_exceeded() })
    }
}

#[cfg(test)]
mod tests {
    use flowlog_planner::planner::ProgramPlanner;
    use proc_macro2::Span;
    use rstest::rstest;

    use super::*;
    use crate::test_harness::codegen;

    /// A transitive closure split over two strata: `Reach`'s base rule,
    /// then the loop that unions it in as its seed.
    const PROGRAM: &str = ".decl Edge(x: int32, y: int32) mutable\n.input Edge\n\
        .decl Reach(x: int32, y: int32)\n.output Reach\n.limitsize Reach(n=10)\n\
        .decl Hop(x: int32)\n.output Hop\n.limitsize Hop(n=5)\n\
        Reach(x, y) :- Edge(x, y).\nReach(x, z) :- Reach(x, y), Edge(y, z).\n\
        Hop(x) :- Edge(x, _).\n";

    /// Returns a code generator over `source` whose engine honours budgets
    /// when `limitsize` says so, with its limits seeded from the program's
    /// plan, and that plan.
    fn seeded(source: &str, limitsize: bool) -> (Codegen, ProgramPlanner) {
        let mut codegen = codegen(source);
        codegen.config.limitsize = limitsize;
        let planner = ProgramPlanner::from_program(&codegen.program, &mut None).expect("plans");
        codegen.seed_limits(planner.strata());
        (codegen, planner)
    }

    fn fingerprint(codegen: &Codegen, name: &str) -> u64 {
        codegen
            .program
            .relations()
            .iter()
            .find(|rel| rel.raw_name() == name)
            .expect("declared")
            .fingerprint()
    }

    #[test]
    fn limited_relations_are_those_with_a_limitsize_in_declaration_order() {
        let codegen = codegen(PROGRAM);
        let names: Vec<&str> = limited_relations(&codegen.program)
            .iter()
            .map(|rel| rel.raw_name())
            .collect();
        assert_eq!(names, ["Reach", "Hop"]);
    }

    /// A relation bound in several strata is counted at the last, which
    /// unions the earlier ones, so its rows are counted once.
    #[test]
    fn a_relation_is_counted_at_its_last_binding_only() {
        let (codegen, planner) = seeded(PROGRAM, true);
        let strata = planner.strata();
        let reach = fingerprint(&codegen, "Reach");
        let binds: Vec<usize> = strata
            .iter()
            .enumerate()
            .filter(|(_, stratum)| stratum.relation_bindings().contains_key(&reach))
            .map(|(index, _)| index)
            .collect();
        let [.., last] = binds.as_slice() else {
            panic!("Reach is bound by some stratum");
        };
        assert!(
            binds.len() > 1,
            "Reach's base rule binds it apart: {binds:?}"
        );
        let binding = Ident::new("next_reach", Span::call_site());
        for stratum in &binds {
            let counted = codegen
                .gen_limit_count(reach, *stratum, quote! { rows }, &binding, &mut None)
                .to_string();
            let expected = if stratum == last {
                quote! {{
                    let limits = limits.clone();
                    (rows).inspect_batch(move |_time, updates| limits.count(0usize, updates))
                }}
            } else {
                quote! { rows }
            };
            assert_eq!(counted, expected.to_string(), "stratum {stratum}");
        }
    }

    /// A relation is counted at its position among the limited relations.
    #[test]
    fn a_relation_counts_at_its_budgets_position() {
        let (codegen, planner) = seeded(PROGRAM, true);
        let strata = planner.strata();
        let hop = fingerprint(&codegen, "Hop");
        let stratum = strata
            .iter()
            .position(|stratum| stratum.relation_bindings().contains_key(&hop))
            .expect("Hop is bound");
        let counted = codegen
            .gen_limit_count(
                hop,
                stratum,
                quote! { rows },
                &Ident::new("hop", Span::call_site()),
                &mut None,
            )
            .to_string();
        assert!(
            counted.contains("limits . count (1usize , updates)"),
            "{counted}"
        );
    }

    /// An engine that does not honour budgets counts nothing and never
    /// stops, and nor does one whose program has no budget.
    // Cases: program, engine honours budgets.
    #[rstest]
    #[case::not_honoured(PROGRAM, false)]
    #[case::no_budget(
        ".decl Edge(x: int32) mutable\n.input Edge\n.decl R(x: int32)\n.output R\nR(x) :- Edge(x).\n",
        true
    )]
    fn nothing_is_counted_without_an_honoured_budget(
        #[case] source: &str,
        #[case] limitsize: bool,
    ) {
        let (codegen, planner) = seeded(source, limitsize);
        let strata = planner.strata();
        for (stratum, planned) in strata.iter().enumerate() {
            for fp in planned.relation_bindings().keys() {
                let counted = codegen.gen_limit_count(
                    *fp,
                    stratum,
                    quote! { rows },
                    &Ident::new("r", Span::call_site()),
                    &mut None,
                );
                assert_eq!(counted.to_string(), "rows");
            }
        }
        assert!(codegen.gen_limit_stop().is_none());
    }

    #[test]
    fn the_step_loop_stops_once_a_budget_is_exceeded() {
        let (codegen, _) = seeded(PROGRAM, true);
        assert_eq!(
            codegen.gen_limit_stop().map(|stop| stop.to_string()),
            Some(quote! { && !limits.is_exceeded() }.to_string())
        );
    }
}
