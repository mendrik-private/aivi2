use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

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
fn manual_snippets_writes_formatted_blocks() {
    let temp = TempDir::new("manual-snippets-format");
    let manual_root = temp.path().join("manual");
    let page = temp.write(
        "manual/guide/example.md",
        "# Example\n\n```aivi\nvalue answer=42\n```\n",
    );
    let todo_path = temp.path().join("todo.json");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("manual-snippets")
        .arg("--root")
        .arg(&manual_root)
        .arg("--todo")
        .arg(&todo_path)
        .arg("--write")
        .output()
        .expect("manual-snippets command should run");

    assert!(
        output.status.success(),
        "manual-snippets should succeed after formatting, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(page).expect("formatted markdown should be readable"),
        "# Example\n\n```aivi\nvalue answer = 42\n```\n"
    );

    let report: Value = serde_json::from_str(
        &fs::read_to_string(todo_path).expect("todo report should be readable"),
    )
    .expect("todo report should be valid json");
    assert_eq!(report["unresolved_fragments"], 0);
    assert_eq!(
        report["rewritten_blocks"],
        Value::from(1),
        "expected the formatter to rewrite one block"
    );
}

#[test]
fn manual_snippets_reports_unresolved_diagnostics() {
    let temp = TempDir::new("manual-snippets-diagnostics");
    let manual_root = temp.path().join("manual");
    temp.write(
        "manual/guide/example.md",
        "# Example\n\n```aivi\nuse missing.module (value)\n```\n",
    );
    let todo_path = temp.path().join("todo.json");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("manual-snippets")
        .arg("--root")
        .arg(&manual_root)
        .arg("--todo")
        .arg(&todo_path)
        .arg("--write")
        .output()
        .expect("manual-snippets command should run");

    assert!(
        !output.status.success(),
        "manual-snippets should fail when unresolved diagnostics remain"
    );

    let report: Value = serde_json::from_str(
        &fs::read_to_string(todo_path).expect("todo report should be readable"),
    )
    .expect("todo report should be valid json");
    assert_eq!(report["unresolved_fragments"], 1);
    assert_eq!(
        report["entries"][0]["lsp_problem_count"],
        Value::from(1),
        "expected the unresolved import to surface through LSP diagnostics"
    );
}

#[test]
fn manual_snippets_does_not_panic_on_imported_record_constructor_gaps() {
    let temp = TempDir::new("manual-snippets-imported-record");
    let manual_root = temp.path().join("manual");
    temp.write(
        "manual/guide/example.md",
        concat!(
            "# Example\n\n",
            "```aivi\n",
            "use aivi.data.json (\n",
            "    Json\n",
            "    JsonObject\n",
            ")\n\n",
            "value payload : Json = JsonObject {\n",
            "    wrong: []\n",
            "}\n\n",
            "```\n",
        ),
    );
    let todo_path = temp.path().join("todo.json");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("manual-snippets")
        .arg("--root")
        .arg(&manual_root)
        .arg("--todo")
        .arg(&todo_path)
        .output()
        .expect("manual-snippets command should run");

    assert_ne!(
        output.status.code(),
        Some(101),
        "manual-snippets should report diagnostics instead of panicking, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "manual-snippets should reject invalid imported-record fields, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
        "manual-snippets should not panic, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Value = serde_json::from_str(
        &fs::read_to_string(todo_path).expect("todo report should be readable"),
    )
    .expect("todo report should be valid json");
    assert_eq!(
        report["unresolved_fragments"],
        Value::from(1),
        "expected unresolved imported-record diagnostics instead of a panic"
    );
}
