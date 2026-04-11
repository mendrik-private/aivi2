use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("temporary parent directories should be creatable");
        }
        fs::write(&path, text).expect("temporary file should be writable");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_command_accepts_stdlib_validation_files() {
    for (name, source, summary) in [
        (
            "text",
            concat!(
                "use aivi.text (nonEmpty)\n",
                "@test\n",
                "value text_ok : Task Text Bool = pure (nonEmpty \"Ada\")\n",
            ),
            "test result: ok. 1 passed; 0 failed; 1 total",
        ),
        (
            "list",
            concat!(
                "use aivi.list (length)\n",
                "@test\n",
                "value list_ok : Task Text Bool = pure (length [1, 2, 3] == 3)\n",
            ),
            "test result: ok. 1 passed; 0 failed; 1 total",
        ),
        (
            "option",
            concat!(
                "use aivi.option (getOrElse)\n",
                "@test\n",
                "value option_ok : Task Text Bool = pure (getOrElse 0 (Some 2) == 2)\n",
            ),
            "test result: ok. 1 passed; 0 failed; 1 total",
        ),
        (
            "float",
            concat!(
                "use aivi.core.float (abs)\n",
                "@test\n",
                "value float_ok : Task Text Bool = pure (abs (0.0 - 1.5) == 1.5)\n",
            ),
            "test result: ok. 1 passed; 0 failed; 1 total",
        ),
    ] {
        let dir = TempDir::new(&format!("test-stdlib-{name}"));
        let path = dir.write("main.aivi", source);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("test")
            .arg(&path)
            .output()
            .expect("test command should run");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "expected {name} bundled-stdlib test input to pass `aivi test`, stdout was: {stdout}, stderr was: {stderr}"
        );
        assert!(
            stderr.is_empty(),
            "expected {name} bundled-stdlib test input to keep stderr empty, stderr was: {stderr}"
        );
        assert!(
            stdout.contains(summary),
            "expected success summary for {name} bundled-stdlib test input, stdout was: {stdout}"
        );
    }
}

#[test]
fn test_command_reports_when_workspace_has_no_tests() {
    let dir = TempDir::new("test-no-tests");
    let path = dir.write("main.aivi", "value answer : Int = 42\n");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("test")
        .arg(&path)
        .output()
        .expect("test command should run");

    assert!(
        !output.status.success(),
        "expected `aivi test` to fail when no `@test` values exist"
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no `@test` values found in the loaded workspace"),
        "expected missing-test diagnostic, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_command_accepts_contains_predicate_forms() {
    let dir = TempDir::new("test-contains-predicate-forms");
    let path = dir.write(
        "main.aivi",
        concat!(
            "use aivi.list (contains)\n",
            "type Coord = Coord Int Int\n",
            "type Coord -> Coord -> Bool\n",
            "func coordEq = target candidate => candidate == target\n",
            "type Coord -> List Coord -> Bool\n",
            "func member = cell items => contains (coordEq cell) items\n",
            "value items : List Coord = [Coord 0 0, Coord 1 1]\n",
            "value cell : Coord = Coord 1 1\n",
            "value directMatch : Bool = contains (. == cell) items\n",
            "value helperMatch : Bool = member cell items\n",
            "value missingMatch : Bool = not (contains (. == (Coord 9 9)) items)\n",
            "@test\n",
            "value directContains : Task Text Bool = pure directMatch\n",
            "@test\n",
            "value helperContains : Task Text Bool = pure helperMatch\n",
            "@test\n",
            "value missingContains : Task Text Bool = pure missingMatch\n",
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("test")
        .arg(&path)
        .output()
        .expect("test command should run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected contains predicate forms to pass `aivi test`, stdout was: {stdout}, stderr was: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "expected contains predicate forms to keep stderr empty, stderr was: {stderr}"
    );
    assert!(
        stdout.contains("test result: ok. 3 passed; 0 failed; 3 total"),
        "expected success summary for contains predicate forms, stdout was: {stdout}"
    );
}
