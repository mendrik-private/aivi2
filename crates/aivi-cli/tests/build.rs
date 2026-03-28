use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("temporary subdirectories should be creatable");
        }
        fs::write(path, contents).expect("temporary file should be writable");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn build_writes_a_self_contained_runnable_bundle() {
    let workspace = TempDir::new("build-static-workspace");
    workspace.write(
        "main.aivi",
        r#"
value screenView =
    <Window title="AIVI" />
"#,
    );
    let output_root = TempDir::new("build-static-output");
    let bundle_path = output_root.path().join("app-bundle");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("build")
        .arg(workspace.path().join("main.aivi"))
        .arg("-o")
        .arg(&bundle_path)
        .output()
        .expect("build command should run");

    assert!(
        output.status.success(),
        "expected build to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("build bundle passed"),
        "expected build summary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains(&bundle_path.display().to_string()),
        "expected bundle path in stdout, got stdout: {stdout}"
    );

    let runtime_path = bundle_path.join("aivi");
    let runtime_metadata = fs::metadata(&runtime_path).expect("bundle should copy the runtime");
    assert!(
        runtime_metadata.len() > 0,
        "copied runtime should not be empty"
    );

    let launcher_path = bundle_path.join("run");
    let launcher = fs::read_to_string(&launcher_path).expect("bundle should write a launcher");
    assert!(
        launcher.contains("exec \"$SCRIPT_DIR/aivi\" run"),
        "expected launcher to invoke bundled runtime, got: {launcher}"
    );
    assert!(
        launcher.contains("screenView"),
        "expected launcher to pin the selected view, got: {launcher}"
    );

    #[cfg(unix)]
    assert_ne!(
        fs::metadata(&launcher_path)
            .expect("launcher metadata should exist")
            .permissions()
            .mode()
            & 0o111,
        0,
        "launcher should be executable"
    );

    let bundled_source = fs::read_to_string(bundle_path.join("app/main.aivi"))
        .expect("bundle should copy the entry source");
    assert_eq!(
        bundled_source,
        "\nvalue screenView =\n    <Window title=\"AIVI\" />\n"
    );
    assert!(
        bundle_path.join("stdlib/aivi.toml").is_file(),
        "bundle should include the stdlib workspace"
    );
}

#[test]
fn build_copies_the_loaded_workspace_closure() {
    let workspace = TempDir::new("build-workspace-closure");
    workspace.write("aivi.toml", "");
    workspace.write(
        "main.aivi",
        r#"
use shared.types (
    Greeting
)

type Welcome = Greeting

value appView =
    <Window title="Workspace" />
"#,
    );
    workspace.write(
        "shared/types.aivi",
        r#"
type Greeting = Text

export (Greeting)
"#,
    );
    let output_root = TempDir::new("build-workspace-output");
    let bundle_path = output_root.path().join("bundle");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("build")
        .arg(workspace.path().join("main.aivi"))
        .arg("-o")
        .arg(&bundle_path)
        .output()
        .expect("build command should run");

    assert!(
        output.status.success(),
        "expected workspace build to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        bundle_path.join("app/aivi.toml").is_file(),
        "bundle should preserve the workspace manifest"
    );
    assert!(
        bundle_path.join("app/shared/types.aivi").is_file(),
        "bundle should copy imported workspace files"
    );
}
