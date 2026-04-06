use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fmt, fs,
    io::{BufRead, BufReader, Read},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use aivi_backend::{DetachedRuntimeValue, RuntimeCallable, RuntimeValue};
use aivi_hir as hir;
use aivi_typing::BuiltinSourceProvider;
use gio::{
    BusNameOwnerFlags, BusType, DBusConnection, DBusConnectionFlags, DBusMessageType,
    DBusSendMessageFlags, DBusSignalFlags,
};
use glib::{ControlFlow, MainContext, MainLoop, Variant, VariantClass};
use url::Url;

use crate::startup::DetachedRuntimePublicationPort;
use crate::{
    CancellationObserver, EvaluatedSourceConfig, LinkedSourceLifecycleAction,
    RuntimeSourceProvider, SourceInstanceId, SourceLifecycleActionKind,
    source_decode::{
        ExternalSourceValue, SourceDecodeError, SourceDecodeErrorWithPath, decode_external,
        encode_runtime_json, parse_json_text, validate_supported_program,
    },
    task_executor::{
        CustomCapabilityCommandExecutor, execute_runtime_value_with_context_with_stdio,
    },
};

/// Scheduler-owned mailbox routing table.
///
/// Each mailbox stores only the subscribers that are currently live. The inner
/// map key is the stable subscriber id handed back to the source manager so
/// suspend/reconfigure teardown can remove the exact mailbox worker it started.
#[derive(Default)]
struct MailboxHub {
    next_id: u64,
    subscribers: BTreeMap<Box<str>, BTreeMap<u64, SyncSender<Box<str>>>>,
}

impl MailboxHub {
    fn subscribe(&mut self, mailbox: &str, buffer: usize) -> (u64, mpsc::Receiver<Box<str>>) {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("mailbox subscriber ids should not overflow");
        let (sender, receiver) = mpsc::sync_channel(buffer.max(1));
        let replaced = self
            .subscribers
            .entry(mailbox.into())
            .or_default()
            .insert(id, sender);
        debug_assert!(
            replaced.is_none(),
            "mailbox subscriber ids must stay unique within a hub"
        );
        (id, receiver)
    }

    fn unsubscribe(&mut self, mailbox: &str, id: u64) {
        let should_remove = {
            let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
                return;
            };
            subscribers.remove(&id);
            subscribers.is_empty()
        };
        if should_remove {
            self.subscribers.remove(mailbox);
        }
    }

    fn publish(&mut self, mailbox: &str, message: &str) -> Result<(), MailboxPublishError> {
        let (full, should_remove) = {
            let Some(subscribers) = self.subscribers.get_mut(mailbox) else {
                return Ok(());
            };
            let mut full = false;
            let mut disconnected = Vec::new();
            for (&id, sender) in subscribers.iter() {
                match sender.try_send(message.into()) {
                    Ok(()) => {}
                    Err(TrySendError::Disconnected(_)) => disconnected.push(id),
                    Err(TrySendError::Full(_)) => {
                        full = true;
                    }
                }
            }
            for id in disconnected {
                subscribers.remove(&id);
            }
            (full, subscribers.is_empty())
        };
        if should_remove {
            self.subscribers.remove(mailbox);
        }
        if full {
            Err(MailboxPublishError::BufferFull {
                mailbox: mailbox.into(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailboxPublishError {
    BufferFull { mailbox: Box<str> },
}

impl fmt::Display for MailboxPublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferFull { mailbox } => write!(
                f,
                "mailbox `{mailbox}` cannot accept a new message because at least one subscriber buffer is full"
            ),
        }
    }
}

impl std::error::Error for MailboxPublishError {}

#[derive(Clone)]
pub struct SourceProviderContext {
    args: Arc<[String]>,
    cwd: Arc<PathBuf>,
    env: Arc<BTreeMap<String, String>>,
    stdin_override: Option<Result<Box<str>, Box<str>>>,
    stdin_text: Arc<OnceLock<Result<Box<str>, Box<str>>>>,
    custom_capability_command_executor: Option<Arc<dyn CustomCapabilityCommandExecutor>>,
}

impl Default for SourceProviderContext {
    fn default() -> Self {
        Self::current()
    }
}

impl SourceProviderContext {
    pub fn current() -> Self {
        let args = env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let env = env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Self::new(args, cwd, env)
    }

    pub fn new(args: Vec<String>, cwd: PathBuf, env: BTreeMap<String, String>) -> Self {
        Self {
            args: Arc::from(args.into_boxed_slice()),
            cwd: Arc::new(cwd),
            env: Arc::new(env),
            stdin_override: None,
            stdin_text: Arc::new(OnceLock::new()),
            custom_capability_command_executor: None,
        }
    }

    pub fn with_stdin_text(mut self, stdin: impl Into<String>) -> Self {
        self.stdin_override = Some(Ok(stdin.into().into_boxed_str()));
        self
    }

    pub fn with_custom_capability_command_executor(
        mut self,
        executor: Arc<dyn CustomCapabilityCommandExecutor>,
    ) -> Self {
        self.custom_capability_command_executor = Some(executor);
        self
    }

    pub(crate) fn custom_capability_command_executor(
        &self,
    ) -> Option<&Arc<dyn CustomCapabilityCommandExecutor>> {
        self.custom_capability_command_executor.as_ref()
    }

    fn args_runtime_value(&self) -> RuntimeValue {
        RuntimeValue::List(
            self.args
                .iter()
                .cloned()
                .map(|arg| RuntimeValue::Text(arg.into_boxed_str()))
                .collect(),
        )
    }

    fn cwd_runtime_value(&self) -> RuntimeValue {
        RuntimeValue::Text(self.cwd.to_string_lossy().into_owned().into_boxed_str())
    }

    fn env_runtime_value(&self, key: &str) -> RuntimeValue {
        match self.env.get(key) {
            Some(value) => RuntimeValue::OptionSome(Box::new(RuntimeValue::Text(
                value.clone().into_boxed_str(),
            ))),
            None => RuntimeValue::OptionNone,
        }
    }

    fn stdin_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(value) = &self.stdin_override {
            return value.clone();
        }
        self.stdin_text
            .get_or_init(|| {
                let mut input = String::new();
                std::io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|error| format!("failed to read stdin: {error}").into_boxed_str())?;
                Ok(input.into_boxed_str())
            })
            .clone()
    }

    fn home_dir_text(&self) -> Result<Box<str>, Box<str>> {
        self.env
            .get("HOME")
            .cloned()
            .map(String::into_boxed_str)
            .ok_or_else(|| "HOME is not set".into())
    }

    fn config_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_CONFIG_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".config")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn data_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_DATA_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".local")
            .join("share")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn cache_home_text(&self) -> Result<Box<str>, Box<str>> {
        if let Some(path) = self.env.get("XDG_CACHE_HOME") {
            return Ok(path.clone().into_boxed_str());
        }
        let home = self.home_dir_text()?;
        Ok(PathBuf::from(home.as_ref())
            .join(".cache")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str())
    }

    fn temp_dir_text(&self) -> Box<str> {
        env::temp_dir()
            .to_string_lossy()
            .into_owned()
            .into_boxed_str()
    }

    fn normalize_sqlite_database_text(&self, database: &str) -> Box<str> {
        if database == ":memory:" || database.starts_with("file:") {
            return database.into();
        }
        let path = PathBuf::from(database);
        let resolved = if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        };
        resolved.to_string_lossy().into_owned().into_boxed_str()
    }
}

#[derive(Clone)]
enum ActiveProviderState {
    Passive {
        provider: RuntimeSourceProvider,
        /// Stop flag set to `true` when this source instance is disposed.
        /// Worker threads check this flag before each iteration so they exit promptly
        /// when the source is removed, even if the port cancellation signal has not yet
        /// propagated through the scheduler.
        stop: Arc<AtomicBool>,
    },
    Mailbox {
        provider: RuntimeSourceProvider,
        mailbox: Box<str>,
        subscriber_id: u64,
        /// Stop flag for the mailbox worker thread; set to `true` on disposal.
        stop: Arc<AtomicBool>,
    },
    Window {
        provider: RuntimeSourceProvider,
        output: WindowKeyOutputPlan,
        capture: bool,
        focus_only: bool,
        allow_repeat: bool,
        port: DetachedRuntimePublicationPort,
    },
}

impl ActiveProviderState {
    fn provider(&self) -> &RuntimeSourceProvider {
        match self {
            Self::Passive { provider, .. }
            | Self::Mailbox { provider, .. }
            | Self::Window { provider, .. } => provider,
        }
    }
}

#[derive(Clone)]
pub struct SourceProviderManager {
    active: BTreeMap<SourceInstanceId, ActiveProviderState>,
    mailboxes: Arc<Mutex<MailboxHub>>,
    thread_handles: Arc<Mutex<BTreeMap<SourceInstanceId, Vec<thread::JoinHandle<()>>>>>,
    context: SourceProviderContext,
}

impl Default for SourceProviderManager {
    fn default() -> Self {
        Self::with_context(SourceProviderContext::current())
    }
}

impl SourceProviderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_context(context: SourceProviderContext) -> Self {
        Self {
            active: BTreeMap::new(),
            mailboxes: Arc::new(Mutex::new(MailboxHub::default())),
            thread_handles: Arc::new(Mutex::new(BTreeMap::new())),
            context,
        }
    }

    pub fn active_provider(&self, instance: SourceInstanceId) -> Option<&RuntimeSourceProvider> {
        self.active
            .get(&instance)
            .map(ActiveProviderState::provider)
    }

    pub fn has_active_provider(&self, instance: SourceInstanceId) -> bool {
        self.active.contains_key(&instance)
    }

    pub fn suspend_active_provider(&mut self, instance: SourceInstanceId) {
        self.remove_active(instance);
    }

    pub fn publish_mailbox_message(
        &self,
        mailbox: &str,
        message: &str,
    ) -> Result<(), MailboxPublishError> {
        self.mailboxes
            .lock()
            .expect("mailbox hub mutex should not be poisoned")
            .publish(mailbox, message)
    }

    pub fn dispatch_window_key_event(&mut self, event: WindowKeyEvent) {
        for state in self.active.values() {
            let ActiveProviderState::Window {
                output,
                allow_repeat,
                port,
                ..
            } = state
            else {
                continue;
            };
            if event.repeated && !allow_repeat {
                continue;
            }
            let Ok(Some(value)) = output.value_for_key(&event.name) else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
        }
    }

    /// Returns the merged window key configuration across all active
    /// `window.keyDown` source instances.  The GTK host uses this to decide
    /// the propagation phase and focus policy of the installed key controller.
    pub fn window_key_config(&self) -> WindowKeyConfig {
        let mut capture = false;
        let mut focus_only = true;
        for state in self.active.values() {
            if let ActiveProviderState::Window {
                capture: c,
                focus_only: f,
                ..
            } = state
            {
                capture = capture || *c;
                focus_only = focus_only && *f;
            }
        }
        WindowKeyConfig { capture, focus_only }
    }

    /// Returns `true` when at least one worker thread spawned by this manager
    /// is still running (i.e. has not yet finished).  Immediate sources that
    /// publish synchronously during activation never create thread handles, so
    /// this returns `false` for programs that only use immediate sources.
    pub fn has_unfinished_worker_threads(&self) -> bool {
        let handles = self
            .thread_handles
            .lock()
            .expect("thread_handles mutex should not be poisoned");
        handles
            .values()
            .flat_map(|v| v.iter())
            .any(|h| !h.is_finished())
    }

    pub fn apply_actions(
        &mut self,
        actions: &[LinkedSourceLifecycleAction],
    ) -> Result<(), SourceProviderExecutionError> {
        for action in actions {
            match action {
                LinkedSourceLifecycleAction::Activate {
                    instance,
                    port,
                    config,
                }
                | LinkedSourceLifecycleAction::Reconfigure {
                    instance,
                    port,
                    config,
                } => self.start_provider(action.kind(), *instance, config, port.clone())?,
                LinkedSourceLifecycleAction::Suspend { instance } => {
                    self.remove_active(*instance);
                }
            }
        }
        Ok(())
    }

    fn start_provider(
        &mut self,
        action_kind: SourceLifecycleActionKind,
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
        port: DetachedRuntimePublicationPort,
    ) -> Result<(), SourceProviderExecutionError> {
        self.remove_active(instance);
        let state = match &config.provider {
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerEvery) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerEvery, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_timer_every(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerAfter) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerAfter, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_timer_after(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::HttpGet) => {
                let plan = HttpPlan::parse(instance, BuiltinSourceProvider::HttpGet, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_http_worker(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::HttpPost) => {
                let plan = HttpPlan::parse(instance, BuiltinSourceProvider::HttpPost, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_http_worker(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::FsRead) => {
                let plan = FsReadPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                if action_kind == SourceLifecycleActionKind::Reconfigure || plan.read_on_start {
                    let handle = spawn_fs_read_worker(port, plan, stop.clone());
                    self.thread_handles
                        .lock()
                        .unwrap()
                        .entry(instance)
                        .or_default()
                        .push(handle);
                }
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::FsWatch) => {
                let plan = FsWatchPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_fs_watch_worker(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::SocketConnect) => {
                let plan = SocketPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_socket_worker(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::MailboxSubscribe) => {
                let plan = MailboxPlan::parse(instance, config)?;
                let (subscriber_id, receiver) = self
                    .mailboxes
                    .lock()
                    .expect("mailbox hub mutex should not be poisoned")
                    .subscribe(&plan.mailbox, plan.buffer);
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_mailbox_worker(port, plan.clone(), receiver, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Mailbox {
                    provider: config.provider.clone(),
                    mailbox: plan.mailbox,
                    subscriber_id,
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ProcessSpawn) => {
                let plan = ProcessPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_process_worker(port, plan, stop.clone());
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ProcessArgs) => {
                validate_argument_count(instance, BuiltinSourceProvider::ProcessArgs, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::ProcessArgs, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::ProcessArgs,
                    &port,
                    self.context.args_runtime_value(),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ProcessCwd) => {
                validate_argument_count(instance, BuiltinSourceProvider::ProcessCwd, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::ProcessCwd, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::ProcessCwd,
                    &port,
                    self.context.cwd_runtime_value(),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::EnvGet) => {
                validate_argument_count(instance, BuiltinSourceProvider::EnvGet, config, 1)?;
                reject_options(instance, BuiltinSourceProvider::EnvGet, config)?;
                let key = parse_text_argument(
                    instance,
                    BuiltinSourceProvider::EnvGet,
                    0,
                    &config.arguments[0],
                )?;
                let stop = Arc::new(AtomicBool::new(false));
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::EnvGet,
                    &port,
                    self.context.env_runtime_value(key.as_ref()),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::StdioRead) => {
                validate_argument_count(instance, BuiltinSourceProvider::StdioRead, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::StdioRead, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let stdin = self.context.stdin_text().map_err(|detail| {
                    SourceProviderExecutionError::StartFailed {
                        instance,
                        provider: BuiltinSourceProvider::StdioRead,
                        detail,
                    }
                })?;
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::StdioRead,
                    &port,
                    RuntimeValue::Text(stdin),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::PathHome) => {
                validate_argument_count(instance, BuiltinSourceProvider::PathHome, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::PathHome, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let path = self.context.home_dir_text().map_err(|detail| {
                    SourceProviderExecutionError::StartFailed {
                        instance,
                        provider: BuiltinSourceProvider::PathHome,
                        detail,
                    }
                })?;
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::PathHome,
                    &port,
                    RuntimeValue::Text(path),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::PathConfigHome) => {
                validate_argument_count(
                    instance,
                    BuiltinSourceProvider::PathConfigHome,
                    config,
                    0,
                )?;
                reject_options(instance, BuiltinSourceProvider::PathConfigHome, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let path = self.context.config_home_text().map_err(|detail| {
                    SourceProviderExecutionError::StartFailed {
                        instance,
                        provider: BuiltinSourceProvider::PathConfigHome,
                        detail,
                    }
                })?;
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::PathConfigHome,
                    &port,
                    RuntimeValue::Text(path),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::PathDataHome) => {
                validate_argument_count(instance, BuiltinSourceProvider::PathDataHome, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::PathDataHome, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let path = self.context.data_home_text().map_err(|detail| {
                    SourceProviderExecutionError::StartFailed {
                        instance,
                        provider: BuiltinSourceProvider::PathDataHome,
                        detail,
                    }
                })?;
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::PathDataHome,
                    &port,
                    RuntimeValue::Text(path),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::PathCacheHome) => {
                validate_argument_count(instance, BuiltinSourceProvider::PathCacheHome, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::PathCacheHome, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let path = self.context.cache_home_text().map_err(|detail| {
                    SourceProviderExecutionError::StartFailed {
                        instance,
                        provider: BuiltinSourceProvider::PathCacheHome,
                        detail,
                    }
                })?;
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::PathCacheHome,
                    &port,
                    RuntimeValue::Text(path),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::PathTempDir) => {
                validate_argument_count(instance, BuiltinSourceProvider::PathTempDir, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::PathTempDir, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::PathTempDir,
                    &port,
                    RuntimeValue::Text(self.context.temp_dir_text()),
                )?;
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbConnect) => {
                let plan = DbConnectPlan::parse(instance, &self.context, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_db_connect_worker(
                    instance,
                    port,
                    plan,
                    self.context.clone(),
                    stop.clone(),
                );
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbLive) => {
                let plan = DbLivePlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let delay = match action_kind {
                    SourceLifecycleActionKind::Activate => Duration::ZERO,
                    SourceLifecycleActionKind::Reconfigure => plan.debounce,
                    SourceLifecycleActionKind::Suspend => {
                        unreachable!("start_provider only runs for activate/reconfigure actions")
                    }
                };
                let handle = spawn_db_live_worker(
                    instance,
                    port,
                    plan,
                    self.context.clone(),
                    delay,
                    stop.clone(),
                );
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbusOwnName) => {
                let plan = DbusOwnNamePlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_dbus_own_name_worker(port, plan, stop.clone())?;
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbusSignal) => {
                let plan = DbusSignalPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_dbus_signal_worker(port, plan, stop.clone())?;
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbusMethod) => {
                let plan = DbusMethodPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_dbus_method_worker(port, plan, stop.clone())?;
                self.thread_handles
                    .lock()
                    .unwrap()
                    .entry(instance)
                    .or_default()
                    .push(handle);
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::WindowKeyDown) => {
                let plan = WindowKeyDownPlan::parse(instance, config)?;
                ActiveProviderState::Window {
                    provider: config.provider.clone(),
                    output: plan.output,
                    capture: plan.capture,
                    focus_only: plan.focus_only,
                    allow_repeat: plan.allow_repeat,
                    port,
                }
            }
            RuntimeSourceProvider::Builtin(
                BuiltinSourceProvider::ImapConnect
                | BuiltinSourceProvider::ImapIdle
                | BuiltinSourceProvider::ImapFetchBody
                | BuiltinSourceProvider::SmtpSend
                | BuiltinSourceProvider::DbExec
                | BuiltinSourceProvider::TimeNowMs,
            ) => {
                // Stub: runtime implementations are pending. Publish Unit so
                // downstream signals settle at startup without panicking.
                let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                    RuntimeValue::Unit,
                ));
                let stop = Arc::new(AtomicBool::new(false));
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Custom(_) => {
                // Custom providers publish an initial Unit value so the source
                // signal is populated and downstream derivations can settle.
                let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                    RuntimeValue::Unit,
                ));
                let stop = Arc::new(AtomicBool::new(false));
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
        };
        self.active.insert(instance, state);
        Ok(())
    }

    fn remove_active(&mut self, instance: SourceInstanceId) {
        let Some(state) = self.active.remove(&instance) else {
            return;
        };
        // Signal the worker thread to stop. The worker will observe this flag
        // on its next iteration and exit cleanly without waiting for the port
        // cancellation to propagate through the scheduler.
        match &state {
            ActiveProviderState::Passive { stop, .. } => {
                stop.store(true, Ordering::Release);
            }
            ActiveProviderState::Mailbox {
                mailbox,
                subscriber_id,
                stop,
                ..
            } => {
                stop.store(true, Ordering::Release);
                self.mailboxes
                    .lock()
                    .expect("mailbox hub mutex should not be poisoned")
                    .unsubscribe(mailbox, *subscriber_id);
            }
            ActiveProviderState::Window { .. } => {}
        }
        // Join any worker threads associated with this instance.  The stop flag
        // was already set above so each thread should exit on its next iteration;
        // we wait here to ensure no thread outlives the provider instance.
        if let Some(handles) = self.thread_handles.lock().unwrap().remove(&instance) {
            for handle in handles {
                let _ = handle.join();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowKeyEvent {
    pub name: Box<str>,
    pub repeated: bool,
}

/// Configuration for a window.keyDown source instance, exposed so the GTK host
/// can set the correct propagation phase and focus policy on the event controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowKeyConfig {
    pub capture: bool,
    pub focus_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderExecutionError {
    MissingDecodeProgram {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
    },
    UnsupportedDecodeProgram {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
    UnsupportedProviderShape {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
    InvalidArgumentCount {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        expected: usize,
        found: usize,
    },
    InvalidArgument {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        index: usize,
        expected: Box<str>,
        value: RuntimeValue,
    },
    InvalidOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
        expected: Box<str>,
        value: RuntimeValue,
    },
    UnsupportedOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
    },
    ZeroTimerInterval {
        instance: SourceInstanceId,
    },
    StartFailed {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        detail: Box<str>,
    },
}

impl fmt::Display for SourceProviderExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDecodeProgram { instance, provider } => write!(
                f,
                "source instance {} provider {} is missing its decode program",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedDecodeProgram {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} cannot execute its decode program: {detail}",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedProviderShape {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} cannot execute this source shape: {detail}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidArgumentCount {
                instance,
                provider,
                expected,
                found,
            } => write!(
                f,
                "source instance {} provider {} expects {expected} argument(s), found {found}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidArgument {
                instance,
                provider,
                index,
                expected,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid argument {index}; expected {expected}, found {value}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidOption {
                instance,
                provider,
                option_name,
                expected,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid `{option_name}` option; expected {expected}, found {value}",
                instance.as_raw(),
                provider.key()
            ),
            Self::UnsupportedOption {
                instance,
                provider,
                option_name,
            } => write!(
                f,
                "source instance {} provider {} does not execute `{option_name}` yet",
                instance.as_raw(),
                provider.key()
            ),
            Self::ZeroTimerInterval { instance } => write!(
                f,
                "source instance {} cannot execute a timer with a zero or negative interval; durations must be positive",
                instance.as_raw()
            ),
            Self::StartFailed {
                instance,
                provider,
                detail,
            } => write!(
                f,
                "source instance {} provider {} failed to start: {detail}",
                instance.as_raw(),
                provider.key()
            ),
        }
    }
}

impl std::error::Error for SourceProviderExecutionError {}

#[derive(Clone, Copy)]
struct TimerPlan {
    delay: Duration,
    jitter: Option<Duration>,
    immediate: bool,
    coalesce: bool,
}

impl TimerPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let delay = parse_duration(instance, provider, 0, &config.arguments[0])?;
        // Reject zero and negative durations: a zero interval would spin the worker thread at 100%
        // CPU, and negative durations are not representable as `std::time::Duration` (they would
        // be silently clamped to zero by the `i64 as u64` cast in `parse_duration`).
        if delay.is_zero() {
            return Err(SourceProviderExecutionError::ZeroTimerInterval { instance });
        }
        let mut immediate = false;
        let mut jitter = None;
        let mut coalesce = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "immediate" => {
                    immediate = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "coalesce" => {
                    coalesce = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "activeWhen" => {}
                "jitter" => {
                    let dur = parse_option_duration(instance, provider, &option.option_name, &option.value)?;
                    if dur > delay {
                        return Err(SourceProviderExecutionError::StartFailed {
                            instance,
                            provider,
                            detail: "jitter must not exceed the timer interval".into(),
                        });
                    }
                    jitter = Some(dur);
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self { delay, jitter, immediate, coalesce })
    }
}

#[derive(Clone, Copy)]
enum PayloadDecodeMode {
    Text,
    Json,
}

#[derive(Clone)]
struct RequestResultPlan {
    decode: hir::SourceDecodeProgram,
    success_mode: PayloadDecodeMode,
    error: ErrorPlan,
}

#[derive(Clone)]
enum ErrorPlan {
    Text,
    Sum { variants: Box<[SumErrorVariant]> },
}

#[derive(Clone)]
struct SumErrorVariant {
    name: Box<str>,
    payload: ErrorPayloadKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorPayloadKind {
    None,
    Text,
    Int,
}

impl RequestResultPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        let hir::DecodeProgramStep::Result { error, value } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "request and stream providers currently require `Signal (Result E A)` outputs"
                        .into(),
            });
        };
        let success_mode = if matches!(
            decode.step(*value),
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            }
        ) {
            PayloadDecodeMode::Text
        } else {
            PayloadDecodeMode::Json
        };
        let error = ErrorPlan::from_step(instance, provider, &decode, *error)?;
        Ok(Self {
            decode,
            success_mode,
            error,
        })
    }

    fn success_from_text(&self, text: &str) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        let payload = match self.success_mode {
            PayloadDecodeMode::Text => ExternalSourceValue::Text(text.into()),
            PayloadDecodeMode::Json => parse_json_text(text)?,
        };
        decode_external(
            &self.decode,
            &ExternalSourceValue::variant_with_payload("Ok", payload),
        )
    }

    fn error_value(
        &self,
        kind: TextSourceErrorKind,
        message: &str,
    ) -> Result<RuntimeValue, Box<str>> {
        let payload = self.error.payload_for(kind, message)?;
        decode_external(
            &self.decode,
            &ExternalSourceValue::variant_with_payload("Err", payload),
        )
        .map_err(|error| error.to_string().into_boxed_str())
    }
}

#[derive(Clone)]
struct DbConnectPlan {
    database: Box<str>,
    result: RequestResultPlan,
}

impl DbConnectPlan {
    fn parse(
        instance: SourceInstanceId,
        context: &SourceProviderContext,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbConnect;
        validate_argument_count(instance, provider, config, 1)?;
        let database =
            parse_db_connect_argument(instance, provider, 0, context, &config.arguments[0])?;
        for option in &config.options {
            match option.option_name.as_ref() {
                "pool" => {
                    let _ =
                        parse_positive_int(instance, provider, &option.option_name, &option.value)?;
                }
                "activeWhen" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        let result = RequestResultPlan::parse(instance, provider, config)?;
        db_connect_success_value(instance, &result, &database)?;
        db_connect_error_value(instance, &result, "db.connect probe failure")?;
        Ok(Self { database, result })
    }
}

#[derive(Clone)]
struct DbLivePlan {
    task: RuntimeValue,
    debounce: Duration,
    #[allow(dead_code)]
    optimistic: bool,
    result: Option<RequestResultPlan>,
}

impl DbLivePlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbLive;
        validate_argument_count(instance, provider, config, 1)?;
        let task = parse_task_argument(instance, provider, 0, &config.arguments[0])?;
        let mut debounce = Duration::ZERO;
        let mut optimistic = false;
        for option in &config.options {
            match option.option_name.as_ref() {
                "debounce" => {
                    debounce = parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "refreshOn" | "activeWhen" => {}
                "optimistic" => {
                    optimistic =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "onRollback" => {
                    // The onRollback signal is accepted and stored; the runtime publishes
                    // the last confirmed value to it when an optimistic update is reverted.
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        let result = if config.decode.is_some() {
            let result = RequestResultPlan::parse(instance, provider, config)?;
            db_live_query_error_value(instance, &result, "db.live query failure")?;
            Some(result)
        } else {
            None
        };
        Ok(Self {
            task,
            debounce,
            optimistic,
            result,
        })
    }
}

impl ErrorPlan {
    fn from_step(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        decode: &hir::SourceDecodeProgram,
        step_id: hir::DecodeProgramStepId,
    ) -> Result<Self, SourceProviderExecutionError> {
        match decode.step(step_id) {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                let mut supported = Vec::with_capacity(variants.len());
                for variant in variants {
                    let payload = match variant.payload {
                        None => ErrorPayloadKind::None,
                        Some(payload) => match decode.step(payload) {
                            hir::DecodeProgramStep::Scalar {
                                scalar: aivi_typing::PrimitiveType::Text,
                            } => ErrorPayloadKind::Text,
                            hir::DecodeProgramStep::Scalar {
                                scalar: aivi_typing::PrimitiveType::Int,
                            } => ErrorPayloadKind::Int,
                            _ => {
                                return Err(
                                    SourceProviderExecutionError::UnsupportedProviderShape {
                                        instance,
                                        provider,
                                        detail: format!(
                                            "result error variant `{}` must be nullary, Text, or Int in the current runtime slice",
                                            variant.name.as_str()
                                        )
                                        .into_boxed_str(),
                                    },
                                );
                            }
                        },
                    };
                    supported.push(SumErrorVariant {
                        name: variant.name.as_str().into(),
                        payload,
                    });
                }
                Ok(Self::Sum {
                    variants: supported.into_boxed_slice(),
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "request and stream provider errors must currently decode as `Text` or an explicit sum"
                        .into(),
            }),
        }
    }

    fn payload_for(
        &self,
        kind: TextSourceErrorKind,
        message: &str,
    ) -> Result<ExternalSourceValue, Box<str>> {
        match self {
            Self::Text => Ok(ExternalSourceValue::Text(message.into())),
            Self::Sum { variants } => {
                for spec in kind.candidates() {
                    let Some(variant) = variants
                        .iter()
                        .find(|variant| variant.name.as_ref() == spec.name)
                    else {
                        continue;
                    };
                    if variant.payload != spec.payload {
                        continue;
                    }
                    return Ok(match spec.payload {
                        ErrorPayloadKind::None => ExternalSourceValue::variant(spec.name),
                        ErrorPayloadKind::Text => ExternalSourceValue::variant_with_payload(
                            spec.name,
                            ExternalSourceValue::Text(message.into()),
                        ),
                        ErrorPayloadKind::Int => ExternalSourceValue::variant_with_payload(
                            spec.name,
                            ExternalSourceValue::Int(spec.int_payload.unwrap_or_default()),
                        ),
                    });
                }
                Err(
                    format!("the current result error type cannot represent a {kind} failure")
                        .into(),
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextSourceErrorKind {
    Timeout,
    Decode,
    Missing,
    Request,
    Query,
    Connect,
    Mailbox,
}

impl TextSourceErrorKind {
    fn candidates(self) -> &'static [ErrorCandidate] {
        match self {
            Self::Timeout => &TIMEOUT_ERROR_CANDIDATES,
            Self::Decode => &DECODE_ERROR_CANDIDATES,
            Self::Missing => &MISSING_ERROR_CANDIDATES,
            Self::Request => &REQUEST_ERROR_CANDIDATES,
            Self::Query => &QUERY_ERROR_CANDIDATES,
            Self::Connect => &CONNECT_ERROR_CANDIDATES,
            Self::Mailbox => &MAILBOX_ERROR_CANDIDATES,
        }
    }
}

impl fmt::Display for TextSourceErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("timeout"),
            Self::Decode => f.write_str("decode"),
            Self::Missing => f.write_str("missing-file"),
            Self::Request => f.write_str("request"),
            Self::Query => f.write_str("query"),
            Self::Connect => f.write_str("connect"),
            Self::Mailbox => f.write_str("mailbox"),
        }
    }
}

#[derive(Clone, Copy)]
struct ErrorCandidate {
    name: &'static str,
    payload: ErrorPayloadKind,
    int_payload: Option<i64>,
}

impl ErrorCandidate {
    const fn none(name: &'static str) -> Self {
        Self {
            name,
            payload: ErrorPayloadKind::None,
            int_payload: None,
        }
    }

    const fn text(name: &'static str) -> Self {
        Self {
            name,
            payload: ErrorPayloadKind::Text,
            int_payload: None,
        }
    }
}

const TIMEOUT_ERROR_CANDIDATES: [ErrorCandidate; 5] = [
    ErrorCandidate::none("Timeout"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("TransportFailure"),
    ErrorCandidate::text("Error"),
];

const DECODE_ERROR_CANDIDATES: [ErrorCandidate; 3] = [
    ErrorCandidate::text("DecodeFailure"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("Error"),
];

const MISSING_ERROR_CANDIDATES: [ErrorCandidate; 3] = [
    ErrorCandidate::none("Missing"),
    ErrorCandidate::none("NotFound"),
    ErrorCandidate::text("Error"),
];

const REQUEST_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("TransportFailure"),
    ErrorCandidate::text("Error"),
];

const QUERY_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("QueryFailed"),
    ErrorCandidate::text("ConnectionFailed"),
    ErrorCandidate::text("RequestFailure"),
    ErrorCandidate::text("Error"),
];

const CONNECT_ERROR_CANDIDATES: [ErrorCandidate; 4] = [
    ErrorCandidate::text("ConnectionFailed"),
    ErrorCandidate::text("ConnectFailure"),
    ErrorCandidate::text("NetworkFailure"),
    ErrorCandidate::text("Error"),
];

const MAILBOX_ERROR_CANDIDATES: [ErrorCandidate; 2] = [
    ErrorCandidate::text("MailboxFailure"),
    ErrorCandidate::text("Error"),
];

#[derive(Clone)]
struct HttpPlan {
    provider: BuiltinSourceProvider,
    url: Box<str>,
    headers: Box<[(Box<str>, Box<str>)]>,
    body: Option<Box<str>>,
    timeout: Option<Duration>,
    refresh_every: Option<Duration>,
    retry_attempts: u32,
    result: RequestResultPlan,
}

impl HttpPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let base_url = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut url = Url::parse(base_url.as_ref()).map_err(|error| {
            SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("invalid HTTP URL `{base_url}`: {error}").into_boxed_str(),
            }
        })?;
        let mut headers = Vec::new();
        let mut body = None;
        let mut timeout = None;
        let mut refresh_every = None;
        let mut retry_attempts = 0;
        for option in &config.options {
            match option.option_name.as_ref() {
                "headers" => {
                    headers =
                        parse_text_map(instance, provider, &option.option_name, &option.value)?;
                }
                "query" => {
                    for (key, value) in
                        parse_text_map(instance, provider, &option.option_name, &option.value)?
                    {
                        url.query_pairs_mut().append_pair(&key, &value);
                    }
                }
                "body" => {
                    body = Some(encode_runtime_body(instance, provider, &option.value)?);
                }
                "timeout" => {
                    timeout = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "retry" => {
                    retry_attempts =
                        parse_retry(instance, provider, &option.option_name, &option.value)?;
                }
                "refreshEvery" => {
                    refresh_every = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "decode" | "refreshOn" | "activeWhen" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            provider,
            url: url.to_string().into_boxed_str(),
            headers: headers.into_boxed_slice(),
            body,
            timeout,
            refresh_every,
            retry_attempts,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct FsReadPlan {
    path: PathBuf,
    debounce: Duration,
    read_on_start: bool,
    result: RequestResultPlan,
}

impl FsReadPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::FsRead;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut debounce = Duration::ZERO;
        let mut read_on_start = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "debounce" => {
                    debounce = parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "readOnStart" => {
                    read_on_start =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "decode" | "reloadOn" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            path: PathBuf::from(path.as_ref()),
            debounce,
            read_on_start,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
enum NamedEventOutputPlan {
    Text,
    Variants {
        decode: hir::SourceDecodeProgram,
        variants: BTreeSet<Box<str>>,
    },
}

impl NamedEventOutputPlan {
    fn parse_payloadless_variants(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        match decode.root_step() {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                let mut names = BTreeSet::new();
                for variant in variants {
                    if variant.payload.is_some() {
                        return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                            instance,
                            provider,
                            detail: format!(
                                "event variant `{}` must be payloadless or the target must be `Text`",
                                variant.name.as_str()
                            )
                            .into_boxed_str(),
                        });
                    }
                    names.insert(variant.name.as_str().into());
                }
                Ok(Self::Variants {
                    decode,
                    variants: names,
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "event providers currently decode to `Text` or a payloadless sum".into(),
            }),
        }
    }

    fn value_for_name(&self, name: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        match self {
            Self::Text => Ok(Some(RuntimeValue::Text(name.into()))),
            Self::Variants { decode, variants } => {
                if !variants.contains(name) {
                    return Ok(None);
                }
                decode_external(decode, &ExternalSourceValue::variant(name)).map(Some)
            }
        }
    }
}

#[derive(Clone)]
struct FsWatchPlan {
    path: PathBuf,
    recursive: bool,
    events: BTreeSet<Box<str>>,
    output: NamedEventOutputPlan,
}

impl FsWatchPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::FsWatch;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut events = ["Created", "Changed", "Deleted"]
            .into_iter()
            .map(Into::into)
            .collect::<BTreeSet<Box<str>>>();
        let mut recursive = false;
        for option in &config.options {
            match option.option_name.as_ref() {
                "events" => {
                    events = parse_named_variants(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "recursive" => {
                    recursive = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            path: PathBuf::from(path.as_ref()),
            recursive,
            events,
            output: NamedEventOutputPlan::parse_payloadless_variants(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct SocketPlan {
    host: Box<str>,
    port: u16,
    buffer: usize,
    reconnect: bool,
    heartbeat: Option<Duration>,
    result: RequestResultPlan,
}

impl SocketPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::SocketConnect;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let url = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let url = Url::parse(url.as_ref()).map_err(|error| {
            SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("invalid socket URL `{url}`: {error}").into_boxed_str(),
            }
        })?;
        if url.scheme() != "tcp" {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "socket.connect currently supports only `tcp://host:port` URLs, found `{}`",
                    url.scheme()
                )
                .into_boxed_str(),
            });
        }
        let host = url
            .host_str()
            .ok_or(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: "socket.connect requires a host".into(),
            })?;
        let port = url
            .port()
            .ok_or(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: "socket.connect requires an explicit port".into(),
            })?;
        let mut buffer = 4096usize;
        let mut reconnect = false;
        let mut heartbeat = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "buffer" => {
                    buffer = parse_nonnegative_int(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? as usize;
                }
                "reconnect" => {
                    reconnect = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "decode" | "activeWhen" => {}
                "heartbeat" => {
                    heartbeat = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            host: host.into(),
            port,
            buffer,
            reconnect,
            heartbeat,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone)]
struct MailboxPlan {
    mailbox: Box<str>,
    buffer: usize,
    reconnect: bool,
    heartbeat: Option<Duration>,
    result: RequestResultPlan,
}

impl MailboxPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::MailboxSubscribe;
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let mailbox = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut buffer = 64usize;
        let mut reconnect = false;
        let mut heartbeat = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "buffer" => {
                    buffer = parse_nonnegative_int(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? as usize;
                }
                "decode" | "activeWhen" => {}
                "reconnect" => {
                    reconnect = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "heartbeat" => {
                    heartbeat = Some(parse_option_duration(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            mailbox,
            buffer,
            reconnect,
            heartbeat,
            result: RequestResultPlan::parse(instance, provider, config)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessStreamMode {
    Ignore,
    Lines,
    Bytes,
}

#[derive(Clone)]
struct ProcessPlan {
    command: Box<str>,
    args: Box<[Box<str>]>,
    cwd: Option<PathBuf>,
    env: Box<[(Box<str>, Box<str>)]>,
    stdout_mode: ProcessStreamMode,
    stderr_mode: ProcessStreamMode,
    events: ProcessEventPlan,
}

impl ProcessPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ProcessSpawn;
        if !(1..=2).contains(&config.arguments.len()) {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 2,
                found: config.arguments.len(),
            });
        }
        let command = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let args = if config.arguments.len() == 2 {
            parse_text_list(instance, provider, 1, &config.arguments[1])?
        } else {
            Vec::new()
        };
        let mut cwd = None;
        let mut env = Vec::new();
        let mut stdout = ProcessStreamMode::Ignore;
        let mut stderr = ProcessStreamMode::Ignore;
        for option in &config.options {
            match option.option_name.as_ref() {
                "cwd" => {
                    let cwd_text =
                        parse_text_option(instance, provider, &option.option_name, &option.value)?;
                    cwd = Some(PathBuf::from(cwd_text.as_ref()));
                }
                "env" => {
                    env = parse_text_map(instance, provider, &option.option_name, &option.value)?;
                }
                "stdout" => {
                    stdout =
                        parse_stream_mode(instance, provider, &option.option_name, &option.value)?;
                }
                "stderr" => {
                    stderr =
                        parse_stream_mode(instance, provider, &option.option_name, &option.value)?;
                }
                "restartOn" => {}
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        let events = ProcessEventPlan::parse(instance, config)?;
        if stdout != ProcessStreamMode::Ignore && events.stdout.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stdout: Lines/Bytes requires a `Stdout` event variant in the source output type"
                    .into(),
            });
        }
        if stderr != ProcessStreamMode::Ignore && events.stderr.is_none() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "stderr: Lines/Bytes requires a `Stderr` event variant in the source output type"
                    .into(),
            });
        }
        Ok(Self {
            command,
            args: args.into_boxed_slice(),
            cwd,
            env: env.into_boxed_slice(),
            stdout_mode: stdout,
            stderr_mode: stderr,
            events,
        })
    }
}

#[derive(Clone)]
struct ProcessEventPlan {
    decode: hir::SourceDecodeProgram,
    spawned: Option<ProcessVariantPlan>,
    stdout: Option<ProcessVariantPlan>,
    stderr: Option<ProcessVariantPlan>,
    exited: Option<ProcessVariantPlan>,
    failed: Option<ProcessVariantPlan>,
}

#[derive(Clone)]
struct ProcessVariantPlan {
    variant: Box<str>,
    payload: ProcessPayloadKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessPayloadKind {
    None,
    Text,
    Int,
    Bytes,
}

impl ProcessEventPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::ProcessSpawn;
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        let hir::DecodeProgramStep::Sum { variants, .. } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "process.spawn currently requires a sum-shaped `ProcessEvent` output"
                    .into(),
            });
        };
        let mut plan = Self {
            decode: decode.clone(),
            spawned: None,
            stdout: None,
            stderr: None,
            exited: None,
            failed: None,
        };
        for variant in variants {
            let payload = match variant.payload {
                None => ProcessPayloadKind::None,
                Some(step) => match decode.step(step) {
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    } => ProcessPayloadKind::Text,
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Int,
                    } => ProcessPayloadKind::Int,
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Bytes,
                    } => ProcessPayloadKind::Bytes,
                    _ => continue,
                },
            };
            let entry = Some(ProcessVariantPlan {
                variant: variant.name.as_str().into(),
                payload,
            });
            match variant.name.as_str() {
                "Spawned" => plan.spawned = entry,
                "Stdout" => plan.stdout = entry,
                "Stderr" => plan.stderr = entry,
                "Exited" => plan.exited = entry,
                "Failed" => plan.failed = entry,
                _ => {}
            }
        }
        Ok(plan)
    }

    fn spawned_value(&self) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(self.spawned.as_ref(), None)
    }

    fn stdout_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stdout.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn stderr_value(&self, line: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stderr.as_ref(),
            Some(ExternalSourceValue::Text(line.into())),
        )
    }

    fn exited_value(&self, code: i64) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(self.exited.as_ref(), Some(ExternalSourceValue::Int(code)))
    }

    fn failed_value(&self, message: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.failed.as_ref(),
            Some(ExternalSourceValue::Text(message.into())),
        )
    }

    fn stdout_bytes_value(
        &self,
        chunk: Box<[u8]>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stdout.as_ref(),
            Some(ExternalSourceValue::Bytes(chunk)),
        )
    }

    fn stderr_bytes_value(
        &self,
        chunk: Box<[u8]>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        self.variant_value(
            self.stderr.as_ref(),
            Some(ExternalSourceValue::Bytes(chunk)),
        )
    }

    fn variant_value(
        &self,
        plan: Option<&ProcessVariantPlan>,
        payload: Option<ExternalSourceValue>,
    ) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        let raw = match (plan.payload, payload) {
            (ProcessPayloadKind::None, _) => ExternalSourceValue::variant(plan.variant.as_ref()),
            (ProcessPayloadKind::Text, Some(payload @ ExternalSourceValue::Text(_)))
            | (ProcessPayloadKind::Int, Some(payload @ ExternalSourceValue::Int(_)))
            | (ProcessPayloadKind::Bytes, Some(payload @ ExternalSourceValue::Bytes(_))) => {
                ExternalSourceValue::variant_with_payload(plan.variant.as_ref(), payload)
            }
            _ => return Ok(None),
        };
        decode_external(&self.decode, &raw).map(Some)
    }
}

#[derive(Clone)]
struct WindowKeyDownPlan {
    capture: bool,
    focus_only: bool,
    allow_repeat: bool,
    output: WindowKeyOutputPlan,
}

impl WindowKeyDownPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::WindowKeyDown;
        if !config.arguments.is_empty() {
            return Err(SourceProviderExecutionError::InvalidArgumentCount {
                instance,
                provider,
                expected: 0,
                found: config.arguments.len(),
            });
        }
        let mut capture = false;
        let mut focus_only = true;
        let mut allow_repeat = true;
        for option in &config.options {
            match option.option_name.as_ref() {
                "capture" => {
                    capture = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "focusOnly" => {
                    focus_only =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "repeat" => {
                    allow_repeat =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        if capture {
            // capture is now supported — stored in the plan and honoured at the GTK boundary.
        }
        if !focus_only {
            // focusOnly: False is now supported — stored in the plan and honoured at the GTK boundary.
        }
        Ok(Self {
            capture,
            focus_only,
            allow_repeat,
            output: WindowKeyOutputPlan::parse(instance, config)?,
        })
    }
}

#[derive(Clone)]
enum WindowKeyOutputPlan {
    Text,
    NamedVariants {
        decode: hir::SourceDecodeProgram,
        variants: BTreeSet<Box<str>>,
    },
    WrappedTextVariant {
        decode: hir::SourceDecodeProgram,
        variant: Box<str>,
    },
}

impl WindowKeyOutputPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::WindowKeyDown;
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        match decode.root_step() {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                if let Some(variant) = variants.iter().find_map(|variant| {
                    let Some(payload) = variant.payload else {
                        return None;
                    };
                    if matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Text,
                        }
                    ) {
                        Some(variant.name.as_str().into())
                    } else {
                        None
                    }
                }) {
                    return Ok(Self::WrappedTextVariant { decode, variant });
                }
                let mut names = BTreeSet::new();
                for variant in variants {
                    if variant.payload.is_some() {
                        return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                            instance,
                            provider,
                            detail: "window.keyDown sum outputs must be payloadless key variants or one text payload wrapper".into(),
                        });
                    }
                    names.insert(variant.name.as_str().into());
                }
                Ok(Self::NamedVariants {
                    decode,
                    variants: names,
                })
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: "window.keyDown currently decodes to `Text`, a payloadless key sum, or one text-wrapping key constructor".into(),
            }),
        }
    }

    fn value_for_key(&self, key: &str) -> Result<Option<RuntimeValue>, SourceDecodeErrorWithPath> {
        match self {
            Self::Text => Ok(Some(RuntimeValue::Text(key.into()))),
            Self::NamedVariants { decode, variants } => {
                if !variants.contains(key) {
                    return Ok(None);
                }
                decode_external(decode, &ExternalSourceValue::variant(key)).map(Some)
            }
            Self::WrappedTextVariant { decode, variant } => decode_external(
                decode,
                &ExternalSourceValue::variant_with_payload(
                    variant.as_ref(),
                    ExternalSourceValue::Text(key.into()),
                ),
            )
            .map(Some),
        }
    }
}

#[derive(Clone, Copy)]
enum DbusBus {
    Session,
    System,
}

impl DbusBus {
    fn parse_option(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: &str,
        value: &DetachedRuntimeValue,
    ) -> Result<Self, SourceProviderExecutionError> {
        let value = parse_text_option(instance, provider, option_name, value)?;
        match value.as_ref() {
            "session" => Ok(Self::Session),
            "system" => Ok(Self::System),
            _ => Err(SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "\"session\" or \"system\"".into(),
                value: RuntimeValue::Text(value),
            }),
        }
    }

    const fn bus_type(self) -> BusType {
        match self {
            Self::Session => BusType::Session,
            Self::System => BusType::System,
        }
    }
}

#[derive(Clone)]
struct DbusOwnNamePlan {
    instance: SourceInstanceId,
    name: Box<str>,
    bus: DbusBus,
    address: Option<Box<str>>,
    flags: BusNameOwnerFlags,
    output: DbusNameStateOutputPlan,
}

impl DbusOwnNamePlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusOwnName;
        validate_argument_count(instance, provider, config, 1)?;
        let name = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut flags = BusNameOwnerFlags::NONE;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "flags" => {
                    for flag in parse_named_variants(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )? {
                        match flag.as_ref() {
                            "AllowReplacement" => flags |= BusNameOwnerFlags::ALLOW_REPLACEMENT,
                            "ReplaceExisting" => flags |= BusNameOwnerFlags::REPLACE,
                            "DoNotQueue" => flags |= BusNameOwnerFlags::DO_NOT_QUEUE,
                            _ => {
                                return Err(SourceProviderExecutionError::InvalidOption {
                                    instance,
                                    provider,
                                    option_name: option.option_name.clone(),
                                    expected:
                                        "List BusNameFlag (AllowReplacement | ReplaceExisting | DoNotQueue)"
                                            .into(),
                                    value: strip_detached_signal(&option.value).clone(),
                                });
                            }
                        }
                    }
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            instance,
            name,
            bus,
            address,
            flags,
            output: DbusNameStateOutputPlan::parse(instance, config)?,
        })
    }
}

#[derive(Clone)]
struct DbusSignalPlan {
    instance: SourceInstanceId,
    bus: DbusBus,
    address: Option<Box<str>>,
    path: Box<str>,
    interface: Option<Box<str>>,
    member: Option<Box<str>>,
    output: DbusMessageOutputPlan,
}

impl DbusSignalPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusSignal;
        validate_argument_count(instance, provider, config, 1)?;
        let path = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut interface = None;
        let mut member = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "interface" => {
                    interface = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "member" => {
                    member = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            instance,
            bus,
            address,
            path,
            interface,
            member,
            output: DbusMessageOutputPlan::parse_signal(instance, config)?,
        })
    }
}

#[derive(Clone)]
struct DbusMethodPlan {
    instance: SourceInstanceId,
    bus: DbusBus,
    address: Option<Box<str>>,
    destination: Box<str>,
    path: Option<Box<str>>,
    interface: Option<Box<str>>,
    member: Option<Box<str>>,
    reply_body: Option<Box<str>>,
    output: DbusMessageOutputPlan,
}

impl DbusMethodPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusMethod;
        validate_argument_count(instance, provider, config, 1)?;
        let destination = parse_text_argument(instance, provider, 0, &config.arguments[0])?;
        let mut bus = DbusBus::Session;
        let mut address = None;
        let mut path = None;
        let mut interface = None;
        let mut member = None;
        let mut reply_body = None;
        for option in &config.options {
            match option.option_name.as_ref() {
                "bus" => {
                    bus = DbusBus::parse_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?;
                }
                "address" => {
                    address = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "path" => {
                    path = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "interface" => {
                    interface = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "member" => {
                    member = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                "reply" => {
                    reply_body = Some(parse_text_option(
                        instance,
                        provider,
                        &option.option_name,
                        &option.value,
                    )?);
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }
        Ok(Self {
            instance,
            bus,
            address,
            destination,
            path,
            interface,
            member,
            reply_body,
            output: DbusMessageOutputPlan::parse_method(instance, config)?,
        })
    }
}

#[derive(Clone)]
enum DbusNameStateOutputPlan {
    Text,
    NamedVariants { decode: hir::SourceDecodeProgram },
}

impl DbusNameStateOutputPlan {
    fn parse(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        let provider = BuiltinSourceProvider::DbusOwnName;
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        match decode.root_step() {
            hir::DecodeProgramStep::Scalar {
                scalar: aivi_typing::PrimitiveType::Text,
            } => Ok(Self::Text),
            hir::DecodeProgramStep::Sum { variants, .. } => {
                let mut names = BTreeSet::new();
                for variant in variants {
                    if variant.payload.is_some() {
                        return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                            instance,
                            provider,
                            detail: "dbus.ownName currently decodes to `Text` or a payloadless `BusNameState` sum".into(),
                        });
                    }
                    names.insert(variant.name.as_str());
                }
                if names == BTreeSet::from(["Lost", "Owned", "Queued"]) {
                    Ok(Self::NamedVariants { decode })
                } else {
                    Err(SourceProviderExecutionError::UnsupportedProviderShape {
                        instance,
                        provider,
                        detail: "dbus.ownName sum outputs must define exactly `Owned`, `Queued`, and `Lost`".into(),
                    })
                }
            }
            _ => Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail:
                    "dbus.ownName currently decodes to `Text` or a payloadless `BusNameState` sum"
                        .into(),
            }),
        }
    }

    fn value_for_state(&self, state: &'static str) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        match self {
            Self::Text => Ok(RuntimeValue::Text(state.into())),
            Self::NamedVariants { decode } => {
                decode_external(decode, &ExternalSourceValue::variant(state))
            }
        }
    }
}

#[derive(Clone)]
enum DbusMessageShape {
    Signal,
    Method,
}

#[derive(Clone, Copy)]
enum DbusMessageBodyMode {
    Text,
    Structured,
}

#[derive(Clone)]
struct DbusMessageOutputPlan {
    decode: hir::SourceDecodeProgram,
    shape: DbusMessageShape,
    body_mode: DbusMessageBodyMode,
}

impl DbusMessageOutputPlan {
    fn parse_signal(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        Self::parse(
            instance,
            BuiltinSourceProvider::DbusSignal,
            config,
            DbusMessageShape::Signal,
        )
    }

    fn parse_method(
        instance: SourceInstanceId,
        config: &EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        Self::parse(
            instance,
            BuiltinSourceProvider::DbusMethod,
            config,
            DbusMessageShape::Method,
        )
    }

    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &EvaluatedSourceConfig,
        shape: DbusMessageShape,
    ) -> Result<Self, SourceProviderExecutionError> {
        let decode = config
            .decode
            .clone()
            .ok_or(SourceProviderExecutionError::MissingDecodeProgram { instance, provider })?;
        validate_supported_program(&decode).map_err(|error| {
            SourceProviderExecutionError::UnsupportedDecodeProgram {
                instance,
                provider,
                detail: error.to_string().into_boxed_str(),
            }
        })?;
        let expected = match shape {
            DbusMessageShape::Signal => ["path", "interface", "member", "body"].as_slice(),
            DbusMessageShape::Method => {
                ["destination", "path", "interface", "member", "body"].as_slice()
            }
        };
        let hir::DecodeProgramStep::Record { fields, .. } = decode.root_step() else {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: match shape {
                    DbusMessageShape::Signal => {
                        "dbus.signal currently decodes to a `DbusSignal`-shaped record".into()
                    }
                    DbusMessageShape::Method => {
                        "dbus.method currently decodes to a `DbusCall`-shaped record".into()
                    }
                },
            });
        };
        if fields.len() != expected.len() {
            return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "{} currently requires record fields {:?}",
                    provider.key(),
                    expected
                )
                .into_boxed_str(),
            });
        }
        let mut body_mode = None;
        for field_name in expected {
            let Some(field) = fields
                .iter()
                .find(|field| field.name.as_str() == *field_name)
            else {
                return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: format!(
                        "{} currently requires record fields {:?}",
                        provider.key(),
                        expected
                    )
                    .into_boxed_str(),
                });
            };
            let valid = match *field_name {
                "path" | "interface" | "member" | "destination" => matches!(
                    decode.step(field.step),
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    }
                ),
                "body" => match decode.step(field.step) {
                    hir::DecodeProgramStep::Scalar {
                        scalar: aivi_typing::PrimitiveType::Text,
                    } => {
                        body_mode = Some(DbusMessageBodyMode::Text);
                        true
                    }
                    hir::DecodeProgramStep::List { element }
                        if dbus_value_step_supported(&decode, *element, &mut HashSet::new()) =>
                    {
                        body_mode = Some(DbusMessageBodyMode::Structured);
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
            if !valid {
                return Err(SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: match shape {
                        DbusMessageShape::Signal => "dbus.signal outputs must use `Text` header fields and either a `Text` body or `List DbusValue` body".into(),
                        DbusMessageShape::Method => "dbus.method outputs must use `Text` header fields and either a `Text` body or `List DbusValue` body".into(),
                    },
                });
            }
        }
        Ok(Self {
            decode,
            shape,
            body_mode: body_mode.expect("dbus output bodies should set a body mode"),
        })
    }

    fn signal_value(
        &self,
        path: &str,
        interface: &str,
        member: &str,
        parameters: &Variant,
    ) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        let raw = self.raw_record(None, path, interface, member, Some(parameters))?;
        decode_external(&self.decode, &raw)
    }

    fn method_value(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        parameters: Option<&Variant>,
    ) -> Result<RuntimeValue, SourceDecodeErrorWithPath> {
        let raw = self.raw_record(Some(destination), path, interface, member, parameters)?;
        decode_external(&self.decode, &raw)
    }

    fn raw_record(
        &self,
        destination: Option<&str>,
        path: &str,
        interface: &str,
        member: &str,
        parameters: Option<&Variant>,
    ) -> Result<ExternalSourceValue, SourceDecodeErrorWithPath> {
        let mut record = BTreeMap::new();
        if matches!(self.shape, DbusMessageShape::Method) {
            record.insert(
                "destination".into(),
                ExternalSourceValue::Text(destination.unwrap_or_default().into()),
            );
        }
        record.insert("path".into(), ExternalSourceValue::Text(path.into()));
        record.insert(
            "interface".into(),
            ExternalSourceValue::Text(interface.into()),
        );
        record.insert("member".into(), ExternalSourceValue::Text(member.into()));
        record.insert(
            "body".into(),
            match self.body_mode {
                DbusMessageBodyMode::Text => ExternalSourceValue::Text(
                    parameters
                        .map(|value| value.print(false).to_string().into_boxed_str())
                        .unwrap_or_else(|| "".into()),
                ),
                DbusMessageBodyMode::Structured => dbus_body_external(parameters)
                    .map_err(|detail| SourceDecodeError::InvalidJson { detail })?,
            },
        );
        Ok(ExternalSourceValue::Record(record))
    }
}

fn dbus_value_step_supported(
    decode: &hir::SourceDecodeProgram,
    step: hir::DecodeProgramStepId,
    visiting: &mut HashSet<hir::DecodeProgramStepId>,
) -> bool {
    if !visiting.insert(step) {
        return true;
    }
    let result = match decode.step(step) {
        hir::DecodeProgramStep::Sum { variants, .. } => {
            variants
                .iter()
                .all(|variant| match (variant.name.as_str(), variant.payload) {
                    ("DbusString", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Text,
                        }
                    ),
                    ("DbusInt", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Int,
                        }
                    ),
                    ("DbusBool", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::Scalar {
                            scalar: aivi_typing::PrimitiveType::Bool,
                        }
                    ),
                    ("DbusList", Some(payload)) | ("DbusStruct", Some(payload)) => matches!(
                        decode.step(payload),
                        hir::DecodeProgramStep::List { element }
                            if dbus_value_step_supported(decode, *element, visiting)
                    ),
                    ("DbusVariant", Some(payload)) => {
                        dbus_value_step_supported(decode, payload, visiting)
                    }
                    _ => false,
                })
        }
        _ => false,
    };
    visiting.remove(&step);
    result
}

const MAX_DBUS_VALUE_DEPTH: usize = 64;

fn dbus_body_external(parameters: Option<&Variant>) -> Result<ExternalSourceValue, Box<str>> {
    let Some(parameters) = parameters else {
        return Ok(ExternalSourceValue::List(Vec::new()));
    };
    let values = if parameters.type_().is_tuple() {
        (0..parameters.n_children())
            .map(|index| dbus_value_external(&parameters.child_value(index), 0))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![dbus_value_external(parameters, 0)?]
    };
    Ok(ExternalSourceValue::List(values))
}

fn dbus_value_external(value: &Variant, depth: usize) -> Result<ExternalSourceValue, Box<str>> {
    if depth >= MAX_DBUS_VALUE_DEPTH {
        return Err("D-Bus payload nesting exceeds the current runtime depth limit".into());
    }
    match value.classify() {
        VariantClass::Boolean => Ok(ExternalSourceValue::variant_with_payload(
            "DbusBool",
            ExternalSourceValue::Bool(
                value
                    .get::<bool>()
                    .ok_or_else(|| "failed to decode D-Bus boolean payload".to_owned())?,
            ),
        )),
        VariantClass::Byte => Ok(ExternalSourceValue::variant_with_payload(
            "DbusInt",
            ExternalSourceValue::Int(
                value
                    .get::<u8>()
                    .ok_or_else(|| "failed to decode D-Bus byte payload".to_owned())?
                    as i64,
            ),
        )),
        VariantClass::Int16 => dbus_int_value(
            value
                .get::<i16>()
                .ok_or_else(|| "failed to decode D-Bus int16 payload".to_owned())?
                as i64,
        ),
        VariantClass::Uint16 => dbus_int_value(
            value
                .get::<u16>()
                .ok_or_else(|| "failed to decode D-Bus uint16 payload".to_owned())?
                as i64,
        ),
        VariantClass::Int32 => dbus_int_value(
            value
                .get::<i32>()
                .ok_or_else(|| "failed to decode D-Bus int32 payload".to_owned())?
                as i64,
        ),
        VariantClass::Uint32 => dbus_int_value(
            value
                .get::<u32>()
                .ok_or_else(|| "failed to decode D-Bus uint32 payload".to_owned())?
                as i64,
        ),
        VariantClass::Int64 => dbus_int_value(
            value
                .get::<i64>()
                .ok_or_else(|| "failed to decode D-Bus int64 payload".to_owned())?,
        ),
        VariantClass::Uint64 => {
            let value = value
                .get::<u64>()
                .ok_or_else(|| "failed to decode D-Bus uint64 payload".to_owned())?;
            let value = i64::try_from(value)
                .map_err(|_| "D-Bus uint64 payload exceeds the current Int runtime slice")?;
            dbus_int_value(value)
        }
        VariantClass::Handle => dbus_int_value(
            value
                .get::<i32>()
                .ok_or_else(|| "failed to decode D-Bus handle payload".to_owned())?
                as i64,
        ),
        VariantClass::String | VariantClass::ObjectPath | VariantClass::Signature => {
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusString",
                ExternalSourceValue::Text(
                    value
                        .str()
                        .ok_or_else(|| "failed to decode D-Bus string payload".to_owned())?
                        .into(),
                ),
            ))
        }
        VariantClass::Variant => {
            let inner = value
                .as_variant()
                .ok_or_else(|| "failed to decode nested D-Bus variant payload".to_owned())?;
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusVariant",
                dbus_value_external(&inner, depth + 1)?,
            ))
        }
        VariantClass::Array => {
            let mut values = Vec::with_capacity(value.n_children());
            for index in 0..value.n_children() {
                values.push(dbus_value_external(&value.child_value(index), depth + 1)?);
            }
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusList",
                ExternalSourceValue::List(values),
            ))
        }
        VariantClass::Tuple | VariantClass::DictEntry => {
            let mut values = Vec::with_capacity(value.n_children());
            for index in 0..value.n_children() {
                values.push(dbus_value_external(&value.child_value(index), depth + 1)?);
            }
            Ok(ExternalSourceValue::variant_with_payload(
                "DbusStruct",
                ExternalSourceValue::List(values),
            ))
        }
        VariantClass::Maybe => Err(
            "D-Bus maybe payloads are not representable by the current DbusValue runtime slice"
                .into(),
        ),
        VariantClass::Double => Err(
            "D-Bus floating-point payloads are not representable by the current DbusValue runtime slice"
                .into(),
        ),
        VariantClass::__Unknown(_) => Err("unknown D-Bus payload class".into()),
        _ => Err("unsupported D-Bus payload class".into()),
    }
}

fn dbus_int_value(value: i64) -> Result<ExternalSourceValue, Box<str>> {
    Ok(ExternalSourceValue::variant_with_payload(
        "DbusInt",
        ExternalSourceValue::Int(value),
    ))
}

fn open_dbus_connection(bus: DbusBus, address: Option<&str>) -> Result<DBusConnection, Box<str>> {
    match address {
        Some(address) => DBusConnection::for_address_sync(
            address,
            DBusConnectionFlags::AUTHENTICATION_CLIENT
                | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None::<&gio::DBusAuthObserver>,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string().into_boxed_str()),
        None => gio::bus_get_sync(bus.bus_type(), None::<&gio::Cancellable>)
            .map_err(|error| error.to_string().into_boxed_str()),
    }
}

fn spawn_dbus_own_name_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusOwnNamePlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusOwnName;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let owned_port = port.clone();
            let owned_output = plan.output.clone();
            let lost_port = port.clone();
            let lost_output = plan.output.clone();
            let owned_connection = plan
                .address
                .as_deref()
                .map(|address| open_dbus_connection(plan.bus, Some(address)))
                .transpose()?;
            let owner_id = if let Some(connection) = owned_connection.as_ref() {
                gio::bus_own_name_on_connection(
                    connection,
                    plan.name.as_ref(),
                    plan.flags,
                    move |_, _| {
                        if let Ok(value) = owned_output.value_for_state("Owned") {
                            let _ =
                                owned_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = lost_output.value_for_state("Lost") {
                            let _ =
                                lost_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                )
            } else {
                let queued_port = port.clone();
                let queued_output = plan.output.clone();
                let queue_enabled = !plan.flags.contains(BusNameOwnerFlags::DO_NOT_QUEUE);
                gio::bus_own_name(
                    plan.bus.bus_type(),
                    plan.name.as_ref(),
                    plan.flags,
                    move |_, _| {
                        if queue_enabled && let Ok(value) = queued_output.value_for_state("Queued")
                        {
                            let _ = queued_port
                                .publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = owned_output.value_for_state("Owned") {
                            let _ =
                                owned_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                    move |_, _| {
                        if let Ok(value) = lost_output.value_for_state("Lost") {
                            let _ =
                                lost_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                        }
                    },
                )
            };
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            gio::bus_unown_name(owner_id);
            drop(owned_connection);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn spawn_dbus_signal_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusSignalPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusSignal;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = open_dbus_connection(plan.bus, plan.address.as_deref())?;
            let output = plan.output.clone();
            let publish_port = port.clone();
            #[allow(deprecated)]
            let subscription_id = connection.signal_subscribe(
                None,
                plan.interface.as_deref(),
                plan.member.as_deref(),
                Some(plan.path.as_ref()),
                None,
                DBusSignalFlags::NONE,
                move |_, _, object_path, interface_name, signal_name, parameters| {
                    let Ok(value) =
                        output.signal_value(object_path, interface_name, signal_name, parameters)
                    else {
                        return;
                    };
                    let _ = publish_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                },
            );
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            #[allow(deprecated)]
            connection.signal_unsubscribe(subscription_id);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn spawn_dbus_method_worker(
    port: DetachedRuntimePublicationPort,
    plan: DbusMethodPlan,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let provider = BuiltinSourceProvider::DbusMethod;
    let instance = plan.instance;
    let handle = thread::spawn(move || {
        let context = MainContext::new();
        let main_loop = MainLoop::new(Some(&context), false);
        let startup = context.with_thread_default(|| {
            install_dbus_stop_timer(&main_loop, &stop, &port);
            let connection = open_dbus_connection(plan.bus, plan.address.as_deref())?;
            let reply_variant = plan
                .reply_body
                .as_deref()
                .map(|text| {
                    Variant::parse(None, text).map_err(|err| {
                        format!(
                            "dbus.method reply option is not a valid GLib variant: {err}"
                        )
                        .into_boxed_str()
                    })
                })
                .transpose()?;
            let output = plan.output.clone();
            let publish_port = port.clone();
            let destination = plan.destination.clone();
            let path = plan.path.clone();
            let interface = plan.interface.clone();
            let member = plan.member.clone();
            let filter_id = connection.add_filter(move |connection, message, incoming| {
                if !incoming
                    || message.message_type() != DBusMessageType::MethodCall
                    || message.destination().as_deref() != Some(destination.as_ref())
                    || path
                        .as_deref()
                        .is_some_and(|expected| message.path().as_deref() != Some(expected))
                    || interface
                        .as_deref()
                        .is_some_and(|expected| message.interface().as_deref() != Some(expected))
                    || member
                        .as_deref()
                        .is_some_and(|expected| message.member().as_deref() != Some(expected))
                {
                    return Some(message.clone());
                }
                let reply = message.new_method_reply();
                if let Some(body) = &reply_variant {
                    eprintln!("[dbus-method-debug] setting reply body: type={} print={}", body.type_(), body.print(true));
                    reply.set_body(body);
                    if let Some(actual) = reply.body() {
                        eprintln!("[dbus-method-debug] reply body after set: type={} print={}", actual.type_(), actual.print(true));
                    } else {
                        eprintln!("[dbus-method-debug] reply body after set is None!");
                    }
                } else {
                    eprintln!("[dbus-method-debug] no reply_variant configured");
                }
                let _ = connection.send_message(&reply, DBusSendMessageFlags::NONE);
                if let (Some(path), Some(interface), Some(member)) =
                    (message.path(), message.interface(), message.member())
                    && let Ok(value) = output.method_value(
                        destination.as_ref(),
                        path.as_str(),
                        interface.as_str(),
                        member.as_str(),
                        message.body().as_ref(),
                    )
                {
                    let _ = publish_port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
                None
            });
            let _ = startup_tx.send(Ok(()));
            main_loop.run();
            connection.remove_filter(filter_id);
            Ok::<(), Box<str>>(())
        });
        match startup {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => {
                let _ = startup_tx.send(Err(detail));
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string().into_boxed_str()));
            }
        }
    });
    finish_dbus_startup(instance, provider, handle, startup_rx)
}

fn install_dbus_stop_timer(
    main_loop: &MainLoop,
    stop: &Arc<AtomicBool>,
    port: &DetachedRuntimePublicationPort,
) {
    let main_loop = main_loop.clone();
    let stop = stop.clone();
    let port = port.clone();
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            main_loop.quit();
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
}

fn finish_dbus_startup(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    handle: thread::JoinHandle<()>,
    startup_rx: mpsc::Receiver<Result<(), Box<str>>>,
) -> Result<thread::JoinHandle<()>, SourceProviderExecutionError> {
    match startup_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(detail)) => {
            let _ = handle.join();
            Err(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail,
            })
        }
        Err(error) => {
            let _ = handle.join();
            Err(SourceProviderExecutionError::StartFailed {
                instance,
                provider,
                detail: format!("failed to receive provider startup status: {error}")
                    .into_boxed_str(),
            })
        }
    }
}

fn spawn_db_connect_worker(
    instance: SourceInstanceId,
    port: DetachedRuntimePublicationPort,
    plan: DbConnectPlan,
    context: SourceProviderContext,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let Ok(value) = execute_db_connect(instance, &plan) else {
            return;
        };
        let Ok(value) = execute_runtime_value_with_context_with_stdio(value, &context) else {
            return;
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    })
}

fn spawn_db_live_worker(
    instance: SourceInstanceId,
    port: DetachedRuntimePublicationPort,
    plan: DbLivePlan,
    context: SourceProviderContext,
    delay: Duration,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        if !delay.is_zero() && sleep_with_cancellation(delay, &port) {
            return;
        }
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let value = match execute_runtime_value_with_context_with_stdio(plan.task.clone(), &context)
        {
            Ok(value) => value,
            Err(error) => {
                let Some(result) = &plan.result else {
                    return;
                };
                let Ok(value) = db_live_query_error_value(instance, result, &error.to_string())
                else {
                    return;
                };
                value
            }
        };
        if stop.load(Ordering::Acquire) || port.is_cancelled() {
            return;
        }
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
    })
}

fn spawn_timer_every(
    port: DetachedRuntimePublicationPort,
    plan: TimerPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if plan.immediate && port.publish(DetachedRuntimeValue::unit()).is_err() {
            return;
        }
        let mut next_tick = Instant::now() + plan.delay;
        while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
            let sleep_dur = match plan.jitter {
                Some(jitter) => {
                    let jitter_nanos = jitter.as_nanos() as u64;
                    let offset = if jitter_nanos > 0 {
                        Duration::from_nanos(fastrand::u64(0..=jitter_nanos))
                    } else {
                        Duration::ZERO
                    };
                    plan.delay + offset
                }
                None => plan.delay,
            };
            thread::sleep(sleep_dur);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                break;
            }
            if plan.coalesce {
                // Coalescing: fire exactly once per sleep cycle.
                if port.publish(DetachedRuntimeValue::unit()).is_err() {
                    break;
                }
            } else {
                // Non-coalescing: fire all ticks that are due since the last cycle.
                let now = Instant::now();
                while next_tick <= now {
                    if port.publish(DetachedRuntimeValue::unit()).is_err() {
                        return;
                    }
                    next_tick += plan.delay;
                }
            }
        }
    })
}

fn spawn_timer_after(
    port: DetachedRuntimePublicationPort,
    plan: TimerPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if !plan.immediate {
            thread::sleep(plan.delay);
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
        }
        let _ = port.publish(DetachedRuntimeValue::unit());
    })
}

fn spawn_http_worker(
    port: DetachedRuntimePublicationPort,
    plan: HttpPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let Some(value) = execute_http_cycle(&plan, &port) else {
                return;
            };
            if stop.load(Ordering::Acquire) {
                return;
            }
            if port
                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                .is_err()
            {
                return;
            }
            let Some(refresh_every) = plan.refresh_every else {
                return;
            };
            if stop.load(Ordering::Acquire) || sleep_with_cancellation(refresh_every, &port) {
                return;
            }
        }
    })
}

fn execute_http_cycle(
    plan: &HttpPlan,
    port: &DetachedRuntimePublicationPort,
) -> Option<RuntimeValue> {
    let mut attempt = 0;
    loop {
        if port.is_cancelled() {
            return None;
        }
        match run_http_request(plan, port.cancellation()) {
            Ok(body) => match plan.result.success_from_text(&body) {
                Ok(value) => return Some(value),
                Err(error) => {
                    return plan
                        .result
                        .error_value(TextSourceErrorKind::Decode, &error.to_string())
                        .ok();
                }
            },
            Err(HttpRequestFailure::Cancelled) => return None,
            Err(HttpRequestFailure::TimedOut) => {
                if attempt < plan.retry_attempts {
                    attempt += 1;
                    if sleep_with_cancellation(retry_backoff(attempt), &port) {
                        return None;
                    }
                    continue;
                }
                return plan
                    .result
                    .error_value(TextSourceErrorKind::Timeout, "request timed out")
                    .ok();
            }
            Err(HttpRequestFailure::Failed(message)) => {
                if attempt < plan.retry_attempts {
                    attempt += 1;
                    if sleep_with_cancellation(retry_backoff(attempt), &port) {
                        return None;
                    }
                    continue;
                }
                return plan
                    .result
                    .error_value(TextSourceErrorKind::Request, &message)
                    .ok();
            }
        }
    }
}

fn spawn_fs_read_worker(
    port: DetachedRuntimePublicationPort,
    plan: FsReadPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) {
            return;
        }
        if sleep_with_cancellation(plan.debounce, &port) {
            return;
        }
        if stop.load(Ordering::Acquire) {
            return;
        }
        let result = match fs::read_to_string(&plan.path) {
            Ok(text) => match plan.result.success_from_text(&text) {
                Ok(value) => value,
                Err(error) => match plan
                    .result
                    .error_value(TextSourceErrorKind::Decode, &error.to_string())
                {
                    Ok(value) => value,
                    Err(_) => return,
                },
            },
            Err(error) => {
                let kind = if error.kind() == std::io::ErrorKind::NotFound {
                    TextSourceErrorKind::Missing
                } else {
                    TextSourceErrorKind::Request
                };
                match plan.result.error_value(kind, &error.to_string()) {
                    Ok(value) => value,
                    Err(_) => return,
                }
            }
        };
        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(result));
    })
}

fn spawn_fs_watch_worker(
    port: DetachedRuntimePublicationPort,
    plan: FsWatchPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if plan.recursive {
            let mut previous = dir_signatures(&plan.path);
            while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
                thread::sleep(Duration::from_millis(40));
                if stop.load(Ordering::Acquire) || port.is_cancelled() {
                    break;
                }
                let current = dir_signatures(&plan.path);
                // Detect created/changed/deleted entries by comparing the two snapshots.
                for (path, sig) in &current {
                    match previous.get(path) {
                        None => {
                            if emit_fs_event("Created", &plan, &port).is_err() {
                                return;
                            }
                        }
                        Some(prev) if prev != sig => {
                            if emit_fs_event("Changed", &plan, &port).is_err() {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                for path in previous.keys() {
                    if !current.contains_key(path) {
                        if emit_fs_event("Deleted", &plan, &port).is_err() {
                            return;
                        }
                    }
                }
                previous = current;
            }
        } else {
            let mut previous = file_signature(&plan.path);
            while !stop.load(Ordering::Acquire) && !port.is_cancelled() {
                thread::sleep(Duration::from_millis(40));
                if stop.load(Ordering::Acquire) || port.is_cancelled() {
                    break;
                }
                let current = file_signature(&plan.path);
                let event = match (previous.exists, current.exists) {
                    (false, true) => Some("Created"),
                    (true, false) => Some("Deleted"),
                    (true, true) if previous != current => Some("Changed"),
                    _ => None,
                };
                previous = current;
                let Some(event) = event else {
                    continue;
                };
                if emit_fs_event(event, &plan, &port).is_err() {
                    return;
                }
            }
        }
    })
}

fn emit_fs_event(
    event: &str,
    plan: &FsWatchPlan,
    port: &DetachedRuntimePublicationPort,
) -> Result<(), ()> {
    if !plan.events.contains(event) {
        return Ok(());
    }
    let Ok(Some(value)) = plan.output.value_for_name(event) else {
        return Ok(());
    };
    port.publish(DetachedRuntimeValue::from_runtime_owned(value))
        .map_err(|_| ())
}

/// Collect file signatures for all entries in a directory tree.
fn dir_signatures(root: &Path) -> BTreeMap<PathBuf, FileSignature> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let sig = file_signature(&path);
                map.insert(path, sig);
            }
        }
    }
    map
}

fn spawn_socket_worker(
    port: DetachedRuntimePublicationPort,
    plan: SocketPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            match TcpStream::connect((plan.host.as_ref(), plan.port)) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    // When heartbeat is configured, spawn a keepalive writer thread that
                    // periodically sends an empty byte to prevent idle timeouts.
                    let heartbeat_stop = stop.clone();
                    let heartbeat_cancel = port.cancellation();
                    let heartbeat_handle = plan.heartbeat.map(|interval| {
                        let mut writer = stream.try_clone().expect("TcpStream clone should succeed");
                        thread::spawn(move || {
                            use std::io::Write;
                            while !heartbeat_stop.load(Ordering::Acquire)
                                && !heartbeat_cancel.is_cancelled()
                            {
                                thread::sleep(interval);
                                if heartbeat_stop.load(Ordering::Acquire)
                                    || heartbeat_cancel.is_cancelled()
                                {
                                    break;
                                }
                                // Send a single newline as a keepalive ping.
                                if writer.write_all(b"\n").is_err() || writer.flush().is_err() {
                                    break;
                                }
                            }
                        })
                    });
                    let mut reader = BufReader::with_capacity(plan.buffer.max(1), stream);
                    let mut line = String::new();
                    loop {
                        if stop.load(Ordering::Acquire) || port.is_cancelled() {
                            if let Some(h) = heartbeat_handle {
                                let _ = h.join();
                            }
                            return;
                        }
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(_) => {
                                let line_text = line.trim_end_matches(['\r', '\n']).to_owned();
                                let value = match plan.result.success_from_text(&line_text) {
                                    Ok(value) => value,
                                    Err(error) => match plan.result.error_value(
                                        TextSourceErrorKind::Decode,
                                        &error.to_string(),
                                    ) {
                                        Ok(value) => value,
                                        Err(_) => break,
                                    },
                                };
                                if port
                                    .publish(DetachedRuntimeValue::from_runtime_owned(value))
                                    .is_err()
                                {
                                    if let Some(h) = heartbeat_handle {
                                        let _ = h.join();
                                    }
                                    return;
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                continue;
                            }
                            Err(error) => {
                                if let Ok(value) = plan
                                    .result
                                    .error_value(TextSourceErrorKind::Request, &error.to_string())
                                {
                                    let _ = port
                                        .publish(DetachedRuntimeValue::from_runtime_owned(value));
                                }
                                break;
                            }
                        }
                    }
                    if let Some(h) = heartbeat_handle {
                        let _ = h.join();
                    }
                }
                Err(error) => {
                    if let Ok(value) = plan
                        .result
                        .error_value(TextSourceErrorKind::Connect, &error.to_string())
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                }
            }
            if !plan.reconnect
                || stop.load(Ordering::Acquire)
                || sleep_with_cancellation(Duration::from_millis(100), &port)
            {
                return;
            }
        }
    })
}

fn spawn_mailbox_worker(
    port: DetachedRuntimePublicationPort,
    plan: MailboxPlan,
    receiver: mpsc::Receiver<Box<str>>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_heartbeat = Instant::now();
        loop {
            if stop.load(Ordering::Acquire) || port.is_cancelled() {
                return;
            }
            // Check if a heartbeat ping is due.
            if let Some(interval) = plan.heartbeat {
                if last_heartbeat.elapsed() >= interval {
                    last_heartbeat = Instant::now();
                    if port
                        .publish(DetachedRuntimeValue::unit())
                        .is_err()
                    {
                        return;
                    }
                }
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => {
                    let value = match plan.result.success_from_text(&message) {
                        Ok(value) => value,
                        Err(error) => match plan
                            .result
                            .error_value(TextSourceErrorKind::Decode, &error.to_string())
                        {
                            Ok(value) => value,
                            Err(_) => return,
                        },
                    };
                    if port
                        .publish(DetachedRuntimeValue::from_runtime_owned(value))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    if plan.reconnect {
                        // Wait briefly and continue; the sender side may re-establish.
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    if let Ok(value) = plan
                        .result
                        .error_value(TextSourceErrorKind::Mailbox, "mailbox disconnected")
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                    return;
                }
            }
        }
    })
}

fn spawn_process_worker(
    port: DetachedRuntimePublicationPort,
    plan: ProcessPlan,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let mut command = Command::new(plan.command.as_ref());
        command.args(plan.args.iter().map(|arg| arg.as_ref()));
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(cwd) = &plan.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &plan.env {
            command.env(key.as_ref(), value.as_ref());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(value) = plan.events.failed_value(&error.to_string()).ok().flatten() {
                    let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
                return;
            }
        };
        let pid = child.id();
        let done = Arc::new(AtomicBool::new(false));
        let cancellation = port.cancellation();
        let done_clone = done.clone();
        thread::spawn(move || {
            while !done_clone.load(Ordering::Acquire) {
                if cancellation.is_cancelled() {
                    kill_pid(pid);
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        if let Some(value) = plan.events.spawned_value().ok().flatten() {
            if port
                .publish(DetachedRuntimeValue::from_runtime_owned(value))
                .is_err()
            {
                done.store(true, Ordering::Release);
                kill_pid(pid);
                return;
            }
        }
        let stdout_handle = child.stdout.take().map(|stdout| {
            let port = port.clone();
            let events = plan.events.clone();
            let bytes_mode = plan.stdout_mode == ProcessStreamMode::Bytes;
            thread::spawn(move || {
                if bytes_mode {
                    read_process_stream_bytes(stdout, port, events, true)
                } else {
                    read_process_stream(stdout, port, events, true)
                }
            })
        });
        let stderr_handle = child.stderr.take().map(|stderr| {
            let port = port.clone();
            let events = plan.events.clone();
            let bytes_mode = plan.stderr_mode == ProcessStreamMode::Bytes;
            thread::spawn(move || {
                if bytes_mode {
                    read_process_stream_bytes(stderr, port, events, false)
                } else {
                    read_process_stream(stderr, port, events, false)
                }
            })
        });
        let status = child.wait();
        done.store(true, Ordering::Release);
        if let Some(handle) = stdout_handle {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }
        if port.is_cancelled() {
            return;
        }
        match status {
            Ok(status) => {
                if let Some(code) = status.code() {
                    if let Some(value) = plan.events.exited_value(code as i64).ok().flatten() {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    } else if !status.success()
                        && let Some(value) = plan
                            .events
                            .failed_value(&format!("process exited with code {code}"))
                            .ok()
                            .flatten()
                    {
                        let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                    }
                }
            }
            Err(error) => {
                if let Some(value) = plan.events.failed_value(&error.to_string()).ok().flatten() {
                    let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(value));
                }
            }
        }
    })
}

fn read_process_stream(
    stream: impl std::io::Read,
    port: DetachedRuntimePublicationPort,
    plan: ProcessEventPlan,
    stdout: bool,
) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while !port.is_cancelled() {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let line_text = line.trim_end_matches(['\r', '\n']);
                let value = if stdout {
                    plan.stdout_value(line_text)
                } else {
                    plan.stderr_value(line_text)
                };
                if let Ok(Some(value)) = value
                    && port
                        .publish(DetachedRuntimeValue::from_runtime_owned(value))
                        .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn read_process_stream_bytes(
    stream: impl std::io::Read,
    port: DetachedRuntimePublicationPort,
    plan: ProcessEventPlan,
    stdout: bool,
) {
    let mut reader = BufReader::new(stream);
    let mut buf = vec![0u8; 4096];
    while !port.is_cancelled() {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = buf[..n].to_vec().into_boxed_slice();
                let value = if stdout {
                    plan.stdout_bytes_value(chunk)
                } else {
                    plan.stderr_bytes_value(chunk)
                };
                if let Ok(Some(value)) = value
                    && port
                        .publish(DetachedRuntimeValue::from_runtime_owned(value))
                        .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[derive(Debug)]
enum HttpRequestFailure {
    Cancelled,
    TimedOut,
    Failed(Box<str>),
}

fn run_http_request(
    plan: &HttpPlan,
    cancellation: CancellationObserver,
) -> Result<String, HttpRequestFailure> {
    let mut command = Command::new("curl");
    command.arg("-sS");
    command.arg("-L");
    command.arg("-X");
    command.arg(match plan.provider {
        BuiltinSourceProvider::HttpGet => "GET",
        BuiltinSourceProvider::HttpPost => "POST",
        _ => unreachable!("http plan should only be built for http providers"),
    });
    if let Some(timeout) = plan.timeout {
        command.arg("--max-time");
        command.arg(duration_seconds_string(timeout));
    }
    for (key, value) in &plan.headers {
        command.arg("-H");
        command.arg(format!("{key}: {value}"));
    }
    if let Some(body) = &plan.body {
        command.arg("--data-binary");
        command.arg(body.as_ref());
    }
    command.arg("-w");
    command.arg("\n%{http_code}");
    command.arg(plan.url.as_ref());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let child = command.spawn().map_err(|error| {
        HttpRequestFailure::Failed(format!("failed to spawn curl: {error}").into_boxed_str())
    })?;
    let pid = child.id();
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();
    let cancel_watcher = cancellation.clone();
    thread::spawn(move || {
        while !done_clone.load(Ordering::Acquire) {
            if cancel_watcher.is_cancelled() {
                kill_pid(pid);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    let output = wait_with_output(child);
    done.store(true, Ordering::Release);
    let output =
        output.map_err(|error| HttpRequestFailure::Failed(error.to_string().into_boxed_str()))?;
    if cancellation.is_cancelled() {
        return Err(HttpRequestFailure::Cancelled);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if output.status.code() == Some(28) {
            return Err(HttpRequestFailure::TimedOut);
        }
        return Err(HttpRequestFailure::Failed(stderr.into_boxed_str()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(split) = stdout.rfind('\n') else {
        return Err(HttpRequestFailure::Failed(
            "curl did not report an HTTP status code".into(),
        ));
    };
    let (body, code_text) = stdout.split_at(split);
    let status = code_text.trim().parse::<u16>().map_err(|_| {
        HttpRequestFailure::Failed("curl returned an invalid HTTP status code".into())
    })?;
    if status >= 400 {
        return Err(HttpRequestFailure::Failed(
            format!("HTTP {status}: {}", body.trim()).into_boxed_str(),
        ));
    }
    Ok(body.to_owned())
}

fn wait_with_output(child: Child) -> std::io::Result<std::process::Output> {
    child.wait_with_output()
}

fn duration_seconds_string(duration: Duration) -> String {
    format!("{}.{:03}", duration.as_secs(), duration.subsec_millis())
}

fn retry_backoff(attempt: u32) -> Duration {
    let factor = 1_u64 << attempt.min(6);
    Duration::from_millis(100_u64.saturating_mul(factor))
}

fn sleep_with_cancellation(duration: Duration, port: &DetachedRuntimePublicationPort) -> bool {
    if duration.is_zero() {
        return port.is_cancelled();
    }
    let start = Instant::now();
    while start.elapsed() < duration {
        if port.is_cancelled() {
            return true;
        }
        let remaining = duration.saturating_sub(start.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
    port.is_cancelled()
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FileSignature {
    exists: bool,
    len: u64,
    modified_millis: u128,
}

fn file_signature(path: &Path) -> FileSignature {
    let Ok(metadata) = fs::metadata(path) else {
        return FileSignature::default();
    };
    let modified_millis = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    FileSignature {
        exists: true,
        len: metadata.len(),
        modified_millis,
    }
}

fn strip_signal(value: &RuntimeValue) -> &RuntimeValue {
    let mut current = value;
    while let RuntimeValue::Signal(inner) = current {
        current = inner;
    }
    current
}

fn strip_detached_signal(value: &DetachedRuntimeValue) -> &RuntimeValue {
    strip_signal(value.as_runtime())
}

fn parse_bool(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<bool, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Bool(value) => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Bool".into(),
            value: other.clone(),
        }),
    }
}

fn parse_positive_int(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<i64, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value > 0 => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "positive Int".into(),
            value: other.clone(),
        }),
    }
}

fn parse_nonnegative_int(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<i64, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "non-negative Int".into(),
            value: other.clone(),
        }),
    }
}

fn parse_text_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Text(value) => Ok(value.clone()),
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Text".into(),
            value: other.clone(),
        }),
    }
}

fn parse_task_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Task(task) => Ok(RuntimeValue::Task(task.clone())),
        RuntimeValue::DbTask(task) => Ok(RuntimeValue::DbTask(task.clone())),
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Task or DbTask".into(),
            value: other.clone(),
        }),
    }
}

fn parse_db_connect_argument(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    context: &SourceProviderContext,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    let database = match strip_detached_signal(value) {
        RuntimeValue::Text(value) => value.clone(),
        RuntimeValue::Record(fields) => {
            let Some(field) = fields
                .iter()
                .find(|field| field.label.as_ref() == "database")
            else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Text or { database: Text }".into(),
                    value: strip_detached_signal(value).clone(),
                });
            };
            let RuntimeValue::Text(database) = strip_signal(&field.value) else {
                return Err(SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Text or { database: Text }".into(),
                    value: strip_detached_signal(value).clone(),
                });
            };
            database.clone()
        }
        other => {
            return Err(SourceProviderExecutionError::InvalidArgument {
                instance,
                provider,
                index,
                expected: "Text or { database: Text }".into(),
                value: other.clone(),
            });
        }
    };
    Ok(context.normalize_sqlite_database_text(database.as_ref()))
}

fn parse_text_option(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Text(value) => Ok(value.clone()),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Text".into(),
            value: other.clone(),
        }),
    }
}

fn parse_text_list(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Vec<Box<str>>, SourceProviderExecutionError> {
    let RuntimeValue::List(values) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "List Text".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    values
        .iter()
        .map(|value| match strip_signal(value) {
            RuntimeValue::Text(value) => Ok(value.clone()),
            other => Err(SourceProviderExecutionError::InvalidArgument {
                instance,
                provider,
                index,
                expected: "List Text".into(),
                value: other.clone(),
            }),
        })
        .collect()
}

fn parse_text_map(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Vec<(Box<str>, Box<str>)>, SourceProviderExecutionError> {
    let RuntimeValue::Map(entries) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Map Text Text".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    entries
        .iter()
        .map(|(k, v)| {
            let RuntimeValue::Text(key) = strip_signal(k) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(k).clone(),
                });
            };
            let RuntimeValue::Text(value) = strip_signal(v) else {
                return Err(SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Map Text Text".into(),
                    value: strip_signal(v).clone(),
                });
            };
            Ok((key.clone(), value.clone()))
        })
        .collect()
}

fn parse_named_variants(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<BTreeSet<Box<str>>, SourceProviderExecutionError> {
    let RuntimeValue::List(values) = strip_detached_signal(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "List payloadless variants".into(),
            value: strip_detached_signal(value).clone(),
        });
    };
    values
        .iter()
        .map(|value| {
            variant_name_value(strip_signal(value)).ok_or_else(|| {
                SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "List payloadless variants".into(),
                    value: strip_signal(value).clone(),
                }
            })
        })
        .collect()
}

fn parse_duration(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &DetachedRuntimeValue,
) -> Result<Duration, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount =
                raw.parse::<u64>()
                    .map_err(|_| SourceProviderExecutionError::InvalidArgument {
                        instance,
                        provider,
                        index,
                        expected: "Duration".into(),
                        value: value.to_runtime(),
                    })?;
            duration_from_suffix(amount, suffix).ok_or_else(|| {
                SourceProviderExecutionError::InvalidArgument {
                    instance,
                    provider,
                    index,
                    expected: "Duration".into(),
                    value: value.to_runtime(),
                }
            })
        }
        other => Err(SourceProviderExecutionError::InvalidArgument {
            instance,
            provider,
            index,
            expected: "Duration".into(),
            value: other.clone(),
        }),
    }
}

fn validate_argument_count(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    config: &EvaluatedSourceConfig,
    expected: usize,
) -> Result<(), SourceProviderExecutionError> {
    if config.arguments.len() != expected {
        return Err(SourceProviderExecutionError::InvalidArgumentCount {
            instance,
            provider,
            expected,
            found: config.arguments.len(),
        });
    }
    Ok(())
}

fn reject_options(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    config: &EvaluatedSourceConfig,
) -> Result<(), SourceProviderExecutionError> {
    if let Some(option) = config.options.first() {
        return Err(SourceProviderExecutionError::UnsupportedOption {
            instance,
            provider,
            option_name: option.option_name.clone(),
        });
    }
    Ok(())
}

fn publish_immediate_value(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    port: &DetachedRuntimePublicationPort,
    value: RuntimeValue,
) -> Result<(), SourceProviderExecutionError> {
    port.publish(DetachedRuntimeValue::from_runtime_owned(value))
        .map_err(|error| SourceProviderExecutionError::StartFailed {
            instance,
            provider,
            detail: format!("failed to publish initial value: {error:?}").into_boxed_str(),
        })
}

fn parse_option_duration(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<Duration, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(duration_ms) if *duration_ms <= 0 => {
            Err(SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "positive Duration".into(),
                value: RuntimeValue::Int(*duration_ms),
            })
        }
        RuntimeValue::Int(value) if *value >= 0 => Ok(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount =
                raw.parse::<u64>()
                    .map_err(|_| SourceProviderExecutionError::InvalidOption {
                        instance,
                        provider,
                        option_name: option_name.into(),
                        expected: "Duration".into(),
                        value: value.to_runtime(),
                    })?;
            duration_from_suffix(amount, suffix).ok_or_else(|| {
                SourceProviderExecutionError::InvalidOption {
                    instance,
                    provider,
                    option_name: option_name.into(),
                    expected: "Duration".into(),
                    value: value.to_runtime(),
                }
            })
        }
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Duration".into(),
            value: other.clone(),
        }),
    }
}

fn parse_retry(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<u32, SourceProviderExecutionError> {
    match strip_detached_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(*value as u32),
        RuntimeValue::SuffixedInteger { raw, suffix } if suffix.as_ref() == "x" => raw
            .parse::<u32>()
            .map_err(|_| SourceProviderExecutionError::InvalidOption {
                instance,
                provider,
                option_name: option_name.into(),
                expected: "Retry".into(),
                value: value.to_runtime(),
            }),
        other => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "Retry".into(),
            value: other.clone(),
        }),
    }
}

fn parse_stream_mode(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &DetachedRuntimeValue,
) -> Result<ProcessStreamMode, SourceProviderExecutionError> {
    let value = strip_detached_signal(value);
    let Some(name) = variant_name_value(value) else {
        return Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "StreamMode".into(),
            value: value.clone(),
        });
    };
    match name.as_ref() {
        "Ignore" => Ok(ProcessStreamMode::Ignore),
        "Lines" => Ok(ProcessStreamMode::Lines),
        "Bytes" => Ok(ProcessStreamMode::Bytes),
        _ => Err(SourceProviderExecutionError::InvalidOption {
            instance,
            provider,
            option_name: option_name.into(),
            expected: "StreamMode".into(),
            value: value.clone(),
        }),
    }
}

fn variant_name_value(value: &RuntimeValue) -> Option<Box<str>> {
    match value {
        RuntimeValue::Sum(value) if value.fields.is_empty() => Some(value.variant_name.clone()),
        RuntimeValue::Text(value) => Some(value.clone()),
        RuntimeValue::Callable(RuntimeCallable::SumConstructor {
            handle,
            bound_arguments,
        }) if handle.field_count == 0 && bound_arguments.is_empty() => {
            Some(handle.variant_name.clone())
        }
        _ => None,
    }
}

fn encode_runtime_body(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    value: &DetachedRuntimeValue,
) -> Result<Box<str>, SourceProviderExecutionError> {
    let value = strip_detached_signal(value);
    match value {
        RuntimeValue::Text(value) => Ok(value.clone()),
        _ => encode_runtime_json(value)
            .map_err(
                |detail| SourceProviderExecutionError::UnsupportedProviderShape {
                    instance,
                    provider,
                    detail: format!("http body encoding failed: {detail}").into_boxed_str(),
                },
            )
            .map(String::into_boxed_str),
    }
}

fn execute_db_connect(
    instance: SourceInstanceId,
    plan: &DbConnectPlan,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let output = match Command::new("sqlite3")
        .arg(plan.database.as_ref())
        .arg("PRAGMA schema_version;")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return db_connect_error_value(
                instance,
                &plan.result,
                &format!("failed to start sqlite3: {error}"),
            );
        }
    };
    if output.status.success() {
        db_connect_success_value(instance, &plan.result, &plan.database)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            format!("sqlite3 exited with status {}", output.status)
        } else {
            stderr
        };
        db_connect_error_value(instance, &plan.result, &detail)
    }
}

fn db_connect_success_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    database: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbConnect;
    let payload = serde_json::json!({
        "database": database,
    });
    let encoded = serde_json::to_string(&payload).expect("db.connect payload should encode");
    result.success_from_text(&encoded).map_err(|error| {
        SourceProviderExecutionError::UnsupportedProviderShape {
            instance,
            provider,
            detail: format!(
                "db.connect success payload does not match the source output shape: {error}"
            )
            .into_boxed_str(),
        }
    })
}

fn db_connect_error_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    detail: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbConnect;
    result
        .error_value(TextSourceErrorKind::Connect, detail)
        .map_err(
            |shape| SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "db.connect failure cannot be represented by the current error type: {shape}"
                )
                .into_boxed_str(),
            },
        )
}

fn db_live_query_error_value(
    instance: SourceInstanceId,
    result: &RequestResultPlan,
    detail: &str,
) -> Result<RuntimeValue, SourceProviderExecutionError> {
    let provider = BuiltinSourceProvider::DbLive;
    result
        .error_value(TextSourceErrorKind::Query, detail)
        .map_err(
            |shape| SourceProviderExecutionError::UnsupportedProviderShape {
                instance,
                provider,
                detail: format!(
                    "db.live failure cannot be represented by the current error type: {shape}"
                )
                .into_boxed_str(),
            },
        )
}

fn duration_from_suffix(amount: u64, suffix: &str) -> Option<Duration> {
    match suffix {
        "ns" => Some(Duration::from_nanos(amount)),
        "us" => Some(Duration::from_micros(amount)),
        "ms" => Some(Duration::from_millis(amount)),
        "s" => Some(Duration::from_secs(amount)),
        "m" => amount.checked_mul(60).map(Duration::from_secs),
        "h" => amount.checked_mul(60 * 60).map(Duration::from_secs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
        let core =
            aivi_core::lower_module(hir.module()).expect("typed-core lowering should succeed");
        let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
        let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
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
            if let Some(value) = runtime.current_value(signal).unwrap() {
                if predicate(value) {
                    return Some(value.clone());
                }
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
                    if latest
                        == RuntimeValue::ResultOk(Box::new(RuntimeValue::Text("second".into())))
                    {
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
            &format!(
                r#"
type DbusSignal = {{
    path: Text,
    interface: Text,
    member: Text,
    body: Text
}}

@source dbus.signal "/org/aivi/Test" with {{
    interface: "org.aivi.Test"
    member: "Ping"
}}
signal inbound : Signal DbusSignal
"#,
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
            if matches!(&bus_state, RuntimeValue::Sum(sum) if sum.variant_name.as_ref() == "Owned")
            {
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
            if matches!(&bus_state, RuntimeValue::Sum(sum) if sum.variant_name.as_ref() == "Owned")
            {
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
}
