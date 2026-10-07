//! The same program compiles to the same code, every time.
//!
//! Planning and code generation iterate maps and sets throughout. When
//! those iterate in an order of their own (`std`'s per-process random
//! hasher), two compilations of one program emit their collections, joins
//! and strata in different orders, and a build is not reproducible.

use std::fs;
use std::path::PathBuf;

use flowlog_build::Builder;
use flowlog_common::SourceMap;

/// Enough relations, strata and operators that an unordered iteration
/// anywhere in the pipeline shows in the generated code.
fn program() -> String {
    let mut text = String::from(
        ".decl edge(x: symbol, y: symbol)\n.input edge\n\
         .decl weight(x: symbol, w: number)\n.input weight\n",
    );
    for i in 0..12 {
        text.push_str(&format!(
            ".decl reach{i}(x: symbol, y: symbol)\n\
             reach{i}(x, y) :- edge(x, y), x != \"n{i}\".\n\
             reach{i}(x, z) :- reach{i}(x, y), edge(y, z).\n\
             .decl open{i}(x: symbol)\n\
             open{i}(x) :- reach{i}(x, _), !reach{i}(_, x).\n\
             .decl heavy{i}(x: symbol, w: number)\n\
             heavy{i}(x, max(w)) :- weight(x, w), open{i}(x).\n\
             .output heavy{i}\n",
        ));
    }
    text
}

fn compile(dir: &PathBuf, intern: bool) -> String {
    let out = dir.join(format!("out-{}", rand_suffix()));
    fs::create_dir_all(&out).unwrap();
    let mut sm = SourceMap::new();
    Builder::default()
        .string_intern(intern)
        .compile_into(&dir.join("program.dl"), &out, &mut sm)
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    fs::read_to_string(out.join("program.rs")).unwrap()
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[test]
fn a_program_compiles_to_the_same_code_every_time() {
    let dir = std::env::temp_dir().join(format!(
        "flowlog_deterministic_{}_{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("program.dl"), program()).unwrap();

    for intern in [false, true] {
        let first = compile(&dir, intern);
        for _ in 0..4 {
            assert!(
                compile(&dir, intern) == first,
                "string_intern({intern}): a second compilation emitted other code"
            );
        }
    }

    fs::remove_dir_all(&dir).unwrap();
}
