use aivi_base::SourceDatabase;
use aivi_hir::{lower_module, typecheck_module};
use aivi_syntax::parse_module;

fn typecheck_text(path: &str, text: &str) -> aivi_hir::TypeCheckReport {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "typecheck input should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let lowered = lower_module(&parsed.module);
    assert!(
        !lowered.has_errors(),
        "typecheck input should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    typecheck_module(lowered.module())
}

fn parse_and_lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "input should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    lower_module(&parsed.module)
}

// ──────────────────────────────────────────────────────────────────────
// Record projection expression: { field: . }
// ──────────────────────────────────────────────────────────────────────

#[test]
fn record_projection_parses_with_unary_subject_sugar() {
    // { score: . } >= 100 should parse as a projection, not record construction.
    let lowered = parse_and_lower_text(
        "record-projection.aivi",
        "type Profile = { score: Int }\n\
         type Profile -> Bool\n\
         func isTopScore = { score: . } >= 100\n",
    );
    assert!(
        !lowered.has_errors(),
        "record projection should lower cleanly: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn record_projection_typechecks() {
    let report = typecheck_text(
        "record-projection-tc.aivi",
        "type Profile = { score: Int }\n\
         type Profile -> Bool\n\
         func isTopScore = { score: . } >= 100\n",
    );
    assert!(
        report.is_ok(),
        "record projection should typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

// ──────────────────────────────────────────────────────────────────────
// Dotted path destructuring: { address.city.name }
// ──────────────────────────────────────────────────────────────────────

#[test]
fn dotted_path_pattern_parses_and_lowers() {
    let lowered = parse_and_lower_text(
        "dotted-path-pattern.aivi",
        "type City = { name: Text }\n\
         type Address = { city: City }\n\
         type User = { address: Address }\n\
         type User -> Text\n\
         func getCityName = user => user\n\
          ||> { address.city.name } -> name\n",
    );
    assert!(
        !lowered.has_errors(),
        "dotted path pattern should lower cleanly: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn dotted_path_pattern_typechecks() {
    let report = typecheck_text(
        "dotted-path-pattern-tc.aivi",
        "type City = { name: Text }\n\
         type Address = { city: City }\n\
         type User = { address: Address }\n\
         type User -> Text\n\
         func getCityName = user => user\n\
          ||> { address.city.name } -> name\n",
    );
    assert!(
        report.is_ok(),
        "dotted path pattern should typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

// ──────────────────────────────────────────────────────────────────────
// Combined: { a.b.c: . } |> f
// ──────────────────────────────────────────────────────────────────────

#[test]
fn dotted_projection_with_pipe_parses_and_lowers() {
    let lowered = parse_and_lower_text(
        "dotted-projection-pipe.aivi",
        "type City = { name: Text }\n\
         type Address = { city: City }\n\
         type User = { address: Address }\n\
         type User -> Text\n\
         func getCityName = { address.city.name: . }\n",
    );
    assert!(
        !lowered.has_errors(),
        "dotted projection should lower cleanly: {:?}",
        lowered.diagnostics()
    );
}
