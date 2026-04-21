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

    let backend = match lower_backend_module(&lambda, hir_module) {
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
struct BuildExecutableSummary {
    executable_path: PathBuf,
    embedded_file_count: usize,
    companion_file_count: usize,
    launcher_size_before_bytes: u64,
    launcher_size_after_bytes: u64,
    shrink_tool: Option<&'static str>,
}

#[derive(Debug)]
struct ExecutableShrinkSummary {
    size_before_bytes: u64,
    size_after_bytes: u64,
    tool: Option<&'static str>,
}

const EMBEDDED_BUNDLE_ARCHIVE_MAGIC: [u8; 16] = *b"AIVI_ARCHIVE_V1_";
const EMBEDDED_BUNDLE_FOOTER_MAGIC: [u8; 16] = *b"AIVI_BUNDLE_V1__";
const EMBEDDED_BUNDLE_FOOTER_LEN: u64 = 24;
const EMBEDDED_BUNDLE_LAUNCH_CWD_FILE: &str = ".aivi-launch-cwd";

#[derive(Debug)]
struct EmbeddedBundleDescriptor {
    archive_offset: u64,
    archive_len: u64,
}

#[derive(Debug)]
struct EmbeddedBundleEntry {
    relative_path: PathBuf,
    source_path: PathBuf,
}

#[derive(Debug)]
struct DecodedEmbeddedBundle {
    extracted_root: tempfile::TempDir,
    generated_entries: BTreeMap<PathBuf, Vec<u8>>,
}

#[derive(Debug)]
struct WorkspaceEmbeddingLayout {
    source_root: PathBuf,
    embedded_prefix: PathBuf,
    launch_cwd: PathBuf,
}

struct CurrentDirGuard {
    previous: PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &Path) -> Result<Self, String> {
        let previous = env::current_dir()
            .map_err(|error| format!("failed to determine current directory: {error}"))?;
        env::set_current_dir(path)
            .map_err(|error| format!("failed to change current directory to {}: {error}", path.display()))?;
        Ok(Self { previous })
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.previous);
    }
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

    let summary = write_run_executable(path, output, &artifact)?;
    println!("build executable passed: {}", path.display());
    println!("  view: {}", artifact.view_name);
    println!("  executable: {}", summary.executable_path.display());
    let shrink_suffix = summary
        .shrink_tool
        .map(|tool| format!(" via {tool}"))
        .unwrap_or_else(|| " (no strip tool found)".to_owned());
    println!(
        "  launcher size: {} -> {}{}",
        format_byte_size(summary.launcher_size_before_bytes),
        format_byte_size(summary.launcher_size_after_bytes),
        shrink_suffix
    );
    println!("  embedded files: {}", summary.embedded_file_count);
    println!("  companion files: {}", summary.companion_file_count);
    println!(
        "build packages the current AIVI runtime plus an embedded source-free app bundle into a single runnable executable."
    );
    Ok(ExitCode::SUCCESS)
}

fn write_run_executable(
    source_path: &Path,
    output: &Path,
    artifact: &RunArtifact,
) -> Result<BuildExecutableSummary, String> {
    if output.exists() {
        return Err(format!(
            "build output {} already exists; choose a fresh executable path",
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
        let bundle_root = staging_guard.path().join("bundle");
        fs::create_dir_all(&bundle_root)
            .map_err(|error| format!("failed to create {}: {error}", bundle_root.display()))?;
        if cfg!(test) {
            write_frozen_run_image_bundle_without_native_kernels(&bundle_root, artifact)?;
        } else {
            write_frozen_run_image_bundle(&bundle_root, artifact)?;
        }
        let workspace_layout = discover_workspace_embedding_layout(source_path);
        if !workspace_layout.launch_cwd.as_os_str().is_empty() {
            fs::create_dir_all(bundle_root.join(&workspace_layout.launch_cwd)).map_err(|error| {
                format!(
                    "failed to create embedded launch cwd {}: {error}",
                    bundle_root.join(&workspace_layout.launch_cwd).display()
                )
            })?;
            write_embedded_launch_cwd_file(&bundle_root, &workspace_layout.launch_cwd)?;
        }
        let companion_file_count = copy_workspace_companion_files(&workspace_layout, &bundle_root)?;

        let staged_executable = staging_guard.path().join(
            output
                .file_name()
                .ok_or_else(|| format!("invalid build output path {}", output.display()))?,
        );
        let current_exe = env::current_exe()
            .map_err(|error| format!("failed to locate current AIVI executable: {error}"))?;
        copy_file_with_permissions(&current_exe, &staged_executable)?;
        ensure_executable(&staged_executable)?;
        let shrink_summary = shrink_staged_executable(&staged_executable)?;
        ensure_executable(&staged_executable)?;
        let embedded_file_count = append_embedded_bundle(&staged_executable, &bundle_root)?;
        Ok(BuildExecutableSummary {
            executable_path: staged_executable,
            embedded_file_count,
            companion_file_count,
            launcher_size_before_bytes: shrink_summary.size_before_bytes,
            launcher_size_after_bytes: shrink_summary.size_after_bytes,
            shrink_tool: shrink_summary.tool,
        })
    })();

    match result {
        Ok(mut summary) => {
            fs::rename(&summary.executable_path, output).map_err(|error| {
                format!(
                    "failed to move build executable into {}: {error}",
                    output.display()
                )
            })?;
            summary.executable_path = output.to_path_buf();
            Ok(summary)
        }
        Err(error) => Err(error),
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

fn shrink_staged_executable(path: &Path) -> Result<ExecutableShrinkSummary, String> {
    let size_before_bytes = fs::metadata(path)
        .map_err(|error| format!("failed to stat staged executable {}: {error}", path.display()))?
        .len();
    for tool in ["llvm-strip", "strip"] {
        match std::process::Command::new(tool).arg(path).output() {
            Ok(output) => {
                if !output.status.success() {
                    return Err(format!(
                        "failed to shrink staged executable {} with {tool}: {}",
                        path.display(),
                        render_process_failure(&output)
                    ));
                }
                let size_after_bytes = fs::metadata(path)
                    .map_err(|error| {
                        format!("failed to stat stripped executable {}: {error}", path.display())
                    })?
                    .len();
                return Ok(ExecutableShrinkSummary {
                    size_before_bytes,
                    size_after_bytes,
                    tool: Some(tool),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to launch {tool} while shrinking staged executable {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(ExecutableShrinkSummary {
        size_before_bytes,
        size_after_bytes: size_before_bytes,
        tool: None,
    })
}

fn render_process_failure(output: &std::process::Output) -> String {
    let status = output
        .status
        .code()
        .map_or_else(|| "terminated by signal".to_owned(), |code| code.to_string());
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    match (stderr.is_empty(), stdout.is_empty()) {
        (true, true) => format!("exit status {status}"),
        (false, true) => format!("exit status {status}; stderr: {stderr}"),
        (true, false) => format!("exit status {status}; stdout: {stdout}"),
        (false, false) => format!("exit status {status}; stderr: {stderr}; stdout: {stdout}"),
    }
}

fn format_byte_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn discover_workspace_embedding_layout(path: &Path) -> WorkspaceEmbeddingLayout {
    let source_root = discover_workspace_root(path);
    if source_root.join("aivi.toml").is_file() {
        return WorkspaceEmbeddingLayout {
            source_root,
            embedded_prefix: PathBuf::new(),
            launch_cwd: PathBuf::new(),
        };
    }
    let basename = source_root
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("app"));
    WorkspaceEmbeddingLayout {
        source_root,
        embedded_prefix: basename.clone(),
        launch_cwd: basename,
    }
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
    let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
    let mut saw_error = false;
    for diagnostic in diagnostics {
        eprintln!("{}\n", renderer.render(diagnostic, sources));
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
    let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
    let mut saw_any = false;
    let mut saw_error = false;
    for diagnostic in diagnostics {
        if !saw_any {
            eprintln!("{} diagnostics:\n", stage.label());
            saw_any = true;
        }
        eprintln!("{}\n", renderer.render(diagnostic, sources));
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

fn maybe_run_embedded_build_output(arguments: &[OsString]) -> Result<Option<ExitCode>, String> {
    if !arguments.is_empty() {
        return Ok(None);
    }
    let executable = env::current_exe()
        .map_err(|error| format!("failed to determine current executable: {error}"))?;
    let Some(bundle) = read_embedded_bundle_descriptor(&executable)? else {
        return Ok(None);
    };
    let decoded = decode_embedded_bundle(&executable, &bundle)?;
    let artifact = load_embedded_run_artifact(&decoded.generated_entries, None)?;
    let launch_cwd = read_embedded_launch_cwd_from_entries(&decoded.generated_entries)?
        .map(|relative| decoded.extracted_root.path().join(relative))
        .unwrap_or_else(|| decoded.extracted_root.path().to_path_buf());
    fs::create_dir_all(&launch_cwd)
        .map_err(|error| format!("failed to create embedded launch cwd {}: {error}", launch_cwd.display()))?;
    let _cwd_guard = CurrentDirGuard::enter(&launch_cwd)?;
    let exit = run_session::launch_run_with_config(
        &executable,
        artifact,
        run_session::RunLaunchConfig::new(SourceProviderManager::with_context(
            SourceProviderContext::current().with_app_dir(launch_cwd.clone()),
        )),
        |_, _| {},
        |_| {},
    )?;
    Ok(Some(exit))
}

fn load_embedded_run_artifact(
    entries: &BTreeMap<PathBuf, Vec<u8>>,
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let frozen_image_key = PathBuf::from(FROZEN_RUN_IMAGE_FILE_NAME);
    if let Some(image_bytes) = entries.get(&frozen_image_key) {
        return load_frozen_run_image_from_bytes(image_bytes, requested_view);
    }
    load_serialized_run_artifact_from_bundle_entries(entries, requested_view)
}

fn load_serialized_run_artifact_from_bundle_entries(
    entries: &BTreeMap<PathBuf, Vec<u8>>,
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let artifact_key = PathBuf::from(RUN_ARTIFACT_FILE_NAME);
    let artifact_bytes = entries
        .get(&artifact_key)
        .ok_or_else(|| format!("embedded bundle is missing {}", artifact_key.display()))?
        .clone();
    let entry_bytes = entries.clone();
    load_serialized_run_artifact_from_bytes(
        artifact_bytes.as_slice(),
        requested_view,
        Box::new(move |relative_path| {
            let key = validate_embedded_bundle_relative_path(relative_path)?;
            entry_bytes.get(&key).cloned().ok_or_else(|| {
                format!(
                    "embedded bundle is missing generated payload {}",
                    key.display()
                )
            })
        }),
    )
}

fn copy_workspace_companion_files(
    workspace_layout: &WorkspaceEmbeddingLayout,
    destination_root: &Path,
) -> Result<usize, String> {
    let mut copied = 0usize;
    copy_workspace_companion_files_recursive(
        workspace_layout,
        &workspace_layout.source_root,
        destination_root,
        &mut copied,
    )?;
    Ok(copied)
}

fn copy_workspace_companion_files_recursive(
    workspace_layout: &WorkspaceEmbeddingLayout,
    current: &Path,
    destination_root: &Path,
    copied: &mut usize,
) -> Result<(), String> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(&workspace_layout.source_root)
            .map_err(|error| format!("failed to relativize {}: {error}", path.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            if should_skip_workspace_dir(relative) {
                continue;
            }
            copy_workspace_companion_files_recursive(
                workspace_layout,
                &path,
                destination_root,
                copied,
            )?;
            continue;
        }
        if !file_type.is_file() || !should_embed_workspace_file(relative) {
            continue;
        }
        let destination_relative = if workspace_layout.embedded_prefix.as_os_str().is_empty() {
            relative.to_path_buf()
        } else {
            workspace_layout.embedded_prefix.join(relative)
        };
        let destination = destination_root.join(&destination_relative);
        if destination.exists() {
            return Err(format!(
                "workspace companion file {} conflicts with generated bundle content",
                destination_relative.display()
            ));
        }
        copy_file_with_permissions(&path, &destination)?;
        *copied += 1;
    }

    Ok(())
}

fn should_skip_workspace_dir(relative: &Path) -> bool {
    let Some(name) = relative.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(name, ".git" | ".hg" | ".svn" | "target" | "out" | "node_modules")
}

fn should_embed_workspace_file(relative: &Path) -> bool {
    let Some(name) = relative.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name == "aivi.toml" {
        return false;
    }
    relative.extension().and_then(|extension| extension.to_str()) != Some("aivi")
}

fn append_embedded_bundle(executable: &Path, bundle_root: &Path) -> Result<usize, String> {
    let entries = collect_embedded_bundle_entries(bundle_root)?;
    let mut output = fs::OpenOptions::new()
        .append(true)
        .open(executable)
        .map_err(|error| format!("failed to open {} for bundle append: {error}", executable.display()))?;
    let mut archive_len = 0u64;

    write_archive_chunk(&mut output, &EMBEDDED_BUNDLE_ARCHIVE_MAGIC, &mut archive_len)?;
    write_archive_chunk(
        &mut output,
        &(entries.len() as u32).to_le_bytes(),
        &mut archive_len,
    )?;
    for entry in &entries {
        let relative = entry
            .relative_path
            .to_str()
            .ok_or_else(|| format!("embedded bundle path {} is not valid UTF-8", entry.relative_path.display()))?;
        let path_bytes = relative.as_bytes();
        write_archive_chunk(
            &mut output,
            &(path_bytes.len() as u32).to_le_bytes(),
            &mut archive_len,
        )?;
        let file_size = fs::metadata(&entry.source_path)
            .map_err(|error| format!("failed to stat {}: {error}", entry.source_path.display()))?
            .len();
        write_archive_chunk(&mut output, &file_size.to_le_bytes(), &mut archive_len)?;
        write_archive_chunk(&mut output, path_bytes, &mut archive_len)?;
        let mut source = fs::File::open(&entry.source_path)
            .map_err(|error| format!("failed to open {}: {error}", entry.source_path.display()))?;
        let copied = std::io::copy(&mut source, &mut output)
            .map_err(|error| format!("failed to append {}: {error}", entry.source_path.display()))?;
        archive_len = archive_len
            .checked_add(copied)
            .ok_or_else(|| "embedded bundle archive exceeded maximum size".to_owned())?;
    }

    output
        .write_all(&EMBEDDED_BUNDLE_FOOTER_MAGIC)
        .map_err(|error| format!("failed to finalize {}: {error}", executable.display()))?;
    output
        .write_all(&archive_len.to_le_bytes())
        .map_err(|error| format!("failed to finalize {}: {error}", executable.display()))?;
    Ok(entries.len())
}

fn collect_embedded_bundle_entries(root: &Path) -> Result<Vec<EmbeddedBundleEntry>, String> {
    let mut entries = Vec::new();
    collect_embedded_bundle_entries_recursive(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn collect_embedded_bundle_entries_recursive(
    root: &Path,
    current: &Path,
    entries: &mut Vec<EmbeddedBundleEntry>,
) -> Result<(), String> {
    let mut dir_entries = fs::read_dir(current)
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?;
    dir_entries.sort_by_key(|entry| entry.file_name());

    for entry in dir_entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_embedded_bundle_entries_recursive(root, &path, entries)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative_path = path
            .strip_prefix(root)
            .map_err(|error| format!("failed to relativize {}: {error}", path.display()))?
            .to_path_buf();
        entries.push(EmbeddedBundleEntry {
            relative_path,
            source_path: path,
        });
    }
    Ok(())
}

fn write_archive_chunk(
    output: &mut fs::File,
    bytes: &[u8],
    archive_len: &mut u64,
) -> Result<(), String> {
    output
        .write_all(bytes)
        .map_err(|error| format!("failed to append embedded bundle: {error}"))?;
    *archive_len = archive_len
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| "embedded bundle archive exceeded maximum size".to_owned())?;
    Ok(())
}

fn write_embedded_launch_cwd_file(bundle_root: &Path, launch_cwd: &Path) -> Result<(), String> {
    let contents = launch_cwd
        .to_str()
        .ok_or_else(|| format!("launch cwd {} is not valid UTF-8", launch_cwd.display()))?;
    fs::write(bundle_root.join(EMBEDDED_BUNDLE_LAUNCH_CWD_FILE), contents).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            bundle_root.join(EMBEDDED_BUNDLE_LAUNCH_CWD_FILE).display()
        )
    })
}

fn read_embedded_launch_cwd_from_entries(
    entries: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<Option<PathBuf>, String> {
    let Some(bytes) = entries.get(&PathBuf::from(EMBEDDED_BUNDLE_LAUNCH_CWD_FILE)) else {
        return Ok(None);
    };
    let text = std::str::from_utf8(bytes).map_err(|error| {
        format!(
            "embedded launch cwd metadata {} is not valid UTF-8: {error}",
            EMBEDDED_BUNDLE_LAUNCH_CWD_FILE
        )
    })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    validate_embedded_bundle_relative_path(trimmed).map(Some)
}

fn read_embedded_bundle_descriptor(path: &Path) -> Result<Option<EmbeddedBundleDescriptor>, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let file_len = file
        .metadata()
        .map_err(|error| format!("failed to stat {}: {error}", path.display()))?
        .len();
    if file_len < EMBEDDED_BUNDLE_FOOTER_LEN {
        return Ok(None);
    }

    use std::io::{Read, Seek, SeekFrom};

    file.seek(SeekFrom::End(-(EMBEDDED_BUNDLE_FOOTER_LEN as i64)))
        .map_err(|error| format!("failed to seek {}: {error}", path.display()))?;
    let mut magic = [0u8; 16];
    file.read_exact(&mut magic)
        .map_err(|error| format!("failed to read embedded bundle footer from {}: {error}", path.display()))?;
    if magic != EMBEDDED_BUNDLE_FOOTER_MAGIC {
        return Ok(None);
    }
    let mut len_bytes = [0u8; 8];
    file.read_exact(&mut len_bytes)
        .map_err(|error| format!("failed to read embedded bundle footer from {}: {error}", path.display()))?;
    let archive_len = u64::from_le_bytes(len_bytes);
    let archive_offset = file_len.checked_sub(EMBEDDED_BUNDLE_FOOTER_LEN + archive_len).ok_or_else(
        || format!("embedded bundle footer in {} points before the start of the file", path.display()),
    )?;
    Ok(Some(EmbeddedBundleDescriptor {
        archive_offset,
        archive_len,
    }))
}

fn decode_embedded_bundle(
    executable: &Path,
    descriptor: &EmbeddedBundleDescriptor,
) -> Result<DecodedEmbeddedBundle, String> {
    use std::io::{Read, Seek, SeekFrom};

    let extracted_root = tempfile::Builder::new()
        .prefix(".aivi-embedded-run-")
        .tempdir()
        .map_err(|error| format!("failed to create embedded bundle tempdir: {error}"))?;
    let mut generated_entries = BTreeMap::new();
    let mut input = fs::File::open(executable)
        .map_err(|error| format!("failed to open {}: {error}", executable.display()))?;
    input
        .seek(SeekFrom::Start(descriptor.archive_offset))
        .map_err(|error| format!("failed to seek {}: {error}", executable.display()))?;
    let mut consumed = 0u64;
    let mut magic = [0u8; 16];
    input
        .read_exact(&mut magic)
        .map_err(|error| format!("failed to read embedded archive header from {}: {error}", executable.display()))?;
    consumed += 16;
    if magic != EMBEDDED_BUNDLE_ARCHIVE_MAGIC {
        return Err(format!(
            "{} contains an unsupported embedded bundle archive",
            executable.display()
        ));
    }
    let entry_count = read_u32_le_from(&mut input, executable)?;
    consumed += 4;

    for _ in 0..entry_count {
        let path_len = read_u32_le_from(&mut input, executable)? as usize;
        consumed += 4;
        let file_len = read_u64_le_from(&mut input, executable)?;
        consumed += 8;
        let mut path_bytes = vec![0u8; path_len];
        input.read_exact(&mut path_bytes).map_err(|error| {
            format!(
                "failed to read embedded bundle path from {}: {error}",
                executable.display()
            )
        })?;
        consumed += path_len as u64;
        let relative = std::str::from_utf8(&path_bytes).map_err(|error| {
            format!(
                "embedded bundle path in {} is not valid UTF-8: {error}",
                executable.display()
            )
        })?;
        let relative = validate_embedded_bundle_relative_path(relative)?;
        let mut bytes = vec![0u8; file_len as usize];
        input.read_exact(&mut bytes).map_err(|error| {
            format!(
                "failed to read embedded bundle entry {} from {}: {error}",
                relative.display(),
                executable.display()
            )
        })?;
        consumed += file_len;
        if should_keep_embedded_entry_in_memory(&relative) {
            generated_entries.insert(relative, bytes);
            continue;
        }
        let destination = extracted_root.path().join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&destination, bytes)
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    }

    if consumed != descriptor.archive_len {
        return Err(format!(
            "{} embedded bundle length mismatch: expected {} bytes, decoded {}",
            executable.display(),
            descriptor.archive_len,
            consumed
        ));
    }

    Ok(DecodedEmbeddedBundle {
        extracted_root,
        generated_entries,
    })
}

fn should_keep_embedded_entry_in_memory(relative: &Path) -> bool {
    if relative == Path::new(FROZEN_RUN_IMAGE_FILE_NAME) {
        return true;
    }
    if relative == Path::new(RUN_ARTIFACT_FILE_NAME) {
        return true;
    }
    if relative == Path::new(EMBEDDED_BUNDLE_LAUNCH_CWD_FILE) {
        return true;
    }
    matches!(
        relative.components().next(),
        Some(std::path::Component::Normal(name)) if name == std::ffi::OsStr::new(RUN_ARTIFACT_PAYLOAD_DIR)
    )
}

fn read_u32_le_from(reader: &mut fs::File, path: &Path) -> Result<u32, String> {
    use std::io::Read;

    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| format!("failed to read embedded bundle header from {}: {error}", path.display()))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_le_from(reader: &mut fs::File, path: &Path) -> Result<u64, String> {
    use std::io::Read;

    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| format!("failed to read embedded bundle header from {}: {error}", path.display()))?;
    Ok(u64::from_le_bytes(bytes))
}

fn validate_embedded_bundle_relative_path(path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        return Err(format!("embedded bundle path {path:?} must stay relative"));
    }
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("embedded bundle path {path:?} must not escape the bundle root"));
    }
    Ok(candidate)
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
        let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
        for diagnostic in lexed.diagnostics() {
            eprintln!("{}\n", renderer.render(diagnostic, &sources));
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
        let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
        for diagnostic in parsed.all_diagnostics() {
            eprintln!("{}\n", renderer.render(diagnostic, &sources));
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
    compile <path> [-o <object>]    Compile a module to native object code only
    build <path> -o <file> [opts]   Package a runnable source-free GTK executable
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

    This command stops at object emission. It does not currently link a
    standalone runnable GTK application. Use `aivi build` for the current
    runnable executable path.
"
        }
        "build" => {
            "\
    aivi build — package a runnable GTK executable

USAGE:
    aivi build <path> -o <executable> [--app <name>] [--view <name>]

ARGS:
    <path>              Path to an .aivi source file or workspace entry

OPTIONS:
    --app <name>
            Select a named app from `[[app]]` in aivi.toml.
            Required when multiple apps are defined and neither --path
            nor [run] entry is given.

    -o, --output <executable>   (required)
            Output path for the packaged executable. The file will be
            directly runnable and contains the embedded app bundle.

    --view <name>
            Dot-separated module path to the view entry point
            (e.g. \"app.main\"). When omitted, uses the default view.

DESCRIPTION:
    Validates the same runnable surface as `aivi run` and packages the
    current runtime binary plus an embedded source-free app bundle
    into a single runnable executable.

    This is the current runnable deployment path. It is distinct from
    `aivi compile`, which emits object code only.
"
        }
        "run" => {
            "\
aivi run — launch a live GTK app

USAGE:
    aivi run [<path>] [--path <path>] [--app <name>] [--view <name>]

ARGS:
    [<path>]            Path to an .aivi source file, workspace entry,
                        or serialized run artifact (.json). When omitted,
                        resolves via aivi.toml [[app]] or [run] entry,
                        then <workspace>/main.aivi.

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
    live widget tree. Serialized run artifacts skip the source/HIR
    pipeline and launch directly from bundled runtime payloads. The app
    runs until the window is closed.
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
