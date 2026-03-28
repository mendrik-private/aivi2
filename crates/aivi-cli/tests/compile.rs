use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
        .join(relative)
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(prefix: &str, extension: &str, contents: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "aivi-{prefix}-{}-{unique}.{extension}",
            std::process::id()
        ));
        fs::write(&path, contents).expect("temporary file should be writable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("aivi-{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("temporary directory should be creatable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn compile_writes_object_and_reports_codegen_boundary() {
    let output_dir = TempDir::new("compile-success");
    let output_path = output_dir.path().join("fixture.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path(
            "milestone-2/valid/pipe-gate-carriers/main.aivi",
        ))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::metadata(&output_path).expect("compile should write an object file");
    assert!(metadata.len() > 0, "object file should not be empty");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("codegen: ok"),
        "expected codegen success in summary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains(&output_path.display().to_string()),
        "expected emitted object path in stdout, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_workspace_type_imports() {
    let output_dir = TempDir::new("compile-workspace-types");
    let output_path = output_dir.path().join("workspace.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path(
            "milestone-2/valid/workspace-type-imports/main.aivi",
        ))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected workspace compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("workspace compile should write an object file");
    assert!(
        metadata.len() > 0,
        "workspace object file should not be empty"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_workspace_value_imports() {
    let output_dir = TempDir::new("compile-workspace-values");
    let output_path = output_dir.path().join("workspace-values.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path(
            "milestone-2/valid/use-member-imports/main.aivi",
        ))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected workspace value compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("workspace value compile should write an object file");
    assert!(
        metadata.len() > 0,
        "workspace value object file should not be empty"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_list_pattern_fixture() {
    let output_dir = TempDir::new("compile-list-patterns");
    let output_path = output_dir.path().join("list-patterns.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path("milestone-2/valid/list-patterns/main.aivi"))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected list pattern compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("list pattern compile should write an object file");
    assert!(
        metadata.len() > 0,
        "list pattern object file should not be empty"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("codegen: ok"),
        "expected codegen success in summary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_catalog_foundation_example() {
    let output_dir = TempDir::new("compile-catalog-foundation");
    let output_path = output_dir.path().join("catalog-foundation.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path(
            "catalog/foundation/surface_values_patterns/main.aivi",
        ))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected catalog foundation compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("catalog foundation compile should write an object file");
    assert!(
        metadata.len() > 0,
        "catalog foundation object file should not be empty"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_catalog_multifile_example() {
    let output_dir = TempDir::new("compile-catalog-workspace");
    let output_path = output_dir.path().join("catalog-workspace.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(fixture_path(
            "catalog/foundation/surface_workspace_imports/main.aivi",
        ))
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected catalog multifile compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("catalog multifile compile should write an object file");
    assert!(
        metadata.len() > 0,
        "catalog multifile object file should not be empty"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compile pipeline passed"),
        "expected compile summary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_additional_compile_safe_catalog_examples() {
    for relative in [
        "catalog/math/math_fft/main.aivi",
        "catalog/math/math_matrix_lu/main.aivi",
        "catalog/math/math_mod_arith_ntt/main.aivi",
        "catalog/tree/tree_segment_tree_lazy/main.aivi",
    ] {
        let output_dir = TempDir::new("compile-catalog-additional");
        let output_path = output_dir.path().join("catalog-example.o");
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("compile")
            .arg(fixture_path(relative))
            .arg("-o")
            .arg(&output_path)
            .output()
            .expect("compile command should run");

        assert!(
            output.status.success(),
            "expected {relative} to compile successfully, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata =
            fs::metadata(&output_path).expect("catalog compile should write an object file");
        assert!(
            metadata.len() > 0,
            "catalog object file should not be empty for {relative}"
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("compile pipeline passed"),
            "expected compile summary for {relative}, got stdout: {stdout}"
        );
    }
}

#[test]
fn compile_reports_codegen_limits_without_emitting_fake_artifacts() {
    let input = TempFile::new(
        "compile-codegen-limit",
        "aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration
    (>) : Duration -> Duration -> Bool

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
    windows
     ?|> ((.delay + 5ms) > 12ms)
"#,
    );
    let output_dir = TempDir::new("compile-codegen-limit");
    let output_path = output_dir.path().join("unsupported.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        !output.status.success(),
        "expected compile to stop at codegen, stdout was: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        fs::metadata(&output_path).is_err(),
        "compile should not emit an object file after codegen failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("codegen failed"),
        "expected codegen stage heading, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("outside the first Cranelift slice"),
        "expected explicit codegen limitation, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("compile pipeline stopped at codegen"),
        "expected explicit stop boundary, got stderr: {stderr}"
    );
}
