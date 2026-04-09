fn compile_file(path: &Path, output: Option<&Path>) -> Result<ExitCode, String> {
    require_file_exists(path)?;
    let snapshot = WorkspaceHirSnapshot::load(path)?;
    let syntax_failed = workspace_syntax_failed(&snapshot, |sources, diagnostics| {
        print_stage_diagnostics(CompileStage::Syntax, sources, diagnostics.iter())
    });
    if syntax_failed {
        print_pipeline_stop(CompileStage::Syntax);
        return Ok(ExitCode::FAILURE);
    }

    let (hir_lowering_failed, hir_validation_failed) = workspace_hir_failed(
        &snapshot,
        |sources, diagnostics| {
            print_stage_diagnostics(CompileStage::HirLowering, sources, diagnostics.iter())
        },
        |sources, diagnostics| {
            print_stage_diagnostics(CompileStage::HirValidation, sources, diagnostics.iter())
        },
    );
    if hir_lowering_failed {
        print_pipeline_stop(CompileStage::HirLowering);
        return Ok(ExitCode::FAILURE);
    }
    if hir_validation_failed {
        print_pipeline_stop(CompileStage::HirValidation);
        return Ok(ExitCode::FAILURE);
    }

    let parsed = snapshot.entry_parsed();
    let lowered = snapshot.entry_hir();
    let hir_module = lowered.module();
    let production_items = production_item_ids(hir_module);
    let excluded_test_items = hir_module.items().iter().count() - production_items.len();
    let excluded_markup_items = production_items
        .iter()
        .filter(|item_id| {
            matches!(
                hir_module.items().get(**item_id),
                Some(Item::Value(value))
                    if matches!(hir_module.exprs()[value.body].kind, ExprKind::Markup(_))
            )
        })
        .count();
    let core = match lower_runtime_module_with_items(hir_module, &production_items) {
        Ok(core) => core,
        Err(errors) => {
            print_stage_errors(CompileStage::TypedCoreLowering, errors.errors());
            print_pipeline_stop(CompileStage::TypedCoreLowering);
            return Ok(ExitCode::FAILURE);
        }
    };
    if let Err(errors) = validate_core_module(&core) {
        print_stage_errors(CompileStage::TypedCoreValidation, errors.errors());
        print_pipeline_stop(CompileStage::TypedCoreValidation);
        return Ok(ExitCode::FAILURE);
    }

    let lambda = match lower_lambda_module(&core) {
        Ok(lambda) => lambda,
        Err(errors) => {
            print_stage_errors(CompileStage::TypedLambdaLowering, errors.errors());
            print_pipeline_stop(CompileStage::TypedLambdaLowering);
            return Ok(ExitCode::FAILURE);
        }
    };
    if let Err(errors) = validate_lambda_module(&lambda) {
        print_stage_errors(CompileStage::TypedLambdaValidation, errors.errors());
        print_pipeline_stop(CompileStage::TypedLambdaValidation);
        return Ok(ExitCode::FAILURE);
    }

    let backend = match lower_backend_module(&lambda) {
        Ok(backend) => backend,
        Err(errors) => {
            print_stage_errors(CompileStage::BackendLowering, errors.errors());
            print_pipeline_stop(CompileStage::BackendLowering);
            return Ok(ExitCode::FAILURE);
        }
    };
    if let Err(errors) = validate_program(&backend) {
        print_stage_errors(CompileStage::BackendValidation, errors.errors());
        print_pipeline_stop(CompileStage::BackendValidation);
        return Ok(ExitCode::FAILURE);
    }

    let compiled = match compile_program_cached(&backend) {
        Ok(compiled) => compiled,
        Err(errors) => {
            print_stage_errors(CompileStage::Codegen, errors.errors());
            print_pipeline_stop(CompileStage::Codegen);
            return Ok(ExitCode::FAILURE);
        }
    };

    if let Some(output_path) = output {
        write_object_file(output_path, compiled.object())?;
    }

    println!("compile pipeline passed: {}", path.display());
    println!(
        "  syntax: ok ({} surface item{})",
        parsed.cst().items.len(),
        plural_suffix(parsed.cst().items.len())
    );
    let hir_item_count = hir_module.items().iter().count();
    println!(
        "  HIR: ok ({} item{}, {} `@test` item{} excluded, {} markup item{} excluded)",
        hir_item_count,
        plural_suffix(hir_item_count),
        excluded_test_items,
        plural_suffix(excluded_test_items),
        excluded_markup_items,
        plural_suffix(excluded_markup_items)
    );
    let core_item_count = core.items().iter().count();
    println!(
        "  typed core: ok ({} item{})",
        core_item_count,
        plural_suffix(core_item_count)
    );
    let lambda_item_count = lambda.items().iter().count();
    let lambda_closure_count = lambda.closures().iter().count();
    println!(
        "  typed lambda: ok ({} item{}, {} closure{})",
        lambda_item_count,
        plural_suffix(lambda_item_count),
        lambda_closure_count,
        plural_suffix(lambda_closure_count)
    );
    let backend_item_count = backend.items().iter().count();
    let pipeline_count = backend.pipelines().iter().count();
    let kernel_count = backend.kernels().iter().count();
    println!(
        "  backend: ok ({} item{}, {} pipeline{}, {} kernel{})",
        backend_item_count,
        plural_suffix(backend_item_count),
        pipeline_count,
        plural_suffix(pipeline_count),
        kernel_count,
        plural_suffix(kernel_count)
    );
    println!(
        "  codegen: ok ({} compiled kernel{}, {} byte{})",
        compiled.kernels().len(),
        plural_suffix(compiled.kernels().len()),
        compiled.object().len(),
        plural_suffix(compiled.object().len())
    );
    if let Some(output_path) = output {
        println!("  object file: {}", output_path.display());
    } else {
        println!("  object file: not written (pass -o/--output to persist it)");
    }
    println!(
        "runtime startup/link integration is not available yet; the supported CLI boundary is Cranelift object code, not a runnable GTK binary."
    );
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug)]
struct BuildBundleSummary {
    launcher_path: PathBuf,
    runtime_path: PathBuf,
    workspace_file_count: usize,
}

fn build_markup_bundle(
    path: &Path,
    output: &Path,
    requested_view: Option<&str>,
) -> Result<ExitCode, String> {
    require_file_exists(path)?;
    if let Some(view) = requested_view {
        validate_module_name(view)?;
    }
    let snapshot = WorkspaceHirSnapshot::load(path)?;
    let syntax_failed = workspace_syntax_failed(&snapshot, |sources, diagnostics| {
        print_diagnostics(sources, diagnostics.iter())
    });
    if syntax_failed {
        return Ok(ExitCode::FAILURE);
    }

    let (hir_lowering_failed, hir_validation_failed) = workspace_hir_failed(
        &snapshot,
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
    );
    if hir_lowering_failed || hir_validation_failed {
        return Ok(ExitCode::FAILURE);
    }

    let lowered = snapshot.entry_hir();
    let artifact = match prepare_run_artifact_with_query_context(
        &snapshot.sources,
        lowered.module(),
        &[],
        requested_view,
        Some(snapshot.backend_query_context()),
    ) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("{message}");
            return Ok(ExitCode::FAILURE);
        }
    };

    let summary = write_run_bundle(&snapshot, path, output, artifact.view_name.as_ref())?;
    println!("build bundle passed: {}", path.display());
    println!("  view: {}", artifact.view_name);
    println!(
        "  workspace closure: {} file{}",
        summary.workspace_file_count,
        plural_suffix(summary.workspace_file_count)
    );
    println!("  launcher: {}", summary.launcher_path.display());
    println!("  runtime: {}", summary.runtime_path.display());
    println!("  bundle: {}", output.display());
    println!(
        "build packages the current AIVI runtime, bundled stdlib, and reachable workspace sources into a runnable bundle directory."
    );
    Ok(ExitCode::SUCCESS)
}

fn write_run_bundle(
    snapshot: &WorkspaceHirSnapshot,
    entry_path: &Path,
    output: &Path,
    view_name: &str,
) -> Result<BuildBundleSummary, String> {
    if output.exists() {
        return Err(format!(
            "build output {} already exists; choose a fresh directory",
            output.display()
        ));
    }
    let staging_parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if staging_parent != Path::new(".") {
        fs::create_dir_all(staging_parent)
            .map_err(|error| format!("failed to create {}: {error}", staging_parent.display()))?;
    }
    // Create a temp staging dir in the same directory as output so that
    // `fs::rename` at the end is guaranteed to be an atomic same-filesystem
    // move.  `StagingDir` (backed by `tempfile::TempDir`) removes the
    // directory on drop even if the process panics before the rename.
    let staging_guard = StagingDir::new_in(staging_parent)?;

    let result = (|| {
        let staging = staging_guard.path();
        let workspace_root = discover_workspace_root(entry_path);
        let entry_relative = entry_path.strip_prefix(&workspace_root).map_err(|_| {
            format!(
                "failed to place {} under discovered workspace root {}",
                entry_path.display(),
                workspace_root.display()
            )
        })?;
        let stdlib_root = discover_bundled_stdlib_root()?;

        let runtime_path = staging.join("aivi");
        let current_exe = env::current_exe()
            .map_err(|error| format!("failed to locate current AIVI executable: {error}"))?;
        copy_file_with_permissions(&current_exe, &runtime_path)?;
        ensure_executable(&runtime_path)?;

        copy_dir_all(&stdlib_root, &staging.join("stdlib"))?;
        let workspace_file_count = copy_workspace_bundle_sources(
            snapshot,
            &workspace_root,
            &stdlib_root,
            &staging.join("app"),
        )?;

        let launcher_path = staging.join("run");
        write_bundle_launcher(&launcher_path, entry_relative, view_name)?;

        Ok(BuildBundleSummary {
            launcher_path,
            runtime_path,
            workspace_file_count,
        })
    })();

    match result {
        Ok(mut summary) => {
            fs::rename(staging_guard.path(), output).map_err(|error| {
                format!(
                    "failed to move build bundle into {}: {error}",
                    output.display()
                )
            })?;
            // The staging directory has been atomically moved to its final
            // location.  When `staging_guard` drops, `TempDir` attempts to
            // remove the original staging path, which no longer exists, so
            // the cleanup is a silent no-op.  The output directory is safe.
            summary.launcher_path = output.join("run");
            summary.runtime_path = output.join("aivi");
            Ok(summary)
        }
        Err(error) => {
            // staging_guard drops here, cleaning up the partial build directory.
            Err(error)
        }
    }
}

fn discover_workspace_root(path: &Path) -> PathBuf {
    let start = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for ancestor in start.ancestors() {
        if ancestor.join("aivi.toml").is_file() {
            return ancestor.to_path_buf();
        }
    }
    start.to_path_buf()
}

fn discover_bundled_stdlib_root() -> Result<PathBuf, String> {
    let mut candidates = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")];
    if let Ok(executable) = env::current_exe()
        && let Some(parent) = executable.parent()
    {
        candidates.push(parent.join("stdlib"));
        candidates.push(parent.join("../stdlib"));
    }

    for candidate in candidates {
        if candidate.join("aivi.toml").is_file() {
            return Ok(fs::canonicalize(&candidate).unwrap_or(candidate));
        }
    }

    Err(
        "failed to locate the bundled stdlib; expected `stdlib/aivi.toml` next to the AIVI executable or workspace checkout"
            .to_owned(),
    )
}

fn copy_workspace_bundle_sources(
    snapshot: &WorkspaceHirSnapshot,
    workspace_root: &Path,
    stdlib_root: &Path,
    output: &Path,
) -> Result<usize, String> {
    fs::create_dir_all(output)
        .map_err(|error| format!("failed to create {}: {error}", output.display()))?;

    let manifest = workspace_root.join("aivi.toml");
    if manifest.is_file() {
        copy_file_with_permissions(&manifest, &output.join("aivi.toml"))?;
    }

    let mut copied = 0;
    for file in &snapshot.files {
        let source_path = file.path(&snapshot.frontend.db);
        if let Ok(relative) = source_path.strip_prefix(workspace_root) {
            copy_file_with_permissions(&source_path, &output.join(relative))?;
            copied += 1;
            continue;
        }
        if source_path.strip_prefix(stdlib_root).is_ok() {
            continue;
        }
        return Err(format!(
            "build currently supports workspace source files plus bundled stdlib only; `{}` was loaded from outside both roots",
            source_path.display()
        ));
    }
    Ok(copied)
}

fn write_bundle_launcher(
    path: &Path,
    entry_relative: &Path,
    view_name: &str,
) -> Result<(), String> {
    let entry = shell_single_quote(&entry_relative.to_string_lossy());
    let view = shell_single_quote(view_name);
    let script = format!(
        "#!/bin/sh\nset -eu\nSCRIPT_DIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nENTRY_REL={entry}\nVIEW_NAME={view}\nexec \"$SCRIPT_DIR/aivi\" run \"$SCRIPT_DIR/app/$ENTRY_REL\" --view \"$VIEW_NAME\"\n"
    );
    fs::write(path, script)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    ensure_executable(path)
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("failed to iterate directory {}: {error}", source.display())
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to read entry type for {}: {error}",
                entry.path().display()
            )
        })?;
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &destination_path)?;
        } else if file_type.is_file() {
            copy_file_with_permissions(&entry.path(), &destination_path)?;
        }
    }
    Ok(())
}

fn copy_file_with_permissions(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|error| {
        format!(
            "failed to copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    let permissions = fs::metadata(source)
        .map_err(|error| format!("failed to stat {}: {error}", source.display()))?
        .permissions();
    fs::set_permissions(destination, permissions).map_err(|error| {
        format!(
            "failed to apply permissions to {}: {error}",
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<(), String> {
    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("failed to stat {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to mark {} executable: {error}", path.display()))
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn print_diagnostics<'a>(
    sources: &SourceDatabase,
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> bool {
    let mut saw_error = false;
    for diagnostic in diagnostics {
        eprintln!("{}\n", diagnostic.render(sources));
        if diagnostic.severity == Severity::Error {
            saw_error = true;
        }
    }
    saw_error
}

fn print_stage_diagnostics<'a>(
    stage: CompileStage,
    sources: &SourceDatabase,
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> bool {
    let mut saw_any = false;
    let mut saw_error = false;
    for diagnostic in diagnostics {
        if !saw_any {
            eprintln!("{} diagnostics:\n", stage.label());
            saw_any = true;
        }
        eprintln!("{}\n", diagnostic.render(sources));
        if diagnostic.severity == Severity::Error {
            saw_error = true;
        }
    }
    saw_error
}

fn print_stage_errors<E: std::fmt::Display>(stage: CompileStage, errors: &[E]) {
    eprintln!("{} failed:", stage.label());
    if errors.is_empty() {
        eprintln!("- no detailed errors were reported");
        return;
    }
    for error in errors {
        eprintln!("- {error}");
    }
}

fn print_pipeline_stop(stage: CompileStage) {
    eprintln!("compile pipeline stopped at {}.", stage.label());
}

fn write_object_file(path: &Path, object: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(path, object).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[derive(Clone, Copy, Debug)]
enum CompileStage {
    Syntax,
    HirLowering,
    HirValidation,
    TypedCoreLowering,
    TypedCoreValidation,
    TypedLambdaLowering,
    TypedLambdaValidation,
    BackendLowering,
    BackendValidation,
    Codegen,
}

impl CompileStage {
    const fn label(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::HirLowering => "HIR lowering",
            Self::HirValidation => "HIR validation",
            Self::TypedCoreLowering => "typed-core lowering",
            Self::TypedCoreValidation => "typed-core validation",
            Self::TypedLambdaLowering => "typed-lambda lowering",
            Self::TypedLambdaValidation => "typed-lambda validation",
            Self::BackendLowering => "backend lowering",
            Self::BackendValidation => "backend validation",
            Self::Codegen => "codegen",
        }
    }
}

fn lex_file(path: &Path) -> Result<ExitCode, String> {
    require_file_exists(path)?;
    let (sources, file_id) = load_source(path)?;
    let file = &sources[file_id];
    let lexed = lex_module(file);

    for token in lexed
        .tokens()
        .iter()
        .filter(|token| !token.kind().is_trivia())
    {
        println!(
            "{kind:?} @{start}..{end} {text:?}{line_start}",
            kind = token.kind(),
            start = token.span().start().as_u32(),
            end = token.span().end().as_u32(),
            text = token.text(file),
            line_start = if token.line_start() {
                " [line-start]"
            } else {
                ""
            },
        );
    }

    if lexed.has_errors() {
        for diagnostic in lexed.diagnostics() {
            eprintln!("{}\n", diagnostic.render(&sources));
        }
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn format_file(path: &Path) -> Result<ExitCode, String> {
    require_file_exists(path)?;
    let (sources, file_id) = load_source(path)?;
    let file = &sources[file_id];
    let parsed = parse_module(file);
    if parsed.has_errors() {
        for diagnostic in parsed.all_diagnostics() {
            eprintln!("{}\n", diagnostic.render(&sources));
        }
        return Ok(ExitCode::FAILURE);
    }

    let formatter = Formatter;
    print!("{}", formatter.format(&parsed.module));
    Ok(ExitCode::SUCCESS)
}

fn format_stdin() -> Result<ExitCode, String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file("<stdin>", source);
    let file = &sources[file_id];
    let parsed = parse_module(file);
    // Per plan/02: tolerate parse errors, emit formatted output regardless.
    let formatter = Formatter;
    print!("{}", formatter.format(&parsed.module));
    Ok(ExitCode::SUCCESS)
}

fn format_check(paths: &[PathBuf]) -> Result<ExitCode, String> {
    let mut any_changed = false;
    for path in paths {
        let (sources, file_id) = load_source(path)?;
        let file = &sources[file_id];
        let parsed = parse_module(file);
        let formatter = Formatter;
        let formatted = formatter.format(&parsed.module);
        if formatted != file.text() {
            println!("{}", path.display());
            any_changed = true;
        }
    }
    if any_changed {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_lsp(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    if args.any(|a| a == "--help" || a == "-h") {
        return print_help(Some(std::ffi::OsStr::new("lsp")));
    }
    tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?
        .block_on(aivi_lsp::run())
        .map_err(|e| format!("LSP server error: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

fn print_usage() {
    eprint!("{}", format_main_help());
}

pub(crate) fn print_help(subcommand: Option<&std::ffi::OsStr>) -> Result<ExitCode, String> {
    match subcommand.map(|s| s.to_string_lossy().into_owned()) {
        None => {
            print!("{}", format_main_help());
        }
        Some(name) if name == "help" => {
            print!("{}", format_main_help());
        }
        Some(name) => match format_subcommand_help(&name) {
            Some(text) => print!("{text}"),
            None => {
                return Err(format!(
                    "unknown command '{name}'. Run `aivi help` to see all commands."
                ));
            }
        },
    }
    Ok(ExitCode::SUCCESS)
}

fn format_main_help() -> String {
    format!(
        "\
aivi {} — a purely functional, reactive, GTK-first language

USAGE:
    aivi <command> [options]
    aivi <path>                     Shorthand for `aivi check <path>`

COMMANDS:
    check [path]                    Type-check a module, directory, or all apps
    compile <path> [-o <object>]    Compile a module to native object code
    build <path> -o <dir> [opts]    Package a runnable GTK app bundle
    run [path] [opts]               Launch a live GTK app
    execute <path> [-- args...]     Run a headless Task program
    test <path>                     Run @test declarations in a workspace
    lex <path>                      Dump the lossless token stream
    fmt <path|--stdin|--check>      Format AIVI source code
    openapi-gen <spec> [-o file]    Generate AIVI types from an OpenAPI spec
    lsp                             Start the language server (stdio)
    mcp [opts]                      Start the MCP introspection server (stdio)
    manual-snippets [opts]          Validate and format manual code blocks
    help [command]                  Show help for a command

OPTIONS:
    -h, --help                      Show this help message
    -V, --version                   Show version

Run `aivi help <command>` for detailed information about a specific command.
",
        env!("CARGO_PKG_VERSION")
    )
}

fn format_subcommand_help(name: &str) -> Option<String> {
    let text = match name {
        "check" => {
            "\
aivi check — type-check a module through HIR

USAGE:
    aivi check [<path>]

ARGS:
    <path>              Path to an .aivi source file, or a directory to check
                        recursively. When omitted, all [[app]] entries in
                        aivi.toml are checked; if only one app (or a [run]
                        entry) is defined, that single entry is checked.

DESCRIPTION:
    Lexes, parses, lowers, and validates one or more modules through the full
    HIR pipeline. Reports any syntax errors, name resolution failures, or type
    errors as diagnostics. Exits with code 0 when all files pass, 1 on any
    diagnostic error.
"
        }
        "compile" => {
            "\
aivi compile — compile a module to native object code

USAGE:
    aivi compile <path> [-o <object>]

ARGS:
    <path>              Path to an .aivi source file

OPTIONS:
    -o, --output <object>
            Path for the output object file. When omitted, the object
            is written to a default location derived from the input path.

DESCRIPTION:
    Lowers the module through typed core, typed lambda IR, backend IR,
    and Cranelift codegen to produce a native object file. Includes all
    compiler stages from parsing through machine code generation.
"
        }
        "build" => {
            "\
aivi build — package a runnable GTK app bundle

USAGE:
    aivi build <path> -o <directory> [--app <name>] [--view <name>]

ARGS:
    <path>              Path to an .aivi source file or workspace entry

OPTIONS:
    --app <name>
            Select a named app from `[[app]]` in aivi.toml.
            Required when multiple apps are defined and neither --path
            nor [run] entry is given.

    -o, --output <directory>    (required)
            Output directory for the packaged bundle. The directory will
            contain everything needed to run the application.

    --view <name>
            Dot-separated module path to the view entry point
            (e.g. \"app.main\"). When omitted, uses the default view.

DESCRIPTION:
    Compiles a GTK/libadwaita application and packages it into a
    self-contained bundle directory. The bundle includes compiled code,
    runtime assets, and the necessary metadata to launch the app.
"
        }
        "run" => {
            "\
aivi run — launch a live GTK app

USAGE:
    aivi run [<path>] [--path <path>] [--app <name>] [--view <name>]

ARGS:
    [<path>]            Path to an .aivi source file or workspace entry.
                        When omitted, resolves via aivi.toml [[app]] or
                        [run] entry, then <workspace>/main.aivi.

OPTIONS:
    --app <name>
            Select a named app from `[[app]]` in aivi.toml.
            Required when multiple apps are defined and neither --path
            nor [run] entry is given.

    --path <path>
            Explicit path to the entry file. Alternative to the
            positional argument.

    --view <name>
            Dot-separated module path to the view entry point
            (e.g. \"app.main\"). When omitted, uses the default view.

DESCRIPTION:
    Compiles and immediately launches a GTK/libadwaita application with
    the full reactive runtime, signal engine, source providers, and
    live widget tree. The app runs until the window is closed.
"
        }
        "execute" => {
            "\
aivi execute — run a headless Task program

USAGE:
    aivi execute <path> [-- args...]

ARGS:
    <path>              Path to an .aivi source file containing a
                        top-level `value main : Task ...` declaration.

    [-- args...]        Arguments passed to the program. Everything
                        after `--` is forwarded as program arguments.

DESCRIPTION:
    Evaluates a top-level Task value without GTK or the widget runtime.
    Useful for command-line tools, scripts, and batch processing written
    in AIVI. The program receives arguments via a ProcessArgs source.
"
        }
        "test" => {
            "\
aivi test — run @test declarations in a workspace

USAGE:
    aivi test <path>

ARGS:
    <path>              Path to an .aivi source file or workspace entry

DESCRIPTION:
    Discovers all `@test value ... : Task ...` declarations in the
    workspace and executes them. Each test runs in isolation. Reports
    pass/fail status for each test and exits with code 0 if all tests
    pass, 1 if any test fails.
"
        }
        "lex" => {
            "\
aivi lex — dump the lossless token stream

USAGE:
    aivi lex <path>

ARGS:
    <path>              Path to an .aivi source file

DESCRIPTION:
    Lexes the source file and prints every token in the lossless token
    stream, including whitespace and comments. Useful for debugging the
    lexer and inspecting tokenization behavior.
"
        }
        "fmt" => {
            "\
aivi fmt — format AIVI source code

USAGE:
    aivi fmt <path>
    aivi fmt --stdin
    aivi fmt --check [path...]

ARGS:
    <path>              Path to an .aivi source file. The formatted
                        output is written to stdout.

OPTIONS:
    --stdin             Read source code from stdin and write the
                        formatted output to stdout.

    --check [path...]   Check whether files are already formatted.
                        Exits with code 0 if all files are formatted,
                        1 if any file would change. Does not modify
                        files. When no paths are given, does nothing.

DESCRIPTION:
    Canonically formats AIVI source code. The formatter preserves
    semantics while normalizing whitespace, indentation, and layout.
    Files with parse errors are left unchanged.
"
        }
        "openapi-gen" => {
            "\
aivi openapi-gen — generate AIVI type declarations from an OpenAPI spec

USAGE:
    aivi openapi-gen <spec.yaml|spec.json> [-o output.aivi]

ARGS:
    <spec>              Path to an OpenAPI 3.x YAML or JSON spec file.

OPTIONS:
    -o <output>         Write the generated AIVI source to a file.
                        When omitted, output is written to stdout.

DESCRIPTION:
    Parses an OpenAPI 3.x specification and emits AIVI type declarations
    for all component schemas, a sum type for auth schemes, and a
    standard ApiError type. The generated file can be used alongside
    an @source api handle to give user-defined types to API responses.

    Example:
        aivi openapi-gen ./petstore.yaml -o types/petstore.aivi
"
        }
        "lsp" => {
            "\
aivi lsp — start the language server

USAGE:
    aivi lsp

DESCRIPTION:
    Starts the AIVI language server using the Language Server Protocol
    over stdio. Provides diagnostics, completion, hover, semantic
    tokens, formatting, go-to-definition, document symbols, and code
    lens capabilities to connected editors.

    Typically launched automatically by the VSCode extension or other
    LSP-compatible editors.
"
        }
        "mcp" => {
            "\
aivi mcp — start the MCP introspection server

USAGE:
    aivi mcp [--path <path>] [--view <name>]

OPTIONS:
    --path <path>
            Path to the .aivi source file or workspace entry.
            When omitted, resolves to <workspace>/main.aivi.

    --view <name>
            Dot-separated module path to the view entry point
            (e.g. \"app.main\"). When omitted, uses the default view.

DESCRIPTION:
    Starts a Model Context Protocol server over stdio for live app
    introspection. Provides tools to launch the app, inspect signals
    and sources, capture the GTK widget tree, emit synthetic events,
    and publish source values — enabling AI-assisted development and
    testing workflows.
"
        }
        "manual-snippets" => {
            "\
aivi manual-snippets — validate and format manual code blocks

USAGE:
    aivi manual-snippets [--root <dir>] [--todo <report.json>] [--write]

OPTIONS:
    --root <dir>
            Root directory containing markdown files to scan.
            Defaults to \"manual\".

    --todo <report.json>
            Path for a JSON report of unresolved or failing code
            blocks. When omitted, no report file is written.

    --write
            Rewrite markdown files in place with formatted code
            blocks. Without this flag, the command is read-only
            and only reports issues.

DESCRIPTION:
    Scans fenced ```aivi code blocks in markdown documentation files,
    validates that they parse and type-check, formats them canonically,
    and optionally rewrites the files. Produces a TODO report of blocks
    that need manual attention.
"
        }
        _ => return None,
    };
    Some(text.to_owned())
}

