#![forbid(unsafe_code)]

use std::{
    cell::Cell,
    env,
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
};

use aivi_backend::{compile_program, lower_module as lower_backend_module, validate_program};
use aivi_base::{Diagnostic, FileId, Severity, SourceDatabase, SourceSpan};
use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
use aivi_gtk::{
    GtkConcreteHost, GtkHostValue, GtkRuntimeExecutor, PlanNodeKind, PropertyPlan,
    lower_markup_expr, lower_widget_bridge,
};
use aivi_hir::{
    ExprKind, Item, Module as HirModule, ValidationMode, ValueItem,
    lower_module as lower_hir_module,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_syntax::{Formatter, ItemKind, TokenKind, lex_module, parse_module};
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

    if first == OsString::from("check") {
        return check_file(&take_path(args)?);
    }

    if first == OsString::from("compile") {
        return run_compile(args);
    }

    if first == OsString::from("run") {
        return run_markup(args);
    }

    if first == OsString::from("lex") {
        return lex_file(&take_path(args)?);
    }

    if first == OsString::from("lsp") {
        return run_lsp(args);
    }

    if first == OsString::from("fmt") {
        return run_fmt(args);
    }

    // Default: treat the first argument as a path and run `check`.
    check_file(&PathBuf::from(first))
}

fn run_fmt(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let Some(next) = args.next() else {
        return Err("expected a path or --stdin/--check argument after `fmt`".to_owned());
    };

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

fn take_path(mut args: impl Iterator<Item = OsString>) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| "expected a path argument".to_owned())
}

fn run_compile(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "expected a path argument after `compile`".to_owned())?;
    let mut output = None;

    while let Some(argument) = args.next() {
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

        return Err(format!(
            "unexpected compile argument `{}`",
            argument.to_string_lossy()
        ));
    }

    compile_file(&path, output.as_deref())
}

fn run_markup(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "expected a path argument after `run`".to_owned())?;
    let mut requested_view = None;

    while let Some(argument) = args.next() {
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

        return Err(format!(
            "unexpected run argument `{}`",
            argument.to_string_lossy()
        ));
    }

    run_markup_file(&path, requested_view.as_deref())
}

fn load_source(path: &Path) -> Result<(SourceDatabase, FileId), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path.to_path_buf(), text);
    Ok((sources, file_id))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RunHostValue;

impl GtkHostValue for RunHostValue {
    fn unit() -> Self {
        Self
    }
}

#[derive(Clone, Debug)]
struct StaticRunArtifact {
    view_name: Box<str>,
    bridge: aivi_gtk::GtkBridgeGraph,
}

#[derive(Clone, Debug)]
struct StaticRunBlocker {
    span: SourceSpan,
    message: String,
}

fn check_file(path: &Path) -> Result<ExitCode, String> {
    let (sources, file_id) = load_source(path)?;
    let file = &sources[file_id];
    let parsed = parse_module(file);
    let syntax_failed = print_diagnostics(&sources, parsed.all_diagnostics());
    if syntax_failed {
        Ok(ExitCode::FAILURE)
    } else {
        let lowered = lower_hir_module(&parsed.module);
        let lowering_failed = print_diagnostics(&sources, lowered.diagnostics());
        let validation_mode = if lowering_failed {
            ValidationMode::Structural
        } else {
            ValidationMode::RequireResolvedNames
        };
        let validation = lowered.module().validate(validation_mode);
        let validation_failed = print_diagnostics(&sources, validation.diagnostics());
        if lowering_failed || validation_failed {
            return Ok(ExitCode::FAILURE);
        }
        println!(
            "syntax + HIR passed: {} ({} item{})",
            path.display(),
            parsed.module.items.len(),
            if parsed.module.items.len() == 1 {
                ""
            } else {
                "s"
            }
        );
        Ok(ExitCode::SUCCESS)
    }
}

fn run_markup_file(path: &Path, requested_view: Option<&str>) -> Result<ExitCode, String> {
    let (sources, file_id) = load_source(path)?;
    let file = &sources[file_id];
    let parsed = parse_module(file);
    let syntax_failed = print_diagnostics(&sources, parsed.all_diagnostics());
    if syntax_failed {
        return Ok(ExitCode::FAILURE);
    }

    let lowered = lower_hir_module(&parsed.module);
    let hir_lowering_failed = print_diagnostics(&sources, lowered.diagnostics());
    let validation_mode = if hir_lowering_failed {
        ValidationMode::Structural
    } else {
        ValidationMode::RequireResolvedNames
    };
    let validation = lowered.module().validate(validation_mode);
    let hir_validation_failed = print_diagnostics(&sources, validation.diagnostics());
    if hir_lowering_failed || hir_validation_failed {
        return Ok(ExitCode::FAILURE);
    }

    let artifact = match prepare_static_run_artifact(&sources, lowered.module(), requested_view) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("{message}");
            return Ok(ExitCode::FAILURE);
        }
    };

    launch_static_run(path, artifact)
}

fn prepare_static_run_artifact(
    sources: &SourceDatabase,
    module: &HirModule,
    requested_view: Option<&str>,
) -> Result<StaticRunArtifact, String> {
    let view = select_run_view(module, requested_view)?;
    let ExprKind::Markup(_) = &module.exprs()[view.body].kind else {
        return Err(format!(
            "run view `{}` is not markup; `aivi run` currently requires a top-level markup-valued `val`",
            view.name.text()
        ));
    };
    let plan = lower_markup_expr(module, view.body).map_err(|error| {
        format!(
            "failed to lower run view `{}` into GTK markup: {error}",
            view.name.text()
        )
    })?;
    validate_static_run_plan(sources, &plan)?;
    let bridge = lower_widget_bridge(&plan).map_err(|error| {
        format!(
            "failed to lower static run view `{}` into a GTK bridge graph: {error}",
            view.name.text()
        )
    })?;
    Ok(StaticRunArtifact {
        view_name: view.name.text().into(),
        bridge,
    })
}

fn select_run_view<'a>(
    module: &'a HirModule,
    requested_view: Option<&str>,
) -> Result<&'a ValueItem, String> {
    let mut markup_values = Vec::new();
    let mut all_values = Vec::new();
    for (_, item) in module.items().iter() {
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
                    "run view `{requested_view}` does not exist and this module exposes no markup-valued top-level `val`s"
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
                "run view `{requested_view}` exists but is not markup; `aivi run` currently requires a markup-valued top-level `val`"
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
        [] => Err("no markup view found; define `val view = <Window ...>` or pass `--view <name>` for another markup-valued top-level `val`".to_owned()),
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

fn validate_static_run_plan(
    sources: &SourceDatabase,
    plan: &aivi_gtk::WidgetPlan,
) -> Result<(), String> {
    let mut blockers = Vec::new();
    for node in plan.nodes() {
        match &node.kind {
            PlanNodeKind::Widget(widget) => {
                for property in &widget.properties {
                    if let PropertyPlan::Setter(setter) = property {
                        blockers.push(StaticRunBlocker {
                            span: setter.site.span,
                            message: format!(
                                "dynamic property `{}` is not supported yet",
                                setter.name.text()
                            ),
                        });
                    }
                }
                for event in &widget.event_hooks {
                    blockers.push(StaticRunBlocker {
                        span: event.site.span,
                        message: format!("event hook `{}` is not supported yet", event.name.text()),
                    });
                }
            }
            PlanNodeKind::Show(_) => blockers.push(StaticRunBlocker {
                span: node.span,
                message: "`<show>` control nodes are not supported yet".to_owned(),
            }),
            PlanNodeKind::Each(_) => blockers.push(StaticRunBlocker {
                span: node.span,
                message: "`<each>` control nodes are not supported yet".to_owned(),
            }),
            PlanNodeKind::Match(_) => blockers.push(StaticRunBlocker {
                span: node.span,
                message: "`<match>` control nodes are not supported yet".to_owned(),
            }),
            PlanNodeKind::With(_) => blockers.push(StaticRunBlocker {
                span: node.span,
                message: "`<with>` control nodes are not supported yet".to_owned(),
            }),
            PlanNodeKind::Empty(_) | PlanNodeKind::Case(_) | PlanNodeKind::Fragment(_) => {}
        }
    }

    if blockers.is_empty() {
        return Ok(());
    }

    let mut rendered = String::from(
        "`aivi run` currently supports static GTK markup only. Unsupported features in the selected view:\n",
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

fn launch_static_run(path: &Path, artifact: StaticRunArtifact) -> Result<ExitCode, String> {
    gtk::init()
        .map_err(|error| format!("failed to initialize GTK for {}: {error}", path.display()))?;

    let executor =
        GtkRuntimeExecutor::new(artifact.bridge, GtkConcreteHost::<RunHostValue>::default())
            .map_err(|error| {
                format!(
                    "failed to mount static GTK view `{}` from {}: {error}",
                    artifact.view_name,
                    path.display()
                )
            })?;
    let root_handles = executor.root_widgets().map_err(|error| {
        format!(
            "failed to collect root widgets for static view `{}`: {error}",
            artifact.view_name
        )
    })?;
    if root_handles.is_empty() {
        return Err(format!(
            "run view `{}` did not produce any root GTK widgets",
            artifact.view_name
        ));
    }

    let root_windows = root_handles
        .into_iter()
        .map(|handle| {
            let widget = executor.host().widget(&handle).ok_or_else(|| {
                format!(
                    "run view `{}` lost GTK root widget {:?} before presentation",
                    artifact.view_name, handle
                )
            })?;
            widget.clone().downcast::<gtk::Window>().map_err(|widget| {
                format!(
                    "`aivi run` currently requires top-level `Window` roots; view `{}` produced a root `{}`",
                    artifact.view_name,
                    widget.type_().name()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    println!(
        "running static GTK view `{}` from {}",
        artifact.view_name,
        path.display()
    );

    let main_loop = glib::MainLoop::new(None, false);
    let remaining = Rc::new(Cell::new(root_windows.len()));
    for window in &root_windows {
        let main_loop = main_loop.clone();
        let remaining = remaining.clone();
        window.connect_close_request(move |_| {
            let next = remaining.get().saturating_sub(1);
            remaining.set(next);
            if next == 0 {
                main_loop.quit();
            }
            glib::Propagation::Proceed
        });
    }
    executor.host().present_root_windows();
    main_loop.run();
    Ok(ExitCode::SUCCESS)
}

fn compile_file(path: &Path, output: Option<&Path>) -> Result<ExitCode, String> {
    let (sources, file_id) = load_source(path)?;
    let file = &sources[file_id];
    let parsed = parse_module(file);
    let syntax_failed =
        print_stage_diagnostics(CompileStage::Syntax, &sources, parsed.all_diagnostics());
    if syntax_failed {
        print_pipeline_stop(CompileStage::Syntax);
        return Ok(ExitCode::FAILURE);
    }

    let lowered = lower_hir_module(&parsed.module);
    let hir_lowering_failed =
        print_stage_diagnostics(CompileStage::HirLowering, &sources, lowered.diagnostics());
    let validation_mode = if hir_lowering_failed {
        ValidationMode::Structural
    } else {
        ValidationMode::RequireResolvedNames
    };
    let validation = lowered.module().validate(validation_mode);
    let hir_validation_failed = print_stage_diagnostics(
        CompileStage::HirValidation,
        &sources,
        validation.diagnostics(),
    );
    if hir_lowering_failed {
        print_pipeline_stop(CompileStage::HirLowering);
        return Ok(ExitCode::FAILURE);
    }
    if hir_validation_failed {
        print_pipeline_stop(CompileStage::HirValidation);
        return Ok(ExitCode::FAILURE);
    }

    let hir_module = lowered.module();
    let core = match lower_core_module(hir_module) {
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

    let compiled = match compile_program(&backend) {
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
        parsed.module.items.len(),
        plural_suffix(parsed.module.items.len())
    );
    let hir_item_count = hir_module.items().iter().count();
    println!(
        "  HIR: ok ({} item{})",
        hir_item_count,
        plural_suffix(hir_item_count)
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

fn run_lsp(_args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?
        .block_on(aivi_lsp::run())
        .map_err(|e| format!("LSP server error: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

fn print_usage() {
    eprintln!(
        "usage:\n  aivi <path>\n  aivi check <path>\n  aivi compile <path> [-o <object>]\n  aivi run <path> [--view <name>]\n  aivi lex <path>\n  aivi fmt <path>\n  aivi fmt --stdin\n  aivi fmt --check [path...]\n  aivi lsp"
    );
    eprintln!(
        "commands:\n  check    Lex, parse, lower, and validate a module through HIR\n  compile  Lower through typed core, typed lambda, backend, and Cranelift codegen\n  run      Launch the current static GTK markup MVP\n  lex      Dump the lossless token stream\n  fmt      Canonically format the supported surface subset\n  lsp      Start the language server"
    );
    eprintln!(
        "milestone-2 surface items: {:?}",
        [
            ItemKind::Type,
            ItemKind::Value,
            ItemKind::Function,
            ItemKind::Signal,
            ItemKind::Class,
            ItemKind::Instance,
            ItemKind::Domain,
            ItemKind::SourceProviderContract,
            ItemKind::Use,
            ItemKind::Export,
        ]
    );
    eprintln!(
        "core pipe operators: {:?}",
        [
            TokenKind::PipeTransform,
            TokenKind::PipeGate,
            TokenKind::PipeCase,
            TokenKind::PipeMap,
            TokenKind::PipeApply,
            TokenKind::PipeRecurStart,
            TokenKind::PipeRecurStep,
            TokenKind::PipeTap,
            TokenKind::PipeFanIn,
        ]
    );
}

#[cfg(test)]
mod tests {
    use super::{check_file, prepare_static_run_artifact};
    use aivi_base::SourceDatabase;
    use aivi_gtk::GtkBridgeNodeKind;
    use aivi_hir::{ValidationMode, lower_module as lower_hir_module};
    use aivi_syntax::parse_module;
    use std::{path::PathBuf, process::ExitCode};

    fn fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/frontend")
            .join(path)
    }

    fn prepare_static_run_from_text(
        path: &str,
        source: &str,
        requested_view: Option<&str>,
    ) -> Result<super::StaticRunArtifact, String> {
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
        prepare_static_run_artifact(&sources, lowered.module(), requested_view)
    }

    #[test]
    fn check_accepts_milestone_two_valid_fixtures() {
        for path in [
            "milestone-2/valid/source-decorator-signals/main.aivi",
            "milestone-2/valid/pipe-recurrence-nonsource-wakeup/main.aivi",
        ] {
            let result = check_file(&fixture(path)).expect("check should run");
            assert_eq!(result, ExitCode::SUCCESS, "expected {path} to pass");
        }
    }

    #[test]
    fn check_rejects_milestone_two_invalid_fixture() {
        let result = check_file(&fixture("milestone-2/invalid/unknown-decorator/main.aivi"))
            .expect("check should run");
        assert_eq!(result, ExitCode::FAILURE);
    }

    #[test]
    fn prepare_static_run_accepts_a_single_static_window_view() {
        let artifact = prepare_static_run_from_text(
            "static-window.aivi",
            r#"
val screenView =
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
    fn prepare_static_run_prefers_named_view_when_present() {
        let artifact = prepare_static_run_from_text(
            "named-view.aivi",
            r#"
val view =
    <Window title="Default" />

val alternate =
    <Window title="Alternate" />
"#,
            None,
        )
        .expect("default `view` should win when multiple markup values exist");
        assert_eq!(artifact.view_name.as_ref(), "view");
    }

    #[test]
    fn prepare_static_run_rejects_dynamic_properties() {
        let error = prepare_static_run_from_text(
            "dynamic-property.aivi",
            r#"
val title = "AIVI"

val view =
    <Window title={title} />
"#,
            None,
        )
        .expect_err("dynamic setters should stay rejected in the static MVP");
        assert!(error.contains("dynamic property `title`"));
    }

    #[test]
    fn prepare_static_run_rejects_control_nodes() {
        let error = prepare_static_run_from_text(
            "control-node.aivi",
            r#"
val view =
    <Window title="AIVI">
        <show when={True}>
            <Label text="Visible" />
        </show>
    </Window>
"#,
            None,
        )
        .expect_err("control nodes should stay rejected in the static MVP");
        assert!(error.contains("`<show>` control nodes are not supported yet"));
    }

    #[test]
    fn prepare_static_run_requires_view_name_when_multiple_markup_values_exist() {
        let error = prepare_static_run_from_text(
            "multiple-views.aivi",
            r#"
val first =
    <Window title="First" />

val second =
    <Window title="Second" />
"#,
            None,
        )
        .expect_err("multiple unnamed markup views should require `--view`");
        assert!(error.contains("--view <name>"));
    }
}
