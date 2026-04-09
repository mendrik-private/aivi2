//! Corpus idempotency tests for the AIVI formatter.
//!
//! These tests verify that running the formatter twice on any source file
//! produces identical output (i.e. the formatter is idempotent).

use aivi_base::SourceDatabase;
use aivi_syntax::{Formatter, TokenKind, lex_module, parse_module};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn format_text(src: &str) -> Option<String> {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file("test.aivi", src);
    let parsed = parse_module(&db[file_id]);
    if parsed.has_errors() {
        return None;
    }
    Some(Formatter.format(&parsed.module))
}

fn retained_token_counts(src: &str) -> BTreeMap<String, usize> {
    let mut db = SourceDatabase::new();
    let file_id = db.add_file("test.aivi", src);
    let source = &db[file_id];
    let lexed = lex_module(source);
    let mut counts = BTreeMap::new();
    for token in lexed.tokens() {
        let Some(key) = retained_token_key(token.kind(), token.text(source)) else {
            continue;
        };
        *counts.entry(key).or_default() += 1;
    }
    counts
}

fn retained_token_key(kind: TokenKind, text: &str) -> Option<String> {
    match kind {
        TokenKind::Identifier => Some(format!("identifier:{text}")),
        TokenKind::Integer => Some(format!("integer:{text}")),
        TokenKind::Float => Some(format!("float:{text}")),
        TokenKind::Decimal => Some(format!("decimal:{text}")),
        TokenKind::BigInt => Some(format!("bigint:{text}")),
        TokenKind::StringLiteral => Some("string-literal".to_owned()),
        TokenKind::RegexLiteral => Some("regex-literal".to_owned()),
        TokenKind::PatchKw
        | TokenKind::TypeKw
        | TokenKind::FuncKw
        | TokenKind::ValueKw
        | TokenKind::SignalKw
        | TokenKind::FromKw
        | TokenKind::ClassKw
        | TokenKind::InstanceKw
        | TokenKind::DomainKw
        | TokenKind::ProviderKw
        | TokenKind::UseKw
        | TokenKind::ExportKw
        | TokenKind::HoistKw => Some(format!("keyword:{kind:?}")),
        TokenKind::At
        | TokenKind::Hash
        | TokenKind::Equals
        | TokenKind::EqualEqual
        | TokenKind::Bang
        | TokenKind::BangEqual
        | TokenKind::Ellipsis
        | TokenKind::DotDot
        | TokenKind::Dot
        | TokenKind::Plus
        | TokenKind::Minus
        | TokenKind::Less
        | TokenKind::Greater
        | TokenKind::LessEqual
        | TokenKind::GreaterEqual
        | TokenKind::LeftArrow
        | TokenKind::Slash
        | TokenKind::Percent
        | TokenKind::Arrow
        | TokenKind::ThinArrow
        | TokenKind::Star
        | TokenKind::PatchApply
        | TokenKind::ColonEquals => Some(format!("operator:{kind:?}")),
        TokenKind::PipeTap => None,
        kind if kind.is_pipe_operator() => Some(format!("pipe:{kind:?}")),
        _ => None,
    }
}

fn assert_preserves_retained_tokens(path: &Path) {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {:?}: {}", path, e));
    let formatted = match format_text(&src) {
        Some(s) => s,
        None => return,
    };
    let before = retained_token_counts(&src);
    let after = retained_token_counts(&formatted);
    let missing = before
        .iter()
        .filter_map(|(key, before_count)| {
            let after_count = after.get(key).copied().unwrap_or(0);
            (after_count < *before_count)
                .then_some(format!("{key} ({before_count} -> {after_count})"))
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "formatter dropped retained tokens for {:?}: {:?}",
        path,
        missing
    );
}

fn collect_aivi_files(base: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !base.exists() {
        return result;
    }
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "aivi") {
                result.push(path);
            }
        }
    }
    result.sort();
    result
}

fn assert_idempotent(path: &Path) {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {:?}: {}", path, e));
    let first = match format_text(&src) {
        Some(s) => s,
        None => return, // skip files with parse errors
    };
    let second = format_text(&first).expect("second format pass should succeed");
    assert_eq!(first, second, "formatter is not idempotent for {:?}", path);
}

// ---------------------------------------------------------------------------
// Corpus: fixtures/frontend/
// ---------------------------------------------------------------------------

#[test]
fn format_corpus_fixtures() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../fixtures/frontend");
    let files = collect_aivi_files(&base);
    assert!(
        !files.is_empty(),
        "no .aivi files found under fixtures/frontend/"
    );
    for path in &files {
        assert_idempotent(path);
    }
}

#[test]
fn format_corpus_fixtures_preserves_retained_tokens() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../fixtures/frontend");
    let files = collect_aivi_files(&base);
    assert!(
        !files.is_empty(),
        "no .aivi files found under fixtures/frontend/"
    );
    for path in &files {
        assert_preserves_retained_tokens(path);
    }
}

// ---------------------------------------------------------------------------
// Corpus: demos/
// ---------------------------------------------------------------------------

#[test]
fn format_corpus_demos() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../demos");
    let files = collect_aivi_files(&base);
    assert!(!files.is_empty(), "no .aivi files found under demos/");
    for path in &files {
        assert_idempotent(path);
    }
}

#[test]
fn format_corpus_demos_preserves_retained_tokens() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../demos");
    let files = collect_aivi_files(&base);
    assert!(!files.is_empty(), "no .aivi files found under demos/");
    for path in &files {
        assert_preserves_retained_tokens(path);
    }
}

// ---------------------------------------------------------------------------
// Corpus: stdlib/
// ---------------------------------------------------------------------------

#[test]
fn format_corpus_stdlib() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../stdlib");
    let files = collect_aivi_files(&base);
    assert!(!files.is_empty(), "no .aivi files found under stdlib/");
    for path in &files {
        assert_idempotent(path);
    }
}

#[test]
fn format_corpus_stdlib_preserves_retained_tokens() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../stdlib");
    let files = collect_aivi_files(&base);
    assert!(!files.is_empty(), "no .aivi files found under stdlib/");
    for path in &files {
        assert_preserves_retained_tokens(path);
    }
}
