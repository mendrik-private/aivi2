use std::{
    cell::Cell,
    collections::VecDeque,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use glib::MainContext;

use crate::{
    DerivedNodeEvaluator, InputHandle, OwnerHandle, Publication, PublicationStamp, Scheduler,
    SchedulerAccessError, SignalHandle, TickOutcome, WorkerPublicationSender, WorkerSendError,
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
        f(&guard)
    }

    fn with_state_mut<R>(&self, f: impl FnOnce(&mut GlibSchedulerState<V, E>) -> R) -> R {
        assert_non_reentrant_driver_access();
        let mut guard = self
            .shared
            .state
            .lock()
            .expect("GLib scheduler state mutex should not be poisoned");
        f(&mut guard)
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
    state: Mutex<GlibSchedulerState<V, E>>,
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
            state: Mutex::new(GlibSchedulerState {
                scheduler,
                evaluator,
                outcomes: VecDeque::new(),
            }),
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
            let should_continue = {
                let mut state = self
                    .state
                    .lock()
                    .expect("GLib scheduler state mutex should not be poisoned");
                let GlibSchedulerState {
                    scheduler,
                    evaluator,
                    outcomes,
                } = &mut *state;
                let outcome = scheduler.tick(evaluator);
                if !outcome.is_empty() {
                    outcomes.push_back(outcome);
                }
                self.tick_enqueued.load(Ordering::Acquire) || scheduler.queued_message_count() > 0
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

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        thread,
    };

    use glib::MainContext;

    use crate::{
        graph::SignalGraphBuilder,
        scheduler::{DependencyValues, Publication, Scheduler},
    };

    use super::{GlibSchedulerDriver, TickExecutionGuard};

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
