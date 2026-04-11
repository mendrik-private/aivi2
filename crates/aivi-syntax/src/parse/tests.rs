use std::{fs, path::PathBuf};

use aivi_base::SourceDatabase;

use super::*;
use crate::{Formatter, ItemKind, TokenKind, lex_module};

fn load(input: &str) -> (SourceDatabase, ParsedModule) {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("test.aivi", input.to_owned());
    let parsed = {
        let file = &sources[file_id];
        parse_module(file)
    };
    (sources, parsed)
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/frontend/milestone-1")
}

fn parse_fixture(relative_path: &str) -> ParsedModule {
    let path = fixture_root().join(relative_path);
    let text = fs::read_to_string(&path).expect("fixture must be readable");
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    parse_module(&sources[file_id])
}

#[test]
fn lexer_recognizes_pipe_operators_class_keywords_and_regex_literals() {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(
            "operators.aivi",
            r#"class Eq A = {
    (==) : A -> A -> Bool
}
instance Eq Blob = {
    (==) left right = same left right
}
domain Duration over Int = {
    literal ms : Int -> Duration
    (*) : Duration -> Int -> Duration
}
signal flow = value |> compute ?|> ready ||> Ready -> keep *|> .email &|> build @|> loop <|@ step | debug <|* merge T|> start F|> stop
value same = left == right
value different = left != right
fun picked = value!
value quotient = left / right
value remainder = left % right
value range = 1..10
value chained =
    result {
        value <- Ok 1
        value
    }
<Label text={status} />
</match>
value datePattern = rx"\d{4}-\d{2}-\d{2}"
"#,
        );
    let file = &sources[file_id];
    let lexed = lex_module(file);
    let kinds: Vec<_> = lexed
        .tokens()
        .iter()
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.kind())
        .collect();

    assert!(kinds.contains(&TokenKind::ClassKw));
    assert!(kinds.contains(&TokenKind::InstanceKw));
    assert!(kinds.contains(&TokenKind::DomainKw));
    assert!(kinds.contains(&TokenKind::ThinArrow));
    assert!(kinds.contains(&TokenKind::EqualEqual));
    assert!(kinds.contains(&TokenKind::Bang));
    assert!(kinds.contains(&TokenKind::BangEqual));
    assert!(kinds.contains(&TokenKind::Star));
    assert!(kinds.contains(&TokenKind::Slash));
    assert!(kinds.contains(&TokenKind::Percent));
    assert!(kinds.contains(&TokenKind::DotDot));
    assert!(kinds.contains(&TokenKind::LeftArrow));
    assert!(kinds.contains(&TokenKind::PipeTransform));
    assert!(kinds.contains(&TokenKind::PipeGate));
    assert!(kinds.contains(&TokenKind::PipeCase));
    assert!(kinds.contains(&TokenKind::PipeMap));
    assert!(kinds.contains(&TokenKind::PipeApply));
    assert!(kinds.contains(&TokenKind::PipeRecurStart));
    assert!(kinds.contains(&TokenKind::PipeRecurStep));
    assert!(kinds.contains(&TokenKind::PipeTap));
    assert!(kinds.contains(&TokenKind::PipeFanIn));
    assert!(kinds.contains(&TokenKind::TruthyBranch));
    assert!(kinds.contains(&TokenKind::FalsyBranch));
    assert!(kinds.contains(&TokenKind::SelfCloseTagEnd));
    assert!(kinds.contains(&TokenKind::CloseTagStart));
    assert!(kinds.contains(&TokenKind::RegexLiteral));
    assert!(lexed.diagnostics().is_empty());
}

#[test]
fn parser_preserves_bare_root_patch_field_selectors() {
    let (_, parsed) = load("value promote = patch { isAdmin: True }\n");
    assert!(
        !parsed.has_errors(),
        "expected patch shorthand to parse cleanly, got diagnostics: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Some(Item::Value(item)) = parsed.module.items().first() else {
        panic!("expected a value item");
    };
    let Some(expr) = item.expr_body() else {
        panic!("expected the value item to have an expression body");
    };
    let ExprKind::PatchLiteral(patch) = &expr.kind else {
        panic!("expected the value body to be a patch literal");
    };
    let Some(entry) = patch.entries.first() else {
        panic!("expected the patch literal to contain one entry");
    };
    let [PatchSelectorSegment::Named { name, dotted, .. }] = entry.selector.segments.as_slice()
    else {
        panic!("expected one named selector segment");
    };

    assert_eq!(name.text, "isAdmin");
    assert!(
        !*dotted,
        "expected the root field selector to stay undotted"
    );
}

#[test]
fn lexer_distinguishes_line_and_doc_comments_as_trivia() {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(
        "comments.aivi",
        "/** module doc **/\nvalue answer = 42 // inline note\n",
    );
    let lexed = lex_module(&sources[file_id]);
    let comment_kinds = lexed
        .tokens()
        .iter()
        .filter_map(|token| match token.kind() {
            TokenKind::DocComment | TokenKind::LineComment => Some(token.kind()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        comment_kinds,
        vec![TokenKind::DocComment, TokenKind::LineComment]
    );
    assert!(comment_kinds.iter().all(|kind| kind.is_trivia()));
    assert!(lexed.diagnostics().is_empty());
}

#[test]
fn parser_preserves_comments_between_top_level_items() {
    let (_, parsed) = load(
        r#"signal one : Signal Int

// keep this comment with the following signal
signal two : Signal Int
"#,
    );

    assert!(!parsed.has_errors());
    let Item::Signal(item) = &parsed.module.items[1] else {
        panic!("expected second item to be a signal");
    };
    assert_eq!(
        item.base.leading_comments,
        vec!["// keep this comment with the following signal"]
    );

    let formatted = Formatter.format(&parsed.module);
    assert!(formatted.contains("// keep this comment with the following signal"));
}

#[test]
fn parser_builds_structured_items_and_source_decorators() {
    let (_, parsed) = load(
        r#"@source http.get "/users" with {
    decode: Strict,
    retry: 3
}
signal users : Signal User

type Bool = True | False
value answer = 42
fun add: Int = x:Int y:Int => x + y
use aivi.network (
    http
)
export main
"#,
    );

    assert!(!parsed.has_errors());
    assert_eq!(parsed.module.items.len(), 6);
    assert_eq!(parsed.module.items[0].kind(), ItemKind::Signal);
    assert_eq!(parsed.module.items[1].kind(), ItemKind::Type);
    assert_eq!(parsed.module.items[2].kind(), ItemKind::Value);
    assert_eq!(parsed.module.items[3].kind(), ItemKind::Fun);
    assert_eq!(parsed.module.items[4].kind(), ItemKind::Use);
    assert_eq!(parsed.module.items[5].kind(), ItemKind::Export);

    match &parsed.module.items[0] {
        Item::Signal(item) => {
            assert_eq!(item.base.decorators.len(), 1);
            assert_eq!(item.base.decorators[0].name.as_dotted(), "source");
            assert_eq!(
                item.name.as_ref().map(|name| name.text.as_str()),
                Some("users")
            );
            match &item.base.decorators[0].payload {
                DecoratorPayload::Source(source) => {
                    assert_eq!(
                        source
                            .provider
                            .as_ref()
                            .map(QualifiedName::as_dotted)
                            .as_deref(),
                        Some("http.get")
                    );
                    assert_eq!(source.arguments.len(), 1);
                    assert!(source.options.is_some());
                }
                other => panic!("expected source decorator, got {other:?}"),
            }
        }
        other => panic!("expected a signal item, got {other:?}"),
    }

    match &parsed.module.items[1] {
        Item::Type(item) => match item.type_body() {
            Some(TypeDeclBody::Sum(sum)) => assert_eq!(sum.variants.len(), 2),
            other => panic!("expected sum type body, got {other:?}"),
        },
        other => panic!("expected type item, got {other:?}"),
    }

    match &parsed.module.items[3] {
        Item::Fun(item) => {
            assert!(!item.parameters.is_empty());
            assert!(matches!(
                item.expr_body().map(|expr| &expr.kind),
                Some(ExprKind::Binary { .. })
            ));
        }
        other => panic!("expected fun item with parameters, got {other:?}"),
    }

    match &parsed.module.items[4] {
        Item::Use(item) => {
            assert_eq!(
                item.path.as_ref().map(QualifiedName::as_dotted).as_deref(),
                Some("aivi.network")
            );
            assert_eq!(item.imports.len(), 1);
            assert_eq!(item.imports[0].path.as_dotted(), "http");
            assert!(item.imports[0].alias.is_none());
        }
        other => panic!("expected use item, got {other:?}"),
    }
}

#[test]
fn parser_builds_sum_type_companions_inside_brace_bodies() {
    let (_, parsed) = load(
        r#"type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Type(item) = &parsed.module.items[0] else {
        panic!("expected type item");
    };
    let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
        panic!("expected sum type body");
    };
    assert_eq!(sum.variants.len(), 2);
    assert_eq!(sum.companions.len(), 1);
    assert_eq!(sum.companions[0].name.text, "opponent");
    assert_eq!(
        sum.companions[0].function_form,
        FunctionSurfaceForm::Explicit
    );
    assert_eq!(sum.companions[0].parameters.len(), 1);
    assert!(sum.companions[0].annotation.is_some());
    assert!(sum.companions[0].body.is_some());
}

#[test]
fn parser_builds_sum_type_companions_with_unary_subject_sugar() {
    let (_, parsed) = load(
        r#"type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = .
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Type(item) = &parsed.module.items[0] else {
        panic!("expected type item");
    };
    let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
        panic!("expected sum type body");
    };
    assert_eq!(sum.companions.len(), 1);
    assert_eq!(
        sum.companions[0].function_form,
        FunctionSurfaceForm::UnarySubjectSugar
    );
    assert_eq!(sum.companions[0].parameters.len(), 1);
    assert!(sum.companions[0].annotation.is_some());
    assert!(sum.companions[0].body.is_some());
}

#[test]
fn parser_builds_inline_annotated_sum_type_companions_with_unary_subject_sugar() {
    let (_, parsed) = load(
        r#"type Player = {
    | Human
    | Computer

    opponent: Player -> Player = .
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Type(item) = &parsed.module.items[0] else {
        panic!("expected type item");
    };
    let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
        panic!("expected sum type body");
    };
    assert_eq!(sum.companions.len(), 1);
    assert_eq!(sum.companions[0].name.text, "opponent");
    assert_eq!(
        sum.companions[0].function_form,
        FunctionSurfaceForm::UnarySubjectSugar
    );
    assert_eq!(sum.companions[0].parameters.len(), 1);
    assert!(sum.companions[0].annotation.is_some());
    assert!(sum.companions[0].body.is_some());
}

#[test]
fn parser_builds_result_blocks_with_bindings_and_tail() {
    let (_, parsed) = load(
        r#"value total =
result {
        left <- Ok 20
        right <- Ok 22
        left + right
    }
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Value(item) = &parsed.module.items[0] else {
        panic!("expected value item");
    };
    let ExprKind::ResultBlock(block) = &item.expr_body().expect("value body").kind else {
        panic!("expected result block body");
    };
    assert_eq!(block.bindings.len(), 2);
    assert_eq!(block.bindings[0].name.text, "left");
    assert_eq!(block.bindings[1].name.text, "right");
    assert!(matches!(
        block.tail.as_deref().map(|expr| &expr.kind),
        Some(ExprKind::Binary { .. })
    ));
}

#[test]
fn parser_builds_single_source_signal_merge() {
    let (_, parsed) = load(
        r#"signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    assert_eq!(parsed.module.items.len(), 1);
    assert_eq!(parsed.module.items[0].kind(), ItemKind::Signal);

    let Item::Signal(item) = &parsed.module.items[0] else {
        panic!("expected signal item");
    };
    let merge = item.merge_body().expect("expected merge body");
    assert_eq!(merge.sources.len(), 1);
    assert_eq!(merge.sources[0].text, "ready");
    assert_eq!(merge.arms.len(), 2);
    assert!(merge.arms[0].source.is_none());
    assert!(merge.arms[1].source.is_none());
}

#[test]
fn parser_builds_multi_source_signal_merge() {
    let (_, parsed) = load(
        r#"signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowUp") => Turn North
  ||> _ => Tick
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Signal(item) = &parsed.module.items[0] else {
        panic!("expected signal item");
    };
    let merge = item.merge_body().expect("expected merge body");
    assert_eq!(merge.sources.len(), 2);
    assert_eq!(merge.sources[0].text, "tick");
    assert_eq!(merge.sources[1].text, "keyDown");
    assert_eq!(merge.arms.len(), 3);
    assert_eq!(
        merge.arms[0].source.as_ref().map(|s| s.text.as_str()),
        Some("tick")
    );
    assert_eq!(
        merge.arms[1].source.as_ref().map(|s| s.text.as_str()),
        Some("keyDown")
    );
    // Default arm has no source prefix
    assert!(merge.arms[2].source.is_none());
}

#[test]
fn parser_builds_signal_merge_with_expression_body() {
    let (_, parsed) = load(
        r#"signal total : Signal Int = ready
  ||> True => left + right
  ||> _ => 0
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Signal(item) = &parsed.module.items[0] else {
        panic!("expected signal item");
    };
    let merge = item.merge_body().expect("expected merge body");
    assert_eq!(merge.sources.len(), 1);
    assert_eq!(merge.arms.len(), 2);
    assert!(matches!(
        merge.arms[0].body.as_ref().map(|e| &e.kind),
        Some(ExprKind::Binary { .. })
    ));
}

#[test]
fn parser_distinguishes_signal_merge_from_pipe_expression() {
    let (_, parsed) = load(
        r#"signal derived = someSignal |> transform
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Signal(item) = &parsed.module.items[0] else {
        panic!("expected signal item");
    };
    // This should be an Expr body, not a Merge body.
    assert!(item.expr_body().is_some());
    assert!(item.merge_body().is_none());
}

#[test]
fn parser_builds_multiline_accumulate_pipe_signal_bodies() {
    let (_, parsed) = load(
        r#"type Key =
  | Left
type Direction =
  | East
fun updateDirection:Direction = key:Key current:Direction => current
signal keyDown: Signal Key = Left
signal direction: Signal Direction = keyDown
 +|> East updateDirection
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Signal(item) = &parsed.module.items[4] else {
        panic!("expected signal item");
    };
    let ExprKind::Pipe(pipe) = &item.expr_body().expect("signal body").kind else {
        panic!("expected signal body to parse as a pipe");
    };
    assert!(matches!(
        pipe.head.as_deref().map(|expr| &expr.kind),
        Some(ExprKind::Name(identifier)) if identifier.text == "keyDown"
    ));
    assert_eq!(pipe.stages.len(), 1);
    let PipeStageKind::Accumulate { seed, step } = &pipe.stages[0].kind else {
        panic!("expected accumulate pipe stage");
    };
    assert!(matches!(seed.kind, ExprKind::Name(ref identifier) if identifier.text == "East"));
    assert!(
        matches!(step.kind, ExprKind::Name(ref identifier) if identifier.text == "updateDirection")
    );
}

#[test]
fn parser_builds_delay_and_burst_pipe_signal_bodies() {
    let (_, parsed) = load(
        r#"signal clicks: Signal Int = 1
signal delayed: Signal Int = clicks
 |> delay 200ms
signal flashed: Signal Int = clicks
 |> burst 75ms 3times
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Signal(delayed) = &parsed.module.items[1] else {
        panic!("expected delayed signal item");
    };
    let ExprKind::Pipe(delay_pipe) = &delayed.expr_body().expect("delay body").kind else {
        panic!("expected delayed signal body to parse as a pipe");
    };
    let PipeStageKind::Delay { duration } = &delay_pipe.stages[0].kind else {
        panic!("expected delay pipe stage");
    };
    assert!(matches!(duration.kind, ExprKind::SuffixedInteger(_)));

    let Item::Signal(flashed) = &parsed.module.items[2] else {
        panic!("expected flashed signal item");
    };
    let ExprKind::Pipe(burst_pipe) = &flashed.expr_body().expect("burst body").kind else {
        panic!("expected burst signal body to parse as a pipe");
    };
    let PipeStageKind::Burst { every, count } = &burst_pipe.stages[0].kind else {
        panic!("expected burst pipe stage");
    };
    assert!(matches!(every.kind, ExprKind::SuffixedInteger(_)));
    assert!(matches!(count.kind, ExprKind::SuffixedInteger(_)));
}

#[test]
fn parser_reports_removed_temporal_pipe_operator_spellings() {
    let (_, parsed) = load(
        r#"signal clicks: Signal Int = 1
signal delayed: Signal Int = clicks
 delay|> 200ms
signal flashed: Signal Int = clicks
 burst|> 75ms 3times
"#,
    );

    assert!(parsed.has_errors());
    assert!(
        parsed
            .all_diagnostics()
            .any(|diagnostic| diagnostic.code == Some(REMOVED_TEMPORAL_PIPE_OPERATOR))
    );
}

#[test]
fn parser_builds_from_signal_fanout_entries() {
    let (_, parsed) = load(
        r#"from state = {
    boardText: renderBoard
    dirLine: .dir |> dirLabel
    gameOver: .status
        ||> Running -> False
        ||> GameOver -> True
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    assert_eq!(parsed.module.items.len(), 1);

    let Item::From(item) = &parsed.module.items[0] else {
        panic!("expected from item");
    };
    assert!(matches!(
        item.source.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Name(name)) if name.text == "state"
    ));
    assert_eq!(item.entries.len(), 3);
    assert_eq!(item.entries[0].name.text, "boardText");
    assert_eq!(item.entries[1].name.text, "dirLine");
    assert_eq!(item.entries[2].name.text, "gameOver");
    assert!(matches!(
        item.entries[1].body.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Pipe(_))
    ));
    assert!(matches!(
        item.entries[2].body.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Pipe(_))
    ));
}

#[test]
fn parser_builds_parameterized_from_entries_with_attached_type_lines() {
    let (_, parsed) = load(
        r#"from state = {
    type Int -> Bool
    atLeast threshold: .score >= threshold
    type Bool
    readyNow: .ready
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::From(item) = &parsed.module.items[0] else {
        panic!("expected from item");
    };
    assert_eq!(item.entries.len(), 2);

    let at_least = &item.entries[0];
    assert_eq!(at_least.name.text, "atLeast");
    assert_eq!(at_least.parameters.len(), 1);
    assert_eq!(
        at_least.parameters[0]
            .name
            .as_ref()
            .expect("parameter name")
            .text,
        "threshold"
    );
    assert!(at_least.constraints.is_empty());
    assert!(matches!(
        at_least
            .annotation
            .as_ref()
            .map(|annotation| &annotation.kind),
        Some(TypeExprKind::Arrow { .. })
    ));

    let ready_now = &item.entries[1];
    assert_eq!(ready_now.name.text, "readyNow");
    assert!(ready_now.parameters.is_empty());
    assert!(matches!(
        ready_now.annotation.as_ref().map(|annotation| &annotation.kind),
        Some(TypeExprKind::Name(name)) if name.text == "Bool"
    ));
}

#[test]
fn parser_allows_result_blocks_to_use_the_last_binding_as_the_implicit_tail() {
    let (_, parsed) = load(
        r#"value lastValue =
    result {
        payload <- Ok 42
    }
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Value(item) = &parsed.module.items[0] else {
        panic!("expected value item");
    };
    let ExprKind::ResultBlock(block) = &item.expr_body().expect("value body").kind else {
        panic!("expected result block body");
    };
    assert_eq!(block.bindings.len(), 1);
    assert!(block.tail.is_none(), "tail should stay implicit in the CST");
}

#[test]
fn parser_builds_use_import_aliases() {
    let (_, parsed) = load(
        r#"use aivi.network (
    http as primaryHttp
    Request as HttpRequest
)
"#,
    );

    assert!(!parsed.has_errors());
    let Item::Use(item) = &parsed.module.items[0] else {
        panic!("expected use item");
    };
    assert_eq!(item.imports.len(), 2);
    assert_eq!(item.imports[0].path.as_dotted(), "http");
    assert_eq!(
        item.imports[0]
            .alias
            .as_ref()
            .map(|alias| alias.text.as_str()),
        Some("primaryHttp")
    );
    assert_eq!(item.imports[1].path.as_dotted(), "Request");
    assert_eq!(
        item.imports[1]
            .alias
            .as_ref()
            .map(|alias| alias.text.as_str()),
        Some("HttpRequest")
    );
}

#[test]
fn parser_builds_grouped_exports() {
    let (_, parsed) = load(
        r#"export (bundledSupportSentinel, BundledSupportToken)
"#,
    );

    assert!(!parsed.has_errors());
    let Item::Export(item) = &parsed.module.items[0] else {
        panic!("expected export item");
    };
    assert_eq!(
        item.targets
            .iter()
            .map(|target| target.text.as_str())
            .collect::<Vec<_>>(),
        vec!["bundledSupportSentinel", "BundledSupportToken"]
    );
}

#[test]
fn lexer_treats_removed_top_level_aliases_as_identifiers() {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("aliases.aivi", "source view result adapter data");
    let lexed = lex_module(&sources[file_id]);
    let kinds: Vec<_> = lexed
        .tokens()
        .iter()
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.kind())
        .collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
        ]
    );
}

#[test]
fn parser_rejects_removed_top_level_alias_declarations() {
    let (_, parsed) = load(
        "source ticks : Signal Int\nview main = 0\nresult bundle = 0\nadapter glue = 0\ndata Flag = On | Off\n",
    );

    assert!(
        parsed.has_errors(),
        "removed alias declarations should stay invalid"
    );
}

#[test]
fn parser_structures_text_interpolation_segments() {
    let (_, parsed) = load(r#"value greeting = "Hello {name}, use \{literal\} braces""#);

    assert!(!parsed.has_errors());
    match &parsed.module.items[0] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Text(text)) => {
                assert_eq!(text.segments.len(), 3);
                assert!(matches!(
                    &text.segments[0],
                    TextSegment::Text(fragment) if fragment.raw == "Hello "
                ));
                assert!(matches!(
                    &text.segments[1],
                    TextSegment::Interpolation(interpolation)
                        if matches!(interpolation.expr.kind, ExprKind::Name(ref identifier) if identifier.text == "name")
                ));
                assert!(matches!(
                    &text.segments[2],
                    TextSegment::Text(fragment)
                        if fragment.raw == ", use {literal} braces"
                ));
            }
            other => panic!("expected interpolated text literal, got {other:?}"),
        },
        other => panic!("expected value item, got {other:?}"),
    }
}

#[test]
fn parser_decodes_text_escape_sequences() {
    let (_, parsed) = load(r#"value board = "top\nbottom \u{41} \x42 \{ok\}""#);

    assert!(!parsed.has_errors());
    match &parsed.module.items[0] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Text(text)) => {
                assert_eq!(text.segments.len(), 1);
                assert!(matches!(
                    &text.segments[0],
                    TextSegment::Text(fragment)
                        if fragment.raw == "top\nbottom A B {ok}"
                ));
            }
            other => panic!("expected text literal, got {other:?}"),
        },
        other => panic!("expected value item, got {other:?}"),
    }
}

#[test]
fn parser_builds_class_members_and_equality_operators_from_fixture() {
    let parsed = parse_fixture("valid/top-level/class_eq.aivi");

    assert!(!parsed.has_errors());
    assert_eq!(parsed.module.items.len(), 2);
    assert_eq!(parsed.module.items[0].kind(), ItemKind::Class);

    match &parsed.module.items[0] {
        Item::Class(item) => {
            assert_eq!(
                item.name.as_ref().map(|name| name.text.as_str()),
                Some("Eq")
            );
            assert_eq!(
                item.type_parameters
                    .iter()
                    .map(|parameter| parameter.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["A"]
            );
            let body = item.class_body().expect("class item should have a body");
            assert_eq!(body.members.len(), 1);
            assert!(matches!(
                body.members[0].name,
                ClassMemberName::Operator(ref operator) if operator.text == "=="
            ));
            assert!(matches!(
                body.members[0].annotation.as_ref().map(|ty| &ty.kind),
                Some(TypeExprKind::Arrow { .. })
            ));
        }
        other => panic!("expected class item, got {other:?}"),
    }

    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator: BinaryOperator::And,
                left,
                right,
            }) => {
                assert!(matches!(
                    left.kind,
                    ExprKind::Binary {
                        operator: BinaryOperator::Equals,
                        ..
                    }
                ));
                assert!(matches!(
                    right.kind,
                    ExprKind::Binary {
                        operator: BinaryOperator::NotEquals,
                        ..
                    }
                ));
            }
            other => panic!("expected `and` root with equality subexpressions, got {other:?}"),
        },
        Item::Fun(_) => {}
        other => panic!("expected function item, got {other:?}"),
    }
}

#[test]
fn parser_respects_binary_precedence_and_left_associativity() {
    let (_, parsed) = load(
        "value ranked = left + middle > threshold and ready or fallback\nvalue diff = a - b - c\n",
    );

    assert!(!parsed.has_errors());

    match &parsed.module.items[0] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator: BinaryOperator::Or,
                left,
                right,
            }) => {
                assert!(matches!(
                    &right.kind,
                    ExprKind::Name(identifier) if identifier.text == "fallback"
                ));
                match &left.kind {
                    ExprKind::Binary {
                        operator: BinaryOperator::And,
                        left,
                        right,
                    } => {
                        assert!(matches!(
                            &right.kind,
                            ExprKind::Name(identifier) if identifier.text == "ready"
                        ));
                        match &left.kind {
                            ExprKind::Binary {
                                operator: BinaryOperator::GreaterThan,
                                left,
                                right,
                            } => {
                                assert!(matches!(
                                    &right.kind,
                                    ExprKind::Name(identifier) if identifier.text == "threshold"
                                ));
                                assert!(matches!(
                                    &left.kind,
                                    ExprKind::Binary {
                                        operator: BinaryOperator::Add,
                                        ..
                                    }
                                ));
                            }
                            other => panic!("expected comparison before `and`, got {other:?}"),
                        }
                    }
                    other => panic!("expected `and` before `or`, got {other:?}"),
                }
            }
            other => panic!("expected precedence-shaped binary tree, got {other:?}"),
        },
        other => panic!("expected ranked value item, got {other:?}"),
    }

    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator: BinaryOperator::Subtract,
                left,
                right,
            }) => {
                assert!(matches!(
                    &right.kind,
                    ExprKind::Name(identifier) if identifier.text == "c"
                ));
                assert!(matches!(
                    &left.kind,
                    ExprKind::Binary {
                        operator: BinaryOperator::Subtract,
                        ..
                    }
                ));
            }
            other => panic!("expected left-associative subtraction tree, got {other:?}"),
        },
        other => panic!("expected diff value item, got {other:?}"),
    }
}

#[test]
fn parser_respects_multiplicative_precedence_and_left_associativity() {
    let (_, parsed) =
        load("value total = base + rate * scale\nvalue grouped = total / count % bucket\n");

    assert!(!parsed.has_errors());

    match &parsed.module.items[0] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator: BinaryOperator::Add,
                left,
                right,
            }) => {
                assert!(matches!(
                    &left.kind,
                    ExprKind::Name(identifier) if identifier.text == "base"
                ));
                assert!(matches!(
                    &right.kind,
                    ExprKind::Binary {
                        operator: BinaryOperator::Multiply,
                        ..
                    }
                ));
            }
            other => panic!("expected additive root with multiplicative rhs, got {other:?}"),
        },
        other => panic!("expected total value item, got {other:?}"),
    }

    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator: BinaryOperator::Modulo,
                left,
                right,
            }) => {
                assert!(matches!(
                    &right.kind,
                    ExprKind::Name(identifier) if identifier.text == "bucket"
                ));
                assert!(matches!(
                    &left.kind,
                    ExprKind::Binary {
                        operator: BinaryOperator::Divide,
                        ..
                    }
                ));
            }
            other => panic!("expected left-associative multiplicative tree, got {other:?}"),
        },
        other => panic!("expected grouped value item, got {other:?}"),
    }
}

#[test]
fn parser_builds_instance_members_with_parameters_and_multiline_bodies() {
    let (_, parsed) = load(
        r#"class Eq A = {
    (==) : A -> A -> Bool
}

fun same:Bool = left:Blob right:Blob => True

instance Eq Blob = {
    (==) left right =
        same left right
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "{:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    assert_eq!(parsed.module.items[2].kind(), ItemKind::Instance);

    let Item::Instance(item) = &parsed.module.items[2] else {
        panic!("expected instance item");
    };
    assert_eq!(
        item.class.as_ref().map(QualifiedName::as_dotted).as_deref(),
        Some("Eq")
    );
    assert!(matches!(
        item.target.as_ref().map(|ty| &ty.kind),
        Some(TypeExprKind::Name(name)) if name.text == "Blob"
    ));
    let body = item.body.as_ref().expect("instance should have a body");
    assert_eq!(body.members.len(), 1);
    assert!(matches!(
        body.members[0].name,
        ClassMemberName::Operator(ref operator) if operator.text == "=="
    ));
    assert_eq!(
        body.members[0]
            .parameters
            .iter()
            .map(|parameter| parameter.text.as_str())
            .collect::<Vec<_>>(),
        vec!["left", "right"]
    );
    assert!(matches!(
        body.members[0].body.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Apply { .. })
    ));
}

#[test]
fn parser_builds_domain_members_from_fixture() {
    let parsed = parse_fixture("valid/top-level/domains.aivi");

    assert!(!parsed.has_errors());
    match &parsed.module.items[1] {
        Item::Domain(item) => {
            assert_eq!(
                item.name.as_ref().map(|name| name.text.as_str()),
                Some("Path")
            );
            assert!(matches!(
                item.carrier.as_ref().map(|carrier| &carrier.kind),
                Some(TypeExprKind::Name(identifier)) if identifier.text == "Text"
            ));
            let body = item.body.as_ref().expect("domain should have a body");
            assert_eq!(body.members.len(), 2);
            assert!(matches!(
                body.members[0].name,
                DomainMemberName::Literal(ref suffix) if suffix.text == "root"
            ));
            assert!(matches!(
                body.members[1].name,
                DomainMemberName::Signature(ClassMemberName::Operator(ref operator))
                    if operator.text == "/"
            ));
        }
        other => panic!("expected domain item, got {other:?}"),
    }
}

#[test]
fn parser_does_not_treat_thin_arrow_as_constraint_separator() {
    // Standalone type annotations starting with (MultiCharApply ...) ->
    // must parse as function types, NOT as constrained types.
    let (_, parsed) = load(concat!(
        "type (List A) -> (Option A) -> (List A)\n",
        "func appendPrev = items prev => items\n",
    ));
    assert!(
        !parsed.has_errors(),
        "standalone function type with (List A) -> should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    // The standalone `type` attaches to the `func`, so items[0] is Func
    let Item::Fun(func_item) = &parsed.module.items[0] else {
        panic!("expected func item with attached type annotation");
    };
    assert!(
        func_item.constraints.is_empty(),
        "expected no constraints — (List A) is a type constructor, not a class constraint"
    );
}

#[test]
fn parser_tracks_constraint_prefixes_on_functions_and_instances() {
    let (_, parsed) = load(
        r#"class Functor F = {
    map : (A -> B) -> F A -> F B
}
fun same:Eq A => Bool = v:A => v == v
instance Eq A => Eq (Option A) = {
    (==) left right = True
}
"#,
    );
    assert!(
        !parsed.has_errors(),
        "expected constrained signatures to parse cleanly, got diagnostics: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Fun(function) = &parsed.module.items[1] else {
        panic!("expected constrained function item");
    };
    assert_eq!(function.constraints.len(), 1);

    let Item::Instance(instance) = &parsed.module.items[2] else {
        panic!("expected constrained instance item");
    };
    assert_eq!(instance.context.len(), 1);
}

#[test]
fn parser_desugars_type_level_record_row_pipes_into_nested_applications() {
    let (_, parsed) = load(concat!(
        "type User = { id: Int, name: Text, createdAt: Text }\n",
        "type Public = User |> Pick (id, createdAt) |> Rename { createdAt: created_at }\n",
    ));
    assert!(
        !parsed.has_errors(),
        "record row transform types should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Type(item) = &parsed.module.items[1] else {
        panic!("expected second item to be a type alias");
    };
    let Some(TypeDeclBody::Alias(alias)) = item.type_body() else {
        panic!("expected type alias body");
    };

    let TypeExprKind::Apply { callee, arguments } = &alias.kind else {
        panic!("expected piped transform to desugar into an application");
    };
    let TypeExprKind::Name(name) = &callee.kind else {
        panic!("expected outer transform callee to be a name");
    };
    assert_eq!(name.text, "Rename");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(arguments[0].kind, TypeExprKind::Record(_)));

    let TypeExprKind::Apply {
        callee: inner_callee,
        arguments: inner_arguments,
    } = &arguments[1].kind
    else {
        panic!("expected inner piped transform to stay nested");
    };
    let TypeExprKind::Name(inner_name) = &inner_callee.kind else {
        panic!("expected inner transform callee to be a name");
    };
    assert_eq!(inner_name.text, "Pick");
    assert_eq!(inner_arguments.len(), 2);
    assert!(matches!(inner_arguments[0].kind, TypeExprKind::Tuple(_)));
    assert!(matches!(
        &inner_arguments[1].kind,
        TypeExprKind::Name(name) if name.text == "User"
    ));
}

#[test]
fn parser_tracks_constraint_prefixes_on_class_members() {
    let (_, parsed) = load(
        r#"class Functor F = {
    map:Applicative G=>(A -> G B) -> F A -> G (F B)
}
"#,
    );
    assert!(
        !parsed.has_errors(),
        "expected constrained class member to parse cleanly, got diagnostics: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Class(class_item) = &parsed.module.items[0] else {
        panic!("expected class item");
    };
    let body = class_item
        .class_body()
        .expect("class item should have a body");
    assert_eq!(body.members.len(), 1);
    assert_eq!(body.members[0].constraints.len(), 1);
    assert!(
        body.members[0].annotation.is_some(),
        "expected class member annotation, got {:?}",
        body.members[0]
    );
}

#[test]
fn parser_rejects_class_head_constraint_prefixes() {
    let (_, parsed) = load(
        r#"class (Functor F, Foldable F) -> Traversable F = {
    traverse : Applicative G -> (A -> G B) -> F A -> G (F B)
}
"#,
    );

    assert!(
        parsed.has_errors(),
        "expected class-head constraint prefixes to be rejected"
    );
    assert!(
        parsed
            .all_diagnostics()
            .any(|diagnostic| diagnostic.code == Some(UNSUPPORTED_CLASS_HEAD_CONSTRAINTS)),
        "expected unsupported class-head constraint diagnostic, got: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
}

#[test]
fn parser_builds_provider_contract_members_from_fixture() {
    let parsed = parse_fixture("valid/top-level/provider_contracts.aivi");

    assert!(!parsed.has_errors());
    assert_eq!(
        parsed.module.items[0].kind(),
        ItemKind::SourceProviderContract
    );
    match &parsed.module.items[0] {
        Item::SourceProviderContract(item) => {
            assert_eq!(
                item.provider
                    .as_ref()
                    .map(QualifiedName::as_dotted)
                    .as_deref(),
                Some("custom.feed")
            );
            let body = item
                .body
                .as_ref()
                .expect("provider contract should have a body");
            assert_eq!(body.members.len(), 5);
            match &body.members[0] {
                SourceProviderContractMember::ArgumentSchema(member) => {
                    assert_eq!(
                        member.name.as_ref().map(|name| name.text.as_str()),
                        Some("path")
                    );
                }
                other => panic!("expected argument schema member, got {other:?}"),
            }
            match &body.members[1] {
                SourceProviderContractMember::OptionSchema(member) => {
                    assert_eq!(
                        member.name.as_ref().map(|name| name.text.as_str()),
                        Some("timeout")
                    );
                }
                other => panic!("expected option schema member, got {other:?}"),
            }
            match &body.members[2] {
                SourceProviderContractMember::OperationSchema(member) => {
                    assert_eq!(
                        member.name.as_ref().map(|name| name.text.as_str()),
                        Some("read")
                    );
                }
                other => panic!("expected operation schema member, got {other:?}"),
            }
            match &body.members[3] {
                SourceProviderContractMember::CommandSchema(member) => {
                    assert_eq!(
                        member.name.as_ref().map(|name| name.text.as_str()),
                        Some("delete")
                    );
                }
                other => panic!("expected command schema member, got {other:?}"),
            }
            match &body.members[4] {
                SourceProviderContractMember::FieldValue(member) => {
                    assert_eq!(
                        member.name.as_ref().map(|name| name.text.as_str()),
                        Some("wakeup")
                    );
                    assert_eq!(
                        member.value.as_ref().map(|value| value.text.as_str()),
                        Some("providerTrigger")
                    );
                }
                other => panic!("expected wakeup field member, got {other:?}"),
            }
        }
        other => panic!("expected provider contract item, got {other:?}"),
    }
}

#[test]
fn parser_distinguishes_compact_literal_suffixes_from_spaced_application() {
    let (_, parsed) = load(
        "domain Duration over Int = {\n    literal ms : Int -> Duration\n}\nvalue compact = 250ms\nvalue spaced = 250 ms\n",
    );

    assert!(!parsed.has_errors());
    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::SuffixedInteger(literal)) => {
                assert_eq!(literal.literal.raw, "250");
                assert_eq!(literal.suffix.text, "ms");
            }
            other => panic!("expected compact suffixed integer, got {other:?}"),
        },
        other => panic!("expected compact value item, got {other:?}"),
    }

    match &parsed.module.items[2] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Apply { callee, arguments }) => {
                assert!(matches!(callee.kind, ExprKind::Integer(_)));
                assert_eq!(arguments.len(), 1);
                assert!(matches!(
                    arguments[0].kind,
                    ExprKind::Name(ref identifier) if identifier.text == "ms"
                ));
            }
            other => panic!("expected spaced application, got {other:?}"),
        },
        other => panic!("expected spaced value item, got {other:?}"),
    }
}

#[test]
fn parser_distinguishes_builtin_noninteger_literals_from_suffix_candidates() {
    fn expect_float(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Float(literal)) => {
                    assert_eq!(literal.raw, raw);
                }
                other => panic!("expected float literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_decimal(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Decimal(literal)) => {
                    assert_eq!(literal.raw, raw);
                }
                other => panic!("expected decimal literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_bigint(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::BigInt(literal)) => {
                    assert_eq!(literal.raw, raw);
                }
                other => panic!("expected bigint literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_suffixed(item: &Item, raw: &str, suffix: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::SuffixedInteger(literal)) => {
                    assert_eq!(literal.literal.raw, raw);
                    assert_eq!(literal.suffix.text, suffix);
                }
                other => panic!("expected suffixed integer literal candidate, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    let (_, parsed) = load(
        "value bigint = 123n\nvalue decimal = 19d\nvalue precise = 19.25d\nvalue floaty = 3.5\nvalue hexish = 0xFF\n",
    );

    assert!(!parsed.has_errors());
    expect_bigint(&parsed.module.items[0], "123n");
    expect_decimal(&parsed.module.items[1], "19d");
    expect_decimal(&parsed.module.items[2], "19.25d");
    expect_float(&parsed.module.items[3], "3.5");
    expect_suffixed(&parsed.module.items[4], "0", "xFF");
}

#[test]
fn parser_accepts_adjacent_negative_numeric_literals() {
    fn expect_integer(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Integer(literal)) => assert_eq!(literal.raw, raw),
                other => panic!("expected integer literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_float(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Float(literal)) => assert_eq!(literal.raw, raw),
                other => panic!("expected float literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_decimal(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Decimal(literal)) => assert_eq!(literal.raw, raw),
                other => panic!("expected decimal literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_bigint(item: &Item, raw: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::BigInt(literal)) => assert_eq!(literal.raw, raw),
                other => panic!("expected bigint literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    fn expect_suffixed(item: &Item, raw: &str, suffix: &str) {
        match item {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::SuffixedInteger(literal)) => {
                    assert_eq!(literal.literal.raw, raw);
                    assert_eq!(literal.suffix.text, suffix);
                }
                other => panic!("expected suffixed integer literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    let (_, parsed) = load(
        "domain Duration over Int = {\n    literal ms : Int -> Duration\n}\nvalue negativeInt = -1\nvalue negativeFloat = -3.4\nvalue negativeDecimal = -19d\nvalue negativePreciseDecimal = -19.25d\nvalue negativeBigInt = -123n\nvalue negativeDuration = -250ms\nvalue subtract = 4 - 3\n",
    );

    assert!(
        !parsed.has_errors(),
        "adjacent negative literals should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    expect_integer(&parsed.module.items[1], "-1");
    expect_float(&parsed.module.items[2], "-3.4");
    expect_decimal(&parsed.module.items[3], "-19d");
    expect_decimal(&parsed.module.items[4], "-19.25d");
    expect_bigint(&parsed.module.items[5], "-123n");
    expect_suffixed(&parsed.module.items[6], "-250", "ms");
    match &parsed.module.items[7] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator,
                left,
                right,
            }) => {
                assert_eq!(*operator, BinaryOperator::Subtract);
                assert!(matches!(left.kind, ExprKind::Integer(ref literal) if literal.raw == "4"));
                assert!(matches!(right.kind, ExprKind::Integer(ref literal) if literal.raw == "3"));
            }
            other => panic!("expected subtraction expression, got {other:?}"),
        },
        other => panic!("expected subtract value item, got {other:?}"),
    }
}

#[test]
fn parser_rejects_spaced_negative_literal_prefixes() {
    let (_, parsed) = load("value badInt = - 3\nvalue badFloat = - 3.4\n");

    assert!(
        parsed.has_errors(),
        "spaced negative literals should stay invalid"
    );
}

#[test]
fn parser_accepts_negative_numeric_constructor_arguments() {
    let (_, parsed) =
        load("type Vector = Delta Int Int\nvalue step = Delta -1 -1\nvalue subtract = 4-3\n");

    assert!(
        !parsed.has_errors(),
        "negative constructor arguments should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Apply { callee, arguments }) => {
                assert!(matches!(callee.kind, ExprKind::Name(ref name) if name.text == "Delta"));
                assert_eq!(arguments.len(), 2);
                assert!(
                    matches!(arguments[0].kind, ExprKind::Integer(ref literal) if literal.raw == "-1")
                );
                assert!(
                    matches!(arguments[1].kind, ExprKind::Integer(ref literal) if literal.raw == "-1")
                );
            }
            other => panic!("expected constructor application, got {other:?}"),
        },
        other => panic!("expected value item, got {other:?}"),
    }

    match &parsed.module.items[2] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Binary {
                operator,
                left,
                right,
            }) => {
                assert_eq!(*operator, BinaryOperator::Subtract);
                assert!(matches!(left.kind, ExprKind::Integer(ref literal) if literal.raw == "4"));
                assert!(matches!(right.kind, ExprKind::Integer(ref literal) if literal.raw == "3"));
            }
            other => panic!("expected compact subtraction, got {other:?}"),
        },
        other => panic!("expected value item, got {other:?}"),
    }
}

#[test]
fn parser_accepts_negative_integer_patterns() {
    let (_, parsed) =
        load("fun isNegativeOne:Bool = value:Int => value\n  ||> -1 -> True\n  ||> _ -> False\n");

    assert!(
        !parsed.has_errors(),
        "negative integer patterns should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Fun(item) = &parsed.module.items[0] else {
        panic!("expected a function item");
    };
    let ExprKind::Pipe(pipe) = &item.expr_body().expect("function should carry a body").kind else {
        panic!("expected the function body to remain a pipe");
    };
    let PipeStageKind::Case(first_case) = &pipe.stages[0].kind else {
        panic!("expected first stage to be a case arm");
    };
    assert!(matches!(
        first_case.pattern.kind,
        PatternKind::Integer(ref literal) if literal.raw == "-1"
    ));
}

#[test]
fn parser_accepts_domain_member_bindings_after_type_annotation() {
    let (_, parsed) = load(
        r#"type Builder = Int -> Duration
domain Duration over Int = {
    type Builder
    make raw = raw
}
"#,
    );

    assert!(
        !parsed.has_errors(),
        "domain member bindings should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Domain(item) = &parsed.module.items[1] else {
        panic!("expected a domain item");
    };
    let body = item.body.as_ref().expect("domain should carry a body");
    assert_eq!(body.members.len(), 1);
    assert!(body.members[0].annotation.is_some());
    assert_eq!(body.members[0].parameters.len(), 1);
    assert_eq!(body.members[0].parameters[0].text, "raw");
    assert!(matches!(
        body.members[0].body.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::Name(identifier)) if identifier.text == "raw"
    ));
}

#[test]
fn parser_builds_map_and_set_literals_without_consuming_bare_names() {
    let (_, parsed) = load(
        "value headers = Map { \"Authorization\": token, \"Accept\": \"application/json\" }\nvalue tags = Set [1, 2, selected]\nvalue bare = Map\n",
    );

    assert!(!parsed.has_errors());

    match &parsed.module.items[0] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Map(map)) => {
                assert_eq!(map.entries.len(), 2);
                assert!(matches!(map.entries[0].key.kind, ExprKind::Text(_)));
                assert!(matches!(
                    map.entries[0].value.kind,
                    ExprKind::Name(ref identifier) if identifier.text == "token"
                ));
                assert!(matches!(map.entries[1].key.kind, ExprKind::Text(_)));
                assert!(matches!(map.entries[1].value.kind, ExprKind::Text(_)));
            }
            other => panic!("expected map literal, got {other:?}"),
        },
        other => panic!("expected map value item, got {other:?}"),
    }

    match &parsed.module.items[1] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Set(elements)) => {
                assert_eq!(elements.len(), 3);
                assert!(matches!(elements[0].kind, ExprKind::Integer(_)));
                assert!(matches!(
                    elements[2].kind,
                    ExprKind::Name(ref identifier) if identifier.text == "selected"
                ));
            }
            other => panic!("expected set literal, got {other:?}"),
        },
        other => panic!("expected set value item, got {other:?}"),
    }

    match &parsed.module.items[2] {
        Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
            Some(ExprKind::Name(identifier)) => assert_eq!(identifier.text, "Map"),
            other => panic!("expected bare `Map` name, got {other:?}"),
        },
        other => panic!("expected bare value item, got {other:?}"),
    }
}

#[test]
fn parser_reports_missing_domain_over_and_carrier() {
    let (_, missing_over) = load("domain Duration Int\n");
    assert!(missing_over.has_errors());
    assert!(
        missing_over
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(MISSING_DOMAIN_OVER))
    );

    let (_, missing_carrier) = load("domain Duration over\n");
    assert!(missing_carrier.has_errors());
    assert!(
        missing_carrier
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(MISSING_DOMAIN_CARRIER))
    );
}

#[test]
fn parser_reports_missing_item_name() {
    let (_, parsed) = load("value = 42\n");

    assert!(parsed.has_errors());
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(MISSING_ITEM_NAME))
    );
    match &parsed.module.items[0] {
        Item::Value(item) => assert!(item.name.is_none()),
        other => panic!("expected a value item, got {other:?}"),
    }
}

#[test]
fn parser_reports_missing_grouped_export_targets() {
    let (_, parsed) = load("export ()\n");

    assert!(parsed.has_errors());
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(MISSING_EXPORT_NAME))
    );
    match &parsed.module.items[0] {
        Item::Export(item) => assert!(item.targets.is_empty()),
        other => panic!("expected an export item, got {other:?}"),
    }
}

#[test]
fn parser_reports_trailing_tokens_after_expression_body() {
    let (_, parsed) =
        load("fun prependCells:List Int = head:Int tail:List Int =>\n    head :: tail\n");

    assert!(parsed.has_errors());
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(TRAILING_DECLARATION_BODY_TOKEN))
    );
}

#[test]
fn parser_accepts_valid_fixture_corpus() {
    let valid_root = fixture_root().join("valid");
    let mut stack = vec![valid_root];
    let mut fixtures = Vec::new();
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path).expect("valid fixture directory must be readable") {
            let entry = entry.expect("fixture dir entry must load");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("aivi") {
                fixtures.push(path);
            }
        }
    }
    fixtures.sort();

    for fixture in fixtures {
        let text = fs::read_to_string(&fixture).expect("fixture text must load");
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(&fixture, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "expected valid fixture {} to parse cleanly, got diagnostics: {:?}",
            fixture.display(),
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        assert!(
            !parsed.module.items.is_empty(),
            "{} should contain items",
            fixture.display()
        );
    }
}

#[test]
fn parser_flags_only_syntax_invalid_fixtures() {
    for relative in [
        "invalid/markup_mismatched_close.aivi",
        "invalid/markup_child_interpolation.aivi",
    ] {
        let parsed = parse_fixture(relative);
        assert!(
            parsed.has_errors(),
            "{relative} should report syntax errors"
        );
    }

    for relative in [
        "invalid/pattern_non_exhaustive_sum.aivi",
        "invalid/val_depends_on_sig.aivi",
        "invalid/source_unknown_option.aivi",
        "invalid/record_missing_required_field.aivi",
        "invalid/each_missing_key.aivi",
        "invalid/gate_non_list.aivi",
        "invalid/regex_bad_pattern.aivi",
        "invalid/regex_invalid_quantifier.aivi",
        "invalid/cluster_unfinished_gate.aivi",
    ] {
        let parsed = parse_fixture(relative);
        assert!(
            !parsed.has_errors(),
            "{relative} should remain for later semantic milestones: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
    }
}

#[test]
fn parser_preserves_qualified_markup_tag_names() {
    let (_, parsed) = load(
        r#"
value view =
    <Window>
        <Paned.start>
            <Label />
        </Paned.start>
    </Window>
"#,
    );
    assert!(
        !parsed.has_errors(),
        "qualified markup names should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Value(view) = &parsed.module.items()[0] else {
        panic!("expected the test item to be a value declaration");
    };
    let ExprKind::Markup(root) = &view
        .expr_body()
        .expect("test value should carry a markup expression body")
        .kind
    else {
        panic!("expected the test value body to be markup");
    };
    let paned_start = root
        .children
        .first()
        .expect("window markup should contain the qualified child-group wrapper");
    assert_eq!(paned_start.name.as_dotted(), "Paned.start");
    assert_eq!(
        paned_start
            .close_name
            .as_ref()
            .expect("qualified wrapper should keep its close tag")
            .as_dotted(),
        "Paned.start"
    );
}

#[test]
fn parser_accepts_subject_placeholders_ranges_and_discard_params() {
    let (_, parsed) = load(
        r#"value subject = .
value projection = .email
value span = 1..10
value values = [1..10]
fun ignore:Int = _ => 0
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected subject/range surface forms to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Value(subject) = &parsed.module.items[0] else {
        panic!("expected subject value item");
    };
    assert!(matches!(
        subject.expr_body().map(|expr| &expr.kind),
        Some(ExprKind::SubjectPlaceholder)
    ));

    let Item::Value(projection) = &parsed.module.items[1] else {
        panic!("expected projection value item");
    };
    assert!(matches!(
        projection.expr_body().map(|expr| &expr.kind),
        Some(ExprKind::AmbientProjection(path))
            if path.fields.len() == 1 && path.fields[0].text == "email"
    ));

    let Item::Value(span) = &parsed.module.items[2] else {
        panic!("expected span value item");
    };
    assert!(matches!(
        span.expr_body().map(|expr| &expr.kind),
        Some(ExprKind::Range { .. })
    ));

    let Item::Value(values) = &parsed.module.items[3] else {
        panic!("expected values item");
    };
    assert!(matches!(
        values.expr_body().map(|expr| &expr.kind),
        Some(ExprKind::List(elements))
            if matches!(elements.as_slice(), [Expr { kind: ExprKind::Range { .. }, .. }])
    ));

    let Item::Fun(ignore) = &parsed.module.items[4] else {
        panic!("expected ignore function item");
    };
    assert_eq!(ignore.parameters.len(), 1);
    assert!(ignore.parameters[0].name.is_none());
}

#[test]
fn parser_accepts_unary_subject_function_bodies_without_arrows() {
    let (_, parsed) = load(
        r#"fun currentStatus:Text = .status
fun scoreLineFor:Text = "Score: {.}"
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected unary subject function sugar to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Fun(current_status) = &parsed.module.items[0] else {
        panic!("expected currentStatus function item");
    };
    assert_eq!(
        current_status.function_form,
        FunctionSurfaceForm::UnarySubjectSugar
    );
    assert_eq!(current_status.parameters.len(), 1);
    assert!(matches!(
        current_status.expr_body().map(|expr| &expr.kind),
        Some(ExprKind::Projection { base, path })
            if matches!(base.kind, ExprKind::Name(ref identifier) if identifier.text == IMPLICIT_FUNCTION_SUBJECT_NAME)
                && path.fields.len() == 1
                && path.fields[0].text == "status"
    ));

    let Item::Fun(score_line_for) = &parsed.module.items[1] else {
        panic!("expected scoreLineFor function item");
    };
    assert_eq!(
        score_line_for.function_form,
        FunctionSurfaceForm::UnarySubjectSugar
    );
    assert_eq!(score_line_for.parameters.len(), 1);
    let Some(Expr {
        kind: ExprKind::Text(text),
        ..
    }) = score_line_for.expr_body()
    else {
        panic!("expected scoreLineFor to lower into a text literal body");
    };
    assert!(matches!(
        text.segments.as_slice(),
        [TextSegment::Text(fragment), TextSegment::Interpolation(interpolation)]
            if fragment.raw == "Score: "
                && matches!(
                    interpolation.expr.kind,
                    ExprKind::Name(ref identifier)
                        if identifier.text == IMPLICIT_FUNCTION_SUBJECT_NAME
                )
    ));
}

#[test]
fn parser_accepts_anonymous_lambda_expressions() {
    let (_, parsed) = load(
        r#"type Coord = Coord Int Int
value cell = Coord 1 1
value explicit = coord => coord == cell
value shorthand = . == cell
value next = 0 |> x => x + 1
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected anonymous lambda expressions to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Value(explicit) = &parsed.module.items[2] else {
        panic!("expected explicit lambda value item");
    };
    let explicit_body = explicit
        .expr_body()
        .expect("explicit lambda value should keep its body");
    let ExprKind::Lambda(lambda) = &explicit_body.kind else {
        panic!("expected explicit lambda expression");
    };
    assert!(matches!(
        lambda.surface_form,
        crate::cst::LambdaSurfaceForm::Explicit
    ));
    assert_eq!(lambda.parameters.len(), 1);
    assert!(matches!(&lambda.body.kind, ExprKind::Binary { .. }));

    let Item::Value(shorthand) = &parsed.module.items[3] else {
        panic!("expected shorthand lambda value item");
    };
    let shorthand_body = shorthand
        .expr_body()
        .expect("shorthand lambda value should keep its body");
    let ExprKind::Lambda(lambda) = &shorthand_body.kind else {
        panic!("expected shorthand lambda expression");
    };
    assert!(matches!(
        lambda.surface_form,
        crate::cst::LambdaSurfaceForm::SubjectShorthand
    ));
    assert_eq!(lambda.parameters.len(), 1);
    assert!(matches!(&lambda.body.kind, ExprKind::Binary { .. }));

    let Item::Value(next) = &parsed.module.items[4] else {
        panic!("expected pipe value item");
    };
    let next_body = next.expr_body().expect("pipe value should keep its body");
    let ExprKind::Pipe(pipe) = &next_body.kind else {
        panic!("expected pipe expression");
    };
    let [stage] = pipe.stages.as_slice() else {
        panic!("expected single pipe stage");
    };
    let PipeStageKind::Transform { expr } = &stage.kind else {
        panic!("expected transform pipe stage");
    };
    let ExprKind::Lambda(lambda) = &expr.kind else {
        panic!("expected explicit lambda inside pipe stage");
    };
    assert!(matches!(
        lambda.surface_form,
        crate::cst::LambdaSurfaceForm::Explicit
    ));
}

#[test]
fn parser_accepts_selected_subject_pipe_bodies_without_arrows() {
    let (_, parsed) = load(
        r#"fun flipsFromDirection = board player coord vector!
 |> rayFrom coord #ray
 |> collectRay board ray
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected selected-subject pipe sugar to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Fun(flips_from_direction) = &parsed.module.items[0] else {
        panic!("expected flipsFromDirection function item");
    };
    assert_eq!(
        flips_from_direction.function_form,
        FunctionSurfaceForm::SelectedSubjectSugar
    );
    assert_eq!(flips_from_direction.parameters.len(), 4);
    let Some(Expr {
        kind: ExprKind::Pipe(pipe),
        ..
    }) = flips_from_direction.expr_body()
    else {
        panic!("expected selected-subject body to parse as a pipe");
    };
    assert!(matches!(
        pipe.head.as_deref().map(|expr| &expr.kind),
        Some(ExprKind::Name(identifier)) if identifier.text == "vector"
    ));
    assert_eq!(pipe.stages.len(), 2);
    assert_eq!(
        pipe.stages[0]
            .result_memo
            .as_ref()
            .map(|memo| memo.text.as_str()),
        Some("ray")
    );
}

#[test]
fn parser_accepts_selected_subject_patch_bodies_without_arrows() {
    let (_, parsed) = load(
        r#"fun recordOpponent = state! coord
    <| {
        collecting: True,
        closed: False,
        trail: [coord]
    }
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected selected-subject patch sugar to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Fun(record_opponent) = &parsed.module.items[0] else {
        panic!("expected recordOpponent function item");
    };
    assert_eq!(
        record_opponent.function_form,
        FunctionSurfaceForm::SelectedSubjectSugar
    );
    let Some(Expr {
        kind: ExprKind::PatchApply { target, patch },
        ..
    }) = record_opponent.expr_body()
    else {
        panic!("expected selected-subject body to parse as a patch apply");
    };
    assert!(matches!(&target.kind, ExprKind::Name(identifier) if identifier.text == "state"));
    assert_eq!(patch.entries.len(), 3);
}

#[test]
fn parser_accepts_selected_subject_record_selectors() {
    let (_, parsed) = load(
        r#"fun readNested = state { x.y.z! }
 |> render
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected selected-subject record selector to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Fun(read_nested) = &parsed.module.items[0] else {
        panic!("expected readNested function item");
    };
    assert_eq!(
        read_nested.function_form,
        FunctionSurfaceForm::SelectedSubjectSugar
    );
    let Some(Expr {
        kind: ExprKind::Pipe(pipe),
        ..
    }) = read_nested.expr_body()
    else {
        panic!("expected record-selector body to parse as a pipe");
    };
    let Some(Expr {
        kind: ExprKind::Projection { base, path },
        ..
    }) = pipe.head.as_deref()
    else {
        panic!("expected record-selector body head to parse as a projection");
    };
    assert!(matches!(&base.kind, ExprKind::Name(identifier) if identifier.text == "state"));
    assert_eq!(
        path.fields
            .iter()
            .map(|field| field.text.as_str())
            .collect::<Vec<_>>(),
        vec!["x", "y", "z"]
    );
}

#[test]
fn parser_rejects_nullary_function_declarations() {
    let (_, parsed) = load("fun constant:Int = => 1\n");

    assert!(parsed.has_errors(), "nullary functions should stay invalid");
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(NULLARY_FUNCTION_DECLARATION) })
    );
}

#[test]
fn parser_accepts_pipe_subject_and_result_memos() {
    let (_, parsed) = load(
        r#"value memoed =
    20
     |> #before before + 1 #after
     |> after + before
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected pipe memos to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Value(value) = &parsed.module.items[0] else {
        panic!("expected memoed value item");
    };
    let Some(Expr {
        kind: ExprKind::Pipe(pipe),
        ..
    }) = value.expr_body()
    else {
        panic!("expected value body to be a pipe expression");
    };
    assert_eq!(pipe.stages.len(), 2);
    let first = &pipe.stages[0];
    assert_eq!(
        first
            .subject_memo
            .as_ref()
            .expect("first stage should preserve subject memo")
            .text,
        "before"
    );
    assert_eq!(
        first
            .result_memo
            .as_ref()
            .expect("first stage should preserve result memo")
            .text,
        "after"
    );
    assert!(pipe.stages[1].subject_memo.is_none());
    assert!(pipe.stages[1].result_memo.is_none());
}

#[test]
fn parser_accepts_pipe_case_stage_memos() {
    let (_, parsed) = load(
        r#"value memoed = Some 2
 ||> #incoming Some value -> value + 1 #resolved
 ||> None -> 0 #resolved
 |> resolved
"#,
    );

    assert!(
        !parsed.has_errors(),
        "expected case-stage pipe memos to parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let Item::Value(value) = &parsed.module.items[0] else {
        panic!("expected memoed value item");
    };
    let Some(Expr {
        kind: ExprKind::Pipe(pipe),
        ..
    }) = value.expr_body()
    else {
        panic!("expected value body to be a pipe expression");
    };
    assert_eq!(pipe.stages.len(), 3);
    assert_eq!(
        pipe.stages[0]
            .subject_memo
            .as_ref()
            .expect("first case arm should preserve the subject memo")
            .text,
        "incoming"
    );
    assert_eq!(
        pipe.stages[0]
            .result_memo
            .as_ref()
            .expect("first case arm should preserve the result memo")
            .text,
        "resolved"
    );
    assert_eq!(
        pipe.stages[1]
            .result_memo
            .as_ref()
            .expect("second case arm should preserve the shared result memo")
            .text,
        "resolved"
    );
}

#[test]
fn parser_builds_hoist_item_with_no_filters() {
    let (_, parsed) = load("hoist\n");
    assert!(
        !parsed.has_errors(),
        "hoist should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    assert_eq!(parsed.module.items.len(), 1);
    let Item::Hoist(hoist) = &parsed.module.items[0] else {
        panic!("expected hoist item");
    };
    assert!(hoist.kind_filters.is_empty());
    assert!(hoist.hiding.is_empty());
}

#[test]
fn parser_builds_hoist_item_with_kind_filters() {
    let (_, parsed) = load("hoist (func, value)\n");
    assert!(
        !parsed.has_errors(),
        "hoist with filters should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Hoist(hoist) = &parsed.module.items[0] else {
        panic!("expected hoist item");
    };
    assert_eq!(hoist.kind_filters.len(), 2);
    assert_eq!(hoist.kind_filters[0].text, "func");
    assert_eq!(hoist.kind_filters[1].text, "value");
}

#[test]
fn parser_builds_hoist_item_with_hiding_clause() {
    let (_, parsed) = load("hoist hiding (length, head)\n");
    assert!(
        !parsed.has_errors(),
        "hoist with hiding should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Hoist(hoist) = &parsed.module.items[0] else {
        panic!("expected hoist item");
    };
    assert!(hoist.kind_filters.is_empty());
    assert_eq!(hoist.hiding.len(), 2);
    assert_eq!(hoist.hiding[0].text, "length");
    assert_eq!(hoist.hiding[1].text, "head");
}

#[test]
fn parser_builds_hoist_item_with_filters_and_hiding() {
    let (_, parsed) = load("hoist (func, value) hiding (map, filter)\n");
    assert!(
        !parsed.has_errors(),
        "hoist with filters and hiding should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let Item::Hoist(hoist) = &parsed.module.items[0] else {
        panic!("expected hoist item");
    };
    assert_eq!(hoist.kind_filters.len(), 2);
    assert_eq!(hoist.hiding.len(), 2);
    assert_eq!(hoist.hiding[0].text, "map");
    assert_eq!(hoist.hiding[1].text, "filter");
}

#[test]
fn parser_rejects_discard_exprs_and_markup_child_interpolation() {
    let (_, parsed) = load(
        r#"value current = _
value view =
    <Label>{current}</Label>
"#,
    );

    assert!(parsed.has_errors());
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(INVALID_DISCARD_EXPR))
    );
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(INVALID_MARKUP_CHILD_CONTENT))
    );
}
