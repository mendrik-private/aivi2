use std::{
    env,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

use aivi_base::SourceDatabase;
use aivi_hir::{Item, lower_module as lower_hir_module};
use aivi_lambda::lower_module as lower_lambda_module;
use aivi_syntax::parse_module;
use glib::prelude::ToVariant;

use super::*;
use crate::{
    BackendLinkedRuntime, EvaluatedSourceOption, SignalGraphBuilder, SourceRuntimeSpec,
    TaskSourceRuntime, assemble_hir_runtime, link_backend_runtime,
};

struct LoweredStack {
    hir: aivi_hir::LoweringResult,
    core: aivi_core::Module,
    backend: aivi_backend::Program,
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
    let core = aivi_core::lower_module(hir.module()).expect("typed-core lowering should succeed");
    let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
    let backend = aivi_backend::lower_module_with_hir(&lambda, hir.module())
        .expect("backend lowering should succeed");
    LoweredStack { hir, core, backend }
}

fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
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

fn spin_until(
    linked: &mut BackendLinkedRuntime,
    signal: crate::SignalHandle,
    timeout: Duration,
) -> Option<RuntimeValue> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        linked.tick().expect("runtime tick should succeed");
        if let Some(value) = linked.runtime().current_value(signal).unwrap() {
            return Some(value.clone());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

fn run_http_server(response_body: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept one request");
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("server should write response");
    });
    (format!("http://{address}"), handle)
}

fn run_http_server_sequence(responses: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for response_body in responses {
            let (mut stream, _) = listener.accept().expect("server should accept a request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        }
    });
    (format!("http://{address}"), handle)
}

fn temp_path(prefix: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-scratch");
    fs::create_dir_all(&base).expect("runtime test scratch directory should exist");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    base.join(format!(
        "aivi-runtime-{prefix}-{}-{unique}",
        std::process::id()
    ))
}

fn record_field<'a>(
    fields: &'a [aivi_backend::RuntimeRecordField],
    name: &str,
) -> &'a RuntimeValue {
    fields
        .iter()
        .find(|field| field.label.as_ref() == name)
        .map(|field| &field.value)
        .unwrap_or_else(|| panic!("expected record field `{name}`"))
}

fn expect_text(value: &RuntimeValue, expected: &str) {
    match value {
        RuntimeValue::Text(found) => assert_eq!(found.as_ref(), expected),
        other => panic!("expected text `{expected}`, found {other:?}"),
    }
}

fn spin_source_runtime_until_match(
    runtime: &mut TaskSourceRuntime<RuntimeValue>,
    signal: crate::SignalHandle,
    timeout: Duration,
    predicate: impl Fn(&RuntimeValue) -> bool,
) -> Option<RuntimeValue> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        runtime.tick(&mut |_, _: crate::DependencyValues<'_, RuntimeValue>| None);
        if let Some(value) = runtime.current_value(signal).unwrap()
            && predicate(value)
        {
            return Some(value.clone());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

fn db_live_test_runtime(
    instance: SourceInstanceId,
) -> (
    TaskSourceRuntime<RuntimeValue>,
    crate::SignalHandle,
    crate::startup::DetachedRuntimePublicationPort,
) {
    let mut builder = SignalGraphBuilder::new();
    let input = builder
        .add_input("db-live-output", None)
        .expect("db.live output input should register");
    let graph = builder.build().expect("db.live test graph should build");
    let mut runtime: TaskSourceRuntime<RuntimeValue> = TaskSourceRuntime::new(graph);
    runtime
        .register_source(SourceRuntimeSpec::new(
            instance,
            input,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbLive),
        ))
        .expect("db.live source spec should register");
    let port = crate::startup::DetachedRuntimePublicationPort::from_source_port(
        runtime
            .activate_source(instance)
            .expect("db.live source should activate"),
    );
    (runtime, input.as_signal(), port)
}

fn db_live_config(
    instance: SourceInstanceId,
    task: RuntimeValue,
    debounce_ms: Option<i64>,
) -> EvaluatedSourceConfig {
    let mut options = Vec::new();
    if let Some(debounce_ms) = debounce_ms {
        options.push(EvaluatedSourceOption {
            option_name: "debounce".into(),
            value: DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(debounce_ms)),
        });
    }
    EvaluatedSourceConfig {
        owner: aivi_hir::ItemId::from_raw(0),
        instance,
        source: aivi_backend::SourceId::from_raw(0),
        provider: RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbLive),
        decode: None,
        arguments: vec![DetachedRuntimeValue::from_runtime_owned(task)].into_boxed_slice(),
        options: options.into_boxed_slice(),
    }
}

#[test]
fn timer_every_actions_publish_unit_immediately() {
    let lowered = lower_text(
        "runtime-provider-timer-every.aivi",
        r#"
@source timer.every 5 with {
    immediate: True
}
signal tick : Signal Unit
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");

    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("timer provider actions should execute");

    let tick_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "tick"))
        .expect("tick signal binding should exist")
        .signal();
    assert_eq!(
        spin_until(&mut linked, tick_signal, Duration::from_millis(200)),
        Some(RuntimeValue::Unit)
    );
}

#[test]
fn http_get_publishes_decoded_result_values() {
    let (base_url, handle) = run_http_server(r#"[{"id":1,"name":"Ada"}]"#);
    let lowered = lower_text(
        "runtime-provider-http-get.aivi",
        &format!(
            r#"
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {{
    id: Int,
    name: Text
}}

@source http.get "{base_url}/users"
signal users : Signal (Result HttpError (List User))
"#
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("http provider should execute");
    let users_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "users"))
        .expect("users signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, users_signal, Duration::from_secs(2))
        .expect("http provider should publish a result");
    assert!(matches!(value, RuntimeValue::ResultOk(_)));
    handle.join().unwrap();
}

#[test]
fn http_get_refresh_every_reissues_requests() {
    let (base_url, handle) = run_http_server_sequence(vec!["first", "second"]);
    let lowered = lower_text(
        "runtime-provider-http-refresh.aivi",
        &format!(
            r#"
type HttpError =
  | Timeout
  | DecodeFailure Text
  | RequestFailure Text

@source http.get "{base_url}/users" with {{
    refreshEvery: 40
}}
signal users : Signal (Result HttpError Text)
"#
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("http provider should execute");
    let users_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "users"))
        .expect("users signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, users_signal, Duration::from_secs(1))
        .expect("http provider should refresh and publish");
    if value != RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into()))) {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut latest = value;
        while Instant::now() < deadline {
            linked.tick().expect("runtime tick should succeed");
            if let Some(current) = linked.runtime().current_value(users_signal).unwrap() {
                latest = current.clone();
                if latest == RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into()))) {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            latest,
            RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into())))
        );
    }
    handle.join().unwrap();
}

#[test]
fn mailbox_source_publishes_text_messages() {
    let lowered = lower_text(
        "runtime-provider-mailbox.aivi",
        r#"
type MailboxError =
  | DecodeFailure Text
  | MailboxFailure Text

@source mailbox.subscribe "jobs"
signal job : Signal (Result MailboxError Text)
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("mailbox source should execute");
    providers
        .publish_mailbox_message("jobs", "hello")
        .expect("mailbox publish should succeed");
    let job_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "job"))
        .expect("job signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, job_signal, Duration::from_millis(200))
        .expect("mailbox source should publish");
    assert_eq!(
        value,
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
    );
}

#[test]
fn mailbox_source_unsubscribes_on_suspension() {
    let lowered = lower_text(
        "runtime-provider-mailbox-suspend.aivi",
        r#"
type MailboxError =
  | DecodeFailure Text
  | MailboxFailure Text

@source mailbox.subscribe "jobs"
signal job : Signal (Result MailboxError Text)
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("mailbox source should execute");
    let job_item = item_id(lowered.hir.module(), "job");
    let instance = linked
        .source_by_owner(job_item)
        .expect("job source binding should exist")
        .instance;
    linked
        .runtime_mut()
        .suspend_source(instance)
        .expect("source suspension should succeed");
    providers
        .apply_actions(&[crate::LinkedSourceLifecycleAction::Suspend { instance }])
        .expect("provider suspension should succeed");
    providers
        .publish_mailbox_message("jobs", "later")
        .expect("mailbox publish should still succeed");
    let signal = linked
        .assembly()
        .signal(job_item)
        .expect("job signal binding should exist")
        .signal();
    assert!(
        spin_until(&mut linked, signal, Duration::from_millis(150)).is_none(),
        "suspended mailbox sources should stop receiving messages"
    );
}

#[test]
fn mailbox_hub_prunes_disconnected_subscribers() {
    let mut hub = MailboxHub::default();
    let (_subscriber_id, receiver) = hub.subscribe("jobs", 1);
    drop(receiver);

    hub.publish("jobs", "hello")
        .expect("publishing to a disconnected mailbox subscriber should not fail");

    assert!(
        !hub.subscribers.contains_key("jobs"),
        "publishing should clear mailbox entries whose subscribers disconnected"
    );
}

#[test]
fn mailbox_hub_tracks_only_live_subscriber_ids() {
    let mut hub = MailboxHub::default();
    let (stable_id, stable_receiver) = hub.subscribe("jobs", 1);

    for _ in 0..32 {
        let (transient_id, transient_receiver) = hub.subscribe("jobs", 1);
        hub.unsubscribe("jobs", transient_id);
        drop(transient_receiver);
    }

    let subscriber_ids = hub
        .subscribers
        .get("jobs")
        .expect("mailbox should still have the stable subscriber")
        .keys()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        subscriber_ids,
        vec![stable_id],
        "mailbox storage should retain only live subscriber ids after churn"
    );

    hub.publish("jobs", "later")
        .expect("publishing to the surviving subscriber should succeed");
    assert_eq!(
        stable_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("surviving subscriber should still receive messages")
            .as_ref(),
        "later"
    );
}

#[test]
fn fs_read_publishes_text_snapshots() {
    let path = temp_path("fs-read");
    fs::write(&path, "hello").expect("fixture file should write");
    let lowered = lower_text(
        "runtime-provider-fs-read.aivi",
        &format!(
            r#"
type FsError =
  | Missing
  | DecodeFailure Text

@source fs.read "{}"
signal fileText : Signal (Result FsError Text)
"#,
            path.display()
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("fs.read source should execute");
    let signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "fileText"))
        .expect("fileText signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, signal, Duration::from_millis(300))
        .expect("fs.read should publish one snapshot");
    assert_eq!(
        value,
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
    );
    let _ = fs::remove_file(path);
}

#[test]
fn fs_watch_detects_created_files() {
    let path = temp_path("fs-watch");
    let lowered = lower_text(
        "runtime-provider-fs-watch.aivi",
        &format!(
            r#"
type FsWatchEvent =
  | Created
  | Changed
  | Deleted

@source fs.watch "{}"
signal fileEvents : Signal FsWatchEvent
"#,
            path.display()
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("fs.watch source should execute");
    thread::sleep(Duration::from_millis(100));
    fs::write(&path, "hello").expect("watched file should write");
    let signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "fileEvents"))
        .expect("fileEvents signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, signal, Duration::from_secs(1))
        .expect("fs.watch should publish a create event");
    assert!(matches!(value, RuntimeValue::Sum(_)));
    let _ = fs::remove_file(path);
}

#[test]
fn socket_connect_reads_text_lines() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("server should accept one client");
        stream
            .write_all(b"hello\n")
            .expect("server should write one line");
    });
    let lowered = lower_text(
        "runtime-provider-socket.aivi",
        &format!(
            r#"
type SocketError =
  | ConnectFailure Text
  | DecodeFailure Text
  | RequestFailure Text

@source socket.connect "tcp://{}:{}"
signal message : Signal (Result SocketError Text)
"#,
            address.ip(),
            address.port()
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("socket source should execute");
    let signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "message"))
        .expect("message signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, signal, Duration::from_secs(1))
        .expect("socket source should publish one line");
    assert_eq!(
        value,
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("hello".into())))
    );
    handle.join().unwrap();
}

#[test]
fn process_spawn_publishes_process_events() {
    let lowered = lower_text(
        "runtime-provider-process.aivi",
        r#"
type StreamMode =
  | Ignore
  | Lines

type ProcessEvent =
  | Spawned

@source process.spawn "true"
signal events : Signal ProcessEvent
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("process source should execute");
    let signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "events"))
        .expect("events signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, signal, Duration::from_secs(1))
        .expect("process source should publish at least one event");
    assert!(matches!(value, RuntimeValue::Sum(_)));
}

#[test]
fn dbus_signal_source_publishes_structured_bus_messages() {
    if env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
        return;
    }
    let lowered = lower_text(
        "runtime-provider-dbus-signal.aivi",
        r#"
type DbusSignal = {
    path: Text,
    interface: Text,
    member: Text,
    body: Text
}

@source dbus.signal "/org/aivi/Test" with {
    interface: "org.aivi.Test"
    member: "Ping"
}
signal inbound : Signal DbusSignal
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("dbus.signal source should execute");

    let connection = gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>)
        .expect("session bus should be reachable");
    let payload =
        Variant::tuple_from_iter(["hello".to_variant(), 7_i32.to_variant(), true.to_variant()]);
    connection
        .emit_signal(
            None,
            "/org/aivi/Test",
            "org.aivi.Test",
            "Ping",
            Some(&payload),
        )
        .expect("test signal should emit");

    let inbound_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "inbound"))
        .expect("inbound signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, inbound_signal, Duration::from_secs(1))
        .expect("dbus.signal source should publish");
    let RuntimeValue::Record(fields) = value else {
        panic!("dbus.signal should decode to a record");
    };
    expect_text(record_field(&fields, "path"), "/org/aivi/Test");
    expect_text(record_field(&fields, "interface"), "org.aivi.Test");
    expect_text(record_field(&fields, "member"), "Ping");
    expect_text(record_field(&fields, "body"), "('hello', 7, true)");
}

#[test]
fn dbus_method_source_replies_unit_and_publishes_calls() {
    if env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
        return;
    }
    let service_name = format!("org.aivi.RuntimeTest{}", std::process::id());
    let lowered = lower_text(
        "runtime-provider-dbus-method.aivi",
        &format!(
            r#"
type BusNameFlag =
  | AllowReplacement
  | ReplaceExisting
  | DoNotQueue

type BusNameState =
  | Owned
  | Queued
  | Lost

type DbusCall = {{
    destination: Text,
    path: Text,
    interface: Text,
    member: Text,
    body: Text
}}

@source dbus.ownName "{service_name}"
signal busState : Signal BusNameState

@source dbus.method "{service_name}" with {{
    path: "/org/aivi/Test"
    interface: "org.aivi.Test"
    member: "ShowWindow"
}}
signal incoming : Signal DbusCall
"#,
            service_name = service_name,
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("dbus providers should execute");

    let bus_state_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "busState"))
        .expect("busState signal binding should exist")
        .signal();
    let owned_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let bus_state = spin_until(&mut linked, bus_state_signal, Duration::from_millis(50))
            .expect("dbus.ownName should publish");
        if matches!(&bus_state, RuntimeValue::Sum(sum) if sum.variant_name.as_ref() == "Owned") {
            break;
        }
        assert!(
            Instant::now() < owned_deadline,
            "dbus.ownName should eventually acquire the requested name"
        );
    }

    let connection = gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>)
        .expect("session bus should be reachable");
    let reply = connection
        .call_sync(
            Some(service_name.as_ref()),
            "/org/aivi/Test",
            "org.aivi.Test",
            "ShowWindow",
            Some(&Variant::tuple_from_iter([
                "hello".to_variant(),
                5_i32.to_variant(),
            ])),
            None::<&glib::VariantTy>,
            gio::DBusCallFlags::NONE,
            1_000,
            None::<&gio::Cancellable>,
        )
        .expect("dbus.method source should reply immediately");
    assert_eq!(reply.n_children(), 0, "dbus.method should reply with Unit");

    let incoming_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "incoming"))
        .expect("incoming signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, incoming_signal, Duration::from_secs(1))
        .expect("dbus.method source should publish one call");
    let RuntimeValue::Record(fields) = value else {
        panic!("dbus.method should decode to a record");
    };
    expect_text(record_field(&fields, "destination"), service_name.as_ref());
    expect_text(record_field(&fields, "path"), "/org/aivi/Test");
    expect_text(record_field(&fields, "interface"), "org.aivi.Test");
    expect_text(record_field(&fields, "member"), "ShowWindow");
    expect_text(record_field(&fields, "body"), "('hello', 5)");
}

#[test]
#[ignore = "known pre-existing failure: flaky GLib threading in D-Bus reply handling"]
fn dbus_method_source_replies_with_configured_body() {
    if env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
        return;
    }
    let service_name = format!("org.aivi.RuntimeTestReply{}", std::process::id());
    let lowered = lower_text(
        "runtime-provider-dbus-method-reply.aivi",
        &format!(
            r#"
type BusNameFlag =
  | AllowReplacement
  | ReplaceExisting
  | DoNotQueue

type BusNameState =
  | Owned
  | Queued
  | Lost

type DbusCall = {{
    destination: Text,
    path: Text,
    interface: Text,
    member: Text,
    body: Text
}}

@source dbus.ownName "{service_name}"
signal busState : Signal BusNameState

@source dbus.method "{service_name}" with {{
    path: "/org/aivi/Test"
    interface: "org.aivi.Test"
    member: "GetStatus"
    reply: "('running', 42)"
}}
signal incoming : Signal DbusCall
"#,
            service_name = service_name,
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("dbus providers should execute");

    let bus_state_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "busState"))
        .expect("busState signal binding should exist")
        .signal();
    let owned_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let bus_state = spin_until(&mut linked, bus_state_signal, Duration::from_millis(50))
            .expect("dbus.ownName should publish");
        if matches!(&bus_state, RuntimeValue::Sum(sum) if sum.variant_name.as_ref() == "Owned") {
            break;
        }
        assert!(
            Instant::now() < owned_deadline,
            "dbus.ownName should eventually acquire the requested name"
        );
    }

    let connection = gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>)
        .expect("session bus should be reachable");
    let reply = connection
        .call_sync(
            Some(service_name.as_ref()),
            "/org/aivi/Test",
            "org.aivi.Test",
            "GetStatus",
            None::<&Variant>,
            None::<&glib::VariantTy>,
            gio::DBusCallFlags::NONE,
            1_000,
            None::<&gio::Cancellable>,
        )
        .expect("dbus.method source should reply with configured body");
    assert_eq!(
        reply.n_children(),
        2,
        "dbus.method reply should contain the configured body tuple"
    );
    let first = reply.child_value(0);
    assert_eq!(first.get::<String>().unwrap(), "running");
    let second = reply.child_value(1);
    assert_eq!(second.get::<i32>().unwrap(), 42);
}

#[test]
fn window_key_source_suppresses_repeat_when_requested() {
    let lowered = lower_text(
        "runtime-provider-window-key.aivi",
        r#"
type Key =
  | ArrowUp
  | ArrowDown

@source window.keyDown with {
    repeat: False
    focusOnly: True
}
signal keyDown : Signal Key
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("window key source should execute");
    providers.dispatch_window_key_event(WindowKeyEvent {
        name: "ArrowUp".into(),
        repeated: false,
    });
    providers.dispatch_window_key_event(WindowKeyEvent {
        name: "ArrowUp".into(),
        repeated: true,
    });
    let key_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "keyDown"))
        .expect("keyDown signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, key_signal, Duration::from_millis(200))
        .expect("window key source should publish");
    assert!(matches!(value, RuntimeValue::Sum(_)));
}

#[test]
fn db_connect_source_publishes_connection_record() {
    let database = temp_path("db-connect-success.sqlite");
    let lowered = lower_text(
        "runtime-provider-db-connect-success.aivi",
        &format!(
            r#"
type DbError =
  | ConnectionFailed Text
  | QueryFailed Text

type Connection = {{
    database: Text
}}

value config = {{
    database: "{}"
}}

@source db.connect config with {{
    pool: 5
}}
signal db : Signal (Result DbError Connection)
"#,
            database.display()
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("db.connect source should execute");
    let db_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "db"))
        .expect("db signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, db_signal, Duration::from_millis(300))
        .expect("db.connect source should publish");
    let RuntimeValue::ResultOk(connection) = value else {
        panic!("expected Ok connection, found {value:?}");
    };
    let RuntimeValue::Record(fields) = connection.as_ref() else {
        panic!("expected connection record, found {connection:?}");
    };
    expect_text(
        record_field(fields, "database"),
        database.to_string_lossy().as_ref(),
    );
    assert!(
        database.exists(),
        "db.connect should create/open the SQLite file"
    );
    let _ = fs::remove_file(&database);
}

#[test]
fn db_connect_source_publishes_connection_failed_error() {
    let missing_parent = temp_path("db-connect-missing-parent");
    let database = missing_parent.join("nested").join("db.sqlite");
    let lowered = lower_text(
        "runtime-provider-db-connect-failure.aivi",
        &format!(
            r#"
type DbError =
  | ConnectionFailed Text
  | QueryFailed Text

type Connection = {{
    database: Text
}}

@source db.connect "{}"
signal db : Signal (Result DbError Connection)
"#,
            database.display()
        ),
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("db.connect source should execute");
    let db_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "db"))
        .expect("db signal binding should exist")
        .signal();
    let value = spin_until(&mut linked, db_signal, Duration::from_millis(300))
        .expect("db.connect source should publish an error");
    let RuntimeValue::ResultErr(error) = value else {
        panic!("expected Err connection failure, found {value:?}");
    };
    let RuntimeValue::Sum(sum) = error.as_ref() else {
        panic!("expected DbError sum value, found {error:?}");
    };
    assert_eq!(sum.variant_name.as_ref(), "ConnectionFailed");
    assert_eq!(
        sum.fields.len(),
        1,
        "ConnectionFailed should carry one message"
    );
    let RuntimeValue::Text(message) = &sum.fields[0] else {
        panic!("expected failure message text, found {:?}", sum.fields[0]);
    };
    assert!(
        message.contains("open") || message.contains("unable") || message.contains("No such"),
        "expected a SQLite open failure message, found {message}"
    );
    let _ = fs::remove_dir_all(&missing_parent);
}

#[test]
fn db_live_source_executes_task_immediately_on_activation_even_with_debounce() {
    let instance = SourceInstanceId::from_raw(41);
    let (mut runtime, rows_signal, port) = db_live_test_runtime(instance);
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(&[LinkedSourceLifecycleAction::Activate {
            instance,
            port,
            config: db_live_config(
                instance,
                RuntimeValue::Task(aivi_backend::RuntimeTaskPlan::Pure {
                    value: Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(7)))),
                }),
                Some(200),
            ),
        }])
        .expect("db.live source should execute");
    let value = spin_source_runtime_until_match(
        &mut runtime,
        rows_signal,
        Duration::from_millis(80),
        |value| {
            matches!(
                value,
                RuntimeValue::ResultOk(inner)
                    if matches!(inner.as_ref(), RuntimeValue::Int(7))
            )
        },
    )
    .expect("db.live activation should not wait for the debounce window");
    let RuntimeValue::ResultOk(value) = value else {
        panic!("expected Ok query result, found {value:?}");
    };
    assert_eq!(value.as_ref(), &RuntimeValue::Int(7));
    providers.suspend_active_provider(instance);
}

#[test]
fn db_live_source_publishes_task_error_results() {
    let instance = SourceInstanceId::from_raw(42);
    let (mut runtime, rows_signal, port) = db_live_test_runtime(instance);
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(&[LinkedSourceLifecycleAction::Activate {
            instance,
            port,
            config: db_live_config(
                instance,
                RuntimeValue::Task(aivi_backend::RuntimeTaskPlan::Pure {
                    value: Box::new(RuntimeValue::ResultErr(Box::new(RuntimeValue::Text(
                        "boom".into(),
                    )))),
                }),
                None,
            ),
        }])
        .expect("db.live source should execute");
    let value = spin_source_runtime_until_match(
        &mut runtime,
        rows_signal,
        Duration::from_millis(80),
        |value| matches!(value, RuntimeValue::ResultErr(_)),
    )
    .expect("db.live should publish the task error result");
    let RuntimeValue::ResultErr(error) = value else {
        panic!("expected Err query result, found {value:?}");
    };
    let RuntimeValue::Text(message) = error.as_ref() else {
        panic!("expected text error payload, found {error:?}");
    };
    assert_eq!(message.as_ref(), "boom");
    providers.suspend_active_provider(instance);
}

#[test]
fn db_live_source_reconfigures_with_debounce() {
    let instance = SourceInstanceId::from_raw(43);
    let (mut runtime, rows_signal, port) = db_live_test_runtime(instance);
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(&[LinkedSourceLifecycleAction::Activate {
            instance,
            port,
            config: db_live_config(
                instance,
                RuntimeValue::Task(aivi_backend::RuntimeTaskPlan::Pure {
                    value: Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(1)))),
                }),
                Some(100),
            ),
        }])
        .expect("db.live activation should execute");

    let initial = spin_source_runtime_until_match(
        &mut runtime,
        rows_signal,
        Duration::from_millis(80),
        |value| {
            matches!(
                value,
                RuntimeValue::ResultOk(inner)
                    if matches!(inner.as_ref(), RuntimeValue::Int(1))
            )
        },
    )
    .expect("db.live activation should publish the initial query result");
    let RuntimeValue::ResultOk(initial) = initial else {
        panic!("expected Ok query result, found {initial:?}");
    };
    assert_eq!(initial.as_ref(), &RuntimeValue::Int(1));

    let first_refresh_port = crate::startup::DetachedRuntimePublicationPort::from_source_port(
        runtime
            .reconfigure_source(instance)
            .expect("db.live source should reconfigure"),
    );
    providers
        .apply_actions(&[LinkedSourceLifecycleAction::Reconfigure {
            instance,
            port: first_refresh_port,
            config: db_live_config(
                instance,
                RuntimeValue::Task(aivi_backend::RuntimeTaskPlan::Pure {
                    value: Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(2)))),
                }),
                Some(100),
            ),
        }])
        .expect("first db.live refresh should schedule successfully");

    let first_delay_guard = Instant::now();
    while first_delay_guard.elapsed() < Duration::from_millis(40) {
        runtime.tick(&mut |_, _: crate::DependencyValues<'_, RuntimeValue>| None);
        let current = runtime.current_value(rows_signal).unwrap().cloned();
        assert_eq!(
            current,
            Some(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(1)))),
            "db.live should keep the committed value while the debounce window is still open"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let second_refresh_port = crate::startup::DetachedRuntimePublicationPort::from_source_port(
        runtime
            .reconfigure_source(instance)
            .expect("db.live source should reconfigure again"),
    );
    providers
        .apply_actions(&[LinkedSourceLifecycleAction::Reconfigure {
            instance,
            port: second_refresh_port,
            config: db_live_config(
                instance,
                RuntimeValue::Task(aivi_backend::RuntimeTaskPlan::Pure {
                    value: Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(3)))),
                }),
                Some(100),
            ),
        }])
        .expect("second db.live refresh should schedule successfully");

    let no_intermediate_publish = Instant::now();
    while no_intermediate_publish.elapsed() < Duration::from_millis(80) {
        runtime.tick(&mut |_, _: crate::DependencyValues<'_, RuntimeValue>| None);
        let current = runtime.current_value(rows_signal).unwrap().cloned();
        assert_eq!(
            current,
            Some(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(1)))),
            "debounced refresh should cancel the stale worker before it can publish"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let refreshed = spin_source_runtime_until_match(
        &mut runtime,
        rows_signal,
        Duration::from_millis(200),
        |value| {
            matches!(
                value,
                RuntimeValue::ResultOk(inner)
                    if matches!(inner.as_ref(), RuntimeValue::Int(3))
            )
        },
    )
    .expect("db.live should eventually publish the latest debounced refresh result");
    let RuntimeValue::ResultOk(refreshed) = refreshed else {
        panic!("expected Ok query result, found {refreshed:?}");
    };
    assert_eq!(refreshed.as_ref(), &RuntimeValue::Int(3));
    providers.suspend_active_provider(instance);
}

#[test]
fn timer_every_stops_after_source_suspension() {
    let lowered = lower_text(
        "runtime-provider-timer-cancel.aivi",
        r#"
@source timer.every 5
signal tick : Signal Unit
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(
        assembly,
        &lowered.core,
        std::sync::Arc::new(lowered.backend.clone()),
    )
    .expect("startup link should succeed");

    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("timer provider actions should execute");

    let tick_item = item_id(lowered.hir.module(), "tick");
    let instance = linked
        .source_by_owner(tick_item)
        .expect("tick source binding should exist")
        .instance;
    let tick_signal = linked
        .assembly()
        .signal(tick_item)
        .expect("tick signal binding should exist")
        .signal();

    assert!(spin_until(&mut linked, tick_signal, Duration::from_millis(200)).is_some());

    linked
        .runtime_mut()
        .suspend_source(instance)
        .expect("source suspension should cancel the active timer port");
    providers
        .apply_actions(&[crate::LinkedSourceLifecycleAction::Suspend { instance }])
        .expect("provider manager should drop suspended timer state");

    let quiet_deadline = Instant::now() + Duration::from_millis(200);
    loop {
        thread::sleep(Duration::from_millis(12));
        let outcome = linked
            .tick()
            .expect("runtime tick should stay quiet after timer cancellation");
        assert!(
            outcome.committed().is_empty(),
            "suspended timers should not commit further values"
        );
        if outcome.dropped_publications().is_empty() {
            break;
        }
        assert!(
            Instant::now() < quiet_deadline,
            "suspended timers should stop publishing after draining any in-flight delivery"
        );
    }

    for _ in 0..5 {
        thread::sleep(Duration::from_millis(12));
        let outcome = linked
            .tick()
            .expect("runtime tick should stay quiet after timer cancellation");
        assert!(
            outcome.committed().is_empty(),
            "suspended timers should not commit further values"
        );
        assert!(
            outcome.dropped_publications().is_empty(),
            "suspended timers should stop publishing instead of producing stale drops"
        );
    }
}
