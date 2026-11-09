//! Domain guards: the test a closure makes before it evaluates a partial
//! function, so that a row outside the function's domain yields no row,
//! rather than a panic or an invented value. The partial functions are
//! integer `/` and `%`, undefined for a zero divisor (float division is
//! IEEE's, defined everywhere), and the built-ins whose domains
//! `flowlog_runtime::builtin` defines, `to_number` and `substr`.
//!
//! A partial function is a relation with no row for an input outside its
//! domain, so a rule derives nothing from a binding that evaluates one
//! there, whichever order the planner evaluates its conjuncts in. Each
//! step guards every expression it evaluates, comparisons and projections
//! alike, and its predicate tests the guard before any comparison. A guard
//! that runs at an earlier step than the rule's last (a comparison pushed
//! down to a join) reads only the variables bound there, so a partial
//! binding it drops has no extension the rule could derive a row from.

use flowlog_parser::ArithmeticOperator;
use flowlog_parser::BuiltinOperator;
use flowlog_parser::DataType;
use flowlog_planner::planner::ArithmeticArgument;
use flowlog_planner::planner::ComparisonExprArgument;
use flowlog_planner::planner::FactorArgument;
use flowlog_planner::planner::TransformationArgument;
use proc_macro2::Ident;
use proc_macro2::TokenStream;
use quote::quote;

use crate::Codegen;
use crate::CodegenError;
use crate::expr::term::arithmetic::join_variable;
use crate::expr::term::arithmetic::kv_variable;
use crate::expr::term::arithmetic::row_variable;
use crate::expr::term::as_str;
use crate::expr::term::builtin::index_literal;
use crate::expr::term::builtin::operands;
use crate::ty::data::KvTypes;

impl Codegen {
    /// Returns a row closure's domain guard over `exprs`, or `None` when
    /// none of them evaluates a partial function outside a domain its
    /// literals already establish. Each variable reads from the row
    /// pattern's `fields`.
    pub(crate) fn row_guard<'a>(
        &mut self,
        exprs: impl IntoIterator<Item = &'a ArithmeticArgument>,
        fields: &[Ident],
        string_intern: bool,
        input_type: &KvTypes,
    ) -> Result<Option<TokenStream>, CodegenError> {
        self.guard(
            exprs,
            string_intern,
            (input_type, None),
            &row_variable(fields, string_intern, input_type),
        )
    }

    /// Returns a key-value closure's domain guard over `exprs`, as
    /// [`Self::row_guard`] does. Each variable reads from the `(k, v)`
    /// parameters; a join argument reads the surviving side of an
    /// antijoin, whose key both sides share.
    pub(crate) fn kv_guard<'a>(
        &mut self,
        exprs: impl IntoIterator<Item = &'a ArithmeticArgument>,
        string_intern: bool,
        input_type: &KvTypes,
    ) -> Result<Option<TokenStream>, CodegenError> {
        self.guard(
            exprs,
            string_intern,
            (input_type, Some(input_type)),
            &kv_variable(string_intern, input_type),
        )
    }

    /// Returns a join closure's domain guard over `exprs`, as
    /// [`Self::row_guard`] does. Each variable reads from the
    /// `(k, lv, rv)` bindings.
    pub(crate) fn join_guard<'a>(
        &mut self,
        exprs: impl IntoIterator<Item = &'a ArithmeticArgument>,
        string_intern: bool,
        left_type: &KvTypes,
        right_type: &KvTypes,
    ) -> Result<Option<TokenStream>, CodegenError> {
        self.guard(
            exprs,
            string_intern,
            (left_type, Some(right_type)),
            &join_variable(string_intern, left_type, right_type),
        )
    }

    /// Returns every check `exprs` need, joined with `&&`, or `None` when
    /// there is none. A check comes after the checks of the expressions
    /// inside it, so `&&` reaches `x / (y / z)`'s `y / z != 0` only once
    /// `z != 0` holds; a repeated check keeps its first place, which its
    /// own inner checks already precede.
    fn guard<'a, F>(
        &mut self,
        exprs: impl IntoIterator<Item = &'a ArithmeticArgument>,
        string_intern: bool,
        types: (&KvTypes, Option<&KvTypes>),
        resolve_var: &F,
    ) -> Result<Option<TokenStream>, CodegenError>
    where
        F: Fn(&TransformationArgument) -> Result<TokenStream, CodegenError>,
    {
        let mut checks = Vec::new();
        for expr in exprs {
            self.arithmetic_checks(expr, string_intern, types, resolve_var, &mut checks)?;
        }
        let mut seen = std::collections::HashSet::new();
        checks.retain(|check| seen.insert(check.to_string()));
        Ok((!checks.is_empty()).then(|| quote! { #( #checks )&&* }))
    }

    /// Appends the checks of `expr`'s factors and divisors to `checks`.
    fn arithmetic_checks<F>(
        &mut self,
        expr: &ArithmeticArgument,
        string_intern: bool,
        types: (&KvTypes, Option<&KvTypes>),
        resolve_var: &F,
        checks: &mut Vec<TokenStream>,
    ) -> Result<(), CodegenError>
    where
        F: Fn(&TransformationArgument) -> Result<TokenStream, CodegenError>,
    {
        self.factor_checks(expr.init(), string_intern, types, resolve_var, checks)?;
        for (op, factor) in expr.rest() {
            self.factor_checks(factor, string_intern, types, resolve_var, checks)?;
            if !matches!(op, ArithmeticOperator::Divide | ArithmeticOperator::Modulo) {
                continue;
            }
            // Every factor of an expression has its type, so the divisor's
            // is the expression's.
            if !self.infer_expr_type(expr, types.0, types.1)?.is_integer() {
                continue;
            }
            let literal = match factor {
                FactorArgument::Const(c) => c.text().parse::<i128>().ok(),
                _ => None,
            };
            let check = match literal {
                Some(0) => quote! { false },
                Some(_) => continue,
                None => {
                    let divisor = self.factor_to_token(factor, string_intern, resolve_var)?;
                    quote! { #divisor != 0 }
                }
            };
            checks.push(check);
        }
        Ok(())
    }

    /// Appends the checks `factor` needs to `checks`: its operands', then a
    /// partial built-in's own.
    fn factor_checks<F>(
        &mut self,
        factor: &FactorArgument,
        string_intern: bool,
        types: (&KvTypes, Option<&KvTypes>),
        resolve_var: &F,
        checks: &mut Vec<TokenStream>,
    ) -> Result<(), CodegenError>
    where
        F: Fn(&TransformationArgument) -> Result<TokenStream, CodegenError>,
    {
        match factor {
            FactorArgument::Var(_) | FactorArgument::Const(_) => Ok(()),
            FactorArgument::Group(inner) | FactorArgument::TupleProj { tuple: inner, .. } => {
                self.arithmetic_checks(inner, string_intern, types, resolve_var, checks)
            }
            FactorArgument::Tuple { fields: args } | FactorArgument::FnCall { args, .. } => {
                for arg in args {
                    self.arithmetic_checks(arg, string_intern, types, resolve_var, checks)?;
                }
                Ok(())
            }
            FactorArgument::Builtin { op, args } => {
                for arg in args {
                    self.arithmetic_checks(arg, string_intern, types, resolve_var, checks)?;
                }
                if let Some(check) = self.builtin_check(*op, args, string_intern, resolve_var)? {
                    checks.push(check);
                }
                Ok(())
            }
        }
    }

    /// Returns the test that a built-in call's arguments lie in its
    /// domain, or `None` for a total built-in or for literal arguments
    /// already inside the domain.
    fn builtin_check<F>(
        &mut self,
        op: BuiltinOperator,
        args: &[ArithmeticArgument],
        string_intern: bool,
        resolve_var: &F,
    ) -> Result<Option<TokenStream>, CodegenError>
    where
        F: Fn(&TransformationArgument) -> Result<TokenStream, CodegenError>,
    {
        match op {
            BuiltinOperator::ToNumber => {
                let [s] = operands(op, args)?;
                if number_literal(s) {
                    return Ok(None);
                }
                let s = self.arithmetic_to_token(s, string_intern, resolve_var)?;
                let s = as_str(&s, string_intern);
                Ok(Some(
                    quote! { ::flowlog_runtime::builtin::to_number_defined(#s) },
                ))
            }
            BuiltinOperator::Substr => {
                let [s, start, len] = operands(op, args)?;
                // Any text has a slice from its first character.
                if index_literal(start) == Some(0) && index_literal(len).is_some() {
                    return Ok(None);
                }
                let s = self.arithmetic_to_token(s, string_intern, resolve_var)?;
                let s = as_str(&s, string_intern);
                let start = self.arithmetic_to_argument(start, string_intern, resolve_var)?;
                let len = self.arithmetic_to_argument(len, string_intern, resolve_var)?;
                Ok(Some(
                    quote! { ::flowlog_runtime::builtin::substr_defined(#s, #start, #len) },
                ))
            }
            BuiltinOperator::Strlen
            | BuiltinOperator::Ord
            | BuiltinOperator::ToString
            | BuiltinOperator::Cat
            | BuiltinOperator::Hash => Ok(None),
        }
    }
}

/// Returns both sides of every comparison in `compares`, the expressions
/// a step evaluates to filter.
pub(crate) fn compare_operands(
    compares: &[ComparisonExprArgument],
) -> impl Iterator<Item = &ArithmeticArgument> {
    compares.iter().flat_map(|c| [c.left(), c.right()])
}

/// Returns `true` if `arg` is a bare string literal inside `to_number`'s
/// domain, which `flowlog_runtime::builtin::to_number_defined` defines.
fn number_literal(arg: &ArithmeticArgument) -> bool {
    match arg.init() {
        FactorArgument::Const(c) if arg.rest().is_empty() => {
            c.ty() == &DataType::String && c.text().parse::<i32>().is_ok()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use flowlog_parser::Constant;
    use flowlog_planner::planner::TransformationArgument::KV;
    use rstest::rstest;

    use super::*;
    use crate::test_harness::codegen;

    /// The value column `v.<idx>` of a key-value closure.
    fn value(idx: usize) -> FactorArgument {
        FactorArgument::Var(KV((false, idx)))
    }

    fn literal(ty: DataType, text: &str) -> FactorArgument {
        FactorArgument::Const(Constant::new(ty, text))
    }

    fn expr(
        init: FactorArgument,
        rest: Vec<(ArithmeticOperator, FactorArgument)>,
    ) -> ArithmeticArgument {
        ArithmeticArgument { init, rest }
    }

    fn builtin(op: BuiltinOperator, args: Vec<FactorArgument>) -> FactorArgument {
        FactorArgument::Builtin {
            op,
            args: args.into_iter().map(|arg| expr(arg, Vec::new())).collect(),
        }
    }

    /// The guard of `exprs` in a key-value closure over three `int32`
    /// values, a string, and a float, rendered.
    fn guard_of(exprs: &[ArithmeticArgument]) -> Option<String> {
        let input_type: KvTypes = (
            Vec::new(),
            vec![
                DataType::Int32,
                DataType::Int32,
                DataType::Int32,
                DataType::String,
                DataType::Float64,
            ],
        );
        codegen("")
            .kv_guard(exprs, false, &input_type)
            .expect("guard")
            .map(|guard| guard.to_string())
    }

    fn tokens(guard: TokenStream) -> Option<String> {
        Some(guard.to_string())
    }

    // Cases: operator.
    #[rstest]
    #[case::divide(ArithmeticOperator::Divide)]
    #[case::modulo(ArithmeticOperator::Modulo)]
    fn an_integer_divisor_is_tested_for_zero(#[case] op: ArithmeticOperator) {
        let quotient = expr(value(0), vec![(op, value(1))]);
        assert_eq!(guard_of(&[quotient]), tokens(quote! { v.1 != 0 }));
    }

    /// A literal divisor is decided here: a nonzero one needs no test, and
    /// zero is never a divisor.
    // Cases: divisor, guard.
    #[rstest]
    #[case::nonzero("2", None)]
    #[case::negative("-1", None)]
    #[case::zero("0", tokens(quote! { false }))]
    fn a_literal_divisor_is_decided_when_generating(
        #[case] divisor: &str,
        #[case] guard: Option<String>,
    ) {
        let quotient = expr(
            value(0),
            vec![(
                ArithmeticOperator::Divide,
                literal(DataType::Int32, divisor),
            )],
        );
        assert_eq!(guard_of(&[quotient]), guard);
    }

    /// IEEE division is defined for a zero divisor.
    #[test]
    fn a_float_divisor_is_not_tested() {
        let quotient = expr(value(4), vec![(ArithmeticOperator::Divide, value(4))]);
        assert_eq!(guard_of(&[quotient]), None);
    }

    /// A divisor that divides is tested after its own divisor, so `&&`
    /// never evaluates it on a zero.
    #[test]
    fn an_inner_divisor_is_tested_first() {
        let inner = expr(value(1), vec![(ArithmeticOperator::Divide, value(2))]);
        let quotient = expr(
            value(0),
            vec![(
                ArithmeticOperator::Divide,
                FactorArgument::Group(Box::new(inner)),
            )],
        );
        assert_eq!(
            guard_of(&[quotient]),
            tokens(quote! { v.2 != 0 && ::flowlog_runtime::arith::div(v.1, v.2) != 0 })
        );
    }

    #[test]
    fn a_check_two_expressions_need_is_made_once() {
        let quotient = expr(value(0), vec![(ArithmeticOperator::Divide, value(1))]);
        let remainder = expr(value(2), vec![(ArithmeticOperator::Modulo, value(1))]);
        assert_eq!(
            guard_of(&[quotient, remainder]),
            tokens(quote! { v.1 != 0 })
        );
    }

    #[test]
    fn total_expressions_need_no_guard() {
        let sum = expr(value(0), vec![(ArithmeticOperator::Plus, value(1))]);
        let length = expr(builtin(BuiltinOperator::Strlen, vec![value(3)]), Vec::new());
        assert_eq!(guard_of(&[sum, length]), None);
    }

    // Cases: argument, guard.
    #[rstest]
    #[case::column(
        value(3),
        tokens(quote! { ::flowlog_runtime::builtin::to_number_defined(v.3.clone().as_str()) })
    )]
    #[case::number_literal(literal(DataType::String, "-42"), None)]
    #[case::text_literal(
        literal(DataType::String, "x"),
        tokens(quote! { ::flowlog_runtime::builtin::to_number_defined("x".to_string().as_str()) })
    )]
    fn to_number_is_tested_unless_its_literal_parses(
        #[case] arg: FactorArgument,
        #[case] guard: Option<String>,
    ) {
        let number = expr(builtin(BuiltinOperator::ToNumber, vec![arg]), Vec::new());
        assert_eq!(guard_of(&[number]), guard);
    }

    // Cases: start, len, guard.
    #[rstest]
    #[case::columns(
        value(0),
        value(1),
        tokens(quote! { ::flowlog_runtime::builtin::substr_defined(v.3.clone().as_str(), v.0, v.1) })
    )]
    #[case::literal_prefix(literal(DataType::Int32, "0"), literal(DataType::Int32, "2"), None)]
    #[case::literal_start_past_zero(
        literal(DataType::Int32, "1"),
        literal(DataType::Int32, "2"),
        tokens(quote! { ::flowlog_runtime::builtin::substr_defined(v.3.clone().as_str(), 1i32, 2i32) })
    )]
    #[case::negative_literal_start(
        literal(DataType::Int32, "-1"),
        literal(DataType::Int32, "2"),
        tokens(quote! { ::flowlog_runtime::builtin::substr_defined(v.3.clone().as_str(), -1i32, 2i32) })
    )]
    fn substr_is_tested_unless_it_takes_a_literal_prefix(
        #[case] start: FactorArgument,
        #[case] len: FactorArgument,
        #[case] guard: Option<String>,
    ) {
        let slice = expr(
            builtin(BuiltinOperator::Substr, vec![value(3), start, len]),
            Vec::new(),
        );
        assert_eq!(guard_of(&[slice]), guard);
    }

    /// A divisor computed by `to_number` is tested only after its text is.
    #[test]
    fn a_built_in_inside_a_divisor_is_tested_first() {
        let number = builtin(BuiltinOperator::ToNumber, vec![value(3)]);
        let quotient = expr(value(0), vec![(ArithmeticOperator::Divide, number)]);
        assert_eq!(
            guard_of(&[quotient]),
            tokens(quote! {
                ::flowlog_runtime::builtin::to_number_defined(v.3.clone().as_str())
                    && v.3.clone().as_str().parse::<i32>().unwrap_or(0) != 0
            })
        );
    }
}
