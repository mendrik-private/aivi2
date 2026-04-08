use super::*;

type MainContextTask<S> = Box<dyn FnOnce(&mut S) + Send + 'static>;

#[derive(Clone)]
pub(super) struct RunLaunchConfig {
    providers: SourceProviderManager,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct RunSessionControl {
    context: glib::MainContext,
    driver: GlibLinkedRuntimeDriver,
    request_tx: sync_mpsc::Sender<MainContextTask<RunSessionState>>,
    notifier: Arc<dyn Fn() + Send + Sync + 'static>,
}

#[allow(dead_code)]
pub(super) struct RunSessionHarness {
    view_name: Box<str>,
    session: Rc<RefCell<RunSessionState>>,
    control: RunSessionControl,
    root_windows: Vec<gtk::Window>,
    startup_manual_sources: RefCell<Option<Box<[aivi_runtime::SourceInstanceId]>>>,
}

#[allow(dead_code)]
pub(super) struct RunSessionAccess<'a> {
    session: &'a mut RunSessionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunSessionPhase {
    Starting,
    Running,
    Stopped,
}

struct MainContextRequestQueue<S> {
    request_tx: sync_mpsc::Sender<MainContextTask<S>>,
    request_rx: sync_mpsc::Receiver<MainContextTask<S>>,
}

#[derive(Debug)]
struct RunHydrationRequest {
    revision: u64,
    globals: BTreeMap<BackendItemId, DetachedRuntimeValue>,
}

#[derive(Debug)]
struct RunHydrationResponse {
    revision: u64,
    result: Result<RunHydrationPlan, String>,
}

struct RunHydrationWorker {
    request_tx: Option<sync_mpsc::Sender<RunHydrationRequest>>,
    response_rx: sync_mpsc::Receiver<RunHydrationResponse>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Default)]
struct HydrationRevisionState {
    next_revision: u64,
    latest_requested: Option<u64>,
    latest_applied: Option<u64>,
    latest_requested_globals: Option<BTreeMap<BackendItemId, DetachedRuntimeValue>>,
}

struct RunHydrationCoordinator {
    worker: RunHydrationWorker,
    revisions: HydrationRevisionState,
}

struct RunSessionLifecycle {
    phase: RunSessionPhase,
    runtime_error: Option<String>,
}

#[derive(Clone, Default)]
struct RunSessionScheduleState {
    work_scheduled: Rc<Cell<bool>>,
}

struct RunSessionState {
    view_name: Box<str>,
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
    executor: GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    driver: GlibLinkedRuntimeDriver,
    hydration: RunHydrationCoordinator,
    required_signal_globals: BTreeMap<BackendItemId, Box<str>>,
    main_context_requests: MainContextRequestQueue<RunSessionState>,
    main_loop: glib::MainLoop,
    lifecycle: RunSessionLifecycle,
}

impl Default for RunLaunchConfig {
    fn default() -> Self {
        Self::new(SourceProviderManager::new())
    }
}

impl RunLaunchConfig {
    pub(super) fn new(providers: SourceProviderManager) -> Self {
        Self { providers }
    }
}

#[allow(dead_code)]
impl RunSessionControl {
    pub(super) fn context(&self) -> glib::MainContext {
        self.context.clone()
    }

    pub(super) fn driver(&self) -> GlibLinkedRuntimeDriver {
        self.driver.clone()
    }

    pub(super) fn request_on_main_context<F>(&self, request: F) -> Result<(), String>
    where
        F: for<'a> FnOnce(&mut RunSessionAccess<'a>) + Send + 'static,
    {
        self.request_tx
            .send(Box::new(move |session: &mut RunSessionState| {
                let mut access = RunSessionAccess { session };
                request(&mut access);
            }))
            .map_err(|_| {
                "run session control is no longer accepting GTK main-context requests".to_owned()
            })?;
        self.wake();
        Ok(())
    }

    pub(super) fn request_quit(&self) -> Result<(), String> {
        self.request_on_main_context(|access| access.quit())
    }

    fn wake(&self) {
        (self.notifier)();
    }
}

#[allow(dead_code)]
impl RunSessionHarness {
    pub(super) fn control(&self) -> RunSessionControl {
        self.control.clone()
    }

    pub(super) fn with_access<R>(&self, f: impl FnOnce(&mut RunSessionAccess<'_>) -> R) -> R {
        let mut session = self.session.borrow_mut();
        let mut access = RunSessionAccess {
            session: &mut session,
        };
        f(&mut access)
    }

    pub(super) fn view_name(&self) -> &str {
        self.view_name.as_ref()
    }

    pub(super) fn root_windows(&self) -> &[gtk::Window] {
        &self.root_windows
    }

    pub(super) fn install_quit_on_last_window_close(&self) {
        let remaining = Rc::new(Cell::new(self.root_windows.len()));
        for window in &self.root_windows {
            let main_loop = self.session.borrow().main_loop.clone();
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
    }

    pub(super) fn present_root_windows(&self) -> Result<(), String> {
        for window in &self.root_windows {
            window.present();
        }
        let Some(instances) = self.startup_manual_sources.borrow_mut().take() else {
            return Ok(());
        };
        for instance in instances.iter().copied() {
            self.control
                .driver()
                .set_source_mode(instance, aivi_runtime::GlibLinkedSourceMode::Live)
                .map_err(|error| {
                    format!(
                        "failed to release startup timer source {} into live mode: {error}",
                        instance.as_raw()
                    )
                })?;
        }
        Ok(())
    }

    pub(super) fn run_main_loop(&self) -> Result<(), String> {
        if let Some(error) = self.session.borrow_mut().lifecycle.take_runtime_error() {
            return Err(error);
        }
        let main_loop = self.session.borrow().main_loop.clone();
        main_loop.run();
        let mut session = self.session.borrow_mut();
        session.lifecycle.mark_stopped();
        if let Some(error) = session.lifecycle.take_runtime_error() {
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn shutdown(&self) {
        // Stop the driver before quitting: suspends all source providers and
        // prevents any further ticks from being queued on the GLib context.
        // This ensures the context is clean for subsequent tests or sessions
        // that share the same GLib main context.
        self.with_access(|access| {
            access.driver().stop();
            access.quit();
        });
        for window in &self.root_windows {
            window.close();
        }
    }
}

#[allow(dead_code)]
impl<'a> RunSessionAccess<'a> {
    pub(super) fn view_name(&self) -> &str {
        self.session.view_name.as_ref()
    }

    pub(super) fn phase(&self) -> RunSessionPhase {
        self.session.lifecycle.phase()
    }

    pub(super) fn driver(&self) -> GlibLinkedRuntimeDriver {
        self.session.driver.clone()
    }

    pub(super) fn executor_mut(
        &mut self,
    ) -> &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue> {
        &mut self.session.executor
    }

    pub(super) fn collect_root_windows(&self) -> Result<Vec<gtk::Window>, String> {
        self.session.collect_root_windows()
    }

    pub(super) fn latest_requested_hydration(&self) -> Option<u64> {
        self.session.hydration.latest_requested()
    }

    pub(super) fn latest_applied_hydration(&self) -> Option<u64> {
        self.session.hydration.latest_applied()
    }

    pub(super) fn queued_message_count(&self) -> usize {
        self.session.driver.queued_message_count()
    }

    pub(super) fn outcome_count(&self) -> usize {
        self.session.driver.outcome_count()
    }

    pub(super) fn failure_count(&self) -> usize {
        self.session.driver.failure_count()
    }

    pub(super) fn process_pending_work(&mut self) -> Result<(), String> {
        self.session.process_pending_work()
    }

    pub(super) fn request_current_hydration(&mut self) -> Result<(), String> {
        let required_signal_globals = self.session.required_signal_globals.clone();
        self.session
            .hydration
            .request_current(&self.session.driver, &required_signal_globals)
    }

    pub(super) fn quit(&mut self) {
        self.session.lifecycle.mark_stopped();
        self.session.main_loop.quit();
    }

    pub(super) fn fail(&mut self, error: impl Into<String>) {
        self.session.fail(error.into());
    }
}

impl<S> MainContextRequestQueue<S> {
    fn new() -> Self {
        let (request_tx, request_rx) = sync_mpsc::channel();
        Self {
            request_tx,
            request_rx,
        }
    }

    fn sender(&self) -> sync_mpsc::Sender<MainContextTask<S>> {
        self.request_tx.clone()
    }

    #[cfg(test)]
    fn enqueue<F>(&self, task: F) -> Result<(), String>
    where
        F: FnOnce(&mut S) + Send + 'static,
    {
        self.request_tx
            .send(Box::new(task))
            .map_err(|_| "GTK main-context request queue has already shut down".to_owned())
    }

    fn try_pop(&self) -> Option<MainContextTask<S>> {
        match self.request_rx.try_recv() {
            Ok(task) => Some(task),
            Err(sync_mpsc::TryRecvError::Empty | sync_mpsc::TryRecvError::Disconnected) => None,
        }
    }
}

impl RunHydrationWorker {
    fn new(
        shared: Arc<RunHydrationStaticState>,
        notifier: Arc<dyn Fn() + Send + Sync + 'static>,
    ) -> Self {
        let (request_tx, request_rx) = sync_mpsc::channel();
        let (response_tx, response_rx) = sync_mpsc::channel();
        let thread = thread::spawn(move || {
            run_hydration_worker_loop(shared, request_rx, response_tx, notifier);
        });
        Self {
            request_tx: Some(request_tx),
            response_rx,
            thread: Some(thread),
        }
    }

    fn request(
        &self,
        revision: u64,
        globals: BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Result<(), String> {
        self.request_tx
            .as_ref()
            .ok_or_else(|| "run hydration worker has already shut down".to_owned())?
            .send(RunHydrationRequest { revision, globals })
            .map_err(|_| {
                "run hydration worker stopped before the request could be queued".to_owned()
            })
    }

    fn drain_ready(&self) -> Vec<RunHydrationResponse> {
        self.response_rx.try_iter().collect()
    }

    /// Like `drain_ready`, but waits briefly for a response that is expected to arrive very soon.
    /// Used immediately after `request()` when the hydration work is fast (sub-millisecond), so
    /// we can apply the result in the same `process_pending_work` cycle instead of waiting for
    /// the next polling wakeup.
    fn drain_ready_immediate(&self) -> Vec<RunHydrationResponse> {
        match self
            .response_rx
            .recv_timeout(std::time::Duration::from_micros(500))
        {
            Ok(first) => {
                let mut results = vec![first];
                results.extend(self.response_rx.try_iter());
                results
            }
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for RunHydrationWorker {
    fn drop(&mut self) {
        self.request_tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl HydrationRevisionState {
    fn next_requested_revision(&mut self) -> u64 {
        self.next_revision = self.next_revision.wrapping_add(1);
        let revision = self.next_revision;
        self.latest_requested = Some(revision);
        revision
    }

    fn should_request_globals(
        &self,
        globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> bool {
        self.latest_requested_globals.as_ref() != Some(globals)
    }

    fn mark_requested_globals(&mut self, globals: BTreeMap<BackendItemId, DetachedRuntimeValue>) {
        self.latest_requested_globals = Some(globals);
    }

    fn latest_requested(&self) -> Option<u64> {
        self.latest_requested
    }

    fn latest_applied(&self) -> Option<u64> {
        self.latest_applied
    }

    fn should_apply(&self, revision: u64) -> bool {
        self.latest_applied
            .map_or(true, |applied| revision > applied)
    }

    fn mark_applied(&mut self, revision: u64) {
        self.latest_applied = Some(revision);
    }
}

impl RunHydrationCoordinator {
    fn new(
        shared: Arc<RunHydrationStaticState>,
        notifier: Arc<dyn Fn() + Send + Sync + 'static>,
    ) -> Self {
        Self {
            worker: RunHydrationWorker::new(shared, notifier),
            revisions: HydrationRevisionState::default(),
        }
    }

    fn latest_requested(&self) -> Option<u64> {
        self.revisions.latest_requested()
    }

    fn latest_applied(&self) -> Option<u64> {
        self.revisions.latest_applied()
    }

    fn request_current(
        &mut self,
        driver: &GlibLinkedRuntimeDriver,
        required_signal_globals: &BTreeMap<BackendItemId, Box<str>>,
    ) -> Result<(), String> {
        let globals = driver
            .current_signal_globals()
            .map_err(|error| format!("{error}"))?;
        let projected = project_run_hydration_globals(required_signal_globals, &globals);
        if !run_hydration_globals_ready(required_signal_globals, &projected) {
            return Ok(());
        }
        self.request(projected)
    }

    fn request(
        &mut self,
        globals: BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Result<(), String> {
        if !self.revisions.should_request_globals(&globals) {
            return Ok(());
        }
        let revision = self.revisions.next_requested_revision();
        self.worker.request(revision, globals.clone())?;
        self.revisions.mark_requested_globals(globals);
        Ok(())
    }

    fn apply_ready(
        &mut self,
        executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    ) -> Result<(), String> {
        self.apply_from(self.worker.drain_ready(), executor)
    }

    /// Like `apply_ready`, but waits briefly for the response that was just requested.
    /// This collapses the two-cycle request→apply pipeline into one for fast hydration.
    fn apply_ready_immediate(
        &mut self,
        executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    ) -> Result<(), String> {
        self.apply_from(self.worker.drain_ready_immediate(), executor)
    }

    fn apply_from(
        &mut self,
        responses: Vec<RunHydrationResponse>,
        executor: &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    ) -> Result<(), String> {
        let latest = responses
            .into_iter()
            .filter(|response| self.revisions.should_apply(response.revision))
            .last();
        let Some(response) = latest else {
            return Ok(());
        };
        let plan = response.result?;
        apply_run_hydration_plan(&plan, executor)?;
        self.revisions.mark_applied(response.revision);
        Ok(())
    }
}

impl RunSessionLifecycle {
    fn new() -> Self {
        Self {
            phase: RunSessionPhase::Starting,
            runtime_error: None,
        }
    }

    fn phase(&self) -> RunSessionPhase {
        self.phase
    }

    fn has_runtime_error(&self) -> bool {
        self.runtime_error.is_some()
    }

    fn mark_running(&mut self) {
        if !matches!(self.phase, RunSessionPhase::Stopped) {
            self.phase = RunSessionPhase::Running;
        }
    }

    fn mark_stopped(&mut self) {
        self.phase = RunSessionPhase::Stopped;
    }

    fn take_runtime_error(&mut self) -> Option<String> {
        self.runtime_error.take()
    }

    fn fail(&mut self, error: String) {
        if self.runtime_error.is_none() {
            self.runtime_error = Some(error);
        }
        self.mark_stopped();
    }
}

impl RunSessionScheduleState {
    fn try_schedule(&self) -> bool {
        !self.work_scheduled.replace(true)
    }

    fn clear(&self) {
        self.work_scheduled.set(false);
    }
}

impl RunSessionState {
    fn fail(&mut self, error: String) {
        self.lifecycle.fail(error);
        self.main_loop.quit();
    }

    fn process_pending_work(&mut self) -> Result<(), String> {
        let queued_events = self.executor.host_mut().drain_events();
        if !queued_events.is_empty() {
            let mut sink = RunEventSink {
                driver: &self.driver,
                executor: &self.executor,
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
        let queued_window_keys = self.executor.host_mut().drain_window_key_events();
        for event in queued_window_keys {
            self.driver
                .dispatch_window_key_event(event.name.as_ref(), event.repeated);
            self.driver.tick_now();
        }
        let failures = self.driver.drain_failures();
        if !failures.is_empty() {
            let source_map = self.driver.build_source_map();
            let graph = self.driver.signal_graph();
            let mut rendered = String::from("live runtime failed during `aivi run`:\n");
            for failure in &failures {
                match failure {
                    GlibLinkedRuntimeFailure::Tick(error) => {
                        let diagnostics = render_runtime_error(error, &source_map, &graph, None);
                        for diag in &diagnostics {
                            rendered.push_str(&format!("  error: {}\n", diag.message));
                            for note in &diag.notes {
                                rendered.push_str(&format!("  note: {note}\n"));
                            }
                            for help in &diag.help {
                                rendered.push_str(&format!("  help: {help}\n"));
                            }
                        }
                    }
                    other => {
                        rendered.push_str("  ");
                        rendered.push_str(&other.to_string());
                        rendered.push('\n');
                    }
                }
            }
            return Err(rendered);
        }
        let has_outcomes = !self.driver.drain_outcomes().is_empty();
        if has_outcomes {
            let required_signal_globals = self.required_signal_globals.clone();
            self.hydration
                .request_current(&self.driver, &required_signal_globals)?;
            // Try to apply immediately: hydration is fast, so the background thread
            // typically responds within microseconds, collapsing the two-cycle pipeline.
            self.hydration.apply_ready_immediate(&mut self.executor)?;
        }
        // Always drain completed hydration responses. Hot sources like timers can keep producing
        // outcomes every cycle, and restricting apply_ready to the no-outcomes branch starves the
        // GTK tree even after the worker finishes planning a newer revision.
        self.hydration.apply_ready(&mut self.executor)?;
        self.drain_main_context_requests();
        Ok(())
    }

    fn drain_main_context_requests(&mut self) {
        while let Some(task) = self.main_context_requests.try_pop() {
            task(self);
            if matches!(self.lifecycle.phase(), RunSessionPhase::Stopped) {
                break;
            }
        }
    }

    fn collect_root_windows(&self) -> Result<Vec<gtk::Window>, String> {
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

fn run_hydration_worker_loop(
    shared: Arc<RunHydrationStaticState>,
    request_rx: sync_mpsc::Receiver<RunHydrationRequest>,
    response_tx: sync_mpsc::Sender<RunHydrationResponse>,
    notifier: Arc<dyn Fn() + Send + Sync + 'static>,
) {
    while let Ok(mut request) = request_rx.recv() {
        while let Ok(next) = request_rx.try_recv() {
            request = next;
        }
        let result = plan_run_hydration(shared.as_ref(), &request.globals);
        if response_tx
            .send(RunHydrationResponse {
                revision: request.revision,
                result,
            })
            .is_err()
        {
            break;
        }
        notifier();
    }
}

fn project_run_hydration_globals(
    required: &BTreeMap<BackendItemId, Box<str>>,
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> BTreeMap<BackendItemId, DetachedRuntimeValue> {
    required
        .keys()
        .filter_map(|item| globals.get(item).cloned().map(|value| (*item, value)))
        .collect()
}

fn hold_startup_timer_sources(
    driver: &GlibLinkedRuntimeDriver,
) -> Result<Box<[aivi_runtime::SourceInstanceId]>, String> {
    let mut instances = Vec::new();
    for binding in driver.source_bindings() {
        let config = driver
            .evaluate_source_config(binding.instance)
            .map_err(|error| {
                format!(
                    "failed to evaluate startup source {}: {error}",
                    binding.instance.as_raw()
                )
            })?;
        let is_timer = config
            .provider
            .builtin_provider()
            .is_some_and(|provider| matches!(provider.key(), "timer.every" | "timer.after"));
        if !is_timer {
            continue;
        }
        driver
            .set_source_mode(binding.instance, aivi_runtime::GlibLinkedSourceMode::Manual)
            .map_err(|error| {
                format!(
                    "failed to hold startup timer source {} in manual mode: {error}",
                    binding.instance.as_raw()
                )
            })?;
        instances.push(binding.instance);
    }
    Ok(instances.into_boxed_slice())
}

pub(super) fn start_run_session_with_launch_config(
    path: &Path,
    artifact: RunArtifact,
    launch_config: RunLaunchConfig,
) -> Result<RunSessionHarness, String> {
    gtk::init()
        .map_err(|error| format!("failed to initialize GTK for {}: {error}", path.display()))?;
    let RunArtifact {
        view_name,
        module,
        bridge,
        hydration_inputs,
        required_signal_globals,
        runtime_assembly,
        core,
        backend,
        event_handlers,
        stub_signal_defaults,
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
    let scheduled_session = Arc::new(std::sync::Mutex::new(
        None::<Arc<glib::thread_guard::ThreadGuard<Box<dyn Fn() + 'static>>>>,
    ));
    let session_notifier: Arc<dyn Fn() + Send + Sync + 'static> = {
        let context = context.clone();
        let scheduled_session = scheduled_session.clone();
        Arc::new(move || {
            let callback = scheduled_session
                .lock()
                .expect("run-session notifier state mutex should not be poisoned")
                .clone();
            let Some(callback) = callback else {
                return;
            };
            let callback = callback.clone();
            context.spawn(async move {
                (callback.get_ref())();
            });
        })
    };
    let driver = GlibLinkedRuntimeDriver::new(
        context.clone(),
        linked,
        launch_config.providers,
        Some(session_notifier.clone()),
    );

    // Pre-seed default values for stub cross-module signal imports so that hydration
    // can fire immediately on first tick instead of waiting indefinitely for signals
    // that are only computed by a companion daemon process.
    for (input_handle, default_value) in stub_signal_defaults {
        if let Ok(stamp) = driver.current_stamp(input_handle) {
            let _ = driver.queue_publication_now(Publication::new(stamp, default_value));
        }
    }

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
    let main_context_requests = MainContextRequestQueue::new();
    let control = RunSessionControl {
        context: context.clone(),
        driver: driver.clone(),
        request_tx: main_context_requests.sender(),
        notifier: session_notifier.clone(),
    };
    let startup_manual_sources = hold_startup_timer_sources(&driver)?;
    let schedule_state = RunSessionScheduleState::default();
    let session = Rc::new(RefCell::new(RunSessionState {
        view_name: view_name.clone(),
        event_handlers,
        executor,
        driver,
        hydration: RunHydrationCoordinator::new(
            Arc::new(RunHydrationStaticState {
                view_name: view_name.clone(),
                module,
                bridge,
                inputs: hydration_inputs,
            }),
            session_notifier,
        ),
        required_signal_globals,
        main_context_requests,
        main_loop: main_loop.clone(),
        lifecycle: RunSessionLifecycle::new(),
    }));
    {
        let weak_session = Rc::downgrade(&session);
        let schedule_state = schedule_state.clone();
        session
            .borrow_mut()
            .executor
            .host_mut()
            .set_event_notifier(Some(Rc::new(move || {
                if let Some(session) = weak_session.upgrade() {
                    schedule_run_session(&session, &schedule_state);
                }
            })));
    }
    {
        let weak_session = Rc::downgrade(&session);
        let schedule_state = schedule_state.clone();
        let callback: Arc<glib::thread_guard::ThreadGuard<Box<dyn Fn() + 'static>>> =
            Arc::new(glib::thread_guard::ThreadGuard::new(Box::new(move || {
                if let Some(session) = weak_session.upgrade() {
                    schedule_run_session(&session, &schedule_state);
                }
            })));
        *scheduled_session
            .lock()
            .expect("run-session notifier state mutex should not be poisoned") = Some(callback);
    }

    session.borrow().driver.tick_now();
    {
        let mut session = session.borrow_mut();
        session.process_pending_work().map_err(|error| {
            format!("failed to start run view `{}`: {error}", session.view_name)
        })?;
        if session.hydration.latest_requested().is_none() {
            let driver = session.driver.clone();
            let required_signal_globals = session.required_signal_globals.clone();
            session
                .hydration
                .request_current(&driver, &required_signal_globals)
                .map_err(|error| {
                    format!("failed to start run view `{}`: {error}", session.view_name)
                })?;
        }
    }
    while {
        let session = session.borrow();
        session.hydration.latest_applied().is_none() && !session.lifecycle.has_runtime_error()
    } {
        context.iteration(true);
    }
    {
        let mut session = session.borrow_mut();
        if let Some(error) = session.lifecycle.take_runtime_error() {
            return Err(format!(
                "failed to start run view `{}`: {error}",
                session.view_name
            ));
        }
    }
    let root_windows = session.borrow().collect_root_windows()?;
    session.borrow_mut().lifecycle.mark_running();

    Ok(RunSessionHarness {
        view_name,
        session,
        control,
        root_windows,
        startup_manual_sources: RefCell::new(Some(startup_manual_sources)),
    })
}

pub(super) fn launch_run_with_config(
    path: &Path,
    artifact: RunArtifact,
    launch_config: RunLaunchConfig,
) -> Result<ExitCode, String> {
    let harness = start_run_session_with_launch_config(path, artifact, launch_config)?;

    println!(
        "running GTK view `{}` from {}",
        harness.view_name(),
        path.display()
    );

    harness.install_quit_on_last_window_close();
    harness.present_root_windows()?;
    harness.run_main_loop()?;
    Ok(ExitCode::SUCCESS)
}

fn schedule_run_session(
    session: &Rc<RefCell<RunSessionState>>,
    schedule_state: &RunSessionScheduleState,
) {
    if !schedule_state.try_schedule() {
        return;
    }
    let weak_session = Rc::downgrade(session);
    let schedule_state = schedule_state.clone();
    glib::idle_add_local_once(move || {
        schedule_state.clear();
        let Some(session) = weak_session.upgrade() else {
            return;
        };
        let mut session = match session.try_borrow_mut() {
            Ok(session) => session,
            Err(_) => {
                schedule_run_session(&session, &schedule_state);
                return;
            }
        };
        if session.lifecycle.has_runtime_error()
            || matches!(session.lifecycle.phase(), RunSessionPhase::Stopped)
        {
            return;
        }
        if let Err(error) = session.process_pending_work() {
            session.fail(error);
        }
    });
}

struct RunEventSink<'a> {
    driver: &'a GlibLinkedRuntimeDriver,
    executor: &'a GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
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
        let payload = match handler.payload {
            ResolvedRunEventPayload::GtkPayload => value.0,
            ResolvedRunEventPayload::ScopedInput => self
                .executor
                .input_value_for_instance(&route.instance, route.binding.input)
                .map(|value| value.0)
                .ok_or_else(|| {
                    format!(
                        "missing scoped event payload input {} for route {} on {}",
                        route.binding.input.as_raw(),
                        route.id,
                        route.instance
                    )
                })?,
        };
        let stamp = self
            .driver
            .current_stamp(handler.signal_input)
            .map_err(|error| format!("{error}"))?;
        self.driver
            .queue_publication_now_current_queue(Publication::new(stamp, payload))
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

#[cfg(test)]
mod tests {
    use super::{
        HydrationRevisionState, MainContextRequestQueue, RunLaunchConfig, RunSessionLifecycle,
        RunSessionPhase, RunSessionScheduleState, project_run_hydration_globals,
        start_run_session_with_launch_config,
    };
    use crate::{RunHydrationStaticState, plan_run_hydration_profiled};
    use aivi_backend::{DetachedRuntimeValue, ItemId as BackendItemId, RuntimeValue};
    use aivi_base::SourceDatabase;
    use aivi_hir::{ValidationMode, lower_module as lower_hir_module};
    use aivi_syntax::parse_module;
    use gtk::prelude::*;
    use std::{
        collections::BTreeMap,
        env,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    fn repo_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn prepare_run_from_path(path: &Path) -> crate::RunArtifact {
        let snapshot = crate::WorkspaceHirSnapshot::load(path)
            .expect("workspace snapshot should load for run-session test");
        assert!(
            !crate::workspace_syntax_failed(&snapshot, |_, diagnostics| diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)),
            "run-session test fixture should parse cleanly"
        );
        let (hir_failed, validation_failed) = crate::workspace_hir_failed(
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
        assert!(!hir_failed, "run-session test fixture should lower cleanly");
        assert!(
            !validation_failed,
            "run-session test fixture should validate cleanly"
        );
        let lowered = snapshot.entry_hir();
        crate::prepare_run_artifact(&snapshot.sources, lowered.module(), &[], None)
            .expect("run-session test fixture should prepare")
    }

    fn prepare_run_from_text(path: &str, source: &str) -> crate::RunArtifact {
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
        crate::prepare_run_artifact(&sources, lowered.module(), &[], None)
            .expect("run-session text fixture should prepare")
    }

    fn pump_context(context: &gtk::glib::MainContext, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        while context.pending() {
            context.iteration(false);
        }
    }

    fn pump_until(
        context: &gtk::glib::MainContext,
        timeout: Duration,
        mut predicate: impl FnMut() -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        while context.pending() {
            context.iteration(false);
        }
        predicate()
    }

    fn required_signal_item(artifact: &crate::RunArtifact, name: &str) -> aivi_backend::ItemId {
        artifact
            .required_signal_globals
            .iter()
            .find_map(|(item, current)| (current.as_ref() == name).then_some(*item))
            .unwrap_or_else(|| panic!("snake demo should expose `{name}` for hydration"))
    }

    fn text_signal_for(
        harness: &super::RunSessionHarness,
        signal_item: aivi_backend::ItemId,
    ) -> String {
        harness.with_access(|access| {
            let globals = access
                .driver()
                .current_signal_globals()
                .expect("signal globals should be readable");
            let value = globals
                .get(&signal_item)
                .expect("required text signal should exist")
                .as_runtime();
            match value {
                RuntimeValue::Text(text) => text.to_string(),
                RuntimeValue::Signal(inner) => match inner.as_ref() {
                    RuntimeValue::Text(text) => text.to_string(),
                    other => panic!("expected text signal payload to be text, found {other:?}"),
                },
                other => panic!("expected text signal to be text, found {other:?}"),
            }
        })
    }

    fn debug_signal_value_for(harness: &super::RunSessionHarness, name: &str) -> String {
        harness.with_access(|access| {
            let driver = access.driver();
            let graph = driver.signal_graph();
            let Some((handle, _)) = graph.signals().find(|(_, spec)| spec.name() == name) else {
                return format!("<missing:{name}>");
            };
            match driver.current_signal_value(handle) {
                Ok(Some(value)) => format!("{value:?}"),
                Ok(None) => "<none>".to_owned(),
                Err(error) => format!("<error:{error}>"),
            }
        })
    }

    fn board_text_for(
        harness: &super::RunSessionHarness,
        board_item: aivi_backend::ItemId,
    ) -> String {
        text_signal_for(harness, board_item)
    }

    fn head_x(board_text: &str) -> usize {
        let row = board_text
            .lines()
            .find(|row| row.contains('@'))
            .expect("board text should contain a snake head");
        row.chars()
            .position(|ch| ch == '@')
            .expect("board row should expose the snake head column")
    }

    fn head_y(board_text: &str) -> usize {
        board_text
            .lines()
            .enumerate()
            .find_map(|(index, row)| row.contains('@').then_some(index))
            .expect("board text should contain a snake head row")
    }

    fn collect_label_texts(widget: &gtk::Widget, labels: &mut Vec<String>) {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
            labels.push(label.label().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect_label_texts(&current, labels);
            child = current.next_sibling();
        }
    }

    fn gtk_board_text_for(harness: &super::RunSessionHarness) -> String {
        let mut labels = Vec::new();
        for window in harness.root_windows() {
            collect_label_texts(&window.clone().upcast::<gtk::Widget>(), &mut labels);
        }
        labels
            .into_iter()
            .find(|text| text.contains('@') && text.contains('\n'))
            .expect("snake window should expose a board label")
    }

    fn find_button_by_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            if button.label().as_deref() == Some(label) {
                return Some(button);
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find_button_by_label(&current, label) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    fn count_buttons_by_label(widget: &gtk::Widget, label: &str) -> usize {
        let own_count = widget
            .clone()
            .downcast::<gtk::Button>()
            .ok()
            .and_then(|button| (button.label().as_deref() == Some(label)).then_some(1))
            .unwrap_or(0);
        let mut child = widget.first_child();
        let mut child_count = 0;
        while let Some(current) = child {
            child_count += count_buttons_by_label(&current, label);
            child = current.next_sibling();
        }
        own_count + child_count
    }

    fn button_label_count_for(harness: &super::RunSessionHarness, label: &str) -> usize {
        harness
            .root_windows()
            .iter()
            .map(|window| count_buttons_by_label(&window.clone().upcast::<gtk::Widget>(), label))
            .sum()
    }

    #[test]
    fn main_context_request_queue_preserves_submission_order() {
        let queue = MainContextRequestQueue::new();
        queue
            .enqueue(|state: &mut Vec<&'static str>| state.push("first"))
            .expect("first request should queue");
        queue
            .enqueue(|state: &mut Vec<&'static str>| state.push("second"))
            .expect("second request should queue");

        let mut state = Vec::new();
        while let Some(task) = queue.try_pop() {
            task(&mut state);
        }

        assert_eq!(state, vec!["first", "second"]);
    }

    #[test]
    fn hydration_revision_state_tracks_latest_requested_and_applied_revisions() {
        let mut revisions = HydrationRevisionState::default();

        let first = revisions.next_requested_revision();
        let second = revisions.next_requested_revision();

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(revisions.latest_requested(), Some(2));
        assert!(revisions.should_apply(first));
        assert!(revisions.should_apply(second));

        revisions.mark_applied(first);
        assert_eq!(revisions.latest_applied(), Some(first));
        assert!(!revisions.should_apply(first));
        assert!(revisions.should_apply(second));

        revisions.mark_applied(second);
        assert_eq!(revisions.latest_applied(), Some(second));
        assert!(!revisions.should_apply(first));
        assert!(!revisions.should_apply(second));
    }

    #[test]
    fn hydration_revision_state_skips_duplicate_requested_globals() {
        let mut revisions = HydrationRevisionState::default();
        let mut first_globals = BTreeMap::new();
        first_globals.insert(
            BackendItemId::from_raw(1),
            DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(7)),
        );

        assert!(revisions.should_request_globals(&first_globals));
        let first = revisions.next_requested_revision();
        revisions.mark_requested_globals(first_globals.clone());

        assert_eq!(first, 1);
        assert!(!revisions.should_request_globals(&first_globals));

        let mut second_globals = first_globals.clone();
        second_globals.insert(
            BackendItemId::from_raw(2),
            DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(9)),
        );
        assert!(revisions.should_request_globals(&second_globals));
    }

    #[test]
    fn project_run_hydration_globals_keeps_only_required_items() {
        let required = BTreeMap::from([
            (BackendItemId::from_raw(1), "alpha".into()),
            (BackendItemId::from_raw(3), "gamma".into()),
        ]);
        let globals = BTreeMap::from([
            (
                BackendItemId::from_raw(1),
                DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(7)),
            ),
            (
                BackendItemId::from_raw(2),
                DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(8)),
            ),
            (
                BackendItemId::from_raw(3),
                DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(9)),
            ),
        ]);

        let projected = project_run_hydration_globals(&required, &globals);

        assert_eq!(
            projected,
            BTreeMap::from([
                (
                    BackendItemId::from_raw(1),
                    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(7)),
                ),
                (
                    BackendItemId::from_raw(3),
                    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(9)),
                ),
            ])
        );
    }

    #[test]
    fn session_lifecycle_keeps_the_first_runtime_error() {
        let mut lifecycle = RunSessionLifecycle::new();

        lifecycle.fail("first".to_owned());
        lifecycle.fail("second".to_owned());

        assert_eq!(lifecycle.phase(), RunSessionPhase::Stopped);
        assert_eq!(lifecycle.take_runtime_error().as_deref(), Some("first"));
    }

    #[test]
    fn schedule_state_coalesces_until_cleared() {
        let state = RunSessionScheduleState::default();

        assert!(state.try_schedule());
        assert!(!state.try_schedule());

        state.clear();

        assert!(state.try_schedule());
    }

    #[test]
    fn startup_manual_sources_take_once() {
        let sources = std::cell::RefCell::new(Some(
            vec![
                aivi_runtime::SourceInstanceId::from_raw(1),
                aivi_runtime::SourceInstanceId::from_raw(2),
            ]
            .into_boxed_slice(),
        ));

        assert_eq!(
            sources
                .borrow_mut()
                .take()
                .as_deref()
                .map(|items: &[aivi_runtime::SourceInstanceId]| items.len()),
            Some(2_usize)
        );
        assert!(sources.borrow_mut().take().is_none());
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn timer_sources_stay_paused_until_windows_present() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = artifact
            .required_signal_globals
            .iter()
            .find_map(|(item, name)| (name.as_ref() == "boardText").then_some(*item))
            .expect("snake demo should expose boardText for hydration");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a paused run session");
        let context = harness.control().context();
        let initial_board = board_text_for(&harness, board_item);
        let initial_head_x = head_x(&initial_board);
        let initial_hydration = harness.with_access(|access| access.latest_applied_hydration());
        assert_eq!(
            initial_head_x, 6,
            "shifted snake demo should start with runway"
        );

        pump_context(&context, Duration::from_millis(250));
        assert_eq!(
            board_text_for(&harness, board_item),
            initial_board,
            "timer-backed board should stay on the initial frame before windows are presented"
        );
        assert_eq!(
            harness.with_access(|access| access.latest_applied_hydration()),
            initial_hydration,
            "startup gating should avoid extra hydrations before presentation"
        );

        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        pump_context(&context, Duration::from_millis(650));
        let advanced_board = board_text_for(&harness, board_item);
        assert!(
            head_x(&advanced_board) > initial_head_x,
            "board should start advancing after presentation releases the startup-held timer source"
        );
        assert!(
            harness.with_access(|access| access.latest_applied_hydration()) > initial_hydration,
            "hydration should advance after timer release"
        );

        harness.shutdown();
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn main_loop_run_advances_timer_driven_board_after_presentation() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = artifact
            .required_signal_globals
            .iter()
            .find_map(|(item, name)| (name.as_ref() == "boardText").then_some(*item))
            .expect("snake demo should expose boardText for hydration");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_board = board_text_for(&harness, board_item);
        let initial_head_x = head_x(&initial_board);
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        let main_loop = harness.session.borrow().main_loop.clone();
        let quit_loop = main_loop.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(650), move || {
            quit_loop.quit();
        });
        main_loop.run();
        let advanced_board = board_text_for(&harness, board_item);
        assert!(
            head_x(&advanced_board) > initial_head_x,
            "the plain run-session main loop should advance the snake after presentation"
        );

        harness.shutdown();
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn main_loop_run_hydrates_board_label_after_timer_ticks() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_board = gtk_board_text_for(&harness);
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        let main_loop = harness.session.borrow().main_loop.clone();
        let quit_loop = main_loop.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(650), move || {
            quit_loop.quit();
        });
        main_loop.run();
        let advanced_board = gtk_board_text_for(&harness);
        assert_ne!(
            advanced_board, initial_board,
            "plain aivi run should hydrate the GTK board label after timer ticks"
        );

        harness.shutdown();
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn harness_run_main_loop_advances_timer_driven_board_without_borrow_panics() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardText");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_board = board_text_for(&harness, board_item);
        let initial_head_x = head_x(&initial_board);
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        let control = harness.control();
        gtk::glib::timeout_add_local_once(Duration::from_millis(650), move || {
            control
                .request_quit()
                .expect("test quit request should enqueue onto the GTK main context");
        });
        harness
            .run_main_loop()
            .expect("plain aivi run should not panic while the session updates itself");
        let advanced_board = board_text_for(&harness, board_item);
        assert!(
            head_x(&advanced_board) > initial_head_x,
            "the real run_main_loop path should keep advancing the snake while the GTK main loop runs"
        );

        harness.shutdown();
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn process_pending_work_applies_queued_window_key_events_immediately() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let direction_item = required_signal_item(&artifact, "dirLine");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        assert_eq!(text_signal_for(&harness, direction_item), "Right");

        harness.with_access(|access| {
            access
                .executor_mut()
                .host_mut()
                .queue_window_key_event("ArrowUp", false);
            access
                .process_pending_work()
                .expect("queued window key should process without waiting for another turn");
        });
        assert_eq!(
            text_signal_for(&harness, direction_item),
            "Up",
            "queued window key events should update the direction signal in the same run-session work cycle"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn button_click_event_payloads_use_current_markup_bindings() {
        let artifact = prepare_run_from_text(
            "event-hook-payload-run.aivi",
            r#"
signal selected : Signal Text
signal selectedText : Signal Text = selected
 +|> "None" keepLatest

type Text -> Text -> Text
func keepLatest = next current=>    next

value rows = ["Alpha", "Beta"]

value main =
    <Window title="Host">
        <Box orientation="vertical">
            <Label text={selectedText} />
            <each of={rows} as={item} key={item}>
                <Button label={item} onClick={selected item} />
            </each>
        </Box>
    </Window>

export main
"#,
        );
        let selected_item = required_signal_item(&artifact, "selectedText");
        let harness = start_run_session_with_launch_config(
            Path::new("event-hook-payload-run.aivi"),
            artifact,
            RunLaunchConfig::default(),
        )
        .expect("payload event handler fixture should start a run session");
        let context = harness.control().context();
        let beta = harness
            .root_windows()
            .iter()
            .find_map(|window| {
                find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Beta")
            })
            .expect("fixture should render a Beta button");
        assert_eq!(text_signal_for(&harness, selected_item), "None");

        beta.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, selected_item) == "Beta"
            }),
            "payload event hooks should publish the clicked row binding into the selected signal"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_run_session_exposes_human_opening_move() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        let status_item = required_signal_item(&artifact, "statusText");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        harness
            .present_root_windows()
            .expect("presenting the reversi window should release startup-held timers");
        assert_eq!(
            text_signal_for(&harness, status_item),
            "You are 🔴",
            "reversi should start on the human turn"
        );

        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose at least one legal opening move");
        assert!(
            opening_move.is_sensitive(),
            "opening move button should be clickable"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_stays_clickable_after_idling_on_human_turn() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        let last_move_item = required_signal_item(&artifact, "lastMoveText");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the reversi window should release startup-held timers");
        let initial_hydration = harness.with_access(|access| access.latest_applied_hydration());
        pump_context(&context, Duration::from_millis(650));
        assert_eq!(
            harness.with_access(|access| access.latest_applied_hydration()),
            initial_hydration,
            "reversi should stay idle on the human turn until the user clicks"
        );

        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should still expose a legal opening move after idling");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, last_move_item) != "Opening position"
            }),
            "an idle reversi session should still accept the first human move"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_human_moves_paint_red_stones_promptly() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the reversi window should release startup-held timers");

        let opening_red_count = button_label_count_for(&harness, "🔴");
        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "the first human move should paint its red stones without waiting for the AI turn"
        );
        assert!(
            pump_until(&context, Duration::from_secs(1), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌")
                        .is_some_and(|button| button.is_sensitive())
                })
            }),
            "after the AI reply the GTK tree should expose a clickable human move"
        );

        let second_move = harness
            .root_windows()
            .iter()
            .find_map(|window| {
                find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌")
                    .filter(|button| button.is_sensitive())
            })
            .expect("reversi should expose another clickable human move after the AI reply");
        let second_turn_red_count = button_label_count_for(&harness, "🔴");
        second_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                button_label_count_for(&harness, "🔴") > second_turn_red_count
            }),
            "the second human move should also paint its red stones right away"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_stays_playable_after_the_first_full_turn() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        let status_item = required_signal_item(&artifact, "statusText");
        let last_move_item = required_signal_item(&artifact, "lastMoveText");
        let preview_item = required_signal_item(&artifact, "previewText");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the reversi window should release startup-held timers");
        let opening_red_count = button_label_count_for(&harness, "🔴");

        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "clicking a legal move should put the new red stone on the board right away"
        );
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, status_item) == "Computer is choosing..."
                    && text_signal_for(&harness, preview_item).starts_with("Computer is eyeing")
            }),
            "the first move should promptly hand control to the AI and expose its planned target"
        );
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "○").is_some()
                })
            }),
            "the board should surface the AI preview while the human animation is still running"
        );
        assert!(
            !pump_until(&context, Duration::from_millis(200), || {
                text_signal_for(&harness, status_item) == "Your turn"
            }),
            "the AI reply should wait for the human animation window instead of landing immediately"
        );
        assert!(
            pump_until(&context, Duration::from_millis(800), || {
                text_signal_for(&harness, status_item) == "Your turn"
                    && text_signal_for(&harness, last_move_item).starts_with("Computer plays")
            }),
            "after the AI reply the game should quickly return to a playable human turn (status: {}, preview: {}, last move: {}, action: {}, session: {}, history: {})",
            text_signal_for(&harness, status_item),
            text_signal_for(&harness, preview_item),
            text_signal_for(&harness, last_move_item),
            debug_signal_value_for(&harness, "action"),
            debug_signal_value_for(&harness, "session"),
            debug_signal_value_for(&harness, "history"),
        );

        assert!(
            pump_until(&context, Duration::from_secs(1), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌")
                        .is_some_and(|button| button.is_sensitive())
                })
            }),
            "after the AI reply the GTK tree should expose a clickable human move"
        );
        let second_move = harness
            .root_windows()
            .iter()
            .find_map(|window| {
                find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌")
                    .filter(|button| button.is_sensitive())
            })
            .expect("reversi should expose another clickable human move after the AI reply");
        let second_turn_red_count = button_label_count_for(&harness, "🔴");
        second_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                button_label_count_for(&harness, "🔴") > second_turn_red_count
            }),
            "the second human move should paint its red stone right away"
        );
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, last_move_item).starts_with("You plays")
            }),
            "the second human move should update the move summary instead of crashing (status: {}, preview: {}, last move: {}, action: {}, session: {}, history: {})",
            text_signal_for(&harness, status_item),
            text_signal_for(&harness, preview_item),
            text_signal_for(&harness, last_move_item),
            debug_signal_value_for(&harness, "action"),
            debug_signal_value_for(&harness, "session"),
            debug_signal_value_for(&harness, "history"),
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_profiled_hydration_reports_fragment_and_kernel_activity() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        let shared = RunHydrationStaticState {
            view_name: artifact.view_name.clone(),
            module: artifact.module.clone(),
            bridge: artifact.bridge.clone(),
            inputs: artifact.hydration_inputs.clone(),
        };
        let status_item = required_signal_item(&artifact, "statusText");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the reversi window should release startup-held timers");

        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, status_item) == "Computer is choosing..."
            }),
            "opening move should quickly reach the AI-thinking state before profiling hydration"
        );

        let globals = harness.with_access(|access| {
            access
                .driver()
                .current_signal_globals()
                .expect("signal globals should be readable for hydration profiling")
        });
        let (_plan, profile) = plan_run_hydration_profiled(&shared, &globals)
            .expect("reversi hydration should be profileable from live runtime globals");

        let total_kernel_calls = profile
            .program_profiles
            .values()
            .map(|program| {
                program
                    .kernels
                    .values()
                    .map(|entry| entry.calls)
                    .sum::<u64>()
            })
            .sum::<u64>();
        let total_item_calls = profile
            .program_profiles
            .values()
            .map(|program| program.items.values().map(|entry| entry.calls).sum::<u64>())
            .sum::<u64>();

        assert!(profile.planned_nodes > 0);
        assert!(profile.evaluated_inputs > 0);
        assert!(!profile.fragment_profiles.is_empty());
        assert!(!profile.program_profiles.is_empty());
        assert!(
            total_kernel_calls > 0 || total_item_calls > 0,
            "profile should capture kernel or item activity for live Reversi hydration"
        );

        harness.shutdown();
    }

    #[gtk::test]
    #[ignore = "known pre-existing failure: recurrence signal kernel missing in snake demo backend"]
    fn space_restarts_snake_after_game_over() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardText");
        let status_item = required_signal_item(&artifact, "statusLine");
        let direction_item = required_signal_item(&artifact, "dirLine");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");

        let driver = harness.control().driver();
        driver.dispatch_window_key_event("ArrowUp", false);
        assert!(
            pump_until(&context, Duration::from_secs(1), || {
                text_signal_for(&harness, direction_item) == "Up"
            }),
            "dispatching ArrowUp should update the snake direction before waiting for a collision"
        );
        pump_context(&context, Duration::from_secs(3));
        assert_eq!(
            text_signal_for(&harness, status_item),
            "Game Over",
            "steering upward should eventually collide with the wall and end the game"
        );
        let game_over_board = board_text_for(&harness, board_item);
        assert_eq!(text_signal_for(&harness, direction_item), "Up");

        driver.dispatch_window_key_event("Space", false);
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                text_signal_for(&harness, status_item) == "Running"
                    && text_signal_for(&harness, direction_item) == "Right"
            }),
            "pressing Space should immediately reset the event-driven snake"
        );
        let restarted_board = board_text_for(&harness, board_item);
        assert_eq!(
            head_y(&restarted_board),
            10,
            "restart should return the snake to its starting row"
        );
        assert!(
            matches!(head_x(&restarted_board), 6 | 7),
            "restart should return the snake to its starting lane before or just after the first timer step"
        );
        assert_ne!(
            restarted_board, game_over_board,
            "restart should replace the game-over board with a fresh starting board"
        );

        harness.shutdown();
    }
}
