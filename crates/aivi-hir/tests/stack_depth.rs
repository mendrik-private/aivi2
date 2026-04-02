//! Stack-safety tests for HIR lowering and validation.
//!
//! These tests build adversarially deep AIVI source strings and run them
//! through the full lower → validate pipeline.
//! A test passes if it completes — panicking with a stack overflow IS a failure.

use aivi_base::SourceDatabase;
use aivi_hir::{ValidationMode, lower_module, validate_module};
use aivi_syntax::parse_module;

/// Run source through parse → lower → validate, ignoring all semantic errors.
/// The only invariant asserted is "no panic / stack overflow".
fn lower_and_validate_unchecked(name: &str, src: &str) {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(name, src);
    let parsed = parse_module(&sources[file_id]);
    let lowered = lower_module(&parsed.module);
    let _ = validate_module(lowered.module(), ValidationMode::Structural);
}

/// 200 chained signals: `signal s0 = 0`, `signal s1 = s0 + 1`, ...
/// Exercises signal-graph traversal depth.
fn long_signal_chain_source(n: usize) -> String {
    let mut s = String::from("signal s0 = 0\n");
    for i in 1..=n {
        s.push_str(&format!("signal s{i} = s{} + 1\n", i - 1));
    }
    s
}

/// A func with one pipe-case arm whose list pattern is nested N levels deep:
/// `[x0, [x1, [x2, [...leaf...]]]]`.
/// Exercises pattern-compilation depth during lowering.
fn deep_nested_list_pattern_source(depth: usize) -> String {
    let mut pattern = String::from("leaf");
    for i in 0..depth {
        pattern = format!("[x{i}, {pattern}]");
    }
    format!("func f = v => v\n ||> {pattern} -> 0\n ||> _ -> 1\n")
}

/// A func whose body is a 500-stage lambda pipe:
/// `func f = x => x |> a => a |> b => b |> ...`
/// Exercises pipe-stage traversal depth.
fn long_pipe_source(stages: usize) -> String {
    // Use distinct param names to keep the source unambiguous for the parser.
    let mut s = String::from("func f =\n    x0 => x0");
    for i in 1..=stages {
        s.push_str(&format!(" |> x{i} => x{i}"));
    }
    s.push('\n');
    s
}

#[test]
fn long_signal_chain_does_not_stack_overflow() {
    lower_and_validate_unchecked("signal_chain.aivi", &long_signal_chain_source(200));
}

#[test]
fn deep_nested_list_pattern_does_not_stack_overflow() {
    lower_and_validate_unchecked("deep_list_pattern.aivi", &deep_nested_list_pattern_source(100));
}

#[test]
fn long_pipe_chain_does_not_stack_overflow() {
    lower_and_validate_unchecked("long_pipe.aivi", &long_pipe_source(500));
}
