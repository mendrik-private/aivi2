use std::{collections::BTreeMap, sync::Arc};

use aivi_backend::{RuntimeCustomCapabilityCommandPlan, RuntimeNamedValue};
use aivi_base::SourceDatabase;
use aivi_hir::{Item, lower_module as lower_hir_module};
use aivi_lambda::lower_module as lower_lambda_module;
use aivi_syntax::parse_module;

use super::*;
use crate::{
    SignalGraphBuilder, TaskRuntimeSpec, TaskSourceRuntime,
    task_executor::CustomCapabilityCommandExecutor,
};

#[derive(Default)]
struct EchoCustomCapabilityCommandExecutor;

impl CustomCapabilityCommandExecutor for EchoCustomCapabilityCommandExecutor {
    fn execute(
        &self,
        _context: &SourceProviderContext,
        plan: &RuntimeCustomCapabilityCommandPlan,
        _stdout: &mut dyn std::io::Write,
        _stderr: &mut dyn std::io::Write,
    ) -> Result<RuntimeValue, crate::task_executor::RuntimeTaskExecutionError> {
        assert_eq!(plan.provider_key.as_ref(), "custom.feed");
        assert_eq!(plan.command.as_ref(), "delete");
        assert_eq!(
            plan.provider_arguments.as_ref(),
            [RuntimeNamedValue {
                name: "root".into(),
                value: RuntimeValue::Text("/tmp/demo".into()),
            }]
        );
        assert_eq!(
            plan.options.as_ref(),
            [RuntimeNamedValue {
                name: "mode".into(),
                value: RuntimeValue::Text("sync".into()),
            }]
        );
        assert_eq!(
            plan.arguments.as_ref(),
            [RuntimeNamedValue {
                name: "arg1".into(),
                value: RuntimeValue::Text("config".into()),
            }]
        );
        Ok(RuntimeValue::Text("deleted".into()))
    }
}

struct LoweredStack {
    hir: hir::LoweringResult,
    core: core::Module,
    backend: BackendProgram,
}

fn lower_text(path: &str, text: &str) -> LoweredStack {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let hir = lower_hir_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "fixture {path} should lower to HIR: {:?}",
        hir.diagnostics()
    );
    let core = core::lower_module(hir.module()).expect("typed-core lowering should succeed");
    let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
    let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
    LoweredStack { hir, core, backend }
}

fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(item) if item.name.text() == name => Some(item_id),
            Item::Function(item) if item.name.text() == name => Some(item_id),
            Item::Signal(item) if item.name.text() == name => Some(item_id),
            Item::Type(item) if item.name.text() == name => Some(item_id),
            Item::Class(item) if item.name.text() == name => Some(item_id),
            Item::Domain(item) if item.name.text() == name => Some(item_id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected item named {name}"))
}

fn backend_item_id(program: &BackendProgram, name: &str) -> BackendItemId {
    program
        .items()
        .iter()
        .find_map(|(item_id, item)| (item.name.as_ref() == name).then_some(item_id))
        .unwrap_or_else(|| panic!("expected backend item named {name}"))
}

fn text_ptr(value: &RuntimeValue) -> *const u8 {
    let RuntimeValue::Text(text) = value else {
        panic!("expected text runtime value");
    };
    text.as_ptr()
}

fn manual_task_linked_runtime(lowered: &LoweredStack, owner_name: &str) -> BackendLinkedRuntime {
    let assembly = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("manual task fixture should assemble");
    let owner = item_id(lowered.hir.module(), owner_name);
    let backend_item = backend_item_id(&lowered.backend, owner_name);

    let mut graph = SignalGraphBuilder::new();
    let owner_handle = graph
        .add_owner(owner_name, None)
        .expect("task owner should allocate");
    let input = graph
        .add_input(format!("{owner_name}#task"), Some(owner_handle))
        .expect("task input should allocate");
    let graph = graph.build().expect("task graph should build");

    let mut runtime: TaskSourceRuntime<
        RuntimeValue,
        hir::SourceDecodeProgram,
        MovingRuntimeValueStore,
    > = TaskSourceRuntime::with_value_store(graph, MovingRuntimeValueStore::default());
    let instance = TaskInstanceId::from_raw(owner.as_raw());
    runtime
        .register_task(TaskRuntimeSpec::new(instance, input))
        .expect("task spec should register");

    let kernel = lowered.backend.items()[backend_item]
        .body
        .expect("manual task fixture should have a lowered backend body");
    let binding = LinkedTaskBinding {
        owner,
        owner_handle,
        input,
        instance,
        backend_item,
        execution: LinkedTaskExecutionBinding::Ready {
            kernel,
            required_signals: Vec::new().into_boxed_slice(),
        },
    };

    BackendLinkedRuntime {
        assembly,
        runtime,
        backend: Arc::new(lowered.backend.clone()),
        signal_items_by_handle: BTreeMap::new(),
        runtime_signal_by_item: BTreeMap::new(),
        derived_signals: BTreeMap::new(),
        reactive_signals: BTreeMap::new(),
        reactive_clauses: BTreeMap::new(),
        linked_recurrence_signals: BTreeMap::new(),
        source_bindings: BTreeMap::new(),
        task_bindings: BTreeMap::from([(instance, binding)]),
        db_changed_routes: Vec::new().into_boxed_slice(),
        temporal_states: BTreeMap::new(),
        temporal_workers: BTreeMap::new(),
        db_commit_invalidation_sink: None,
        execution_context: SourceProviderContext::current(),
    }
}

fn signal_handle(linked: &BackendLinkedRuntime, module: &hir::Module, name: &str) -> SignalHandle {
    linked
        .assembly()
        .signal(item_id(module, name))
        .unwrap_or_else(|| panic!("signal binding should exist for {name}"))
        .signal()
}

fn activation_port_for_owner(
    linked: &BackendLinkedRuntime,
    module: &hir::Module,
    outcome: &LinkedSourceTickOutcome,
    owner_name: &str,
) -> DetachedRuntimePublicationPort {
    let instance = linked
        .source_by_owner(item_id(module, owner_name))
        .unwrap_or_else(|| panic!("source binding should exist for {owner_name}"))
        .instance;
    outcome
        .source_actions()
        .iter()
        .find_map(|action| match action {
            LinkedSourceLifecycleAction::Activate {
                instance: action_instance,
                port,
                ..
            } if *action_instance == instance => Some(port.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected activation for source {owner_name}"))
}

fn pump_for_commit_count(
    linked: &mut BackendLinkedRuntime,
    signal: SignalHandle,
    duration: std::time::Duration,
) -> usize {
    let deadline = std::time::Instant::now() + duration;
    let mut commits = 0;
    while std::time::Instant::now() < deadline {
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("pump tick should succeed");
        commits += outcome
            .scheduler()
            .committed()
            .iter()
            .filter(|&&committed| committed == signal)
            .count();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    commits
}

fn pump_until_commit_count(
    linked: &mut BackendLinkedRuntime,
    signal: SignalHandle,
    timeout: std::time::Duration,
    expected: usize,
) -> usize {
    let deadline = std::time::Instant::now() + timeout;
    let mut commits = 0;
    loop {
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("pump tick should succeed");
        commits += outcome
            .scheduler()
            .committed()
            .iter()
            .filter(|&&committed| committed == signal)
            .count();
        if commits >= expected {
            return commits;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for runtime condition"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn user_value(active: bool, email: &str) -> DetachedRuntimeValue {
    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Record(vec![
        aivi_backend::RuntimeRecordField {
            label: "active".into(),
            value: RuntimeValue::Bool(active),
        },
        aivi_backend::RuntimeRecordField {
            label: "email".into(),
            value: RuntimeValue::Text(email.into()),
        },
    ]))
}

#[test]
fn linked_runtime_ticks_simple_signals_and_evaluates_source_config() {
    let lowered = lower_text(
        "runtime-startup-basic.aivi",
        r#"
value prefix = "https://example.com/"

signal id = 7
signal next = id + 1
signal enabled = True
signal label = "Ada"

@source http.get "{prefix}{id}" with {
    activeWhen: enabled
}
signal users : Signal Text
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");

    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let next_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "next"))
        .expect("next signal binding should exist")
        .signal();
    let id_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "id"))
        .expect("id signal binding should exist")
        .signal();
    let label_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "label"))
        .expect("label signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(id_signal).unwrap(),
        Some(&RuntimeValue::Int(7))
    );
    assert_eq!(
        linked.runtime().current_value(next_signal).unwrap(),
        Some(&RuntimeValue::Int(8))
    );
    let label_value = linked
        .runtime()
        .current_value(label_signal)
        .unwrap()
        .expect("label signal should commit");
    let globals = linked
        .current_signal_globals()
        .expect("signal globals should snapshot committed values");
    let label_item = backend_item_id(&lowered.backend, "label");
    let label_snapshot = globals
        .get(&label_item)
        .expect("signal snapshot should carry label value");
    let RuntimeValue::Text(committed_label) = label_value else {
        panic!("label signal should carry text")
    };
    let RuntimeValue::Signal(snapshot_inner) = label_snapshot.as_runtime() else {
        panic!("signal snapshot should preserve wrapped signal shape")
    };
    let RuntimeValue::Text(snapshot_label) = snapshot_inner.as_ref() else {
        panic!("signal snapshot should carry text payload")
    };
    assert_ne!(
        committed_label.as_ptr(),
        snapshot_label.as_ptr(),
        "committed signal snapshots must detach boundary storage from scheduler-owned values"
    );
    assert_eq!(outcome.source_actions().len(), 1);
    let action = &outcome.source_actions()[0];
    assert_eq!(action.kind(), SourceLifecycleActionKind::Activate);
    let config = action.config().expect("activation should carry config");
    assert_eq!(
        config.arguments.as_ref(),
        &[RuntimeValue::Text("https://example.com/7".into())]
    );
    assert!(
        config.options.is_empty(),
        "scheduler-owned lifecycle options should not leak into provider config"
    );
}

#[test]
fn linked_runtime_uses_dedicated_signal_body_kernels_without_item_body() {
    let mut lowered = lower_text(
        "runtime-startup-signal-body-kernel.aivi",
        r#"
signal id = 7
signal next = id
"#,
    );
    let next_item = item_id(lowered.hir.module(), "next");
    let next_signal = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("runtime assembly should build")
        .signal(next_item)
        .expect("next signal binding should exist")
        .derived()
        .expect("next should be a derived signal");
    let backend_next = backend_item_id(&lowered.backend, "next");
    let body_kernel = match &lowered.backend.items()[backend_next].kind {
        BackendItemKind::Signal(info) => info
            .body_kernel
            .expect("derived signals should carry a dedicated signal body kernel"),
        other => panic!("next should lower as a backend signal, found {other:?}"),
    };
    lowered
        .backend
        .items_mut()
        .get_mut(backend_next)
        .expect("next backend item should remain addressable")
        .body = None;

    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed with a dedicated signal body kernel");

    assert_eq!(
        linked
            .derived_signal(next_signal)
            .expect("next derived signal should link")
            .body_kernel,
        Some(body_kernel)
    );

    linked.tick().expect("linked runtime tick should succeed");
    assert_eq!(
        linked
            .runtime()
            .current_value(next_signal.as_signal())
            .expect("next signal lookup should succeed"),
        Some(&RuntimeValue::Int(7))
    );
}

#[test]
fn linked_runtime_relocates_committed_signal_values_between_ticks() {
    let lowered = lower_text(
        "runtime-startup-moving-gc.aivi",
        r#"
signal label = "Ada"
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let label_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "label"))
        .expect("label signal binding should exist")
        .signal();

    linked
        .tick()
        .expect("initial linked runtime tick should succeed");
    let first = linked
        .runtime()
        .current_value(label_signal)
        .unwrap()
        .expect("label signal should commit on the first tick");
    let first_ptr = text_ptr(first);

    let outcome = linked
        .tick()
        .expect("second linked runtime tick should succeed");
    assert!(
        outcome.is_empty(),
        "empty linked-runtime ticks should still serve as moving-GC safe points"
    );
    let second = linked
        .runtime()
        .current_value(label_signal)
        .unwrap()
        .expect("label signal should stay committed after relocation");
    assert_eq!(second, &RuntimeValue::Text("Ada".into()));
    assert_ne!(
        first_ptr,
        text_ptr(second),
        "linked runtime must expose relocated committed text storage on the next tick"
    );
}

#[test]
fn linked_runtime_exposes_reactive_program_dependencies() {
    let lowered = lower_text(
        "runtime-startup-reactive-program-metadata.aivi",
        r#"
signal id = 7
signal next = id
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let id = signal_handle(&linked, lowered.hir.module(), "id");
    let next = signal_handle(&linked, lowered.hir.module(), "next");
    let node = linked
        .reactive_program()
        .signal(next)
        .expect("next signal should appear in the linked reactive program");

    assert_eq!(node.dependencies(), &[id]);
    assert_eq!(node.root_signals(), &[id]);
}

#[test]
fn linked_runtime_reports_missing_signal_snapshots_for_source_config() {
    let lowered = lower_text(
        "runtime-startup-missing-snapshot.aivi",
        r#"
@source http.get "/host"
signal apiHost : Signal Text

@source http.get "{apiHost}/users"
signal users : Signal Text
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let users = item_id(lowered.hir.module(), "users");
    let instance = linked
        .source_by_owner(users)
        .expect("users source binding should exist")
        .instance;
    let error = linked
        .evaluate_source_config(instance)
        .expect_err("missing signal snapshots should be reported");
    assert!(matches!(
        error,
        BackendRuntimeError::MissingCommittedSignalSnapshot { instance: found, .. } if found == instance
    ));
}

#[test]
fn linked_runtime_reports_missing_signal_item_mappings_for_source_config() {
    let lowered = lower_text(
        "runtime-startup-missing-signal-mapping.aivi",
        r#"
@source http.get "/host"
signal apiHost : Signal Text

@source http.get "{apiHost}/users"
signal users : Signal Text
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let users = item_id(lowered.hir.module(), "users");
    let binding = linked
        .source_by_owner(users)
        .expect("users source binding should exist")
        .clone();
    let required_item = binding.arguments[0].required_signals[0];
    linked.runtime_signal_by_item.remove(&required_item);

    let error = linked
        .evaluate_source_config(binding.instance)
        .expect_err("missing signal-item mappings should be reported explicitly");
    assert!(matches!(
        error,
        BackendRuntimeError::MissingSignalItemMapping {
            instance,
            item,
            ..
        } if instance == binding.instance && item == required_item
    ));
}

#[test]
fn linked_runtime_keeps_timer_restart_on_in_lifecycle_metadata() {
    let lowered = lower_text(
        "runtime-startup-timer-restart-on.aivi",
        r#"
signal rearm = True

@source timer.after 120 with {
    restartOn: rearm
}
signal ready : Signal Unit
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let ready = item_id(lowered.hir.module(), "ready");
    let instance = linked
        .source_by_owner(ready)
        .expect("timer source binding should exist")
        .instance;
    let config = linked
        .evaluate_source_config(instance)
        .expect("timer restartOn should stay in lifecycle metadata, not eager provider config");
    assert!(
        config
            .options
            .iter()
            .all(|option| option.option_name.as_ref() != "restartOn"),
        "timer restartOn should not be evaluated into provider options"
    );
}

#[test]
fn linked_runtime_spawns_task_workers_and_commits_publications() {
    let lowered = lower_text(
        "runtime-startup-manual-task-success.aivi",
        r#"
value answer = 42
"#,
    );
    let mut linked = manual_task_linked_runtime(&lowered, "answer");
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "answer"))
        .expect("manual task binding should exist")
        .clone();

    let handle = linked
        .spawn_task_worker(binding.instance)
        .expect("task worker should spawn");
    assert_eq!(
        handle
            .join()
            .expect("task worker thread should join cleanly"),
        Ok(LinkedTaskWorkerOutcome::Published)
    );

    let outcome = linked.tick().expect("task publication tick should succeed");
    assert!(!outcome.is_empty());
    assert_eq!(
        linked
            .runtime()
            .current_value(binding.input.as_signal())
            .expect("task sink should be readable"),
        Some(&RuntimeValue::Int(42))
    );
}

#[test]
fn linked_runtime_task_workers_execute_runtime_task_plans_before_publication() {
    let lowered = lower_text(
        "runtime-startup-manual-task-plan-success.aivi",
        r#"
value answer : Task Text Int = pure 42
"#,
    );
    let mut linked = manual_task_linked_runtime(&lowered, "answer");
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "answer"))
        .expect("manual task binding should exist")
        .clone();

    let handle = linked
        .spawn_task_worker(binding.instance)
        .expect("task worker should spawn");
    assert_eq!(
        handle
            .join()
            .expect("task worker thread should join cleanly"),
        Ok(LinkedTaskWorkerOutcome::Published)
    );

    let outcome = linked.tick().expect("task publication tick should succeed");
    assert!(!outcome.is_empty());
    assert_eq!(
        linked
            .runtime()
            .current_value(binding.input.as_signal())
            .expect("task sink should be readable"),
        Some(&RuntimeValue::Int(42))
    );
}

#[test]
fn linked_runtime_task_workers_execute_custom_capability_commands_through_context() {
    let lowered = lower_text(
        "runtime-startup-custom-capability-command-task.aivi",
        r#"
type FeedSource = Unit

value mode = "sync"

provider custom.feed
    argument root: Text
    option mode: Text
    command delete : Text -> Task Text Text

@source custom.feed "/tmp/demo" with {
    mode: mode
}
signal feed : FeedSource

value cleanup : Task Text Text = feed.delete "config"
"#,
    );
    let mut linked = manual_task_linked_runtime(&lowered, "cleanup");
    linked.set_execution_context(
        SourceProviderContext::current()
            .with_custom_capability_command_executor(Arc::new(EchoCustomCapabilityCommandExecutor)),
    );
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "cleanup"))
        .expect("manual task binding should exist")
        .clone();

    let handle = linked
        .spawn_task_worker(binding.instance)
        .expect("task worker should spawn");
    assert_eq!(
        handle
            .join()
            .expect("task worker thread should join cleanly"),
        Ok(LinkedTaskWorkerOutcome::Published)
    );

    let outcome = linked.tick().expect("task publication tick should succeed");
    assert!(!outcome.is_empty());
    assert_eq!(
        linked
            .runtime()
            .current_value(binding.input.as_signal())
            .expect("task sink should be readable"),
        Some(&RuntimeValue::Text("deleted".into()))
    );
}

#[test]
fn linked_runtime_reports_task_worker_evaluation_failures_explicitly() {
    let lowered = lower_text(
        "runtime-startup-manual-task-error.aivi",
        r#"
value total:Int = 1 / 0
"#,
    );
    let mut linked = manual_task_linked_runtime(&lowered, "total");
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "total"))
        .expect("manual task binding should exist")
        .clone();

    let handle = linked
        .spawn_task_worker(binding.instance)
        .expect("task worker should spawn");
    let result = handle
        .join()
        .expect("task worker thread should join cleanly");
    assert!(matches!(
        result,
        Err(LinkedTaskWorkerError::Evaluation { instance, owner, .. })
            if instance == binding.instance && owner == binding.owner
    ));
    let outcome = linked
        .tick()
        .expect("failed task should still allow empty ticks");
    assert!(outcome.is_empty());
    assert_eq!(
        linked
            .runtime()
            .current_value(binding.input.as_signal())
            .expect("task sink should be readable"),
        None
    );
}

#[test]
fn linked_runtime_task_execution_respects_cancellation_and_owner_teardown() {
    let lowered = lower_text(
        "runtime-startup-manual-task-cancel.aivi",
        r#"
value answer = 42
"#,
    );
    let mut linked = manual_task_linked_runtime(&lowered, "answer");
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "answer"))
        .expect("manual task binding should exist")
        .clone();

    let prepared = linked
        .prepare_task_execution(binding.instance)
        .expect("task execution should prepare");
    linked
        .runtime_mut()
        .cancel_task(binding.instance)
        .expect("task cancellation should succeed");
    assert_eq!(
        execute_task_plan(prepared).expect("cancelled task execution should not error"),
        LinkedTaskWorkerOutcome::Cancelled
    );
    let outcome = linked.tick().expect("cancelled task tick should succeed");
    assert!(outcome.is_empty());
    assert_eq!(
        linked
            .runtime()
            .current_value(binding.input.as_signal())
            .expect("task sink should be readable"),
        None
    );

    let prepared = linked
        .prepare_task_execution(binding.instance)
        .expect("task execution should prepare again");
    linked
        .runtime_mut()
        .dispose_owner(binding.owner_handle)
        .expect("owner disposal should succeed");
    assert_eq!(
        execute_task_plan(prepared).expect("disposed task execution should not error"),
        LinkedTaskWorkerOutcome::Cancelled
    );
    linked.tick().expect("owner-disposal tick should succeed");
    assert_eq!(
        linked
            .runtime()
            .is_owner_active(binding.owner_handle)
            .expect("task owner should be queryable"),
        false
    );
}

#[test]
fn linked_runtime_keeps_recurrent_task_body_gap_explicit() {
    let lowered = lower_text(
        "runtime-startup-task-body-gap.aivi",
        r#"
domain Retry over Int
    literal times : Int -> Retry

fun step:Int = n:Int=>    n

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
    );
    let assembly = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("task-body-gap runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("task-body-gap startup link should succeed");
    let binding = linked
        .task_by_owner(item_id(lowered.hir.module(), "retried"))
        .expect("recurrent task binding should exist");
    assert!(matches!(
        &binding.execution,
        LinkedTaskExecutionBinding::Blocked(LinkedTaskExecutionBlocker::MissingLoweredBody)
    ));
    let instance = binding.instance;
    assert!(matches!(
        linked.spawn_task_worker(instance),
        Err(BackendRuntimeError::TaskExecutionBlocked {
            instance: blocked_instance,
            ..
        }) if blocked_instance == instance
    ));
}

#[test]
fn linked_runtime_evaluates_helper_kernels_with_inline_case_pipes() {
    let lowered = lower_text(
        "runtime-startup-inline-case-helper.aivi",
        r#"
fun choose:Text = maybeName:(Option Text)=>    maybeName
     ||> Some name -> name
     ||> None -> "guest"

signal maybeName = Some "Ada"
signal label = choose maybeName
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let label_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "label"))
        .expect("label signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(label_signal).unwrap(),
        Some(&RuntimeValue::Text("Ada".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_evaluates_signal_inline_case_kernels_against_committed_snapshots() {
    let lowered = lower_text(
        "runtime-startup-signal-inline-case.aivi",
        r#"
fun greetSelected:Signal Text = prefix:Text fallback:Text selected:Signal (Option Text)=>    selected
     ||> Some name -> "{prefix}:{name}"
     ||> None -> "{prefix}:{fallback}"

signal selectedUser : Signal (Option Text) = Some "Ada"

signal greeting : Signal Text =
    greetSelected "user" "guest" selectedUser
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let greeting_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "greeting"))
        .expect("greeting signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(greeting_signal).unwrap(),
        Some(&RuntimeValue::Text("user:Ada".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_evaluates_signal_truthy_falsy_kernels_against_committed_snapshots() {
    let lowered = lower_text(
        "runtime-startup-signal-inline-truthy-falsy.aivi",
        r#"
fun renderStatus:Signal Text = prefix:Text readyText:Text waitText:Text statusReady:Signal Bool=>    statusReady
     T|> "{prefix}:{readyText}"
     F|> "{prefix}:{waitText}"

signal ready : Signal Bool = True

signal status : Signal Text =
    renderStatus "state" "go" "wait" ready
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let status_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "status"))
        .expect("status signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("state:go".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_evaluates_direct_signal_truthy_falsy_bodies() {
    let lowered = lower_text(
        "runtime-startup-direct-signal-truthy-falsy.aivi",
        r#"
type User = {
    name: Text
}

type LoadError = {
    message: Text
}

type FormError = {
    message: Text
}

signal ready : Signal Bool = True

signal maybeUser : Signal (Option User) = Some { name: "Ada" }

signal loaded : Signal (Result LoadError User) = Err { message: "offline" }

signal submitted : Signal (Validation FormError User) = Valid { name: "Grace" }

signal readyText : Signal Text =
    ready
     T|> "start"
     F|> "wait"

signal greeting : Signal Text =
    maybeUser
     T|> .name
     F|> "guest"

signal rendered : Signal Text =
    loaded
     T|> .name
     F|> .message

signal summary : Signal Text =
    submitted
     T|> .name
     F|> .message
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    for (name, expected) in [
        ("readyText", RuntimeValue::Text("start".into())),
        ("greeting", RuntimeValue::Text("Ada".into())),
        ("rendered", RuntimeValue::Text("offline".into())),
        ("summary", RuntimeValue::Text("Grace".into())),
    ] {
        let signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), name))
            .unwrap_or_else(|| panic!("signal binding should exist for {name}"))
            .signal();
        assert_eq!(
            linked.runtime().current_value(signal).unwrap(),
            Some(&expected),
            "signal {name} should commit the expected truthy/falsy result"
        );
    }
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_evaluates_direct_signal_transform_bodies() {
    let lowered = lower_text(
        "runtime-startup-direct-signal-transform.aivi",
        r#"
type User = {
    name: Text
}

type Session = {
    user: User
}

signal session : Signal Session = { user: { name: "Ada" } }

signal label : Signal Text =
    session
     |> .user
     |> .name
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let label_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "label"))
        .expect("label signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(label_signal).unwrap(),
        Some(&RuntimeValue::Text("Ada".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_evaluates_direct_signal_case_bodies_with_same_module_sum_patterns() {
    let lowered = lower_text(
        "runtime-startup-direct-signal-inline-case.aivi",
        r#"
type Status =
  | Idle
  | Ready Text
  | Failed Text Text

signal current : Signal Status =
    Failed "503" "offline"

signal label : Signal Text =
    current
     ||> Idle -> "idle"
     ||> Ready name -> name
     ||> Failed code message -> "{code}:{message}"
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let label_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "label"))
        .expect("label signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(label_signal).unwrap(),
        Some(&RuntimeValue::Text("503:offline".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_executes_signal_filter_gate_pipelines() {
    let lowered = lower_text(
        "runtime-startup-direct-signal-gate.aivi",
        r#"
type User = {
    active: Bool,
    email: Text
}

type Session = {
    user: User
}

value seed:User = { active: True, email: "ada@example.com" }

signal sessions : Signal Session = { user: seed }

signal activeUsers : Signal User =
    sessions
     |> .user
     ?|> .active
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("signal filter gate pipelines should now link successfully");

    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");

    let active_users_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "activeUsers"))
        .expect("activeUsers signal binding should exist")
        .signal();

    assert!(
        linked
            .runtime()
            .current_value(active_users_signal)
            .unwrap()
            .is_some(),
        "activeUsers should commit a value because user.active is True"
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_executes_signal_filter_gate_pipelines_without_prefix_body() {
    let lowered = lower_text(
        "runtime-startup-direct-signal-gate-head-only.aivi",
        r#"
type User = {
    active: Bool,
    email: Text
}

signal users : Signal User = { active: True, email: "ada@example.com" }

signal activeUsers : Signal User =
    users
     ?|> .active
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("head-only signal filter pipelines should now link successfully");

    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");

    let active_users_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "activeUsers"))
        .expect("activeUsers signal binding should exist")
        .signal();

    assert!(
        linked
            .runtime()
            .current_value(active_users_signal)
            .unwrap()
            .is_some(),
        "activeUsers should commit a value because user.active is True"
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
#[ignore = "known pre-existing failure: layout mismatch in backend body kernel for fanout pipelines"]
fn linked_runtime_executes_signal_fanout_map_and_join_pipelines() {
    let lowered = lower_text(
        "runtime-startup-signal-fanout.aivi",
        r#"
type User = {
    active: Bool,
    email: Text
}

fun joinEmails:Text = items:List Text=>    "joined"

signal liveUsers : Signal (List User) = [
    { active: True, email: "ada@example.com" }
]

signal liveEmails : Signal (List Text) =
    liveUsers
     *|> .email

signal liveJoinedEmails : Signal Text =
    liveUsers
     *|> .email
     <|* joinEmails
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("signal fanout pipelines should now link successfully");

    let outcome = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");

    let live_emails_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "liveEmails"))
        .expect("liveEmails signal binding should exist")
        .signal();
    let live_joined_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "liveJoinedEmails"))
        .expect("liveJoinedEmails signal binding should exist")
        .signal();

    assert_eq!(
        linked.runtime().current_value(live_emails_signal).unwrap(),
        Some(&RuntimeValue::List(vec![RuntimeValue::Text(
            "ada@example.com".into()
        )]))
    );
    assert_eq!(
        linked.runtime().current_value(live_joined_signal).unwrap(),
        Some(&RuntimeValue::Text("joined".into()))
    );
    assert!(outcome.source_actions().is_empty());
}

#[test]
fn linked_runtime_links_source_backed_body_signals_without_recurrence() {
    let lowered = lower_text(
        "runtime-startup-source-body-trigger.aivi",
        r#"
provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal status : Signal Text =
    "ready"
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("source-backed body signals should now link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("source activation tick should succeed");
    assert_eq!(first.source_actions().len(), 1);
    let port = match &first.source_actions()[0] {
        LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
        _ => panic!("expected source activation"),
    };
    let status_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "status"))
        .expect("status signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("ready".into())),
        "source-backed body signals should commit their body immediately"
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(
        RuntimeValue::Text("ignored".into()),
    ))
    .expect("source publication should queue");
    let second = linked
        .tick_with_source_lifecycle()
        .expect("source publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("ready".into()))
    );
    assert!(second.source_actions().is_empty());
}

#[test]
fn linked_runtime_executes_reactive_when_clauses_end_to_end() {
    let lowered = lower_text(
        "runtime-startup-reactive-when.aivi",
        r#"
provider custom.ready
    wakeup: providerTrigger

provider custom.enabled
    wakeup: providerTrigger

@source custom.ready
signal ready : Signal Bool

@source custom.enabled
signal enabled : Signal Bool

signal left = 20
signal right = 22

signal readyAndEnabled = ready and enabled

signal total : Signal Int = ready | readyAndEnabled
  ||> readyAndEnabled True => left + right + 1
  ||> ready True => left + right
  ||> _ => 0
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("reactive when clauses should now link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial reactive tick should succeed");
    assert_eq!(first.source_actions().len(), 2);
    let ready_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "ready");
    let enabled_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "enabled");
    let total_signal = signal_handle(&linked, lowered.hir.module(), "total");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(0)),
        "reactive signals should commit their seed body before any when clause fires"
    );

    ready_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("ready publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("ready-only reactive tick should succeed");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(42)),
        "the first when clause should fire once ready becomes true"
    );

    enabled_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("enabled publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("overlapping reactive tick should succeed");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(43)),
        "later firing when clauses should win by source order"
    );

    ready_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(false),
        ))
        .expect("ready reset should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("guard-false reactive tick should succeed");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(43)),
        "false guards should preserve the previously committed reactive value"
    );
}

#[test]
fn linked_runtime_executes_pattern_armed_reactive_updates_end_to_end() {
    let lowered = lower_text(
        "runtime-startup-pattern-reactive-when.aivi",
        r#"
type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down

signal heading : Signal Direction = event
  ||> Turn dir => dir
  ||> _ => Up

signal tickSeen : Signal Bool = event
  ||> Tick => True
  ||> _ => False
"#,
    );
    let assembly = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("runtime assembly should build for pattern-armed reactive updates");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("pattern-armed reactive updates should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial pattern-armed reactive tick should succeed");
    assert!(
        first.source_actions().is_empty(),
        "constant pattern-armed fixture should not request source actions"
    );

    let heading_signal = signal_handle(&linked, lowered.hir.module(), "heading");
    let tick_seen_signal = signal_handle(&linked, lowered.hir.module(), "tickSeen");
    let Some(RuntimeValue::Sum(heading)) = linked.runtime().current_value(heading_signal).unwrap()
    else {
        panic!("heading signal should hold a sum value after the reactive tick");
    };
    assert_eq!(&*heading.type_name, "Direction");
    assert_eq!(&*heading.variant_name, "Down");
    assert!(heading.fields.is_empty());
    assert_eq!(
        linked.runtime().current_value(tick_seen_signal).unwrap(),
        Some(&RuntimeValue::Bool(false)),
        "non-matching pattern arms should leave the other target untouched"
    );
}

#[test]
fn linked_runtime_executes_source_pattern_reactive_updates_end_to_end() {
    let lowered = lower_text(
        "runtime-startup-source-pattern-reactive-when.aivi",
        r#"
provider custom.ready
    wakeup: providerTrigger

@source custom.ready
signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
    );
    let assembly = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("runtime assembly should build for source-pattern reactive updates");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("source-pattern reactive updates should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial source-pattern reactive tick should succeed");
    assert_eq!(first.source_actions().len(), 1);
    let ready_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "ready");
    let total_signal = signal_handle(&linked, lowered.hir.module(), "total");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(0)),
        "seeded targets should retain their seed until a source-pattern clause matches"
    );

    ready_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("ready publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("matching source-pattern tick should succeed");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(42)),
        "matching source-pattern clauses should commit their body"
    );

    ready_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(false),
        ))
        .expect("ready reset should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("non-matching source-pattern tick should succeed");
    assert_eq!(
        linked.runtime().current_value(total_signal).unwrap(),
        Some(&RuntimeValue::Int(42)),
        "non-matching source-pattern clauses should preserve the committed value"
    );
}

#[test]
fn linked_runtime_source_pattern_updates_only_fire_for_their_trigger_source() {
    let lowered = lower_text(
        "runtime-startup-source-pattern-reactive-trigger-gating.aivi",
        r#"
provider custom.tick
    wakeup: providerTrigger

provider custom.key
    wakeup: providerTrigger

@source custom.tick
signal tick : Signal Bool

@source custom.key
signal key : Signal Bool

signal current : Signal Int = tick | key
  ||> tick True => 1
  ||> key True => 2
  ||> _ => 0
"#,
    );
    let assembly = crate::assemble_hir_runtime(lowered.hir.module())
        .expect("runtime assembly should build for source-pattern trigger gating");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("source-pattern trigger gating fixture should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial trigger-gating tick should succeed");
    assert_eq!(first.source_actions().len(), 2);
    let tick_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "tick");
    let key_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "key");
    let current_signal = signal_handle(&linked, lowered.hir.module(), "current");
    assert_eq!(
        linked.runtime().current_value(current_signal).unwrap(),
        Some(&RuntimeValue::Int(0)),
        "seeded targets should remain seeded before any source-pattern trigger fires"
    );

    key_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("key publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("key trigger tick should succeed");
    assert_eq!(
        linked.runtime().current_value(current_signal).unwrap(),
        Some(&RuntimeValue::Int(2)),
        "matching key publications should commit the key clause body"
    );

    tick_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("tick publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("tick trigger tick should succeed");
    assert_eq!(
        linked.runtime().current_value(current_signal).unwrap(),
        Some(&RuntimeValue::Int(1)),
        "later tick publications should not be overwritten by stale key source values"
    );
}

#[test]
fn linked_runtime_applies_target_pipelines_to_reactive_when_bodies() {
    let lowered = lower_text(
        "runtime-startup-reactive-when-pipeline.aivi",
        r#"
provider custom.ready
    wakeup: providerTrigger

provider custom.user
    wakeup: providerTrigger

type User = {
    active: Bool,
    email: Text
}

@source custom.ready
signal ready : Signal Bool

@source custom.user
signal incoming : Signal User

signal seed : Signal User = { active: True, email: "seed@example.com" }

signal current : Signal User = ready
  ||> True => incoming
  ||> _ => seed
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("reactive when targets with supported pipelines should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial reactive pipeline tick should succeed");
    assert_eq!(first.source_actions().len(), 2);
    let ready_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "ready");
    let incoming_port =
        activation_port_for_owner(&linked, lowered.hir.module(), &first, "incoming");
    let current_signal = signal_handle(&linked, lowered.hir.module(), "current");
    assert_eq!(
        linked.runtime().current_value(current_signal).unwrap(),
        Some(&RuntimeValue::Record(vec![
            aivi_backend::RuntimeRecordField {
                label: "active".into(),
                value: RuntimeValue::Bool(true),
            },
            aivi_backend::RuntimeRecordField {
                label: "email".into(),
                value: RuntimeValue::Text("seed@example.com".into()),
            },
        ])),
        "the default arm body should provide the initial value"
    );

    ready_port
        .publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(true),
        ))
        .expect("ready publication should queue");
    incoming_port
        .publish(user_value(true, "active@example.com"))
        .expect("active user publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("active reactive pipeline tick should succeed");
    assert_eq!(
        linked.runtime().current_value(current_signal).unwrap(),
        Some(&RuntimeValue::Record(vec![
            aivi_backend::RuntimeRecordField {
                label: "active".into(),
                value: RuntimeValue::Bool(true),
            },
            aivi_backend::RuntimeRecordField {
                label: "email".into(),
                value: RuntimeValue::Text("active@example.com".into()),
            },
        ])),
        "reactive merge arm should commit the incoming value when guard matches"
    );
}

#[test]
fn linked_runtime_applies_all_source_recurrence_steps() {
    let lowered = lower_text(
        "runtime-startup-source-recurrence-steps.aivi",
        r#"
fun bump:Int = n:Int=>    n + 1

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal counter : Signal Int =
    0
     @|> bump
     <|@ bump
     <|@ bump
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("source-backed recurrences should now link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial recurrence tick should succeed");
    assert_eq!(first.source_actions().len(), 1);
    let port = match &first.source_actions()[0] {
        LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
        _ => panic!("expected source activation"),
    };
    let counter_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "counter"))
        .expect("counter signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(counter_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        1,
    )))
    .expect("source publication should queue");
    let second = linked
        .tick_with_source_lifecycle()
        .expect("recurrence publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(counter_signal).unwrap(),
        Some(&RuntimeValue::Int(3))
    );
    assert!(second.source_actions().is_empty());
}

#[test]
fn linked_runtime_task_values_lower_to_zero_parameter_backend_items() {
    let lowered = lower_text(
        "runtime-startup-task-parameters-invariant.aivi",
        r#"
domain Retry over Int
    literal times : Int -> Retry

fun keep:Int = n:Int=>    n

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> keep
     <|@ keep
"#,
    );
    let backend_item = backend_item_id(&lowered.backend, "retried");
    assert!(
        lowered.backend.items()[backend_item].parameters.is_empty(),
        "startup-linked tasks currently come from parameterless top-level values"
    );
}

#[test]
fn linked_runtime_applies_accumulate_steps_once_per_wakeup() {
    let lowered = lower_text(
        "runtime-startup-accumulate-signal.aivi",
        r#"
fun step:Int = next:Int current:Int=>    current + next

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal next : Signal Int

signal counter : Signal Int =
    next
     +|> 0 step
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("accumulate signals should now link successfully");
    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial accumulate tick should succeed");
    assert_eq!(first.source_actions().len(), 1);
    let port = match &first.source_actions()[0] {
        LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
        _ => panic!("expected source activation"),
    };
    let counter_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "counter"))
        .expect("counter signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(counter_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        2,
    )))
    .expect("first source publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("first accumulate publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(counter_signal).unwrap(),
        Some(&RuntimeValue::Int(2))
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        3,
    )))
    .expect("second source publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("second accumulate publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(counter_signal).unwrap(),
        Some(&RuntimeValue::Int(5))
    );
}

#[test]
fn linked_runtime_applies_previous_and_diff_temporal_stages_once_per_publication() {
    let lowered = lower_text(
        "runtime-startup-temporal-signal.aivi",
        r#"
fun delta:Int = previous:Int current:Int=>    current - previous

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal score : Signal Int

signal previousScore : Signal Int =
    score
     ~|> 0

signal scoreDelta : Signal Int =
    score
     -|> 0

signal scoreDeltaFn : Signal Int =
    score
     -|> delta
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("temporal derived signals should now link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial temporal tick should succeed");
    assert_eq!(first.source_actions().len(), 1);
    let port = match &first.source_actions()[0] {
        LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
        _ => panic!("expected source activation"),
    };

    let previous_signal = signal_handle(&linked, lowered.hir.module(), "previousScore");
    let delta_signal = signal_handle(&linked, lowered.hir.module(), "scoreDelta");
    let delta_fn_signal = signal_handle(&linked, lowered.hir.module(), "scoreDeltaFn");

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        10,
    )))
    .expect("first score publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("first temporal publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(previous_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );
    assert_eq!(
        linked.runtime().current_value(delta_signal).unwrap(),
        Some(&RuntimeValue::Int(10))
    );
    assert_eq!(
        linked.runtime().current_value(delta_fn_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        13,
    )))
    .expect("second score publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("second temporal publication tick should succeed");
    assert_eq!(
        linked.runtime().current_value(previous_signal).unwrap(),
        Some(&RuntimeValue::Int(10))
    );
    assert_eq!(
        linked.runtime().current_value(delta_signal).unwrap(),
        Some(&RuntimeValue::Int(3))
    );
    assert_eq!(
        linked.runtime().current_value(delta_fn_signal).unwrap(),
        Some(&RuntimeValue::Int(3))
    );
}

#[test]
fn linked_runtime_reemits_delay_stage_once_after_interval() {
    let lowered = lower_text(
        "runtime-startup-delay-signal.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
}

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal score : Signal Int

signal delayedScore : Signal Int =
    score
     |> delay 25ms
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("delay temporal signals should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial delay tick should succeed");
    let port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "score");
    let delayed_signal = signal_handle(&linked, lowered.hir.module(), "delayedScore");

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        10,
    )))
    .expect("delayed source publication should queue");
    let first_publication = linked
        .tick_with_source_lifecycle()
        .expect("delay publication tick should succeed");
    assert_eq!(
        first_publication
            .scheduler()
            .committed()
            .iter()
            .filter(|&&signal| signal == delayed_signal)
            .count(),
        0
    );
    assert_eq!(
        linked.runtime().current_value(delayed_signal).unwrap(),
        None
    );
    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(10)
        ),
        0
    );
    assert_eq!(
        linked.runtime().current_value(delayed_signal).unwrap(),
        None
    );
    assert_eq!(
        pump_until_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(250),
            1
        ),
        1
    );
    assert_eq!(
        linked.runtime().current_value(delayed_signal).unwrap(),
        Some(&RuntimeValue::Int(10))
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(50)
        ),
        0
    );
}

#[test]
fn linked_runtime_reemits_burst_stage_exactly_n_times() {
    let lowered = lower_text(
        "runtime-startup-burst-signal.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
}

domain Retry over Int = {
    literal times : Int -> Retry
}

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal score : Signal Int

signal burstScore : Signal Int =
    score
     |> burst 15ms 3times
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("burst temporal signals should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial burst tick should succeed");
    let port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "score");
    let burst_signal = signal_handle(&linked, lowered.hir.module(), "burstScore");

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        7,
    )))
    .expect("burst source publication should queue");
    let first_publication = linked
        .tick_with_source_lifecycle()
        .expect("burst publication tick should succeed");
    assert_eq!(
        first_publication
            .scheduler()
            .committed()
            .iter()
            .filter(|&&signal| signal == burst_signal)
            .count(),
        0
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            burst_signal,
            std::time::Duration::from_millis(8)
        ),
        0
    );

    assert_eq!(
        pump_until_commit_count(
            &mut linked,
            burst_signal,
            std::time::Duration::from_millis(250),
            3
        ),
        3
    );
    assert_eq!(
        linked.runtime().current_value(burst_signal).unwrap(),
        Some(&RuntimeValue::Int(7))
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            burst_signal,
            std::time::Duration::from_millis(50)
        ),
        0
    );
}

#[test]
fn linked_runtime_replaces_in_flight_delay_schedules_with_newer_payloads() {
    let lowered = lower_text(
        "runtime-startup-delay-retrigger-signal.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
}

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal score : Signal Int

signal delayedScore : Signal Int =
    score
     |> delay 40ms
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("retriggered delay signals should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial delay tick should succeed");
    let port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "score");
    let delayed_signal = signal_handle(&linked, lowered.hir.module(), "delayedScore");

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        1,
    )))
    .expect("first delayed source publication should queue");
    let first_publication = linked
        .tick_with_source_lifecycle()
        .expect("first delay publication tick should succeed");
    assert_eq!(
        first_publication
            .scheduler()
            .committed()
            .iter()
            .filter(|&&signal| signal == delayed_signal)
            .count(),
        0
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(10)
        ),
        0
    );

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        2,
    )))
    .expect("second delayed source publication should queue");
    let second_publication = linked
        .tick_with_source_lifecycle()
        .expect("second delay publication tick should succeed");
    assert_eq!(
        second_publication
            .scheduler()
            .committed()
            .iter()
            .filter(|&&signal| signal == delayed_signal)
            .count(),
        0
    );
    assert_eq!(
        linked.runtime().current_value(delayed_signal).unwrap(),
        None
    );

    assert_eq!(
        pump_until_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(250),
            1
        ),
        1
    );
    assert_eq!(
        linked.runtime().current_value(delayed_signal).unwrap(),
        Some(&RuntimeValue::Int(2))
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            delayed_signal,
            std::time::Duration::from_millis(70)
        ),
        0
    );
}

#[test]
fn linked_runtime_resumes_composed_temporal_stages_after_helper_wakeups() {
    let lowered = lower_text(
        "runtime-startup-delay-burst-composed-signal.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
}

domain Retry over Int = {
    literal times : Int -> Retry
}

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal score : Signal Int

signal flashScore : Signal Int =
    score
     |> delay 15ms
     |> burst 12ms 3times
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("composed temporal signals should link successfully");

    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial composed temporal tick should succeed");
    let port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "score");
    let flash_signal = signal_handle(&linked, lowered.hir.module(), "flashScore");

    port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
        9,
    )))
    .expect("composed temporal source publication should queue");
    let first_publication = linked
        .tick_with_source_lifecycle()
        .expect("composed temporal publication tick should succeed");
    assert_eq!(
        first_publication
            .scheduler()
            .committed()
            .iter()
            .filter(|&&signal| signal == flash_signal)
            .count(),
        0
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            flash_signal,
            std::time::Duration::from_millis(8)
        ),
        0
    );

    assert_eq!(
        pump_until_commit_count(
            &mut linked,
            flash_signal,
            std::time::Duration::from_millis(300),
            3
        ),
        3
    );
    assert_eq!(
        linked.runtime().current_value(flash_signal).unwrap(),
        Some(&RuntimeValue::Int(9))
    );

    assert_eq!(
        pump_for_commit_count(
            &mut linked,
            flash_signal,
            std::time::Duration::from_millis(50)
        ),
        0
    );
}

#[test]
fn linked_runtime_keeps_recurrence_value_when_only_non_wakeup_dependencies_change() {
    let lowered = lower_text(
        "runtime-startup-recurrence-non-wakeup-deps.aivi",
        r#"
fun setDirection:Int = next:Int current:Int=>    next

fun stepGame:Int = tick:Int current:Int=>    current + direction

provider custom.turn
    wakeup: providerTrigger

provider custom.tick
    wakeup: providerTrigger

@source custom.turn
signal turn : Signal Int

signal direction : Signal Int =
    turn
     +|> 1 setDirection

@source custom.tick
signal tick : Signal Int

signal game : Signal Int =
    tick
     +|> 0 stepGame
"#,
    );
    let assembly =
        crate::assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("recurrence with non-wakeup dependencies should link successfully");
    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial recurrence tick should succeed");
    assert_eq!(first.source_actions().len(), 2);
    let turn_instance = linked
        .source_by_owner(item_id(lowered.hir.module(), "turn"))
        .expect("turn source binding should exist")
        .instance;
    let tick_instance = linked
        .source_by_owner(item_id(lowered.hir.module(), "tick"))
        .expect("tick source binding should exist")
        .instance;
    let mut turn_port = None;
    let mut tick_port = None;
    for action in first.source_actions() {
        let LinkedSourceLifecycleAction::Activate {
            instance,
            port,
            config: _,
        } = action
        else {
            panic!("initial source lifecycle should only activate providers");
        };
        match instance {
            instance if *instance == turn_instance => {
                turn_port = Some(port.clone());
            }
            instance if *instance == tick_instance => {
                tick_port = Some(port.clone());
            }
            _ => {}
        }
    }
    let turn_port = turn_port.expect("turn source should activate");
    let tick_port = tick_port.expect("tick source should activate");
    let game_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "game"))
        .expect("game signal binding should exist")
        .signal();
    let direction_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "direction"))
        .expect("direction signal binding should exist")
        .signal();
    assert_eq!(
        linked.runtime().current_value(game_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );
    assert_eq!(
        linked.runtime().current_value(direction_signal).unwrap(),
        Some(&RuntimeValue::Int(1))
    );

    turn_port
        .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            5,
        )))
        .expect("direction publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("direction-only tick should succeed");
    assert_eq!(
        linked.runtime().current_value(direction_signal).unwrap(),
        Some(&RuntimeValue::Int(5))
    );
    assert_eq!(
        linked.runtime().current_value(game_signal).unwrap(),
        Some(&RuntimeValue::Int(0)),
        "non-wakeup dependency changes must preserve the current recurrence snapshot"
    );

    tick_port
        .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            1,
        )))
        .expect("tick publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("tick publication should advance the recurrence");
    assert_eq!(
        linked.runtime().current_value(game_signal).unwrap(),
        Some(&RuntimeValue::Int(5)),
        "the next wakeup should apply the latest direction"
    );
}
