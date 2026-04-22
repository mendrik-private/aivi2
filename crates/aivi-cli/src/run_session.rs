use super::*;
type MainContextTask<S> = Box<dyn FnOnce(&mut S) + Send + 'static>;

#[derive(Clone)]
pub(super) struct RunLaunchConfig {
    providers: SourceProviderManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunStartupStage {
    GtkInit,
    RuntimeLink,
    SessionSetup,
    InitialRuntimeTick,
    InitialHydrationWait,
    RootWindowCollection,
}

impl RunStartupStage {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::GtkInit => "GTK init",
            Self::RuntimeLink => "runtime link",
            Self::SessionSetup => "session setup",
            Self::InitialRuntimeTick => "initial runtime tick",
            Self::InitialHydrationWait => "initial hydration wait",
            Self::RootWindowCollection => "root window collect",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RunStartupMetrics {
    pub gtk_init: Duration,
    pub runtime_link: Duration,
    pub session_setup: Duration,
    pub initial_runtime_tick: Duration,
    pub initial_hydration_wait: Duration,
    pub root_window_collection: Duration,
    pub window_presentation: Duration,
    pub total_to_session_ready: Duration,
}

impl RunStartupMetrics {
    fn with_stage_duration(
        mut self,
        stage: RunStartupStage,
        duration: Duration,
        total_to_session_ready: Duration,
    ) -> Self {
        match stage {
            RunStartupStage::GtkInit => self.gtk_init = duration,
            RunStartupStage::RuntimeLink => self.runtime_link = duration,
            RunStartupStage::SessionSetup => self.session_setup = duration,
            RunStartupStage::InitialRuntimeTick => self.initial_runtime_tick = duration,
            RunStartupStage::InitialHydrationWait => self.initial_hydration_wait = duration,
            RunStartupStage::RootWindowCollection => self.root_window_collection = duration,
        }
        self.total_to_session_ready = total_to_session_ready;
        self
    }

    pub(super) fn stage_duration(self, stage: RunStartupStage) -> Duration {
        match stage {
            RunStartupStage::GtkInit => self.gtk_init,
            RunStartupStage::RuntimeLink => self.runtime_link,
            RunStartupStage::SessionSetup => self.session_setup,
            RunStartupStage::InitialRuntimeTick => self.initial_runtime_tick,
            RunStartupStage::InitialHydrationWait => self.initial_hydration_wait,
            RunStartupStage::RootWindowCollection => self.root_window_collection,
        }
    }

    pub(super) fn with_window_presentation(mut self, duration: Duration) -> Self {
        self.window_presentation = duration;
        self
    }

    pub(super) fn total_to_first_present(self) -> Duration {
        self.total_to_session_ready + self.window_presentation
    }
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
    startup_metrics: RunStartupMetrics,
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

fn render_run_error_report(
    title: &str,
    context_lines: &[(&str, &str)],
    details: &str,
    footer: &str,
) -> String {
    render_run_error_report_with_color(
        title,
        context_lines,
        details,
        footer,
        progress_color_enabled(),
    )
}

fn render_run_error_report_with_color(
    title: &str,
    context_lines: &[(&str, &str)],
    details: &str,
    footer: &str,
    color_enabled: bool,
) -> String {
    let mut rendered = String::new();
    rendered.push_str(&format!(
        "{} {} {} {}\n",
        progress_paint_rgb(color_enabled, (255, 85, 85), "╭─"),
        progress_paint_rgb(color_enabled, (189, 147, 249), "aivi run"),
        progress_paint_dim(color_enabled, "•"),
        progress_paint_rgb(color_enabled, (255, 184, 108), title),
    ));
    let label_width = context_lines
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        .max(6);
    for (label, value) in context_lines
        .iter()
        .copied()
        .filter(|(_, value)| !value.is_empty())
    {
        rendered.push_str(&format!(
            "{} {} {}\n",
            progress_paint_dim(color_enabled, "│"),
            progress_paint_rgb(
                color_enabled,
                (139, 233, 253),
                &format!("{label:<label_width$}"),
            ),
            progress_paint_dim(color_enabled, value),
        ));
    }
    rendered.push_str(&format!(
        "{} {}\n",
        progress_paint_dim(color_enabled, "├─"),
        progress_paint_rgb(color_enabled, (102, 217, 239), "details"),
    ));
    for line in details.trim_end().lines() {
        if line.is_empty() {
            rendered.push_str(&format!("{}\n", progress_paint_dim(color_enabled, "│")));
        } else {
            rendered.push_str(&format!(
                "{} {}\n",
                progress_paint_dim(color_enabled, "│"),
                line
            ));
        }
    }
    rendered.push_str(&format!(
        "{} {}\n",
        progress_paint_dim(color_enabled, "╰─"),
        progress_paint_rgb(color_enabled, (80, 250, 123), footer),
    ));
    rendered
}

fn strip_ansi_for_run_report_detection(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        stripped.push(ch);
    }
    stripped
}

fn is_run_error_report(text: &str) -> bool {
    strip_ansi_for_run_report_detection(text).starts_with("╭─ aivi run • ")
}

fn render_backend_runtime_link_error(
    error: &aivi_runtime::BackendRuntimeLinkError,
    module: Option<&HirModule>,
    backend: &BackendProgram,
) -> String {
    let hir_item_name = |item: HirItemId| {
        module
            .and_then(|module| module.items().get(item))
            .map(|item| match item {
                Item::Type(item) => item.name.text(),
                Item::Value(item) => item.name.text(),
                Item::Function(item) => item.name.text(),
                Item::Signal(item) => item.name.text(),
                Item::Class(item) => item.name.text(),
                Item::Domain(item) => item.name.text(),
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => "<anonymous>",
            })
            .unwrap_or("<unknown>")
    };
    match error {
        aivi_runtime::BackendRuntimeLinkError::DuplicateBackendOrigin {
            item,
            first,
            second,
        } => {
            format!(
                "HIR item {} ({}) lowered to multiple backend items: item{} ({}) and item{} ({})",
                item,
                hir_item_name(*item),
                first,
                backend.item_name(*first),
                second,
                backend.item_name(*second)
            )
        }
        aivi_runtime::BackendRuntimeLinkError::MissingBackendItem { item } => {
            format!(
                "HIR runtime item {item} ({}) has no linked backend item",
                hir_item_name(*item)
            )
        }
        aivi_runtime::BackendRuntimeLinkError::BackendItemNotSignal { item, backend_item } => {
            format!(
                "HIR signal {} ({}) lowered to non-signal backend item item{} ({})",
                item,
                hir_item_name(*item),
                backend_item,
                backend.item_name(*backend_item)
            )
        }
        aivi_runtime::BackendRuntimeLinkError::MissingSignalBody { item, backend_item } => {
            format!(
                "linked derived signal {} ({}) has no backend body kernel on item{} ({})",
                item,
                hir_item_name(*item),
                backend_item,
                backend.item_name(*backend_item)
            )
        }
        aivi_runtime::BackendRuntimeLinkError::MissingItemBodyForGlobal { owner, item } => {
            format!(
                "owner {} ({}) references non-signal global item{} ({}) without a backend body kernel",
                owner,
                hir_item_name(*owner),
                item,
                backend.item_name(*item)
            )
        }
        _ => error.to_string(),
    }
}

#[derive(Clone, Default)]
struct RunSessionScheduleState {
    work_scheduled: Rc<Cell<bool>>,
    rerun_requested: Rc<Cell<bool>>,
}

struct RunSessionState {
    path: PathBuf,
    view_name: Box<str>,
    kind: RunSessionKind,
    driver: GlibLinkedRuntimeDriver,
    sources: Option<aivi_base::SourceDatabase>,
    required_signal_globals: BTreeMap<BackendItemId, Box<str>>,
    main_context_requests: MainContextRequestQueue<RunSessionState>,
    main_loop: glib::MainLoop,
    lifecycle: RunSessionLifecycle,
}

struct RunGtkSessionState {
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
    executor: GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue>,
    hydration: RunHydrationCoordinator,
    startup_gate: RunStartupGate,
}

enum RunSessionKind {
    Gtk(Box<RunGtkSessionState>),
    HeadlessTask,
}

struct RunStartupGate {
    manual_sources: Option<Box<[aivi_runtime::SourceInstanceId]>>,
    roots_presented: bool,
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

    pub(super) fn cache_home(&self) -> Result<PathBuf, Box<str>> {
        self.providers.cache_home()
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

    pub(super) fn startup_metrics(&self) -> RunStartupMetrics {
        self.startup_metrics
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
        let driver = self.control.driver();
        let mut session = self.session.borrow_mut();
        if let RunSessionKind::Gtk(state) = &mut session.kind {
            let initial_hydration_applied = state.hydration.latest_applied().is_some();
            state
                .startup_gate
                .mark_roots_presented(&driver, initial_hydration_applied)
                .map_err(|error| {
                    session.render_error_report(
                        "startup failed",
                        &error,
                        "fix the presentation issue above and rerun `aivi run`.",
                    )
                })?;
            session.process_pending_work().map_err(|error| {
                session.render_error_report(
                    "startup failed",
                    &error,
                    "fix the presentation issue above and rerun `aivi run`.",
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

    pub(super) fn runtime_error(&self) -> Option<&str> {
        self.session.lifecycle.runtime_error()
    }

    pub(super) fn driver(&self) -> GlibLinkedRuntimeDriver {
        self.session.driver.clone()
    }

    pub(super) fn executor_mut(
        &mut self,
    ) -> &mut GtkRuntimeExecutor<GtkConcreteHost<RunHostValue>, RunHostValue> {
        match &mut self.session.kind {
            RunSessionKind::Gtk(state) => &mut state.executor,
            RunSessionKind::HeadlessTask => {
                panic!("headless run sessions do not expose a GTK executor")
            }
        }
    }

    pub(super) fn collect_root_windows(&self) -> Result<Vec<gtk::Window>, String> {
        self.session.collect_root_windows()
    }

    pub(super) fn latest_requested_hydration(&self) -> Option<u64> {
        match &self.session.kind {
            RunSessionKind::Gtk(state) => state.hydration.latest_requested(),
            RunSessionKind::HeadlessTask => None,
        }
    }

    pub(super) fn latest_applied_hydration(&self) -> Option<u64> {
        match &self.session.kind {
            RunSessionKind::Gtk(state) => state.hydration.latest_applied(),
            RunSessionKind::HeadlessTask => None,
        }
    }

    pub(super) fn queued_message_count(&self) -> usize {
        self.session.driver.queued_message_count()
    }

    pub(super) fn has_pending_gtk_events(&self) -> bool {
        match &self.session.kind {
            RunSessionKind::Gtk(state) => state.executor.host().has_pending_events(),
            RunSessionKind::HeadlessTask => false,
        }
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
        match &mut self.session.kind {
            RunSessionKind::Gtk(state) => state
                .hydration
                .request_current(&self.session.driver, &required_signal_globals),
            RunSessionKind::HeadlessTask => {
                Err("headless run sessions do not use GTK hydration".to_owned())
            }
        }
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
        self.request_rx.try_recv().ok()
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
        self.latest_applied.is_none_or(|applied| revision > applied)
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
            .rfind(|response| self.revisions.should_apply(response.revision));
        let Some(response) = latest else {
            return Ok(());
        };
        let plan = response.result?;
        apply_run_hydration_plan(&plan, executor)?;
        self.revisions.mark_applied(response.revision);
        Ok(())
    }
}

impl RunStartupGate {
    fn new(manual_sources: Option<Box<[aivi_runtime::SourceInstanceId]>>) -> Self {
        Self {
            manual_sources,
            roots_presented: false,
        }
    }

    fn mark_roots_presented(
        &mut self,
        driver: &GlibLinkedRuntimeDriver,
        initial_hydration_applied: bool,
    ) -> Result<(), String> {
        self.roots_presented = true;
        self.release_if_ready(driver, initial_hydration_applied)
    }

    fn release_if_ready(
        &mut self,
        driver: &GlibLinkedRuntimeDriver,
        initial_hydration_applied: bool,
    ) -> Result<(), String> {
        if !self.roots_presented || !initial_hydration_applied {
            return Ok(());
        }
        let Some(instances) = self.manual_sources.take() else {
            return Ok(());
        };
        for instance in instances.iter().copied() {
            driver
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

    fn runtime_error(&self) -> Option<&str> {
        self.runtime_error.as_deref()
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
        if self.work_scheduled.replace(true) {
            self.rerun_requested.set(true);
            false
        } else {
            true
        }
    }

    fn finish_cycle(&self) -> bool {
        if self.rerun_requested.replace(false) {
            true
        } else {
            self.work_scheduled.set(false);
            false
        }
    }

    fn clear(&self) {
        self.work_scheduled.set(false);
        self.rerun_requested.set(false);
    }
}

impl RunSessionState {
    fn fail(&mut self, error: String) {
        let rendered = self.render_error_report(
            "runtime crash",
            &error,
            "fix the runtime issue above and rerun `aivi run`.",
        );
        self.lifecycle.fail(rendered);
        self.main_loop.quit();
    }

    fn process_pending_work(&mut self) -> Result<(), String> {
        match &mut self.kind {
            RunSessionKind::Gtk(state) => {
                let queued_events = state.executor.host_mut().drain_events();
                if !queued_events.is_empty() {
                    let mut sink = RunEventSink {
                        driver: &self.driver,
                        executor: &state.executor,
                        handlers: &state.event_handlers,
                    };
                    for event in queued_events {
                        state
                            .executor
                            .dispatch_event(event.route, event.value, &mut sink)
                            .map_err(|error| {
                                format!("failed to dispatch GTK event {}: {error}", event.route)
                            })?;
                    }
                }
                let queued_window_keys = state.executor.host_mut().drain_window_key_events();
                for event in queued_window_keys {
                    for publication in self
                        .driver
                        .collect_window_key_publications(event.name.as_ref(), event.repeated)
                    {
                        self.driver
                            .queue_publication_now_isolated_budgeted(publication)
                            .map_err(|error| {
                                format!("failed to queue window key publication: {error}")
                            })?;
                    }
                }
                let dark_mode_events = state.executor.host_mut().drain_dark_mode_events();
                for is_dark in dark_mode_events {
                    self.driver.dispatch_dark_mode_changed(is_dark);
                }
                let clipboard_events = state.executor.host_mut().drain_clipboard_events();
                for text in clipboard_events {
                    self.driver.dispatch_clipboard_changed(text);
                }
                let window_size_events = state.executor.host_mut().drain_window_size_events();
                for (width, height) in window_size_events {
                    self.driver.dispatch_window_size_changed(width, height);
                }
                let window_focus_events = state.executor.host_mut().drain_window_focus_events();
                for focused in window_focus_events {
                    self.driver.dispatch_window_focus_changed(focused);
                }
                let failures = self.driver.drain_failures();
                if !failures.is_empty() {
                    return Err(render_run_runtime_failures(
                        &self.path,
                        self.view_name.as_ref(),
                        &self.driver,
                        self.sources.as_ref(),
                        &failures,
                    ));
                }
                self.driver.drain_outcomes();
                if state.startup_gate.roots_presented {
                    let required_signal_globals = self.required_signal_globals.clone();
                    let latest_requested = state.hydration.latest_requested();
                    state
                        .hydration
                        .request_current(&self.driver, &required_signal_globals)?;
                    if state.hydration.latest_requested() != latest_requested {
                        state.hydration.apply_ready_immediate(&mut state.executor)?;
                    }
                    state.hydration.apply_ready(&mut state.executor)?;
                    state.startup_gate.release_if_ready(
                        &self.driver,
                        state.hydration.latest_applied().is_some(),
                    )?;
                }
            }
            RunSessionKind::HeadlessTask => {
                let failures = self.driver.drain_failures();
                if !failures.is_empty() {
                    return Err(render_run_runtime_failures(
                        &self.path,
                        self.view_name.as_ref(),
                        &self.driver,
                        self.sources.as_ref(),
                        &failures,
                    ));
                }
                self.driver.drain_outcomes();
            }
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
        let RunSessionKind::Gtk(state) = &self.kind else {
            return Ok(Vec::new());
        };
        let root_handles = state.executor.root_widgets().map_err(|error| {
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
                let widget = state.executor.host().widget(&handle).ok_or_else(|| {
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

    fn render_error_report(&self, title: &str, details: &str, footer: &str) -> String {
        if is_run_error_report(details) {
            return details.to_owned();
        }
        let path_display = self.path.display().to_string();
        render_run_error_report(
            title,
            &[
                ("path", path_display.as_str()),
                ("view", self.view_name.as_ref()),
            ],
            details,
            footer,
        )
    }
}

fn render_run_runtime_failures(
    path: &Path,
    view_name: &str,
    driver: &GlibLinkedRuntimeDriver,
    sources: Option<&aivi_base::SourceDatabase>,
    failures: &[GlibLinkedRuntimeFailure],
) -> String {
    let source_map = driver.build_source_map();
    let graph = driver.signal_graph();
    let backend = driver.backend_program();
    let fallback_sources;
    let sources = if let Some(sources) = sources {
        sources
    } else {
        fallback_sources = aivi_base::SourceDatabase::new();
        &fallback_sources
    };
    let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
    let mut rendered = String::new();
    for failure in failures {
        match failure {
            GlibLinkedRuntimeFailure::Tick(error) => {
                let diagnostics = aivi_runtime::render_runtime_error(
                    error,
                    &source_map,
                    &graph,
                    backend.as_deref(),
                );
                rendered.push_str(&renderer.render_all(diagnostics.iter(), sources));
                rendered.push('\n');
            }
            GlibLinkedRuntimeFailure::ProviderExecution(error) => {
                let diagnostics = aivi_runtime::render_provider_execution_error(error, &source_map);
                rendered.push_str(&renderer.render_all(diagnostics.iter(), sources));
                rendered.push('\n');
            }
        }
    }
    let path_display = path.display().to_string();
    render_run_error_report(
        "runtime crash",
        &[("path", path_display.as_str()), ("view", view_name)],
        rendered.trim_end(),
        "fix the runtime issue above and rerun `aivi run`.",
    )
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
        let is_timer = driver
            .source_provider(binding.instance)
            .and_then(|provider| provider.builtin_provider())
            .is_some_and(|provider| matches!(provider.key(), "timer.every" | "timer.after"));
        if !is_timer {
            continue;
        }
        driver
            .evaluate_source_config(binding.instance)
            .map_err(|error| {
                format!(
                    "failed to evaluate startup source {}: {error}",
                    binding.instance.as_raw()
                )
            })?;
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

fn record_startup_stage<F>(
    startup_metrics: &mut RunStartupMetrics,
    stage: RunStartupStage,
    duration: Duration,
    total_to_session_ready: Duration,
    on_stage_completed: &mut F,
) where
    F: FnMut(RunStartupStage, &RunStartupMetrics),
{
    *startup_metrics = startup_metrics.with_stage_duration(stage, duration, total_to_session_ready);
    on_stage_completed(stage, startup_metrics);
}

pub(super) fn start_run_session_with_launch_config(
    path: &Path,
    artifact: RunArtifact,
    launch_config: RunLaunchConfig,
) -> Result<RunSessionHarness, String> {
    start_run_session_with_launch_config_and_reporter(path, artifact, launch_config, |_, _| {})
}

fn start_run_session_with_launch_config_and_reporter<F>(
    path: &Path,
    artifact: RunArtifact,
    mut launch_config: RunLaunchConfig,
    mut on_stage_completed: F,
) -> Result<RunSessionHarness, String>
where
    F: FnMut(RunStartupStage, &RunStartupMetrics),
{
    let startup_started = Instant::now();
    let mut startup_metrics = RunStartupMetrics::default();
    let RunArtifact {
        view_name,
        kind,
        required_signal_globals,
        sources,
        runtime_assembly,
        runtime_link,
        runtime_tables,
        backend,
        backend_native_kernels,
        stub_signal_defaults,
    } = artifact;
    let path_display = path.display().to_string();
    let view_name_display = view_name.to_string();
    if let Some(diagnostic_sources) = sources.clone() {
        let diagnostic_source_map =
            aivi_runtime::RuntimeSourceMap::from_assembly(&runtime_assembly);
        let error_path = path_display.clone();
        let error_view = view_name_display.clone();
        launch_config
            .providers
            .set_decode_diagnostic_reporter(Arc::new(move |instance, provider, error| {
                let diagnostics = aivi_runtime::render_source_decode_error(
                    instance,
                    provider,
                    &error,
                    &diagnostic_source_map,
                );
                let renderer = aivi_base::DiagnosticRenderer::new(aivi_base::ColorMode::Auto);
                eprintln!(
                    "{}",
                    render_run_error_report(
                        "source decode failed",
                        &[
                            ("path", error_path.as_str()),
                            ("view", error_view.as_str()),
                            ("source", provider.key()),
                        ],
                        &renderer.render_all(diagnostics.iter(), &diagnostic_sources),
                        &format!(
                            "update source `{}` (instance {}) and rerun `aivi run`.",
                            provider.key(),
                            instance.as_raw()
                        ),
                    )
                );
            }));
    }
    if matches!(kind, RunArtifactKind::Gtk(_)) {
        let gtk_init_started = Instant::now();
        gtk::init().map_err(|error| {
            render_run_error_report(
                "startup failed",
                &[
                    ("path", path_display.as_str()),
                    ("view", view_name_display.as_str()),
                ],
                &format!("failed to initialize GTK for {}: {error}", path.display()),
                "ensure GTK is available, then rerun `aivi run`.",
            )
        })?;
        let gtk_init = gtk_init_started.elapsed();
        record_startup_stage(
            &mut startup_metrics,
            RunStartupStage::GtkInit,
            gtk_init,
            startup_started.elapsed(),
            &mut on_stage_completed,
        );
    }
    let runtime_link_started = Instant::now();
    let linked = if let Some(runtime_tables) = runtime_tables {
        aivi_runtime::link_backend_runtime_with_tables_and_native_kernels_from_payload(
            runtime_assembly,
            backend.clone(),
            backend_native_kernels.clone(),
            runtime_tables,
        )
        .map_err(|errors| {
            let mut rendered = String::new();
            for error in errors.errors() {
                rendered.push_str("- ");
                if let Some(program) = backend.as_program() {
                    rendered.push_str(&render_backend_runtime_link_error(
                        error,
                        None,
                        program.as_ref(),
                    ));
                } else {
                    rendered.push_str(&error.to_string());
                }
                rendered.push('\n');
            }
            render_run_error_report(
                "startup failed",
                &[
                    ("path", path_display.as_str()),
                    ("view", view_name_display.as_str()),
                ],
                &format!(
                    "failed to instantiate frozen backend runtime for `aivi build` output:\n{}",
                    rendered.trim_end()
                ),
                "rebuild the bundle or fix the runtime mismatch, then rerun.",
            )
        })?
    } else {
        aivi_runtime::link_backend_runtime_with_seed_and_native_kernels_from_payload(
            runtime_assembly,
            backend.clone(),
            backend_native_kernels.clone(),
            &runtime_link,
        )
        .map_err(|errors| {
            let mut rendered = String::new();
            for error in errors.errors() {
                rendered.push_str("- ");
                if let Some(program) = backend.as_program() {
                    rendered.push_str(&render_backend_runtime_link_error(
                        error,
                        None,
                        program.as_ref(),
                    ));
                } else {
                    rendered.push_str(&error.to_string());
                }
                rendered.push('\n');
            }
            render_run_error_report(
                "startup failed",
                &[
                    ("path", path_display.as_str()),
                    ("view", view_name_display.as_str()),
                ],
                &format!(
                    "failed to link backend runtime for `aivi run`:\n{}",
                    rendered.trim_end()
                ),
                "fix the runtime-link errors above and rerun `aivi run`.",
            )
        })?
    };
    let runtime_link = runtime_link_started.elapsed();
    record_startup_stage(
        &mut startup_metrics,
        RunStartupStage::RuntimeLink,
        runtime_link,
        startup_started.elapsed(),
        &mut on_stage_completed,
    );

    let session_setup_started = Instant::now();
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
            context.invoke(move || (callback.get_ref())());
            context.wakeup();
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
    let main_context_requests = MainContextRequestQueue::new();
    let control = RunSessionControl {
        context: context.clone(),
        driver: driver.clone(),
        request_tx: main_context_requests.sender(),
        notifier: session_notifier.clone(),
    };
    let session_kind = match kind {
        RunArtifactKind::Gtk(surface) => {
            let executor = GtkRuntimeExecutor::new(
                surface.bridge.clone(),
                GtkConcreteHost::<RunHostValue>::default(),
            )
            .map_err(|error| {
                render_run_error_report(
                    "startup failed",
                    &[
                        ("path", path_display.as_str()),
                        ("view", view_name_display.as_str()),
                    ],
                    &format!(
                        "failed to mount GTK view `{}` from {}: {error}",
                        view_name,
                        path.display()
                    ),
                    "fix the GTK markup/runtime issue above and rerun `aivi run`.",
                )
            })?;
            let startup_manual_sources = hold_startup_timer_sources(&driver)?;
            RunSessionKind::Gtk(Box::new(RunGtkSessionState {
                event_handlers: surface.event_handlers,
                executor,
                hydration: RunHydrationCoordinator::new(
                    Arc::new(RunHydrationStaticState {
                        view_name: view_name.clone(),
                        patterns: surface.patterns,
                        bridge: surface.bridge,
                        inputs: surface.hydration_inputs,
                        runtime_execution: Arc::new(RunFragmentExecutionUnit::new(
                            backend.clone(),
                            backend_native_kernels.clone(),
                        )),
                    }),
                    session_notifier.clone(),
                ),
                startup_gate: RunStartupGate::new(Some(startup_manual_sources)),
            }))
        }
        RunArtifactKind::HeadlessTask { .. } => RunSessionKind::HeadlessTask,
    };
    let schedule_state = RunSessionScheduleState::default();
    let session = Rc::new(RefCell::new(RunSessionState {
        path: path.to_path_buf(),
        view_name: view_name.clone(),
        kind: session_kind,
        driver,
        sources,
        required_signal_globals,
        main_context_requests,
        main_loop: main_loop.clone(),
        lifecycle: RunSessionLifecycle::new(),
    }));
    {
        let mut borrowed = session.borrow_mut();
        if let RunSessionKind::Gtk(state) = &mut borrowed.kind {
            let weak_session = Rc::downgrade(&session);
            let schedule_state = schedule_state.clone();
            state
                .executor
                .host_mut()
                .set_event_notifier(Some(Rc::new(move || {
                    let Some(session) = weak_session.upgrade() else {
                        return;
                    };
                    let mut borrowed = match session.try_borrow_mut() {
                        Ok(session) => session,
                        Err(_) => {
                            schedule_run_session(&session, &schedule_state);
                            return;
                        }
                    };
                    if borrowed.lifecycle.has_runtime_error()
                        || matches!(borrowed.lifecycle.phase(), RunSessionPhase::Stopped)
                    {
                        return;
                    }
                    if let Err(error) = borrowed.process_pending_work() {
                        borrowed.fail(error);
                    }
                })));
        }
    }
    {
        let weak_session = Rc::downgrade(&session);
        let schedule_state = schedule_state.clone();
        let callback: Arc<glib::thread_guard::ThreadGuard<Box<dyn Fn() + 'static>>> =
            Arc::new(glib::thread_guard::ThreadGuard::new(Box::new(move || {
                let Some(session) = weak_session.upgrade() else {
                    return;
                };
                let mut borrowed = match session.try_borrow_mut() {
                    Ok(session) => session,
                    Err(_) => {
                        schedule_run_session(&session, &schedule_state);
                        return;
                    }
                };
                if borrowed.lifecycle.has_runtime_error()
                    || matches!(borrowed.lifecycle.phase(), RunSessionPhase::Stopped)
                {
                    return;
                }
                if let Err(error) = borrowed.process_pending_work() {
                    borrowed.fail(error);
                    return;
                }
                let should_rerun = matches!(
                    &borrowed.kind,
                    RunSessionKind::Gtk(state)
                        if state.hydration.latest_applied() != state.hydration.latest_requested()
                );
                if should_rerun {
                    drop(borrowed);
                    schedule_run_session(&session, &schedule_state);
                }
            })));
        *scheduled_session
            .lock()
            .expect("run-session notifier state mutex should not be poisoned") = Some(callback);
    }
    let session_setup = session_setup_started.elapsed();
    record_startup_stage(
        &mut startup_metrics,
        RunStartupStage::SessionSetup,
        session_setup,
        startup_started.elapsed(),
        &mut on_stage_completed,
    );

    let initial_runtime_tick_started = Instant::now();
    session.borrow().driver.tick_now();
    {
        let mut session = session.borrow_mut();
        session.process_pending_work().map_err(|error| {
            session.render_error_report(
                "startup failed",
                &error,
                "fix the startup issue above and rerun `aivi run`.",
            )
        })?;
    }
    let initial_runtime_tick = initial_runtime_tick_started.elapsed();
    record_startup_stage(
        &mut startup_metrics,
        RunStartupStage::InitialRuntimeTick,
        initial_runtime_tick,
        startup_started.elapsed(),
        &mut on_stage_completed,
    );
    if matches!(&session.borrow().kind, RunSessionKind::Gtk(_)) {
        record_startup_stage(
            &mut startup_metrics,
            RunStartupStage::InitialHydrationWait,
            Duration::ZERO,
            startup_started.elapsed(),
            &mut on_stage_completed,
        );
    }
    {
        let mut session = session.borrow_mut();
        if let Some(error) = session.lifecycle.take_runtime_error() {
            return Err(error);
        }
    }
    let root_windows = if matches!(&session.borrow().kind, RunSessionKind::Gtk(_)) {
        let root_window_collection_started = Instant::now();
        let root_windows = session.borrow().collect_root_windows().map_err(|error| {
            render_run_error_report(
                "startup failed",
                &[
                    ("path", path_display.as_str()),
                    ("view", view_name_display.as_str()),
                ],
                &error,
                "fix the root-window issue above and rerun `aivi run`.",
            )
        })?;
        let root_window_collection = root_window_collection_started.elapsed();
        record_startup_stage(
            &mut startup_metrics,
            RunStartupStage::RootWindowCollection,
            root_window_collection,
            startup_started.elapsed(),
            &mut on_stage_completed,
        );
        root_windows
    } else {
        Vec::new()
    };
    session.borrow_mut().lifecycle.mark_running();

    Ok(RunSessionHarness {
        view_name,
        session,
        control,
        root_windows,
        startup_metrics,
    })
}

pub(super) fn launch_run_with_config<P, F>(
    path: &Path,
    artifact: RunArtifact,
    launch_config: RunLaunchConfig,
    mut on_progress: P,
    on_started: F,
) -> Result<ExitCode, String>
where
    P: FnMut(RunStartupStage, &RunStartupMetrics),
    F: FnOnce(&RunStartupMetrics),
{
    let harness = start_run_session_with_launch_config_and_reporter(
        path,
        artifact,
        launch_config,
        &mut on_progress,
    )?;

    let startup_metrics = if harness.root_windows().is_empty() {
        println!(
            "running headless entry `{}` from {}",
            harness.view_name(),
            path.display()
        );
        harness.startup_metrics()
    } else {
        println!(
            "running GTK view `{}` from {}",
            harness.view_name(),
            path.display()
        );
        harness.install_quit_on_last_window_close();
        let present_started = Instant::now();
        harness.present_root_windows()?;
        harness
            .startup_metrics()
            .with_window_presentation(present_started.elapsed())
    };
    on_started(&startup_metrics);
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
    spawn_run_session_callback(weak_session, schedule_state);
}

fn spawn_run_session_callback(
    weak_session: std::rc::Weak<RefCell<RunSessionState>>,
    schedule_state: RunSessionScheduleState,
) {
    glib::MainContext::default().spawn_local(async move {
        let Some(session) = weak_session.upgrade() else {
            schedule_state.clear();
            return;
        };
        let mut session = match session.try_borrow_mut() {
            Ok(session) => session,
            Err(_) => {
                spawn_run_session_callback(weak_session, schedule_state);
                return;
            }
        };
        if session.lifecycle.has_runtime_error()
            || matches!(session.lifecycle.phase(), RunSessionPhase::Stopped)
        {
            schedule_state.clear();
            return;
        }
        if let Err(error) = session.process_pending_work() {
            session.fail(error);
        }
        drop(session);
        if schedule_state.finish_cycle() {
            spawn_run_session_callback(weak_session, schedule_state);
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
        HydrationRevisionState, MainContextRequestQueue, RunFragmentExecutionUnit, RunLaunchConfig,
        RunSessionHarness, RunSessionLifecycle, RunSessionPhase, RunSessionScheduleState,
        RunStartupGate, RunStartupStage, is_run_error_report, project_run_hydration_globals,
        render_run_error_report_with_color, start_run_session_with_launch_config,
        start_run_session_with_launch_config_and_reporter,
    };
    use crate::{RunHydrationStaticState, plan_run_hydration_profiled};
    use aivi_backend::{DetachedRuntimeValue, ItemId as BackendItemId, RuntimeValue};
    use aivi_base::SourceDatabase;
    use aivi_hir::{ValidationMode, lower_module as lower_hir_module};
    use aivi_runtime::set_native_kernel_plans_enabled;
    use aivi_syntax::parse_module;
    use gtk::prelude::*;
    use std::{
        collections::BTreeMap,
        env,
        path::{Path, PathBuf},
        sync::{Arc, Once},
        time::{Duration, Instant},
    };

    fn ensure_interpreted_run_session_tests() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| set_native_kernel_plans_enabled(false));
    }

    fn repo_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn prepare_run_from_path(path: &Path) -> crate::RunArtifact {
        ensure_interpreted_run_session_tests();
        let snapshot = crate::WorkspaceHirSnapshot::load(path)
            .expect("workspace snapshot should load for run-session test");
        let parsed = snapshot.entry_parsed();
        assert!(
            !parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.severity == aivi_base::Severity::Error),
            "run-session test fixture should parse cleanly: {:?}",
            parsed
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
                .map(|diagnostic| diagnostic.render(&snapshot.sources))
                .collect::<Vec<_>>()
        );
        let lowered = snapshot.entry_hir();
        let hir_diagnostics = lowered
            .hir_diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            .map(|diagnostic| diagnostic.render(&snapshot.sources))
            .collect::<Vec<_>>();
        let validation_mode = if hir_diagnostics.is_empty() {
            ValidationMode::RequireResolvedNames
        } else {
            ValidationMode::Structural
        };
        let validation_diagnostics = lowered
            .module()
            .validate(validation_mode)
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity == aivi_base::Severity::Error)
            .map(|diagnostic| diagnostic.render(&snapshot.sources))
            .collect::<Vec<_>>();
        assert!(
            hir_diagnostics.is_empty(),
            "run-session test fixture should lower cleanly: {hir_diagnostics:?}"
        );
        assert!(
            validation_diagnostics.is_empty(),
            "run-session test fixture should validate cleanly: {validation_diagnostics:?}"
        );
        crate::prepare_run_artifact(&snapshot.sources, lowered.module(), &[], None)
            .expect("run-session test fixture should prepare")
    }

    fn prepare_run_from_text(path: &str, source: &str) -> crate::RunArtifact {
        ensure_interpreted_run_session_tests();
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

    fn prepare_reversi_run() -> (PathBuf, crate::RunArtifact) {
        let path = repo_path("demos/reversi.aivi");
        let artifact = prepare_run_from_path(&path);
        (path, artifact)
    }

    fn reversi_source_with_initial_board(initial_board: &str) -> String {
        let original = include_str!("../../../demos/reversi.aivi");
        let replaced = original.replacen(
            r#"func buildInitialDisc = x y => (x, y)
 ||> (3, 3) -> White
 ||> (4, 4) -> White
 ||> (3, 4) -> Black
 ||> (4, 3) -> Black
 ||> _      -> Empty"#,
            initial_board,
            1,
        );
        assert_ne!(
            replaced, original,
            "reversi fixture should replace the opening board"
        );
        replaced
    }

    fn near_endgame_reversi_source() -> String {
        reversi_source_with_initial_board(
            r#"func buildInitialDisc = x y => (x, y)
 ||> (7, 0) -> Empty
 ||> (6, 0) -> White
 ||> _      -> Black"#,
        )
    }

    fn computer_final_reversi_source() -> String {
        reversi_source_with_initial_board(
            r#"func buildInitialDisc = x y => (x, y)
 ||> (0, 0) -> White
 ||> (1, 0) -> Empty
 ||> (2, 0) -> White
 ||> (4, 0) -> Empty
 ||> _      -> Black"#,
        )
    }

    fn pass_chain_terminal_reversi_source() -> String {
        reversi_source_with_initial_board(
            r#"func buildInitialDisc = x y => (x, y)
 ||> (1, 0) -> White
 ||> (3, 0) -> White
 ||> (4, 0) -> Empty
 ||> (5, 0) -> Empty
 ||> (7, 0) -> Empty
 ||> _      -> Black"#,
        )
    }

    fn assert_reversi_restart_resets_after_terminal_fixture(
        source: String,
        fixture_name: &str,
        terminal_timeout: Duration,
    ) {
        let workspace = tempfile::tempdir().expect("reversi fixture workspace should create");
        let fixture_path = workspace.path().join("main.aivi");
        std::fs::write(&fixture_path, source).expect("reversi fixture should write");
        let artifact = prepare_run_from_path(&fixture_path);
        let harness = start_run_session_with_launch_config(
            &fixture_path,
            artifact,
            RunLaunchConfig::default(),
        )
        .expect("reversi fixture should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));

        let opening_state = debug_signal_value_for(&harness, "state");
        let opening_move = find_sensitive_button_by_label(&harness, "◌")
            .expect("reversi fixture should expose the initial legal human move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, terminal_timeout, || {
                debug_signal_value_for(&harness, "phase").contains("HumanReady")
                    && !has_sensitive_button_by_label(&harness, "◌")
                    && debug_signal_value_for(&harness, "state") != opening_state
            }),
            "{fixture_name} should settle into a terminal state after the final move sequence (phase: {}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            debug_signal_value_for(&harness, "state"),
        );
        let restart = harness
            .root_windows()
            .iter()
            .find_map(|window| {
                find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Restart")
            })
            .expect("reversi window should expose a restart button");
        assert!(
            restart.is_visible() && restart.is_sensitive(),
            "restart should stay visible and clickable after {fixture_name} reaches game over"
        );
        assert!(
            restart.allocated_width() > 0 && restart.allocated_height() > 0,
            "restart should keep a non-zero GTK allocation after {fixture_name} reaches game over"
        );
        restart.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                debug_signal_value_for(&harness, "state") == opening_state
                    && has_sensitive_button_by_label(&harness, "◌")
            }),
            "restart should restore the opening state after {fixture_name} reaches game over (phase: {}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            debug_signal_value_for(&harness, "state"),
        );

        harness.shutdown();
    }

    fn pump_context(context: &gtk::glib::MainContext, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            let slice =
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now()));
            if slice.is_zero() {
                break;
            }
            gtk::glib::timeout_add_local_once(slice, || {});
            context.iteration(true);
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
            let slice =
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now()));
            if slice.is_zero() {
                break;
            }
            gtk::glib::timeout_add_local_once(slice, || {});
            context.iteration(true);
        }
        while context.pending() {
            context.iteration(false);
        }
        predicate()
    }

    fn present_root_windows_and_wait_for_hydration(harness: &RunSessionHarness, timeout: Duration) {
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the run-session window should start initial hydration");
        assert!(
            pump_until(&context, timeout, || {
                harness
                    .with_access(|access| access.latest_applied_hydration())
                    .is_some()
            }),
            "run-session initial hydration should apply after window presentation"
        );
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

    fn named_signal_value_for(harness: &super::RunSessionHarness, name: &str) -> RuntimeValue {
        harness.with_access(|access| {
            let driver = access.driver();
            let graph = driver.signal_graph();
            let Some((handle, _)) = graph.signals().find(|(_, spec)| spec.name() == name) else {
                panic!("expected live runtime signal `{name}`");
            };
            driver
                .current_signal_value(handle)
                .expect("signal value should be readable")
                .unwrap_or_else(|| panic!("signal `{name}` should have a current value"))
                .into_runtime()
        })
    }

    fn runtime_signal_payload(value: &RuntimeValue) -> &RuntimeValue {
        match value {
            RuntimeValue::Signal(inner) => inner.as_ref(),
            other => other,
        }
    }

    fn runtime_record_fields<'a>(
        value: &'a RuntimeValue,
        context: &str,
    ) -> &'a [aivi_backend::RuntimeRecordField] {
        match runtime_signal_payload(value) {
            RuntimeValue::Record(fields) => fields,
            other => panic!("expected {context} to be a record, found {other:?}"),
        }
    }

    fn runtime_record_field<'a>(
        fields: &'a [aivi_backend::RuntimeRecordField],
        label: &str,
        context: &str,
    ) -> &'a RuntimeValue {
        fields
            .iter()
            .find_map(|field| (field.label.as_ref() == label).then_some(&field.value))
            .unwrap_or_else(|| panic!("expected {context} to include field `{label}`"))
    }

    fn runtime_int(value: &RuntimeValue, context: &str) -> i64 {
        match runtime_signal_payload(value) {
            RuntimeValue::Int(value) => *value,
            other => panic!("expected {context} to be an Int, found {other:?}"),
        }
    }

    fn runtime_text(value: &RuntimeValue, context: &str) -> String {
        match runtime_signal_payload(value) {
            RuntimeValue::Text(value) => value.to_string(),
            other => panic!("expected {context} to be Text, found {other:?}"),
        }
    }

    fn runtime_sum_variant(value: &RuntimeValue, context: &str) -> String {
        match runtime_signal_payload(value) {
            RuntimeValue::Sum(value) => value.variant_name.to_string(),
            other => panic!("expected {context} to be a sum value, found {other:?}"),
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct SnakeRenderTile {
        column: i64,
        row: i64,
        asset: String,
    }

    fn board_tiles_for(
        harness: &super::RunSessionHarness,
        board_item: aivi_backend::ItemId,
    ) -> Vec<SnakeRenderTile> {
        harness.with_access(|access| {
            let globals = access
                .driver()
                .current_signal_globals()
                .expect("signal globals should be readable");
            let value = globals
                .get(&board_item)
                .expect("required board tile signal should exist")
                .as_runtime();
            let RuntimeValue::List(items) = runtime_signal_payload(value) else {
                panic!("expected board tile signal to be a List, found {value:?}");
            };
            items
                .iter()
                .map(|tile| {
                    let fields = runtime_record_fields(tile, "snake render tile");
                    SnakeRenderTile {
                        column: runtime_int(
                            runtime_record_field(fields, "column", "snake render tile"),
                            "snake render tile column",
                        ),
                        row: runtime_int(
                            runtime_record_field(fields, "row", "snake render tile"),
                            "snake render tile row",
                        ),
                        asset: runtime_text(
                            runtime_record_field(fields, "asset", "snake render tile"),
                            "snake render tile asset",
                        ),
                    }
                })
                .collect()
        })
    }

    fn head_tile(tiles: &[SnakeRenderTile]) -> &SnakeRenderTile {
        tiles
            .iter()
            .find(|tile| {
                Path::new(&tile.asset)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("head_"))
            })
            .expect("snake tiles should include a head sprite")
    }

    fn snake_direction_for(harness: &super::RunSessionHarness) -> String {
        let state = named_signal_value_for(harness, "state");
        let fields = runtime_record_fields(&state, "snake state");
        runtime_sum_variant(
            runtime_record_field(fields, "dir", "snake state"),
            "snake direction",
        )
    }

    fn snake_status_for(harness: &super::RunSessionHarness) -> String {
        let state = named_signal_value_for(harness, "state");
        let fields = runtime_record_fields(&state, "snake state");
        runtime_sum_variant(
            runtime_record_field(fields, "status", "snake state"),
            "snake status",
        )
    }

    fn collect_picture_files(widget: &gtk::Widget, files: &mut Vec<String>) {
        if let Ok(picture) = widget.clone().downcast::<gtk::Picture>()
            && let Some(path) = picture.file().and_then(|file| file.path())
        {
            files.push(path.to_string_lossy().into_owned());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect_picture_files(&current, files);
            child = current.next_sibling();
        }
    }

    fn gtk_board_picture_files_for(harness: &super::RunSessionHarness) -> Vec<String> {
        let mut files = Vec::new();
        for window in harness.root_windows() {
            collect_picture_files(&window.clone().upcast::<gtk::Widget>(), &mut files);
        }
        files
    }

    fn find_button_by_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button);
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
        let own_count = widget_text(widget)
            .is_some_and(|text| text == label)
            .then_some(1)
            .unwrap_or(0);
        let mut child = widget.first_child();
        let mut child_count = 0;
        while let Some(current) = child {
            child_count += count_buttons_by_label(&current, label);
            child = current.next_sibling();
        }
        own_count + child_count
    }

    fn widget_text(widget: &gtk::Widget) -> Option<String> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            return button.label().as_deref().map(ToOwned::to_owned);
        }
        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
            return Some(label.text().to_string());
        }
        None
    }

    fn button_label_count_for(harness: &super::RunSessionHarness, label: &str) -> usize {
        harness
            .root_windows()
            .iter()
            .map(|window| count_buttons_by_label(&window.clone().upcast::<gtk::Widget>(), label))
            .sum()
    }

    fn find_sensitive_button_by_label(
        harness: &super::RunSessionHarness,
        label: &str,
    ) -> Option<gtk::Button> {
        harness.root_windows().iter().find_map(|window| {
            find_button_by_label(&window.clone().upcast::<gtk::Widget>(), label)
                .filter(|button| button.is_sensitive())
        })
    }

    fn has_sensitive_button_by_label(harness: &super::RunSessionHarness, label: &str) -> bool {
        find_sensitive_button_by_label(harness, label).is_some()
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
    fn run_error_report_renders_boxed_tui_layout_with_color() {
        let rendered = render_run_error_report_with_color(
            "runtime crash",
            &[("path", "demos/snake.aivi"), ("view", "main")],
            "something bad happened",
            "fix it and rerun.",
            true,
        );

        assert!(rendered.contains("\x1b[38;2;"));
        assert!(is_run_error_report(&rendered));
        assert!(rendered.contains("╭─"));
        assert!(rendered.contains("├─"));
        assert!(rendered.contains("╰─"));
        assert!(rendered.contains("something bad happened"));
    }

    #[test]
    fn run_error_report_plain_mode_avoids_ansi() {
        let rendered = render_run_error_report_with_color(
            "startup failed",
            &[("path", "demos/reversi.aivi"), ("view", "main")],
            "failed to link backend runtime",
            "fix it and rerun.",
            false,
        );

        assert!(!rendered.contains("\x1b["));
        assert!(rendered.starts_with("╭─ aivi run • startup failed"));
        assert!(rendered.contains("failed to link backend runtime"));
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
    fn startup_gate_starts_unpresented_with_manual_sources() {
        let gate = RunStartupGate::new(Some(
            vec![
                aivi_runtime::SourceInstanceId::from_raw(1),
                aivi_runtime::SourceInstanceId::from_raw(2),
            ]
            .into_boxed_slice(),
        ));

        assert!(!gate.roots_presented);
        assert_eq!(
            gate.manual_sources
                .as_deref()
                .map(|items: &[aivi_runtime::SourceInstanceId]| items.len()),
            Some(2)
        );
    }

    #[gtk::test]
    fn startup_progress_reports_completed_prepresent_stages() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let artifact = prepare_run_from_text(
            "startup-progress.aivi",
            r#"
value main =
    <Window title="Host">
        <Label text="Ready" />
    </Window>

export main
"#,
        );
        let reported = std::cell::RefCell::new(Vec::new());
        let harness = start_run_session_with_launch_config_and_reporter(
            Path::new("startup-progress.aivi"),
            artifact,
            RunLaunchConfig::default(),
            |stage, metrics| reported.borrow_mut().push((stage, *metrics)),
        )
        .expect("startup progress fixture should start a run session");
        let reported = reported.into_inner();
        assert_eq!(
            reported
                .iter()
                .map(|(stage, _)| *stage)
                .collect::<Vec<RunStartupStage>>(),
            vec![
                RunStartupStage::GtkInit,
                RunStartupStage::RuntimeLink,
                RunStartupStage::SessionSetup,
                RunStartupStage::InitialRuntimeTick,
                RunStartupStage::InitialHydrationWait,
                RunStartupStage::RootWindowCollection,
            ]
        );
        assert_eq!(reported[0].1.runtime_link, Duration::default());
        assert_eq!(reported[0].1.session_setup, Duration::default());
        assert_eq!(reported[0].1.initial_runtime_tick, Duration::default());
        assert_eq!(reported[0].1.initial_hydration_wait, Duration::default());
        assert_eq!(reported[0].1.root_window_collection, Duration::default());
        assert_eq!(reported[4].1.root_window_collection, Duration::default());
        assert!(
            reported.windows(2).all(|pair| {
                pair[1].1.total_to_session_ready >= pair[0].1.total_to_session_ready
            })
        );
        assert_eq!(
            reported.last().map(|(_, metrics)| *metrics),
            Some(harness.startup_metrics())
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn timer_sources_stay_paused_until_windows_present() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardTiles");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a paused run session");
        let context = harness.control().context();
        let initial_board = board_tiles_for(&harness, board_item);
        let initial_head = head_tile(&initial_board);
        let initial_hydration = harness.with_access(|access| access.latest_applied_hydration());
        assert_eq!(
            initial_head.column, 6,
            "shifted snake demo should start with runway"
        );

        pump_context(&context, Duration::from_millis(250));
        assert_eq!(
            board_tiles_for(&harness, board_item),
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
        assert!(
            pump_until(&context, Duration::from_secs(1), || {
                head_tile(&board_tiles_for(&harness, board_item)).column > initial_head.column
            }),
            "board should start advancing after presentation releases the startup-held timer source"
        );
        let advanced_board = board_tiles_for(&harness, board_item);
        assert!(
            head_tile(&advanced_board).column > initial_head.column,
            "board should keep advancing after presentation releases the startup-held timer source"
        );
        harness.shutdown();
    }

    #[gtk::test]
    fn main_loop_run_advances_timer_driven_board_after_presentation() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardTiles");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_board = board_tiles_for(&harness, board_item);
        let initial_head = head_tile(&initial_board);
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        let main_loop = harness.session.borrow().main_loop.clone();
        let quit_loop = main_loop.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(650), move || {
            quit_loop.quit();
        });
        main_loop.run();
        let advanced_board = board_tiles_for(&harness, board_item);
        assert!(
            head_tile(&advanced_board).column > initial_head.column,
            "the plain run-session main loop should advance the snake after presentation"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn main_loop_run_hydrates_board_pictures_after_timer_ticks() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_files = gtk_board_picture_files_for(&harness);
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        let main_loop = harness.session.borrow().main_loop.clone();
        let quit_loop = main_loop.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(650), move || {
            quit_loop.quit();
        });
        main_loop.run();
        let advanced_files = gtk_board_picture_files_for(&harness);
        assert_ne!(
            advanced_files, initial_files,
            "plain aivi run should hydrate the GTK picture grid after timer ticks"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn harness_run_main_loop_advances_timer_driven_board_without_borrow_panics() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardTiles");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let initial_board = board_tiles_for(&harness, board_item);
        let initial_head = head_tile(&initial_board);
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
        let advanced_board = board_tiles_for(&harness, board_item);
        assert!(
            head_tile(&advanced_board).column > initial_head.column,
            "the real run_main_loop path should keep advancing the snake while the GTK main loop runs"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn process_pending_work_applies_queued_window_key_events_immediately() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        harness
            .present_root_windows()
            .expect("presenting the run-session window should release startup timers");
        assert_eq!(snake_direction_for(&harness), "East");

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
            snake_direction_for(&harness),
            "North",
            "queued window key events should update the direction signal in the same run-session work cycle"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn queued_window_keys_do_not_pull_pending_timer_ticks_forward() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let artifact = prepare_run_from_text(
            "queued-window-key-no-timer-collapse.aivi",
            r#"
type Direction = Up | Right
type Key = Key Text

type Direction -> Text
func dirLineFor = arg1 => arg1
 ||> Up    -> "Up"
 ||> Right -> "Right"

type Int -> Text
func tickLineFor = arg1 =>
    "Ticks: {arg1}"

type Key -> Direction -> Direction
func updateDirection = key current => key
 ||> Key "ArrowUp" -> Up
 ||> _             -> current

type Unit -> Int -> Int
func countTick = unit current =>
    current + 1

@source timer.every 1000ms with {
    immediate: False,
    coalesce: True
}
signal tick : Signal Unit

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown : Signal Key

signal direction : Signal Direction =
    keyDown
     +|> Right updateDirection

signal ticks : Signal Int =
    tick
     +|> 0 countTick

from direction = {
    dirLine: dirLineFor
}

from ticks = {
    tickLine: tickLineFor
}

value main =
    <Window title="Queued window key probe">
        <Box orientation="vertical">
            <Label text={dirLine} />
            <Label text={tickLine} />
        </Box>
    </Window>

export main
"#,
        );
        let direction_item = required_signal_item(&artifact, "dirLine");
        let tick_item = required_signal_item(&artifact, "tickLine");
        let path = PathBuf::from("queued-window-key-no-timer-collapse.aivi");
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("queued window key probe should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the probe window should release startup-held timers");

        let driver = harness.control().driver();
        let timer_binding = driver
            .source_bindings()
            .into_iter()
            .find(|binding| {
                driver
                    .source_provider(binding.instance)
                    .is_some_and(|provider| {
                        provider
                            .builtin_provider()
                            .is_some_and(|builtin| builtin.key() == "timer.every")
                    })
            })
            .expect("probe should expose a timer source binding");
        driver
            .set_source_mode(
                timer_binding.instance,
                aivi_runtime::GlibLinkedSourceMode::Manual,
            )
            .expect("probe timer should switch to manual mode");

        assert_eq!(text_signal_for(&harness, direction_item), "Right");
        assert_eq!(text_signal_for(&harness, tick_item), "Ticks: 0");

        let timer_stamp = driver
            .current_stamp(timer_binding.input)
            .expect("probe timer input should expose a publication stamp");
        driver
            .queue_publication(aivi_runtime::Publication::new(
                timer_stamp,
                DetachedRuntimeValue::unit(),
            ))
            .expect("probe timer publication should queue");

        harness.with_access(|access| {
            access
                .executor_mut()
                .host_mut()
                .queue_window_key_event("ArrowUp", false);
            access
                .process_pending_work()
                .expect("queued window key should process without forcing pending timer work");
        });

        assert_eq!(
            text_signal_for(&harness, direction_item),
            "Up",
            "queued window key events should still update direction in the same work cycle"
        );
        assert_eq!(
            text_signal_for(&harness, tick_item),
            "Ticks: 0",
            "processing a queued key must not also drain an already queued timer publication"
        );

        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                text_signal_for(&harness, tick_item) == "Ticks: 1"
            }),
            "the queued timer publication should still apply on the later async wake"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn button_click_event_payloads_use_current_markup_bindings() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
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
        harness
            .present_root_windows()
            .expect("presenting the fixture window should trigger initial hydration");
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Beta").is_some()
                })
            }),
            "fixture should render a Beta button after presentation"
        );
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
    fn parameterized_from_selectors_refresh_markup_after_signal_updates() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let artifact = prepare_run_from_text(
            "from-selector-refresh-run.aivi",
            r#"
type Coord = Coord Int Int

type State = {
    on: Bool
}

type Coord -> State -> Text
func stateCellLabel = cell state => state.on
 T|> "On"
 F|> "Off"

signal click : Signal Unit
type Unit -> State -> State
func step = input current => { on: not (current.on) }

value initialState = { on: False }

signal state : Signal State = click
 +|> initialState step

from state = {
    type Coord -> Text
    cellLabel cell: stateCellLabel cell
}

value main =
    <Window title="From selector refresh">
        <Button label={cellLabel (Coord 0 0)} onClick={click} />
    </Window>

export main
"#,
        );
        let harness = start_run_session_with_launch_config(
            Path::new("from-selector-refresh-run.aivi"),
            artifact,
            RunLaunchConfig::default(),
        )
        .expect("from-selector refresh fixture should start a run session");
        let context = harness.control().context();
        harness
            .present_root_windows()
            .expect("presenting the fixture window should trigger initial hydration");
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Off").is_some()
                })
            }),
            "fixture should render the initial Off label after presentation"
        );
        let button = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Off"))
            .expect("fixture should render the initial Off label");

        button.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                harness.root_windows().iter().any(|window| {
                    find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "On").is_some()
                })
            }),
            "parameterized from selectors should refresh markup labels after signal updates (state: {})",
            debug_signal_value_for(&harness, "state"),
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_run_session_exposes_human_opening_move() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let (path, artifact) = prepare_reversi_run();
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));

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
        let (path, artifact) = prepare_reversi_run();
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));
        let initial_hydration = harness.with_access(|access| access.latest_applied_hydration());
        let opening_red_count = button_label_count_for(&harness, "🔴");
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
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "an idle reversi session should still accept the first human move"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_restart_resets_the_board_during_the_ai_turn() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let (path, artifact) = prepare_reversi_run();
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));

        let opening_red_count = button_label_count_for(&harness, "🔴");
        let opening_state = debug_signal_value_for(&harness, "state");
        let opening_move = find_sensitive_button_by_label(&harness, "◌")
            .expect("reversi should expose a clickable opening move");
        let restart = harness
            .root_windows()
            .iter()
            .find_map(|window| {
                find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "Restart")
            })
            .expect("reversi window should expose a restart button");

        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "the opening human move should land before attempting a restart"
        );

        restart.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                debug_signal_value_for(&harness, "state") == opening_state
            }),
            "restart should restore the opening board even if the AI turn had already started (phase: {}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            debug_signal_value_for(&harness, "state"),
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_restart_resets_after_game_over() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        assert_reversi_restart_resets_after_terminal_fixture(
            near_endgame_reversi_source(),
            "the near-endgame human-final fixture",
            Duration::from_millis(250),
        );
    }

    #[gtk::test]
    fn reversi_restart_resets_after_computer_final_move() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        assert_reversi_restart_resets_after_terminal_fixture(
            computer_final_reversi_source(),
            "the computer-final fixture",
            Duration::from_secs(4),
        );
    }

    #[gtk::test]
    fn reversi_restart_resets_after_pass_chain_game_over() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        assert_reversi_restart_resets_after_terminal_fixture(
            pass_chain_terminal_reversi_source(),
            "the pass-chain terminal fixture",
            Duration::from_secs(8),
        );
    }

    #[gtk::test]
    fn reversi_human_moves_paint_red_stones_promptly() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let (path, artifact) = prepare_reversi_run();
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));

        let opening_red_count = button_label_count_for(&harness, "🔴");
        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "the first human move should paint its red stones without waiting for the AI turn (phase: {}, requested: {:?}, applied: {:?}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            harness.with_access(|access| access.latest_requested_hydration()),
            harness.with_access(|access| access.latest_applied_hydration()),
            debug_signal_value_for(&harness, "state"),
        );
        assert!(
            pump_until(&context, Duration::from_secs(4), || {
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
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > second_turn_red_count
            }),
            "the second human move should also paint its red stones right away"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_stays_playable_after_the_first_full_turn() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let (path, artifact) = prepare_reversi_run();
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));
        let opening_red_count = button_label_count_for(&harness, "🔴");
        let opening_white_count = button_label_count_for(&harness, "⚪");

        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "clicking a legal move should put the new red stone on the board right away"
        );
        pump_context(&context, Duration::from_millis(100));
        assert!(
            !has_sensitive_button_by_label(&harness, "◌"),
            "the first move should promptly hand control away from the human player"
        );
        let thinking_white_count = button_label_count_for(&harness, "⚪");
        assert_eq!(
            thinking_white_count,
            opening_white_count - 1,
            "the board should stay visually unchanged while the computer is only thinking"
        );
        assert!(
            pump_until(&context, Duration::from_millis(600), || {
                button_label_count_for(&harness, "⚪") > thinking_white_count
                    && !has_sensitive_button_by_label(&harness, "◌")
            }),
            "the computer target should flash onto the board before the move commits"
        );
        assert!(
            pump_until(&context, Duration::from_secs(4), || {
                has_sensitive_button_by_label(&harness, "◌")
            }),
            "after the computer flash sequence the game should return to a playable human turn (phase: {}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            debug_signal_value_for(&harness, "state"),
        );

        assert!(
            pump_until(&context, Duration::from_secs(4), || {
                has_sensitive_button_by_label(&harness, "◌")
            }),
            "after the AI reply the GTK tree should expose a clickable human move"
        );
        let second_move = find_sensitive_button_by_label(&harness, "◌")
            .expect("reversi should expose another clickable human move after the AI reply");
        let second_turn_red_count = button_label_count_for(&harness, "🔴");
        second_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > second_turn_red_count
            }),
            "the second human move should still land without crashing (phase: {}, state: {})",
            debug_signal_value_for(&harness, "phase"),
            debug_signal_value_for(&harness, "state"),
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn reversi_profiled_hydration_reports_fragment_and_kernel_activity() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let (path, artifact) = prepare_reversi_run();
        let shared = RunHydrationStaticState {
            view_name: artifact.view_name.clone(),
            patterns: artifact.patterns.clone(),
            bridge: artifact.bridge.clone(),
            inputs: artifact.hydration_inputs.clone(),
            runtime_execution: Arc::new(RunFragmentExecutionUnit::new(
                artifact.backend.clone(),
                artifact.backend_native_kernels.clone(),
            )),
        };
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("reversi demo should start a run session");
        let context = harness.control().context();
        present_root_windows_and_wait_for_hydration(&harness, Duration::from_secs(1));

        let opening_red_count = button_label_count_for(&harness, "🔴");
        let opening_move = harness
            .root_windows()
            .iter()
            .find_map(|window| find_button_by_label(&window.clone().upcast::<gtk::Widget>(), "◌"))
            .expect("reversi board should expose a legal opening move");
        opening_move.emit_clicked();
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                button_label_count_for(&harness, "🔴") > opening_red_count
            }),
            "opening move should land before profiling hydration"
        );
        pump_context(&context, Duration::from_millis(100));
        assert!(
            !has_sensitive_button_by_label(&harness, "◌"),
            "profiling should sample the live reversi session after it has left the opening human turn"
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
    #[ignore = "manual latency probe for snake turn hydration"]
    fn snake_profiled_turn_hydration_reports_runtime_cost() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let shared = RunHydrationStaticState {
            view_name: artifact.view_name.clone(),
            patterns: artifact.patterns.clone(),
            bridge: artifact.bridge.clone(),
            inputs: artifact.hydration_inputs.clone(),
            runtime_execution: Arc::new(RunFragmentExecutionUnit::new(
                artifact.backend.clone(),
                artifact.backend_native_kernels.clone(),
            )),
        };
        let harness =
            start_run_session_with_launch_config(&path, artifact, RunLaunchConfig::default())
                .expect("snake demo should start a run session");
        let context = harness.control().context();
        let driver = harness.control().driver();
        harness
            .present_root_windows()
            .expect("presenting the snake window should release startup-held timers");

        for binding in driver.source_bindings() {
            if driver
                .source_provider(binding.instance)
                .is_some_and(|provider| {
                    provider
                        .builtin_provider()
                        .is_some_and(|builtin| builtin.key() == "timer.every")
                })
            {
                driver
                    .set_source_mode(binding.instance, aivi_runtime::GlibLinkedSourceMode::Manual)
                    .expect("snake timer source should switch to manual mode");
            }
        }
        pump_context(&context, Duration::from_millis(50));

        let baseline_globals = harness.with_access(|access| {
            access
                .driver()
                .current_signal_globals()
                .expect("baseline snake globals should be readable")
        });
        let (_baseline_plan, baseline_profile) =
            plan_run_hydration_profiled(&shared, &baseline_globals)
                .expect("baseline snake hydration should be profileable");

        driver.dispatch_window_key_event("ArrowUp", false);
        assert!(
            pump_until(&context, Duration::from_millis(250), || {
                snake_direction_for(&harness) == "North"
            }),
            "dispatching ArrowUp should update the snake direction before profiling hydration"
        );

        let turned_globals = harness.with_access(|access| {
            access
                .driver()
                .current_signal_globals()
                .expect("turned snake globals should be readable")
        });
        let (_turn_plan, turn_profile) = plan_run_hydration_profiled(&shared, &turned_globals)
            .expect("turned snake hydration should be profileable");

        eprintln!(
            "snake hydration profile: baseline={:?}, turn={:?}, baseline_nodes={}, turn_nodes={}, baseline_inputs={}, turn_inputs={}",
            baseline_profile.total_time,
            turn_profile.total_time,
            baseline_profile.planned_nodes,
            turn_profile.planned_nodes,
            baseline_profile.evaluated_inputs,
            turn_profile.evaluated_inputs
        );

        assert!(baseline_profile.planned_nodes > 0);
        assert!(turn_profile.planned_nodes > 0);

        harness.shutdown();
    }

    #[gtk::test]
    fn space_restarts_snake_after_game_over() {
        let path = repo_path("demos/snake.aivi");
        let artifact = prepare_run_from_path(&path);
        let board_item = required_signal_item(&artifact, "boardTiles");
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
                snake_direction_for(&harness) == "North"
            }),
            "dispatching ArrowUp should update the snake direction before waiting for a collision"
        );
        assert!(
            pump_until(&context, Duration::from_secs(5), || {
                snake_status_for(&harness) == "GameOver"
            }),
            "steering upward should eventually collide with the wall and end the game"
        );
        let game_over_board = board_tiles_for(&harness, board_item);
        assert_eq!(snake_direction_for(&harness), "North");

        driver.dispatch_window_key_event("Space", false);
        assert!(
            pump_until(&context, Duration::from_millis(100), || {
                snake_status_for(&harness) == "Running" && snake_direction_for(&harness) == "East"
            }),
            "pressing Space should immediately reset the event-driven snake"
        );
        let restarted_board = board_tiles_for(&harness, board_item);
        let restarted_head = head_tile(&restarted_board);
        assert_eq!(
            restarted_head.row, 10,
            "restart should return the snake to its starting row"
        );
        assert!(
            matches!(restarted_head.column, 6 | 7),
            "restart should return the snake to its starting lane before or just after the first timer step"
        );
        assert_ne!(
            restarted_board, game_over_board,
            "restart should replace the game-over board with a fresh starting board"
        );

        harness.shutdown();
    }

    #[gtk::test]
    fn headless_run_session_starts_without_gtk_windows_and_activates_sources() {
        let _guard = crate::gtk_test_lock().lock().expect("gtk test lock");
        let artifact = prepare_run_from_text(
            "headless-run.aivi",
            r#"
use aivi.stdio (
    stdoutWrite
)

@source process.cwd
signal cwd : Signal Text

value main : Task Text Unit =
    stdoutWrite ""
"#,
        );
        let path = Path::new("headless-run.aivi");
        let harness =
            start_run_session_with_launch_config(path, artifact, RunLaunchConfig::default())
                .expect("headless run session should start without GTK setup");

        assert!(
            harness.root_windows().is_empty(),
            "headless runs should not materialize GTK root windows"
        );
        let cwd = runtime_text(&named_signal_value_for(&harness, "cwd"), "cwd");
        assert!(
            cwd.contains(std::path::MAIN_SEPARATOR),
            "headless runs should activate immediate process sources"
        );

        harness.shutdown();
    }
}
