#![forbid(unsafe_code)]

mod manual_snippets;
mod mcp;
mod run_session;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
    sync::{Arc, mpsc as sync_mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use aivi_backend::{
    DetachedRuntimeValue, ItemId as BackendItemId, KernelEvaluator, Program as BackendProgram,
    RuntimeFloat, RuntimeRecordField, RuntimeValue, compile_program_cached,
    lower_module as lower_backend_module, validate_program,
};
use aivi_base::{Diagnostic, FileId, Severity, SourceDatabase, SourceSpan};
use aivi_core::{
    IncludedItems, RuntimeFragmentSpec, lower_runtime_fragment,
    lower_runtime_module_with_items, lower_runtime_module_with_workspace,
    runtime_fragment_included_items,
    validate_module as validate_core_module,
};
use aivi_gtk::{
    GtkBridgeGraph, GtkBridgeNodeKind, GtkBridgeNodeRef, GtkChildGroup, GtkCollectionKey,
    GtkConcreteEventPayload, GtkConcreteHost, GtkExecutionPath, GtkHostValue, GtkNodeInstance,
    GtkRuntimeExecutor, RepeatedChildPolicy, RuntimePropertyBinding, RuntimeShowMountPolicy,
    SetterSource, lookup_widget_event, lookup_widget_schema, lower_markup_expr,
    lower_widget_bridge,
};
use aivi_hir::{
    BuiltinTerm, BuiltinType, DecoratorPayload, ExprId as HirExprId, ExprKind, GateRecordField,
    GateType, GeneralExprOutcome, GeneralExprParameter, ImportBindingMetadata, ImportId,
    ImportValueType, Item, ItemId as HirItemId, MarkupRuntimeExprSites, Module as HirModule,
    PatternId as HirPatternId, PatternKind, TermResolution, ValidationMode, ValueItem, collect_markup_runtime_expr_sites, elaborate_runtime_expr_with_env,
    signal_payload_type,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::{
    HirModuleResult, RootDatabase, SourceFile as QuerySourceFile, hir_module as query_hir_module,
    parsed_file as query_parsed_file, resolve_v1_entrypoint,
};
use aivi_runtime::{
    BackendLinkedRuntime, GlibLinkedRuntimeDriver, GlibLinkedRuntimeFailure, HirRuntimeAssembly,
    InputHandle as RuntimeInputHandle, Publication, SourceProviderContext, SourceProviderManager,
    assemble_hir_runtime_with_items, execute_runtime_value_with_context, link_backend_runtime,
    render_runtime_error,
};
use aivi_syntax::{Formatter, lex_module, parse_module};
use gtk::{glib, prelude::*};

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
    let resolved = resolve_v1_entrypoint(&cwd, explicit_path, app_name)
        .map_err(|error| format!("failed to resolve entrypoint for `aivi {command_name}`: {error}"))?;
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
        let workspace_root = aivi_query::discover_workspace_root_from_directory(&cwd);
        if let Ok(manifest) = aivi_query::parse_manifest(&workspace_root) {
            if manifest.apps.len() > 1 {
                let exe = env::current_exe().map_err(|e| {
                    format!("failed to locate aivi executable: {e}")
                })?;
                let mut children: Vec<std::process::Child> = manifest
                    .apps
                    .iter()
                    .map(|app| {
                        let mut cmd = std::process::Command::new(&exe);
                        cmd.arg("run").arg("--app").arg(&app.name);
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

    let resolved = resolve_run_entrypoint(&cwd, requested_path.as_deref(), requested_app.as_deref())?;
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

struct WorkspaceFrontend {
    db: RootDatabase,
    entry: QuerySourceFile,
}

impl WorkspaceFrontend {
    fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let db = RootDatabase::new();
        let entry = QuerySourceFile::new(&db, path.to_path_buf(), text);
        Ok(Self { db, entry })
    }

    fn warm(&self) {
        let _ = query_hir_module(&self.db, self.entry);
    }

    fn files(&self) -> Vec<QuerySourceFile> {
        self.db.files()
    }

    fn sources(&self) -> SourceDatabase {
        self.db.source_database()
    }
}

struct WorkspaceHirSnapshot {
    frontend: WorkspaceFrontend,
    sources: SourceDatabase,
    files: Vec<QuerySourceFile>,
}

impl WorkspaceHirSnapshot {
    fn load(path: &Path) -> Result<Self, String> {
        let frontend = WorkspaceFrontend::load(path)?;
        frontend.warm();
        let sources = frontend.sources();
        let files = frontend.files();
        Ok(Self {
            frontend,
            sources,
            files,
        })
    }

    fn entry_parsed(&self) -> Arc<aivi_query::ParsedFileResult> {
        query_parsed_file(&self.frontend.db, self.frontend.entry)
    }

    fn entry_hir(&self) -> Arc<aivi_query::HirModuleResult> {
        query_hir_module(&self.frontend.db, self.frontend.entry)
    }
}

/// Compute a module name from a file path relative to the workspace root.
/// Returns e.g. "libs.types" for "<root>/libs/types.aivi".
fn module_name_from_path(workspace_root: &Path, file_path: &Path) -> Option<String> {
    let relative = file_path.strip_prefix(workspace_root).ok()?;
    if relative.extension()?.to_str()? != "aivi" {
        return None;
    }
    let mut segments = relative
        .iter()
        .map(|seg| seg.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    let file_name = segments.pop()?;
    let stem = Path::new(&file_name).file_stem()?.to_str()?.to_owned();
    segments.push(stem);
    Some(segments.join("."))
}

/// Collect all non-entry workspace HIR modules in topological dependency order
/// (dependencies before dependents) so that workspace function bodies are
/// available when later modules reference them.
fn collect_workspace_hirs_sorted(
    snapshot: &WorkspaceHirSnapshot,
) -> Vec<(String, Arc<HirModuleResult>)> {
    let entry_path_raw = snapshot.frontend.entry.path(&snapshot.frontend.db);
    // Canonicalize to absolute paths so strip_prefix works correctly when aivi is
    // invoked with a relative path (e.g. `aivi run apps/ui/main.aivi`).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry_path = std::fs::canonicalize(&entry_path_raw)
        .or_else(|_| std::fs::canonicalize(cwd.join(&entry_path_raw)))
        .unwrap_or_else(|_| cwd.join(&entry_path_raw));
    let workspace_root_raw = discover_workspace_root(&entry_path);
    let workspace_root = std::fs::canonicalize(&workspace_root_raw)
        .unwrap_or(workspace_root_raw);

    // Collect (module_name, file, hir) for all non-entry, non-stdlib workspace files.
    let mut ws_modules: Vec<(String, QuerySourceFile, Arc<HirModuleResult>)> = Vec::new();
    for &file in &snapshot.files {
        let path_raw = file.path(&snapshot.frontend.db);
        let path = std::fs::canonicalize(&path_raw)
            .or_else(|_| std::fs::canonicalize(cwd.join(&path_raw)))
            .unwrap_or_else(|_| cwd.join(&path_raw));
        if path == entry_path {
            continue;
        }
        let Some(module_name) = module_name_from_path(&workspace_root, &path) else {
            continue;
        };
        // Skip bundled stdlib modules (e.g. aivi.list, aivi.option).
        if module_name.starts_with("aivi.") {
            continue;
        }
        let hir = query_hir_module(&snapshot.frontend.db, file);
        ws_modules.push((module_name, file, hir));
    }

    // Build dependency graph: module_name → set of workspace module names it depends on.
    let ws_names: std::collections::HashSet<&str> =
        ws_modules.iter().map(|(n, _, _)| n.as_str()).collect();
    let deps: Vec<(String, Vec<String>)> = ws_modules
        .iter()
        .map(|(name, _, hir)| {
            let module_hir = hir.module();
            let mut module_deps = Vec::new();
            for (_, item) in module_hir.items().iter() {
                let aivi_hir::Item::Use(use_item) = item else {
                    continue;
                };
                let dep_name = use_item.module.to_string();
                if ws_names.contains(dep_name.as_str()) && dep_name != *name {
                    module_deps.push(dep_name);
                }
            }
            module_deps.sort();
            module_deps.dedup();
            (name.clone(), module_deps)
        })
        .collect();

    // Topological sort (Kahn's algorithm):
    // in_degree[A] = number of A's unprocessed dependencies.
    let mut in_degree: HashMap<String, usize> =
        deps.iter().map(|(n, d)| (n.clone(), d.len())).collect();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (name, module_deps) in &deps {
        for dep in module_deps {
            adjacency.entry(dep.clone()).or_default().push(name.clone());
        }
    }
    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    let mut sorted_names: Vec<String> = Vec::new();
    while let Some(name) = queue.pop_front() {
        sorted_names.push(name.clone());
        for dependent in adjacency.get(&name).cloned().unwrap_or_default() {
            let count = in_degree.entry(dependent.clone()).or_insert(0);
            *count = count.saturating_sub(1);
            if *count == 0 {
                queue.push_back(dependent);
            }
        }
    }

    // Build final result in topological order.
    let module_map: HashMap<String, Arc<HirModuleResult>> = ws_modules
        .into_iter()
        .map(|(name, _, hir)| (name, hir))
        .collect();
    sorted_names
        .into_iter()
        .filter_map(|name| {
            let hir = module_map.get(&name)?.clone();
            Some((name, hir))
        })
        .collect()
}

fn workspace_syntax_failed(
    snapshot: &WorkspaceHirSnapshot,
    mut print: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
) -> bool {
    let mut failed = false;
    for file in &snapshot.files {
        let parsed = query_parsed_file(&snapshot.frontend.db, *file);
        failed |= print(&snapshot.sources, parsed.diagnostics());
    }
    failed
}

fn workspace_hir_failed(
    snapshot: &WorkspaceHirSnapshot,
    mut print_hir: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
    mut print_validation: impl FnMut(&SourceDatabase, &[Diagnostic]) -> bool,
) -> (bool, bool) {
    let mut lowering_failed = false;
    let mut validation_failed = false;
    for file in &snapshot.files {
        let hir = query_hir_module(&snapshot.frontend.db, *file);
        let file_lowering_failed = print_hir(&snapshot.sources, hir.hir_diagnostics());
        lowering_failed |= file_lowering_failed;
        let validation_mode = if file_lowering_failed {
            ValidationMode::Structural
        } else {
            ValidationMode::RequireResolvedNames
        };
        let validation = hir.module().validate(validation_mode);
        validation_failed |= print_validation(&snapshot.sources, validation.diagnostics());
    }
    (lowering_failed, validation_failed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHostValue(DetachedRuntimeValue);

impl GtkHostValue for RunHostValue {
    fn unit() -> Self {
        Self(DetachedRuntimeValue::unit())
    }

    fn from_bool(v: bool) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(v),
        ))
    }

    fn from_text(v: &str) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Text(v.to_owned().into()),
        ))
    }

    fn from_f64(v: f64) -> Self {
        match RuntimeFloat::new(v) {
            Some(rf) => Self(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Float(rf),
            )),
            None => Self::unit(),
        }
    }

    fn from_i64(v: i64) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Int(v),
        ))
    }

    fn as_bool(&self) -> Option<bool> {
        strip_signal_runtime_value(self.0.to_runtime()).as_bool()
    }

    fn as_i64(&self) -> Option<i64> {
        strip_signal_runtime_value(self.0.to_runtime()).as_i64()
    }

    fn as_f64(&self) -> Option<f64> {
        strip_signal_runtime_value(self.0.to_runtime()).as_float()
    }

    fn as_text(&self) -> Option<&str> {
        match strip_signal_runtime_ref(self.0.as_runtime()) {
            RuntimeValue::Text(value) => Some(value.as_ref()),
            RuntimeValue::Sum(sum) if sum.fields.is_empty() => Some(sum.variant_name.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct RunArtifact {
    view_name: Box<str>,
    module: HirModule,
    bridge: GtkBridgeGraph,
    hydration_inputs: BTreeMap<RuntimeInputHandle, CompiledRunInput>,
    required_signal_globals: BTreeMap<BackendItemId, Box<str>>,
    runtime_assembly: HirRuntimeAssembly,
    core: aivi_core::Module,
    backend: Arc<BackendProgram>,
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
    /// Default values to publish into stub Input signal handles for cross-module
    /// workspace imports before the first hydration cycle. Keyed by the input handle
    /// that was synthesised in the runtime assembly for each import signal.
    stub_signal_defaults: Vec<(RuntimeInputHandle, DetachedRuntimeValue)>,
}

#[derive(Clone, Debug)]
struct RunValidationBlocker {
    span: SourceSpan,
    message: String,
}

#[derive(Clone, Debug)]
struct CompiledRunFragment {
    expr: HirExprId,
    parameters: Vec<GeneralExprParameter>,
    program: BackendProgram,
    item: BackendItemId,
    required_signal_globals: Vec<CompiledRunSignalGlobal>,
}

#[derive(Clone, Debug)]
struct CompiledRunSignalGlobal {
    fragment_item: BackendItemId,
    runtime_item: BackendItemId,
    name: Box<str>,
}

#[derive(Clone, Debug)]
enum CompiledRunInput {
    Expr(CompiledRunFragment),
    Text(CompiledRunText),
}

#[derive(Clone, Debug)]
struct CompiledRunText {
    segments: Box<[CompiledRunTextSegment]>,
}

#[derive(Clone, Debug)]
enum CompiledRunTextSegment {
    Text(Box<str>),
    Interpolation(CompiledRunFragment),
}

#[derive(Clone, Debug)]
enum RunInputSpec {
    Expr(HirExprId),
    Text(aivi_hir::TextLiteral),
}

#[derive(Clone, Debug)]
struct RunHydrationStaticState {
    view_name: Box<str>,
    module: HirModule,
    bridge: GtkBridgeGraph,
    inputs: BTreeMap<RuntimeInputHandle, CompiledRunInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHydrationPlan {
    root: HydratedRunNode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HydratedRunNode {
    Widget {
        instance: GtkNodeInstance,
        properties: Box<[HydratedRunProperty]>,
        event_inputs: Box<[HydratedRunProperty]>,
        children: Box<[HydratedRunNode]>,
    },
    Show {
        instance: GtkNodeInstance,
        when_input: RuntimeInputHandle,
        when: bool,
        keep_mounted_input: Option<RuntimeInputHandle>,
        keep_mounted: bool,
        children: Box<[HydratedRunNode]>,
    },
    Each {
        instance: GtkNodeInstance,
        collection_input: RuntimeInputHandle,
        kind: HydratedRunEachKind,
        empty_branch: Option<Box<HydratedRunNode>>,
    },
    Match {
        instance: GtkNodeInstance,
        scrutinee_input: RuntimeInputHandle,
        active_case: usize,
        branch: Box<HydratedRunNode>,
    },
    Case {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
    Fragment {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
    With {
        instance: GtkNodeInstance,
        value_input: RuntimeInputHandle,
        children: Box<[HydratedRunNode]>,
    },
    Empty {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HydratedRunProperty {
    input: RuntimeInputHandle,
    value: DetachedRuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HydratedRunEachKind {
    Positional {
        item_count: usize,
        items: Box<[HydratedRunEachItem]>,
    },
    Keyed {
        key_input: RuntimeInputHandle,
        keys: Box<[GtkCollectionKey]>,
        items: Box<[HydratedRunEachItem]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HydratedRunEachItem {
    children: Box<[HydratedRunNode]>,
}

#[derive(Clone, Debug)]
enum ResolvedRunEventPayload {
    GtkPayload,
    ScopedInput,
}

#[derive(Clone, Debug)]
struct ResolvedRunEventHandler {
    signal_item: aivi_hir::ItemId,
    signal_name: Box<str>,
    signal_input: RuntimeInputHandle,
    payload: ResolvedRunEventPayload,
}
/// RAII wrapper that deletes a temporary file on drop.
///
/// This ensures temporary files are cleaned up even when the program exits
/// early due to an error or a panic.
#[allow(dead_code)]
struct TempFile(PathBuf);

#[allow(dead_code)]
impl TempFile {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// RAII wrapper around [`tempfile::TempDir`] that places a staging directory
/// in the given parent so it lives on the same filesystem as the final output
/// path, enabling an atomic `fs::rename` on success.
///
/// On drop the directory and all its contents are removed automatically,
/// even when the process exits early due to an error or a panic.
struct StagingDir(tempfile::TempDir);

impl StagingDir {
    /// Create a new temporary staging directory inside `parent`.
    fn new_in(parent: &Path) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix(".aivi-bundle-staging-")
            .tempdir_in(parent)
            .map_err(|error| {
                format!(
                    "failed to create staging directory in {}: {error}",
                    parent.display()
                )
            })?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

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

    let parsed = snapshot.entry_parsed();
    println!(
        "syntax + HIR passed: {} ({} surface item{}, {} workspace file{})",
        path.display(),
        parsed.cst().items.len(),
        plural_suffix(parsed.cst().items.len()),
        snapshot.files.len(),
        plural_suffix(snapshot.files.len())
    );

    if timings {
        let total = total_start.elapsed();
        eprintln!("timings for `aivi check` ({}):", path.display());
        eprintln!("  load + parse:  {:>8.2?}", load_duration);
        eprintln!("  syntax check:  {:>8.2?}", syntax_duration);
        eprintln!("  HIR lowering:  {:>8.2?}", hir_duration);
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

    let t0 = Instant::now();
    let syntax_failed = workspace_syntax_failed(&snapshot, |sources, diagnostics| {
        print_diagnostics(sources, diagnostics.iter())
    });
    let syntax_duration = t0.elapsed();
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
    if hir_lowering_failed || hir_validation_failed {
        return Ok(ExitCode::FAILURE);
    }

    let t0 = Instant::now();
    let lowered = snapshot.entry_hir();
    let workspace_hir_arcs = collect_workspace_hirs_sorted(&snapshot);
    let workspace_hirs: Vec<(&str, &HirModule)> = workspace_hir_arcs
        .iter()
        .map(|(name, arc)| (name.as_str(), arc.module()))
        .collect();
    let artifact = match prepare_run_artifact(
        &snapshot.sources,
        lowered.module(),
        &workspace_hirs,
        requested_view,
    ) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("{message}");
            return Ok(ExitCode::FAILURE);
        }
    };
    let artifact_duration = t0.elapsed();

    if timings {
        let total = total_start.elapsed();
        eprintln!("timings for `aivi run` ({}):", path.display());
        eprintln!("  load + parse:       {:>8.2?}", load_duration);
        eprintln!("  syntax check:       {:>8.2?}", syntax_duration);
        eprintln!("  HIR lowering:       {:>8.2?}", hir_duration);
        eprintln!("  artifact prep:      {:>8.2?}", artifact_duration);
        eprintln!("  total (pre-launch): {:>8.2?}", total);
    }

    run_session::launch_run_with_config(path, artifact, launch_config)
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

    let tests = discover_workspace_tests(&snapshot);
    if tests.is_empty() {
        write_output_line(stderr, "no `@test` values found in the loaded workspace")?;
        return Ok(ExitCode::FAILURE);
    }

    let mut passed = 0usize;
    let mut failed = 0usize;

    for test in tests {
        let hir = query_hir_module(&snapshot.frontend.db, test.file);
        let module = hir.module();
        let artifact = match prepare_test_artifact(module, test.owner) {
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

fn discover_workspace_tests(snapshot: &WorkspaceHirSnapshot) -> Vec<DiscoveredWorkspaceTest> {
    let mut tests = Vec::new();
    for file in &snapshot.files {
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

fn prepare_test_artifact(
    module: &HirModule,
    test_owner: HirItemId,
) -> Result<ExecuteArtifact, String> {
    let fragment = test_runtime_fragment(module, test_owner)?;
    let included_items = runtime_fragment_included_items(module, &fragment);
    if test_can_use_backend_only_path(module, test_owner, &included_items) {
        if let Ok(artifact) = prepare_backend_only_test_artifact(module, &fragment) {
            return Ok(artifact);
        }
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
) -> Result<ExecuteArtifact, String> {
    let lowered = lower_runtime_fragment_backend_stack(module, fragment, "`aivi test`")?;
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

fn select_execute_main<'a>(module: &'a HirModule) -> Result<&'a ValueItem, String> {
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
        let mut evaluator = KernelEvaluator::new(&backend);
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
                    format!(
                        "failed to apply source lifecycle actions for `aivi execute`: {error}"
                    )
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

fn prepare_run_artifact(
    sources: &SourceDatabase,
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let included_items = production_item_ids(module);
    let view = select_run_view(module, requested_view)?;
    let view_owner = find_value_owner(module, view).ok_or_else(|| {
        format!(
            "failed to recover owning item for run view `{}`",
            view.name.text()
        )
    })?;
    let ExprKind::Markup(_) = &module.exprs()[view.body].kind else {
        return Err(format!(
            "run view `{}` is not markup; `aivi run` currently requires a top-level markup-valued `value`",
            view.name.text()
        ));
    };
    let plan = lower_markup_expr(module, view.body).map_err(|error| {
        format!(
            "failed to lower run view `{}` into GTK markup: {error}",
            view.name.text()
        )
    })?;
    let bridge = lower_widget_bridge(&plan).map_err(|error| {
        format!(
            "failed to lower run view `{}` into a GTK bridge graph: {error}",
            view.name.text()
        )
    })?;
    validate_run_plan(sources, &bridge)?;
    let lowered = if workspace_hirs.is_empty() {
        lower_runtime_backend_stack_with_items_fast(module, &included_items, "`aivi run`")?
    } else {
        lower_runtime_backend_stack_with_workspace(
            module,
            workspace_hirs,
            &included_items,
            "`aivi run`",
        )?
    };
    let runtime_assembly =
        assemble_hir_runtime_with_items(module, &included_items).map_err(|errors| {
            let mut rendered = String::from("failed to assemble runtime plans for `aivi run`:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    let runtime_backend_by_hir = backend_items_by_hir(&lowered.core, lowered.backend.as_ref());
    let sites = collect_markup_runtime_expr_sites(module, view.body).map_err(|error| {
        let span_info = match &error {
            aivi_hir::MarkupRuntimeExprSiteError::UnknownExprType { span, .. } => {
                format!(" (failing expr at {})", source_location(sources, *span))
            }
            _ => String::new(),
        };
        format!(
            "failed to collect runtime expression environments for run view at {}: {error}{span_info}",
            source_location(sources, module.exprs()[view.body].span)
        )
    })?;
    let hydration_inputs = compile_run_inputs(
        sources,
        module,
        view_owner,
        &sites,
        &bridge,
        lowered.backend.as_ref(),
        &runtime_backend_by_hir,
    )?;
    let required_signal_globals = collect_run_required_signal_globals(&hydration_inputs);
    let event_handlers =
        resolve_run_event_handlers(module, &sites, &bridge, &runtime_assembly, sources)?;
    let stub_signal_defaults = collect_stub_signal_defaults(module, &runtime_assembly);
    Ok(RunArtifact {
        view_name: view.name.text().into(),
        module: module.clone(),
        bridge,
        hydration_inputs,
        required_signal_globals,
        runtime_assembly,
        core: lowered.core,
        backend: lowered.backend,
        event_handlers,
        stub_signal_defaults,
    })
}

fn select_run_view<'a>(
    module: &'a HirModule,
    requested_view: Option<&str>,
) -> Result<&'a ValueItem, String> {
    let mut markup_values = Vec::new();
    let mut all_values = Vec::new();
    for (item_id, item) in module.items().iter() {
        if item_is_test(module, item_id) {
            continue;
        }
        let Item::Value(value) = item else {
            continue;
        };
        all_values.push(value);
        if matches!(module.exprs()[value.body].kind, ExprKind::Markup(_)) {
            markup_values.push(value);
        }
    }

    if let Some(requested_view) = requested_view {
        let Some(value) = all_values
            .into_iter()
            .find(|value| value.name.text() == requested_view)
        else {
            let available = markup_view_names(&markup_values);
            return Err(if available.is_empty() {
                format!(
                    "run view `{requested_view}` does not exist and this module exposes no markup-valued top-level `value`s"
                )
            } else {
                format!(
                    "run view `{requested_view}` does not exist; available markup views: {}",
                    available.join(", ")
                )
            });
        };
        return if matches!(module.exprs()[value.body].kind, ExprKind::Markup(_)) {
            Ok(value)
        } else {
            Err(format!(
                "run view `{requested_view}` exists but is not markup; `aivi run` currently requires a markup-valued top-level `value`"
            ))
        };
    }

    if let Some(view) = markup_values
        .iter()
        .copied()
        .find(|value| value.name.text() == "view")
    {
        return Ok(view);
    }

    match markup_values.as_slice() {
        [single] => Ok(*single),
        [] => Err("no markup view found; define `value view = <Window ...>` or pass `--view <name>` for another markup-valued top-level `value`".to_owned()),
        many => Err(format!(
            "multiple markup views are available ({}); rename one to `view` or pass `--view <name>`",
            markup_view_names(many).join(", ")
        )),
    }
}

fn markup_view_names(values: &[&ValueItem]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.name.text().to_owned())
        .collect()
}

fn find_value_owner(module: &HirModule, value: &ValueItem) -> Option<aivi_hir::ItemId> {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(candidate)
                if candidate.body == value.body && candidate.name.text() == value.name.text() =>
            {
                Some(item_id)
            }
            _ => None,
        })
}

fn validate_run_plan(sources: &SourceDatabase, bridge: &GtkBridgeGraph) -> Result<(), String> {
    let mut blockers = Vec::<RunValidationBlocker>::new();

    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Widget(widget) => {
                let Some(schema) = lookup_widget_schema(&widget.widget) else {
                    blockers.push(RunValidationBlocker {
                        span: node.span,
                        message: format!(
                            "`aivi run` does not support GTK widget `{}` yet",
                            run_widget_name(&widget.widget)
                        ),
                    });
                    continue;
                };
                for property in &widget.properties {
                    let (name, span) = match property {
                        RuntimePropertyBinding::Static(property) => {
                            (property.name.text(), property.site.span)
                        }
                        RuntimePropertyBinding::Setter(binding) => {
                            (binding.name.text(), binding.site.span)
                        }
                    };
                    if schema.property(name).is_none() {
                        blockers.push(RunValidationBlocker {
                            span,
                            message: format!(
                                "`aivi run` does not support property `{name}` on GTK widget `{}` yet",
                                schema.markup_name
                            ),
                        });
                    }
                }
                for event in &widget.event_hooks {
                    if schema.event(event.name.text()).is_none() {
                        blockers.push(RunValidationBlocker {
                            span: event.site.span,
                            message: format!(
                                "`aivi run` does not support event `{}` on GTK widget `{}` yet",
                                event.name.text(),
                                schema.markup_name
                            ),
                        });
                    }
                }
                validate_run_widget_children(
                    node.span,
                    count_unnamed_widget_children(bridge, &widget.default_children.roots),
                    schema,
                    &mut blockers,
                );
            }
            GtkBridgeNodeKind::Group(group) => validate_run_group_children(
                node.span,
                group.body.roots.len(),
                group.descriptor,
                &mut blockers,
            ),
            GtkBridgeNodeKind::Show(_)
            | GtkBridgeNodeKind::Each(_)
            | GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Match(_)
            | GtkBridgeNodeKind::Case(_)
            | GtkBridgeNodeKind::Fragment(_)
            | GtkBridgeNodeKind::With(_) => {}
        }
    }

    let mut root_widgets = collect_run_root_widgets(bridge, bridge.root());
    root_widgets.sort();
    root_widgets.dedup();
    if root_widgets.is_empty() {
        blockers.push(RunValidationBlocker {
            span: bridge.root_node().span,
            message:
                "`aivi run` requires the selected view to produce at least one reachable GTK root widget"
                    .to_owned(),
        });
    }
    for root in root_widgets {
        let Some(node) = bridge.node(root.plan) else {
            continue;
        };
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        let Some(schema) = lookup_widget_schema(&widget.widget) else {
            continue;
        };
        if !schema.is_window_root() {
            blockers.push(RunValidationBlocker {
                span: node.span,
                message: format!(
                    "`aivi run` currently requires reachable root widgets to be `Window`; found `{}`",
                    schema.markup_name
                ),
            });
        }
    }

    if blockers.is_empty() {
        return Ok(());
    }

    let mut rendered = String::from(
        "`aivi run` does not support every GTK/runtime feature yet. Unsupported features in the selected view:\n",
    );
    for blocker in blockers {
        rendered.push_str("- ");
        rendered.push_str(&source_location(sources, blocker.span));
        rendered.push_str(": ");
        rendered.push_str(&blocker.message);
        rendered.push('\n');
    }
    Err(rendered)
}

fn count_unnamed_widget_children(bridge: &GtkBridgeGraph, roots: &[GtkBridgeNodeRef]) -> usize {
    roots
        .iter()
        .filter(|root| {
            !matches!(
                bridge.node(root.plan).map(|node| &node.kind),
                Some(GtkBridgeNodeKind::Group(_))
            )
        })
        .count()
}

fn validate_run_widget_children(
    span: SourceSpan,
    child_count: usize,
    schema: &aivi_gtk::GtkWidgetSchema,
    blockers: &mut Vec<RunValidationBlocker>,
) {
    match schema.default_child_group() {
        aivi_gtk::GtkDefaultChildGroup::None if child_count == 0 => {}
        aivi_gtk::GtkDefaultChildGroup::None => blockers.push(RunValidationBlocker {
            span,
            message: format!(
                "`aivi run` does not support child widgets under `{}`; the current widget schema defines no child group for this widget",
                schema.markup_name
            ),
        }),
        aivi_gtk::GtkDefaultChildGroup::One(group) if group.accepts_child_count(child_count) => {}
        aivi_gtk::GtkDefaultChildGroup::One(group) => blockers.push(RunValidationBlocker {
            span,
            message: match group.max_children {
                Some(max_children) => format!(
                    "`aivi run` does not support {child_count} child widget(s) in `{}` group `{}`; this {} group allows at most {max_children}",
                    schema.markup_name,
                    group.name,
                    group.container.label()
                ),
                None => format!(
                    "`aivi run` does not support {child_count} child widget(s) in `{}` group `{}`",
                    schema.markup_name,
                    group.name
                ),
            },
        }),
        aivi_gtk::GtkDefaultChildGroup::Ambiguous if child_count == 0 => {}
        aivi_gtk::GtkDefaultChildGroup::Ambiguous => blockers.push(RunValidationBlocker {
            span,
            message: format!(
                "`aivi run` cannot place unnamed children under `{}` because the widget schema declares multiple child groups",
                schema.markup_name
            ),
        }),
    }
}

fn validate_run_group_children(
    span: SourceSpan,
    child_count: usize,
    group: &aivi_gtk::GtkChildGroupDescriptor,
    blockers: &mut Vec<RunValidationBlocker>,
) {
    if group.accepts_child_count(child_count) {
        return;
    }
    blockers.push(RunValidationBlocker {
        span,
        message: match group.max_children {
            Some(max_children) => format!(
                "`aivi run` does not support {child_count} child widget(s) in named group `{}`; this {} group allows at most {max_children}",
                group.name,
                group.container.label()
            ),
            None => format!(
                "`aivi run` does not support {child_count} child widget(s) in named group `{}`",
                group.name
            ),
        },
    });
}

fn collect_run_root_widgets(
    bridge: &GtkBridgeGraph,
    root: GtkBridgeNodeRef,
) -> Vec<GtkBridgeNodeRef> {
    let mut widgets = Vec::new();
    let mut worklist = vec![root];
    while let Some(node_ref) = worklist.pop() {
        let Some(node) = bridge.node(node_ref.plan) else {
            continue;
        };
        match &node.kind {
            GtkBridgeNodeKind::Widget(_) => widgets.push(node_ref),
            GtkBridgeNodeKind::Group(group) => extend_child_group_roots(&mut worklist, &group.body),
            GtkBridgeNodeKind::Show(show) => extend_child_group_roots(&mut worklist, &show.body),
            GtkBridgeNodeKind::Each(each) => {
                extend_child_group_roots(&mut worklist, &each.item_template);
                if let Some(empty) = &each.empty_branch {
                    extend_child_group_roots(&mut worklist, &empty.body);
                }
            }
            GtkBridgeNodeKind::Empty(empty) => extend_child_group_roots(&mut worklist, &empty.body),
            GtkBridgeNodeKind::Match(match_node) => {
                for case in &match_node.cases {
                    extend_child_group_roots(&mut worklist, &case.body);
                }
            }
            GtkBridgeNodeKind::Case(case) => extend_child_group_roots(&mut worklist, &case.body),
            GtkBridgeNodeKind::Fragment(fragment) => {
                extend_child_group_roots(&mut worklist, &fragment.body)
            }
            GtkBridgeNodeKind::With(with_node) => {
                extend_child_group_roots(&mut worklist, &with_node.body)
            }
        }
    }
    widgets
}

fn extend_child_group_roots(worklist: &mut Vec<GtkBridgeNodeRef>, group: &GtkChildGroup) {
    worklist.extend(group.roots.iter().rev().copied());
}

fn source_location(sources: &SourceDatabase, span: SourceSpan) -> String {
    let file = &sources[span.file()];
    let location = file.line_column(span.span().start());
    format!(
        "{}:{}:{}",
        file.path().display(),
        location.line,
        location.column
    )
}

struct LoweredRunBackendStack {
    core: aivi_core::Module,
    backend: Arc<BackendProgram>,
}

fn lower_runtime_backend_stack_with_items(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    lower_runtime_backend_stack_impl(module, included_items, command_name, true)
}

fn lower_runtime_backend_stack_with_items_fast(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    lower_runtime_backend_stack_impl(module, included_items, command_name, false)
}

fn lower_runtime_backend_stack_impl(
    module: &HirModule,
    included_items: &IncludedItems,
    command_name: &str,
    validate: bool,
) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_module_with_items(module, included_items).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed core:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_core_module(&core).map_err(|errors| {
            let mut rendered = format!("typed-core validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    let lambda = lower_lambda_module(&core).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_lambda_module(&lambda).map_err(|errors| {
            let mut rendered = format!("typed-lambda validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    let backend = lower_backend_module(&lambda).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    if validate {
        validate_program(&backend).map_err(|errors| {
            let mut rendered = format!("backend validation failed for {command_name}:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    }
    Ok(LoweredRunBackendStack {
        core,
        backend: Arc::new(backend),
    })
}

fn lower_runtime_backend_stack_with_workspace(
    module: &HirModule,
    workspace_hirs: &[(&str, &HirModule)],
    included_items: &IncludedItems,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_module_with_workspace(module, workspace_hirs, included_items)
        .map_err(|errors| {
            let mut rendered = format!("failed to lower {command_name} module into typed core:\n");
            for error in errors.errors() {
                rendered.push_str("- ");
                rendered.push_str(&error.to_string());
                rendered.push('\n');
            }
            rendered
        })?;
    validate_core_module(&core).map_err(|errors| {
        let mut rendered = format!("typed-core validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let lambda = lower_lambda_module(&core).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_lambda_module(&lambda).map_err(|errors| {
        let mut rendered = format!("typed-lambda validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let backend = lower_backend_module(&lambda).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_program(&backend).map_err(|errors| {
        let mut rendered = format!("backend validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    Ok(LoweredRunBackendStack {
        core,
        backend: Arc::new(backend),
    })
}


fn lower_runtime_fragment_backend_stack(
    module: &HirModule,
    fragment: &RuntimeFragmentSpec,
    command_name: &str,
) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_fragment(module, fragment).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed core:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_core_module(&core.module).map_err(|errors| {
        let mut rendered = format!("typed-core validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let lambda = lower_lambda_module(&core.module).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_lambda_module(&lambda).map_err(|errors| {
        let mut rendered = format!("typed-lambda validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let backend = lower_backend_module(&lambda).map_err(|errors| {
        let mut rendered = format!("failed to lower {command_name} module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_program(&backend).map_err(|errors| {
        let mut rendered = format!("backend validation failed for {command_name}:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    Ok(LoweredRunBackendStack {
        core: core.module,
        backend: Arc::new(backend),
    })
}

fn resolve_run_event_handlers(
    module: &HirModule,
    sites: &MarkupRuntimeExprSites,
    bridge: &GtkBridgeGraph,
    runtime_assembly: &HirRuntimeAssembly,
    sources: &SourceDatabase,
) -> Result<BTreeMap<HirExprId, ResolvedRunEventHandler>, String> {
    let mut handlers = BTreeMap::new();
    for node in bridge.nodes() {
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        for event in &widget.event_hooks {
            let resolved = resolve_run_event_handler(
                module,
                sites,
                runtime_assembly,
                sources,
                &widget.widget,
                event.name.text(),
                event.handler,
            )?;
            handlers.entry(event.handler).or_insert(resolved);
        }
    }
    Ok(handlers)
}

fn resolve_run_event_handler(
    module: &HirModule,
    sites: &MarkupRuntimeExprSites,
    runtime_assembly: &HirRuntimeAssembly,
    sources: &SourceDatabase,
    widget: &aivi_hir::NamePath,
    event_name: &str,
    expr: HirExprId,
) -> Result<ResolvedRunEventHandler, String> {
    let location = source_location(sources, module.exprs()[expr].span);
    let Some(event) = lookup_widget_event(widget, event_name) else {
        return Err(format!(
            "event handler at {location} uses unsupported GTK event `{}` on widget `{}`",
            event_name,
            run_widget_name(widget)
        ));
    };
    match &module.exprs()[expr].kind {
        ExprKind::Name(reference) => {
            let resolved =
                resolve_run_event_signal_target(module, runtime_assembly, reference, &location)?;
            let payload = event.payload;
            if !event_signal_accepts_payload(resolved.inner_payload_type.as_ref(), payload) {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but `{}` on `{}` publishes `{}` and requires `{}`",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                    event_name,
                    run_widget_name(widget),
                    payload.label(),
                    payload.required_signal_type_label()
                ));
            }
            Ok(ResolvedRunEventHandler {
                signal_item: resolved.item_id,
                signal_name: resolved.signal_name,
                signal_input: resolved.signal_input,
                payload: ResolvedRunEventPayload::GtkPayload,
            })
        }
        ExprKind::Apply { callee, arguments } => {
            if arguments.len() != 1 {
                return Err(format!(
                    "event handler at {location} must call a direct signal name with exactly one explicit payload expression"
                ));
            }
            let ExprKind::Name(reference) = &module.exprs()[*callee].kind else {
                return Err(format!(
                    "event handler at {location} must call a direct signal name when providing an explicit payload"
                ));
            };
            let payload_expr =
                arguments.iter().next().copied().expect(
                    "single-argument handler applications should expose a payload expression",
                );
            let resolved =
                resolve_run_event_signal_target(module, runtime_assembly, reference, &location)?;
            let required_payload = resolved.inner_payload_type.clone().ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} points at signal `{}`, but explicit payload hooks require a known `Signal A` payload type",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                )
            })?;
            let site = sites.get(payload_expr).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} uses payload expression {} without a collected runtime environment",
                    name_path_text(&reference.path),
                    payload_expr.as_raw()
                )
            })?;
            if site.ty != required_payload {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but the explicit payload has type `{}` and the signal requires `{}`",
                    name_path_text(&reference.path),
                    resolved.signal_name,
                    site.ty,
                    required_payload
                ));
            }
            Ok(ResolvedRunEventHandler {
                signal_item: resolved.item_id,
                signal_name: resolved.signal_name,
                signal_input: resolved.signal_input,
                payload: ResolvedRunEventPayload::ScopedInput,
            })
        }
        _ => Err(format!(
            "event handler at {location} must be a direct signal name or a direct signal application with one explicit payload"
        )),
    }
}

/// Result of resolving an event signal target, covering both same-module and cross-module cases.
struct EventSignalResolution {
    item_id: aivi_hir::ItemId,
    signal_name: Box<str>,
    signal_input: RuntimeInputHandle,
    /// Inner payload type of `Signal A` (e.g. `Unit`, `Text`), used for payload validation.
    inner_payload_type: Option<GateType>,
}

fn resolve_run_event_signal_target(
    module: &HirModule,
    runtime_assembly: &HirRuntimeAssembly,
    reference: &aivi_hir::TermReference,
    location: &str,
) -> Result<EventSignalResolution, String> {
    let hir_item_count =
        u32::try_from(module.items().iter().count()).expect("HIR item count fits u32");
    match reference.resolution.as_ref() {
        aivi_hir::ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            let Item::Signal(signal) = &module.items()[*item_id] else {
                return Err(format!(
                    "event handler `{}` at {location} resolves to a {}, but event hooks require an input-backed signal",
                    name_path_text(&reference.path),
                    hir_item_kind_label(&module.items()[*item_id])
                ));
            };
            let binding = runtime_assembly.signal(*item_id).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location} points at signal `{}` without a runtime binding",
                    name_path_text(&reference.path),
                    signal.name.text()
                )
            })?;
            let Some(signal_input) = binding.input() else {
                return Err(format!(
                    "event handler `{}` at {location} points at signal `{}`, but only direct input signals are publishable from GTK events",
                    name_path_text(&reference.path),
                    signal.name.text()
                ));
            };
            let inner_payload_type = signal_payload_type(module, signal);
            Ok(EventSignalResolution {
                item_id: *item_id,
                signal_name: signal.name.text().into(),
                signal_input,
                inner_payload_type,
            })
        }
        aivi_hir::ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            let Some(import_binding) = module.imports().get(*import_id) else {
                return Err(format!(
                    "event handler `{}` at {location}: import binding not found",
                    name_path_text(&reference.path)
                ));
            };
            let ImportBindingMetadata::Value {
                ty: ImportValueType::Signal(inner_ty),
            } = &import_binding.metadata
            else {
                return Err(format!(
                    "event handler `{}` at {location} resolves to a cross-module import that is not a Signal",
                    name_path_text(&reference.path)
                ));
            };
            let synthetic_id =
                aivi_hir::ItemId::from_raw(hir_item_count + import_id.as_raw());
            let binding = runtime_assembly.signal(synthetic_id).ok_or_else(|| {
                format!(
                    "event handler `{}` at {location}: no runtime stub found for cross-module signal `{}`",
                    name_path_text(&reference.path),
                    import_binding.local_name.text()
                )
            })?;
            let Some(signal_input) = binding.input() else {
                return Err(format!(
                    "event handler `{}` at {location}: cross-module signal `{}` is not a publishable input signal",
                    name_path_text(&reference.path),
                    import_binding.local_name.text()
                ));
            };
            let inner_payload_type = import_value_type_to_gate_type(inner_ty);
            Ok(EventSignalResolution {
                item_id: synthetic_id,
                signal_name: import_binding.local_name.text().into(),
                signal_input,
                inner_payload_type,
            })
        }
        _ => Err(format!(
            "event handler `{}` at {location} must resolve directly to a signal item",
            name_path_text(&reference.path)
        )),
    }
}

/// Checks whether a resolved signal inner type accepts the given GTK event payload.
fn event_signal_accepts_payload(
    inner_ty: Option<&GateType>,
    payload: GtkConcreteEventPayload,
) -> bool {
    let Some(inner_ty) = inner_ty else {
        return false;
    };
    match payload {
        GtkConcreteEventPayload::Unit => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Unit))
        }
        GtkConcreteEventPayload::Bool => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Bool))
        }
        GtkConcreteEventPayload::Text => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Text))
        }
        GtkConcreteEventPayload::F64 => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Float))
        }
        GtkConcreteEventPayload::I64 => {
            matches!(inner_ty, GateType::Primitive(BuiltinType::Int))
        }
    }
}

/// Converts an `ImportValueType` to a `GateType` without needing module context.
/// Returns `None` for `Named` types (user-defined type constructors) which require module lookup.
fn import_value_type_to_gate_type(ty: &ImportValueType) -> Option<GateType> {
    Some(match ty {
        ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
        ImportValueType::Tuple(elements) => GateType::Tuple(
            elements
                .iter()
                .filter_map(import_value_type_to_gate_type)
                .collect(),
        ),
        ImportValueType::Record(fields) => GateType::Record(
            fields
                .iter()
                .filter_map(|field| {
                    import_value_type_to_gate_type(&field.ty).map(|ty| GateRecordField {
                        name: field.name.to_string(),
                        ty,
                    })
                })
                .collect(),
        ),
        ImportValueType::Arrow { parameter, result } => GateType::Arrow {
            parameter: Box::new(import_value_type_to_gate_type(parameter)?),
            result: Box::new(import_value_type_to_gate_type(result)?),
        },
        ImportValueType::List(element) => {
            GateType::List(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Map { key, value } => GateType::Map {
            key: Box::new(import_value_type_to_gate_type(key)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Set(element) => {
            GateType::Set(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Option(element) => {
            GateType::Option(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Result { error, value } => GateType::Result {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Validation { error, value } => GateType::Validation {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::Signal(element) => {
            GateType::Signal(Box::new(import_value_type_to_gate_type(element)?))
        }
        ImportValueType::Task { error, value } => GateType::Task {
            error: Box::new(import_value_type_to_gate_type(error)?),
            value: Box::new(import_value_type_to_gate_type(value)?),
        },
        ImportValueType::TypeVariable { index, name } => GateType::TypeParameter {
            parameter: aivi_hir::TypeParameterId::from_raw(u32::MAX - *index as u32),
            name: name.clone(),
        },
        ImportValueType::Named { .. } => return None,
    })
}




fn name_path_text(path: &aivi_hir::NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}

fn run_widget_name(path: &aivi_hir::NamePath) -> &str {
    path.segments()
        .iter()
        .last()
        .expect("NamePath is non-empty")
        .text()
}

fn hir_item_kind_label(item: &Item) -> &'static str {
    match item {
        Item::Type(_) => "type",
        Item::Value(_) => "value",
        Item::Function(_) => "function",
        Item::Signal(_) => "signal",
        Item::Class(_) => "class",
        Item::Domain(_) => "domain",
        Item::SourceProviderContract(_) => "provider",
        Item::Instance(_) => "instance",
        Item::Use(_) => "use",
        Item::Export(_) => "export",
        Item::Hoist(_) => "hoist",
    }
}

fn collect_run_input_specs_from_bridge(
    module: &HirModule,
    bridge: &GtkBridgeGraph,
) -> BTreeMap<RuntimeInputHandle, RunInputSpec> {
    let mut inputs = BTreeMap::new();
    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Widget(widget) => {
                for property in &widget.properties {
                    if let RuntimePropertyBinding::Setter(setter) = property {
                        let spec = match &setter.source {
                            SetterSource::Expr(expr) => RunInputSpec::Expr(*expr),
                            SetterSource::InterpolatedText(text) => {
                                RunInputSpec::Text(text.clone())
                            }
                        };
                        inputs.insert(setter.input, spec);
                    }
                }
                for event in &widget.event_hooks {
                    if let Some(payload_expr) = event_handler_payload_expr(module, event.handler) {
                        inputs.insert(event.input, RunInputSpec::Expr(payload_expr));
                    }
                }
            }
            GtkBridgeNodeKind::Group(_) => {}
            GtkBridgeNodeKind::Show(show) => {
                inputs.insert(show.when.input, RunInputSpec::Expr(show.when.expr));
                if let RuntimeShowMountPolicy::KeepMounted { decision } = &show.mount {
                    inputs.insert(decision.input, RunInputSpec::Expr(decision.expr));
                }
            }
            GtkBridgeNodeKind::Each(each) => {
                inputs.insert(
                    each.collection.input,
                    RunInputSpec::Expr(each.collection.expr),
                );
                if let Some(key_input) = &each.key_input {
                    inputs.insert(key_input.input, RunInputSpec::Expr(key_input.expr));
                }
            }
            GtkBridgeNodeKind::Match(match_node) => {
                inputs.insert(
                    match_node.scrutinee.input,
                    RunInputSpec::Expr(match_node.scrutinee.expr),
                );
            }
            GtkBridgeNodeKind::With(with_node) => {
                inputs.insert(
                    with_node.value.input,
                    RunInputSpec::Expr(with_node.value.expr),
                );
            }
            GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Case(_)
            | GtkBridgeNodeKind::Fragment(_) => {}
        }
    }
    inputs
}

fn compile_run_inputs(
    sources: &SourceDatabase,
    module: &HirModule,
    view_owner: aivi_hir::ItemId,
    sites: &aivi_hir::MarkupRuntimeExprSites,
    bridge: &GtkBridgeGraph,
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
) -> Result<BTreeMap<RuntimeInputHandle, CompiledRunInput>, String> {
    let mut inputs = BTreeMap::new();
    for (input, spec) in collect_run_input_specs_from_bridge(module, bridge) {
        let compiled = match spec {
            RunInputSpec::Expr(expr) => CompiledRunInput::Expr(compile_run_expr_fragment(
                sources,
                module,
                view_owner,
                sites,
                expr,
                runtime_backend,
                runtime_backend_by_hir,
            )?),
            RunInputSpec::Text(text) => {
                let mut segments = Vec::with_capacity(text.segments.len());
                for segment in text.segments {
                    match segment {
                        aivi_hir::TextSegment::Text(text) => {
                            segments.push(CompiledRunTextSegment::Text(text.raw));
                        }
                        aivi_hir::TextSegment::Interpolation(interpolation) => segments.push(
                            CompiledRunTextSegment::Interpolation(compile_run_expr_fragment(
                                sources,
                                module,
                                view_owner,
                                sites,
                                interpolation.expr,
                                runtime_backend,
                                runtime_backend_by_hir,
                            )?),
                        ),
                    }
                }
                CompiledRunInput::Text(CompiledRunText {
                    segments: segments.into_boxed_slice(),
                })
            }
        };
        inputs.insert(input, compiled);
    }
    Ok(inputs)
}

fn event_handler_payload_expr(module: &HirModule, handler: HirExprId) -> Option<HirExprId> {
    let ExprKind::Apply { arguments, .. } = &module.exprs()[handler].kind else {
        return None;
    };
    if arguments.len() != 1 {
        return None;
    }
    arguments.iter().next().copied()
}

fn collect_run_required_signal_globals(
    inputs: &BTreeMap<RuntimeInputHandle, CompiledRunInput>,
) -> BTreeMap<BackendItemId, Box<str>> {
    let mut required = BTreeMap::new();
    for input in inputs.values() {
        extend_run_required_signal_globals(input, &mut required);
    }
    required
}

fn extend_run_required_signal_globals(
    input: &CompiledRunInput,
    required: &mut BTreeMap<BackendItemId, Box<str>>,
) {
    match input {
        CompiledRunInput::Expr(fragment) => {
            for dependency in &fragment.required_signal_globals {
                required
                    .entry(dependency.runtime_item)
                    .or_insert_with(|| dependency.name.clone());
            }
        }
        CompiledRunInput::Text(text) => {
            for segment in &text.segments {
                let CompiledRunTextSegment::Interpolation(fragment) = segment else {
                    continue;
                };
                for dependency in &fragment.required_signal_globals {
                    required
                        .entry(dependency.runtime_item)
                        .or_insert_with(|| dependency.name.clone());
                }
            }
        }
    }
}

fn run_hydration_globals_ready(
    required: &BTreeMap<BackendItemId, Box<str>>,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> bool {
    required.keys().all(|item| globals.contains_key(item))
}

/// For each workspace Signal import in the assembly's stub Input signals, compute
/// a type-based default runtime value to pre-seed the signal before the first
/// hydration cycle. This prevents hydration from blocking on cross-module signals
/// that have no daemon publisher.
fn collect_stub_signal_defaults(
    module: &HirModule,
    assembly: &HirRuntimeAssembly,
) -> Vec<(RuntimeInputHandle, DetachedRuntimeValue)> {
    let hir_item_count =
        u32::try_from(module.items().iter().count()).expect("HIR item count fits u32");
    let mut defaults = Vec::new();
    for signal_binding in assembly.signals() {
        let raw = signal_binding.item.as_raw();
        if raw < hir_item_count {
            continue; // Real HIR item, not a stub.
        }
        let import_id = ImportId::from_raw(raw - hir_item_count);
        let Some(import_binding) = module.imports().get(import_id) else {
            continue;
        };
        let ImportBindingMetadata::Value {
            ty: ImportValueType::Signal(inner_ty),
        } = &import_binding.metadata
        else {
            continue;
        };
        let Some(input) = signal_binding.input() else {
            continue;
        };
        let default_value =
            DetachedRuntimeValue::from_runtime_owned(default_runtime_value_for_import_type(inner_ty));
        defaults.push((input, default_value));
    }
    defaults
}

fn default_runtime_value_for_import_type(ty: &ImportValueType) -> RuntimeValue {
    match ty {
        ImportValueType::Primitive(builtin) => match builtin {
            BuiltinType::Text => RuntimeValue::Text("".into()),
            BuiltinType::Int => RuntimeValue::Int(0),
            BuiltinType::Bool => RuntimeValue::Bool(false),
            BuiltinType::Float => RuntimeValue::Float(RuntimeFloat::new(0.0_f64).expect("0.0 is a valid float")),
            BuiltinType::Unit => RuntimeValue::Unit,
            _ => RuntimeValue::Unit,
        },
        ImportValueType::List(_) => RuntimeValue::List(vec![]),
        ImportValueType::Set(_) => RuntimeValue::Set(vec![]),
        ImportValueType::Map { .. } => RuntimeValue::Map(Default::default()),
        ImportValueType::Option(_) => RuntimeValue::OptionNone,
        ImportValueType::Result { error, .. } => RuntimeValue::ResultErr(Box::new(
            default_runtime_value_for_import_type(error),
        )),
        ImportValueType::Validation { error, .. } => RuntimeValue::ValidationInvalid(Box::new(
            default_runtime_value_for_import_type(error),
        )),
        ImportValueType::Tuple(elements) => RuntimeValue::Tuple(
            elements
                .iter()
                .map(default_runtime_value_for_import_type)
                .collect(),
        ),
        ImportValueType::Record(fields) => RuntimeValue::Record(
            fields
                .iter()
                .map(|f| RuntimeRecordField {
                    label: f.name.clone(),
                    value: default_runtime_value_for_import_type(&f.ty),
                })
                .collect(),
        ),
        ImportValueType::Signal(inner) => RuntimeValue::Signal(Box::new(
            default_runtime_value_for_import_type(inner),
        )),
        // Functions, tasks, and named/variable types cannot be trivially defaulted.
        ImportValueType::Arrow { .. }
        | ImportValueType::Task { .. }
        | ImportValueType::TypeVariable { .. }
        | ImportValueType::Named { .. } => RuntimeValue::Unit,
    }
}

fn compile_run_expr_fragment(
    sources: &SourceDatabase,
    module: &HirModule,
    view_owner: aivi_hir::ItemId,
    sites: &aivi_hir::MarkupRuntimeExprSites,
    expr: HirExprId,
    runtime_backend: &BackendProgram,
    runtime_backend_by_hir: &BTreeMap<aivi_hir::ItemId, BackendItemId>,
) -> Result<CompiledRunFragment, String> {
    let site = sites.get(expr).ok_or_else(|| {
        format!(
            "run view references expression {} at {} without a collected runtime environment",
            expr.as_raw(),
            source_location(sources, module.exprs()[expr].span)
        )
    })?;
    let body = elaborate_runtime_expr_with_env(module, expr, &site.parameters, Some(&site.ty))
        .map_err(|blocked| {
            format!(
                "failed to elaborate runtime expression at {}: {}",
                source_location(sources, site.span),
                blocked
            )
        })?;
    let fragment = RuntimeFragmentSpec {
        name: format!("__run_fragment_{}", expr.as_raw()).into_boxed_str(),
        owner: view_owner,
        body_expr: expr,
        parameters: site.parameters.clone(),
        body,
    };
    let core = lower_runtime_fragment(module, &fragment).map_err(|error| {
        format!(
            "failed to lower runtime expression at {} into typed core: {error}",
            source_location(sources, site.span)
        )
    })?;
    let lambda = lower_lambda_module(&core.module).map_err(|error| {
        format!(
            "failed to lower runtime expression at {} into typed lambda: {error}",
            source_location(sources, site.span)
        )
    })?;
    validate_lambda_module(&lambda).map_err(|error| {
        format!(
            "typed lambda validation failed for runtime expression at {}: {error}",
            source_location(sources, site.span)
        )
    })?;
    let backend = lower_backend_module(&lambda).map_err(|error| {
        format!(
            "failed to lower runtime expression at {} into backend IR: {error}",
            source_location(sources, site.span)
        )
    })?;
    validate_program(&backend).map_err(|error| {
        format!(
            "backend validation failed for runtime expression at {}: {error}",
            source_location(sources, site.span)
        )
    })?;
    let item = backend
        .items()
        .iter()
        .find_map(|(item_id, item)| (item.name == core.entry_name).then_some(item_id))
        .ok_or_else(|| {
            format!(
                "backend lowering did not preserve runtime fragment `{}` for expression at {}",
                core.entry_name,
                source_location(sources, site.span)
            )
        })?;
    let required_signal_globals = backend.items()[item]
        .body
        .map(|kernel| backend.kernels()[kernel].global_items.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|fragment_item| {
            let fragment_decl = backend.items().get(fragment_item).ok_or_else(|| {
                format!(
                    "compiled runtime fragment {} references missing backend item {}",
                    expr.as_raw(),
                    fragment_item
                )
            })?;
            let core_item = core
                .module
                .items()
                .get(fragment_decl.origin)
                .ok_or_else(|| {
                    format!(
                        "compiled runtime fragment {} lost core→HIR origin for backend item {}",
                        expr.as_raw(),
                        fragment_item
                    )
                })?;
            let hir_item = core_item.origin;
            // eprintln!("DEBUG: fragment {} backend item {} core item {} hir_item {} name={}", expr.as_raw(), fragment_item, fragment_decl.origin, hir_item.as_raw(), core_item.name);
            // Look up the HIR item. For cross-module signals, the origin is a synthetic ID
            // that doesn't correspond to a real HIR item — in that case fall back to the
            // core item's own name (which is the import's local name).
            let hir_lookup = module.items().get(hir_item);
            // eprintln!("DEBUG:   hir_lookup = {:?}", hir_lookup.map(|i| i.label()));
            let signal_name: Box<str> = match hir_lookup {
                Some(Item::Signal(signal)) => signal.name.text().into(),
                Some(_) => return Ok(None),
                None => core_item.name.clone(),
            };
            let runtime_item = if hir_lookup.is_some() {
                runtime_backend_by_hir.get(&hir_item).copied().ok_or_else(|| {
                    format!(
                        "runtime fragment {} needs signal `{signal_name}` but the live run backend has no matching item",
                        expr.as_raw(),
                    )
                })?
            } else {
                // Synthetic origin: find the backend item by signal name instead.
                runtime_backend.items().iter()
                    .find_map(|(bid, bitem)| {
                        (bitem.name.as_ref() == signal_name.as_ref()
                            && matches!(bitem.kind, aivi_backend::ItemKind::Signal(_)))
                        .then_some(bid)
                    })
                    .ok_or_else(|| {
                        format!(
                            "runtime fragment {} needs signal `{signal_name}` (synthetic origin) but no matching signal found",
                            expr.as_raw(),
                        )
                    })?
            };
            let runtime_decl = runtime_backend.items().get(runtime_item).ok_or_else(|| {
                format!(
                    "live run backend is missing runtime item {} for signal `{signal_name}`",
                    runtime_item,
                )
            })?;
            if !matches!(runtime_decl.kind, aivi_backend::ItemKind::Signal(_)) {
                return Err(format!(
                    "live run backend item {} for signal `{signal_name}` is not a signal",
                    runtime_item,
                ));
            }
            Ok(Some(CompiledRunSignalGlobal {
                fragment_item,
                runtime_item,
                name: signal_name,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(CompiledRunFragment {
        expr,
        parameters: site.parameters.clone(),
        program: backend,
        item,
        required_signal_globals,
    })
}

type RuntimeBindingEnv = BTreeMap<aivi_hir::BindingId, RuntimeValue>;
fn plan_run_hydration(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> Result<RunHydrationPlan, String> {
    let runtime_globals = runtime_globals_from_detached(globals);
    Ok(RunHydrationPlan {
        root: plan_run_node(
            shared,
            &runtime_globals,
            &GtkNodeInstance::root(shared.bridge.root()),
            &RuntimeBindingEnv::new(),
        )?,
    })
}

fn runtime_globals_from_detached(
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> BTreeMap<BackendItemId, RuntimeValue> {
    globals
        .iter()
        .map(|(&item, value)| (item, value.to_runtime()))
        .collect()
}

fn plan_run_node(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    instance: &GtkNodeInstance,
    env: &RuntimeBindingEnv,
) -> Result<HydratedRunNode, String> {
    let view_name = shared.view_name.as_ref();
    let node = shared.bridge.node(instance.node.plan).ok_or_else(|| {
        format!(
            "run view `{view_name}` is missing GTK node {}",
            instance.node
        )
    })?;
    match &node.kind {
        GtkBridgeNodeKind::Widget(widget) => {
            let mut properties = Vec::new();
            for property in &widget.properties {
                if let RuntimePropertyBinding::Setter(setter) = property {
                    properties.push(HydratedRunProperty {
                        input: setter.input,
                        value: DetachedRuntimeValue::from_runtime_owned(evaluate_run_input(
                            &shared.inputs,
                            globals,
                            setter.input,
                            env,
                        )?),
                    });
                }
            }
            let mut event_inputs = Vec::new();
            for event in &widget.event_hooks {
                if !shared.inputs.contains_key(&event.input) {
                    continue;
                }
                event_inputs.push(HydratedRunProperty {
                    input: event.input,
                    value: DetachedRuntimeValue::from_runtime_owned(evaluate_run_input(
                        &shared.inputs,
                        globals,
                        event.input,
                        env,
                    )?),
                });
            }
            Ok(HydratedRunNode::Widget {
                instance: instance.clone(),
                properties: properties.into_boxed_slice(),
                event_inputs: event_inputs.into_boxed_slice(),
                children: plan_run_child_group(
                    shared,
                    globals,
                    &widget.default_children.roots,
                    instance.path.clone(),
                    env,
                )?,
            })
        }
        GtkBridgeNodeKind::Group(group) => Ok(HydratedRunNode::Fragment {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &group.body.roots,
                instance.path.clone(),
                env,
            )?,
        }),
        GtkBridgeNodeKind::Show(show) => {
            let when = runtime_bool(evaluate_run_input(
                &shared.inputs,
                globals,
                show.when.input,
                env,
            )?)
            .ok_or_else(|| {
                format!(
                    "run view `{view_name}` expected `<show when>` on {instance} to evaluate to Bool"
                )
            })?;
            let (keep_mounted_input, keep_mounted) = match &show.mount {
                RuntimeShowMountPolicy::UnmountWhenHidden => (None, false),
                RuntimeShowMountPolicy::KeepMounted { decision } => (
                    Some(decision.input),
                    runtime_bool(evaluate_run_input(
                        &shared.inputs,
                        globals,
                        decision.input,
                        env,
                    )?)
                    .ok_or_else(|| {
                        format!(
                            "run view `{view_name}` expected `<show keepMounted>` on {instance} to evaluate to Bool"
                        )
                    })?,
                ),
            };
            let children = if when || keep_mounted {
                plan_run_child_group(
                    shared,
                    globals,
                    &show.body.roots,
                    instance.path.clone(),
                    env,
                )?
            } else {
                Vec::new().into_boxed_slice()
            };
            Ok(HydratedRunNode::Show {
                instance: instance.clone(),
                when_input: show.when.input,
                when,
                keep_mounted_input,
                keep_mounted,
                children,
            })
        }
        GtkBridgeNodeKind::Each(each) => {
            let values = runtime_list_values(evaluate_run_input(
                &shared.inputs,
                globals,
                each.collection.input,
                env,
            )?)
            .ok_or_else(|| {
                format!(
                    "run view `{view_name}` expected `<each>` collection on {instance} to evaluate to a List"
                )
            })?;
            let collection_is_empty = values.is_empty();
            let kind = match &each.child_policy {
                RepeatedChildPolicy::Positional { .. } => {
                    let mut items = Vec::with_capacity(values.len());
                    for (index, value) in values.into_iter().enumerate() {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value);
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Positional(index),
                        );
                        items.push(HydratedRunEachItem {
                            children: plan_run_child_group(
                                shared,
                                globals,
                                &each.item_template.roots,
                                path,
                                &child_env,
                            )?,
                        });
                    }
                    HydratedRunEachKind::Positional {
                        item_count: items.len(),
                        items: items.into_boxed_slice(),
                    }
                }
                RepeatedChildPolicy::Keyed { .. } => {
                    let key_input = each.key_input.as_ref().ok_or_else(|| {
                        format!(
                            "run view `{view_name}` is missing a keyed `<each>` runtime input on {instance}"
                        )
                    })?;
                    let mut items = Vec::with_capacity(values.len());
                    let mut keys = Vec::with_capacity(values.len());
                    for value in values {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value);
                        let collection_key = runtime_collection_key(evaluate_run_input(
                            &shared.inputs,
                            globals,
                            key_input.input,
                            &child_env,
                        )?)
                        .ok_or_else(|| {
                            format!(
                                "run view `{view_name}` expected `<each>` key on {instance} to be displayable"
                            )
                        })?;
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Keyed(collection_key.clone()),
                        );
                        keys.push(collection_key);
                        items.push(HydratedRunEachItem {
                            children: plan_run_child_group(
                                shared,
                                globals,
                                &each.item_template.roots,
                                path,
                                &child_env,
                            )?,
                        });
                    }
                    HydratedRunEachKind::Keyed {
                        key_input: key_input.input,
                        keys: keys.into_boxed_slice(),
                        items: items.into_boxed_slice(),
                    }
                }
            };
            let empty_branch = if collection_is_empty {
                each.empty_branch
                    .as_ref()
                    .map(|empty| {
                        plan_run_node(
                            shared,
                            globals,
                            &GtkNodeInstance::with_path(empty.empty, instance.path.clone()),
                            env,
                        )
                    })
                    .transpose()?
                    .map(Box::new)
            } else {
                None
            };
            Ok(HydratedRunNode::Each {
                instance: instance.clone(),
                collection_input: each.collection.input,
                kind,
                empty_branch,
            })
        }
        GtkBridgeNodeKind::Match(match_node) => {
            let value =
                evaluate_run_input(&shared.inputs, globals, match_node.scrutinee.input, env)?;
            let mut matched = None;
            for (index, branch) in match_node.cases.iter().enumerate() {
                let mut bindings = RuntimeBindingEnv::new();
                if match_pattern(&shared.module, branch.pattern, &value, &mut bindings)? {
                    matched = Some((index, branch, bindings));
                    break;
                }
            }
            let Some((active_case, branch, bindings)) = matched else {
                return Err(format!(
                    "run view `{view_name}` found no matching `<match>` case for node {instance}"
                ));
            };
            let mut case_env = env.clone();
            case_env.extend(bindings);
            Ok(HydratedRunNode::Match {
                instance: instance.clone(),
                scrutinee_input: match_node.scrutinee.input,
                active_case,
                branch: Box::new(plan_run_node(
                    shared,
                    globals,
                    &GtkNodeInstance::with_path(branch.case, instance.path.clone()),
                    &case_env,
                )?),
            })
        }
        GtkBridgeNodeKind::Case(case) => Ok(HydratedRunNode::Case {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &case.body.roots,
                instance.path.clone(),
                env,
            )?,
        }),
        GtkBridgeNodeKind::Fragment(fragment) => Ok(HydratedRunNode::Fragment {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &fragment.body.roots,
                instance.path.clone(),
                env,
            )?,
        }),
        GtkBridgeNodeKind::With(with_node) => {
            let value = evaluate_run_input(&shared.inputs, globals, with_node.value.input, env)?;
            let mut child_env = env.clone();
            child_env.insert(with_node.binding, strip_signal_runtime_value(value));
            Ok(HydratedRunNode::With {
                instance: instance.clone(),
                value_input: with_node.value.input,
                children: plan_run_child_group(
                    shared,
                    globals,
                    &with_node.body.roots,
                    instance.path.clone(),
                    &child_env,
                )?,
            })
        }
        GtkBridgeNodeKind::Empty(empty) => Ok(HydratedRunNode::Empty {
            instance: instance.clone(),
            children: plan_run_child_group(
                shared,
                globals,
                &empty.body.roots,
                instance.path.clone(),
                env,
            )?,
        }),
    }
}

fn plan_run_child_group(
    shared: &RunHydrationStaticState,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    roots: &[aivi_gtk::GtkBridgeNodeRef],
    path: GtkExecutionPath,
    env: &RuntimeBindingEnv,
) -> Result<Box<[HydratedRunNode]>, String> {
    let mut children = Vec::with_capacity(roots.len());
    for &root in roots {
        children.push(plan_run_node(
            shared,
            globals,
            &GtkNodeInstance::with_path(root, path.clone()),
            env,
        )?);
    }
    Ok(children.into_boxed_slice())
}

fn apply_run_hydration_plan(
    plan: &RunHydrationPlan,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    apply_run_node(&plan.root, executor)
}

fn apply_run_children(
    children: &[HydratedRunNode],
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    for child in children {
        apply_run_node(child, executor)?;
    }
    Ok(())
}

fn apply_run_node(
    node: &HydratedRunNode,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    match node {
        HydratedRunNode::Widget {
            instance,
            properties,
            event_inputs,
            children,
        } => {
            for property in properties {
                executor
                    .set_input_for_instance(
                        instance,
                        property.input,
                        RunHostValue(property.value.clone()),
                    )
                    .map_err(|error| {
                        format!(
                            "failed to apply dynamic input {} on {}: {error}",
                            property.input.as_raw(),
                            instance
                        )
                    })?;
            }
            for event_input in event_inputs {
                executor
                    .set_input_for_instance(
                        instance,
                        event_input.input,
                        RunHostValue(event_input.value.clone()),
                    )
                    .map_err(|error| {
                        format!(
                            "failed to apply event input {} on {}: {error}",
                            event_input.input.as_raw(),
                            instance
                        )
                    })?;
            }
            apply_run_children(children, executor)
        }
        HydratedRunNode::Show {
            instance,
            when,
            keep_mounted,
            children,
            ..
        } => {
            executor
                .update_show(instance, *when, *keep_mounted)
                .map_err(|error| format!("failed to update `<show>` node {instance}: {error}"))?;
            apply_run_children(children, executor)
        }
        HydratedRunNode::Each {
            instance,
            kind,
            empty_branch,
            ..
        } => {
            match kind {
                HydratedRunEachKind::Positional { item_count, items } => {
                    executor
                        .update_each_positional(instance, *item_count)
                        .map_err(|error| {
                            format!("failed to update positional `<each>` node {instance}: {error}")
                        })?;
                    for item in items {
                        apply_run_children(&item.children, executor)?;
                    }
                }
                HydratedRunEachKind::Keyed { keys, items, .. } => {
                    executor
                        .update_each_keyed(instance, keys)
                        .map_err(|error| {
                            format!("failed to update keyed `<each>` node {instance}: {error}")
                        })?;
                    for item in items {
                        apply_run_children(&item.children, executor)?;
                    }
                }
            }
            if let Some(empty_branch) = empty_branch {
                apply_run_node(empty_branch, executor)?;
            }
            Ok(())
        }
        HydratedRunNode::Match {
            instance,
            active_case,
            branch,
            ..
        } => {
            executor
                .update_match(instance, *active_case)
                .map_err(|error| format!("failed to update `<match>` node {instance}: {error}"))?;
            apply_run_node(branch, executor)
        }
        HydratedRunNode::Case { children, .. }
        | HydratedRunNode::Fragment { children, .. }
        | HydratedRunNode::With { children, .. }
        | HydratedRunNode::Empty { children, .. } => apply_run_children(children, executor),
    }
}

fn evaluate_run_input(
    inputs: &BTreeMap<RuntimeInputHandle, CompiledRunInput>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    input: RuntimeInputHandle,
    env: &RuntimeBindingEnv,
) -> Result<RuntimeValue, String> {
    let compiled = inputs.get(&input).ok_or_else(|| {
        format!(
            "missing compiled runtime input {} for live run hydration",
            input.as_raw()
        )
    })?;
    match compiled {
        CompiledRunInput::Expr(fragment) => evaluate_compiled_run_fragment(fragment, globals, env),
        CompiledRunInput::Text(text) => evaluate_compiled_run_text(text, globals, env),
    }
}

fn evaluate_compiled_run_fragment(
    fragment: &CompiledRunFragment,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    env: &RuntimeBindingEnv,
) -> Result<RuntimeValue, String> {
    let args = fragment
        .parameters
        .iter()
        .map(|parameter| {
            env.get(&parameter.binding).cloned().ok_or_else(|| {
                format!(
                    "missing runtime value for binding `{}` while evaluating expression {}",
                    parameter.name,
                    fragment.expr.as_raw()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let item = &fragment.program.items()[fragment.item];
    let mut evaluator = KernelEvaluator::new(&fragment.program);
    let required_globals = fragment
        .required_signal_globals
        .iter()
        .map(|dep| {
            globals
                .get(&dep.runtime_item)
                .cloned()
                .map(|value| (dep.fragment_item, value))
                .ok_or_else(|| {
                    format!(
                        "runtime expression {} requires current signal `{}` (runtime item {}) but no committed snapshot exists",
                        fragment.expr.as_raw(),
                        dep.name,
                        dep.runtime_item
                    )
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if args.is_empty() {
        evaluator
            .evaluate_item(fragment.item, &required_globals)
            .map_err(|error| format!("{error}"))
    } else {
        let kernel = item.body.ok_or_else(|| {
            format!(
                "compiled runtime fragment {} has no executable body",
                fragment.expr.as_raw()
            )
        })?;
        evaluator
            .evaluate_kernel(kernel, None, &args, &required_globals)
            .map_err(|error| format!("{error}"))
    }
}

fn backend_items_by_hir(
    core: &aivi_core::Module,
    backend: &BackendProgram,
) -> BTreeMap<aivi_hir::ItemId, BackendItemId> {
    let core_to_hir = core
        .items()
        .iter()
        .map(|(core_id, item)| (core_id, item.origin))
        .collect::<BTreeMap<_, _>>();
    backend
        .items()
        .iter()
        .filter_map(|(backend_id, item)| {
            core_to_hir
                .get(&item.origin)
                .copied()
                .map(|hir_id| (hir_id, backend_id))
        })
        .collect()
}

fn evaluate_compiled_run_text(
    text: &CompiledRunText,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    env: &RuntimeBindingEnv,
) -> Result<RuntimeValue, String> {
    let mut rendered = String::new();
    for segment in &text.segments {
        match segment {
            CompiledRunTextSegment::Text(text) => rendered.push_str(text),
            CompiledRunTextSegment::Interpolation(fragment) => {
                let value = strip_signal_runtime_value(evaluate_compiled_run_fragment(
                    fragment, globals, env,
                )?);
                if matches!(value, RuntimeValue::Callable(_)) {
                    return Err(format!(
                        "text interpolation for expression {} produced a callable runtime value",
                        fragment.expr.as_raw()
                    ));
                }
                rendered.push_str(&value.to_string());
            }
        }
    }
    Ok(RuntimeValue::Text(rendered.into_boxed_str()))
}

fn runtime_bool(value: RuntimeValue) -> Option<bool> {
    strip_signal_runtime_value(value).as_bool()
}

fn runtime_list_values(value: RuntimeValue) -> Option<Vec<RuntimeValue>> {
    match strip_signal_runtime_value(value) {
        RuntimeValue::List(values) => Some(values),
        _ => None,
    }
}

fn runtime_collection_key(value: RuntimeValue) -> Option<GtkCollectionKey> {
    let value = strip_signal_runtime_value(value);
    (!matches!(value, RuntimeValue::Callable(_))).then(|| GtkCollectionKey::new(value.to_string()))
}

fn strip_signal_runtime_value(mut value: RuntimeValue) -> RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = *inner;
    }
    value
}

fn strip_signal_runtime_ref(mut value: &RuntimeValue) -> &RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = inner.as_ref();
    }
    value
}

fn match_pattern(
    module: &HirModule,
    pattern_id: HirPatternId,
    value: &RuntimeValue,
    bindings: &mut RuntimeBindingEnv,
) -> Result<bool, String> {
    let pattern = &module.patterns()[pattern_id];
    match &pattern.kind {
        PatternKind::Wildcard => Ok(true),
        PatternKind::Binding(binding) => {
            bindings.insert(binding.binding, strip_signal_runtime_value(value.clone()));
            Ok(true)
        }
        PatternKind::Integer(integer) => Ok(matches!(
            strip_signal_runtime_value(value.clone()),
            RuntimeValue::Int(found) if integer.raw.parse::<i64>().ok() == Some(found)
        )),
        PatternKind::Text(text) => Ok(matches!(
            strip_signal_runtime_value(value.clone()),
            RuntimeValue::Text(found)
                if text_literal_static_text(text).as_deref() == Some(found.as_ref())
        )),
        PatternKind::Tuple(elements) => {
            let RuntimeValue::Tuple(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            let expected = elements.iter().copied().collect::<Vec<_>>();
            if expected.len() != found.len() {
                return Ok(false);
            }
            let mut matches = true;
            for (pattern, value) in expected.into_iter().zip(found.iter()) {
                matches &= match_pattern(module, pattern, value, bindings)?;
            }
            Ok(matches)
        }
        PatternKind::List { elements, rest } => {
            let RuntimeValue::List(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            if found.len() < elements.len() {
                return Ok(false);
            }
            if rest.is_none() && found.len() != elements.len() {
                return Ok(false);
            }
            let mut matches = true;
            for (pattern, value) in elements.iter().copied().zip(found.iter()) {
                matches &= match_pattern(module, pattern, value, bindings)?;
            }
            if let Some(rest) = rest {
                let remaining = RuntimeValue::List(found[elements.len()..].to_vec());
                matches &= match_pattern(module, *rest, &remaining, bindings)?;
            }
            Ok(matches)
        }
        PatternKind::Record(fields) => {
            let RuntimeValue::Record(found) = strip_signal_runtime_value(value.clone()) else {
                return Ok(false);
            };
            let mut matches = true;
            for field in fields {
                let Some(found_field) = found
                    .iter()
                    .find(|candidate| candidate.label.as_ref() == field.label.text())
                else {
                    return Ok(false);
                };
                matches &= match_pattern(module, field.pattern, &found_field.value, bindings)?;
            }
            Ok(matches)
        }
        PatternKind::Constructor { callee, arguments } => match callee.resolution.as_ref() {
            aivi_hir::ResolutionState::Resolved(TermResolution::Builtin(term)) => {
                match_builtin_pattern(*term, arguments, value, module, bindings)
            }
            aivi_hir::ResolutionState::Resolved(TermResolution::Item(item)) => {
                let RuntimeValue::Sum(found) = strip_signal_runtime_value(value.clone()) else {
                    return Ok(false);
                };
                let variant_name = callee.path.segments().last().text();
                if found.item != *item || found.variant_name.as_ref() != variant_name {
                    return Ok(false);
                }
                if arguments.len() != found.fields.len() {
                    return Ok(false);
                }
                let mut matches = true;
                for (pattern, field) in arguments.iter().copied().zip(found.fields.iter()) {
                    matches &= match_pattern(module, pattern, field, bindings)?;
                }
                Ok(matches)
            }
            _ => Ok(false),
        },
        PatternKind::UnresolvedName(_) => Ok(false),
    }
}

fn match_builtin_pattern(
    term: BuiltinTerm,
    arguments: &[HirPatternId],
    value: &RuntimeValue,
    module: &HirModule,
    bindings: &mut RuntimeBindingEnv,
) -> Result<bool, String> {
    let Some(payload) = truthy_falsy_payload(value, term) else {
        return Ok(false);
    };
    match (payload, arguments) {
        (None, []) => Ok(true),
        (Some(payload), [argument]) => match_pattern(module, *argument, &payload, bindings),
        _ => Ok(false),
    }
}

fn truthy_falsy_payload(
    value: &RuntimeValue,
    constructor: BuiltinTerm,
) -> Option<Option<RuntimeValue>> {
    match (constructor, strip_signal_runtime_value(value.clone())) {
        (BuiltinTerm::True, RuntimeValue::Bool(true))
        | (BuiltinTerm::False, RuntimeValue::Bool(false))
        | (BuiltinTerm::None, RuntimeValue::OptionNone) => Some(None),
        (BuiltinTerm::Some, RuntimeValue::OptionSome(payload))
        | (BuiltinTerm::Ok, RuntimeValue::ResultOk(payload))
        | (BuiltinTerm::Err, RuntimeValue::ResultErr(payload))
        | (BuiltinTerm::Valid, RuntimeValue::ValidationValid(payload))
        | (BuiltinTerm::Invalid, RuntimeValue::ValidationInvalid(payload)) => Some(Some(*payload)),
        _ => None,
    }
}

fn text_literal_static_text(text: &aivi_hir::TextLiteral) -> Option<String> {
    let mut rendered = String::new();
    for segment in &text.segments {
        match segment {
            aivi_hir::TextSegment::Text(fragment) => rendered.push_str(fragment.raw.as_ref()),
            aivi_hir::TextSegment::Interpolation(_) => return None,
        }
    }
    Some(rendered)
}

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
    let artifact = match prepare_run_artifact(&snapshot.sources, lowered.module(), &[], requested_view) {
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
    check <path>                    Type-check a module through HIR
    compile <path> [-o <object>]    Compile a module to native object code
    build <path> -o <dir> [opts]    Package a runnable GTK app bundle
    run [path] [opts]               Launch a live GTK app
    execute <path> [-- args...]     Run a headless Task program
    test <path>                     Run @test declarations in a workspace
    lex <path>                      Dump the lossless token stream
    fmt <path|--stdin|--check>      Format AIVI source code
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
        "check" => "\
aivi check — type-check a module through HIR

USAGE:
    aivi check <path>

ARGS:
    <path>              Path to an .aivi source file or workspace entry

DESCRIPTION:
    Lexes, parses, lowers, and validates a module through the full HIR
    pipeline. Reports any syntax errors, name resolution failures, or
    type errors as diagnostics. Exits with code 0 on success, 1 on
    diagnostic errors.
",
        "compile" => "\
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
",
        "build" => "\
aivi build — package a runnable GTK app bundle

USAGE:
    aivi build <path> -o <directory> [--app <name>] [--view <name>]

ARGS:
    <path>              Path to an .aivi source file or workspace entry

OPTIONS:
    --app <name>
            Select a named app from `[[app]]` in aivi.toml.
            Required when multiple apps are defined and no --path is given.

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
",
        "run" => "\
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
            Required when multiple apps are defined and no --path is given.

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
",
        "execute" => "\
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
",
        "test" => "\
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
",
        "lex" => "\
aivi lex — dump the lossless token stream

USAGE:
    aivi lex <path>

ARGS:
    <path>              Path to an .aivi source file

DESCRIPTION:
    Lexes the source file and prints every token in the lossless token
    stream, including whitespace and comments. Useful for debugging the
    lexer and inspecting tokenization behavior.
",
        "fmt" => "\
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
",
        "lsp" => "\
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
",
        "mcp" => "\
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
",
        "manual-snippets" => "\
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
",
        _ => return None,
    };
    Some(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        HydratedRunNode, ResolvedRunEventHandler, ResolvedRunEventPayload, RunHydrationStaticState,
        WorkspaceHirSnapshot, check_file, execute_file_with_context, plan_run_hydration,
        prepare_execute_artifact, prepare_run_artifact, run_hydration_globals_ready,
        test_file_with_context,
    };
    use aivi_backend::{DetachedRuntimeValue, RuntimeTaskPlan, RuntimeValue};
    use aivi_base::SourceDatabase;
    use aivi_gtk::{GtkBridgeNodeKind, RuntimePropertyBinding, RuntimeShowMountPolicy};
    use aivi_hir::{ValidationMode, lower_module as lower_hir_module};
    use aivi_runtime::{SourceProviderContext, execute_runtime_task_plan};
    use aivi_syntax::parse_module;
    use std::{
        collections::BTreeMap,
        env, fs,
        path::{Path, PathBuf},
        process::ExitCode,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/frontend")
            .join(path)
    }

    fn repo_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after unix epoch")
                .as_nanos();
            let path =
                env::temp_dir().join(format!("aivi-cli-{prefix}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).expect("temporary directory should be creatable");
            Self { path }
        }

        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("temporary parent directories should exist");
            }
            fs::write(&path, text).expect("temporary workspace file should be writable");
            path
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn prepare_run_from_text(
        path: &str,
        source: &str,
        requested_view: Option<&str>,
    ) -> Result<super::RunArtifact, String> {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, source);
        let file = &sources[file_id];
        let parsed = parse_module(file);
        assert!(!parsed.has_errors(), "test input should parse cleanly");
        let lowered = lower_hir_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "test input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let validation = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            validation.diagnostics().is_empty(),
            "test input should validate cleanly: {:?}",
            validation.diagnostics()
        );
        prepare_run_artifact(&sources, lowered.module(), &[], requested_view)
    }

    fn prepare_run_from_workspace(
        root: &TempDir,
        entry_relative: &str,
        requested_view: Option<&str>,
    ) -> Result<super::RunArtifact, String> {
        let snapshot = WorkspaceHirSnapshot::load(&root.path().join(entry_relative))?;
        assert!(
            !super::workspace_syntax_failed(&snapshot, |_, diagnostics| diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)),
            "workspace fixture should parse cleanly"
        );
        let (hir_failed, validation_failed) = super::workspace_hir_failed(
            &snapshot,
            |_, diagnostics| {
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            },
            |_, diagnostics| {
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            },
        );
        assert!(!hir_failed, "workspace fixture should lower cleanly");
        assert!(
            !validation_failed,
            "workspace fixture should validate cleanly"
        );
        let lowered = snapshot.entry_hir();
        prepare_run_artifact(&snapshot.sources, lowered.module(), &[], requested_view)
    }

    fn prepare_run_from_path(
        path: &Path,
        requested_view: Option<&str>,
    ) -> Result<super::RunArtifact, String> {
        let snapshot = WorkspaceHirSnapshot::load(path)?;
        assert!(
            !super::workspace_syntax_failed(&snapshot, |_, diagnostics| diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)),
            "workspace fixture should parse cleanly"
        );
        let (hir_failed, validation_failed) = super::workspace_hir_failed(
            &snapshot,
            |_, diagnostics| {
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            },
            |_, diagnostics| {
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            },
        );
        assert!(!hir_failed, "workspace fixture should lower cleanly");
        assert!(
            !validation_failed,
            "workspace fixture should validate cleanly"
        );
        let lowered = snapshot.entry_hir();
        prepare_run_artifact(&snapshot.sources, lowered.module(), &[], requested_view)
    }

    #[test]
    fn resolve_run_entrypoint_prefers_explicit_path_over_implicit_workspace_main() {
        let workspace = TempDir::new("run-entry-explicit");
        workspace.write("aivi.toml", "");
        let cwd = workspace.path().join("tooling");
        fs::create_dir_all(&cwd).expect("tooling directory should exist");
        let explicit = workspace.write("apps/demo.aivi", "value demo = 1\n");

        let resolved = super::resolve_run_entrypoint(&cwd, Some(&explicit))
            .expect("explicit path should bypass implicit resolution");

        assert_eq!(resolved.entry_path, explicit);
    }

    #[test]
    fn resolve_run_entrypoint_uses_workspace_root_main_when_present() {
        let workspace = TempDir::new("run-entry-implicit");
        workspace.write("aivi.toml", "");
        let expected = workspace.write("main.aivi", "value view = <Window title=\"AIVI\" />\n");
        let cwd = workspace.path().join("tooling/nested");
        fs::create_dir_all(&cwd).expect("nested tooling directory should exist");

        let resolved = super::resolve_run_entrypoint(&cwd, None)
            .expect("implicit resolution should use workspace-root main.aivi");

        assert_eq!(resolved.entry_path, expected);
    }

    #[test]
    fn resolve_run_entrypoint_reports_missing_implicit_main_with_path_hint() {
        let workspace = TempDir::new("run-entry-missing");
        workspace.write("aivi.toml", "");
        let cwd = workspace.path().join("tooling");
        fs::create_dir_all(&cwd).expect("tooling directory should exist");

        let error = super::resolve_run_entrypoint(&cwd, None)
            .expect_err("missing main.aivi should fail without guessing");

        assert!(error.contains("failed to resolve entrypoint for `aivi run`"));
        assert!(error.contains(&workspace.path().join("main.aivi").display().to_string()));
        assert!(error.contains("--path <entry-file>") || error.contains("aivi.toml"));
    }

    fn execute_workspace(
        path: &Path,
        context: SourceProviderContext,
    ) -> (ExitCode, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_file_with_context(path, context, &mut stdout, &mut stderr)
            .expect("execute should return an exit code");
        (
            code,
            String::from_utf8(stdout).expect("stdout should stay UTF-8 in tests"),
            String::from_utf8(stderr).expect("stderr should stay UTF-8 in tests"),
        )
    }

    fn test_workspace(path: &Path, context: SourceProviderContext) -> (ExitCode, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = test_file_with_context(path, context, &mut stdout, &mut stderr)
            .expect("test should return an exit code");
        (
            code,
            String::from_utf8(stdout).expect("stdout should stay UTF-8 in tests"),
            String::from_utf8(stderr).expect("stderr should stay UTF-8 in tests"),
        )
    }

    fn control_window_source() -> &'static str {
        r#"
type Item = {
    id: Int,
    title: Text
}

type Screen =
  | Loading
  | Ready (List Item)
  | Failed Text

value view =
    <Window title="Users">
        <show when={True} keepMounted={True}>
            <with value={Ready [
                { id: 1, title: "Alpha" },
                { id: 2, title: "Beta" }
            ]} as={currentScreen}>
                <match on={currentScreen}>
                    <case pattern={Loading}>
                        <Label text="Loading..." />
                    </case>
                    <case pattern={Ready items}>
                        <each of={items} as={item} key={item.id}>
                            <Label text={item.title} />
                            <empty>
                                <Label text="No items" />
                            </empty>
                        </each>
                    </case>
                    <case pattern={Failed reason}>
                        <Label text="Error {reason}" />
                    </case>
                </match>
            </with>
        </show>
    </Window>
"#
    }

    fn planner_window_source() -> &'static str {
        r#"
value view =
    <Window title="Users">
        <show when={True} keepMounted={True}>
            <with value={"Alpha"} as={label}>
                <Label text={label} />
                <Label text="Ready" />
                <fragment>
                    <Label text="{label}" />
                </fragment>
            </with>
        </show>
    </Window>
"#
    }

    #[test]
    fn check_accepts_milestone_two_valid_fixtures() {
        for path in [
            "milestone-2/valid/source-decorator-signals/main.aivi",
            "milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi",
        ] {
            let result = check_file(&fixture(path), false).expect("check should run");
            assert_eq!(result, ExitCode::SUCCESS, "expected {path} to pass");
        }
    }

    #[test]
    fn check_rejects_milestone_two_invalid_fixture() {
        let result = check_file(&fixture("milestone-2/invalid/unknown-decorator/main.aivi"), false)
            .expect("check should run");
        assert_eq!(result, ExitCode::FAILURE);
    }

    #[test]
    fn prepare_run_accepts_a_single_static_window_view() {
        let artifact = prepare_run_from_text(
            "static-window.aivi",
            r#"
value screenView =
    <Window title="AIVI" />
"#,
            None,
        )
        .expect("static window markup should be runnable");
        assert_eq!(artifact.view_name.as_ref(), "screenView");
        let root = artifact.bridge.root_node();
        let GtkBridgeNodeKind::Widget(widget) = &root.kind else {
            panic!("expected a root widget, found {:?}", root.kind.tag());
        };
        assert_eq!(widget.widget.segments().last().text(), "Window");
    }

    #[test]
    fn prepare_run_accepts_workspace_type_imports() {
        let workspace = TempDir::new("workspace-run");
        workspace.write(
            "main.aivi",
            r#"
use shared.types (
    Greeting
)

type Welcome = Greeting

value view =
    <Window title="Workspace" />
"#,
        );
        workspace.write(
            "shared/types.aivi",
            r#"
type Greeting = Text
type Farewell = Text

export (Greeting, Farewell)
"#,
        );

        let artifact = prepare_run_from_workspace(&workspace, "main.aivi", None)
            .expect("workspace run preparation should resolve imported types");
        assert_eq!(artifact.view_name.as_ref(), "view");
    }

    #[test]
    fn prepare_run_accepts_snake_demo() {
        let artifact = prepare_run_from_path(&repo_path("demos/snake.aivi"), None)
            .expect("snake demo should prepare for run");
        assert_eq!(artifact.view_name.as_ref(), "main");
        let root = artifact.bridge.root_node();
        let GtkBridgeNodeKind::Widget(widget) = &root.kind else {
            panic!("expected a root widget, found {:?}", root.kind.tag());
        };
        assert_eq!(widget.widget.segments().last().text(), "Window");
        let required = artifact
            .required_signal_globals
            .values()
            .map(|name| name.as_ref())
            .collect::<Vec<_>>();
        assert!(required.contains(&"boardText"));
        assert!(required.contains(&"scoreLine"));
        assert!(required.contains(&"statusLine"));
        assert!(required.contains(&"dirLine"));
    }

    #[test]
    fn run_hydration_waits_for_required_signal_snapshots() {
        let artifact = prepare_run_from_path(&repo_path("demos/snake.aivi"), None)
            .expect("snake demo should prepare for run");
        assert!(
            !run_hydration_globals_ready(&artifact.required_signal_globals, &BTreeMap::new()),
            "empty runtime globals must not be treated as ready for snake hydration"
        );

        let globals = artifact
            .required_signal_globals
            .keys()
            .copied()
            .map(|item| {
                (
                    item,
                    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Text("ready".into())),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(
            run_hydration_globals_ready(&artifact.required_signal_globals, &globals),
            "hydration should proceed once every compiled signal dependency has a snapshot"
        );
    }

    #[test]
    fn prepare_run_prefers_named_view_when_present() {
        let artifact = prepare_run_from_text(
            "named-view.aivi",
            r#"
value view =
    <Window title="Default" />

value alternate =
    <Window title="Alternate" />
"#,
            None,
        )
        .expect("default `view` should win when multiple markup values exist");
        assert_eq!(artifact.view_name.as_ref(), "view");
    }

    #[test]
    fn prepare_run_accepts_dynamic_properties() {
        let artifact = prepare_run_from_text(
            "dynamic-property.aivi",
            r#"
value title = "AIVI"

value view =
    <Window title={title} />
        "#,
            None,
        )
        .expect("dynamic setters should compile for live run hydration");
        let root = artifact.bridge.root_node();
        let GtkBridgeNodeKind::Widget(widget) = &root.kind else {
            panic!("expected a root widget, found {:?}", root.kind.tag());
        };
        assert!(widget.properties.iter().any(|property| {
            matches!(
                property,
                RuntimePropertyBinding::Setter(setter) if setter.name.text() == "title"
            )
        }));
        assert!(!artifact.hydration_inputs.is_empty());
    }

    #[test]
    fn prepare_run_accepts_control_nodes() {
        let artifact = prepare_run_from_text(
            "control-node.aivi",
            r#"
value view =
    <Window title="AIVI">
        <show when={True}>
            <Label text="Visible" />
        </show>
    </Window>
        "#,
            None,
        )
        .expect("control nodes should compile for live run hydration");
        assert!(
            artifact
                .bridge
                .nodes()
                .iter()
                .any(|node| matches!(node.kind, GtkBridgeNodeKind::Show(_)))
        );
    }

    #[test]
    fn prepare_run_collects_fine_grained_runtime_inputs() {
        let artifact = prepare_run_from_text("control-window.aivi", control_window_source(), None)
            .expect("control window should compile for live run hydration");
        let root = artifact.bridge.root_node();
        let GtkBridgeNodeKind::Widget(window) = &root.kind else {
            panic!("expected a window root, found {:?}", root.kind.tag());
        };
        let show_ref = window.default_children.roots[0];
        let show = artifact
            .bridge
            .node(show_ref.plan)
            .expect("show child should exist in the bridge");
        let GtkBridgeNodeKind::Show(show) = &show.kind else {
            panic!("expected a show node, found {:?}", show.kind.tag());
        };
        assert!(artifact.hydration_inputs.contains_key(&show.when.input));
        let RuntimeShowMountPolicy::KeepMounted { decision } = &show.mount else {
            panic!("expected keepMounted input");
        };
        assert!(artifact.hydration_inputs.contains_key(&decision.input));

        let with_ref = show.body.roots[0];
        let with_node = artifact
            .bridge
            .node(with_ref.plan)
            .expect("with child should exist in the bridge");
        let GtkBridgeNodeKind::With(with_node) = &with_node.kind else {
            panic!("expected a with node, found {:?}", with_node.kind.tag());
        };
        assert!(
            artifact
                .hydration_inputs
                .contains_key(&with_node.value.input)
        );

        let match_ref = with_node.body.roots[0];
        let match_node = artifact
            .bridge
            .node(match_ref.plan)
            .expect("match child should exist in the bridge");
        let GtkBridgeNodeKind::Match(match_node) = &match_node.kind else {
            panic!("expected a match node, found {:?}", match_node.kind.tag());
        };
        assert!(
            artifact
                .hydration_inputs
                .contains_key(&match_node.scrutinee.input)
        );

        let ready_case = &match_node.cases[1];
        let ready_case = artifact
            .bridge
            .node(ready_case.case.plan)
            .expect("ready case should exist in the bridge");
        let GtkBridgeNodeKind::Case(ready_case) = &ready_case.kind else {
            panic!("expected a case node, found {:?}", ready_case.kind.tag());
        };
        let each_ref = ready_case.body.roots[0];
        let each_node = artifact
            .bridge
            .node(each_ref.plan)
            .expect("each child should exist in the bridge");
        let GtkBridgeNodeKind::Each(each_node) = &each_node.kind else {
            panic!("expected an each node, found {:?}", each_node.kind.tag());
        };
        assert!(
            artifact
                .hydration_inputs
                .contains_key(&each_node.collection.input)
        );
        let key_input = each_node
            .key_input
            .as_ref()
            .expect("keyed each nodes should expose a runtime key input");
        assert!(artifact.hydration_inputs.contains_key(&key_input.input));
        assert_eq!(artifact.hydration_inputs.len(), 8);
    }

    #[test]
    fn run_hydration_planner_precomputes_control_and_setter_updates_off_thread() {
        let artifact = prepare_run_from_text("planner-window.aivi", planner_window_source(), None)
            .expect("planner window should compile for live run hydration");
        let shared = RunHydrationStaticState {
            view_name: artifact.view_name.clone(),
            module: artifact.module.clone(),
            bridge: artifact.bridge.clone(),
            inputs: artifact.hydration_inputs.clone(),
        };
        let plan = plan_run_hydration(&shared, &BTreeMap::new())
            .expect("inline planner window should plan without runtime globals");

        let HydratedRunNode::Widget { children, .. } = &plan.root else {
            panic!("expected a window hydration root");
        };
        let [
            HydratedRunNode::Show {
                when,
                keep_mounted,
                children,
                ..
            },
        ] = children.as_ref()
        else {
            panic!("expected a single show child under the window root");
        };
        assert!(*when);
        assert!(*keep_mounted);

        let [HydratedRunNode::With { children, .. }] = children.as_ref() else {
            panic!("expected a single with child inside the show body");
        };
        let [
            HydratedRunNode::Widget {
                properties: alpha_props,
                ..
            },
            HydratedRunNode::Widget {
                properties: ready_props,
                ..
            },
            HydratedRunNode::Fragment {
                children: fragment_children,
                ..
            },
        ] = children.as_ref()
        else {
            panic!("expected the with body to contain two labels and one fragment");
        };
        assert_eq!(alpha_props.len(), 1);
        assert_eq!(alpha_props[0].value, RuntimeValue::Text("Alpha".into()));
        assert!(ready_props.is_empty());

        let [
            HydratedRunNode::Widget {
                properties: fragment_props,
                ..
            },
        ] = fragment_children.as_ref()
        else {
            panic!("expected the fragment child to contain one label widget");
        };
        assert_eq!(fragment_props.len(), 1);
        assert_eq!(fragment_props[0].value, RuntimeValue::Text("Alpha".into()));
    }

    #[test]
    fn prepare_run_accepts_direct_signal_event_hooks() {
        let artifact = prepare_run_from_text(
            "event-hook.aivi",
            r#"
signal click : Signal Unit

value view =
    <Window title="Host">
        <Button label="Save" onClick={click} />
    </Window>
"#,
            None,
        )
        .expect("event hooks should resolve when they target direct input signals");
        let widget = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Button" =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should keep the button widget");
        let handler = widget
            .event_hooks
            .first()
            .expect("button should keep one event hook")
            .handler;
        assert!(artifact.event_handlers.contains_key(&handler));
    }

    #[test]
    fn prepare_run_accepts_signal_payload_event_hooks_with_markup_bindings() {
        let artifact = prepare_run_from_text(
            "event-hook-payload.aivi",
            r#"
signal selected : Signal Text
signal selectedText : Signal Text = selected
 +|> "None" keepLatest

type Text -> Text -> Text
func keepLatest = next current=>    next

value rows = ["Alpha", "Beta"]

value view =
    <Window title="Host">
        <Box>
            <Label text={selectedText} />
            <each of={rows} as={item} key={item}>
                <Button label={item} onClick={selected item} />
            </each>
        </Box>
    </Window>
"#,
            None,
        )
        .expect("event hooks should accept direct signal payload expressions from markup bindings");
        let widget = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Button" =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should keep the button widget template");
        let handler = widget
            .event_hooks
            .first()
            .expect("button should keep one event hook");
        assert!(artifact.event_handlers.contains_key(&handler.handler));
        assert!(artifact.hydration_inputs.contains_key(&handler.input));
        assert!(matches!(
            artifact.event_handlers.get(&handler.handler),
            Some(ResolvedRunEventHandler {
                payload: ResolvedRunEventPayload::ScopedInput,
                ..
            })
        ));
    }

    #[test]
    fn prepare_run_accepts_with_bindings_from_signal_payloads() {
        let artifact = prepare_run_from_text(
            "with-signal-payload.aivi",
            r#"
type Screen = {
    title: Text
}

signal screen : Signal Screen

value view =
    <Window title="Host">
        <with value={screen} as={currentScreen}>
            <Label text={currentScreen.title} />
        </with>
    </Window>
"#,
            None,
        )
        .expect("with bindings should expose the current payload of signal expressions");
        let root = artifact
            .bridge
            .node(artifact.bridge.root().plan)
            .expect("window root should exist in the bridge");
        let GtkBridgeNodeKind::Widget(window) = &root.kind else {
            panic!("expected a widget root, found {:?}", root.kind.tag());
        };
        let with_ref = window.default_children.roots[0];
        let with_node = artifact
            .bridge
            .node(with_ref.plan)
            .expect("window child should exist");
        let GtkBridgeNodeKind::With(with_node) = &with_node.kind else {
            panic!("expected a with child, found {:?}", with_node.kind.tag());
        };
        let label_ref = with_node.body.roots[0];
        let label_node = artifact
            .bridge
            .node(label_ref.plan)
            .expect("label child should exist");
        let GtkBridgeNodeKind::Widget(label) = &label_node.kind else {
            panic!("expected a label widget, found {:?}", label_node.kind.tag());
        };
        let text_input = label
            .properties
            .iter()
            .find_map(|property| match property {
                RuntimePropertyBinding::Setter(binding) if binding.name.text() == "text" => {
                    Some(binding.input)
                }
                _ => None,
            })
            .expect("label text should stay dynamic under the with binding");
        assert!(
            artifact
                .hydration_inputs
                .contains_key(&with_node.value.input)
        );
        assert!(artifact.hydration_inputs.contains_key(&text_input));
    }

    #[test]
    fn prepare_run_accepts_expanded_widget_catalog_entries() {
        let artifact = prepare_run_from_text(
            "expanded-widget-catalog.aivi",
            r#"
signal submit : Signal Unit

value entryText = "Draft"
value canEdit = False
value isEnabled = True
value view =
    <Window title="Host">
        <ScrolledWindow>
            <Box>
                <Entry text={entryText} placeholderText="Search" editable={canEdit} onActivate={submit} />
                <Switch active={isEnabled} />
            </Box>
        </ScrolledWindow>
    </Window>
"#,
            None,
        )
        .expect("expanded widget catalog entries should validate and prepare for run");
        let widget_names = artifact
            .bridge
            .nodes()
            .iter()
            .filter_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget) => {
                    Some(widget.widget.segments().last().text().to_owned())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(widget_names.iter().any(|name| name == "ScrolledWindow"));
        assert!(widget_names.iter().any(|name| name == "Entry"));
        assert!(widget_names.iter().any(|name| name == "Switch"));

        let entry = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Entry" =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should keep the entry widget");
        let handler = entry
            .event_hooks
            .first()
            .expect("entry should keep one activation event hook")
            .handler;
        assert!(artifact.event_handlers.contains_key(&handler));
    }

    #[test]
    fn prepare_run_accepts_entry_change_text_events() {
        let artifact = prepare_run_from_text(
            "entry-change-events.aivi",
            r#"
signal changed : Signal Text

value query = "Draft"
value view =
    <Window title="Host">
        <Entry text={query} onChange={changed} />
    </Window>
"#,
            None,
        )
        .expect("entry text change events should validate and prepare for run");
        let entry = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Entry" =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should keep the entry widget");
        assert_eq!(entry.event_hooks.len(), 1);
        let handler = entry.event_hooks[0].handler;
        assert!(artifact.event_handlers.contains_key(&handler));
    }

    #[test]
    fn prepare_run_accepts_additional_common_widgets_and_switch_toggle_events() {
        let artifact = prepare_run_from_text(
            "additional-widget-catalog.aivi",
            r#"
signal toggled : Signal Bool

value showButtons = False
value isEnabled = True
value view =
    <Window title="Host">
        <Viewport>
            <Frame label="Controls">
                <Box>
                    <HeaderBar showTitleButtons={showButtons}>
                        <HeaderBar.titleWidget>
                            <Label text="Profile" />
                        </HeaderBar.titleWidget>
                    </HeaderBar>
                    <Separator orientation="Horizontal" />
                    <Switch active={isEnabled} onToggle={toggled} />
                </Box>
            </Frame>
        </Viewport>
    </Window>
"#,
            None,
        )
        .expect("additional common widgets should validate and prepare for run");
        let widget_names = artifact
            .bridge
            .nodes()
            .iter()
            .filter_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget) => {
                    Some(widget.widget.segments().last().text().to_owned())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(widget_names.iter().any(|name| name == "Viewport"));
        assert!(widget_names.iter().any(|name| name == "Frame"));
        assert!(widget_names.iter().any(|name| name == "HeaderBar"));
        assert!(widget_names.iter().any(|name| name == "Separator"));
        assert!(widget_names.iter().any(|name| name == "Switch"));

        let switch = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Switch" =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should keep the switch widget");
        let handler = switch
            .event_hooks
            .first()
            .expect("switch should keep one toggle event hook")
            .handler;
        assert!(artifact.event_handlers.contains_key(&handler));
    }

    #[test]
    fn prepare_run_accepts_named_child_groups_for_paned_and_header_bar() {
        let artifact = prepare_run_from_text(
            "named-child-groups.aivi",
            r#"
value showButtons = False
value view =
    <Window title="Host">
        <Paned orientation="Horizontal">
            <Paned.start>
                <Label text="Primary" />
            </Paned.start>
            <Paned.end>
                <HeaderBar showTitleButtons={showButtons}>
                    <HeaderBar.start>
                        <Button label="Back" />
                    </HeaderBar.start>
                    <HeaderBar.titleWidget>
                        <Label text="Inbox" />
                    </HeaderBar.titleWidget>
                    <HeaderBar.end>
                        <Button label="More" />
                    </HeaderBar.end>
                </HeaderBar>
            </Paned.end>
        </Paned>
    </Window>
"#,
            None,
        )
        .expect("named child groups should prepare successfully for run");

        let groups = artifact
            .bridge
            .nodes()
            .iter()
            .filter_map(|node| match &node.kind {
                GtkBridgeNodeKind::Group(group) => Some((
                    group.widget.segments().last().text().to_owned(),
                    group.descriptor.name.to_owned(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "Paned" && group == "start")
        );
        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "Paned" && group == "end")
        );
        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "HeaderBar" && group == "start")
        );
        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "HeaderBar" && group == "titleWidget")
        );
        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "HeaderBar" && group == "end")
        );
    }

    #[test]
    fn prepare_run_accepts_window_titlebars_and_compact_button_properties() {
        let artifact = prepare_run_from_text(
            "window-titlebar-and-button-props.aivi",
            r#"
value showButtons = True
value view =
    <Window title="Host">
        <Window.titlebar>
            <HeaderBar showTitleButtons={showButtons}>
                <HeaderBar.start>
                    <Label text="Status" />
                </HeaderBar.start>
                <HeaderBar.end>
                    <Button label="Restart" compact hasFrame={False} widthRequest={26} heightRequest={26} />
                </HeaderBar.end>
            </HeaderBar>
        </Window.titlebar>
        <Button label="A" compact hasFrame={False} widthRequest={26} heightRequest={26} />
    </Window>
"#,
            None,
        )
        .expect("window titlebars and compact button properties should prepare successfully");

        let groups = artifact
            .bridge
            .nodes()
            .iter()
            .filter_map(|node| match &node.kind {
                GtkBridgeNodeKind::Group(group) => Some((
                    group.widget.segments().last().text().to_owned(),
                    group.descriptor.name.to_owned(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(
            groups
                .iter()
                .any(|(widget, group)| widget == "Window" && group == "titlebar")
        );

        let restart = artifact
            .bridge
            .nodes()
            .iter()
            .find_map(|node| match &node.kind {
                GtkBridgeNodeKind::Widget(widget)
                    if widget.widget.segments().last().text() == "Button"
                        && widget
                            .properties
                            .iter()
                            .any(|property| property.name().text() == "label")
                        && widget.event_hooks.is_empty() =>
                {
                    Some(widget)
                }
                _ => None,
            })
            .expect("bridge should retain a content button widget");
        let property_names = restart
            .properties
            .iter()
            .map(|property| property.name().text().to_owned())
            .collect::<Vec<_>>();
        assert!(property_names.iter().any(|name| name == "compact"));
        assert!(property_names.iter().any(|name| name == "hasFrame"));
        assert!(property_names.iter().any(|name| name == "widthRequest"));
        assert!(property_names.iter().any(|name| name == "heightRequest"));
    }

    #[test]
    fn prepare_run_rejects_non_window_root_widgets() {
        let error = prepare_run_from_text(
            "button-root.aivi",
            r#"
value view =
    <Button label="Save" />
"#,
            None,
        )
        .expect_err("non-window roots should be rejected before launch");
        assert!(error.contains("reachable root widgets"));
        assert!(error.contains("Window"));
    }

    #[test]
    fn prepare_run_rejects_unsupported_widget_catalog_entries() {
        let error = prepare_run_from_text(
            "unsupported-widget.aivi",
            r#"
value view =
    <Window title="Host">
        <Notebook />
    </Window>
"#,
            None,
        )
        .expect_err("widgets outside the schema catalog should be rejected before launch");
        assert!(error.contains("Notebook"));
    }

    #[test]
    fn prepare_run_rejects_child_widgets_on_leaf_widgets() {
        let error = prepare_run_from_text(
            "leaf-children.aivi",
            r#"
value view =
    <Window title="Host">
        <Button label="Save">
            <Label text="Nested" />
        </Button>
    </Window>
"#,
            None,
        )
        .expect_err("leaf widgets should reject child markup from schema validation");
        assert!(error.contains("does not support child widgets under `Button`"));
    }

    #[test]
    fn prepare_run_rejects_multiple_window_children() {
        let error = prepare_run_from_text(
            "window-too-many-children.aivi",
            r#"
value view =
    <Window title="Host">
        <Label text="First" />
        <Label text="Second" />
    </Window>
"#,
            None,
        )
        .expect_err("single-child window content should be validated before launch");
        assert!(error.contains("group `content`"));
        assert!(error.contains("allows at most 1"));
    }

    #[test]
    fn prepare_run_rejects_multiple_scrolled_window_children() {
        let error = prepare_run_from_text(
            "scrolled-window-too-many-children.aivi",
            r#"
value view =
    <Window title="Host">
        <ScrolledWindow>
            <Label text="First" />
            <Label text="Second" />
        </ScrolledWindow>
    </Window>
"#,
            None,
        )
        .expect_err("single-child scrolled window content should be validated before launch");
        assert!(error.contains("ScrolledWindow"));
        assert!(error.contains("group `content`"));
        assert!(error.contains("allows at most 1"));
    }

    #[test]
    fn prepare_run_rejects_unnamed_header_bar_children() {
        let error = prepare_run_from_text(
            "header-bar-unnamed-child.aivi",
            r#"
value view =
    <Window title="Host">
        <HeaderBar>
            <Label text="Profile" />
        </HeaderBar>
    </Window>
"#,
            None,
        )
        .expect_err("multi-slot header bars should require explicit child-group wrappers");
        assert!(error.contains("cannot place unnamed children under `HeaderBar`"));
        assert!(error.contains("multiple child groups"));
    }

    #[test]
    fn prepare_run_rejects_event_payload_mismatch() {
        let error = prepare_run_from_text(
            "event-payload-mismatch.aivi",
            r#"
signal click : Signal Int

value view =
    <Window title="Host">
        <Button label="Save" onClick={click} />
    </Window>
"#,
            None,
        )
        .expect_err("button clicks should require Signal Unit handlers");
        assert!(error.contains("Signal Unit"));
        assert!(error.contains("onClick"));
    }

    #[test]
    fn prepare_run_rejects_explicit_event_payload_type_mismatch() {
        let error = prepare_run_from_text(
            "event-explicit-payload-mismatch.aivi",
            r#"
signal click : Signal Int

value view =
    <Window title="Host">
        <Button label="Save" onClick={click "wrong"} />
    </Window>
"#,
            None,
        )
        .expect_err("explicit event payloads should match the target signal payload type");
        assert!(error.contains("explicit payload"));
        assert!(error.contains("Text"));
        assert!(error.contains("Int"));
    }

    #[test]
    fn prepare_run_requires_view_name_when_multiple_markup_values_exist() {
        let error = prepare_run_from_text(
            "multiple-views.aivi",
            r#"
value first =
    <Window title="First" />

value second =
    <Window title="Second" />
"#,
            None,
        )
        .expect_err("multiple unnamed markup views should require `--view`");
        assert!(error.contains("--view <name>"));
    }

    #[test]
    fn test_command_discovers_workspace_tests_and_applies_mock_overrides() {
        let workspace = TempDir::new("workspace-tests");
        let entry = workspace.write(
            "main.aivi",
            r#"
use util (
    probe
)
use aivi.fs (
    exists
)

@source process.cwd
signal cwd : Signal Text

fun mockedProbe:Task Text Bool = path:Text=>    exists "{cwd}/flag.txt"

@test
@mock(probe, mockedProbe)
value mocked_exists : Task Text Bool =
    probe "missing.txt"
"#,
        );
        workspace.write(
            "util.aivi",
            r#"
use aivi.fs (
    exists
)

@source process.cwd
signal cwd : Signal Text

fun probe:Task Text Bool = path:Text=>    exists path

@test
value service_smoke : Task Text Bool =
    exists "{cwd}/flag.txt"
"#,
        );
        fs::write(workspace.path().join("flag.txt"), "ok")
            .expect("test fixture should be writable");

        let (code, stdout, stderr) = test_workspace(
            &entry,
            SourceProviderContext::new(Vec::new(), workspace.path().to_path_buf(), BTreeMap::new()),
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            stderr.is_empty(),
            "stderr should stay empty, found {stderr:?}"
        );
        assert!(stdout.contains("ok   "));
        assert!(stdout.contains("util.aivi"));
        assert!(stdout.contains("mocked_exists"));
        assert!(stdout.contains("service_smoke"));
        assert!(stdout.contains("test result: ok. 2 passed; 0 failed; 2 total"));
    }

    #[test]
    fn production_entrypoint_selection_ignores_test_declarations() {
        let execute_workspace = TempDir::new("execute-test-entry");
        let execute_entry = execute_workspace.write(
            "main.aivi",
            r#"
use aivi.stdio (
    stderrWrite
)

@test
value main : Task Text Unit =
    stderrWrite "hidden"
"#,
        );
        let execute_snapshot =
            WorkspaceHirSnapshot::load(&execute_entry).expect("workspace should load");
        let execute_lowered = execute_snapshot.entry_hir();
        let execute_error = prepare_execute_artifact(execute_lowered.module())
            .expect_err("`aivi execute` should ignore `@test main`");
        assert!(execute_error.contains("no top-level `value main` found"));

        let run_error = prepare_run_from_text(
            "run-test-view.aivi",
            r#"
@test
value view =
    <Window title="Hidden" />
"#,
            None,
        )
        .expect_err("`aivi run` should ignore `@test view`");
        assert!(run_error.contains("no markup view found"));
    }

    #[test]
    fn execute_reads_host_context_sources_and_writes_stdout() {
        let workspace = TempDir::new("execute-context");
        let entry = workspace.write(
            "main.aivi",
            r#"
use aivi.stdio (
    stdoutWrite
)

@source process.args
signal cliArgs : Signal (List Text)

@source process.cwd
signal cwd : Signal Text

@source env.get "ACCESS_TOKEN"
signal token : Signal (Option Text)

@source stdio.read
signal stdinText : Signal Text

@source path.home
signal homeDir : Signal Text

@source path.configHome
signal configHome : Signal Text

@source path.dataHome
signal dataHome : Signal Text

@source path.cacheHome
signal cacheHome : Signal Text

@source path.tempDir
signal tempDir : Signal Text

value main : Task Text Unit =
    stdoutWrite "{cliArgs}|{cwd}|{token}|{stdinText}|{homeDir}|{configHome}|{dataHome}|{cacheHome}|{tempDir}"
"#,
        );
        let cwd = workspace.path().join("working");
        fs::create_dir_all(&cwd).expect("execute cwd should be creatable");
        let home = workspace.path().join("home");
        let config = workspace.path().join("config");
        let data = workspace.path().join("data");
        let cache = workspace.path().join("cache");
        for path in [&home, &config, &data, &cache] {
            fs::create_dir_all(path).expect("context directories should be creatable");
        }
        let context = SourceProviderContext::new(
            vec!["alpha".to_owned(), "beta".to_owned()],
            cwd.clone(),
            BTreeMap::from([
                ("HOME".to_owned(), home.to_string_lossy().into_owned()),
                (
                    "XDG_CONFIG_HOME".to_owned(),
                    config.to_string_lossy().into_owned(),
                ),
                (
                    "XDG_DATA_HOME".to_owned(),
                    data.to_string_lossy().into_owned(),
                ),
                (
                    "XDG_CACHE_HOME".to_owned(),
                    cache.to_string_lossy().into_owned(),
                ),
                ("ACCESS_TOKEN".to_owned(), "secret".to_owned()),
            ]),
        )
        .with_stdin_text("payload");

        let (code, stdout, stderr) = execute_workspace(&entry, context);

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            stderr.is_empty(),
            "stderr should stay empty, found {stderr:?}"
        );
        assert_eq!(
            stdout,
            format!(
                "[alpha, beta]|{}|Some secret|payload|{}|{}|{}|{}|{}",
                cwd.display(),
                home.display(),
                config.display(),
                data.display(),
                cache.display(),
                env::temp_dir().display()
            )
        );
    }

    #[test]
    fn execute_runs_stderr_task_without_touching_stdout() {
        let workspace = TempDir::new("execute-stderr");
        let entry = workspace.write(
            "main.aivi",
            r#"
use aivi.stdio (
    stderrWrite
)

value main : Task Text Unit =
    stderrWrite "problem"
"#,
        );

        let (code, stdout, stderr) = execute_workspace(
            &entry,
            SourceProviderContext::new(Vec::new(), workspace.path().to_path_buf(), BTreeMap::new()),
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            stdout.is_empty(),
            "stdout should stay empty, found {stdout:?}"
        );
        assert_eq!(stderr, "problem");
    }

    #[test]
    fn execute_writes_text_files_relative_to_the_cli_context() {
        let workspace = TempDir::new("execute-write-text");
        let entry = workspace.write(
            "main.aivi",
            r#"
use aivi.fs (
    writeText
)

@source process.cwd
signal cwd : Signal Text

value main : Task Text Unit =
    writeText "{cwd}/out.txt" "hello from execute"
"#,
        );
        let cwd = workspace.path().join("cwd");
        fs::create_dir_all(&cwd).expect("execute cwd should be creatable");

        let (code, stdout, stderr) = execute_workspace(
            &entry,
            SourceProviderContext::new(Vec::new(), cwd.clone(), BTreeMap::new()),
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            stdout.is_empty(),
            "stdout should stay empty, found {stdout:?}"
        );
        assert!(
            stderr.is_empty(),
            "stderr should stay empty, found {stderr:?}"
        );
        assert_eq!(
            fs::read_to_string(cwd.join("out.txt")).expect("text task should create output file"),
            "hello from execute"
        );
    }

    #[test]
    fn execute_creates_and_deletes_filesystem_paths() {
        let workspace = TempDir::new("execute-fs-paths");
        let create_entry = workspace.write(
            "create.aivi",
            r#"
use aivi.fs (
    createDirAll
)

@source process.cwd
signal cwd : Signal Text

value main : Task Text Unit =
    createDirAll "{cwd}/nested/logs"
"#,
        );
        let delete_entry = workspace.write(
            "delete.aivi",
            r#"
use aivi.fs (
    deleteFile
)

@source process.cwd
signal cwd : Signal Text

value main : Task Text Unit =
    deleteFile "{cwd}/remove-me.txt"
"#,
        );
        let cwd = workspace.path().join("cwd");
        fs::create_dir_all(&cwd).expect("execute cwd should be creatable");
        fs::write(cwd.join("remove-me.txt"), "bye").expect("delete fixture should be writable");

        let (create_code, create_stdout, create_stderr) = execute_workspace(
            &create_entry,
            SourceProviderContext::new(Vec::new(), cwd.clone(), BTreeMap::new()),
        );
        assert_eq!(create_code, ExitCode::SUCCESS);
        assert!(create_stdout.is_empty());
        assert!(create_stderr.is_empty());
        assert!(cwd.join("nested/logs").is_dir());

        let (delete_code, delete_stdout, delete_stderr) = execute_workspace(
            &delete_entry,
            SourceProviderContext::new(Vec::new(), cwd.clone(), BTreeMap::new()),
        );
        assert_eq!(delete_code, ExitCode::SUCCESS);
        assert!(delete_stdout.is_empty());
        assert!(delete_stderr.is_empty());
        assert!(!cwd.join("remove-me.txt").exists());
    }

    #[test]
    fn execute_runtime_task_plan_writes_raw_bytes() {
        let workspace = TempDir::new("execute-write-bytes");
        let path = workspace.path().join("blob.bin");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_task_plan(
            RuntimeTaskPlan::FsWriteBytes {
                path: path.to_string_lossy().into_owned().into_boxed_str(),
                bytes: vec![0, 1, 2, 3].into_boxed_slice(),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("write-bytes task should execute");

        assert_eq!(result, RuntimeValue::Unit);
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        assert_eq!(
            fs::read(&path).expect("written bytes should be readable"),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn execute_runtime_task_plan_returns_pure_payload() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = execute_runtime_task_plan(
            RuntimeTaskPlan::Pure {
                value: Box::new(RuntimeValue::Bool(true)),
            },
            &mut stdout,
            &mut stderr,
        )
        .expect("pure task should execute");

        assert_eq!(result, RuntimeValue::Bool(true));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }
}
