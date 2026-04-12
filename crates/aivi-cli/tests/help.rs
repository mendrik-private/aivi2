use std::process::Command;

#[test]
fn help_compile_makes_object_boundary_explicit() {
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("help")
        .arg("compile")
        .output()
        .expect("help compile command should run");

    assert!(
        output.status.success(),
        "expected help compile to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("native object file"),
        "expected object-file wording, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("This command stops at object emission."),
        "expected explicit object-emission boundary, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("standalone runnable GTK application"),
        "expected explicit runnable-boundary note, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("Use `aivi build` for the current")
            && stdout.contains("runnable bundle path"),
        "expected build guidance, got stdout: {stdout}"
    );
}

#[test]
fn help_build_explains_current_bundle_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("help")
        .arg("build")
        .output()
        .expect("help build command should run");

    assert!(
        output.status.success(),
        "expected help build to succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("runnable bundle directory"),
        "expected bundle wording, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("current runtime binary, bundled stdlib, and reachable workspace files"),
        "expected runtime+workspace wording, got stdout: {stdout}"
    );
    assert!(
        stdout.contains("`aivi compile`, which emits object code only"),
        "expected compile contrast, got stdout: {stdout}"
    );
}
