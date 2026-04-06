//! Corpus idempotency tests for the AIVI formatter.
//!
//! These tests verify that running the formatter twice on any source file
//! produces identical output (i.e. the formatter is idempotent).

use aivi_base::SourceDatabase;
use aivi_syntax::{Formatter, parse_module};
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
    assert_eq!(
        first,
        second,
        "formatter is not idempotent for {:?}",
        path
    );
}

// ---------------------------------------------------------------------------
// Corpus: fixtures/frontend/
// ---------------------------------------------------------------------------

#[test]
fn format_corpus_fixtures() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = manifest.join("../../fixtures/frontend");
    let files = collect_aivi_files(&base);
    assert!(!files.is_empty(), "no .aivi files found under fixtures/frontend/");
    for path in &files {
        assert_idempotent(path);
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
