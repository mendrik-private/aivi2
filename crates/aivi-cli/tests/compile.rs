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
    root: PathBuf,
    path: PathBuf,
}

impl TempFile {
    fn new(prefix: &str, extension: &str, contents: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("aivi-{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).expect("temporary file directory should be creatable");
        let path = root.join(format!("main.{extension}"));
        fs::write(&path, contents).expect("temporary file should be writable");
        Self { root, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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
fn compile_accepts_reactive_update_programs() {
    let input = TempFile::new(
        "compile-reactive-update",
        "aivi",
        concat!(
            "signal left = 20\n",
            "signal right = 22\n",
            "signal ready = True\n",
            "signal enabled = False\n",
            "\n",
            "signal total : Signal Int = ready | enabled\n",
            "  ||> ready True => left + right\n",
            "  ||> enabled True => left + right + 1\n",
            "  ||> _ => 0\n",
        ),
    );
    let output_dir = TempDir::new("compile-reactive-update");
    let output_path = output_dir.path().join("reactive-update.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected reactive update compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("reactive update compile should write an object file");
    assert!(
        metadata.len() > 0,
        "reactive update object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_inline_pipe_gate_programs() {
    let input = TempFile::new(
        "compile-inline-pipe-gate",
        "aivi",
        concat!(
            "value maybePositive : Option Int = 2\n",
            " ?|> True\n",
            "\n",
            "value missingNumber : Option Int = 2\n",
            " ?|> False\n",
            "\n",
            "value maybeGreeting : Option Text = \"hello\"\n",
            " ?|> True\n",
            "\n",
            "value missingGreeting : Option Text = \"hello\"\n",
            " ?|> False\n",
        ),
    );
    let output_dir = TempDir::new("compile-inline-pipe-gate");
    let output_path = output_dir.path().join("inline-pipe-gate.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected inline pipe gate compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("inline pipe gate compile should write an object file");
    assert!(
        metadata.len() > 0,
        "inline pipe gate object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_anonymous_lambda_programs() {
    let input = TempFile::new(
        "compile-anonymous-lambdas",
        "aivi",
        concat!(
            "value increment : Int -> Int = x => x + 1\n",
            "value positive : Int -> Bool = . > 0\n",
            "value next : Int = increment 0\n",
            "value isNextPositive : Bool = positive next\n",
        ),
    );
    let output_dir = TempDir::new("compile-anonymous-lambdas");
    let output_path = output_dir.path().join("anonymous-lambdas.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected anonymous lambda compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("anonymous lambda compile should write an object file");
    assert!(
        metadata.len() > 0,
        "anonymous lambda object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_pattern_armed_reactive_update_program() {
    let input = TempFile::new(
        "compile-pattern-reactive-update",
        "aivi",
        concat!(
            "signal event = Some 3\n",
            "signal total : Signal Int = event\n",
            "  ||> Some value => value\n",
            "  ||> _ => 0\n",
        ),
    );
    let output_dir = TempDir::new("compile-pattern-reactive-update");
    let output_path = output_dir.path().join("pattern-reactive-update.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected pattern-armed reactive update compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::metadata(&output_path)
        .expect("pattern-armed reactive update compile should write an object file");
    assert!(
        metadata.len() > 0,
        "pattern-armed reactive update object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_temporal_signal_programs() {
    let input = TempFile::new(
        "compile-temporal-signal",
        "aivi",
        concat!(
            "fun delta:Int = previous:Int current:Int=>    current - previous\n",
            "\n",
            "provider custom.feed\n",
            "    wakeup: providerTrigger\n",
            "\n",
            "@source custom.feed\n",
            "signal score : Signal Int\n",
            "\n",
            "signal previousScore : Signal Int =\n",
            "    score\n",
            "     ~|> 0\n",
            "\n",
            "signal scoreDelta : Signal Int =\n",
            "    score\n",
            "     -|> 0\n",
            "\n",
            "signal scoreDeltaFn : Signal Int =\n",
            "    score\n",
            "     -|> delta\n",
        ),
    );
    let output_dir = TempDir::new("compile-temporal-signal");
    let output_path = output_dir.path().join("temporal-signal.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected temporal signal compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("temporal signal compile should write an object file");
    assert!(
        metadata.len() > 0,
        "temporal signal object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_accepts_delay_and_burst_pipe_stage_programs() {
    let input = TempFile::new(
        "compile-delay-burst",
        "aivi",
        concat!(
            "provider custom.feed\n",
            "    wakeup: providerTrigger\n",
            "\n",
            "@source custom.feed\n",
            "signal score : Signal Int\n",
            "\n",
            "signal delayedScore : Signal Int =\n",
            "    score\n",
            "     |> delay 200ms\n",
            "\n",
            "signal burstScore : Signal Int =\n",
            "    score\n",
            "     |> burst 75ms 3times\n",
        ),
    );
    let output_dir = TempDir::new("compile-delay-burst");
    let output_path = output_dir.path().join("delay-burst.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected delay/burst pipe stage compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::metadata(&output_path)
        .expect("delay/burst pipe stage compile should write an object file");
    assert!(
        metadata.len() > 0,
        "delay/burst pipe stage object file should not be empty"
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
fn compile_accepts_pipe_stage_memo_programs() {
    let input = TempFile::new(
        "compile-pipe-stage-memos",
        "aivi",
        concat!(
            "type Int -> Int -> Int\n",
            "func add = total x =>\n",
            "    total + x\n",
            "\n",
            "type StageChoice =\n",
            "  | Ready Int\n",
            "  | Missing\n",
            "\n",
            "type Int -> Result Text Int\n",
            "func nonNegative = n => n >= 0\n",
            " T|> Ok n\n",
            " F|> Err \"negative\"\n",
            "\n",
            "value transformed : Int = 20\n",
            " |> #before before + 1 #after\n",
            " |> after + before\n",
            "\n",
            "value tapped : Int = 20\n",
            " | #current current + 1 #same\n",
            " |> same + current\n",
            "\n",
            "value chosen : Int = Ready 2\n",
            " ||> Ready value -> value + 1 #resolved\n",
            " ||> Missing -> 0 #resolved\n",
            " |> resolved + 1\n",
            "\n",
            "value branched : Int = Some 2\n",
            " T|> . + 1 #branch\n",
            " F|> 0 #branch\n",
            " |> branch + 1\n",
            "\n",
            "value kept : Option Int = 2\n",
            " ?|> #candidate candidate > 0 #filtered\n",
            " |> filtered\n",
            "\n",
            "value checked : Result Text Int = 2\n",
            " !|> #candidate nonNegative candidate #checked\n",
            " |> checked\n",
        ),
    );
    let output_dir = TempDir::new("compile-pipe-stage-memos");
    let output_path = output_dir.path().join("pipe-stage-memos.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected pipe-stage memo compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata =
        fs::metadata(&output_path).expect("pipe-stage memo compile should write an object file");
    assert!(
        metadata.len() > 0,
        "pipe-stage memo object file should not be empty"
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
fn compile_accepts_source_pattern_reactive_update_program() {
    let input = TempFile::new(
        "compile-source-pattern-reactive-update",
        "aivi",
        concat!(
            "signal incoming : Signal (Option Int)\n",
            "signal total : Signal Int = incoming\n",
            "  ||> Some value => value\n",
            "  ||> _ => 0\n",
        ),
    );
    let output_dir = TempDir::new("compile-source-pattern-reactive-update");
    let output_path = output_dir.path().join("source-pattern-reactive-update.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected source-pattern reactive update compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::metadata(&output_path)
        .expect("source-pattern reactive update compile should write an object file");
    assert!(
        metadata.len() > 0,
        "source-pattern reactive update object file should not be empty"
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
    assert!(
        stdout.contains("runtime startup/link integration is not available yet"),
        "expected explicit runtime/link boundary, got stdout: {stdout}"
    );
}

#[test]
fn compile_writes_object_and_reports_codegen_boundary() {
    let input = TempFile::new(
        "compile-success",
        "aivi",
        concat!(
            "value maybePositive : Option Int = 2\n",
            " ?|> True\n",
            "\n",
            "value missingNumber : Option Int = 2\n",
            " ?|> False\n",
            "\n",
            "value maybeGreeting : Option Text = \"hello\"\n",
            " ?|> True\n",
            "\n",
            "value missingGreeting : Option Text = \"hello\"\n",
            " ?|> False\n",
        ),
    );
    let output_dir = TempDir::new("compile-success");
    let output_path = output_dir.path().join("fixture.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
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
fn compile_rejects_recursive_list_pattern_fixture_with_cycle_error() {
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
        !output.status.success(),
        "expected recursive list pattern compile to fail during backend lowering"
    );
    assert!(
        !output_path.exists(),
        "recursive list pattern compile should not emit an object file"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backend lowering detected a global item dependency cycle"),
        "expected global cycle diagnostic, got stderr: {stderr}"
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
    suffix ms : Int = value => Duration value
    type Duration -> Int
    toMillis value = value
    type Duration -> Duration -> Duration
    (+)

instance Eq Duration = {
    (==) left right = toMillis left == toMillis right
    (!=) left right = toMillis left != toMillis right
}

instance Ord Duration = {
    compare left right = compare (toMillis left) (toMillis right)
}

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

#[test]
fn compile_stops_at_hir_validation_for_invalid_result_blocks() {
    let input = TempFile::new(
        "compile-invalid-result-block",
        "aivi",
        concat!(
            "value broken: Result Text Int =\n",
            "    result {\n",
            "        x <- 42\n",
            "        x\n",
            "    }\n",
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .output()
        .expect("compile command should run");

    assert!(
        !output.status.success(),
        "expected invalid result block compile to fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::result-block-binding-not-result"),
        "expected result-block diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("compile pipeline stopped at HIR validation"),
        "expected compile to stop during HIR validation, got stderr: {stderr}"
    );
}

#[test]
fn compile_supports_dynamic_list_wrappers_with_partial_item_callables() {
    let input = TempFile::new(
        "compile-list-wrapper-partials",
        "aivi",
        r#"
type Int -> Int -> Int
fun add = left right => left + right

type Int -> Int -> Bool
fun above = threshold value => value > threshold

type Int -> Int -> (List Int)
fun duplicateFrom = base value => [base, value]

type Int -> (Option Int)
fun positiveOption = value => value > 0
 T|> Some value
 F|> None

type (List Int) -> (List Int)
fun mapped = values => map (add 1) values

type (List Int) -> Bool
fun anyAboveOne = values => any (above 1) values

type (List Int) -> (Option Int)
fun foundAboveOne = values => find (above 1) values

type (List Int) -> (List Int)
fun flattened = values => flatMap (duplicateFrom 0) values
"#,
    );
    let output_dir = TempDir::new("compile-list-wrapper-partials");
    let output_path = output_dir.path().join("list-wrapper-partials.o");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("compile")
        .arg(input.path())
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("compile command should run");

    assert!(
        output.status.success(),
        "expected compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::metadata(&output_path).is_ok(),
        "expected compile to emit an object file"
    );
}
