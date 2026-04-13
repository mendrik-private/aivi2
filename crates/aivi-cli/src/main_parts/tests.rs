use super::{
    HydratedRunNode, ResolvedRunEventHandler, ResolvedRunEventPayload, RunHydrationStaticState,
    WorkspaceHirSnapshot, check_file, execute_file_with_context, plan_run_hydration,
    plan_run_hydration_profiled, prepare_execute_artifact, prepare_run_artifact,
    run_hydration_globals_ready, test_file_with_context,
};
use aivi_backend::{DetachedRuntimeValue, RuntimeTaskPlan, RuntimeValue};
use aivi_base::SourceDatabase;
use aivi_gtk::{GtkBridgeNodeKind, RuntimePropertyBinding, RuntimeShowMountPolicy};
use aivi_hir::{BuiltinType, ImportValueType, ValidationMode, lower_module as lower_hir_module};
use aivi_runtime::{SourceProviderContext, execute_runtime_task_plan};
use aivi_syntax::parse_module;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
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
    super::prepare_run_artifact_with_query_context(
        &snapshot.sources,
        lowered.module(),
        &[],
        requested_view,
        Some(snapshot.backend_query_context()),
    )
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
    super::prepare_run_artifact_with_query_context(
        &snapshot.sources,
        lowered.module(),
        &[],
        requested_view,
        Some(snapshot.backend_query_context()),
    )
}

#[test]
fn stub_signal_defaults_skip_named_payloads() {
    let named = ImportValueType::Named {
        type_name: "Message".into(),
        arguments: Vec::new(),
        definition: None,
    };
    assert_eq!(super::default_runtime_value_for_import_type(&named), None);
    assert_eq!(
        super::default_runtime_value_for_import_type(&ImportValueType::Option(Box::new(
            ImportValueType::Primitive(BuiltinType::Text),
        ))),
        Some(RuntimeValue::OptionNone)
    );
}

#[test]
fn resolve_run_entrypoint_prefers_explicit_path_over_implicit_workspace_main() {
    let workspace = TempDir::new("run-entry-explicit");
    workspace.write("aivi.toml", "");
    let cwd = workspace.path().join("tooling");
    fs::create_dir_all(&cwd).expect("tooling directory should exist");
    let explicit = workspace.write("apps/demo.aivi", "value demo = 1\n");

    let resolved = super::resolve_run_entrypoint(&cwd, Some(&explicit), None)
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

    let resolved = super::resolve_run_entrypoint(&cwd, None, None)
        .expect("implicit resolution should use workspace-root main.aivi");

    assert_eq!(resolved.entry_path, expected);
}

#[test]
fn resolve_run_entrypoint_reports_missing_implicit_main_with_path_hint() {
    let workspace = TempDir::new("run-entry-missing");
    workspace.write("aivi.toml", "");
    let cwd = workspace.path().join("tooling");
    fs::create_dir_all(&cwd).expect("tooling directory should exist");

    let error = super::resolve_run_entrypoint(&cwd, None, None)
        .expect_err("missing main.aivi should fail without guessing");

    assert!(error.contains("failed to resolve entrypoint for `aivi run`"));
    assert!(error.contains(&workspace.path().join("main.aivi").display().to_string()));
    assert!(error.contains("--path <entry-file>") || error.contains("aivi.toml"));
}

#[test]
fn resolve_run_entrypoint_uses_manifest_run_entry_when_multiple_apps_exist() {
    let workspace = TempDir::new("run-entry-manifest-default");
    workspace.write(
        "aivi.toml",
        concat!(
            "[run]\n",
            "entry = \"apps/ui/main.aivi\"\n",
            "\n",
            "[[app]]\n",
            "name = \"ui\"\n",
            "entry = \"apps/ui/main.aivi\"\n",
            "\n",
            "[[app]]\n",
            "name = \"tray\"\n",
            "entry = \"apps/tray/main.aivi\"\n",
        ),
    );
    let expected = workspace.write(
        "apps/ui/main.aivi",
        "value main = <Window title=\"UI\" />\n",
    );
    workspace.write(
        "apps/tray/main.aivi",
        "value quickCompose = <Window title=\"Tray\" />\n",
    );
    let cwd = workspace.path().join("tooling/nested");
    fs::create_dir_all(&cwd).expect("nested tooling directory should exist");

    let resolved = super::resolve_run_entrypoint(&cwd, None, None)
        .expect("manifest [run] entry should be the default run target");

    assert_eq!(resolved.entry_path, expected);
    assert_eq!(resolved.manifest_view, None);
}

fn execute_workspace(path: &Path, context: SourceProviderContext) -> (ExitCode, String, String) {
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
    let result = check_file(
        &fixture("milestone-2/invalid/unknown-decorator/main.aivi"),
        false,
    )
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
fn prepare_run_tracks_transitive_signal_globals_for_parameterized_from_selectors() {
    let artifact = prepare_run_from_text(
        "parameterized-from-run.aivi",
        r#"
type State = { count: Int }

type Int -> State -> Bool
func atLeastFromState = threshold state => state.count >= threshold

signal state : Signal State = { count: 1 }

from state = {
    type Int -> Bool
    atLeast threshold: atLeastFromState threshold
}

value view =
    <Window title="AIVI">
        <Button label="Go" sensitive={atLeast 0} />
    </Window>
"#,
        None,
    )
    .expect("parameterized from-selector view should prepare for run");
    let required = artifact
        .required_signal_globals
        .values()
        .map(|name| name.as_ref())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"state"),
        "hydration fragments calling parameterized from-selectors should project their transitive signal dependency"
    );
}

#[test]
fn prepare_run_accepts_truthy_falsy_parameterized_from_selectors_with_same_block_signals() {
    let artifact = prepare_run_from_text(
        "parameterized-from-truthy-falsy-run.aivi",
        r#"
type State = { ready: Bool, label: Text }

signal state : Signal State = { ready: True, label: "Go" }

from state = {
    ready: .ready
    baseLabel: .label

    type Text -> Text
    cellLabel fallback: ready
     T|> baseLabel
     F|> fallback
}

value view =
    <Window title="AIVI">
        <Button label={cellLabel "."} />
    </Window>
"#,
        None,
    )
    .expect("same-block truthy/falsy parameterized from-selector view should prepare for run");
    assert_eq!(artifact.view_name.as_ref(), "view");
    let required = artifact
        .required_signal_globals
        .values()
        .map(|name| name.as_ref())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"state"),
        "truthy/falsy parameterized from-selector fragments should keep the source signal as a runtime dependency"
    );
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
fn run_fragment_compiler_reuses_query_backend_units_and_execution_cache() {
    let workspace = TempDir::new("run-fragment-query-cache");
    let entry = workspace.write(
        "main.aivi",
        r#"
value title = "AIVI"

value view =
    <Window title={title} />
"#,
    );
    let snapshot = WorkspaceHirSnapshot::load(&entry).expect("workspace snapshot should load");
    let lowered = snapshot.entry_hir();
    let module = lowered.module();
    let view = super::select_run_view(module, None).expect("view should resolve");
    let view_owner = super::find_value_owner(module, view).expect("view owner should resolve");
    let plan = super::lower_markup_expr(module, view.body)
        .expect("view markup should lower into a GTK plan");
    let bridge = super::lower_widget_bridge(&plan).expect("GTK plan should lower into a bridge");
    let sites = super::collect_markup_runtime_expr_sites(module, view.body)
        .expect("runtime expression sites should collect");
    let included_items = super::production_item_ids(module);
    let runtime_stack =
        super::lower_runtime_backend_stack_with_items_fast(module, &included_items, "`aivi run`")
            .expect("runtime backend stack should lower");
    let runtime_backend_by_hir =
        super::backend_items_by_hir(&runtime_stack.core, runtime_stack.backend.as_ref());
    let expr = super::collect_run_input_specs_from_bridge(module, &bridge)
        .into_values()
        .find_map(|spec| match spec {
            super::RunInputSpec::Expr(expr) => Some(expr),
            super::RunInputSpec::Text(_) => None,
        })
        .expect("dynamic title should produce one runtime expression input");

    let mut compiler = super::RunFragmentCompiler::new(
        &snapshot.sources,
        module,
        view_owner,
        &sites,
        runtime_stack.backend.as_ref(),
        &runtime_backend_by_hir,
        Some(snapshot.backend_query_context()),
    );
    let (first, compiled_now) = compiler
        .compile(expr)
        .expect("first compilation should succeed");
    let (second, compiled_again) = compiler
        .compile(expr)
        .expect("cached recompilation should succeed");

    assert!(compiled_now);
    assert!(!compiled_again);
    assert!(Arc::ptr_eq(&first.execution, &second.execution));

    let mut second_compiler = super::RunFragmentCompiler::new(
        &snapshot.sources,
        module,
        view_owner,
        &sites,
        runtime_stack.backend.as_ref(),
        &runtime_backend_by_hir,
        Some(snapshot.backend_query_context()),
    );
    let (third, _) = second_compiler
        .compile(expr)
        .expect("second compiler should reuse the query-backed fragment backend");

    assert!(Arc::ptr_eq(
        &first.execution.backend,
        &third.execution.backend
    ));
    assert_eq!(first.item, third.item);
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
        patterns: artifact.patterns.clone(),
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
fn run_hydration_profile_tracks_fragment_and_kernel_activity() {
    let artifact = prepare_run_from_text("planner-window.aivi", planner_window_source(), None)
        .expect("planner window should compile for live run hydration");
    let shared = RunHydrationStaticState {
        view_name: artifact.view_name.clone(),
        patterns: artifact.patterns.clone(),
        bridge: artifact.bridge.clone(),
        inputs: artifact.hydration_inputs.clone(),
    };

    let (_plan, profile) = plan_run_hydration_profiled(&shared, &BTreeMap::new())
        .expect("planner window should produce a hydration profile");
    let temp = TempDir::new("run-artifact-profile-roundtrip");
    let artifact_path = super::write_serialized_run_artifact_bundle(temp.path(), &artifact)
        .expect("run artifact bundle should write");
    let reloaded = super::load_serialized_run_artifact(&artifact_path, None)
        .expect("serialized run artifact should reload");
    let reloaded_plan = plan_run_hydration(
        &RunHydrationStaticState {
            view_name: reloaded.view_name.clone(),
            patterns: reloaded.patterns.clone(),
            bridge: reloaded.bridge.clone(),
            inputs: reloaded.hydration_inputs.clone(),
        },
        &BTreeMap::new(),
    )
    .expect("reloaded artifact should hydrate");

    assert!(profile.planned_nodes > 0);
    assert!(profile.evaluated_inputs > 0);
    assert!(!profile.fragment_profiles.is_empty());
    assert!(!profile.program_profiles.is_empty());
    assert_eq!(artifact.view_name, reloaded.view_name);
    assert_eq!(artifact.patterns, reloaded.patterns);
    assert_eq!(artifact.bridge, reloaded.bridge);
    assert_eq!(artifact.event_handlers.len(), reloaded.event_handlers.len());
    assert_eq!(
        artifact.required_signal_globals,
        reloaded.required_signal_globals
    );
    assert!(
        !reloaded.backend_native_kernels.is_empty(),
        "serialized run artifact should reload precompiled native kernel sidecars"
    );
    assert!(matches!(reloaded_plan.root, HydratedRunNode::Widget { .. }));
    assert!(
        profile
            .program_profiles
            .values()
            .any(|program| !program.kernels.is_empty())
    );
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
value canFocus = False
value view =
    <Window title="Host">
        <Window.titlebar>
            <HeaderBar showTitleButtons={showButtons}>
                <HeaderBar.start>
                    <Label text="Status" />
                </HeaderBar.start>
                <HeaderBar.end>
                    <Button label="Restart" focusable={canFocus} compact hasFrame={False} widthRequest={26} heightRequest={26} />
                </HeaderBar.end>
            </HeaderBar>
        </Window.titlebar>
        <Button label="A" focusable={canFocus} compact hasFrame={False} widthRequest={26} heightRequest={26} />
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
    assert!(property_names.iter().any(|name| name == "focusable"));
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

type Text -> Task Text Bool
func mockedProbe = path=>    exists "{cwd}/flag.txt"

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

type Text -> Task Text Bool
func probe = path=>    exists path

@test
value service_smoke : Task Text Bool =
    exists "{cwd}/flag.txt"
"#,
    );
    fs::write(workspace.path().join("flag.txt"), "ok").expect("test fixture should be writable");

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
