#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
    sync::Arc,
};

use aivi_backend::{
    ItemId as BackendItemId, KernelEvaluator, Program as BackendProgram, RuntimeValue,
    compile_program, lower_module as lower_backend_module, validate_program,
};
use aivi_base::{Diagnostic, FileId, Severity, SourceDatabase, SourceSpan};
use aivi_core::{
    RuntimeFragmentSpec, lower_module as lower_core_module, lower_runtime_fragment,
    lower_runtime_module, validate_module as validate_core_module,
};
use aivi_gtk::{
    GtkBridgeGraph, GtkBridgeNodeKind, GtkBridgeNodeRef, GtkChildGroup, GtkCollectionKey,
    GtkConcreteEventPayload, GtkConcreteHost, GtkExecutionPath, GtkHostValue, GtkNodeInstance,
    GtkRuntimeExecutor, RepeatedChildPolicy, RuntimePropertyBinding, RuntimeShowMountPolicy,
    SetterSource, concrete_event_payload, concrete_supports_property, concrete_widget_is_window,
    lower_markup_expr, lower_widget_bridge,
};
use aivi_hir::{
    BuiltinTerm, BuiltinType, ExprId as HirExprId, ExprKind, GeneralExprParameter, Item,
    Module as HirModule, PatternId as HirPatternId, PatternKind, TermResolution, TypeKind,
    TypeResolution, ValidationMode, ValueItem, collect_markup_runtime_expr_sites,
    elaborate_runtime_expr_with_env, lower_module as lower_hir_module,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_runtime::{
    GlibLinkedRuntimeDriver, HirRuntimeAssembly, InputHandle as RuntimeInputHandle, Publication,
    SourceProviderManager, assemble_hir_runtime, link_backend_runtime,
};
use aivi_syntax::{Formatter, ItemKind, TokenKind, lex_module, parse_module};
use gtk::{glib, prelude::*};
use tokio::sync::mpsc;

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHostValue(RuntimeValue);

impl GtkHostValue for RunHostValue {
    fn unit() -> Self {
        Self(RuntimeValue::Unit)
    }

    fn as_bool(&self) -> Option<bool> {
        strip_signal_runtime_value(self.0.clone()).as_bool()
    }

    fn as_i64(&self) -> Option<i64> {
        strip_signal_runtime_value(self.0.clone()).as_i64()
    }

    fn as_text(&self) -> Option<&str> {
        match &self.0 {
            RuntimeValue::Text(value) => Some(value.as_ref()),
            RuntimeValue::Signal(value) => match value.as_ref() {
                RuntimeValue::Text(value) => Some(value.as_ref()),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct RunArtifact {
    view_name: Box<str>,
    module: HirModule,
    bridge: GtkBridgeGraph,
    fragments: BTreeMap<HirExprId, CompiledRunFragment>,
    runtime_assembly: HirRuntimeAssembly,
    core: aivi_core::Module,
    backend: Arc<BackendProgram>,
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
}

#[derive(Clone, Debug)]
struct RunValidationBlocker {
    span: SourceSpan,
    message: String,
}

#[derive(Clone, Debug)]
struct CompiledRunFragment {
    parameters: Vec<GeneralExprParameter>,
    program: BackendProgram,
    item: BackendItemId,
    required_globals: Vec<BackendItemId>,
}

#[derive(Clone, Debug)]
struct ResolvedRunEventHandler {
    signal_item: aivi_hir::ItemId,
    signal_name: Box<str>,
    input: RuntimeInputHandle,
}

struct RunSession {
    view_name: Box<str>,
    module: HirModule,
    bridge: GtkBridgeGraph,
    fragments: BTreeMap<HirExprId, CompiledRunFragment>,
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
    executor: GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    driver: GlibLinkedRuntimeDriver,
    main_loop: glib::MainLoop,
    work_scheduled: bool,
    runtime_error: Option<String>,
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

    let artifact = match prepare_run_artifact(&sources, lowered.module(), requested_view) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("{message}");
            return Ok(ExitCode::FAILURE);
        }
    };

    launch_run(path, artifact)
}

fn prepare_run_artifact(
    sources: &SourceDatabase,
    module: &HirModule,
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let view = select_run_view(module, requested_view)?;
    let view_owner = find_run_view_owner(module, view).ok_or_else(|| {
        format!(
            "failed to recover owning item for run view `{}`",
            view.name.text()
        )
    })?;
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
    let bridge = lower_widget_bridge(&plan).map_err(|error| {
        format!(
            "failed to lower run view `{}` into a GTK bridge graph: {error}",
            view.name.text()
        )
    })?;
    validate_run_plan(sources, &bridge)?;
    let lowered = lower_run_backend_stack(module)?;
    let runtime_assembly = assemble_hir_runtime(module).map_err(|errors| {
        let mut rendered = String::from("failed to assemble runtime plans for `aivi run`:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let fragments = compile_run_fragments(sources, module, view_owner, view.body, &bridge)?;
    let event_handlers = resolve_run_event_handlers(module, &bridge, &runtime_assembly, sources)?;
    Ok(RunArtifact {
        view_name: view.name.text().into(),
        module: module.clone(),
        bridge,
        fragments,
        runtime_assembly,
        core: lowered.core,
        backend: lowered.backend,
        event_handlers,
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

fn find_run_view_owner(module: &HirModule, view: &ValueItem) -> Option<aivi_hir::ItemId> {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(candidate)
                if candidate.body == view.body && candidate.name.text() == view.name.text() =>
            {
                Some(item_id)
            }
            _ => None,
        })
}

fn validate_run_plan(sources: &SourceDatabase, bridge: &GtkBridgeGraph) -> Result<(), String> {
    let mut blockers = Vec::<RunValidationBlocker>::new();

    for node in bridge.nodes() {
        let GtkBridgeNodeKind::Widget(widget) = &node.kind else {
            continue;
        };
        for property in &widget.properties {
            let (name, span) = match property {
                RuntimePropertyBinding::Static(property) => {
                    (property.name.text(), property.site.span)
                }
                RuntimePropertyBinding::Setter(binding) => (binding.name.text(), binding.site.span),
            };
            if !concrete_supports_property(&widget.widget, name) {
                blockers.push(RunValidationBlocker {
                    span,
                    message: format!(
                        "`aivi run` does not support property `{name}` on GTK widget `{}` yet",
                        run_widget_name(&widget.widget)
                    ),
                });
            }
        }
        for event in &widget.event_hooks {
            if concrete_event_payload(&widget.widget, event.name.text()).is_none() {
                blockers.push(RunValidationBlocker {
                    span: event.site.span,
                    message: format!(
                        "`aivi run` does not support event `{}` on GTK widget `{}` yet",
                        event.name.text(),
                        run_widget_name(&widget.widget)
                    ),
                });
            }
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
        if !concrete_widget_is_window(&widget.widget) {
            blockers.push(RunValidationBlocker {
                span: node.span,
                message: format!(
                    "`aivi run` currently requires reachable root widgets to be `Window`; found `{}`",
                    run_widget_name(&widget.widget)
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

fn lower_run_backend_stack(module: &HirModule) -> Result<LoweredRunBackendStack, String> {
    let core = lower_runtime_module(module).map_err(|errors| {
        let mut rendered = String::from("failed to lower `aivi run` module into typed core:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_core_module(&core).map_err(|errors| {
        let mut rendered = String::from("typed-core validation failed for `aivi run`:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let lambda = lower_lambda_module(&core).map_err(|errors| {
        let mut rendered = String::from("failed to lower `aivi run` module into typed lambda:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_lambda_module(&lambda).map_err(|errors| {
        let mut rendered = String::from("typed-lambda validation failed for `aivi run`:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    let backend = lower_backend_module(&lambda).map_err(|errors| {
        let mut rendered = String::from("failed to lower `aivi run` module into backend IR:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;
    validate_program(&backend).map_err(|errors| {
        let mut rendered = String::from("backend validation failed for `aivi run`:\n");
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

fn resolve_run_event_handlers(
    module: &HirModule,
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
    runtime_assembly: &HirRuntimeAssembly,
    sources: &SourceDatabase,
    widget: &aivi_hir::NamePath,
    event_name: &str,
    expr: HirExprId,
) -> Result<ResolvedRunEventHandler, String> {
    let location = source_location(sources, module.exprs()[expr].span);
    let ExprKind::Name(reference) = &module.exprs()[expr].kind else {
        return Err(format!(
            "event handler at {location} must be a direct signal name, not {:?}",
            module.exprs()[expr].kind
        ));
    };
    let aivi_hir::ResolutionState::Resolved(TermResolution::Item(item_id)) =
        reference.resolution.as_ref()
    else {
        return Err(format!(
            "event handler `{}` at {location} must resolve directly to a same-module signal item",
            name_path_text(&reference.path)
        ));
    };
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
    let Some(input) = binding.input() else {
        return Err(format!(
            "event handler `{}` at {location} points at signal `{}`, but only direct input signals are publishable from GTK events",
            name_path_text(&reference.path),
            signal.name.text()
        ));
    };
    let Some(payload) = concrete_event_payload(widget, event_name) else {
        return Err(format!(
            "event handler `{}` at {location} uses unsupported GTK event `{}` on widget `{}`",
            name_path_text(&reference.path),
            event_name,
            run_widget_name(widget)
        ));
    };
    if !signal_accepts_event_payload(module, signal, payload) {
        return Err(format!(
            "event handler `{}` at {location} points at signal `{}`, but `{}` on `{}` publishes `{}` and requires `{}`",
            name_path_text(&reference.path),
            signal.name.text(),
            event_name,
            run_widget_name(widget),
            event_payload_label(payload),
            required_signal_type_label(payload)
        ));
    }
    Ok(ResolvedRunEventHandler {
        signal_item: *item_id,
        signal_name: signal.name.text().into(),
        input,
    })
}

fn signal_accepts_event_payload(
    module: &HirModule,
    signal: &aivi_hir::SignalItem,
    payload: GtkConcreteEventPayload,
) -> bool {
    let Some(annotation) = signal.annotation else {
        return false;
    };
    let TypeKind::Apply { callee, arguments } = &module.types()[annotation].kind else {
        return false;
    };
    if arguments.len() != 1 {
        return false;
    }
    let TypeKind::Name(reference) = &module.types()[*callee].kind else {
        return false;
    };
    if !matches!(
        &reference.resolution,
        aivi_hir::ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal))
    ) {
        return false;
    }
    let Some(payload_ty) = arguments.iter().next().copied() else {
        return false;
    };
    match payload {
        GtkConcreteEventPayload::Unit => type_is_builtin(module, payload_ty, BuiltinType::Unit),
    }
}

fn type_is_builtin(module: &HirModule, ty: aivi_hir::TypeId, builtin: BuiltinType) -> bool {
    match &module.types()[ty].kind {
        TypeKind::Name(reference) => matches!(
            &reference.resolution,
            aivi_hir::ResolutionState::Resolved(TypeResolution::Builtin(resolved))
                if *resolved == builtin
        ),
        TypeKind::Tuple(_)
        | TypeKind::Record(_)
        | TypeKind::Arrow { .. }
        | TypeKind::Apply { .. } => false,
    }
}

fn event_payload_label(payload: GtkConcreteEventPayload) -> &'static str {
    match payload {
        GtkConcreteEventPayload::Unit => "Unit",
    }
}

fn required_signal_type_label(payload: GtkConcreteEventPayload) -> &'static str {
    match payload {
        GtkConcreteEventPayload::Unit => "`Signal Unit`",
    }
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
    }
}

fn collect_run_exprs_from_bridge(bridge: &GtkBridgeGraph) -> BTreeSet<HirExprId> {
    let mut exprs = BTreeSet::new();
    for node in bridge.nodes() {
        match &node.kind {
            GtkBridgeNodeKind::Widget(widget) => {
                for property in &widget.properties {
                    if let RuntimePropertyBinding::Setter(setter) = property {
                        match &setter.source {
                            SetterSource::Expr(expr) => {
                                exprs.insert(*expr);
                            }
                            SetterSource::InterpolatedText(text) => {
                                for segment in &text.segments {
                                    if let aivi_hir::TextSegment::Interpolation(interpolation) =
                                        segment
                                    {
                                        exprs.insert(interpolation.expr);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            GtkBridgeNodeKind::Show(show) => {
                exprs.insert(show.when.expr);
                if let RuntimeShowMountPolicy::KeepMounted { decision } = &show.mount {
                    exprs.insert(decision.expr);
                }
            }
            GtkBridgeNodeKind::Each(each) => {
                exprs.insert(each.collection.expr);
                if let RepeatedChildPolicy::Keyed { key, .. } = &each.child_policy {
                    exprs.insert(*key);
                }
            }
            GtkBridgeNodeKind::Match(match_node) => {
                exprs.insert(match_node.scrutinee.expr);
            }
            GtkBridgeNodeKind::With(with_node) => {
                exprs.insert(with_node.value.expr);
            }
            GtkBridgeNodeKind::Empty(_)
            | GtkBridgeNodeKind::Case(_)
            | GtkBridgeNodeKind::Fragment(_) => {}
        }
    }
    exprs
}

fn compile_run_fragments(
    sources: &SourceDatabase,
    module: &HirModule,
    view_owner: aivi_hir::ItemId,
    view_body: HirExprId,
    bridge: &GtkBridgeGraph,
) -> Result<BTreeMap<HirExprId, CompiledRunFragment>, String> {
    let sites = collect_markup_runtime_expr_sites(module, view_body).map_err(|error| {
        format!(
            "failed to collect runtime expression environments for run view at {}: {error}",
            source_location(sources, module.exprs()[view_body].span)
        )
    })?;
    let mut fragments = BTreeMap::new();
    for expr in collect_run_exprs_from_bridge(bridge) {
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
        let required_globals = backend.items()[item]
            .body
            .map(|kernel| backend.kernels()[kernel].global_items.clone())
            .unwrap_or_default();
        fragments.insert(
            expr,
            CompiledRunFragment {
                parameters: site.parameters.clone(),
                program: backend,
                item,
                required_globals,
            },
        );
    }
    Ok(fragments)
}

fn launch_run(path: &Path, artifact: RunArtifact) -> Result<ExitCode, String> {
    gtk::init()
        .map_err(|error| format!("failed to initialize GTK for {}: {error}", path.display()))?;
    let RunArtifact {
        view_name,
        module,
        bridge,
        fragments,
        runtime_assembly,
        core,
        backend,
        event_handlers,
    } = artifact;
    let linked = link_backend_runtime(runtime_assembly, &core, backend).map_err(|errors| {
        let mut rendered = String::from("failed to link backend runtime for `aivi run`:\n");
        for error in errors.errors() {
            rendered.push_str("- ");
            rendered.push_str(&error.to_string());
            rendered.push('\n');
        }
        rendered
    })?;

    let context = glib::MainContext::default();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<()>();
    let driver = GlibLinkedRuntimeDriver::new(
        context.clone(),
        linked,
        SourceProviderManager::new(),
        Some(Arc::new(move || {
            let _ = update_tx.send(());
        })),
    );
    let main_loop = glib::MainLoop::new(Some(&context), false);
    let executor =
        GtkRuntimeExecutor::new(bridge.clone(), GtkConcreteHost::<RunHostValue>::default())
            .map_err(|error| {
                format!(
                    "failed to mount GTK view `{}` from {}: {error}",
                    view_name,
                    path.display()
                )
            })?;
    let session = Rc::new(RefCell::new(RunSession {
        view_name,
        module,
        bridge,
        fragments,
        event_handlers,
        executor,
        driver,
        main_loop: main_loop.clone(),
        work_scheduled: false,
        runtime_error: None,
    }));
    {
        let weak_session = Rc::downgrade(&session);
        session
            .borrow_mut()
            .executor
            .host_mut()
            .set_event_notifier(Some(Rc::new(move || {
                if let Some(session) = weak_session.upgrade() {
                    schedule_run_session(&session);
                }
            })));
    }
    {
        let weak_session = Rc::downgrade(&session);
        context.spawn_local(async move {
            while update_rx.recv().await.is_some() {
                let Some(session) = weak_session.upgrade() else {
                    break;
                };
                schedule_run_session(&session);
            }
        });
    }

    session.borrow().driver.tick_now();
    {
        let mut session = session.borrow_mut();
        session.hydrate_current_state().map_err(|error| {
            format!(
                "failed to hydrate run view `{}`: {error}",
                session.view_name
            )
        })?;
        session.process_pending_work().map_err(|error| {
            format!("failed to start run view `{}`: {error}", session.view_name)
        })?;
    }
    let root_windows = session.borrow_mut().collect_root_windows()?;

    println!(
        "running GTK view `{}` from {}",
        session.borrow().view_name,
        path.display()
    );

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
    session.borrow().executor.host().present_root_windows();
    main_loop.run();
    let mut session = session.borrow_mut();
    if let Some(error) = session.runtime_error.take() {
        return Err(error);
    }
    Ok(ExitCode::SUCCESS)
}

type RuntimeBindingEnv = BTreeMap<aivi_hir::BindingId, RuntimeValue>;

impl RunSession {
    fn hydrate_current_state(&mut self) -> Result<(), String> {
        let globals = self
            .driver
            .current_signal_globals()
            .map_err(|error| format!("{error}"))?;
        hydrate_run_view(
            &self.module,
            &self.bridge,
            self.view_name.as_ref(),
            &self.fragments,
            &globals,
            &mut self.executor,
        )
    }

    fn process_pending_work(&mut self) -> Result<(), String> {
        let queued_events = self.executor.host_mut().drain_events();
        if !queued_events.is_empty() {
            let mut sink = RunEventSink {
                driver: &self.driver,
                handlers: &self.event_handlers,
            };
            for event in queued_events {
                self.executor
                    .dispatch_event(event.route, event.value, &mut sink)
                    .map_err(|error| {
                        format!("failed to dispatch GTK event {}: {error}", event.route)
                    })?;
            }
        }
        let failures = self.driver.drain_failures();
        if !failures.is_empty() {
            let mut rendered = String::from("live runtime failed during `aivi run`:\n");
            for failure in failures {
                rendered.push_str("- ");
                rendered.push_str(&failure.to_string());
                rendered.push('\n');
            }
            return Err(rendered);
        }
        if !self.driver.drain_outcomes().is_empty() {
            self.hydrate_current_state()?;
        }
        Ok(())
    }

    fn collect_root_windows(&mut self) -> Result<Vec<gtk::Window>, String> {
        let root_handles = self.executor.root_widgets().map_err(|error| {
            format!(
                "failed to collect root widgets for run view `{}`: {error}",
                self.view_name
            )
        })?;
        if root_handles.is_empty() {
            return Err(format!(
                "run view `{}` did not produce any root GTK widgets",
                self.view_name
            ));
        }
        root_handles
            .into_iter()
            .map(|handle| {
                let widget = self.executor.host().widget(&handle).ok_or_else(|| {
                    format!(
                        "run view `{}` lost GTK root widget {:?} before presentation",
                        self.view_name, handle
                    )
                })?;
                widget.clone().downcast::<gtk::Window>().map_err(|widget| {
                    format!(
                        "`aivi run` currently requires top-level `Window` roots; view `{}` produced a root `{}`",
                        self.view_name,
                        widget.type_().name()
                    )
                })
            })
            .collect()
    }
}

fn schedule_run_session(session: &Rc<RefCell<RunSession>>) {
    if session.borrow().work_scheduled {
        return;
    }
    session.borrow_mut().work_scheduled = true;
    let weak_session = Rc::downgrade(session);
    glib::idle_add_local_once(move || {
        let Some(session) = weak_session.upgrade() else {
            return;
        };
        let mut session = session.borrow_mut();
        session.work_scheduled = false;
        if session.runtime_error.is_some() {
            return;
        }
        if let Err(error) = session.process_pending_work() {
            session.runtime_error = Some(error);
            session.main_loop.quit();
        }
    });
}

struct RunEventSink<'a> {
    driver: &'a GlibLinkedRuntimeDriver,
    handlers: &'a BTreeMap<HirExprId, ResolvedRunEventHandler>,
}

impl aivi_gtk::GtkEventSink<RunHostValue> for RunEventSink<'_> {
    type Error = String;

    fn dispatch_event(
        &mut self,
        route: &aivi_gtk::GtkEventRoute,
        value: RunHostValue,
    ) -> Result<(), Self::Error> {
        let handler = self.handlers.get(&route.binding.handler).ok_or_else(|| {
            format!(
                "missing resolved event handler for expression {} on route {}",
                route.binding.handler.as_raw(),
                route.id
            )
        })?;
        let stamp = self
            .driver
            .current_stamp(handler.input)
            .map_err(|error| format!("{error}"))?;
        self.driver
            .queue_publication_now(Publication::new(stamp, value.0))
            .map_err(|error| {
                format!(
                    "failed to publish GTK event on route {} into signal `{}` (item {}): {error}",
                    route.id,
                    handler.signal_name,
                    handler.signal_item.as_raw()
                )
            })
    }
}

fn hydrate_run_view(
    module: &HirModule,
    bridge: &GtkBridgeGraph,
    view_name: &str,
    fragments: &BTreeMap<HirExprId, CompiledRunFragment>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
) -> Result<(), String> {
    hydrate_node(
        module,
        bridge,
        fragments,
        globals,
        view_name,
        executor,
        &GtkNodeInstance::root(bridge.root()),
        &RuntimeBindingEnv::new(),
    )
}

fn hydrate_node(
    module: &HirModule,
    bridge: &GtkBridgeGraph,
    fragments: &BTreeMap<HirExprId, CompiledRunFragment>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    view_name: &str,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    instance: &GtkNodeInstance,
    env: &RuntimeBindingEnv,
) -> Result<(), String> {
    let node = bridge.node(instance.node.plan).ok_or_else(|| {
        format!(
            "run view `{view_name}` is missing GTK node {}",
            instance.node
        )
    })?;
    match &node.kind {
        GtkBridgeNodeKind::Widget(widget) => {
            for property in &widget.properties {
                if let RuntimePropertyBinding::Setter(setter) = property {
                    let value = match &setter.source {
                        SetterSource::Expr(expr) => {
                            evaluate_run_fragment(fragments, globals, *expr, env)?
                        }
                        SetterSource::InterpolatedText(text) => RuntimeValue::Text(
                            evaluate_runtime_text(fragments, globals, text, env)?,
                        ),
                    };
                    executor
                        .set_property_for_instance(instance, setter.input, RunHostValue(value))
                        .map_err(|error| {
                            format!(
                                "failed to apply dynamic property `{}` on {}: {error}",
                                setter.name.text(),
                                instance
                            )
                        })?;
                }
            }
            hydrate_child_group(
                module,
                bridge,
                fragments,
                globals,
                view_name,
                executor,
                &widget.default_children.roots,
                instance.path.clone(),
                env,
            )
        }
        GtkBridgeNodeKind::Show(show) => {
            let when = runtime_bool(evaluate_run_fragment(fragments, globals, show.when.expr, env)?)
                .ok_or_else(|| {
                    format!(
                        "run view `{view_name}` expected `<show when>` on {instance} to evaluate to Bool"
                    )
                })?;
            let keep_mounted = match &show.mount {
                RuntimeShowMountPolicy::UnmountWhenHidden => false,
                RuntimeShowMountPolicy::KeepMounted { decision } => runtime_bool(
                    evaluate_run_fragment(fragments, globals, decision.expr, env)?,
                )
                .ok_or_else(|| {
                    format!(
                        "run view `{view_name}` expected `<show keepMounted>` on {instance} to evaluate to Bool"
                    )
                })?,
            };
            executor
                .update_show(instance, when, keep_mounted)
                .map_err(|error| format!("failed to update `<show>` node {instance}: {error}"))?;
            if when || keep_mounted {
                hydrate_child_group(
                    module,
                    bridge,
                    fragments,
                    globals,
                    view_name,
                    executor,
                    &show.body.roots,
                    instance.path.clone(),
                    env,
                )?;
            }
            Ok(())
        }
        GtkBridgeNodeKind::Each(each) => {
            let values = runtime_list_values(evaluate_run_fragment(
                fragments,
                globals,
                each.collection.expr,
                env,
            )?)
            .ok_or_else(|| {
                format!(
                    "run view `{view_name}` expected `<each>` collection on {instance} to evaluate to a List"
                )
            })?;
            let collection_is_empty = values.is_empty();
            match &each.child_policy {
                RepeatedChildPolicy::Positional { .. } => {
                    executor
                        .update_each_positional(instance, values.len())
                        .map_err(|error| {
                            format!("failed to update positional `<each>` node {instance}: {error}")
                        })?;
                    for (index, value) in values.into_iter().enumerate() {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value);
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Positional(index),
                        );
                        hydrate_child_group(
                            module,
                            bridge,
                            fragments,
                            globals,
                            view_name,
                            executor,
                            &each.item_template.roots,
                            path,
                            &child_env,
                        )?;
                    }
                }
                RepeatedChildPolicy::Keyed { key, .. } => {
                    let mut keyed_items = Vec::with_capacity(values.len());
                    let mut keys = Vec::with_capacity(values.len());
                    for value in values {
                        let mut child_env = env.clone();
                        child_env.insert(each.binding, value.clone());
                        let key_value =
                            evaluate_run_fragment(fragments, globals, *key, &child_env)?;
                        let collection_key = runtime_collection_key(key_value).ok_or_else(|| {
                            format!(
                                "run view `{view_name}` expected `<each>` key on {instance} to be displayable"
                            )
                        })?;
                        keys.push(collection_key.clone());
                        keyed_items.push((collection_key, child_env));
                    }
                    executor
                        .update_each_keyed(instance, &keys)
                        .map_err(|error| {
                            format!("failed to update keyed `<each>` node {instance}: {error}")
                        })?;
                    for (collection_key, child_env) in keyed_items {
                        let path = instance.path.pushed(
                            instance.node,
                            aivi_gtk::GtkRepeatedChildIdentity::Keyed(collection_key),
                        );
                        hydrate_child_group(
                            module,
                            bridge,
                            fragments,
                            globals,
                            view_name,
                            executor,
                            &each.item_template.roots,
                            path,
                            &child_env,
                        )?;
                    }
                }
            }
            if collection_is_empty {
                if let Some(empty) = &each.empty_branch {
                    hydrate_node(
                        module,
                        bridge,
                        fragments,
                        globals,
                        view_name,
                        executor,
                        &GtkNodeInstance::with_path(empty.empty, instance.path.clone()),
                        env,
                    )?;
                }
            }
            Ok(())
        }
        GtkBridgeNodeKind::Match(match_node) => {
            let value = evaluate_run_fragment(fragments, globals, match_node.scrutinee.expr, env)?;
            let mut matched = None;
            for (index, branch) in match_node.cases.iter().enumerate() {
                let mut bindings = RuntimeBindingEnv::new();
                if match_pattern(module, branch.pattern, &value, &mut bindings)? {
                    matched = Some((index, branch.clone(), bindings));
                    break;
                }
            }
            let Some((index, branch, bindings)) = matched else {
                return Err(format!(
                    "run view `{view_name}` found no matching `<match>` case for node {instance}"
                ));
            };
            executor
                .update_match(instance, index)
                .map_err(|error| format!("failed to update `<match>` node {instance}: {error}"))?;
            let mut case_env = env.clone();
            case_env.extend(bindings);
            hydrate_node(
                module,
                bridge,
                fragments,
                globals,
                view_name,
                executor,
                &GtkNodeInstance::with_path(branch.case, instance.path.clone()),
                &case_env,
            )
        }
        GtkBridgeNodeKind::Case(case) => hydrate_child_group(
            module,
            bridge,
            fragments,
            globals,
            view_name,
            executor,
            &case.body.roots,
            instance.path.clone(),
            env,
        ),
        GtkBridgeNodeKind::Fragment(fragment) => hydrate_child_group(
            module,
            bridge,
            fragments,
            globals,
            view_name,
            executor,
            &fragment.body.roots,
            instance.path.clone(),
            env,
        ),
        GtkBridgeNodeKind::With(with_node) => {
            let value = evaluate_run_fragment(fragments, globals, with_node.value.expr, env)?;
            let mut child_env = env.clone();
            child_env.insert(with_node.binding, value);
            hydrate_child_group(
                module,
                bridge,
                fragments,
                globals,
                view_name,
                executor,
                &with_node.body.roots,
                instance.path.clone(),
                &child_env,
            )
        }
        GtkBridgeNodeKind::Empty(empty) => hydrate_child_group(
            module,
            bridge,
            fragments,
            globals,
            view_name,
            executor,
            &empty.body.roots,
            instance.path.clone(),
            env,
        ),
    }
}

fn hydrate_child_group(
    module: &HirModule,
    bridge: &GtkBridgeGraph,
    fragments: &BTreeMap<HirExprId, CompiledRunFragment>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    view_name: &str,
    executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    roots: &[aivi_gtk::GtkBridgeNodeRef],
    path: GtkExecutionPath,
    env: &RuntimeBindingEnv,
) -> Result<(), String> {
    for &root in roots {
        hydrate_node(
            module,
            bridge,
            fragments,
            globals,
            view_name,
            executor,
            &GtkNodeInstance::with_path(root, path.clone()),
            env,
        )?;
    }
    Ok(())
}

fn evaluate_run_fragment(
    fragments: &BTreeMap<HirExprId, CompiledRunFragment>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    expr: HirExprId,
    env: &RuntimeBindingEnv,
) -> Result<RuntimeValue, String> {
    let fragment = fragments.get(&expr).ok_or_else(|| {
        format!(
            "missing compiled runtime fragment for expression {}",
            expr.as_raw()
        )
    })?;
    let args = fragment
        .parameters
        .iter()
        .map(|parameter| {
            env.get(&parameter.binding).cloned().ok_or_else(|| {
                format!(
                    "missing runtime value for binding `{}` while evaluating expression {}",
                    parameter.name,
                    expr.as_raw()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let item = &fragment.program.items()[fragment.item];
    let mut evaluator = KernelEvaluator::new(&fragment.program);
    let required_globals = fragment
        .required_globals
        .iter()
        .map(|item| {
            globals
                .get(item)
                .cloned()
                .map(|value| (*item, value))
                .ok_or_else(|| {
                    format!(
                        "runtime expression {} requires current signal item {} but no committed snapshot exists",
                        expr.as_raw(),
                        item
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
                expr.as_raw()
            )
        })?;
        evaluator
            .evaluate_kernel(kernel, None, &args, &required_globals)
            .map_err(|error| format!("{error}"))
    }
}

fn evaluate_runtime_text(
    fragments: &BTreeMap<HirExprId, CompiledRunFragment>,
    globals: &BTreeMap<BackendItemId, RuntimeValue>,
    text: &aivi_hir::TextLiteral,
    env: &RuntimeBindingEnv,
) -> Result<Box<str>, String> {
    let mut rendered = String::new();
    for segment in &text.segments {
        match segment {
            aivi_hir::TextSegment::Text(fragment) => rendered.push_str(fragment.raw.as_ref()),
            aivi_hir::TextSegment::Interpolation(interpolation) => {
                let value = strip_signal_runtime_value(evaluate_run_fragment(
                    fragments,
                    globals,
                    interpolation.expr,
                    env,
                )?);
                if matches!(value, RuntimeValue::Callable(_)) {
                    return Err(format!(
                        "text interpolation for expression {} produced a callable runtime value",
                        interpolation.expr.as_raw()
                    ));
                }
                rendered.push_str(&value.to_string());
            }
        }
    }
    Ok(rendered.into_boxed_str())
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
        "commands:\n  check    Lex, parse, lower, and validate a module through HIR\n  compile  Lower through typed core, typed lambda, backend, and Cranelift codegen\n  run      Launch the current live GTK runtime path\n  lex      Dump the lossless token stream\n  fmt      Canonically format the supported surface subset\n  lsp      Start the language server"
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
    use super::{check_file, prepare_run_artifact};
    use aivi_base::SourceDatabase;
    use aivi_gtk::{GtkBridgeNodeKind, RuntimePropertyBinding};
    use aivi_hir::{ValidationMode, lower_module as lower_hir_module};
    use aivi_syntax::parse_module;
    use std::{path::PathBuf, process::ExitCode};

    fn fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/frontend")
            .join(path)
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
        prepare_run_artifact(&sources, lowered.module(), requested_view)
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
    fn prepare_run_accepts_a_single_static_window_view() {
        let artifact = prepare_run_from_text(
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
    fn prepare_run_prefers_named_view_when_present() {
        let artifact = prepare_run_from_text(
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
    fn prepare_run_accepts_dynamic_properties() {
        let artifact = prepare_run_from_text(
            "dynamic-property.aivi",
            r#"
val title = "AIVI"

val view =
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
        assert!(!artifact.fragments.is_empty());
    }

    #[test]
    fn prepare_run_accepts_control_nodes() {
        let artifact = prepare_run_from_text(
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
    fn prepare_run_accepts_direct_signal_event_hooks() {
        let artifact = prepare_run_from_text(
            "event-hook.aivi",
            r#"
sig click : Signal Unit

val view =
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
    fn prepare_run_rejects_non_window_root_widgets() {
        let error = prepare_run_from_text(
            "button-root.aivi",
            r#"
val view =
    <Button label="Save" />
"#,
            None,
        )
        .expect_err("non-window roots should be rejected before launch");
        assert!(error.contains("reachable root widgets"));
        assert!(error.contains("Window"));
    }

    #[test]
    fn prepare_run_rejects_event_payload_mismatch() {
        let error = prepare_run_from_text(
            "event-payload-mismatch.aivi",
            r#"
sig click : Signal Int

val view =
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
    fn prepare_run_requires_view_name_when_multiple_markup_values_exist() {
        let error = prepare_run_from_text(
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
