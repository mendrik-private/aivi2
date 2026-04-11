/// Assert that `path` exists on disk and is readable.
///
/// Returns an error message suitable for printing to stderr when the file is
/// absent, saving callers from getting a less-informative I/O error later in
/// the pipeline.
fn require_file_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!(
            "error: input file does not exist: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Validate that a module/path component does not contain path-traversal
/// sequences (`..`) or absolute path components.
///
/// User-supplied names are occasionally forwarded into file-system paths (e.g.
/// as part of bundle output directory names).  Rejecting traversal components
/// up front prevents an attacker from escaping the intended output root.
fn validate_module_name(name: &str) -> Result<(), String> {
    // Reject empty names.
    if name.is_empty() {
        return Err("error: module name must not be empty".to_owned());
    }
    // Reject absolute paths supplied as a module name.
    if Path::new(name).is_absolute() {
        return Err(format!(
            "error: module name `{name}` must not be an absolute path"
        ));
    }
    // Reject path traversal components anywhere in the name.
    for component in Path::new(name).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!(
                "error: module name `{name}` must not contain `..` path traversal components"
            ));
        }
    }
    Ok(())
}

/// Recursively collect all `.aivi` files under `dir`, sorted for deterministic output.
fn collect_aivi_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(current) = dirs.pop() {
        let entries = fs::read_dir(&current).map_err(|error| {
            format!("failed to read directory `{}`: {error}", current.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read directory entry in `{}`: {error}",
                    current.display()
                )
            })?;
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "aivi") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Check every `.aivi` file found recursively under `dir`.
fn check_directory(dir: &Path, timings: bool) -> Result<ExitCode, String> {
    let files = collect_aivi_files(dir)?;
    if files.is_empty() {
        println!("no .aivi files found in `{}`", dir.display());
        return Ok(ExitCode::SUCCESS);
    }
    let mut any_failed = false;
    for path in &files {
        match check_file(path, timings)? {
            ExitCode::SUCCESS => {}
            _ => any_failed = true,
        }
    }
    if any_failed {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Check every `[[app]]` entry defined in `aivi.toml`.
fn check_all_apps(
    apps: &[aivi_query::AppConfig],
    workspace_root: &Path,
    timings: bool,
) -> Result<ExitCode, String> {
    let mut any_failed = false;
    for app in apps {
        let entry_path = workspace_root.join(&app.entry);
        match check_file(&entry_path, timings)? {
            ExitCode::SUCCESS => {}
            _ => any_failed = true,
        }
    }
    if any_failed {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn canonicalize_check_path(cwd: &Path, path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .or_else(|_| fs::canonicalize(cwd.join(path)))
        .unwrap_or_else(|_| cwd.join(path))
}

fn include_project_workspace_file(
    workspace_root: &Path,
    bundled_stdlib_root: Option<&Path>,
    file_path: &Path,
) -> bool {
    if !file_path.starts_with(workspace_root) {
        return false;
    }

    if let Some(stdlib_root) = bundled_stdlib_root
        && workspace_root != stdlib_root
        && file_path.starts_with(stdlib_root)
    {
        return false;
    }

    true
}

fn check_file(path: &Path, timings: bool) -> Result<ExitCode, String> {
    let total_start = Instant::now();
    require_file_exists(path)?;

    let t0 = Instant::now();
    let snapshot = WorkspaceHirSnapshot::load(path)?;
    let load_duration = t0.elapsed();

    let t0 = Instant::now();
    let syntax_failed = workspace_syntax_failed(&snapshot, |sources, diagnostics| {
        print_diagnostics(sources, diagnostics.iter())
    });
    let syntax_duration = t0.elapsed();
    if syntax_failed {
        return Ok(ExitCode::FAILURE);
    }

    let t0 = Instant::now();
    let (lowering_failed, validation_failed) = workspace_hir_failed(
        &snapshot,
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
    );
    let hir_duration = t0.elapsed();
    if lowering_failed || validation_failed {
        return Ok(ExitCode::FAILURE);
    }

    // After HIR passes, collect LSP-level unused-symbol warnings for each file.
    let t0 = Instant::now();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry_path = canonicalize_check_path(&cwd, path);
    let workspace_root_raw = discover_workspace_root(&entry_path);
    let workspace_root = fs::canonicalize(&workspace_root_raw).unwrap_or(workspace_root_raw);
    let bundled_stdlib_root = discover_bundled_stdlib_root().ok();
    let mut unused_count = 0usize;
    for file in &snapshot.files {
        let file_path = canonicalize_check_path(&cwd, &file.path(&snapshot.frontend.db));
        if !include_project_workspace_file(
            &workspace_root,
            bundled_stdlib_root.as_deref(),
            &file_path,
        ) {
            continue;
        }
        let hir = query_hir_module(&snapshot.frontend.db, *file);
        let has_errors = hir
            .diagnostics()
            .iter()
            .any(|d| d.severity == Severity::Error);
        if !has_errors {
            let warnings = aivi_lsp::collect_unused_native_diagnostics(hir.module(), hir.source());
            unused_count += warnings.len();
            print_diagnostics(&snapshot.sources, warnings.iter());
        }
    }
    let unused_duration = t0.elapsed();

    let parsed = snapshot.entry_parsed();
    println!(
        "syntax + HIR passed: {} ({} surface item{}, {} workspace file{}{})",
        path.display(),
        parsed.cst().items.len(),
        plural_suffix(parsed.cst().items.len()),
        snapshot.files.len(),
        plural_suffix(snapshot.files.len()),
        if unused_count > 0 {
            format!(
                ", {} unused-symbol warning{}",
                unused_count,
                plural_suffix(unused_count)
            )
        } else {
            String::new()
        }
    );

    if timings {
        let total = total_start.elapsed();
        eprintln!("timings for `aivi check` ({}):", path.display());
        eprintln!("  load + parse:  {:>8.2?}", load_duration);
        eprintln!("  syntax check:  {:>8.2?}", syntax_duration);
        eprintln!("  HIR lowering:  {:>8.2?}", hir_duration);
        eprintln!("  unused check:  {:>8.2?}", unused_duration);
        eprintln!("  total:         {:>8.2?}", total);
    }

    Ok(ExitCode::SUCCESS)
}

fn run_markup_file_with_launch_config(
    path: &Path,
    requested_view: Option<&str>,
    launch_config: run_session::RunLaunchConfig,
    timings: bool,
) -> Result<ExitCode, String> {
    let total_start = Instant::now();
    require_file_exists(path)?;
    if let Some(view) = requested_view {
        validate_module_name(view)?;
    }

    let t0 = Instant::now();
    let snapshot = WorkspaceHirSnapshot::load(path)?;
    let load_duration = t0.elapsed();
    if timings {
        print_run_prelaunch_stage_progress("load + parse", load_duration, total_start.elapsed());
    }

    let t0 = Instant::now();
    let syntax_failed = workspace_syntax_failed(&snapshot, |sources, diagnostics| {
        print_diagnostics(sources, diagnostics.iter())
    });
    let syntax_duration = t0.elapsed();
    if timings {
        print_run_prelaunch_stage_progress("syntax check", syntax_duration, total_start.elapsed());
    }
    if syntax_failed {
        return Ok(ExitCode::FAILURE);
    }

    let t0 = Instant::now();
    let (hir_lowering_failed, hir_validation_failed) = workspace_hir_failed(
        &snapshot,
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
        |sources, diagnostics| print_diagnostics(sources, diagnostics.iter()),
    );
    let hir_duration = t0.elapsed();
    if timings {
        print_run_prelaunch_stage_progress("HIR lowering", hir_duration, total_start.elapsed());
    }
    if hir_lowering_failed || hir_validation_failed {
        return Ok(ExitCode::FAILURE);
    }

    let lowered = snapshot.entry_hir();
    let t0 = Instant::now();
    let workspace_hir_arcs = collect_workspace_hirs_sorted(&snapshot);
    let workspace_collection_duration = t0.elapsed();
    if timings {
        print_run_prelaunch_stage_progress(
            "workspace collect",
            workspace_collection_duration,
            total_start.elapsed(),
        );
    }
    let workspace_hirs: Vec<(&str, &HirModule)> = workspace_hir_arcs
        .iter()
        .map(|(name, arc)| (name.as_str(), arc.module()))
        .collect();
    let mut report_prelaunch_stage = |stage: &'static str, duration: Duration| {
        if timings {
            print_run_prelaunch_stage_progress(stage, duration, total_start.elapsed());
        }
    };
    let mut prepared = match prepare_run_artifact_with_metrics_and_progress(
        &snapshot.sources,
        lowered.module(),
        &workspace_hirs,
        requested_view,
        Some(snapshot.backend_query_context()),
        &mut report_prelaunch_stage,
    ) {
        Ok(prepared) => prepared,
        Err(message) => {
            eprintln!("{message}");
            return Ok(ExitCode::FAILURE);
        }
    };
    prepared.metrics.workspace_collection = workspace_collection_duration;
    let artifact_metrics = prepared.metrics;
    let query_cache = snapshot.frontend.db.cache_stats();

    run_session::launch_run_with_config(
        path,
        prepared.artifact,
        launch_config,
        move |stage, startup_metrics| {
            if timings {
                print_run_startup_stage_progress(stage, *startup_metrics);
            }
        },
        move |startup_metrics| {
            if timings {
                print_run_timing_report(
                    path,
                    load_duration,
                    syntax_duration,
                    hir_duration,
                    query_cache,
                    artifact_metrics,
                    *startup_metrics,
                    total_start.elapsed(),
                );
            }
        },
    )
}

#[derive(Debug)]
struct ExecuteArtifact {
    task_owner: HirItemId,
    runtime_assembly: Option<HirRuntimeAssembly>,
    core: Option<aivi_core::Module>,
    backend: Arc<BackendProgram>,
    backend_item: Option<BackendItemId>,
}

struct TestTaskOutcome {
    passed: bool,
    detail: Option<String>,
}

fn execute_file(path: &Path, program_args: &[String]) -> Result<ExitCode, String> {
    let context = current_execute_source_context(program_args)?;
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_file_with_context(path, context, &mut stdout, &mut stderr)
}

fn test_file(path: &Path) -> Result<ExitCode, String> {
    let context = current_execute_source_context(&[])?;
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    test_file_with_context(path, context, &mut stdout, &mut stderr)
}

fn test_file_with_context(
    path: &Path,
    context: SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, String> {
    require_file_exists(path)?;
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

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry_path = canonicalize_check_path(&cwd, path);
    let workspace_root_raw = discover_workspace_root(&entry_path);
    let workspace_root = fs::canonicalize(&workspace_root_raw).unwrap_or(workspace_root_raw);
    let bundled_stdlib_root = discover_bundled_stdlib_root().ok();

    let tests = discover_workspace_tests(&snapshot, &workspace_root, bundled_stdlib_root.as_deref());
    if tests.is_empty() {
        write_output_line(stderr, "no `@test` values found in the loaded workspace")?;
        return Ok(ExitCode::FAILURE);
    }

    let mut passed = 0usize;
    let mut failed = 0usize;

    for test in tests {
        let hir = query_hir_module(&snapshot.frontend.db, test.file);
        let module = hir.module();
        let artifact = match prepare_test_artifact_with_query_context(
            module,
            test.owner,
            Some(snapshot.backend_query_context()),
        ) {
            Ok(artifact) => artifact,
            Err(message) => {
                failed += 1;
                write_output_line(stderr, &format!("fail {}: {message}", test.location))?;
                continue;
            }
        };
        let value = match evaluate_task_owner_value(
            path,
            artifact,
            context.clone(),
            "`aivi test`",
            &format!("test `{}`", test.name),
        ) {
            Ok(value) => value,
            Err(message) => {
                failed += 1;
                write_output_line(stderr, &format!("fail {}: {message}", test.location))?;
                continue;
            }
        };
        match execute_test_task_value(value, &context, stdout, stderr) {
            Ok(TestTaskOutcome {
                passed: true,
                detail,
            }) => {
                passed += 1;
                match detail {
                    Some(detail) => {
                        write_output_line(stdout, &format!("ok   {}: {detail}", test.location))?
                    }
                    None => write_output_line(stdout, &format!("ok   {}", test.location))?,
                }
            }
            Ok(TestTaskOutcome {
                passed: false,
                detail,
            }) => {
                failed += 1;
                match detail {
                    Some(detail) => {
                        write_output_line(stderr, &format!("fail {}: {detail}", test.location))?
                    }
                    None => write_output_line(stderr, &format!("fail {}", test.location))?,
                }
            }
            Err(message) => {
                failed += 1;
                write_output_line(stderr, &format!("fail {}: {message}", test.location))?;
            }
        }
    }

    let total = passed + failed;
    if failed == 0 {
        write_output_line(
            stdout,
            &format!("test result: ok. {passed} passed; 0 failed; {total} total"),
        )?;
        Ok(ExitCode::SUCCESS)
    } else {
        write_output_line(
            stderr,
            &format!("test result: FAILED. {passed} passed; {failed} failed; {total} total"),
        )?;
        Ok(ExitCode::FAILURE)
    }
}

fn current_execute_source_context(
    program_args: &[String],
) -> Result<SourceProviderContext, String> {
    let cwd = env::current_dir().map_err(|error| {
        format!("failed to determine current directory for `aivi execute`: {error}")
    })?;
    let env_vars = env::vars_os()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(SourceProviderContext::new(
        program_args.to_vec(),
        cwd,
        env_vars,
    ))
}

fn item_is_test(module: &HirModule, item_id: HirItemId) -> bool {
    module.items().get(item_id).is_some_and(|item| {
        item.decorators().iter().any(|decorator_id| {
            module
                .decorators()
                .get(*decorator_id)
                .is_some_and(|decorator| matches!(decorator.payload, DecoratorPayload::Test(_)))
        })
    })
}

fn production_item_ids(module: &HirModule) -> IncludedItems {
    module
        .items()
        .iter()
        .filter_map(|(item_id, _)| (!item_is_test(module, item_id)).then_some(item_id))
        .collect()
}

#[derive(Clone)]
struct DiscoveredWorkspaceTest {
    file: QuerySourceFile,
    owner: HirItemId,
    name: Box<str>,
    location: String,
}

fn discover_workspace_tests(
    snapshot: &WorkspaceHirSnapshot,
    workspace_root: &Path,
    bundled_stdlib_root: Option<&Path>,
) -> Vec<DiscoveredWorkspaceTest> {
    let mut tests = Vec::new();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for file in &snapshot.files {
        let file_path = canonicalize_check_path(&cwd, &file.path(&snapshot.frontend.db));
        if !include_project_workspace_file(workspace_root, bundled_stdlib_root, &file_path) {
            continue;
        }
        let hir = query_hir_module(&snapshot.frontend.db, *file);
        let module = hir.module();
        for (item_id, item) in module.items().iter() {
            let Item::Value(value) = item else {
                continue;
            };
            if !item_is_test(module, item_id) {
                continue;
            }
            tests.push(DiscoveredWorkspaceTest {
                file: *file,
                owner: item_id,
                name: value.name.text().into(),
                location: format!(
                    "{}::{}",
                    source_location(&snapshot.sources, value.header.span),
                    value.name.text()
                ),
            });
        }
    }
    tests.sort_by(|left, right| {
        left.location
            .cmp(&right.location)
            .then_with(|| left.name.cmp(&right.name))
    });
    tests
}

fn execute_file_with_context(
    path: &Path,
    context: SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, String> {
    require_file_exists(path)?;
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
    let artifact = match prepare_execute_artifact(lowered.module()) {
        Ok(artifact) => artifact,
        Err(message) => {
            write_output_line(stderr, &message)?;
            return Ok(ExitCode::FAILURE);
        }
    };
    if let Err(message) = launch_execute(path, artifact, context, stdout, stderr) {
        write_output_line(stderr, &message)?;
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn prepare_execute_artifact(module: &HirModule) -> Result<ExecuteArtifact, String> {
    let main = select_execute_main(module)?;
    let main_owner = find_value_owner(module, main).ok_or_else(|| {
        format!(
            "failed to recover owning item for execute entrypoint `{}`",
            main.name.text()
        )
    })?;
    let included_items = production_item_ids(module);
    let lowered =
        lower_runtime_backend_stack_with_items(module, &included_items, "`aivi execute`")?;
    let runtime_assembly =
        assemble_hir_runtime_with_items(module, &included_items).map_err(|errors| {
            let mut rendered =
                String::from("failed to assemble runtime plans for `aivi execute`:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    if runtime_assembly.task_by_owner(main_owner).is_none() {
        return Err(
            "`aivi execute` requires `value main` to be annotated as `Task ...`".to_owned(),
        );
    }
    Ok(ExecuteArtifact {
        task_owner: main_owner,
        runtime_assembly: Some(runtime_assembly),
        core: Some(lowered.core),
        backend: lowered.backend,
        backend_item: None,
    })
}

fn prepare_test_artifact_with_query_context(
    module: &HirModule,
    test_owner: HirItemId,
    query_context: Option<BackendQueryContext<'_>>,
) -> Result<ExecuteArtifact, String> {
    let fragment = test_runtime_fragment(module, test_owner)?;
    let included_items = runtime_fragment_included_items(module, &fragment);
    if test_can_use_backend_only_path(module, test_owner, &included_items)
        && let Ok(artifact) = prepare_backend_only_test_artifact(module, &fragment, query_context) {
            return Ok(artifact);
        }
    let lowered = lower_runtime_backend_stack_with_items(module, &included_items, "`aivi test`")?;
    let runtime_assembly =
        assemble_hir_runtime_with_items(module, &included_items).map_err(|errors| {
            let mut rendered = String::from("failed to assemble runtime plans for `aivi test`:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    if runtime_assembly.task_by_owner(test_owner).is_none() {
        return Err(
            "`aivi test` requires every `@test` value to be annotated as `Task ...`".to_owned(),
        );
    }
    Ok(ExecuteArtifact {
        task_owner: test_owner,
        runtime_assembly: Some(runtime_assembly),
        core: Some(lowered.core),
        backend: lowered.backend,
        backend_item: None,
    })
}

fn prepare_backend_only_test_artifact(
    module: &HirModule,
    fragment: &RuntimeFragmentSpec,
    query_context: Option<BackendQueryContext<'_>>,
) -> Result<ExecuteArtifact, String> {
    let lowered = compile_runtime_fragment_backend_unit(
        module,
        fragment,
        query_context,
        "failed to compile backend-only `aivi test` fragment",
    )?;
    let backend_item = lowered
        .backend
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == fragment.name.as_ref())
        .map(|(item_id, _)| item_id)
        .ok_or_else(|| {
            format!(
                "failed to find compiled backend item for `{}` in `aivi test` fragment",
                fragment.name
            )
        })?;
    Ok(ExecuteArtifact {
        task_owner: fragment.owner,
        runtime_assembly: None,
        core: None,
        backend: lowered.backend,
        backend_item: Some(backend_item),
    })
}

fn test_can_use_backend_only_path(
    module: &HirModule,
    test_owner: HirItemId,
    included_items: &IncludedItems,
) -> bool {
    if item_has_mock(module, test_owner) {
        return false;
    }
    included_items
        .iter()
        .all(|item_id| !matches!(module.items().get(*item_id), Some(Item::Signal(_))))
}

fn item_has_mock(module: &HirModule, owner: HirItemId) -> bool {
    let Some(item) = module.items().get(owner) else {
        return false;
    };
    item.decorators().iter().any(|decorator_id| {
        matches!(
            module
                .decorators()
                .get(*decorator_id)
                .map(|decorator| &decorator.payload),
            Some(DecoratorPayload::Mock(_))
        )
    })
}

fn test_runtime_fragment(
    module: &HirModule,
    test_owner: HirItemId,
) -> Result<RuntimeFragmentSpec, String> {
    let report = aivi_hir::elaborate_general_expressions(module)
        .into_items()
        .into_iter()
        .find(|item| item.owner == test_owner)
        .ok_or_else(|| {
            format!("failed to recover general-expression elaboration for test owner {test_owner}")
        })?;
    let body = match report.outcome {
        GeneralExprOutcome::Lowered(body) => body,
        GeneralExprOutcome::Blocked(blocked) => {
            return Err(format!(
                "failed to elaborate `@test` body for owner {test_owner}: {blocked}"
            ));
        }
    };
    Ok(RuntimeFragmentSpec {
        name: format!("__test_fragment_{}", test_owner.as_raw()).into_boxed_str(),
        owner: test_owner,
        body_expr: report.body_expr,
        parameters: report.parameters,
        body,
    })
}

fn select_execute_main(module: &HirModule) -> Result<&ValueItem, String> {
    let mut found_value = None;
    let mut found_non_value_kind = None;
    for (item_id, item) in module.items().iter() {
        if item_is_test(module, item_id) {
            continue;
        }
        match item {
            Item::Value(value) if value.name.text() == "main" => {
                found_value = Some(value);
            }
            Item::Function(item) if item.name.text() == "main" => {
                found_non_value_kind = Some("function")
            }
            Item::Signal(item) if item.name.text() == "main" => {
                found_non_value_kind = Some("signal")
            }
            Item::Type(item) if item.name.text() == "main" => found_non_value_kind = Some("type"),
            Item::Class(item) if item.name.text() == "main" => found_non_value_kind = Some("class"),
            Item::Domain(item) if item.name.text() == "main" => {
                found_non_value_kind = Some("domain")
            }
            _ => {}
        }
    }
    if let Some(value) = found_value {
        return Ok(value);
    }
    if let Some(kind) = found_non_value_kind {
        return Err(format!(
            "`aivi execute` requires a top-level `value main : Task ...`; found top-level `{kind} main` instead"
        ));
    }
    Err("no top-level `value main` found; define `value main : Task ... = ...`".to_owned())
}

fn launch_execute(
    path: &Path,
    artifact: ExecuteArtifact,
    context: SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), String> {
    let value =
        evaluate_task_owner_value(path, artifact, context.clone(), "`aivi execute`", "`main`")?;
    execute_main_task_value(value, &context, stdout, stderr)
}

fn evaluate_task_owner_value(
    path: &Path,
    artifact: ExecuteArtifact,
    context: SourceProviderContext,
    command_name: &str,
    entry_name: &str,
) -> Result<RuntimeValue, String> {
    let ExecuteArtifact {
        task_owner,
        runtime_assembly,
        core,
        backend,
        backend_item,
    } = artifact;
    if let Some(backend_item) = backend_item {
        let executable = BackendExecutableProgram::interpreted(backend.as_ref())
            .with_execution_options(aivi_backend::BackendExecutionOptions {
                prefer_interpreter: cfg!(test),
                ..Default::default()
            });
        let mut evaluator = executable.create_engine();
        let globals = BTreeMap::new();
        return evaluator
            .evaluate_item(backend_item, &globals)
            .map_err(|error| {
                format!(
                    "failed to evaluate {entry_name} for {command_name} in {}: {error}",
                    path.display()
                )
            });
    }
    let Some(runtime_assembly) = runtime_assembly else {
        return Err(format!(
            "failed to prepare runtime assembly for {command_name} in {}",
            path.display()
        ));
    };
    let Some(core) = core else {
        return Err(format!(
            "failed to prepare typed core for {command_name} in {}",
            path.display()
        ));
    };
    let mut linked = link_backend_runtime(runtime_assembly, &core, backend).map_err(|errors| {
        let mut rendered = format!(
            "failed to link backend runtime for {command_name} in {}:\n",
            path.display()
        );
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let mut providers = SourceProviderManager::with_context(context);
    settle_execute_sources(&mut linked, &mut providers)?;
    linked
        .evaluate_task_value_by_owner(task_owner)
        .map(|value| value.into_runtime())
        .map_err(|error| {
            format!(
                "failed to evaluate {entry_name} for {command_name} in {}: {error}",
                path.display()
            )
        })
}

fn settle_execute_sources(
    linked: &mut BackendLinkedRuntime,
    providers: &mut SourceProviderManager,
) -> Result<(), String> {
    // Phase 1: process all source lifecycle actions (activations, reconfigurations)
    // until the graph is stable.  This handles immediate sources completely and
    // spawns worker threads for asynchronous sources.
    loop {
        let outcome = linked.tick_with_source_lifecycle().map_err(|error| {
            format!("failed to tick linked runtime for `aivi execute`: {error}")
        })?;
        let had_source_actions = !outcome.source_actions().is_empty();
        providers
            .apply_actions(outcome.source_actions())
            .map_err(|error| {
                format!("failed to apply source lifecycle actions for `aivi execute`: {error}")
            })?;
        if !had_source_actions && linked.queued_message_count() == 0 {
            break;
        }
    }

    // Phase 2: if any worker-thread sources were spawned (HTTP, FS, Timer, etc.),
    // wait for them to publish at least once before returning.  Immediate sources
    // (ProcessArgs, ProcessCwd, EnvGet, …) never create thread handles so this
    // phase is skipped entirely for programs that only use immediate sources.
    if !providers.has_unfinished_worker_threads() {
        return Ok(());
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));

        // Tick to drain publications that arrived from worker threads and
        // keep settling until the graph is stable again.
        loop {
            let outcome = linked.tick_with_source_lifecycle().map_err(|error| {
                format!("failed to tick linked runtime for `aivi execute`: {error}")
            })?;
            let had_source_actions = !outcome.source_actions().is_empty();
            providers
                .apply_actions(outcome.source_actions())
                .map_err(|error| {
                    format!("failed to apply source lifecycle actions for `aivi execute`: {error}")
                })?;
            if !had_source_actions && linked.queued_message_count() == 0 {
                break;
            }
        }

        if !providers.has_unfinished_worker_threads() {
            return Ok(());
        }
    }

    Ok(())
}

fn execute_main_task_value(
    value: RuntimeValue,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), String> {
    if !matches!(&value, RuntimeValue::Task(_) | RuntimeValue::DbTask(_)) {
        return Err(format!(
            "`aivi execute` expected `main` to evaluate to a task plan, found `{value}`"
        ));
    }
    let result = execute_runtime_value_with_context(value, context, stdout, stderr)
        .map_err(|error| error.to_string())?;
    if result != RuntimeValue::Unit {
        write_output_line(stdout, &result.to_string())?;
    }
    Ok(())
}

fn execute_test_task_value(
    value: RuntimeValue,
    context: &SourceProviderContext,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<TestTaskOutcome, String> {
    if !matches!(&value, RuntimeValue::Task(_) | RuntimeValue::DbTask(_)) {
        return Err(format!(
            "`aivi test` expected each `@test` value to evaluate to a task plan, found `{value}`"
        ));
    }
    let result = execute_runtime_value_with_context(value, context, stdout, stderr)
        .map_err(|error| error.to_string())?;
    Ok(match result {
        RuntimeValue::Unit => TestTaskOutcome {
            passed: true,
            detail: None,
        },
        RuntimeValue::Bool(true) => TestTaskOutcome {
            passed: true,
            detail: None,
        },
        RuntimeValue::Bool(false) => TestTaskOutcome {
            passed: false,
            detail: Some("returned false".to_owned()),
        },
        RuntimeValue::ResultOk(value) => {
            let detail = (*value != RuntimeValue::Unit).then(|| value.to_string());
            TestTaskOutcome {
                passed: true,
                detail,
            }
        }
        RuntimeValue::ResultErr(error) => TestTaskOutcome {
            passed: false,
            detail: Some(error.to_string()),
        },
        RuntimeValue::ValidationValid(value) => {
            let detail = (*value != RuntimeValue::Unit).then(|| value.to_string());
            TestTaskOutcome {
                passed: true,
                detail,
            }
        }
        RuntimeValue::ValidationInvalid(error) => TestTaskOutcome {
            passed: false,
            detail: Some(error.to_string()),
        },
        other => {
            return Err(format!(
                "`aivi test` only supports task results of `Unit`, `Bool`, `Result`, or `Validation`; found `{other}`"
            ));
        }
    })
}

fn write_output_line(target: &mut impl Write, text: &str) -> Result<(), String> {
    writeln!(target, "{text}").map_err(|error| format!("failed to write CLI output: {error}"))
}
