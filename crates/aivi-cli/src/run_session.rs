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
        self.with_access(|access| access.quit());
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

    fn latest_requested(&self) -> Option<u64> {
        self.latest_requested
    }

    fn latest_applied(&self) -> Option<u64> {
        self.latest_applied
    }

    fn should_apply(&self, revision: u64) -> bool {
        self.latest_requested
            .map_or(true, |requested| revision >= requested)
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
        if !run_hydration_globals_ready(required_signal_globals, &globals) {
            return Ok(());
        }
        self.request(globals)
    }

    fn request(
        &mut self,
        globals: BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Result<(), String> {
        let revision = self.revisions.next_requested_revision();
        self.worker.request(revision, globals)
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
            let mut rendered = String::from("live runtime failed during `aivi run`:\n");
            for failure in failures {
                rendered.push_str("- ");
                rendered.push_str(&failure.to_string());
                rendered.push('\n');
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
        } else {
            // No new outcomes this cycle — apply any response that arrived since last time.
            self.hydration.apply_ready(&mut self.executor)?;
        }
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
        ..
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
    let (update_tx, update_rx) = sync_mpsc::channel::<()>();
    let session_notifier: Arc<dyn Fn() + Send + Sync + 'static> = {
        let update_tx = update_tx.clone();
        Arc::new(move || {
            let _ = update_tx.send(());
        })
    };
    let driver = GlibLinkedRuntimeDriver::new(
        context.clone(),
        linked,
        launch_config.providers,
        Some(session_notifier.clone()),
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
        glib::timeout_add_local(std::time::Duration::from_millis(1), move || {
            let Some(session) = weak_session.upgrade() else {
                return glib::ControlFlow::Break;
            };
            loop {
                match update_rx.try_recv() {
                    Ok(()) => schedule_run_session(&session, &schedule_state),
                    Err(sync_mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(sync_mpsc::TryRecvError::Disconnected) => {
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::{
        HydrationRevisionState, MainContextRequestQueue, RunLaunchConfig, RunSessionLifecycle,
        RunSessionPhase, RunSessionScheduleState, start_run_session_with_launch_config,
    };
    use aivi_backend::RuntimeValue;
    use gtk::prelude::*;
    use std::{
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
        crate::prepare_run_artifact(&snapshot.sources, lowered.module(), None)
            .expect("run-session test fixture should prepare")
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
        row.find('@')
            .expect("board row should expose the snake head column")
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
        assert!(!revisions.should_apply(first));
        assert!(revisions.should_apply(second));

        revisions.mark_applied(second);
        assert_eq!(revisions.latest_applied(), Some(second));
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

        pump_context(&context, Duration::from_millis(650));
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
            pump_until(&context, Duration::from_secs(3), || {
                text_signal_for(&harness, status_item) == "Game Over"
            }),
            "steering upward should eventually collide with the wall and end the game"
        );
        let game_over_board = board_text_for(&harness, board_item);
        assert_eq!(text_signal_for(&harness, direction_item), "Up");

        driver.dispatch_window_key_event("Space", false);
        assert!(
            pump_until(&context, Duration::from_secs(2), || {
                text_signal_for(&harness, status_item) == "Running"
            }),
            "pressing Space after game over should restart the snake on a live timer tick"
        );
        let restarted_board = board_text_for(&harness, board_item);
        assert_eq!(text_signal_for(&harness, direction_item), "Right");
        assert_eq!(
            head_x(&restarted_board),
            6,
            "restart should return the snake to its initial starting lane"
        );
        assert_ne!(
            restarted_board, game_over_board,
            "restart should replace the game-over board with a fresh starting board"
        );

        harness.shutdown();
    }
}
