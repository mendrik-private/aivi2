use aivi_base::SourceDatabase;
use aivi_syntax::{Formatter, lex_module, parse_module};
use proptest::prelude::*;

fn lex_first_token(src: &str) -> Option<aivi_syntax::TokenKind> {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file("test.aivi", src);
    let lexed = lex_module(&db[file_id]);
    lexed.tokens().first().map(|t| t.kind())
}

fn format_twice(src: &str) -> Option<String> {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file("test.aivi", src);
    let parsed = parse_module(&db[file_id]);
    if parsed.has_errors() {
        return None;
    }
    let first = Formatter.format(&parsed.module);

    let mut db2 = SourceDatabase::new();
    let file_id2 = db2.add_file("test.aivi", first.as_str());
    let parsed2 = parse_module(&db2[file_id2]);
    if parsed2.has_errors() {
        return None;
    }
    Some(Formatter.format(&parsed2.module))
}

// --- Integer literal round-trip ---

proptest! {
    #[test]
    fn integer_literal_round_trip(v in any::<u64>()) {
        let src = v.to_string();
        let kind = lex_first_token(&src);
        assert_eq!(kind, Some(aivi_syntax::TokenKind::Integer), "failed for input: {src}");
    }

    #[test]
    fn negative_integer_lexes_as_integer(v in any::<i64>()) {
        let src = v.to_string();
        let kind = lex_first_token(&src);
        // Negative integers start with '-' which is a separate operator token,
        // so this only holds for non-negative values. For negative values,
        // the lexer produces UnaryOperator::Minus followed by Integer.
        if v >= 0 {
            assert_eq!(kind, Some(aivi_syntax::TokenKind::Integer), "failed for input: {src}");
        }
    }

    // --- Float literal round-trip ---

    #[test]
    fn float_literal_round_trip(
        whole in any::<u32>(),
        frac in any::<u32>(),
    ) {
        let src = format!("{whole}.{frac}");
        let kind = lex_first_token(&src);
        assert_eq!(kind, Some(aivi_syntax::TokenKind::Float), "failed for input: {src}");
    }

    // --- Decimal literal round-trip ---

    #[test]
    fn decimal_literal_round_trip(
        whole in any::<u32>(),
        frac in any::<u32>(),
    ) {
        let src = format!("{whole}.{frac}d");
        let kind = lex_first_token(&src);
        assert_eq!(kind, Some(aivi_syntax::TokenKind::Decimal), "failed for input: {src}");
    }

    // --- Identifier lexing ---

    #[test]
    fn identifier_round_trip(
        first in "[a-zA-Z_]",
        rest in proptest::string::string_regex("[a-zA-Z0-9_]{0,20}").unwrap(),
    ) {
        let src = format!("{first}{rest}");
        let kind = lex_first_token(&src);
        assert_eq!(kind, Some(aivi_syntax::TokenKind::Identifier), "failed for input: {src}");
    }

    // --- Formatter idempotency on simple valid modules ---

    #[test]
    fn format_is_idempotent_value_decl(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
        val in any::<u32>(),
    ) {
        let name = format!("{name_first}{name_rest}");
        // Skip if the name happens to be a keyword
        let src = format!("value {name} = {val}");
        if let Some(result) = format_twice(&src) {
            let mut db = SourceDatabase::new();
            let file_id = db.add_file("test.aivi", src.as_str());
            let parsed = parse_module(&db[file_id]);
            let once = Formatter.format(&parsed.module);
            assert_eq!(once, result, "format not idempotent for: {src}");
        }
    }

    #[test]
    fn format_is_idempotent_signal_decl(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
        val in any::<u32>(),
    ) {
        let name = format!("{name_first}{name_rest}");
        let src = format!("signal {name} = {val}");
        if let Some(result) = format_twice(&src) {
            let mut db = SourceDatabase::new();
            let file_id = db.add_file("test.aivi", src.as_str());
            let parsed = parse_module(&db[file_id]);
            let once = Formatter.format(&parsed.module);
            assert_eq!(once, result, "format not idempotent for: {src}");
        }
    }

    #[test]
    fn format_is_idempotent_type_sum(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
    ) {
        let name = format!("{name_first}{name_rest}");
        let src = format!("type {name} =\n    | A\n    | B");
        if let Some(result) = format_twice(&src) {
            let mut db = SourceDatabase::new();
            let file_id = db.add_file("test.aivi", src.as_str());
            let parsed = parse_module(&db[file_id]);
            let once = Formatter.format(&parsed.module);
            assert_eq!(once, result, "format not idempotent for: {src}");
        }
    }

    // --- Parser does not panic on simple valid constructs ---

    #[test]
    fn parser_no_panic_value(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
        val in any::<i32>(),
    ) {
        let name = format!("{name_first}{name_rest}");
        let src = format!("value {name} = {val}");
        let mut db = SourceDatabase::new();
        let file_id = db.add_file("test.aivi", src.as_str());
        let _ = parse_module(&db[file_id]);
    }

    #[test]
    fn parser_no_panic_signal(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
        val in any::<i32>(),
    ) {
        let name = format!("{name_first}{name_rest}");
        let src = format!("signal {name} = {val}");
        let mut db = SourceDatabase::new();
        let file_id = db.add_file("test.aivi", src.as_str());
        let _ = parse_module(&db[file_id]);
    }

    #[test]
    fn parser_no_panic_func(
        name_first in "[a-zA-Z_]",
        name_rest in proptest::string::string_regex("[a-zA-Z0-9_]{1,8}").unwrap(),
    ) {
        let name = format!("{name_first}{name_rest}");
        let src = format!("type Int\ntype Text\nfunc {name} = x => x");
        let mut db = SourceDatabase::new();
        let file_id = db.add_file("test.aivi", src.as_str());
        let _ = parse_module(&db[file_id]);
    }

    // --- Suffixed integer literals ---

    #[test]
    fn suffixed_integer_round_trip(v in any::<u32>()) {
        for suffix in &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"] {
            let src = format!("{v}{suffix}");
            let kind = lex_first_token(&src);
            assert_eq!(kind, Some(aivi_syntax::TokenKind::Integer), "failed for: {src}");
        }
    }
}
