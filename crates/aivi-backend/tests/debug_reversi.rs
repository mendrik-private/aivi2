use std::{fs, path::PathBuf};

use aivi_backend::{lower_module as lower_backend_module, validate_program};
use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::RootDatabase;

#[test]
fn debug_reversi_backend_pretty() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let path = root.join("demos").join("reversi.aivi");
    let text = fs::read_to_string(&path).expect("reversi demo should be readable");
    let db = RootDatabase::new();
    let file = db.open_file(&path, &text);
    let lowered = aivi_query::hir_module(&db, file);
    assert!(
        lowered.hir_diagnostics().is_empty(),
        "workspace fixture should lower to HIR: {:?}",
        lowered.hir_diagnostics()
    );
    let core = lower_core_module(lowered.module()).expect("workspace HIR should lower into typed core");
    validate_core_module(&core).expect("workspace typed core should validate");
    let lambda = lower_lambda_module(&core).expect("workspace lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("workspace lambda should validate");
    let backend = lower_backend_module(&lambda).expect("workspace backend lowering should succeed");
    validate_program(&backend).expect("workspace backend should validate");
    println!("{}", backend.pretty());
}
