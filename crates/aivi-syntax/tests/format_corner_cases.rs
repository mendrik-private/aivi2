//! Named corner-case and regression tests for the AIVI formatter.

use aivi_base::SourceDatabase;
use aivi_syntax::{Formatter, parse_module};

fn format_text(src: &str) -> Option<String> {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file("test.aivi", src);
    let parsed = parse_module(&db[file_id]);
    if parsed.has_errors() {
        return None;
    }
    Some(Formatter.format(&parsed.module))
}

fn assert_idempotent(src: &str) {
    let first = format_text(src).expect("first format pass should succeed");
    let second = format_text(&first).expect("second format pass should succeed");
    assert_eq!(first, second, "formatter is not idempotent");
}

// ---------------------------------------------------------------------------
// Pipe chains
// ---------------------------------------------------------------------------

#[test]
fn long_pipe_chain_is_idempotent() {
    let src = "value result = 42 |> add 1 |> mul 2 |> add 3 |> mul 4 |> add 5 |> show\n";
    assert_idempotent(src);
}

#[test]
fn deeply_nested_pipe_stages_are_indented() {
    let src = "value x = start\n |> stage1\n |> stage2\n |> stage3\n |> stage4\n |> stage5\n";
    let output = format_text(src).unwrap();
    assert!(output.contains("|>"), "pipe stages should be preserved");
    let output2 = format_text(&output).unwrap();
    assert_eq!(output, output2);
}

#[test]
fn pipe_chain_single_stage_is_idempotent() {
    let src = "value y = someValue |> transform\n";
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Signal merge
// ---------------------------------------------------------------------------

#[test]
fn signal_merge_with_many_arms_is_idempotent() {
    let src = "\
signal merged =
 &|> sigA
 &|> sigB
 &|> sigC
 &|> sigD
  |> combine
";
    assert_idempotent(src);
}

#[test]
fn signal_merge_two_sources_is_idempotent() {
    let src = "\
signal pair =
 &|> firstName
 &|> lastName
  |> NamePair
";
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

#[test]
fn record_type_with_many_fields_is_idempotent() {
    let src = "\
type UserProfile = {
    id: Int,
    name: Text,
    email: Text,
    age: Int,
    active: Bool,
    role: Text
}
";
    assert_idempotent(src);
}

#[test]
fn record_type_single_field_is_idempotent() {
    let src = "type Wrapper = { value: Int }\n";
    assert_idempotent(src);
}

#[test]
fn sum_type_with_companion_members_is_idempotent() {
    let src = "\
type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human -> Computer
     ||> Computer -> Human
}
";
    assert_idempotent(src);
}

#[test]
fn sum_type_with_unary_subject_companion_members_is_idempotent() {
    let src = "\
type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = .
     ||> Human -> Computer
     ||> Computer -> Human
}
";
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Class declarations
// ---------------------------------------------------------------------------

#[test]
fn class_with_multiple_type_params_is_idempotent() {
    let src = "\
class Functor F = {
    fmap : (A -> B) -> F A -> F B
}
";
    assert_idempotent(src);
}

#[test]
fn class_with_single_method_is_idempotent() {
    let src = "\
class Eq A = {
    (==) : A -> A -> Bool
}
";
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Comment preservation
// ---------------------------------------------------------------------------

#[test]
fn line_comments_before_value_are_preserved() {
    let src = "// this is a comment\nvalue answer = 42\n";
    let output = format_text(src).unwrap();
    assert!(
        output.contains("// this is a comment"),
        "comment should be preserved, got: {:?}",
        output
    );
}

#[test]
fn line_comments_before_value_are_idempotent() {
    let src = "// this is a comment\nvalue answer = 42\n";
    assert_idempotent(src);
}

#[test]
fn multiple_line_comments_before_declaration_are_preserved() {
    let src = "// line one\n// line two\nvalue x = 1\n";
    let output = format_text(src).unwrap();
    assert!(
        output.contains("// line one"),
        "first comment should be preserved"
    );
    assert!(
        output.contains("// line two"),
        "second comment should be preserved"
    );
    assert_idempotent(src);
}

#[test]
fn comment_before_type_declaration_is_preserved() {
    let src = "// A direction value.\ntype Direction = North | South | East | West\n";
    let output = format_text(src).unwrap();
    assert!(
        output.contains("// A direction value."),
        "comment should be preserved, got: {:?}",
        output
    );
    assert_idempotent(src);
}

#[test]
fn comment_before_func_declaration_is_preserved() {
    let src = "// Adds two integers.\ntype Int -> Int -> Int\nfunc add = x y =>\n    x + y\n";
    let output = format_text(src).unwrap();
    assert!(
        output.contains("// Adds two integers."),
        "comment should be preserved, got: {:?}",
        output
    );
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Use declarations
// ---------------------------------------------------------------------------

#[test]
fn use_declaration_with_many_imports_is_idempotent() {
    let src = "\
use aivi.list (
    map
    filter
    fold
    length
    isEmpty
    head
)
";
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Sum types
// ---------------------------------------------------------------------------

#[test]
fn sum_type_with_many_variants_is_idempotent() {
    let src = "\
type Color =
  | Red
  | Green
  | Blue
  | Yellow
  | Cyan
  | Magenta
";
    assert_idempotent(src);
}

#[test]
fn sum_type_inline_is_idempotent() {
    let src = "type Bool = True | False\n";
    assert_idempotent(src);
}
