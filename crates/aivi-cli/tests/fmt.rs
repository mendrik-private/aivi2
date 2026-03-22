use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(prefix: &str, contents: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "aivi-{prefix}-{}-{unique}.aivi",
            std::process::id()
        ));
        fs::write(&path, contents).expect("temporary input should be writable");
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

#[test]
fn fmt_normalizes_markup_layout() {
    let input = TempFile::new(
        "fmt-normalize",
        "val dashboard=<fragment><Label text=\"Inbox\"/><show when={True} keepMounted={True}><with value={formatCount count} as={label}><Label text={label}/></with></show></fragment>\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(
        output.status.success(),
        "fmt should succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        concat!(
            "val dashboard =\n",
            "    <fragment>\n",
            "        <Label text=\"Inbox\" />\n",
            "        <show when={True} keepMounted={True}>\n",
            "            <with value={formatCount count} as={label}>\n",
            "                <Label text={label} />\n",
            "            </with>\n",
            "        </show>\n",
            "    </fragment>\n",
        )
    );
}

#[test]
fn fmt_fails_on_syntax_errors() {
    let input = TempFile::new("fmt-error", "val broken = <show when={True}>\n");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(!output.status.success(), "fmt should fail on syntax errors");
    assert!(
        !output.stderr.is_empty(),
        "fmt should report diagnostics on stderr"
    );
}
