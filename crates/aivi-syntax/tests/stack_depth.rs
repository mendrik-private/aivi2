//! Stack-safety tests for the AIVI parser and CST.
//!
//! These tests generate adversarially deep inputs and verify
//! the parser returns without a stack overflow.
//! A test passes if it completes — panicking with a stack overflow IS a failure.

use aivi_base::SourceDatabase;
use aivi_syntax::parse_module;

/// `value result = 0 |> x => x + 1 |> x => x + 1 ...` with N pipe stages.
fn deep_pipe_source(stages: usize) -> String {
    let mut s = String::from("value result = 0");
    for _ in 0..stages {
        s.push_str(" |> x => x + 1");
    }
    s
}

/// `type Deep = { field0: { field1: { ... Int } } }` nested N levels deep.
fn deep_record_type_source(depth: usize) -> String {
    let mut inner = String::from("Int");
    for i in 0..depth {
        inner = format!("{{ field{i}: {inner} }}");
    }
    format!("type Deep = {inner}")
}

/// A single pipe-case arm with a list pattern nested N levels deep:
/// `[x0, [x1, [x2, [...]]]]`.
fn deep_nested_list_pattern_source(depth: usize) -> String {
    // Build the innermost pattern and wrap outward.
    let mut pattern = String::from("leaf");
    for i in 0..depth {
        pattern = format!("[x{i}, {pattern}]");
    }
    // A func with one deep pattern arm and a wildcard fallback.
    format!("func f = v => v\n ||> {pattern} -> 0\n ||> _ -> 1\n")
}

fn parse_unchecked(name: &str, src: &str) {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file(name, src);
    // We deliberately ignore errors — the module may be semantically invalid
    // at depth.  The invariant under test is solely "no stack overflow".
    let _parsed = parse_module(&db[file_id]);
}

#[test]
fn parser_handles_deep_pipe_without_stack_overflow() {
    parse_unchecked("deep_pipe.aivi", &deep_pipe_source(1000));
}

#[test]
fn parser_handles_deep_record_type_without_stack_overflow() {
    parse_unchecked("deep_record.aivi", &deep_record_type_source(200));
}

#[test]
fn parser_handles_deep_nested_list_pattern_without_stack_overflow() {
    parse_unchecked("deep_list_pattern.aivi", &deep_nested_list_pattern_source(100));
}
