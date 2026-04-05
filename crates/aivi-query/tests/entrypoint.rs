use std::{
    fs,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use aivi_query::{EntrypointOrigin, EntrypointResolutionError, resolve_v1_entrypoint};

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-workspaces")
            .join(format!("aivi-query-{prefix}-{}-{unique}", process::id()));
        fs::create_dir_all(&path).expect("scratch directory should be creatable");
        Self { path }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("scratch parent directories should be creatable");
        }
        fs::write(&path, text).expect("scratch file should be writable");
        path
    }

    fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir_all(&path).expect("scratch directory should be creatable");
        path
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn explicit_override_bypasses_implicit_main_discovery() {
    let workspace = ScratchDir::new("explicit-override");
    workspace.write("aivi.toml", "");
    let cwd = workspace.mkdir("nested/tool");
    let explicit_entry = workspace.write("apps/demo.aivi", "value demo = 1\n");

    let resolved = resolve_v1_entrypoint(&cwd, Some(&explicit_entry), None)
        .expect("explicit overrides should bypass implicit workspace discovery");

    assert_eq!(resolved.origin(), EntrypointOrigin::ExplicitPath);
    assert_eq!(resolved.entry_path(), explicit_entry.as_path());
    assert_eq!(resolved.workspace_root(), workspace.path());
}

#[test]
fn implicit_resolution_uses_the_nearest_manifest_ancestor() {
    let workspace = ScratchDir::new("nearest-manifest");
    workspace.write("aivi.toml", "");
    workspace.write("main.aivi", "value outer = 1\n");
    let nested_root = workspace.mkdir("apps");
    workspace.write("apps/aivi.toml", "");
    let nested_entry = workspace.write("apps/main.aivi", "value inner = 2\n");
    let cwd = workspace.mkdir("apps/tooling");

    let resolved = resolve_v1_entrypoint(&cwd, None, None)
        .expect("implicit discovery should resolve the nearest workspace root");

    assert_eq!(resolved.origin(), EntrypointOrigin::ImplicitWorkspaceMain);
    assert_eq!(resolved.workspace_root(), nested_root.as_path());
    assert_eq!(resolved.entry_path(), nested_entry.as_path());
}

#[test]
fn implicit_resolution_uses_the_current_directory_when_no_manifest_exists() {
    let workspace = ScratchDir::new("cwd-fallback");
    let cwd = workspace.mkdir("playground");
    let entry = workspace.write("playground/main.aivi", "value view = 3\n");

    let resolved =
        resolve_v1_entrypoint(&cwd, None, None).expect("current directory should be the workspace root");

    assert_eq!(resolved.origin(), EntrypointOrigin::ImplicitWorkspaceMain);
    assert_eq!(resolved.workspace_root(), cwd.as_path());
    assert_eq!(resolved.entry_path(), entry.as_path());
}

#[test]
fn missing_implicit_main_reports_the_expected_path_without_guessing() {
    let workspace = ScratchDir::new("missing-main");
    workspace.write("aivi.toml", "");
    workspace.write("apps/alternate.aivi", "value alternate = 4\n");
    let cwd = workspace.mkdir("apps/tooling");
    let expected_path = workspace.path().join("main.aivi");

    let error = resolve_v1_entrypoint(&cwd, None, None)
        .expect_err("v1 discovery should refuse to guess when main.aivi is absent");

    assert_eq!(
        error,
        EntrypointResolutionError::MissingImplicitEntrypoint {
            workspace_root: workspace.path().to_path_buf(),
            expected_path: expected_path.clone(),
        }
    );
    assert_eq!(error.workspace_root(), workspace.path());
    assert_eq!(error.expected_path(), expected_path.as_path());
    let message = error.to_string();
    assert!(message.contains(expected_path.to_string_lossy().as_ref()));
    assert!(message.contains("--path <entry-file>"));
}
