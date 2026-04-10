#[cfg(test)]
pub(crate) fn gtk_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = env::args_os();
    let _binary = args.next();

    let Some(first) = args.next() else {
        print_usage();
        return Ok(ExitCode::from(2));
    };

    if first == "--help" || first == "-h" || first == "help" {
        let subcommand = args.next();
        return print_help(subcommand.as_deref());
    }

    if first == "--version" || first == "-V" {
        println!("aivi {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }

    if first == OsString::from("check") {
        return run_check(args);
    }

    if first == OsString::from("compile") {
        return run_compile(args);
    }

    if first == OsString::from("build") {
        return run_build(args);
    }

    if first == OsString::from("run") {
        return run_markup(args);
    }

    if first == OsString::from("execute") {
        return run_execute(args);
    }

    if first == OsString::from("test") {
        return run_test(args);
    }

    if first == OsString::from("lex") {
        let path_arg = take_path_or_help(args)?;
        return match path_arg {
            PathOrHelp::Help => print_help(Some(std::ffi::OsStr::new("lex"))),
            PathOrHelp::Path(p) => lex_file(&p),
        };
    }

    if first == OsString::from("lsp") {
        return run_lsp(args);
    }

    if first == OsString::from("mcp") {
        return mcp::run_mcp(args);
    }

    if first == OsString::from("manual-snippets") {
        return manual_snippets::run(args);
    }

    if first == OsString::from("fmt") {
        return run_fmt(args);
    }

    if first == OsString::from("openapi-gen") {
        return run_openapi_gen(args);
    }

    // Default: treat the first argument as a path and run `check`.
    check_file(&PathBuf::from(first), false)
}

enum PathOrHelp {
    Path(PathBuf),
    Help,
}

fn take_path_or_help(mut args: impl Iterator<Item = OsString>) -> Result<PathOrHelp, String> {
    let arg = args
        .next()
        .ok_or_else(|| "expected a path argument".to_owned())?;
    if arg == "--help" || arg == "-h" {
        return Ok(PathOrHelp::Help);
    }
    let path = PathBuf::from(arg);
    if !path.exists() {
        eprintln!("error: file not found: {}", path.display());
        std::process::exit(2);
    }
    Ok(PathOrHelp::Path(path))
}

fn run_fmt(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let Some(next) = args.next() else {
        return Err("expected a path or --stdin/--check argument after `fmt`".to_owned());
    };

    if next == "--help" || next == "-h" {
        return print_help(Some(std::ffi::OsStr::new("fmt")));
    }

    if next == OsString::from("--stdin") {
        return format_stdin();
    }

    if next == OsString::from("--check") {
        // Collect remaining paths; if none given use no-op (no files = no changes).
        let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
        return format_check(&paths);
    }

    // Treat as a file path — format to stdout (legacy behaviour).
    format_file(&PathBuf::from(next))
}

fn run_openapi_gen(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let Some(next) = args.next() else {
        return Err("expected a spec path argument after `openapi-gen`\n\
             Usage: aivi openapi-gen <spec.yaml> [-o output.aivi]"
            .to_owned());
    };

    if next == "--help" || next == "-h" {
        return print_help(Some(std::ffi::OsStr::new("openapi-gen")));
    }

    let spec_path = PathBuf::from(&next);
    if !spec_path.exists() {
        return Err(format!("spec file not found: {}", spec_path.display()));
    }

    let mut output_path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == OsString::from("-o") {
            let out = args
                .next()
                .ok_or_else(|| "expected a path after `-o`".to_owned())?;
            output_path = Some(PathBuf::from(out));
        }
    }

    let spec = aivi_openapi::parse_spec(&spec_path).map_err(|e| format!("error: {e}"))?;
    let resolved = aivi_openapi::resolve_spec(spec, &spec_path).map_err(|errs| {
        errs.iter()
            .map(|e| format!("error: {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let generated = aivi_openapi::generate_aivi_types(&resolved);

    match output_path {
        Some(path) => {
            fs::write(&path, &generated.aivi_source)
                .map_err(|e| format!("failed to write output to `{}`: {e}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => {
            print!("{}", generated.aivi_source);
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Resolve the entry file for a CLI command, using `aivi.toml` `[run] entry`
/// as the fallback when no explicit path is provided on the command line.
fn resolve_command_entrypoint(
    command_name: &str,
    explicit_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let cwd = env::current_dir().map_err(|error| {
        format!("failed to determine current directory for `aivi {command_name}`: {error}")
    })?;
    resolve_v1_entrypoint(&cwd, explicit_path, None)
        .map(|resolved| resolved.entry_path().to_path_buf())
        .map_err(|error| format!("failed to resolve entrypoint for `aivi {command_name}`: {error}"))
}

fn run_check(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut timings = false;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("check")));
        }
        if argument == OsString::from("--timings") {
            timings = true;
            continue;
        }
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `check`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("check path was provided more than once".to_owned());
            }
            continue;
        }
        if requested_path.replace(PathBuf::from(&argument)).is_some() {
            return Err("check path was provided more than once".to_owned());
        }
    }

    // Directory: check every .aivi file found recursively inside it.
    if let Some(ref dir) = requested_path {
        if dir.is_dir() {
            return check_directory(dir, timings);
        }
    }

    // No path given and the manifest declares multiple apps: check them all.
    if requested_path.is_none() {
        let cwd = env::current_dir().map_err(|error| {
            format!("failed to determine current directory for `aivi check`: {error}")
        })?;
        let workspace_root = discover_workspace_root_from_directory(&cwd);
        let manifest = parse_manifest(&workspace_root)
            .map_err(|message| format!("failed to parse aivi.toml: {message}"))?;
        if manifest.apps.len() > 1 {
            return check_all_apps(&manifest.apps, &workspace_root, timings);
        }
    }

    let path = resolve_command_entrypoint("check", requested_path.as_deref())?;
    check_file(&path, timings)
}

fn run_compile(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut output = None;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("compile")));
        }
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `compile`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("compile path was provided more than once".to_owned());
            }
            continue;
        }
        if argument == OsString::from("-o") || argument == OsString::from("--output") {
            let artifact = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path after `-o`/`--output` for `compile`".to_owned())?;
            if output.replace(artifact).is_some() {
                return Err("compile output path was provided more than once".to_owned());
            }
            continue;
        }
        if requested_path.replace(PathBuf::from(&argument)).is_some() {
            return Err("compile path was provided more than once".to_owned());
        }
    }

    let path = resolve_command_entrypoint("compile", requested_path.as_deref())?;
    compile_file(&path, output.as_deref())
}

fn run_build(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut output = None;
    let mut requested_view = None;
    let mut requested_app = None;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("build")));
        }
        if argument == OsString::from("--app") {
            let name = args
                .next()
                .ok_or_else(|| "expected a name after `--app` for `build`".to_owned())?;
            if requested_app
                .replace(name.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("build app name was provided more than once".to_owned());
            }
            continue;
        }
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `build`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("build path was provided more than once".to_owned());
            }
            continue;
        }
        if argument == OsString::from("-o") || argument == OsString::from("--output") {
            let bundle = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path after `-o`/`--output` for `build`".to_owned())?;
            if output.replace(bundle).is_some() {
                return Err("build output path was provided more than once".to_owned());
            }
            continue;
        }

        if argument == OsString::from("--view") {
            let view = args
                .next()
                .ok_or_else(|| "expected a value name after `--view` for `build`".to_owned())?;
            if requested_view
                .replace(view.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("build view name was provided more than once".to_owned());
            }
            continue;
        }

        if requested_path.replace(PathBuf::from(&argument)).is_some() {
            return Err("build path was provided more than once".to_owned());
        }
    }

    let output =
        output.ok_or_else(|| "expected `-o`/`--output <directory>` for `build`".to_owned())?;
    let resolved = resolve_run_entrypoint_for_build(
        "build",
        requested_path.as_deref(),
        requested_app.as_deref(),
    )?;
    let view = requested_view
        .as_deref()
        .or(resolved.manifest_view.as_deref())
        .map(str::to_owned);
    if let Some(view) = &view {
        let segments: Vec<&str> = view.split('.').collect();
        if let Err(e) = validate_module_path(&segments) {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
    build_markup_bundle(&resolved.entry_path, &output, view.as_deref())
}

fn resolve_run_entrypoint_for_build(
    command_name: &str,
    explicit_path: Option<&Path>,
    app_name: Option<&str>,
) -> Result<ResolvedRunEntrypoint, String> {
    let cwd = env::current_dir().map_err(|error| {
        format!("failed to determine current directory for `aivi {command_name}`: {error}")
    })?;
    let resolved = resolve_v1_entrypoint(&cwd, explicit_path, app_name).map_err(|error| {
        format!("failed to resolve entrypoint for `aivi {command_name}`: {error}")
    })?;
    Ok(ResolvedRunEntrypoint {
        entry_path: resolved.entry_path().to_path_buf(),
        manifest_view: resolved.manifest_view().map(str::to_owned),
    })
}

fn run_markup(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut requested_view = None;
    let mut requested_app = None;
    let mut timings = false;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("run")));
        }

        if argument == OsString::from("--timings") {
            timings = true;
            continue;
        }

        if argument == OsString::from("--app") {
            let name = args
                .next()
                .ok_or_else(|| "expected a name after `--app` for `run`".to_owned())?;
            if requested_app
                .replace(name.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("run app name was provided more than once".to_owned());
            }
            continue;
        }

        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `run`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("run path was provided more than once".to_owned());
            }
            continue;
        }

        if argument == OsString::from("--view") {
            let view = args
                .next()
                .ok_or_else(|| "expected a value name after `--view` for `run`".to_owned())?;
            if requested_view
                .replace(view.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("run view name was provided more than once".to_owned());
            }
            continue;
        }

        if requested_path.replace(PathBuf::from(&argument)).is_some() {
            return Err("run path was provided more than once".to_owned());
        }
    }

    if let Some(view) = &requested_view {
        let segments: Vec<&str> = view.split('.').collect();
        if let Err(e) = validate_module_path(&segments) {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
    let cwd = env::current_dir().map_err(|error| {
        format!("failed to determine current directory for `aivi run`: {error}")
    })?;

    // When no explicit path or --app was given and the manifest defines multiple
    // [[app]] entries, launch every app as a separate subprocess so each gets its
    // own GTK main loop.
    if requested_path.is_none() && requested_app.is_none() {
        let workspace_root = discover_workspace_root_from_directory(&cwd);
        if let Ok(manifest) = parse_manifest(&workspace_root) {
            if manifest.apps.len() > 1 {
                let exe = env::current_exe()
                    .map_err(|e| format!("failed to locate aivi executable: {e}"))?;
                let mut children: Vec<std::process::Child> = manifest
                    .apps
                    .iter()
                    .map(|app| {
                        let mut cmd = std::process::Command::new(&exe);
                        cmd.arg("run").arg("--app").arg(&app.name);
                        if timings {
                            cmd.arg("--timings");
                        }
                        if let Some(view) = requested_view.as_deref().or(app.view.as_deref()) {
                            cmd.arg("--view").arg(view);
                        }
                        cmd.spawn().map_err(|e| {
                            format!("failed to spawn `aivi run --app {}`: {e}", app.name)
                        })
                    })
                    .collect::<Result<_, _>>()?;
                let mut any_failed = false;
                for child in &mut children {
                    match child.wait() {
                        Ok(status) if !status.success() => any_failed = true,
                        Err(e) => {
                            eprintln!("error waiting for child process: {e}");
                            any_failed = true;
                        }
                        _ => {}
                    }
                }
                return Ok(if any_failed {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                });
            }
        }
    }

    let resolved =
        resolve_run_entrypoint(&cwd, requested_path.as_deref(), requested_app.as_deref())?;
    let view = requested_view
        .as_deref()
        .or(resolved.manifest_view.as_deref())
        .map(str::to_owned);
    run_markup_file_with_launch_config(
        &resolved.entry_path,
        view.as_deref(),
        run_session::RunLaunchConfig::default(),
        timings,
    )
}

#[derive(Debug)]
struct ResolvedRunEntrypoint {
    entry_path: PathBuf,
    manifest_view: Option<String>,
}

fn resolve_run_entrypoint(
    current_dir: &Path,
    explicit_path: Option<&Path>,
    app_name: Option<&str>,
) -> Result<ResolvedRunEntrypoint, String> {
    let resolved = resolve_v1_entrypoint(current_dir, explicit_path, app_name)
        .map_err(|error| format!("failed to resolve entrypoint for `aivi run`: {error}"))?;
    Ok(ResolvedRunEntrypoint {
        entry_path: resolved.entry_path().to_path_buf(),
        manifest_view: resolved.manifest_view().map(str::to_owned),
    })
}

fn run_execute(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut program_args = Vec::new();
    let mut accepting_program_args = false;

    while let Some(argument) = args.next() {
        if accepting_program_args {
            program_args.push(argument.to_string_lossy().into_owned());
            continue;
        }
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("execute")));
        }
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `execute`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("execute path was provided more than once".to_owned());
            }
            continue;
        }
        if argument == OsString::from("--") {
            accepting_program_args = true;
            continue;
        }
        if requested_path.is_none() {
            requested_path = Some(PathBuf::from(&argument));
            continue;
        }
        return Err(format!(
            "unexpected execute argument `{}`; pass program arguments after `--`",
            argument.to_string_lossy()
        ));
    }

    let path = resolve_command_entrypoint("execute", requested_path.as_deref())?;
    execute_file(&path, &program_args)
}

fn run_test(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return print_help(Some(std::ffi::OsStr::new("test")));
        }
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `test`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("test path was provided more than once".to_owned());
            }
            continue;
        }
        if requested_path.replace(PathBuf::from(&argument)).is_some() {
            return Err("test path was provided more than once".to_owned());
        }
    }

    let path = resolve_command_entrypoint("test", requested_path.as_deref())?;
    test_file(&path)
}

fn validate_module_path(path: &[&str]) -> Result<(), String> {
    for segment in path {
        if segment.is_empty()
            || segment.contains("..")
            || *segment == "."
            || segment.contains('/')
            || segment.contains('\\')
        {
            return Err(format!("invalid module path segment: '{}'", segment));
        }
    }
    Ok(())
}

fn load_source(path: &Path) -> Result<(SourceDatabase, FileId), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path.to_path_buf(), text);
    Ok((sources, file_id))
}
