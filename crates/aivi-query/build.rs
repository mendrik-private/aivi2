use std::{env, fs, io::Write, path::Path};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let stdlib_root = Path::new(&manifest_dir).join("../../stdlib");
    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("stdlib_embedded.rs");

    let mut out = fs::File::create(&out_path).unwrap();
    writeln!(out, "/// Stdlib source files embedded in the binary.").unwrap();
    writeln!(
        out,
        "/// Key: path relative to stdlib root (e.g. `aivi/list.aivi`)."
    )
    .unwrap();
    writeln!(out, "static STDLIB_EMBEDDED: &[(&str, &str)] = &[").unwrap();

    collect_stdlib_files(&stdlib_root, &stdlib_root, &mut out);

    writeln!(out, "];").unwrap();

    walk_and_emit_rerun(&stdlib_root);
}

fn collect_stdlib_files(root: &Path, dir: &Path, out: &mut fs::File) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().unwrap().to_str().unwrap();
            if dir_name == "tests" {
                continue;
            }
            collect_stdlib_files(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("aivi") {
            let relative = path.strip_prefix(root).unwrap();
            let key = relative.to_str().unwrap().replace('\\', "/");
            writeln!(
                out,
                r#"    ("{key}", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib/{key}"))),"#
            )
            .unwrap();
        }
    }
}

fn walk_and_emit_rerun(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_and_emit_rerun(&path);
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
