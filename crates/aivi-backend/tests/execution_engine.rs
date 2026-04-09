use std::collections::BTreeMap;

use aivi_backend::{
    BackendExecutableProgram, BackendExecutionEngine, BackendExecutionEngineKind,
    BackendKernelArtifactCache, KernelEvaluator, RuntimeValue, compile_program,
    lower_module as lower_backend_module, validate_program,
};
use aivi_base::SourceDatabase;
use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_syntax::parse_module;

fn lower_text(path: &str, text: &str) -> aivi_backend::Program {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "backend test input should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );

    let hir = aivi_hir::lower_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "backend test input should lower to HIR: {:?}",
        hir.diagnostics()
    );

    let core = lower_core_module(hir.module()).expect("HIR should lower into typed core");
    validate_core_module(&core).expect("typed core should validate before backend lowering");

    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate before backend lowering");

    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");
    backend
}

fn find_item(program: &aivi_backend::Program, name: &str) -> aivi_backend::ItemId {
    program
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("expected backend item `{name}`"))
}

#[test]
fn kernel_evaluator_supports_the_backend_execution_engine_trait() {
    let backend = lower_text("backend-engine-trait.aivi", "value total:Int = 21 + 21\n");
    let mut engine: Box<dyn BackendExecutionEngine + '_> = Box::new(KernelEvaluator::new(&backend));

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Interpreter);
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "total"), &BTreeMap::new())
            .expect("trait-object execution should evaluate"),
        RuntimeValue::Int(42)
    );
}

#[test]
fn interpreted_executable_program_creates_profiled_interpreter_engines() {
    let backend = lower_text(
        "backend-engine-profiled.aivi",
        "value total:Int = 21 + 21\n",
    );
    let total = find_item(&backend, "total");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_profiled_engine();

    assert_eq!(
        executable.engine_kind(),
        BackendExecutionEngineKind::Interpreter
    );
    assert!(executable.compiled_object().is_none());
    assert!(engine.profile().is_some());
    assert_eq!(
        engine
            .evaluate_item(total, &BTreeMap::new())
            .expect("profiled interpreter engine should evaluate"),
        RuntimeValue::Int(42)
    );

    let profile = engine
        .profile_snapshot()
        .expect("profiled interpreter should expose a profile snapshot");
    assert_eq!(profile.items[&total].calls, 1);
}

#[test]
fn compiled_executable_program_keeps_object_artifacts_and_interpreter_fallback() {
    let backend = lower_text(
        "backend-engine-compiled.aivi",
        "value total:Int = 21 + 21\n",
    );
    let total = find_item(&backend, "total");
    let executable = BackendExecutableProgram::compile(&backend)
        .expect("executable-program compile should preserve object emission");
    let compiled = executable
        .compiled_object()
        .expect("compiled executable program should retain object artifacts");
    let mut engine = executable.create_engine();

    assert_eq!(
        executable.engine_kind(),
        BackendExecutionEngineKind::Interpreter
    );
    assert!(!compiled.object().is_empty());
    assert!(!compiled.kernels().is_empty());
    assert_eq!(
        engine.evaluate_item(total, &BTreeMap::new()).expect(
            "compiled executable program should still evaluate through interpreter fallback"
        ),
        RuntimeValue::Int(42)
    );
}

#[test]
fn kernel_fingerprints_stay_stable_for_unchanged_kernels() {
    let original = lower_text(
        "backend-engine-fingerprint.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 1 + 1\n",
    );
    let changed = lower_text(
        "backend-engine-fingerprint.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 2 + 2\n",
    );

    let original_total = original.items()[find_item(&original, "total")]
        .body
        .expect("total should lower into a body kernel");
    let original_other = original.items()[find_item(&original, "other")]
        .body
        .expect("other should lower into a body kernel");
    let changed_total = changed.items()[find_item(&changed, "total")]
        .body
        .expect("total should lower into a body kernel");
    let changed_other = changed.items()[find_item(&changed, "other")]
        .body
        .expect("other should lower into a body kernel");

    let original_exec = BackendExecutableProgram::interpreted(&original);
    let changed_exec = BackendExecutableProgram::interpreted(&changed);

    assert_eq!(
        original_exec.kernel_fingerprint(original_total),
        changed_exec.kernel_fingerprint(changed_total)
    );
    assert_ne!(
        original_exec.kernel_fingerprint(original_other),
        changed_exec.kernel_fingerprint(changed_other)
    );
}

#[test]
fn lazy_kernel_compilation_reuses_eager_kernel_metadata_for_supported_programs() {
    let backend = lower_text(
        "backend-engine-lazy-supported.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 5 + 8\n",
    );
    let total = backend.items()[find_item(&backend, "total")]
        .body
        .expect("total should lower into a body kernel");

    let eager = compile_program(&backend).expect("full-program compilation should succeed");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let lazy = executable
        .compile_kernel(total)
        .expect("single-kernel lazy compilation should succeed");

    assert_eq!(
        lazy.metadata(),
        eager
            .kernel(total)
            .expect("eager compilation should retain the same kernel metadata")
    );
    assert!(!lazy.object().is_empty());
}

#[test]
fn lazy_kernel_compilation_can_skip_unrelated_unsupported_kernels_and_reuse_memory_cache() {
    let backend = lower_text(
        "backend-engine-lazy-unsupported.aivi",
        r#"
domain Path over Text

fun samePath:Bool = left:Path right:Path=>    left == right
value total:Int = 21 + 21
"#,
    );
    let total = backend.items()[find_item(&backend, "total")]
        .body
        .expect("total should lower into a body kernel");

    assert!(
        compile_program(&backend).is_err(),
        "full-program eager compilation should still reject unrelated unsupported kernels"
    );

    let executable = BackendExecutableProgram::interpreted(&backend);
    let expected_fingerprint = executable.kernel_fingerprint(total);
    let lazy = executable
        .compile_kernel(total)
        .expect("single-kernel lazy compilation should still compile supported kernels");
    assert_eq!(lazy.kernel_id(), total);
    assert_eq!(lazy.fingerprint(), expected_fingerprint);
    assert!(!lazy.object().is_empty());

    let mut cache = BackendKernelArtifactCache::new();
    let cached = cache
        .get_or_compile(&backend, total)
        .expect("memory cache should compile and store the first lazy artifact")
        .clone();
    let cached_again = cache
        .get_or_compile(&backend, total)
        .expect("memory cache should reuse the stored lazy artifact")
        .clone();
    assert_eq!(cache.len(), 1);
    assert_eq!(cached, cached_again);
    assert_eq!(cached, lazy);
}
