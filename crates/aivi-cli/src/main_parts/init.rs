pub fn run_init(args: impl Iterator<Item = std::ffi::OsString>) -> Result<ExitCode, String> {
    let mut args = args.peekable();

    let name = args
        .next()
        .ok_or_else(|| "expected a project name after `init`\nUsage: aivi init <project-name>".to_owned())?;

    if name == "--help" || name == "-h" {
        return print_help(Some(std::ffi::OsStr::new("init")));
    }

    let dir = PathBuf::from(&name);
    if dir.exists() {
        return Err(format!(
            "directory `{}` already exists; choose a different project name",
            dir.display()
        ));
    }

    fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create directory `{}`: {e}", dir.display()))?;

    let project_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("untitled");

    write_project_files(&dir, project_name)?;

    eprintln!(
        "Created AIVI project `{project_name}` in {}",
        dir.display()
    );
    eprintln!("  cd {project_name}");
    eprintln!("  aivi run");

    Ok(ExitCode::SUCCESS)
}

fn write_project_files(dir: &std::path::Path, name: &str) -> Result<(), String> {
    let manifest = format!(
        "[workspace]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[run]\nentry = \"main.aivi\"\n"
    );
    fs::write(dir.join("aivi.toml"), manifest)
        .map_err(|e| format!("failed to write aivi.toml: {e}"))?;

    let main_source = format!(
        "\
use aivi.gtk (
    Application
    ApplicationWindow
    Box
    Label
    HeaderBar
)

value app = Application {{
    id: \"org.aivi.{name}\"
}}

value window = ApplicationWindow {{
    application: app
    title: \"{name}\"
    defaultSize: (800, 600)
    child: Box {{
        orientation: 0
        children: [
            HeaderBar {{
                title: \"{name}\"
                showTitle: True
            }}
            Label {{
                label: \"Hello from AIVI!\"
                halign: 3
                valign: 3
            }}
        ]
    }}
}}
"
    );
    fs::write(dir.join("main.aivi"), main_source)
        .map_err(|e| format!("failed to write main.aivi: {e}"))?;

    let gitignore = "/target\n.aivi-cache\n";
    fs::write(dir.join(".gitignore"), gitignore)
        .map_err(|e| format!("failed to write .gitignore: {e}"))?;

    Ok(())
}
