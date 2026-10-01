//! The program's boundary. [`relation`] declares each relation, [`input`]
//! owns the inputs' loaders, collections, and handles, and [`output`] the
//! inspectors and emitters the outputs go through; [`gen_relations`] bundles
//! what a frontend includes as its relation module.

pub(crate) mod input;
pub(crate) mod output;
pub(crate) mod relation;

use flowlog_parser::Program;
use proc_macro2::TokenStream;
use quote::quote;

use crate::CodegenError;
use crate::io::input::gen_inputs_container;
use crate::io::relation::gen_declaration;

/// Returns the relation module a frontend includes: a `Relation`
/// declaration for every input and output, then the `Inputs` container of
/// the inputs' loaders.
///
/// The generated code expects its parent module to supply `Ts`; interned
/// facts use the runtime's string pool.
pub fn gen_relations(program: &Program, string_intern: bool) -> Result<TokenStream, CodegenError> {
    let edbs = program.edbs();
    let outputs = program.idbs();
    let declarations = edbs
        .iter()
        .copied()
        .chain(outputs.into_iter().filter(|output| {
            !edbs
                .iter()
                .any(|input| input.fingerprint() == output.fingerprint())
        }))
        .map(|relation| gen_declaration(program, relation, string_intern))
        .collect::<Result<Vec<_>, _>>()?;
    let inputs = gen_inputs_container(&edbs, string_intern);
    Ok(quote! {
        use super::*;
        #(#declarations)*
        #inputs
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flowlog_common::Config;
    use flowlog_common::SourceMap;

    use super::*;

    fn generate(source: &str) -> String {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("program.dl");
        fs::write(&path, source).expect("program");
        let program = flowlog_parser::parse(
            path.to_str().expect("path"),
            &[],
            &mut SourceMap::default(),
            &mut Config::default(),
        )
        .expect("parse");
        gen_relations(&program, false)
            .expect("generate relations")
            .to_string()
    }

    #[test]
    fn a_relation_that_is_both_input_and_output_is_declared_once() {
        let generated = generate(".decl Edge(id: int32)\n.input Edge\n.output Edge\n");
        let declaration = quote! { impl ::flowlog_runtime::io::Relation for Reledge }.to_string();
        assert_eq!(generated.matches(&declaration).count(), 1, "{generated}");
    }
}
