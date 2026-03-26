use super::*;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::mpsc as sync_mpsc,
    thread,
    time::{Duration, Instant},
};

use aivi_backend::{RuntimeFloat, RuntimeRecordField, RuntimeSumValue, RuntimeValue};
use aivi_runtime::{
    GlibLinkedSourceMode, SourceProviderContext, SourceProviderManager, decode_external,
    encode_runtime_json, parse_json_text,
};
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const HOST_POLL_INTERVAL: Duration = Duration::from_millis(10);
const HYDRATION_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);

type HostTask = Box<dyn FnOnce(&mut McpHostState) + Send + 'static>;

pub(super) fn run_mcp(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut requested_path = None;
    let mut requested_view = None;

    while let Some(argument) = args.next() {
        if argument == OsString::from("--path") {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path value after `--path` for `mcp`".to_owned())?;
            if requested_path.replace(path).is_some() {
                return Err("mcp path was provided more than once".to_owned());
            }
            continue;
        }

        if argument == OsString::from("--view") {
            let view = args
                .next()
                .ok_or_else(|| "expected a value name after `--view` for `mcp`".to_owned())?;
            if requested_view
                .replace(view.to_string_lossy().into_owned())
                .is_some()
            {
                return Err("mcp view name was provided more than once".to_owned());
            }
            continue;
        }

        return Err(format!(
            "unexpected argument `{}` for `mcp`; expected only `--path` and `--view`",
            argument.to_string_lossy()
        ));
    }

    if let Some(view) = &requested_view {
        let segments: Vec<&str> = view.split('.').collect();
        validate_module_path(&segments)?;
    }

    let cwd = env::current_dir().map_err(|error| {
        format!("failed to determine current directory for `aivi mcp`: {error}")
    })?;
    let entry_path = resolve_initial_entry_path(&cwd, requested_path.as_deref())?;
    let configured = ConfiguredTarget {
        entry_path,
        default_view: requested_view,
    };

    let (task_tx, task_rx) = sync_mpsc::channel::<HostTask>();
    let controller = McpHostController { task_tx };
    let server_handle = thread::Builder::new()
        .name("aivi-mcp-stdio".to_owned())
        .spawn({
            let configured = configured.clone();
            move || run_stdio_server(controller, configured)
        })
        .map_err(|error| format!("failed to start `aivi mcp` stdio worker: {error}"))?;

    let host_result = run_host_loop(task_rx, configured);
    let server_result = server_handle
        .join()
        .map_err(|_| "`aivi mcp` stdio worker panicked".to_owned())?;

    host_result?;
    server_result?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Clone)]
struct ConfiguredTarget {
    entry_path: Option<PathBuf>,
    default_view: Option<String>,
}

#[derive(Clone)]
struct McpHostController {
    task_tx: sync_mpsc::Sender<HostTask>,
}

impl McpHostController {
    fn call<R>(
        &self,
        task: impl FnOnce(&mut McpHostState) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String>
    where
        R: Send + 'static,
    {
        let (response_tx, response_rx) = sync_mpsc::sync_channel(1);
        self.task_tx
            .send(Box::new(move |host| {
                let _ = response_tx.send(task(host));
            }))
            .map_err(|_| "`aivi mcp` host loop has already stopped".to_owned())?;
        response_rx
            .recv()
            .map_err(|_| "`aivi mcp` host loop dropped the response channel".to_owned())?
    }

    fn shutdown(&self) {
        let _ = self
            .task_tx
            .send(Box::new(|host| host.shutting_down = true));
    }
}

struct McpHostState {
    context: glib::MainContext,
    configured: ConfiguredTarget,
    session: Option<HostedSession>,
    widget_ids: BTreeMap<usize, u64>,
    next_widget_id: u64,
    shutting_down: bool,
}

struct HostedSession {
    harness: run_session::RunSessionHarness,
    path: PathBuf,
    view_name: String,
}

impl McpHostState {
    fn launch_prepared(&mut self, prepared: PreparedLaunch) -> Result<SessionStatus, String> {
        self.stop_session();
        let entry_path = prepared.entry_path.clone();
        let harness = run_session::start_run_session_with_launch_config(
            &prepared.entry_path,
            prepared.artifact,
            prepared.launch_config,
        )?;
        harness.install_quit_on_last_window_close();
        harness.present_root_windows();
        let view_name = harness.view_name().to_owned();
        self.widget_ids.clear();
        self.next_widget_id = 0;
        self.configured.entry_path = Some(entry_path.clone());
        self.configured.default_view = Some(view_name.clone());
        self.session = Some(HostedSession {
            view_name,
            harness,
            path: entry_path,
        });
        self.session_status()
    }

    fn stop_app(&mut self) -> Result<SessionStatus, String> {
        self.stop_session();
        Ok(self.session_status_unlaunched())
    }

    fn stop_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.harness.shutdown();
        }
        self.widget_ids.clear();
        self.next_widget_id = 0;
    }

    fn session_status(&self) -> Result<SessionStatus, String> {
        let Some(session) = &self.session else {
            return Ok(self.session_status_unlaunched());
        };
        let runtime = session.harness.with_access(|access| SessionRuntimeStatus {
            phase: phase_label(access.phase()).to_owned(),
            latest_requested_hydration: access.latest_requested_hydration(),
            latest_applied_hydration: access.latest_applied_hydration(),
            queued_messages: access.queued_message_count(),
            queued_outcomes: access.outcome_count(),
            queued_failures: access.failure_count(),
        });
        Ok(SessionStatus {
            launched: true,
            configured_entry_path: self
                .configured
                .entry_path
                .as_ref()
                .map(|path| path.display().to_string()),
            configured_view: self.configured.default_view.clone(),
            active_entry_path: Some(session.path.display().to_string()),
            active_view: Some(session.view_name.clone()),
            phase: Some(runtime.phase),
            root_window_count: session.harness.root_windows().len(),
            latest_requested_hydration: runtime.latest_requested_hydration,
            latest_applied_hydration: runtime.latest_applied_hydration,
            queued_messages: Some(runtime.queued_messages),
            queued_outcomes: Some(runtime.queued_outcomes),
            queued_failures: Some(runtime.queued_failures),
        })
    }

    fn session_status_unlaunched(&self) -> SessionStatus {
        SessionStatus {
            launched: false,
            configured_entry_path: self
                .configured
                .entry_path
                .as_ref()
                .map(|path| path.display().to_string()),
            configured_view: self.configured.default_view.clone(),
            active_entry_path: None,
            active_view: None,
            phase: None,
            root_window_count: 0,
            latest_requested_hydration: None,
            latest_applied_hydration: None,
            queued_messages: None,
            queued_outcomes: None,
            queued_failures: None,
        }
    }

    fn list_signals(&self, query: ListSignalsArgs) -> Result<Vec<SignalSnapshot>, String> {
        let session = self.require_session()?;
        let mut signals = session
            .harness
            .with_access(|access| snapshot_signals(&access.driver()))?;
        if let Some(filter) = query.name_contains.as_deref() {
            let needle = filter.to_lowercase();
            signals.retain(|signal| {
                signal.name.to_lowercase().contains(&needle)
                    || signal
                        .owner_path
                        .as_ref()
                        .is_some_and(|path| path.to_lowercase().contains(&needle))
            });
        }
        Ok(signals)
    }

    fn get_signal(&self, selector: SignalSelector) -> Result<SignalSnapshot, String> {
        let signals = self.list_signals(ListSignalsArgs::default())?;
        resolve_signal(signals, &selector)
    }

    fn assert_signal(
        &self,
        selector: SignalSelector,
        expected: JsonValue,
    ) -> Result<SignalAssertResult, String> {
        let signal = self.get_signal(selector.clone())?;
        let matched = signal.value == Some(expected.clone());
        Ok(SignalAssertResult {
            selector,
            matched,
            actual: signal.value.clone(),
            expected,
            signal,
        })
    }

    fn list_sources(&self) -> Result<Vec<SourceSnapshot>, String> {
        let session = self.require_session()?;
        session
            .harness
            .with_access(|access| snapshot_sources(&access.driver()))
    }

    fn set_source_mode(&self, args: SetSourceModeArgs) -> Result<SourceModeResult, String> {
        let session = self.require_session()?;
        let source_id = parse_source_id(&args.source_id)?;
        let mode = parse_source_mode(&args.mode)?;
        session.harness.with_access(|access| {
            access
                .driver()
                .set_source_mode(source_id, mode)
                .map_err(|error| format!("failed to set source mode: {error}"))
        })?;
        if matches!(mode, GlibLinkedSourceMode::Live) {
            self.process_context_work();
        }
        let updated = self
            .list_sources()?
            .into_iter()
            .find(|source| source.id == args.source_id)
            .ok_or_else(|| format!("source `{}` disappeared after mode change", args.source_id))?;
        Ok(SourceModeResult { source: updated })
    }

    fn publish_source_value(
        &mut self,
        args: PublishSourceValueArgs,
    ) -> Result<SourcePublishResult, String> {
        let source_id = parse_source_id(&args.source_id)?;
        let before = self.list_signals(ListSignalsArgs::default())?;
        let session = self.require_session()?;
        session.harness.with_access(|access| {
            let driver = access.driver();
            if args.suspend_live.unwrap_or(true) {
                driver
                    .set_source_mode(source_id, GlibLinkedSourceMode::Manual)
                    .map_err(|error| {
                        format!(
                            "failed to enter manual mode for source {}: {error}",
                            source_id.as_raw()
                        )
                    })?;
            }
            let config = driver.evaluate_source_config(source_id).map_err(|error| {
                format!(
                    "failed to evaluate source config for {}: {error}",
                    source_id.as_raw()
                )
            })?;
            let runtime = runtime_value_from_source_json(&args.value, config.decode.as_ref())?;
            driver
                .inject_source_value(source_id, DetachedRuntimeValue::from_runtime_owned(runtime))
                .map_err(|error| format!("failed to inject source value: {error}"))?;
            access.process_pending_work()
        })?;
        self.settle_session()?;
        let after = self.list_signals(ListSignalsArgs::default())?;
        let source = self
            .list_sources()?
            .into_iter()
            .find(|source| source.id == args.source_id)
            .ok_or_else(|| format!("source `{}` disappeared after publication", args.source_id))?;
        Ok(SourcePublishResult {
            source,
            changed_signals: diff_signals(&before, &after),
            session: self.session_status()?,
        })
    }

    fn snapshot_gtk_tree(&mut self, args: SnapshotGtkArgs) -> Result<Vec<WidgetSnapshot>, String> {
        let roots = self.root_widgets();
        roots
            .into_iter()
            .filter_map(|widget| {
                let snapshot = self.snapshot_widget(&widget, Vec::new());
                match (snapshot, args.include_hidden.unwrap_or(false)) {
                    (Ok(snapshot), true) => Some(Ok(snapshot)),
                    (Ok(snapshot), false) if snapshot.visible => Some(Ok(snapshot)),
                    (Ok(_), false) => None,
                    (Err(error), _) => Some(Err(error)),
                }
            })
            .collect()
    }

    fn find_widgets(&mut self, query: FindWidgetsArgs) -> Result<Vec<WidgetMatch>, String> {
        let trees = self.snapshot_gtk_tree(SnapshotGtkArgs {
            include_hidden: Some(query.include_hidden.unwrap_or(false)),
        })?;
        let mut matches = Vec::new();
        for root in &trees {
            collect_widget_matches(root, &query, &mut matches);
        }
        Ok(matches)
    }

    fn emit_gtk_event(&mut self, args: EmitGtkEventArgs) -> Result<EventResult, String> {
        let before = self.list_signals(ListSignalsArgs::default())?;
        let widget = match args.event.as_str() {
            "window_key" => None,
            _ => Some(
                self.find_widget_by_id(&args.widget_id)?
                    .ok_or_else(|| format!("no live GTK widget matches `{}`", args.widget_id))?,
            ),
        };
        let session = self.require_session()?;
        match args.event.as_str() {
            "click" | "activate" => emit_activate_event(widget.as_ref().expect("widget required"))?,
            "set_text" => {
                let text = args
                    .text
                    .as_deref()
                    .ok_or_else(|| "`set_text` requires a `text` argument".to_owned())?;
                emit_set_text(widget.as_ref().expect("widget required"), text)?;
            }
            "set_active" => {
                let active = args
                    .active
                    .ok_or_else(|| "`set_active` requires an `active` argument".to_owned())?;
                emit_set_active(widget.as_ref().expect("widget required"), active)?;
            }
            "focus" => emit_focus(widget.as_ref().expect("widget required"))?,
            "window_key" => {
                let key = args
                    .key
                    .as_deref()
                    .ok_or_else(|| "`window_key` requires a `key` argument".to_owned())?;
                session.harness.with_access(|access| {
                    access
                        .driver()
                        .dispatch_window_key_event(key, args.repeated.unwrap_or(false));
                });
            }
            other => {
                return Err(format!(
                    "unsupported GTK event `{other}`; use one of click, activate, set_text, set_active, focus, window_key"
                ));
            }
        }
        session
            .harness
            .with_access(|access| access.process_pending_work())?;
        self.settle_session()?;
        let after = self.list_signals(ListSignalsArgs::default())?;
        let gtk = self.snapshot_gtk_tree(SnapshotGtkArgs::default())?;
        Ok(EventResult {
            changed_signals: diff_signals(&before, &after),
            gtk,
            session: self.session_status()?,
        })
    }

    fn require_session(&self) -> Result<&HostedSession, String> {
        self.session
            .as_ref()
            .ok_or_else(|| "the app is not running; call `launch_app` first".to_owned())
    }

    fn process_context_work(&self) {
        while self.context.iteration(false) {}
    }

    fn settle_session(&self) -> Result<(), String> {
        let session = self.require_session()?;
        let deadline = Instant::now() + HYDRATION_SETTLE_TIMEOUT;
        loop {
            self.process_context_work();
            let pending = session.harness.with_access(|access| {
                access.latest_requested_hydration() != access.latest_applied_hydration()
            });
            if !pending {
                break;
            }
            if Instant::now() >= deadline {
                return Err("timed out waiting for GTK hydration to settle".to_owned());
            }
            thread::sleep(HOST_POLL_INTERVAL);
        }
        session
            .harness
            .with_access(|access| access.process_pending_work())?;
        Ok(())
    }

    fn root_widgets(&self) -> Vec<gtk::Widget> {
        self.session
            .as_ref()
            .map(|session| {
                session
                    .harness
                    .root_windows()
                    .iter()
                    .cloned()
                    .map(|window| window.upcast::<gtk::Widget>())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_widget_by_id(&mut self, widget_id: &str) -> Result<Option<gtk::Widget>, String> {
        let target = parse_widget_id(widget_id)?;
        for root in self.root_widgets() {
            if let Some(found) = self.find_widget_in_subtree(&root, target) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn find_widget_in_subtree(&mut self, widget: &gtk::Widget, target: u64) -> Option<gtk::Widget> {
        if self.widget_id_for(widget) == target {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = self.find_widget_in_subtree(&current, target) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    fn snapshot_widget(
        &mut self,
        widget: &gtk::Widget,
        path: Vec<String>,
    ) -> Result<WidgetSnapshot, String> {
        let id = self.widget_id_for(widget);
        let role = widget_role(widget);
        let text = widget_text(widget);
        let value = widget_value(widget)?;
        let mut children = Vec::new();
        let mut child = widget.first_child();
        let mut child_index = 0usize;
        while let Some(current) = child {
            let mut child_path = path.clone();
            child_path.push(format!("{}[{child_index}]", role));
            children.push(self.snapshot_widget(&current, child_path)?);
            child = current.next_sibling();
            child_index += 1;
        }
        Ok(WidgetSnapshot {
            id: format!("widget:{id}"),
            kind: widget.type_().name().to_owned(),
            role,
            text,
            value,
            visible: widget.is_visible(),
            sensitive: widget.is_sensitive(),
            focused: widget.has_focus(),
            actions: widget_actions(widget),
            path,
            children,
        })
    }

    fn widget_id_for(&mut self, widget: &gtk::Widget) -> u64 {
        let key = widget.as_ptr() as usize;
        if let Some(id) = self.widget_ids.get(&key) {
            return *id;
        }
        self.next_widget_id = self.next_widget_id.wrapping_add(1);
        let id = self.next_widget_id;
        self.widget_ids.insert(key, id);
        id
    }
}

#[derive(Clone)]
struct PreparedLaunch {
    entry_path: PathBuf,
    artifact: RunArtifact,
    launch_config: run_session::RunLaunchConfig,
}

fn prepare_launch_request(
    entry_path: &Path,
    requested_view: Option<String>,
    source_context: LaunchSourceArgs,
) -> Result<PreparedLaunch, String> {
    require_file_exists(entry_path)?;
    if let Some(view) = requested_view.as_deref() {
        validate_module_name(view)?;
    }
    let snapshot = WorkspaceHirSnapshot::load(entry_path)?;
    if let Some(diagnostics) = rendered_workspace_errors(&snapshot) {
        return Err(diagnostics);
    }
    let lowered = snapshot.entry_hir();
    let artifact = prepare_run_artifact(
        &snapshot.sources,
        lowered.module(),
        requested_view.as_deref(),
    )?;
    Ok(PreparedLaunch {
        entry_path: entry_path.to_path_buf(),
        artifact,
        launch_config: run_session::RunLaunchConfig::new(SourceProviderManager::with_context(
            build_source_context(source_context)?,
        )),
    })
}

fn resolve_initial_entry_path(
    current_dir: &Path,
    explicit_path: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    match resolve_v1_entrypoint(current_dir, explicit_path) {
        Ok(resolved) => Ok(Some(resolved.entry_path().to_path_buf())),
        Err(aivi_query::EntrypointResolutionError::MissingImplicitEntrypoint { .. })
            if explicit_path.is_none() =>
        {
            Ok(None)
        }
        Err(error) => Err(format!(
            "failed to resolve entrypoint for `aivi mcp`: {error}"
        )),
    }
}

fn resolve_launch_entry_path(
    configured: &ConfiguredTarget,
    args: &LaunchSourceArgs,
) -> Result<PathBuf, String> {
    if let Some(path) = &args.path {
        return Ok(PathBuf::from(path));
    }
    configured.entry_path.clone().ok_or_else(|| {
        "no app entrypoint is configured; pass `path` to `launch_app` or start `aivi mcp --path <entry-file>`".to_owned()
    })
}

fn effective_configured_target(
    controller: &McpHostController,
    fallback: &ConfiguredTarget,
) -> ConfiguredTarget {
    controller
        .call(|host| Ok(host.configured.clone()))
        .unwrap_or_else(|_| fallback.clone())
}

fn build_source_context(args: LaunchSourceArgs) -> Result<SourceProviderContext, String> {
    let cwd = match args.cwd {
        Some(cwd) => PathBuf::from(cwd),
        None => env::current_dir().map_err(|error| {
            format!("failed to determine current directory for source context: {error}")
        })?,
    };
    let mut env_vars = env::vars_os()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if let Some(overrides) = args.env {
        for (key, value) in overrides {
            env_vars.insert(key, value);
        }
    }
    let mut context = SourceProviderContext::new(args.args.unwrap_or_default(), cwd, env_vars);
    if let Some(stdin_text) = args.stdin_text {
        context = context.with_stdin_text(stdin_text);
    }
    Ok(context)
}

fn rendered_workspace_errors(snapshot: &WorkspaceHirSnapshot) -> Option<String> {
    let mut rendered = String::new();
    let mut saw_error = false;

    for file in &snapshot.files {
        let parsed = query_parsed_file(&snapshot.frontend.db, *file);
        for diagnostic in parsed.diagnostics() {
            rendered.push_str(&diagnostic.render(&snapshot.sources));
            rendered.push_str("\n\n");
            saw_error |= diagnostic.severity == Severity::Error;
        }
    }
    if saw_error {
        return Some(rendered.trim_end().to_owned());
    }

    for file in &snapshot.files {
        let hir = query_hir_module(&snapshot.frontend.db, *file);
        let mut file_lowering_failed = false;
        for diagnostic in hir.hir_diagnostics() {
            rendered.push_str(&diagnostic.render(&snapshot.sources));
            rendered.push_str("\n\n");
            file_lowering_failed |= diagnostic.severity == Severity::Error;
            saw_error |= diagnostic.severity == Severity::Error;
        }
        let validation_mode = if file_lowering_failed {
            ValidationMode::Structural
        } else {
            ValidationMode::RequireResolvedNames
        };
        let validation = hir.module().validate(validation_mode);
        for diagnostic in validation.diagnostics() {
            rendered.push_str(&diagnostic.render(&snapshot.sources));
            rendered.push_str("\n\n");
            saw_error |= diagnostic.severity == Severity::Error;
        }
    }

    saw_error.then(|| rendered.trim_end().to_owned())
}

fn run_host_loop(
    task_rx: sync_mpsc::Receiver<HostTask>,
    configured: ConfiguredTarget,
) -> Result<(), String> {
    let context = glib::MainContext::default();
    let mut host = McpHostState {
        context: context.clone(),
        configured,
        session: None,
        widget_ids: BTreeMap::new(),
        next_widget_id: 0,
        shutting_down: false,
    };

    while !host.shutting_down {
        host.process_context_work();
        match task_rx.recv_timeout(HOST_POLL_INTERVAL) {
            Ok(task) => task(&mut host),
            Err(sync_mpsc::RecvTimeoutError::Timeout) => continue,
            Err(sync_mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    host.stop_session();
    Ok(())
}

fn run_stdio_server(
    controller: McpHostController,
    configured: ConfiguredTarget,
) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Some(message) = read_json_rpc_message(&mut reader)? {
        let request: JsonRpcRequest = serde_json::from_value(message)
            .map_err(|error| format!("failed to decode MCP JSON-RPC request: {error}"))?;
        let Some(id) = request.id.clone() else {
            if request.method == "notifications/initialized" {
                continue;
            }
            continue;
        };
        let response = match handle_json_rpc_request(&controller, &configured, request) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": error.code,
                    "message": error.message,
                    "data": error.data,
                }
            }),
        };
        write_json_rpc_message(&mut writer, &response)?;
    }

    controller.shutdown();
    Ok(())
}

fn handle_json_rpc_request(
    controller: &McpHostController,
    configured: &ConfiguredTarget,
    request: JsonRpcRequest,
) -> Result<JsonValue, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "aivi",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": if configured.entry_path.is_some() {
                "Use launch_app to start the configured app, then inspect signals, GTK structure, and source state.".to_owned()
            } else {
                "Use launch_app with `path` to start an app, then inspect signals, GTK structure, and source state.".to_owned()
            }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tool_call(
            controller,
            configured,
            request.params.unwrap_or(JsonValue::Null),
        ),
        method => Err(JsonRpcError::method_not_found(method)),
    }
}

fn handle_tool_call(
    controller: &McpHostController,
    configured: &ConfiguredTarget,
    params: JsonValue,
) -> Result<JsonValue, JsonRpcError> {
    let request: ToolCallRequest = serde_json::from_value(params)
        .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
    let arguments = request
        .arguments
        .unwrap_or(JsonValue::Object(Default::default()));
    let result = match request.name.as_str() {
        "launch_app" => {
            let args: LaunchSourceArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let configured = effective_configured_target(controller, configured);
            let requested_view = args
                .view
                .clone()
                .or_else(|| configured.default_view.clone());
            let entry_path = resolve_launch_entry_path(&configured, &args)
                .map_err(JsonRpcError::tool_failure)?;
            let prepared = prepare_launch_request(&entry_path, requested_view, args)
                .map_err(JsonRpcError::tool_failure)?;
            let status = controller
                .call(move |host| host.launch_prepared(prepared))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!(
                    "Launched `{}`",
                    status.active_view.as_deref().unwrap_or("app")
                ),
                json!({ "session": status }),
            )
        }
        "restart_app" => {
            let args: LaunchSourceArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let configured = effective_configured_target(controller, configured);
            let requested_view = args
                .view
                .clone()
                .or_else(|| configured.default_view.clone());
            let entry_path = resolve_launch_entry_path(&configured, &args)
                .map_err(JsonRpcError::tool_failure)?;
            let prepared = prepare_launch_request(&entry_path, requested_view, args)
                .map_err(JsonRpcError::tool_failure)?;
            let status = controller
                .call(move |host| host.launch_prepared(prepared))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!(
                    "Restarted `{}`",
                    status.active_view.as_deref().unwrap_or("app")
                ),
                json!({ "session": status }),
            )
        }
        "stop_app" => {
            let status = controller
                .call(|host| host.stop_app())
                .map_err(JsonRpcError::tool_failure)?;
            tool_success("Stopped the app", json!({ "session": status }))
        }
        "session_status" => {
            let status = controller
                .call(|host| host.session_status())
                .map_err(JsonRpcError::tool_failure)?;
            tool_success("Fetched session status", json!({ "session": status }))
        }
        "list_signals" => {
            let args: ListSignalsArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let signals = controller
                .call(move |host| host.list_signals(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Listed {} signal(s)", signals.len()),
                json!({ "signals": signals }),
            )
        }
        "get_signal" => {
            let args: SignalSelector = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let signal = controller
                .call(move |host| host.get_signal(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Fetched signal {}", signal.id),
                json!({ "signal": signal }),
            )
        }
        "assert_signal" => {
            let args: AssertSignalArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let assertion = controller
                .call(move |host| host.assert_signal(args.selector, args.expected))
                .map_err(JsonRpcError::tool_failure)?;
            if assertion.matched {
                tool_success(
                    format!("Signal {} matched the expected value", assertion.signal.id),
                    json!({ "assertion": assertion }),
                )
            } else {
                tool_error(
                    format!(
                        "Signal {} did not match the expected value",
                        assertion.signal.id
                    ),
                    json!({ "assertion": assertion }),
                )
            }
        }
        "list_sources" => {
            let sources = controller
                .call(|host| host.list_sources())
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Listed {} source(s)", sources.len()),
                json!({ "sources": sources }),
            )
        }
        "set_source_mode" => {
            let args: SetSourceModeArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let result = controller
                .call(move |host| host.set_source_mode(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Set {} to {} mode", result.source.id, result.source.mode),
                json!({ "source": result.source }),
            )
        }
        "publish_source_value" => {
            let args: PublishSourceValueArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let result = controller
                .call(move |host| host.publish_source_value(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Published a new value into {}", result.source.id),
                json!({
                    "source": result.source,
                    "changedSignals": result.changed_signals,
                    "session": result.session,
                }),
            )
        }
        "snapshot_gtk_tree" => {
            let args: SnapshotGtkArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let gtk = controller
                .call(move |host| host.snapshot_gtk_tree(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Captured {} GTK root(s)", gtk.len()),
                json!({ "roots": gtk }),
            )
        }
        "find_widgets" => {
            let args: FindWidgetsArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let widgets = controller
                .call(move |host| host.find_widgets(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!("Found {} widget(s)", widgets.len()),
                json!({ "widgets": widgets }),
            )
        }
        "emit_gtk_event" => {
            let args: EmitGtkEventArgs = serde_json::from_value(arguments)
                .map_err(|error| JsonRpcError::invalid_params(error.to_string()))?;
            let result = controller
                .call(move |host| host.emit_gtk_event(args))
                .map_err(JsonRpcError::tool_failure)?;
            tool_success(
                format!(
                    "Applied the GTK event; {} signal(s) changed",
                    result.changed_signals.len()
                ),
                json!({
                    "changedSignals": result.changed_signals,
                    "gtk": result.gtk,
                    "session": result.session,
                }),
            )
        }
        other => return Err(JsonRpcError::method_not_found(other)),
    }?;
    Ok(result)
}

fn tool_definitions() -> Vec<JsonValue> {
    vec![
        json!({
            "name": "launch_app",
            "description": "Launch the configured AIVI app or the provided `path`, with optional source-context overrides.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "view": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "stdin_text": { "type": "string" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "restart_app",
            "description": "Restart the configured AIVI app or the provided `path`, with optional source-context overrides.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "view": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "stdin_text": { "type": "string" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "stop_app",
            "description": "Stop the current app session.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "session_status",
            "description": "Inspect app/session lifecycle status and hydration state.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "list_signals",
            "description": "List live runtime signals with stable IDs, owners, values, generations, and dependencies.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name_contains": { "type": "string" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "get_signal",
            "description": "Fetch one signal by stable ID or by unique name/owner path.",
            "inputSchema": signal_selector_schema()
        }),
        json!({
            "name": "assert_signal",
            "description": "Assert that one signal currently equals the expected JSON value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "owner_path": { "type": "string" },
                    "expected": {}
                },
                "required": ["expected"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "list_sources",
            "description": "List live source instances, their providers, effective configs, and live/manual state.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "set_source_mode",
            "description": "Switch a source instance between live and manual modes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_id": { "type": "string" },
                    "mode": { "type": "string", "enum": ["live", "manual"] }
                },
                "required": ["source_id", "mode"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "publish_source_value",
            "description": "Inject a decoded value into a source input. By default this enters manual mode first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_id": { "type": "string" },
                    "value": {},
                    "suspend_live": { "type": "boolean" }
                },
                "required": ["source_id", "value"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "snapshot_gtk_tree",
            "description": "Capture a semantic GTK widget tree for the live app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_hidden": { "type": "boolean" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "find_widgets",
            "description": "Search the semantic GTK snapshot for widgets by role, text, focus, or actionability.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text_contains": { "type": "string" },
                    "role": { "type": "string" },
                    "focused": { "type": "boolean" },
                    "actionable": { "type": "boolean" },
                    "include_hidden": { "type": "boolean" }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "emit_gtk_event",
            "description": "Emulate a supported GTK interaction on a live widget and wait for hydration to settle.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "widget_id": { "type": "string" },
                    "event": {
                        "type": "string",
                        "enum": ["click", "activate", "set_text", "set_active", "focus", "window_key"]
                    },
                    "text": { "type": "string" },
                    "active": { "type": "boolean" },
                    "key": { "type": "string" },
                    "repeated": { "type": "boolean" }
                },
                "required": ["event"],
                "additionalProperties": false
            }
        }),
    ]
}

fn signal_selector_schema() -> JsonValue {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "owner_path": { "type": "string" }
        },
        "additionalProperties": false
    })
}

fn tool_success(
    summary: impl Into<String>,
    structured: JsonValue,
) -> Result<JsonValue, JsonRpcError> {
    Ok(json!({
        "content": [{ "type": "text", "text": summary.into() }],
        "structuredContent": structured,
        "isError": false,
    }))
}

fn tool_error(
    summary: impl Into<String>,
    structured: JsonValue,
) -> Result<JsonValue, JsonRpcError> {
    Ok(json!({
        "content": [{ "type": "text", "text": summary.into() }],
        "structuredContent": structured,
        "isError": true,
    }))
}

fn read_json_rpc_message(reader: &mut impl BufRead) -> Result<Option<JsonValue>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read MCP header: {error}"))?;
        if read == 0 {
            return Ok(None);
        }
        if line == "\r\n" {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("invalid MCP header line: {line:?}"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid MCP Content-Length header: {error}"))?,
            );
        }
    }
    let content_length = content_length
        .ok_or_else(|| "MCP message is missing a Content-Length header".to_owned())?;
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("failed to read MCP message body: {error}"))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| format!("failed to parse MCP JSON body: {error}"))
}

fn write_json_rpc_message(writer: &mut impl Write, value: &JsonValue) -> Result<(), String> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| format!("failed to encode MCP JSON body: {error}"))?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())
        .map_err(|error| format!("failed to write MCP header: {error}"))?;
    writer
        .write_all(&payload)
        .map_err(|error| format!("failed to write MCP body: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush MCP response: {error}"))
}

fn snapshot_signals(
    driver: &aivi_runtime::GlibLinkedRuntimeDriver,
) -> Result<Vec<SignalSnapshot>, String> {
    let graph = driver.signal_graph();
    graph
        .signals()
        .map(|(handle, spec)| {
            let value = driver
                .current_signal_value(handle)
                .map_err(|error| format!("failed to read signal {}: {error}", handle.as_raw()))?
                .map(|value| runtime_json(&value))
                .transpose()?;
            let owner_path = spec.owner().map(|owner| owner_path(&graph, owner));
            let generation = spec
                .is_input()
                .then(|| {
                    driver
                        .current_generation(handle.as_input())
                        .map(|generation| generation.as_raw())
                        .map_err(|error| {
                            format!(
                                "failed to read generation for signal {}: {error}",
                                handle.as_raw()
                            )
                        })
                })
                .transpose()?;
            let dependencies = spec
                .kind()
                .as_derived()
                .map(|derived| {
                    derived
                        .dependencies()
                        .iter()
                        .map(|dependency| format!("signal:{}", dependency.as_raw()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(SignalSnapshot {
                id: format!("signal:{}", handle.as_raw()),
                name: spec.name().to_owned(),
                owner_path,
                kind: if spec.is_input() {
                    "input".to_owned()
                } else {
                    "derived".to_owned()
                },
                value,
                generation,
                dependencies,
            })
        })
        .collect()
}

fn snapshot_sources(
    driver: &aivi_runtime::GlibLinkedRuntimeDriver,
) -> Result<Vec<SourceSnapshot>, String> {
    driver
        .source_bindings()
        .into_iter()
        .map(|binding| {
            let config = driver
                .evaluate_source_config(binding.instance)
                .map_err(|error| {
                    format!(
                        "failed to evaluate source {}: {error}",
                        binding.instance.as_raw()
                    )
                })?;
            let arguments = config
                .arguments
                .iter()
                .map(runtime_json)
                .collect::<Result<Vec<_>, _>>()?;
            let options = config
                .options
                .iter()
                .map(|option| {
                    Ok((
                        option.option_name.as_ref().to_owned(),
                        runtime_json(&option.value)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?;
            Ok(SourceSnapshot {
                id: format!("source:{}", binding.instance.as_raw()),
                owner: format!("{}", binding.owner),
                signal_id: format!("signal:{}", binding.signal.as_raw()),
                input_signal_id: format!("signal:{}", binding.input.as_raw()),
                provider: format_source_provider(&config.provider),
                mode: match driver.source_mode(binding.instance) {
                    GlibLinkedSourceMode::Live => "live".to_owned(),
                    GlibLinkedSourceMode::Manual => "manual".to_owned(),
                },
                runtime_active: driver.is_source_active(binding.instance),
                provider_active: driver.has_active_provider(binding.instance),
                decode_program: config.decode.is_some(),
                arguments,
                options,
            })
        })
        .collect()
}

fn resolve_signal(
    signals: Vec<SignalSnapshot>,
    selector: &SignalSelector,
) -> Result<SignalSnapshot, String> {
    if let Some(id) = &selector.id {
        return signals
            .into_iter()
            .find(|signal| &signal.id == id)
            .ok_or_else(|| format!("no signal matches `{id}`"));
    }

    let matches = signals
        .into_iter()
        .filter(|signal| {
            selector
                .name
                .as_ref()
                .is_none_or(|name| &signal.name == name)
        })
        .filter(|signal| {
            selector
                .owner_path
                .as_ref()
                .is_none_or(|owner_path| signal.owner_path.as_ref() == Some(owner_path))
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err("the selector did not match any signal".to_owned()),
        [signal] => Ok(signal.clone()),
        _ => Err("the selector matched more than one signal; provide `id` or `owner_path` to disambiguate".to_owned()),
    }
}

fn diff_signals(before: &[SignalSnapshot], after: &[SignalSnapshot]) -> Vec<SignalSnapshot> {
    let previous = before
        .iter()
        .map(|signal| (signal.id.as_str(), signal))
        .collect::<BTreeMap<_, _>>();
    after
        .iter()
        .filter(|signal| {
            previous.get(signal.id.as_str()).is_none_or(|previous| {
                previous.value != signal.value || previous.generation != signal.generation
            })
        })
        .cloned()
        .collect()
}

fn owner_path(graph: &aivi_runtime::SignalGraph, owner: aivi_runtime::OwnerHandle) -> String {
    let mut segments = Vec::new();
    let mut current = Some(owner);
    while let Some(handle) = current {
        let spec = graph
            .owner(handle)
            .expect("owner handle produced by graph iteration should stay valid");
        segments.push(spec.name().to_owned());
        current = spec.parent();
    }
    segments.reverse();
    segments.join(".")
}

fn runtime_json(value: &DetachedRuntimeValue) -> Result<JsonValue, String> {
    let text = encode_runtime_json(value.as_runtime())
        .map_err(|error| format!("failed to encode runtime value as JSON: {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse encoded runtime JSON: {error}"))
}

fn runtime_value_from_source_json(
    value: &JsonValue,
    decode: Option<&aivi_hir::SourceDecodeProgram>,
) -> Result<RuntimeValue, String> {
    if let Some(program) = decode {
        let raw = serde_json::to_string(value)
            .map_err(|error| format!("failed to encode source payload JSON: {error}"))?;
        let external = parse_json_text(&raw)
            .map_err(|error| format!("failed to parse source payload JSON: {error}"))?;
        return decode_external(program, &external)
            .map_err(|error| format!("failed to decode source payload: {error}"));
    }
    runtime_value_from_json(value)
}

fn runtime_value_from_json(value: &JsonValue) -> Result<RuntimeValue, String> {
    match value {
        JsonValue::Null => Ok(RuntimeValue::Unit),
        JsonValue::Bool(value) => Ok(RuntimeValue::Bool(*value)),
        JsonValue::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Ok(RuntimeValue::Int(integer))
            } else {
                let float = value.as_f64().ok_or_else(|| {
                    format!("JSON number `{value}` is not representable as Float")
                })?;
                let runtime = RuntimeFloat::new(float)
                    .ok_or_else(|| format!("JSON number `{value}` is not a finite Float"))?;
                Ok(RuntimeValue::Float(runtime))
            }
        }
        JsonValue::String(value) => Ok(RuntimeValue::Text(value.clone().into_boxed_str())),
        JsonValue::Array(values) => values
            .iter()
            .map(runtime_value_from_json)
            .collect::<Result<Vec<_>, _>>()
            .map(RuntimeValue::List),
        JsonValue::Object(map) => {
            if let Some(tag) = map.get("tag").and_then(JsonValue::as_str) {
                let payload = map.get("payload");
                return match tag {
                    "None" if payload.is_none() => Ok(RuntimeValue::OptionNone),
                    "Some" => Ok(RuntimeValue::OptionSome(Box::new(runtime_value_from_json(
                        payload.ok_or_else(|| "`Some` requires a `payload` value".to_owned())?,
                    )?))),
                    "Ok" => Ok(RuntimeValue::ResultOk(Box::new(runtime_value_from_json(
                        payload.ok_or_else(|| "`Ok` requires a `payload` value".to_owned())?,
                    )?))),
                    "Err" => Ok(RuntimeValue::ResultErr(Box::new(runtime_value_from_json(
                        payload.ok_or_else(|| "`Err` requires a `payload` value".to_owned())?,
                    )?))),
                    "Valid" => Ok(RuntimeValue::ValidationValid(Box::new(
                        runtime_value_from_json(
                            payload
                                .ok_or_else(|| "`Valid` requires a `payload` value".to_owned())?,
                        )?,
                    ))),
                    "Invalid" => Ok(RuntimeValue::ValidationInvalid(Box::new(
                        runtime_value_from_json(
                            payload
                                .ok_or_else(|| "`Invalid` requires a `payload` value".to_owned())?,
                        )?,
                    ))),
                    _ => {
                        let fields = match payload {
                            None => Vec::new(),
                            Some(JsonValue::Array(values)) => values
                                .iter()
                                .map(runtime_value_from_json)
                                .collect::<Result<Vec<_>, _>>()?,
                            Some(value) => vec![runtime_value_from_json(value)?],
                        };
                        Ok(RuntimeValue::Sum(RuntimeSumValue {
                            item: aivi_hir::ItemId::from_raw(0),
                            type_name: tag.to_owned().into_boxed_str(),
                            variant_name: tag.to_owned().into_boxed_str(),
                            fields,
                        }))
                    }
                };
            }
            map.iter()
                .map(|(label, value)| {
                    Ok(RuntimeRecordField {
                        label: label.clone().into_boxed_str(),
                        value: runtime_value_from_json(value)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
                .map(RuntimeValue::Record)
        }
    }
}

fn widget_role(widget: &gtk::Widget) -> String {
    if widget.is::<gtk::Window>() {
        return "window".to_owned();
    }
    if widget.is::<gtk::Button>() {
        return "button".to_owned();
    }
    if widget.is::<gtk::Label>() {
        return "label".to_owned();
    }
    if widget.is::<gtk::Entry>() {
        return "entry".to_owned();
    }
    if widget.is::<gtk::Switch>() {
        return "switch".to_owned();
    }
    if widget.is::<gtk::CheckButton>() {
        return "check-button".to_owned();
    }
    if widget.is::<gtk::ToggleButton>() {
        return "toggle-button".to_owned();
    }
    if widget.is::<gtk::Box>() {
        return "box".to_owned();
    }
    if widget.is::<gtk::ScrolledWindow>() {
        return "scrolled-window".to_owned();
    }
    widget
        .type_()
        .name()
        .trim_start_matches("Gtk")
        .to_ascii_lowercase()
}

fn widget_text(widget: &gtk::Widget) -> Option<String> {
    if let Ok(window) = widget.clone().downcast::<gtk::Window>() {
        return window.title().map(|title| title.to_string());
    }
    if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
        return button.label().map(|label| label.to_string());
    }
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        return Some(label.label().to_string());
    }
    if let Ok(entry) = widget.clone().downcast::<gtk::Entry>() {
        return Some(entry.text().to_string());
    }
    None
}

fn widget_value(widget: &gtk::Widget) -> Result<Option<JsonValue>, String> {
    if let Ok(switch) = widget.clone().downcast::<gtk::Switch>() {
        return Ok(Some(JsonValue::Bool(switch.is_active())));
    }
    if let Ok(check) = widget.clone().downcast::<gtk::CheckButton>() {
        return Ok(Some(JsonValue::Bool(check.is_active())));
    }
    if let Ok(toggle) = widget.clone().downcast::<gtk::ToggleButton>() {
        return Ok(Some(JsonValue::Bool(toggle.is_active())));
    }
    if let Ok(entry) = widget.clone().downcast::<gtk::Entry>() {
        return Ok(Some(JsonValue::String(entry.text().to_string())));
    }
    Ok(None)
}

fn widget_actions(widget: &gtk::Widget) -> Vec<String> {
    let mut actions = Vec::new();
    if widget.is::<gtk::Button>() {
        actions.push("click".to_owned());
    }
    if widget.is::<gtk::Entry>() {
        actions.push("set_text".to_owned());
    }
    if widget.is::<gtk::Switch>()
        || widget.is::<gtk::CheckButton>()
        || widget.is::<gtk::ToggleButton>()
    {
        actions.push("set_active".to_owned());
    }
    if widget.can_focus() {
        actions.push("focus".to_owned());
    }
    if widget.is::<gtk::Window>() {
        actions.push("window_key".to_owned());
    }
    actions
}

fn collect_widget_matches(
    snapshot: &WidgetSnapshot,
    query: &FindWidgetsArgs,
    out: &mut Vec<WidgetMatch>,
) {
    let text_matches = query.text_contains.as_ref().is_none_or(|needle| {
        snapshot
            .text
            .as_ref()
            .is_some_and(|text| text.contains(needle))
    });
    let role_matches = query
        .role
        .as_ref()
        .is_none_or(|role| &snapshot.role == role);
    let focus_matches = query
        .focused
        .is_none_or(|focused| snapshot.focused == focused);
    let actionable_matches = query
        .actionable
        .is_none_or(|actionable| !actionable || !snapshot.actions.is_empty());
    if text_matches && role_matches && focus_matches && actionable_matches {
        out.push(WidgetMatch {
            id: snapshot.id.clone(),
            kind: snapshot.kind.clone(),
            role: snapshot.role.clone(),
            text: snapshot.text.clone(),
            value: snapshot.value.clone(),
            visible: snapshot.visible,
            sensitive: snapshot.sensitive,
            focused: snapshot.focused,
            actions: snapshot.actions.clone(),
            path: snapshot.path.clone(),
        });
    }
    for child in &snapshot.children {
        collect_widget_matches(child, query, out);
    }
}

fn emit_activate_event(widget: &gtk::Widget) -> Result<(), String> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
        button.emit_clicked();
        return Ok(());
    }
    Err(format!(
        "widget `{}` only supports activation for gtk::Button right now",
        widget.type_().name()
    ))
}

fn emit_set_text(widget: &gtk::Widget, text: &str) -> Result<(), String> {
    if let Ok(entry) = widget.clone().downcast::<gtk::Entry>() {
        entry.set_text(text);
        return Ok(());
    }
    Err(format!(
        "widget `{}` does not support `set_text`",
        widget.type_().name()
    ))
}

fn emit_set_active(widget: &gtk::Widget, active: bool) -> Result<(), String> {
    if let Ok(switch) = widget.clone().downcast::<gtk::Switch>() {
        switch.set_active(active);
        return Ok(());
    }
    if let Ok(check) = widget.clone().downcast::<gtk::CheckButton>() {
        check.set_active(active);
        return Ok(());
    }
    if let Ok(toggle) = widget.clone().downcast::<gtk::ToggleButton>() {
        toggle.set_active(active);
        return Ok(());
    }
    Err(format!(
        "widget `{}` does not support `set_active`",
        widget.type_().name()
    ))
}

fn emit_focus(widget: &gtk::Widget) -> Result<(), String> {
    if widget.grab_focus() {
        Ok(())
    } else {
        Err(format!("widget `{}` refused focus", widget.type_().name()))
    }
}

fn parse_source_id(text: &str) -> Result<aivi_runtime::SourceInstanceId, String> {
    parse_prefixed_u32(text, "source:")
        .map(aivi_runtime::SourceInstanceId::from_raw)
        .map_err(|error| format!("invalid source id `{text}`: {error}"))
}

fn parse_widget_id(text: &str) -> Result<u64, String> {
    parse_prefixed_u64(text, "widget:")
        .map_err(|error| format!("invalid widget id `{text}`: {error}"))
}

fn parse_source_mode(text: &str) -> Result<GlibLinkedSourceMode, String> {
    match text {
        "live" => Ok(GlibLinkedSourceMode::Live),
        "manual" => Ok(GlibLinkedSourceMode::Manual),
        other => Err(format!(
            "unsupported source mode `{other}`; expected `live` or `manual`"
        )),
    }
}

fn parse_prefixed_u32(text: &str, prefix: &str) -> Result<u32, String> {
    text.strip_prefix(prefix)
        .unwrap_or(text)
        .parse::<u32>()
        .map_err(|error| error.to_string())
}

fn parse_prefixed_u64(text: &str, prefix: &str) -> Result<u64, String> {
    text.strip_prefix(prefix)
        .unwrap_or(text)
        .parse::<u64>()
        .map_err(|error| error.to_string())
}

fn phase_label(phase: run_session::RunSessionPhase) -> &'static str {
    match phase {
        run_session::RunSessionPhase::Starting => "starting",
        run_session::RunSessionPhase::Running => "running",
        run_session::RunSessionPhase::Stopped => "stopped",
    }
}

fn format_source_provider(provider: &aivi_runtime::RuntimeSourceProvider) -> String {
    match provider {
        aivi_runtime::RuntimeSourceProvider::Builtin(provider) => format!("{provider:?}"),
        aivi_runtime::RuntimeSourceProvider::Custom(provider) => provider.to_string(),
    }
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<JsonValue>,
    method: String,
    params: Option<JsonValue>,
}

#[derive(Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
    data: Option<JsonValue>,
}

impl JsonRpcError {
    fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method `{method}` is not supported"),
            data: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    fn tool_failure(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Default, Deserialize)]
struct ToolCallRequest {
    name: String,
    arguments: Option<JsonValue>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct LaunchSourceArgs {
    path: Option<String>,
    view: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<BTreeMap<String, String>>,
    stdin_text: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct ListSignalsArgs {
    name_contains: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct SignalSelector {
    id: Option<String>,
    name: Option<String>,
    owner_path: Option<String>,
}

#[derive(Deserialize)]
struct AssertSignalArgs {
    #[serde(flatten)]
    selector: SignalSelector,
    expected: JsonValue,
}

#[derive(Clone, Default, Deserialize)]
struct SnapshotGtkArgs {
    include_hidden: Option<bool>,
}

#[derive(Clone, Default, Deserialize)]
struct FindWidgetsArgs {
    text_contains: Option<String>,
    role: Option<String>,
    focused: Option<bool>,
    actionable: Option<bool>,
    include_hidden: Option<bool>,
}

#[derive(Clone, Deserialize)]
struct EmitGtkEventArgs {
    widget_id: String,
    event: String,
    text: Option<String>,
    active: Option<bool>,
    key: Option<String>,
    repeated: Option<bool>,
}

#[derive(Clone, Deserialize)]
struct SetSourceModeArgs {
    source_id: String,
    mode: String,
}

#[derive(Clone, Deserialize)]
struct PublishSourceValueArgs {
    source_id: String,
    value: JsonValue,
    suspend_live: Option<bool>,
}

#[derive(Serialize)]
struct SessionStatus {
    launched: bool,
    configured_entry_path: Option<String>,
    configured_view: Option<String>,
    active_entry_path: Option<String>,
    active_view: Option<String>,
    phase: Option<String>,
    root_window_count: usize,
    latest_requested_hydration: Option<u64>,
    latest_applied_hydration: Option<u64>,
    queued_messages: Option<usize>,
    queued_outcomes: Option<usize>,
    queued_failures: Option<usize>,
}

struct SessionRuntimeStatus {
    phase: String,
    latest_requested_hydration: Option<u64>,
    latest_applied_hydration: Option<u64>,
    queued_messages: usize,
    queued_outcomes: usize,
    queued_failures: usize,
}

#[derive(Clone, Serialize)]
struct SignalSnapshot {
    id: String,
    name: String,
    owner_path: Option<String>,
    kind: String,
    value: Option<JsonValue>,
    generation: Option<u64>,
    dependencies: Vec<String>,
}

#[derive(Serialize)]
struct SignalAssertResult {
    selector: SignalSelector,
    matched: bool,
    actual: Option<JsonValue>,
    expected: JsonValue,
    signal: SignalSnapshot,
}

#[derive(Clone, Serialize)]
struct SourceSnapshot {
    id: String,
    owner: String,
    signal_id: String,
    input_signal_id: String,
    provider: String,
    mode: String,
    runtime_active: bool,
    provider_active: bool,
    decode_program: bool,
    arguments: Vec<JsonValue>,
    options: BTreeMap<String, JsonValue>,
}

#[derive(Serialize)]
struct SourceModeResult {
    source: SourceSnapshot,
}

#[derive(Serialize)]
struct SourcePublishResult {
    source: SourceSnapshot,
    changed_signals: Vec<SignalSnapshot>,
    session: SessionStatus,
}

#[derive(Clone, Serialize)]
struct WidgetSnapshot {
    id: String,
    kind: String,
    role: String,
    text: Option<String>,
    value: Option<JsonValue>,
    visible: bool,
    sensitive: bool,
    focused: bool,
    actions: Vec<String>,
    path: Vec<String>,
    children: Vec<WidgetSnapshot>,
}

#[derive(Serialize)]
struct WidgetMatch {
    id: String,
    kind: String,
    role: String,
    text: Option<String>,
    value: Option<JsonValue>,
    visible: bool,
    sensitive: bool,
    focused: bool,
    actions: Vec<String>,
    path: Vec<String>,
}

#[derive(Serialize)]
struct EventResult {
    changed_signals: Vec<SignalSnapshot>,
    gtk: Vec<WidgetSnapshot>,
    session: SessionStatus,
}

#[cfg(test)]
mod tests {
    use super::{
        ConfiguredTarget, JsonRpcError, JsonRpcRequest, MCP_PROTOCOL_VERSION, McpHostController,
        handle_json_rpc_request, parse_prefixed_u32, parse_prefixed_u64, read_json_rpc_message,
        resolve_initial_entry_path, runtime_value_from_json, write_json_rpc_message,
    };
    use aivi_backend::RuntimeValue;
    use serde_json::{Value as JsonValue, json};
    use std::{io::BufReader, path::PathBuf, sync::mpsc as sync_mpsc};

    #[test]
    fn prefixed_ids_accept_raw_and_prefixed_forms() {
        assert_eq!(parse_prefixed_u32("source:7", "source:").unwrap(), 7);
        assert_eq!(parse_prefixed_u32("7", "source:").unwrap(), 7);
        assert_eq!(parse_prefixed_u64("widget:9", "widget:").unwrap(), 9);
        assert_eq!(parse_prefixed_u64("9", "widget:").unwrap(), 9);
    }

    #[test]
    fn runtime_json_decoder_supports_core_shapes() {
        assert_eq!(
            runtime_value_from_json(&json!(null)).unwrap(),
            RuntimeValue::Unit
        );
        assert_eq!(
            runtime_value_from_json(&json!(true)).unwrap(),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            runtime_value_from_json(&json!(7)).unwrap(),
            RuntimeValue::Int(7)
        );
        assert_eq!(
            runtime_value_from_json(&json!({"tag": "Some", "payload": 7})).unwrap(),
            RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(7)))
        );
        assert_eq!(
            runtime_value_from_json(&json!({"tag": "Ready", "payload": [1, 2]})).unwrap(),
            RuntimeValue::Sum(aivi_backend::RuntimeSumValue {
                item: aivi_hir::ItemId::from_raw(0),
                type_name: "Ready".into(),
                variant_name: "Ready".into(),
                fields: vec![RuntimeValue::Int(1), RuntimeValue::Int(2)],
            })
        );
    }

    #[test]
    fn json_rpc_framing_round_trips_payloads() {
        let value = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": { "ok": true }
        });
        let mut buffer = Vec::new();
        write_json_rpc_message(&mut buffer, &value).expect("message should encode");
        let decoded = read_json_rpc_message(&mut BufReader::new(buffer.as_slice()))
            .expect("message should decode")
            .expect("reader should yield a message");
        assert_eq!(decoded, value);
    }

    #[test]
    fn initialize_and_tools_list_expose_the_mcp_surface() {
        let (task_tx, _task_rx) = sync_mpsc::channel();
        let controller = McpHostController { task_tx };
        let configured = ConfiguredTarget {
            entry_path: Some(PathBuf::from("fixtures/snake/main.aivi")),
            default_view: None,
        };
        let initialize = handle_json_rpc_request(
            &controller,
            &configured,
            JsonRpcRequest {
                jsonrpc: "2.0".to_owned(),
                id: Some(JsonValue::from(1)),
                method: "initialize".to_owned(),
                params: Some(json!({})),
            },
        )
        .expect("initialize should succeed");
        assert_eq!(initialize["protocolVersion"], json!(MCP_PROTOCOL_VERSION));
        assert_eq!(initialize["serverInfo"]["name"], json!("aivi"));
        assert!(
            initialize["instructions"]
                .as_str()
                .expect("initialize instructions should be text")
                .contains("configured app")
        );

        let tools = handle_json_rpc_request(
            &controller,
            &configured,
            JsonRpcRequest {
                jsonrpc: "2.0".to_owned(),
                id: Some(JsonValue::from(2)),
                method: "tools/list".to_owned(),
                params: Some(json!({})),
            },
        )
        .expect("tools/list should succeed");
        let tool_names: Vec<&str> = tools["tools"]
            .as_array()
            .expect("tools/list should return an array")
            .iter()
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .expect("tool definitions should include a string name")
            })
            .collect();
        assert_eq!(
            tool_names,
            vec![
                "launch_app",
                "restart_app",
                "stop_app",
                "session_status",
                "list_signals",
                "get_signal",
                "assert_signal",
                "list_sources",
                "set_source_mode",
                "publish_source_value",
                "snapshot_gtk_tree",
                "find_widgets",
                "emit_gtk_event",
            ]
        );
        let launch_schema = &tools["tools"][0]["inputSchema"]["properties"];
        assert!(
            launch_schema.get("path").is_some(),
            "launch_app should advertise an optional explicit path"
        );
    }

    #[test]
    fn resolve_initial_entry_path_allows_missing_implicit_main() {
        let temp = tempfile::tempdir().expect("tempdir should create");
        let resolved = resolve_initial_entry_path(temp.path(), None)
            .expect("missing implicit main is allowed");
        assert!(
            resolved.is_none(),
            "server startup should remain unbound when no implicit main exists"
        );
    }

    #[test]
    fn launch_app_requires_path_when_server_starts_unbound() {
        let (task_tx, task_rx) = sync_mpsc::channel();
        drop(task_rx);
        let controller = McpHostController { task_tx };
        let configured = ConfiguredTarget {
            entry_path: None,
            default_view: None,
        };
        let error = handle_json_rpc_request(
            &controller,
            &configured,
            JsonRpcRequest {
                jsonrpc: "2.0".to_owned(),
                id: Some(JsonValue::from(3)),
                method: "tools/call".to_owned(),
                params: Some(json!({
                    "name": "launch_app",
                    "arguments": {}
                })),
            },
        )
        .expect_err("unbound launch_app should fail with an actionable message");
        assert_eq!(error.code, JsonRpcError::tool_failure("").code);
        assert!(
            error.message.contains("pass `path`"),
            "tool failure should tell the caller how to bind an app"
        );
    }
}
