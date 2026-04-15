use std::{
    cell::Cell,
    collections::{BTreeSet, VecDeque},
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use aivi_backend::DetachedRuntimeValue;
use glib::MainContext;

use crate::{
    BackendLinkedRuntime, BackendRuntimeError, DerivedNodeEvaluator, EvaluatedSourceConfig,
    Generation, InputHandle, LinkedSourceBinding, LinkedSourceTickOutcome, OwnerHandle,
    Publication, PublicationStamp, RuntimeSourceProvider, Scheduler, SchedulerAccessError,
    SignalGraph, SignalHandle, SourceInstanceId, SourceProviderExecutionError,
    SourceProviderManager, TaskSourceRuntimeError, TickOutcome, WorkerPublicationSender,
    WorkerSendError,
};

/// Drive a scheduler from a GLib main context without letting worker threads mutate scheduler
/// state directly.
///
/// The scheduler/evaluator pair sits behind a narrow `Arc<Mutex<_>>` only because GLib's cross-
/// thread wake APIs require `Send` captures. All actual `tick()` execution still happens on the
/// owned `MainContext`; workers only enqueue publications and request a main-context wakeup.
#[derive(Clone)]
pub struct GlibSchedulerDriver<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    context: MainContext,
    shared: Arc<GlibSchedulerShared<V, E>>,
}

impl<V, E> GlibSchedulerDriver<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    pub fn new(context: MainContext, scheduler: Scheduler<V>, evaluator: E) -> Self {
        Self {
            context: context.clone(),
            shared: Arc::new(GlibSchedulerShared::new(scheduler, evaluator, context)),
        }
    }

    pub fn context(&self) -> &MainContext {
        &self.context
    }

    pub fn worker_sender(&self) -> GlibWorkerPublicationSender<V, E> {
        GlibWorkerPublicationSender {
            sender: self.with_state(|state| state.scheduler.worker_sender()),
            shared: self.shared.clone(),
        }
    }

    pub fn queue_publication(&self, publication: Publication<V>) -> Result<(), GlibSchedulerError> {
        self.with_state_mut(|state| state.scheduler.queue_publication(publication))
            .map_err(GlibSchedulerError::SchedulerAccess)?;
        self.shared.request_tick();
        Ok(())
    }

    pub fn queue_publication_now(
        &self,
        publication: Publication<V>,
    ) -> Result<(), GlibSchedulerError> {
        self.with_state_mut(|state| state.scheduler.queue_publication(publication))
            .map_err(GlibSchedulerError::SchedulerAccess)?;
        self.shared
            .drive_pending_ticks(GlibTickDrainMode::UntilIdle);
        Ok(())
    }

    pub fn queue_dispose_owner(&self, owner: OwnerHandle) -> Result<(), GlibSchedulerError> {
        self.with_state_mut(|state| state.scheduler.queue_dispose_owner(owner))
            .map_err(GlibSchedulerError::SchedulerAccess)?;
        self.shared.request_tick();
        Ok(())
    }

    pub fn current_stamp(
        &self,
        input: InputHandle,
    ) -> Result<PublicationStamp, GlibSchedulerError> {
        self.with_state(|state| state.scheduler.current_stamp(input))
            .map_err(GlibSchedulerError::SchedulerAccess)
    }

    pub fn advance_generation(
        &self,
        input: InputHandle,
    ) -> Result<PublicationStamp, GlibSchedulerError> {
        self.with_state_mut(|state| state.scheduler.advance_generation(input))
            .map_err(GlibSchedulerError::SchedulerAccess)
    }

    pub fn current_value(&self, signal: SignalHandle) -> Result<Option<V>, GlibSchedulerError>
    where
        V: Clone,
    {
        self.with_state(|state| {
            state
                .scheduler
                .current_value(signal)
                .map(|value| value.cloned())
        })
        .map_err(GlibSchedulerError::SchedulerAccess)
    }

    pub fn tick_count(&self) -> u64 {
        self.with_state(|state| state.scheduler.tick_count())
    }

    pub fn queued_message_count(&self) -> usize {
        self.with_state(|state| state.scheduler.queued_message_count())
    }

    pub fn outcome_count(&self) -> usize {
        self.with_state(|state| state.outcomes.len())
    }

    pub fn drain_outcomes(&self) -> Vec<TickOutcome> {
        self.with_state_mut(|state| state.outcomes.drain(..).collect())
    }

    fn with_state<R>(&self, f: impl FnOnce(&GlibSchedulerState<V, E>) -> R) -> R {
        assert_non_reentrant_driver_access();
        let guard = self
            .shared
            .state
            .lock()
            .expect("GLib scheduler state mutex should not be poisoned");
        f(guard
            .as_ref()
            .expect("GLib scheduler state should be present when not inside a tick"))
    }

    fn with_state_mut<R>(&self, f: impl FnOnce(&mut GlibSchedulerState<V, E>) -> R) -> R {
        assert_non_reentrant_driver_access();
        let mut guard = self
            .shared
            .state
            .lock()
            .expect("GLib scheduler state mutex should not be poisoned");
        f(guard
            .as_mut()
            .expect("GLib scheduler state should be present when not inside a tick"))
    }
}

#[derive(Debug)]
pub enum GlibSchedulerError {
    SchedulerAccess(SchedulerAccessError),
}

impl fmt::Display for GlibSchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchedulerAccess(error) => write!(f, "GLib scheduler access failed: {error:?}"),
        }
    }
}

impl Error for GlibSchedulerError {}

thread_local! {
    static GLIB_DRIVER_TICK_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

struct TickExecutionGuard;

impl TickExecutionGuard {
    fn enter() -> Self {
        GLIB_DRIVER_TICK_ACTIVE.with(|active| {
            assert!(
                !active.replace(true),
                "GLib scheduler ticks must not re-enter themselves"
            );
        });
        Self
    }
}

impl Drop for TickExecutionGuard {
    fn drop(&mut self) {
        GLIB_DRIVER_TICK_ACTIVE.with(|active| active.set(false));
    }
}

fn assert_non_reentrant_driver_access() {
    GLIB_DRIVER_TICK_ACTIVE.with(|active| {
        assert!(
            !active.get(),
            "GLib scheduler driver access must not re-enter the driver while a tick is evaluating"
        );
    });
}

const GLIB_ASYNC_WAKE_TICK_BUDGET: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlibTickDrainMode {
    UntilIdle,
    UntilCurrentQueue,
    AsyncWakeBudgeted,
}

impl GlibTickDrainMode {
    fn should_continue_after_tick(
        self,
        async_wake_requested: bool,
        queued_messages: usize,
    ) -> bool {
        match self {
            Self::UntilIdle | Self::AsyncWakeBudgeted => {
                async_wake_requested || queued_messages > 0
            }
            Self::UntilCurrentQueue => queued_messages > 0,
        }
    }

    fn should_reschedule_after_break(self, async_wake_requested: bool) -> bool {
        matches!(self, Self::UntilCurrentQueue) && async_wake_requested
    }

    fn should_yield_after_tick(self, drained_ticks: usize) -> bool {
        matches!(self, Self::AsyncWakeBudgeted) && drained_ticks >= GLIB_ASYNC_WAKE_TICK_BUDGET
    }
}

#[derive(Clone)]
pub struct GlibWorkerPublicationSender<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    sender: WorkerPublicationSender<V>,
    shared: Arc<GlibSchedulerShared<V, E>>,
}

impl<V, E> GlibWorkerPublicationSender<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    pub fn publish(&self, publication: Publication<V>) -> Result<(), WorkerSendError<V>> {
        self.sender.publish(publication)?;
        self.shared.request_tick();
        Ok(())
    }
}

struct GlibSchedulerShared<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    context: MainContext,
    /// State is held inside `Option` so the tick loop can take it out of the
    /// Mutex for the duration of `scheduler.tick(evaluator)`, releasing the
    /// lock while the tick runs.  The `TickExecutionGuard` / re-entrancy check
    /// ensures no caller can observe the `None` state: any concurrent access
    /// will panic before reaching the Mutex.
    state: Mutex<Option<GlibSchedulerState<V, E>>>,
    tick_enqueued: AtomicBool,
}

impl<V, E> GlibSchedulerShared<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    fn new(scheduler: Scheduler<V>, evaluator: E, context: MainContext) -> Self {
        Self {
            context,
            state: Mutex::new(Some(GlibSchedulerState {
                scheduler,
                evaluator,
                outcomes: VecDeque::new(),
            })),
            tick_enqueued: AtomicBool::new(false),
        }
    }

    fn request_tick(self: &Arc<Self>) {
        if self.tick_enqueued.swap(true, Ordering::AcqRel) {
            return;
        }

        self.spawn_async_tick_drive();
    }

    fn spawn_async_tick_drive(self: &Arc<Self>) {
        let shared = self.clone();
        self.context.spawn(async move {
            shared.drive_pending_ticks(GlibTickDrainMode::AsyncWakeBudgeted);
        });
    }

    fn drive_pending_ticks(self: &Arc<Self>, mode: GlibTickDrainMode) {
        let guard = TickExecutionGuard::enter();
        let mut drained_ticks = 0usize;
        let mut reschedule = false;
        loop {
            self.tick_enqueued.store(false, Ordering::Release);

            // Phase 1: take the full state out of the Mutex so the lock is
            // released before the tick runs.
            let mut state = self
                .state
                .lock()
                .expect("GLib scheduler state mutex should not be poisoned")
                .take()
                .expect("GLib scheduler state should be present before a tick");

            // Phase 2: run the tick with no lock held.  GTK redraws and other
            // GLib event sources are free to run between the two lock scopes.
            let outcome = state.scheduler.tick(&mut state.evaluator);

            // Phase 3: re-acquire the lock to store the outcome and put the
            // state back.
            let should_continue = {
                let mut guard = self
                    .state
                    .lock()
                    .expect("GLib scheduler state mutex should not be poisoned");
                let queued_count = state.scheduler.queued_message_count();
                if !outcome.is_empty() {
                    state.outcomes.push_back(outcome);
                }
                *guard = Some(state);
                let async_wake_requested = self.tick_enqueued.load(Ordering::Acquire);
                (
                    async_wake_requested,
                    mode.should_continue_after_tick(async_wake_requested, queued_count),
                )
            };

            drained_ticks = drained_ticks.saturating_add(1);
            let (async_wake_requested, should_continue) = should_continue;

            if !should_continue {
                reschedule |= mode.should_reschedule_after_break(async_wake_requested);
                break;
            }
            if mode.should_yield_after_tick(drained_ticks) {
                reschedule = true;
                break;
            }
        }
        drop(guard);
        if reschedule {
            self.request_tick();
        }
    }
}

struct GlibSchedulerState<V, E>
where
    V: Send + 'static,
    E: DerivedNodeEvaluator<V> + Send + 'static,
{
    scheduler: Scheduler<V>,
    evaluator: E,
    outcomes: VecDeque<TickOutcome>,
}

#[derive(Clone)]
pub struct GlibLinkedRuntimeDriver {
    context: MainContext,
    shared: Arc<GlibLinkedRuntimeShared>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlibLinkedSourceMode {
    Live,
    Manual,
}

impl GlibLinkedRuntimeDriver {
    pub fn new(
        context: MainContext,
        linked: BackendLinkedRuntime,
        providers: SourceProviderManager,
        notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) -> Self {
        let shared = Arc::new(GlibLinkedRuntimeShared::new(
            linked,
            providers,
            context.clone(),
            notifier,
        ));
        let worker_notifier = shared.worker_notifier();
        let db_commit_invalidation_sink = shared.db_commit_invalidation_sink();
        {
            let mut state = shared
                .state
                .lock()
                .expect("GLib linked runtime state mutex should not be poisoned");
            state
                .as_mut()
                .expect("GLib linked runtime state should exist before worker notifier install")
                .linked
                .runtime_mut()
                .set_worker_publication_notifier(Some(worker_notifier));
            state
                .as_mut()
                .expect("GLib linked runtime state should exist before commit invalidation install")
                .linked
                .set_db_commit_invalidation_sink(Some(db_commit_invalidation_sink));
        }
        Self { context, shared }
    }

    pub fn context(&self) -> &MainContext {
        &self.context
    }

    pub fn queue_publication(
        &self,
        publication: Publication<DetachedRuntimeValue>,
    ) -> Result<(), GlibLinkedRuntimeAccessError> {
        let (stamp, value) = publication.into_parts();
        self.with_state_mut(|state| {
            state
                .linked
                .runtime_mut()
                .queue_publication(Publication::new(stamp, value.into_runtime()))
        })
        .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)?;
        self.shared.request_tick();
        Ok(())
    }

    pub fn queue_publication_now(
        &self,
        publication: Publication<DetachedRuntimeValue>,
    ) -> Result<(), GlibLinkedRuntimeAccessError> {
        let (stamp, value) = publication.into_parts();
        self.with_state_mut(|state| {
            state
                .linked
                .runtime_mut()
                .queue_publication(Publication::new(stamp, value.into_runtime()))
        })
        .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)?;
        self.tick_now();
        Ok(())
    }

    pub fn queue_publication_now_current_queue(
        &self,
        publication: Publication<DetachedRuntimeValue>,
    ) -> Result<(), GlibLinkedRuntimeAccessError> {
        let (stamp, value) = publication.into_parts();
        self.with_state_mut(|state| {
            state
                .linked
                .runtime_mut()
                .queue_publication(Publication::new(stamp, value.into_runtime()))
        })
        .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)?;
        self.shared
            .drive_pending_ticks(GlibTickDrainMode::UntilCurrentQueue);
        Ok(())
    }

    pub fn current_stamp(
        &self,
        input: InputHandle,
    ) -> Result<PublicationStamp, GlibLinkedRuntimeAccessError> {
        self.with_state(|state| state.linked.runtime().current_stamp(input))
            .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)
    }

    pub fn current_signal_globals(
        &self,
    ) -> Result<
        std::collections::BTreeMap<aivi_backend::ItemId, DetachedRuntimeValue>,
        GlibLinkedRuntimeAccessError,
    > {
        self.with_state(|state| state.linked.current_signal_globals())
            .map_err(GlibLinkedRuntimeAccessError::Backend)
    }

    pub fn signal_graph(&self) -> SignalGraph {
        self.with_state(|state| state.linked.signal_graph().clone())
    }

    /// Build a [`RuntimeSourceMap`] for rendering runtime errors with source context.
    pub fn build_source_map(&self) -> crate::source_map::RuntimeSourceMap {
        self.with_state(|state| state.linked.build_source_map())
    }

    pub fn current_signal_value(
        &self,
        signal: SignalHandle,
    ) -> Result<Option<DetachedRuntimeValue>, GlibLinkedRuntimeAccessError> {
        self.with_state(|state| {
            state
                .linked
                .runtime()
                .current_value(signal)
                .map(|value| value.cloned().map(DetachedRuntimeValue::from_runtime_owned))
        })
        .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)
    }

    pub fn current_generation(
        &self,
        input: InputHandle,
    ) -> Result<Generation, GlibLinkedRuntimeAccessError> {
        self.with_state(|state| state.linked.runtime().current_generation(input))
            .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)
    }

    pub fn source_bindings(&self) -> Vec<LinkedSourceBinding> {
        self.with_state(|state| state.linked.source_bindings().cloned().collect())
    }

    /// Stop this driver permanently: prevent any further ticks from being
    /// scheduled, and suspend every active source provider so that background
    /// worker threads are stopped and their GLib-context callbacks are removed.
    ///
    /// This must be called during session teardown (before the GLib main loop
    /// exits) so that the GLib context is clean for the next session, which is
    /// critical in test environments where multiple sessions share a single
    /// GLib main context.
    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        let instances: Vec<SourceInstanceId> = self
            .source_bindings()
            .into_iter()
            .map(|b| b.instance)
            .collect();
        self.with_state_mut(|state| {
            for instance in instances {
                let _ = state.linked.runtime_mut().suspend_source(instance);
                state.providers.suspend_active_provider(instance);
            }
        });
    }

    pub fn source_binding(&self, instance: SourceInstanceId) -> Option<LinkedSourceBinding> {
        self.with_state(|state| state.linked.source_binding(instance).cloned())
    }

    pub fn source_provider(&self, instance: SourceInstanceId) -> Option<RuntimeSourceProvider> {
        self.with_state(|state| {
            state
                .linked
                .runtime()
                .source_spec(instance)
                .map(|spec| spec.provider.clone())
        })
    }

    pub fn evaluate_source_config(
        &self,
        instance: SourceInstanceId,
    ) -> Result<EvaluatedSourceConfig, GlibLinkedRuntimeAccessError> {
        self.with_state(|state| state.linked.evaluate_source_config(instance))
            .map_err(GlibLinkedRuntimeAccessError::Backend)
    }

    pub fn backend(&self) -> Arc<aivi_backend::Program> {
        self.with_state(|state| state.linked.backend_arc())
    }

    pub fn is_source_active(&self, instance: SourceInstanceId) -> bool {
        self.with_state(|state| state.linked.runtime().is_source_active(instance))
    }

    pub fn has_active_provider(&self, instance: SourceInstanceId) -> bool {
        self.with_state(|state| state.providers.has_active_provider(instance))
    }

    pub fn source_mode(&self, instance: SourceInstanceId) -> GlibLinkedSourceMode {
        self.with_state(|state| {
            if state.manual_sources.contains(&instance) {
                GlibLinkedSourceMode::Manual
            } else {
                GlibLinkedSourceMode::Live
            }
        })
    }

    pub fn set_source_mode(
        &self,
        instance: SourceInstanceId,
        mode: GlibLinkedSourceMode,
    ) -> Result<(), GlibLinkedRuntimeAccessError> {
        self.with_state_mut(|state| {
            if state.linked.source_binding(instance).is_none() {
                return Err(GlibLinkedRuntimeAccessError::UnknownSourceInstance { instance });
            }
            match mode {
                GlibLinkedSourceMode::Manual => {
                    state.manual_sources.insert(instance);
                }
                GlibLinkedSourceMode::Live => {
                    state.manual_sources.remove(&instance);
                }
            }
            state
                .linked
                .runtime_mut()
                .suspend_source(instance)
                .map_err(GlibLinkedRuntimeAccessError::RuntimeAccess)?;
            state.providers.suspend_active_provider(instance);
            Ok(())
        })?;
        if matches!(mode, GlibLinkedSourceMode::Live) {
            self.shared.request_tick();
        }
        Ok(())
    }

    pub fn inject_source_value(
        &self,
        instance: SourceInstanceId,
        value: DetachedRuntimeValue,
    ) -> Result<PublicationStamp, GlibLinkedRuntimeAccessError> {
        let binding = self
            .source_binding(instance)
            .ok_or(GlibLinkedRuntimeAccessError::UnknownSourceInstance { instance })?;
        let stamp = self.current_stamp(binding.input)?;
        self.queue_publication_now(Publication::new(stamp, value))?;
        Ok(stamp)
    }

    pub fn dispatch_window_key_event(&self, name: &str, repeated: bool) {
        self.with_state_mut(|state| {
            state
                .providers
                .dispatch_window_key_event(crate::providers::WindowKeyEvent {
                    name: name.into(),
                    repeated,
                });
        });
    }

    /// Publishes a dark-mode state change to all active `gtk.darkMode` source instances.
    /// Called from the GTK main thread when `adw::StyleManager` dark state changes,
    /// and once at startup with the current dark state.
    pub fn dispatch_dark_mode_changed(&self, is_dark: bool) {
        self.with_state_mut(|state| {
            state.providers.dispatch_dark_mode_changed(is_dark);
        });
    }

    /// Publishes a clipboard text change to all active `clipboard.changed` source instances.
    /// Called from the GTK main thread when `gdk::Display::default().clipboard()` changes.
    pub fn dispatch_clipboard_changed(&self, text: String) {
        self.with_state_mut(|state| {
            state.providers.dispatch_clipboard_changed(text);
        });
    }

    /// Publishes a window size change to all active `window.size` source instances.
    pub fn dispatch_window_size_changed(&self, width: i32, height: i32) {
        self.with_state_mut(|state| {
            state.providers.dispatch_window_size_changed(width, height);
        });
    }

    /// Publishes a window focus change to all active `window.focus` source instances.
    pub fn dispatch_window_focus_changed(&self, focused: bool) {
        self.with_state_mut(|state| {
            state.providers.dispatch_window_focus_changed(focused);
        });
    }

    pub fn queued_message_count(&self) -> usize {
        self.with_state(|state| state.linked.queued_message_count())
    }

    pub fn outcome_count(&self) -> usize {
        self.with_state(|state| state.outcomes.len())
    }

    pub fn failure_count(&self) -> usize {
        self.with_state(|state| state.failures.len())
    }

    pub fn drain_outcomes(&self) -> Vec<LinkedSourceTickOutcome> {
        self.with_state_mut(|state| state.outcomes.drain(..).collect())
    }

    pub fn drain_failures(&self) -> Vec<GlibLinkedRuntimeFailure> {
        self.with_state_mut(|state| state.failures.drain(..).collect())
    }

    pub fn tick_now(&self) {
        self.shared
            .drive_pending_ticks(GlibTickDrainMode::UntilIdle);
    }

    pub fn request_tick(&self) {
        self.shared.request_tick();
    }

    fn with_state<R>(&self, f: impl FnOnce(&GlibLinkedRuntimeState) -> R) -> R {
        assert_non_reentrant_driver_access();
        let guard = self
            .shared
            .state
            .lock()
            .expect("GLib linked runtime state mutex should not be poisoned");
        f(guard
            .as_ref()
            .expect("GLib linked runtime state should be present when not inside a tick"))
    }

    fn with_state_mut<R>(&self, f: impl FnOnce(&mut GlibLinkedRuntimeState) -> R) -> R {
        assert_non_reentrant_driver_access();
        let mut guard = self
            .shared
            .state
            .lock()
            .expect("GLib linked runtime state mutex should not be poisoned");
        f(guard
            .as_mut()
            .expect("GLib linked runtime state should be present when not inside a tick"))
    }
}

#[derive(Debug)]
pub enum GlibLinkedRuntimeAccessError {
    RuntimeAccess(TaskSourceRuntimeError),
    Backend(BackendRuntimeError),
    UnknownSourceInstance { instance: SourceInstanceId },
}

impl fmt::Display for GlibLinkedRuntimeAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeAccess(error) => {
                write!(f, "GLib linked runtime access failed: {error:?}")
            }
            Self::Backend(error) => write!(f, "GLib linked runtime backend access failed: {error}"),
            Self::UnknownSourceInstance { instance } => {
                write!(
                    f,
                    "GLib linked runtime does not know source instance {}",
                    instance.as_raw()
                )
            }
        }
    }
}

impl Error for GlibLinkedRuntimeAccessError {}

#[derive(Clone, Debug)]
pub enum GlibLinkedRuntimeFailure {
    Tick(BackendRuntimeError),
    ProviderExecution(SourceProviderExecutionError),
}

impl fmt::Display for GlibLinkedRuntimeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tick(error) => write!(f, "linked runtime tick failed: {error}"),
            Self::ProviderExecution(error) => {
                write!(f, "linked runtime source execution failed: {error}")
            }
        }
    }
}

impl Error for GlibLinkedRuntimeFailure {}

struct GlibLinkedRuntimeShared {
    context: MainContext,
    /// Linked runtime state is kept inside `Option` so ticks can take ownership
    /// of the whole state, release the mutex while backend kernels and source
    /// lifecycle evaluation run, then store the updated state back afterward.
    state: Mutex<Option<GlibLinkedRuntimeState>>,
    tick_enqueued: AtomicBool,
    /// Set to `true` by `stop()` to prevent any further ticks from being
    /// scheduled or executed.  Already-queued async tasks check this flag at
    /// entry and return immediately, so that background-thread publications
    /// that arrived just before `stop()` do not process under a dead session.
    stopped: AtomicBool,
    notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
}

impl GlibLinkedRuntimeShared {
    fn new(
        linked: BackendLinkedRuntime,
        providers: SourceProviderManager,
        context: MainContext,
        notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            context,
            state: Mutex::new(Some(GlibLinkedRuntimeState {
                linked,
                providers,
                manual_sources: BTreeSet::new(),
                outcomes: VecDeque::new(),
                failures: VecDeque::new(),
            })),
            tick_enqueued: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            notifier,
        }
    }

    fn worker_notifier(self: &Arc<Self>) -> Arc<dyn Fn() + Send + Sync + 'static> {
        let shared = self.clone();
        Arc::new(move || shared.request_tick())
    }

    fn db_commit_invalidation_sink(self: &Arc<Self>) -> crate::startup::DbCommitInvalidationSink {
        let shared = self.clone();
        Arc::new(move |invalidation| {
            if shared.stopped.load(Ordering::Acquire) {
                return;
            }
            let shared = shared.clone();
            let context = shared.context.clone();
            context.spawn(async move {
                if shared.stopped.load(Ordering::Acquire) {
                    return;
                }
                if shared.apply_db_commit_invalidation(&invalidation) {
                    shared.request_tick();
                }
            });
        })
    }

    fn request_tick(self: &Arc<Self>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if self.tick_enqueued.swap(true, Ordering::AcqRel) {
            return;
        }

        self.spawn_async_tick_drive();
    }

    fn spawn_async_tick_drive(self: &Arc<Self>) {
        let shared = self.clone();
        self.context.spawn(async move {
            shared.drive_pending_ticks(GlibTickDrainMode::AsyncWakeBudgeted);
        });
    }

    fn notify_tick_ready(&self) {
        if let Some(notifier) = &self.notifier {
            notifier();
        }
    }

    fn apply_db_commit_invalidation(
        &self,
        invalidation: &crate::task_executor::RuntimeDbCommitInvalidation,
    ) -> bool {
        let mut guard = self
            .state
            .lock()
            .expect("GLib linked runtime state mutex should not be poisoned");
        let state = guard
            .as_mut()
            .expect("GLib linked runtime state should be present outside active ticks");
        state.linked.invalidate_db_commit(invalidation)
    }

    fn drive_pending_ticks(self: &Arc<Self>, mode: GlibTickDrainMode) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let guard = TickExecutionGuard::enter();
        let mut notify = false;
        let mut drained_ticks = 0usize;
        let mut reschedule = false;
        loop {
            self.tick_enqueued.store(false, Ordering::Release);
            let mut state = self
                .state
                .lock()
                .expect("GLib linked runtime state mutex should not be poisoned")
                .take()
                .expect("GLib linked runtime state should be present before a tick");
            let should_continue = match state.linked.tick_with_source_lifecycle() {
                Ok(outcome) => {
                    let provider_actions = outcome
                        .source_actions()
                        .iter()
                        .filter(|action| {
                            matches!(action, crate::LinkedSourceLifecycleAction::Suspend { .. })
                                || !state.manual_sources.contains(&action.instance())
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if let Err(error) = state.providers.apply_actions(&provider_actions) {
                        state
                            .failures
                            .push_back(GlibLinkedRuntimeFailure::ProviderExecution(error));
                        notify = true;
                        (false, false)
                    } else {
                        if !outcome.scheduler().is_empty() {
                            state.outcomes.push_back(outcome);
                            notify = true;
                        }
                        let async_wake_requested = self.tick_enqueued.load(Ordering::Acquire);
                        (
                            async_wake_requested,
                            mode.should_continue_after_tick(
                                async_wake_requested,
                                state.linked.queued_message_count(),
                            ),
                        )
                    }
                }
                Err(error) => {
                    state
                        .failures
                        .push_back(GlibLinkedRuntimeFailure::Tick(error));
                    notify = true;
                    (false, false)
                }
            };
            drained_ticks = drained_ticks.saturating_add(1);
            *self
                .state
                .lock()
                .expect("GLib linked runtime state mutex should not be poisoned") = Some(state);
            let (async_wake_requested, should_continue) = should_continue;

            if !should_continue {
                reschedule |= mode.should_reschedule_after_break(async_wake_requested);
                break;
            }
            if mode.should_yield_after_tick(drained_ticks) {
                reschedule = true;
                break;
            }
        }
        drop(guard);
        if notify {
            self.notify_tick_ready();
        }
        if reschedule {
            self.request_tick();
        }
    }
}

struct GlibLinkedRuntimeState {
    linked: BackendLinkedRuntime,
    providers: SourceProviderManager,
    manual_sources: BTreeSet<SourceInstanceId>,
    outcomes: VecDeque<LinkedSourceTickOutcome>,
    failures: VecDeque<GlibLinkedRuntimeFailure>,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        panic::{AssertUnwindSafe, catch_unwind},
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use aivi_backend::{DetachedRuntimeValue, ItemId as BackendItemId, RuntimeValue};
    use aivi_base::SourceDatabase;
    use aivi_core as core;
    use aivi_hir as hir;
    use aivi_hir::lower_module as lower_hir_module;
    use aivi_lambda::lower_module as lower_lambda_module;
    use aivi_syntax::parse_module;
    use glib::MainContext;

    use crate::{
        SourceProviderManager,
        graph::SignalGraphBuilder,
        scheduler::{DependencyValues, Publication, Scheduler},
    };

    use super::{
        GLIB_ASYNC_WAKE_TICK_BUDGET, GlibLinkedRuntimeDriver, GlibSchedulerDriver,
        TickExecutionGuard,
    };

    struct LoweredStack {
        hir: hir::LoweringResult,
        core: core::Module,
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
        let core = core::lower_module(hir.module()).expect("typed-core lowering should succeed");
        let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
        let backend = aivi_backend::lower_module_with_hir(&lambda, hir.module())
            .expect("backend lowering should succeed");
        LoweredStack { hir, core, backend }
    }

    fn backend_item_id(program: &aivi_backend::Program, name: &str) -> BackendItemId {
        program
            .items()
            .iter()
            .find_map(|(item_id, item)| (item.name.as_ref() == name).then_some(item_id))
            .unwrap_or_else(|| panic!("expected backend item named {name}"))
    }

    fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                hir::Item::Value(item) if item.name.text() == name => Some(item_id),
                hir::Item::Signal(item) if item.name.text() == name => Some(item_id),
                hir::Item::Function(item) if item.name.text() == name => Some(item_id),
                hir::Item::Type(item) if item.name.text() == name => Some(item_id),
                hir::Item::Class(item) if item.name.text() == name => Some(item_id),
                hir::Item::Domain(item) if item.name.text() == name => Some(item_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    fn test_sqlite_path(prefix: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-scratch");
        fs::create_dir_all(&base).expect("runtime GLib test scratch directory should exist");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        base.join(format!(
            "aivi-runtime-glib-{prefix}-{}-{unique}.sqlite",
            std::process::id()
        ))
    }

    fn seed_users_db(path: &Path, names: &[&str]) {
        let mut script =
            String::from("create table users(id integer primary key, name text not null);");
        for (index, name) in names.iter().enumerate() {
            script.push_str("insert into users(id, name) values (");
            script.push_str(&(index + 1).to_string());
            script.push_str(", '");
            script.push_str(&name.replace('\'', "''"));
            script.push_str("');");
        }
        let output = Command::new("sqlite3")
            .arg(path)
            .arg(&script)
            .output()
            .expect("sqlite3 should be available for GLib runtime tests");
        assert!(
            output.status.success(),
            "sqlite3 should seed {} successfully: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn expected_signal_text(value: &str) -> DetachedRuntimeValue {
        DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Signal(Box::new(
            RuntimeValue::Text(value.into()),
        )))
    }

    fn expected_runtime_int(value: i64) -> DetachedRuntimeValue {
        DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(value))
    }

    fn pump_context(context: &MainContext, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            thread::sleep(Duration::from_millis(10));
        }
        while context.pending() {
            context.iteration(false);
        }
    }

    fn pump_until(context: &MainContext, mut condition: impl FnMut() -> bool) {
        let _ = pump_until_with_iterations(context, &mut condition);
    }

    fn pump_until_with_iterations(
        context: &MainContext,
        mut condition: impl FnMut() -> bool,
    ) -> usize {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut iterations = 0usize;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
                iterations = iterations.saturating_add(1);
            }
            if condition() {
                return iterations;
            }
            thread::sleep(Duration::from_millis(10));
        }
        while context.pending() {
            context.iteration(false);
            iterations = iterations.saturating_add(1);
        }
        if condition() {
            return iterations;
        }
        panic!("GLib main context did not reach the expected scheduler state");
    }

    #[test]
    fn glib_driver_processes_main_thread_publications_on_the_context() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| inputs.value(0).copied(),
                );

                let stamp = driver.current_stamp(input).unwrap();
                driver
                    .queue_publication(Publication::new(stamp, 7_i32))
                    .unwrap();

                pump_until(&context, || driver.outcome_count() > 0);

                let outcomes = driver.drain_outcomes();
                assert_eq!(outcomes.len(), 1);
                assert_eq!(
                    outcomes[0].committed(),
                    &[input.as_signal(), mirror.as_signal()]
                );
                assert_eq!(driver.current_value(mirror.as_signal()).unwrap(), Some(7));
                assert_eq!(driver.tick_count(), 1);
            })
            .unwrap();
    }

    #[test]
    fn glib_driver_can_force_same_input_publications_into_separate_ticks() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| inputs.value(0).copied(),
                );

                let stamp = driver.current_stamp(input).unwrap();
                driver
                    .queue_publication_now(Publication::new(stamp, 7_i32))
                    .unwrap();
                driver
                    .queue_publication_now(Publication::new(stamp, 9_i32))
                    .unwrap();

                let outcomes = driver.drain_outcomes();
                assert_eq!(outcomes.len(), 2);
                assert_eq!(driver.current_value(mirror.as_signal()).unwrap(), Some(9));
                assert_eq!(driver.tick_count(), 2);
            })
            .unwrap();
    }

    #[test]
    fn glib_driver_processes_worker_publications_on_the_context() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| inputs.value(0).copied(),
                );

                let sender = driver.worker_sender();
                let stamp = driver.current_stamp(input).unwrap();
                thread::spawn(move || {
                    sender.publish(Publication::new(stamp, 11_i32)).unwrap();
                })
                .join()
                .unwrap();

                pump_until(&context, || driver.outcome_count() > 0);

                let outcomes = driver.drain_outcomes();
                assert_eq!(outcomes.len(), 1);
                assert_eq!(
                    outcomes[0].committed(),
                    &[input.as_signal(), mirror.as_signal()]
                );
                assert_eq!(driver.current_value(mirror.as_signal()).unwrap(), Some(11));
                assert_eq!(driver.tick_count(), 1);
            })
            .unwrap();
    }

    #[test]
    fn glib_driver_yields_long_worker_publication_chains_across_multiple_wakes() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let publish_next: Arc<Mutex<Option<Box<dyn Fn(i32) + Send + 'static>>>> =
                    Arc::new(Mutex::new(None));
                let publish_next_in_evaluator = publish_next.clone();
                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| {
                        let current = inputs.value(0).copied();
                        if let Some(current) = current
                            && let Some(callback) = publish_next_in_evaluator
                                .lock()
                                .expect("publish-next hook mutex should not be poisoned")
                                .as_ref()
                        {
                            callback(current);
                        }
                        current
                    },
                );

                let sender = driver.worker_sender();
                let stamp = driver.current_stamp(input).unwrap();
                let target = GLIB_ASYNC_WAKE_TICK_BUDGET as i32 + 8;
                *publish_next
                    .lock()
                    .expect("publish-next hook mutex should not be poisoned") =
                    Some(Box::new(move |current| {
                        if current < target {
                            sender
                                .publish(Publication::new(stamp, current + 1))
                                .expect("worker publication chain should stay live");
                        }
                    }));

                driver
                    .queue_publication(Publication::new(stamp, 0_i32))
                    .unwrap();

                let iterations = pump_until_with_iterations(&context, || {
                    driver.current_value(mirror.as_signal()).unwrap() == Some(target)
                });

                assert!(
                    iterations > 1,
                    "long worker-publication chains should yield across multiple GLib wakes"
                );
                assert_eq!(
                    driver.current_value(mirror.as_signal()).unwrap(),
                    Some(target)
                );
            })
            .unwrap();
    }

    #[test]
    fn glib_linked_runtime_processes_window_key_source_events_via_context_wakeups() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let lowered = lower_text(
                    "glib-runtime-window-key-source.aivi",
                    r#"
@source window.keyDown with {
    repeat: True
    focusOnly: True
}
signal keyDown : Signal Text
"#,
                );
                let assembly = crate::assemble_hir_runtime(lowered.hir.module())
                    .expect("runtime assembly should build");
                let linked = crate::link_backend_runtime(
                    assembly,
                    &lowered.core,
                    Arc::new(lowered.backend.clone()),
                )
                .expect("startup link should succeed");
                let driver = GlibLinkedRuntimeDriver::new(
                    context.clone(),
                    linked,
                    SourceProviderManager::new(),
                    None,
                );
                let key_item = backend_item_id(&lowered.backend, "keyDown");

                driver.tick_now();
                for name in ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"] {
                    driver.dispatch_window_key_event(name, false);
                    pump_until(&context, || {
                        driver.failure_count() > 0
                            || driver
                                .current_signal_globals()
                                .ok()
                                .and_then(|globals| globals.get(&key_item).cloned())
                                == Some(expected_signal_text(name))
                    });

                    assert_eq!(driver.failure_count(), 0);
                    let outcomes = driver.drain_outcomes();
                    assert!(
                        !outcomes.is_empty(),
                        "window key event {name} should commit at least one linked runtime tick"
                    );
                    assert_eq!(
                        driver
                            .current_signal_globals()
                            .expect("signal globals should remain readable")
                            .get(&key_item),
                        Some(&expected_signal_text(name))
                    );
                }
            })
            .unwrap();
    }

    #[test]
    fn glib_linked_runtime_current_queue_does_not_drain_async_timer_wakes() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let lowered = lower_text(
                    "glib-runtime-current-queue-only.aivi",
                    r#"
signal trigger : Signal Int

@source timer.after 1 with {
    immediate: False,
    restartOn: trigger
}
signal wake : Signal Unit

signal phase : Signal Text = wake | trigger
  ||> wake _ => "timer"
  ||> trigger _ => "trigger"
"#,
                );
                let assembly = crate::assemble_hir_runtime(lowered.hir.module())
                    .expect("runtime assembly should build");
                let trigger_input = assembly
                    .signal(item_id(lowered.hir.module(), "trigger"))
                    .expect("trigger signal binding should exist")
                    .input()
                    .expect("trigger should lower as a runtime input");
                let phase_item = backend_item_id(&lowered.backend, "phase");
                let linked = crate::link_backend_runtime(
                    assembly,
                    &lowered.core,
                    Arc::new(lowered.backend.clone()),
                )
                .expect("startup link should succeed");
                let driver = GlibLinkedRuntimeDriver::new(
                    context.clone(),
                    linked,
                    SourceProviderManager::new(),
                    None,
                );

                driver.tick_now();
                let stamp = driver.current_stamp(trigger_input).unwrap();
                driver
                    .queue_publication_now_current_queue(Publication::new(stamp, expected_runtime_int(1)))
                    .expect("current-queue publication should succeed");

                assert_eq!(
                    driver
                        .current_signal_globals()
                        .expect("signal globals should remain readable")
                        .get(&phase_item),
                    Some(&expected_signal_text("trigger")),
                    "the direct publication should settle its own queue before timer follow-up wakes run"
                );

                pump_until(&context, || {
                    driver.failure_count() > 0
                        || driver
                            .current_signal_globals()
                            .ok()
                            .and_then(|globals| globals.get(&phase_item).cloned())
                            == Some(expected_signal_text("timer"))
                });
                assert_eq!(driver.failure_count(), 0);
                assert_eq!(
                    driver
                        .current_signal_globals()
                        .expect("signal globals should stay readable after the timer wake")
                        .get(&phase_item),
                    Some(&expected_signal_text("timer"))
                );
            })
            .unwrap();
    }

    #[test]
    fn glib_linked_runtime_stop_keeps_queued_follow_up_callbacks_inert() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let lowered = lower_text(
                    "glib-runtime-stop-after-queued-follow-up.aivi",
                    r#"
signal count : Signal Int
signal mirror : Signal Int = count
"#,
                );
                let assembly = crate::assemble_hir_runtime(lowered.hir.module())
                    .expect("runtime assembly should build");
                let count_input = assembly
                    .signal(item_id(lowered.hir.module(), "count"))
                    .expect("count signal binding should exist")
                    .input()
                    .expect("count should lower as a runtime input");
                let mirror_signal = assembly
                    .signal(item_id(lowered.hir.module(), "mirror"))
                    .expect("mirror signal binding should exist")
                    .signal();
                let driver_slot: Arc<Mutex<Option<GlibLinkedRuntimeDriver>>> =
                    Arc::new(Mutex::new(None));
                let first_wake = Arc::new(Mutex::new(true));
                let notifier: Arc<dyn Fn() + Send + Sync + 'static> = {
                    let driver_slot = driver_slot.clone();
                    let first_wake = first_wake.clone();
                    Arc::new(move || {
                        let mut first = first_wake
                            .lock()
                            .expect("first-wake mutex should not be poisoned");
                        if !*first {
                            return;
                        }
                        *first = false;
                        drop(first);

                        let driver = driver_slot
                            .lock()
                            .expect("driver slot mutex should not be poisoned")
                            .as_ref()
                            .expect("driver should be installed before notifier runs")
                            .clone();
                        let stamp = driver.current_stamp(count_input).unwrap();
                        driver
                            .queue_publication(Publication::new(
                                stamp,
                                DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(2)),
                            ))
                            .expect("follow-up publication should queue before stop");
                        driver.stop();
                    })
                };
                let linked = crate::link_backend_runtime(
                    assembly,
                    &lowered.core,
                    Arc::new(lowered.backend.clone()),
                )
                .expect("startup link should succeed");
                let driver = GlibLinkedRuntimeDriver::new(
                    context.clone(),
                    linked,
                    SourceProviderManager::new(),
                    Some(notifier),
                );
                *driver_slot
                    .lock()
                    .expect("driver slot mutex should not be poisoned") = Some(driver.clone());

                let stamp = driver.current_stamp(count_input).unwrap();
                driver
                    .queue_publication(Publication::new(
                        stamp,
                        DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(1)),
                    ))
                    .expect("initial publication should queue");

                pump_until(&context, || {
                    driver.failure_count() > 0 || driver.outcome_count() > 0
                });
                pump_context(&context, Duration::from_millis(50));

                assert_eq!(driver.failure_count(), 0);
                assert_eq!(
                    driver.current_signal_value(mirror_signal).unwrap(),
                    Some(expected_runtime_int(1))
                );
                assert_eq!(
                    driver.drain_outcomes().len(),
                    1,
                    "stop should suppress the queued follow-up callback before it can commit"
                );
            })
            .unwrap();
    }

    #[test]
    fn glib_linked_runtime_refreshes_db_live_after_matching_commit() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let primary_db = test_sqlite_path("db-live-primary");
                let secondary_db = test_sqlite_path("db-live-secondary");
                seed_users_db(&primary_db, &[]);
                seed_users_db(&secondary_db, &["Bob"]);
                let lowered = lower_text(
                    "glib-runtime-db-live-commit-refresh.aivi",
                    &format!(
                        r#"
use aivi.db (paramInt, paramText, statement)

signal primaryChanged : Signal Unit
signal secondaryChanged : Signal Unit

type DatabaseHandle = {{
    database: Text
}}

type RandomHandle = Unit

value primaryConn = {{
    database: "{}"
}}

value secondaryConn = {{
    database: "{}"
}}

@source db primaryConn
signal primaryDb : DatabaseHandle

@source db secondaryConn
signal secondaryDb : DatabaseHandle

@source random
signal entropy : RandomHandle

value primaryUsers = {{
    name: "users",
    conn: primaryConn,
    changed: primaryChanged
}}

value secondaryUsers = {{
    name: "users",
    conn: secondaryConn,
    changed: secondaryChanged
}}

value samplePrimary : Task Text Bytes =
    entropy.bytes 16

value sampleSecondary : Task Text Bytes =
    entropy.bytes 16

@source db.live samplePrimary with {{
    refreshOn: primaryUsers.changed
}}
signal primaryBytes : Signal (Result Text Bytes)

@source db.live sampleSecondary with {{
    refreshOn: secondaryUsers.changed
}}
signal secondaryBytes : Signal (Result Text Bytes)

value addPrimary : Task Text Unit =
    primaryDb.commit ["users"] [
        statement "insert into users(id, name) values (?, ?)" [paramInt 1, paramText "Ada"]
    ]
"#,
                        primary_db.display(),
                        secondary_db.display(),
                    ),
                );
                let assembly = crate::assemble_hir_runtime(lowered.hir.module())
                    .expect("runtime assembly should build");
                let primary_rows_signal = assembly
                    .signal(item_id(lowered.hir.module(), "primaryBytes"))
                    .expect("primaryBytes signal binding should exist")
                    .signal();
                let primary_rows_input = assembly
                    .signal(item_id(lowered.hir.module(), "primaryBytes"))
                    .expect("primaryBytes signal binding should exist")
                    .input()
                    .expect("primaryBytes should be source-input-backed");
                let secondary_rows_signal = assembly
                    .signal(item_id(lowered.hir.module(), "secondaryBytes"))
                    .expect("secondaryBytes signal binding should exist")
                    .signal();
                let secondary_rows_input = assembly
                    .signal(item_id(lowered.hir.module(), "secondaryBytes"))
                    .expect("secondaryBytes signal binding should exist")
                    .input()
                    .expect("secondaryBytes should be source-input-backed");
                let primary_changed_input = assembly
                    .signal(item_id(lowered.hir.module(), "primaryChanged"))
                    .expect("primaryChanged signal binding should exist")
                    .input()
                    .expect("primaryChanged should be input-backed");
                let secondary_changed_input = assembly
                    .signal(item_id(lowered.hir.module(), "secondaryChanged"))
                    .expect("secondaryChanged signal binding should exist")
                    .input()
                    .expect("secondaryChanged should be input-backed");
                let primary_source_instance = assembly
                    .source_by_owner(item_id(lowered.hir.module(), "primaryBytes"))
                    .expect("primaryBytes source binding should exist")
                    .spec
                    .instance;
                let secondary_source_instance = assembly
                    .source_by_owner(item_id(lowered.hir.module(), "secondaryBytes"))
                    .expect("secondaryBytes source binding should exist")
                    .spec
                    .instance;
                let add_primary_owner = item_id(lowered.hir.module(), "addPrimary");
                let linked = crate::link_backend_runtime(
                    assembly,
                    &lowered.core,
                    Arc::new(lowered.backend.clone()),
                )
                .expect("startup link should succeed");
                let driver = GlibLinkedRuntimeDriver::new(
                    context.clone(),
                    linked,
                    SourceProviderManager::new(),
                    None,
                );

                driver.tick_now();
                pump_until(&context, || {
                    driver.failure_count() > 0
                        || (driver
                            .current_signal_value(primary_rows_signal)
                            .ok()
                            .flatten()
                            .is_some()
                            && driver
                                .current_signal_value(secondary_rows_signal)
                                .ok()
                                .flatten()
                                .is_some())
                });
                if driver.failure_count() != 0 {
                    panic!(
                        "unexpected GLib runtime failures after commit refresh: {:?}",
                        driver.drain_failures()
                    );
                }
                let primary_rows_generation = driver.current_generation(primary_rows_input).unwrap();
                let secondary_rows_generation =
                    driver.current_generation(secondary_rows_input).unwrap();
                driver.drain_outcomes();

                let primary_generation = driver.current_generation(primary_changed_input).unwrap();
                let secondary_generation =
                    driver.current_generation(secondary_changed_input).unwrap();
                let handle = driver.with_state_mut(|state| {
                    state
                        .linked
                        .spawn_task_worker_by_owner(add_primary_owner)
                        .expect("db commit task worker should spawn")
                });
                assert_eq!(
                    handle
                        .join()
                        .expect("task worker thread should join cleanly"),
                    Ok(crate::LinkedTaskWorkerOutcome::Published)
                );

                pump_until(&context, || {
                    driver.failure_count() > 0
                        || driver.current_generation(primary_rows_input).ok()
                            != Some(primary_rows_generation)
                });

                if driver.failure_count() != 0 {
                    panic!(
                        "unexpected GLib runtime failures after commit refresh: {:?}",
                        driver.drain_failures()
                    );
                }
                assert_eq!(
                    driver.current_generation(secondary_rows_input).unwrap(),
                    secondary_rows_generation
                );
                assert_ne!(
                    driver.current_generation(primary_rows_input).unwrap(),
                    primary_rows_generation
                );
                assert_ne!(
                    driver.current_generation(primary_changed_input).unwrap(),
                    primary_generation
                );
                assert_eq!(
                    driver.current_generation(secondary_changed_input).unwrap(),
                    secondary_generation
                );
                let outcomes = driver.drain_outcomes();
                assert!(
                    outcomes.iter().any(|outcome| outcome.source_actions().iter().any(|action| {
                        matches!(
                            action,
                            crate::LinkedSourceLifecycleAction::Reconfigure { instance, .. }
                                if *instance == primary_source_instance
                        )
                    })),
                    "matching db.commit invalidation should reconfigure the primary db.live source"
                );
                assert!(
                    !outcomes.iter().any(|outcome| outcome.source_actions().iter().any(|action| {
                        matches!(
                            action,
                            crate::LinkedSourceLifecycleAction::Reconfigure { instance, .. }
                                if *instance == secondary_source_instance
                        )
                    })),
                    "commit invalidation should not reconfigure db.live sources bound to other connections"
                );
                let _ = fs::remove_file(primary_db);
                let _ = fs::remove_file(secondary_db);
            })
            .unwrap();
    }

    #[test]
    fn glib_linked_runtime_skips_db_live_invalidation_after_failed_commit() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let database = test_sqlite_path("db-live-commit-failure");
                seed_users_db(&database, &[]);
                let lowered = lower_text(
                    "glib-runtime-db-live-commit-failure.aivi",
                    &format!(
                        r#"
use aivi.db (paramInt, statement)

signal usersChanged : Signal Unit

type DatabaseHandle = {{
    database: Text
}}

type RandomHandle = Unit

value conn = {{
    database: "{}"
}}

@source db conn
signal database : DatabaseHandle

@source random
signal entropy : RandomHandle

value users = {{
    name: "users",
    conn: conn,
    changed: usersChanged
}}

value sampleUsers : Task Text Bytes =
    entropy.bytes 16

@source db.live sampleUsers with {{
    refreshOn: users.changed
}}
signal rows : Signal (Result Text Bytes)

value failInsert : Task Text Unit =
    database.commit ["users"] [
        statement "insert into missing_table(id) values (?)" [paramInt 1]
    ]
"#,
                        database.display(),
                    ),
                );
                let assembly = crate::assemble_hir_runtime(lowered.hir.module())
                    .expect("runtime assembly should build");
                let rows_signal = assembly
                    .signal(item_id(lowered.hir.module(), "rows"))
                    .expect("rows signal binding should exist")
                    .signal();
                let changed_input = assembly
                    .signal(item_id(lowered.hir.module(), "usersChanged"))
                    .expect("usersChanged signal binding should exist")
                    .input()
                    .expect("usersChanged should be input-backed");
                let rows_source_instance = assembly
                    .source_by_owner(item_id(lowered.hir.module(), "rows"))
                    .expect("rows source binding should exist")
                    .spec
                    .instance;
                let fail_task_signal = assembly
                    .task_by_owner(item_id(lowered.hir.module(), "failInsert"))
                    .expect("failInsert task binding should exist")
                    .input
                    .as_signal();
                let fail_insert_owner = item_id(lowered.hir.module(), "failInsert");
                let linked = crate::link_backend_runtime(
                    assembly,
                    &lowered.core,
                    Arc::new(lowered.backend.clone()),
                )
                .expect("startup link should succeed");
                let driver = GlibLinkedRuntimeDriver::new(
                    context.clone(),
                    linked,
                    SourceProviderManager::new(),
                    None,
                );

                driver.tick_now();
                pump_until(&context, || {
                    driver.failure_count() > 0
                        || driver
                            .current_signal_value(rows_signal)
                            .ok()
                            .flatten()
                            .is_some()
                });
                assert_eq!(driver.failure_count(), 0);
                let rows_before = driver
                    .current_signal_value(rows_signal)
                    .expect("db.live source value should be readable")
                    .expect("db.live source should publish a value");
                driver.drain_outcomes();

                let changed_generation = driver.current_generation(changed_input).unwrap();
                let handle = driver.with_state_mut(|state| {
                    state
                        .linked
                        .spawn_task_worker_by_owner(fail_insert_owner)
                        .expect("failing db commit task worker should spawn")
                });
                assert_eq!(
                    handle
                        .join()
                        .expect("task worker thread should join cleanly"),
                    Ok(crate::LinkedTaskWorkerOutcome::Published)
                );
                pump_until(&context, || {
                    driver.failure_count() > 0
                        || driver
                            .current_signal_value(fail_task_signal)
                            .ok()
                            .flatten()
                            .is_some()
                });

                assert_eq!(driver.failure_count(), 0);
                assert_eq!(
                    driver
                        .current_signal_value(rows_signal)
                        .expect("db.live source value should stay readable")
                        .expect("db.live source should stay published"),
                    rows_before
                );
                assert_eq!(
                    driver.current_generation(changed_input).unwrap(),
                    changed_generation
                );
                let outcomes = driver.drain_outcomes();
                assert!(
                    !outcomes
                        .iter()
                        .any(|outcome| outcome.source_actions().iter().any(|action| {
                            matches!(
                                action,
                                crate::LinkedSourceLifecycleAction::Reconfigure { instance, .. }
                                    if *instance == rows_source_instance
                            )
                        })),
                    "failed db.commit should not reconfigure the dependent db.live source"
                );

                let _ = fs::remove_file(database);
            })
            .unwrap();
    }

    #[test]
    fn glib_driver_panics_when_evaluator_reenters_driver_api() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let reenter: Arc<Mutex<Option<Box<dyn Fn() + Send + 'static>>>> =
                    Arc::new(Mutex::new(None));
                let reenter_in_evaluator = reenter.clone();
                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| {
                        if let Some(callback) = reenter_in_evaluator
                            .lock()
                            .expect("reentry hook mutex should not be poisoned")
                            .as_ref()
                        {
                            callback();
                        }
                        inputs.value(0).copied()
                    },
                );
                let driver_for_hook = driver.clone();
                *reenter
                    .lock()
                    .expect("reentry hook mutex should not be poisoned") =
                    Some(Box::new(move || {
                        let _ = driver_for_hook.tick_count();
                    }));

                let stamp = driver.current_stamp(input).unwrap();
                let panic = catch_unwind(AssertUnwindSafe(|| {
                    driver
                        .queue_publication_now(Publication::new(stamp, 1_i32))
                        .unwrap();
                }));
                assert!(
                    panic.is_err(),
                    "reentrant evaluator access should fail fast instead of blocking on the driver"
                );
            })
            .unwrap();
    }

    #[test]
    fn glib_driver_panics_before_reentrant_access_can_deadlock() {
        let context = MainContext::new();
        context
            .with_thread_default(|| {
                let mut builder = SignalGraphBuilder::new();
                let input = builder.add_input("input", None).unwrap();
                let mirror = builder.add_derived("mirror", None).unwrap();
                builder.define_derived(mirror, [input.as_signal()]).unwrap();

                let driver = GlibSchedulerDriver::new(
                    context.clone(),
                    Scheduler::new(builder.build().unwrap()),
                    move |_signal, inputs: DependencyValues<'_, i32>| inputs.value(0).copied(),
                );

                let _guard = TickExecutionGuard::enter();
                let panic = catch_unwind(AssertUnwindSafe(|| driver.tick_count()));
                assert!(
                    panic.is_err(),
                    "reentrant driver access should fail fast instead of blocking on the mutex"
                );
            })
            .unwrap();
    }
}
