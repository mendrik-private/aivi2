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
    /// Live dark-mode stream: publishes `Bool` on each system dark-mode change.
    DarkMode {
        provider: RuntimeSourceProvider,
        port: DetachedRuntimePublicationPort,
    },
    /// Live clipboard stream: publishes `Text` on each clipboard content change.
    Clipboard {
        provider: RuntimeSourceProvider,
        port: DetachedRuntimePublicationPort,
    },
    /// Live window-size stream: publishes a Record `{ width: Int, height: Int }` on each resize.
    WindowSize {
        provider: RuntimeSourceProvider,
        port: DetachedRuntimePublicationPort,
    },
    /// Live window-focus stream: publishes `Bool` (true = focused) on each focus change.
    WindowFocus {
        provider: RuntimeSourceProvider,
        port: DetachedRuntimePublicationPort,
    },
}

impl ActiveProviderState {
    fn provider(&self) -> &RuntimeSourceProvider {
        match self {
            Self::Passive { provider, .. }
            | Self::Mailbox { provider, .. }
            | Self::Window { provider, .. }
            | Self::DarkMode { provider, .. }
            | Self::Clipboard { provider, .. }
            | Self::WindowSize { provider, .. }
            | Self::WindowFocus { provider, .. } => provider,
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

    pub fn app_dir(&self) -> &Path {
        self.context.app_dir()
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

    pub fn collect_window_key_publications(
        &self,
        event: &WindowKeyEvent,
    ) -> Vec<crate::Publication<aivi_backend::DetachedRuntimeValue>> {
        let mut publications = Vec::new();
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
            publications.push(crate::Publication::new(
                port.stamp(),
                aivi_backend::DetachedRuntimeValue::from_runtime_owned(value),
            ));
        }
        publications
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
        WindowKeyConfig {
            capture,
            focus_only,
        }
    }

    /// Publishes a dark-mode change to all active `gtk.darkMode` source instances.
    /// Called by the GTK main thread whenever `adw::StyleManager` dark state changes.
    pub fn dispatch_dark_mode_changed(&mut self, is_dark: bool) {
        for state in self.active.values() {
            let ActiveProviderState::DarkMode { port, .. } = state else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(is_dark),
            ));
        }
    }

    /// Publishes a clipboard change to all active `clipboard.changed` source instances.
    /// Called by the GTK main thread whenever the GDK clipboard content changes.
    pub fn dispatch_clipboard_changed(&mut self, text: String) {
        for state in self.active.values() {
            let ActiveProviderState::Clipboard { port, .. } = state else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Text(text.clone().into()),
            ));
        }
    }

    /// Publishes a window size change to all active `window.size` source instances.
    pub fn dispatch_window_size_changed(&mut self, width: i32, height: i32) {
        for state in self.active.values() {
            let ActiveProviderState::WindowSize { port, .. } = state else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Record(vec![
                    aivi_backend::RuntimeRecordField {
                        label: "width".into(),
                        value: RuntimeValue::Int(width as i64),
                    },
                    aivi_backend::RuntimeRecordField {
                        label: "height".into(),
                        value: RuntimeValue::Int(height as i64),
                    },
                ]),
            ));
        }
    }

    /// Publishes a window focus change to all active `window.focus` source instances.
    pub fn dispatch_window_focus_changed(&mut self, focused: bool) {
        for state in self.active.values() {
            let ActiveProviderState::WindowFocus { port, .. } = state else {
                continue;
            };
            let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(focused),
            ));
        }
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
            RuntimeSourceProvider::Builtin(
                provider @ (BuiltinSourceProvider::ApiGet
                | BuiltinSourceProvider::ApiPost
                | BuiltinSourceProvider::ApiPut
                | BuiltinSourceProvider::ApiPatch
                | BuiltinSourceProvider::ApiDelete),
            ) => {
                let plan = ApiPlan::parse(instance, *provider, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_api_worker(port, plan, stop.clone());
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ProcessAppDir) => {
                validate_argument_count(instance, BuiltinSourceProvider::ProcessAppDir, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::ProcessAppDir, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                publish_immediate_value(
                    instance,
                    BuiltinSourceProvider::ProcessAppDir,
                    &port,
                    self.context.app_dir_runtime_value(),
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::GoaMailAccounts) => {
                let plan = GoaMailAccountsPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_goa_mail_accounts_worker(port, plan, stop.clone())?;
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
                let handle =
                    spawn_dbus_method_worker(port, plan, self.context.clone(), stop.clone())?;
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::NotificationsEvents) => {
                let plan = NotificationEventsPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_notification_events_worker(port, plan, stop.clone())?;
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::DbusEmit) => {
                let plan = DbusEmitPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_dbus_emit_worker(port, plan, stop.clone())?;
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::GtkDarkMode) => {
                validate_argument_count(instance, BuiltinSourceProvider::GtkDarkMode, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::GtkDarkMode, config)?;
                // The GTK host publishes the initial dark-mode state via `dispatch_dark_mode_changed`
                // shortly after activation (before the first tick). Subsequent changes are pushed
                // whenever `adw::StyleManager::dark` changes.
                ActiveProviderState::DarkMode {
                    provider: config.provider.clone(),
                    port,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::GtkClipboard) => {
                validate_argument_count(instance, BuiltinSourceProvider::GtkClipboard, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::GtkClipboard, config)?;
                // The GTK host publishes the initial clipboard text via `dispatch_clipboard_changed`
                // shortly after activation. Subsequent changes are pushed whenever the GDK
                // clipboard changes.
                ActiveProviderState::Clipboard {
                    provider: config.provider.clone(),
                    port,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::GtkWindowSize) => {
                validate_argument_count(instance, BuiltinSourceProvider::GtkWindowSize, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::GtkWindowSize, config)?;
                ActiveProviderState::WindowSize {
                    provider: config.provider.clone(),
                    port,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::GtkWindowFocus) => {
                validate_argument_count(instance, BuiltinSourceProvider::GtkWindowFocus, config, 0)?;
                reject_options(instance, BuiltinSourceProvider::GtkWindowFocus, config)?;
                ActiveProviderState::WindowFocus {
                    provider: config.provider.clone(),
                    port,
                }
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ImapConnect) => {
                let plan = ImapConnectPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_imap_connect_worker(port, plan, stop.clone())?;
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ImapIdle) => {
                let plan = ImapIdlePlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_imap_idle_worker(port, plan, stop.clone())?;
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
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::ImapFetchBody) => {
                let plan = ImapFetchBodyPlan::parse(instance, config)?;
                let stop = Arc::new(AtomicBool::new(false));
                let handle = spawn_imap_fetch_body_worker(port, plan, stop.clone())?;
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
            RuntimeSourceProvider::Builtin(
                BuiltinSourceProvider::SmtpSend
                | BuiltinSourceProvider::DbExec
                | BuiltinSourceProvider::TimeNowMs,
            ) => {
                // Stub: runtime implementations are pending. Publish Unit so
                // downstream signals settle at startup without panicking.
                let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Unit));
                let stop = Arc::new(AtomicBool::new(false));
                ActiveProviderState::Passive {
                    provider: config.provider.clone(),
                    stop,
                }
            }
            RuntimeSourceProvider::Custom(_) => {
                // Custom providers publish an initial Unit value so the source
                // signal is populated and downstream derivations can settle.
                let _ = port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Unit));
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
            ActiveProviderState::Window { .. } | ActiveProviderState::DarkMode { .. } | ActiveProviderState::Clipboard { .. } | ActiveProviderState::WindowSize { .. } | ActiveProviderState::WindowFocus { .. } => {}
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
