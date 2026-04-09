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

#[test]
fn selected_subject_pipe_bodies_lower_cleanly() {
    let lowered = parse_and_lower_text(
        "selected-subject-pipe.aivi",
        "type Box = { value: Int }\n\
         type Int -> Int -> Int\n\
         func add = left right => left + right\n\
         type Int -> Box -> Int\n\
         func readValue = amount box!\n\
          |> .value\n\
          |> add amount\n",
    );
    assert!(
        !lowered.has_errors(),
        "selected-subject pipe sugar should lower cleanly: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn selected_subject_patch_bodies_typecheck() {
    let report = typecheck_text(
        "selected-subject-patch.aivi",
        "type Coord = Coord Int Int\n\
         type State = {\n\
             collecting: Bool,\n\
             closed: Bool,\n\
             trail: List Coord\n\
         }\n\
         type State -> Coord -> State\n\
         func recordOpponent = state! coord\n\
             <| {\n\
                 collecting: True,\n\
                 closed: False,\n\
                 trail: [coord]\n\
             }\n",
    );
    assert!(
        report.is_ok(),
        "selected-subject patch sugar should typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn selected_subject_record_selectors_typecheck() {
    let report = typecheck_text(
        "selected-subject-record-selector.aivi",
        "type Z = { z: Int }\n\
         type Y = { y: Z }\n\
         type X = { x: Y }\n\
         type Int -> Int\n\
         func addOne = value => value + 1\n\
         type X -> Int\n\
         func readNested = state { x.y.z! }\n\
          |> addOne\n",
    );
    assert!(
        report.is_ok(),
        "selected-subject record selectors should typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}
