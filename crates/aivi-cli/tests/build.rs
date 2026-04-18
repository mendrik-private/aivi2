use std::{
    collections::BTreeMap,
    env, fs,
    io::{Read, Seek, SeekFrom},
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

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

const EMBEDDED_BUNDLE_ARCHIVE_MAGIC: [u8; 16] = *b"AIVI_ARCHIVE_V1_";
const EMBEDDED_BUNDLE_FOOTER_MAGIC: [u8; 16] = *b"AIVI_BUNDLE_V1__";
const EMBEDDED_BUNDLE_FOOTER_LEN: u64 = 24;

fn read_embedded_bundle_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut file = fs::File::open(path).expect("built executable should exist");
    let file_len = file
        .metadata()
        .expect("built executable metadata should exist")
        .len();
    assert!(
        file_len >= EMBEDDED_BUNDLE_FOOTER_LEN,
        "built executable should contain an embedded bundle footer"
    );

    file.seek(SeekFrom::End(-(EMBEDDED_BUNDLE_FOOTER_LEN as i64)))
        .expect("should seek to embedded bundle footer");
    let mut magic = [0u8; 16];
    file.read_exact(&mut magic)
        .expect("should read embedded bundle footer magic");
    assert_eq!(
        magic, EMBEDDED_BUNDLE_FOOTER_MAGIC,
        "built executable should end with the embedded bundle footer"
    );
    let archive_len = read_u64(&mut file);
    let archive_offset = file_len
        .checked_sub(EMBEDDED_BUNDLE_FOOTER_LEN + archive_len)
        .expect("archive footer should point inside the executable");
    file.seek(SeekFrom::Start(archive_offset))
        .expect("should seek to embedded archive start");
    file.read_exact(&mut magic)
        .expect("should read embedded archive magic");
    assert_eq!(
        magic, EMBEDDED_BUNDLE_ARCHIVE_MAGIC,
        "embedded archive should carry the expected magic"
    );
    let entry_count = read_u32(&mut file);
    let mut entries = BTreeMap::new();
    for _ in 0..entry_count {
        let path_len = read_u32(&mut file) as usize;
        let file_len = read_u64(&mut file) as usize;
        let mut path_bytes = vec![0u8; path_len];
        file.read_exact(&mut path_bytes)
            .expect("should read embedded path bytes");
        let mut contents = vec![0u8; file_len];
        file.read_exact(&mut contents)
            .expect("should read embedded file bytes");
        let path = String::from_utf8(path_bytes).expect("embedded paths should be valid UTF-8");
        entries.insert(path, contents);
    }
    entries
}

fn read_u32(reader: &mut fs::File) -> u32 {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .expect("should read u32 from embedded bundle");
    u32::from_le_bytes(bytes)
}

fn read_u64(reader: &mut fs::File) -> u64 {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .expect("should read u64 from embedded bundle");
    u64::from_le_bytes(bytes)
}

#[test]
fn build_writes_a_self_contained_runnable_executable() {
    let workspace = TempDir::new("build-static-workspace");
    workspace.write(
        "main.aivi",
        r#"
value screenView =
    <Window title="AIVI" />
"#,
    );
    let output_root = TempDir::new("build-static-output");
    let executable_path = output_root.path().join("app");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("build")
        .arg(workspace.path().join("main.aivi"))
        .arg("-o")
        .arg(&executable_path)
        .output()
        .expect("build command should run");

    assert!(
        output.status.success(),
        "expected build to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("build executable passed"),
        "expected build summary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains(&executable_path.display().to_string()),
        "expected executable path in stdout, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("launcher size:"),
        "expected launcher size summary, got stdout: {stdout}"
    );

    let runtime_metadata =
        fs::metadata(&executable_path).expect("build should write the executable output");
    assert!(
        runtime_metadata.len() > 0,
        "built executable should not be empty"
    );
    let current_launcher_bytes = fs::metadata(env!("CARGO_BIN_EXE_aivi"))
        .expect("current aivi launcher metadata should exist")
        .len();
    if current_launcher_bytes > 100 * 1024 * 1024 {
        assert!(
            runtime_metadata.len() * 2 < current_launcher_bytes,
            "debug-profile executable builds should strip the staged launcher aggressively; launcher was {} bytes, built executable was {} bytes",
            current_launcher_bytes,
            runtime_metadata.len()
        );
    }

    #[cfg(unix)]
    assert_ne!(
        runtime_metadata.permissions().mode() & 0o111,
        0,
        "built executable should be executable"
    );

    let entries = read_embedded_bundle_entries(&executable_path);
    let run_artifact = entries
        .get("run-artifact.bin")
        .expect("embedded executable should carry the serialized run artifact");
    assert!(
        !run_artifact.is_empty(),
        "expected non-empty binary run artifact payload"
    );
    assert!(
        !entries.contains_key("run-artifact.json"),
        "embedded executable should no longer carry a JSON run artifact"
    );
    let payload_entries = entries
        .keys()
        .filter(|entry| entry.starts_with("payloads/"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        payload_entries.iter().all(|entry| !entry.ends_with(".json")),
        "embedded executable should not keep JSON backend payloads, got: {payload_entries:?}"
    );
    assert!(
        payload_entries.iter().any(|entry| entry.ends_with(".bin")),
        "embedded executable should keep binary payloads, got: {payload_entries:?}"
    );
    assert!(
        payload_entries
            .iter()
            .any(|entry| entry.starts_with("payloads/native-") && entry.ends_with(".bin")),
        "embedded executable should emit native kernel sidecars, got: {payload_entries:?}"
    );
    assert!(
        !entries.contains_key("main.aivi"),
        "source-free executables should not embed source files"
    );
    assert!(
        !entries.keys().any(|entry| entry.starts_with("stdlib/")),
        "source-free executables should not carry the stdlib workspace"
    );
}

#[test]
fn build_emits_a_source_free_executable_even_for_workspace_closures() {
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
    let executable_path = output_root.path().join("bundle-app");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("build")
        .arg(workspace.path().join("main.aivi"))
        .arg("-o")
        .arg(&executable_path)
        .output()
        .expect("build command should run");

    assert!(
        output.status.success(),
        "expected workspace build to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = read_embedded_bundle_entries(&executable_path);
    assert!(
        entries.contains_key("run-artifact.bin"),
        "embedded executable should carry a serialized run artifact"
    );
    assert!(
        !entries.contains_key("aivi.toml"),
        "source-free executables should not copy the workspace manifest"
    );
    assert!(
        !entries.contains_key("shared/types.aivi"),
        "source-free executables should not copy imported workspace files"
    );
}

#[test]
fn build_accepts_snake_and_reversi_demos() {
    let output_root = TempDir::new("build-demo-output");
    for demo in ["snake", "reversi"] {
        let executable_path = output_root.path().join(demo);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("build")
            .arg(repo_path(&format!("demos/{demo}.aivi")))
            .arg("-o")
            .arg(&executable_path)
            .output()
            .expect("demo build command should run");

        assert!(
            output.status.success(),
            "expected {demo} build to succeed, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            executable_path.is_file(),
            "expected {demo} executable to exist"
        );
        let entries = read_embedded_bundle_entries(&executable_path);
        assert!(
            entries.contains_key("run-artifact.bin"),
            "expected {demo} run artifact to exist"
        );
        assert!(
            entries.keys().any(|entry| entry.starts_with("payloads/")),
            "expected {demo} payload directory to be embedded"
        );
        assert!(
            entries.contains_key(".aivi-launch-cwd"),
            "expected {demo} executable to record its embedded launch cwd"
        );
        if demo == "snake" {
            assert_eq!(
                String::from_utf8(
                    entries
                        .get(".aivi-launch-cwd")
                        .expect("snake launch cwd metadata should exist")
                        .clone()
                )
                .expect("snake launch cwd metadata should stay valid UTF-8"),
                "demos",
                "snake executable should restore the demos cwd basename for asset lookup"
            );
            assert!(
                entries.contains_key("demos/assets/empty.png"),
                "snake executable should embed demo tile assets under demos/assets"
            );
        }
    }
}
