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
    Publication, PublicationStamp, Scheduler, SchedulerAccessError, SignalGraph, SignalHandle,
    SourceInstanceId, SourceProviderExecutionError, SourceProviderManager, TaskSourceRuntimeError,
    TickOutcome, WorkerPublicationSender, WorkerSendError,
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
        self.shared.drive_pending_ticks();
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

        let shared = self.clone();
        self.context.spawn(async move {
            shared.drive_pending_ticks();
        });
    }

    fn drive_pending_ticks(&self) {
        let _guard = TickExecutionGuard::enter();
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
                self.tick_enqueued.load(Ordering::Acquire) || queued_count > 0
            };

            if !should_continue {
                break;
            }
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

    pub fn source_binding(&self, instance: SourceInstanceId) -> Option<LinkedSourceBinding> {
        self.with_state(|state| state.linked.source_binding(instance).cloned())
    }

    pub fn evaluate_source_config(
        &self,
        instance: SourceInstanceId,
    ) -> Result<EvaluatedSourceConfig, GlibLinkedRuntimeAccessError> {
        self.with_state(|state| state.linked.evaluate_source_config(instance))
            .map_err(GlibLinkedRuntimeAccessError::Backend)
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
        self.shared.drive_pending_ticks();
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
            notifier,
        }
    }

    fn worker_notifier(self: &Arc<Self>) -> Arc<dyn Fn() + Send + Sync + 'static> {
        let shared = self.clone();
        Arc::new(move || shared.request_tick())
    }

    fn request_tick(self: &Arc<Self>) {
        if self.tick_enqueued.swap(true, Ordering::AcqRel) {
            return;
        }

        let shared = self.clone();
        self.context.spawn(async move {
            shared.drive_pending_ticks();
        });
    }

    fn notify_tick_ready(&self) {
        if let Some(notifier) = &self.notifier {
            notifier();
        }
    }

    fn drive_pending_ticks(&self) {
        let _guard = TickExecutionGuard::enter();
        let mut notify = false;
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
                        false
                    } else {
                        if !outcome.scheduler().is_empty() {
                            state.outcomes.push_back(outcome);
                            notify = true;
                        }
                        self.tick_enqueued.load(Ordering::Acquire)
                            || state.linked.queued_message_count() > 0
                    }
                }
                Err(error) => {
                    state
                        .failures
                        .push_back(GlibLinkedRuntimeFailure::Tick(error));
                    notify = true;
                    false
                }
            };
            *self
                .state
                .lock()
                .expect("GLib linked runtime state mutex should not be poisoned") = Some(state);

            if !should_continue {
                break;
            }
        }
        if notify {
            self.notify_tick_ready();
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
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{Arc, Mutex},
        thread,
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

    use super::{GlibLinkedRuntimeDriver, GlibSchedulerDriver, TickExecutionGuard};

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
        let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
        LoweredStack { hir, core, backend }
    }

    fn backend_item_id(program: &aivi_backend::Program, name: &str) -> BackendItemId {
        program
            .items()
            .iter()
            .find_map(|(item_id, item)| (item.name.as_ref() == name).then_some(item_id))
            .unwrap_or_else(|| panic!("expected backend item named {name}"))
    }

    fn expected_signal_text(value: &str) -> DetachedRuntimeValue {
        DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Signal(Box::new(
            RuntimeValue::Text(value.into()),
        )))
    }

    fn pump_until(context: &MainContext, mut condition: impl FnMut() -> bool) {
        for _ in 0..32 {
            if condition() {
                return;
            }
            context.iteration(true);
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
