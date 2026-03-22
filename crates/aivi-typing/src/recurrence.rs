//! Focused RFC §11.7 recurrence-target planning.
//!
//! The current compiler wave still lacks full expression typing and runtime lowering, but the
//! recurrence RFC already requires two honest questions to be answered before scheduler lowering:
//! which built-in runtime family a recurrent pipe is allowed to target, and whether the current
//! compiler can already prove an explicit wakeup source. This module keeps both decisions closed,
//! typed, and independently testable without pretending the later layers already exist.

use std::{error::Error, fmt};

use crate::source_contracts::BuiltinSourceProvider;

/// Closed lowering targets that RFC §11.7 currently permits for recurrent pipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceTarget {
    Signal,
    Task,
    SourceHelper,
}

/// Explicit evidence the current frontend can carry forward into recurrence planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceTargetEvidence {
    SignalItemBody,
    ExplicitSignalAnnotation,
    ExplicitTaskAnnotation,
    SourceHelper,
}

impl RecurrenceTargetEvidence {
    pub const fn target(self) -> RecurrenceTarget {
        match self {
            Self::SignalItemBody | Self::ExplicitSignalAnnotation => RecurrenceTarget::Signal,
            Self::ExplicitTaskAnnotation => RecurrenceTarget::Task,
            Self::SourceHelper => RecurrenceTarget::SourceHelper,
        }
    }
}

/// Focused recurrence-lowering decision chosen before wakeup and scheduler checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecurrencePlan {
    target: RecurrenceTarget,
    evidence: RecurrenceTargetEvidence,
}

impl RecurrencePlan {
    pub const fn from_evidence(evidence: RecurrenceTargetEvidence) -> Self {
        Self {
            target: evidence.target(),
            evidence,
        }
    }

    pub const fn target(self) -> RecurrenceTarget {
        self.target
    }

    pub const fn evidence(self) -> RecurrenceTargetEvidence {
        self.evidence
    }
}

/// Focused planning error for recurrent pipes whose lowering target is still unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceTargetError {
    MissingExplicitTarget,
}

impl fmt::Display for RecurrenceTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExplicitTarget => f.write_str(
                "the compiler cannot yet determine an explicit recurrence lowering target",
            ),
        }
    }
}

impl Error for RecurrenceTargetError {}

/// Closed explicit wakeup classes that RFC §11.7 currently permits for recurrent pipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceWakeupKind {
    Timer,
    Backoff,
    SourceEvent,
    ProviderDefinedTrigger,
}

/// Built-in source-side proof the current compiler can carry into recurrence wakeup planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinSourceWakeupCause {
    ProviderTimer,
    RetryPolicy,
    PollingPolicy,
    TriggerSignal,
    ReactiveInputs,
    ProviderDefinedTrigger,
}

impl BuiltinSourceWakeupCause {
    pub const fn kind(self) -> RecurrenceWakeupKind {
        match self {
            Self::ProviderTimer | Self::PollingPolicy => RecurrenceWakeupKind::Timer,
            Self::RetryPolicy => RecurrenceWakeupKind::Backoff,
            Self::TriggerSignal | Self::ReactiveInputs => RecurrenceWakeupKind::SourceEvent,
            Self::ProviderDefinedTrigger => RecurrenceWakeupKind::ProviderDefinedTrigger,
        }
    }
}

/// Custom source-side proof the current compiler can carry into recurrence wakeup planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CustomSourceWakeupCause {
    ReactiveInputs,
    DeclaredWakeup(RecurrenceWakeupKind),
}

impl CustomSourceWakeupCause {
    pub const fn kind(self) -> RecurrenceWakeupKind {
        match self {
            Self::ReactiveInputs => RecurrenceWakeupKind::SourceEvent,
            Self::DeclaredWakeup(kind) => kind,
        }
    }
}

/// Explicit non-source wakeup proofs the current frontend can carry today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NonSourceWakeupCause {
    ExplicitTimer,
    ExplicitBackoff,
}

impl NonSourceWakeupCause {
    pub const fn kind(self) -> RecurrenceWakeupKind {
        match self {
            Self::ExplicitTimer => RecurrenceWakeupKind::Timer,
            Self::ExplicitBackoff => RecurrenceWakeupKind::Backoff,
        }
    }
}

/// Explicit wakeup evidence the current frontend can already prove for recurrent pipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceWakeupEvidence {
    BuiltinSource {
        provider: BuiltinSourceProvider,
        cause: BuiltinSourceWakeupCause,
    },
    CustomSource {
        cause: CustomSourceWakeupCause,
    },
    NonSource { cause: NonSourceWakeupCause },
}

impl RecurrenceWakeupEvidence {
    pub const fn kind(self) -> RecurrenceWakeupKind {
        match self {
            Self::BuiltinSource { cause, .. } => cause.kind(),
            Self::CustomSource { cause } => cause.kind(),
            Self::NonSource { cause } => cause.kind(),
        }
    }

    pub const fn provider(self) -> Option<BuiltinSourceProvider> {
        match self {
            Self::BuiltinSource { provider, .. } => Some(provider),
            Self::CustomSource { .. } | Self::NonSource { .. } => None,
        }
    }

    pub const fn builtin_source_cause(self) -> Option<BuiltinSourceWakeupCause> {
        match self {
            Self::BuiltinSource { cause, .. } => Some(cause),
            Self::CustomSource { .. } | Self::NonSource { .. } => None,
        }
    }

    pub const fn custom_source_cause(self) -> Option<CustomSourceWakeupCause> {
        match self {
            Self::BuiltinSource { .. } | Self::NonSource { .. } => None,
            Self::CustomSource { cause } => Some(cause),
        }
    }

    pub const fn non_source_cause(self) -> Option<NonSourceWakeupCause> {
        match self {
            Self::BuiltinSource { .. } | Self::CustomSource { .. } => None,
            Self::NonSource { cause } => Some(cause),
        }
    }
}

/// Focused wakeup proof chosen before recurrence node lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecurrenceWakeupPlan {
    kind: RecurrenceWakeupKind,
    evidence: RecurrenceWakeupEvidence,
}

impl RecurrenceWakeupPlan {
    pub const fn from_evidence(evidence: RecurrenceWakeupEvidence) -> Self {
        Self {
            kind: evidence.kind(),
            evidence,
        }
    }

    pub const fn kind(self) -> RecurrenceWakeupKind {
        self.kind
    }

    pub const fn evidence(self) -> RecurrenceWakeupEvidence {
        self.evidence
    }
}

/// Explicit source-backed wakeup surface that resolved HIR can prove today.
///
/// This deliberately records only the closed wakeup-related facts the compiler already knows
/// structurally: which built-in provider is selected, whether named wakeup policy slots such as
/// `retry` / `refreshEvery` / `refreshOn` are present, and whether source arguments or options are
/// reactive. The actual option-expression types still belong to later source typing work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceRecurrenceWakeupContext {
    provider: BuiltinSourceProvider,
    has_retry_policy: bool,
    has_polling_policy: bool,
    has_signal_trigger: bool,
    has_reactive_inputs: bool,
}

impl SourceRecurrenceWakeupContext {
    pub const fn new(provider: BuiltinSourceProvider) -> Self {
        Self {
            provider,
            has_retry_policy: false,
            has_polling_policy: false,
            has_signal_trigger: false,
            has_reactive_inputs: false,
        }
    }

    pub const fn provider(self) -> BuiltinSourceProvider {
        self.provider
    }

    pub const fn has_retry_policy(self) -> bool {
        self.has_retry_policy
    }

    pub const fn has_polling_policy(self) -> bool {
        self.has_polling_policy
    }

    pub const fn has_signal_trigger(self) -> bool {
        self.has_signal_trigger
    }

    pub const fn has_reactive_inputs(self) -> bool {
        self.has_reactive_inputs
    }

    pub const fn with_retry_policy(mut self) -> Self {
        self.has_retry_policy = true;
        self
    }

    pub const fn with_polling_policy(mut self) -> Self {
        self.has_polling_policy = true;
        self
    }

    pub const fn with_signal_trigger(mut self) -> Self {
        self.has_signal_trigger = true;
        self
    }

    pub const fn with_reactive_inputs(mut self) -> Self {
        self.has_reactive_inputs = true;
        self
    }
}

/// Focused custom-source wakeup surface that resolved HIR can prove today.
///
/// Reactive source arguments/options are provider-independent RFC semantics, so they already prove
/// source-event wakeups even for custom providers. Any stronger proof must arrive through explicit
/// provider metadata; the current surface syntax does not populate that hook yet, but later source
/// contract work can do so without reshaping recurrence planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CustomSourceRecurrenceWakeupContext {
    declared_wakeup: Option<RecurrenceWakeupKind>,
    has_reactive_inputs: bool,
}

impl CustomSourceRecurrenceWakeupContext {
    pub const fn new() -> Self {
        Self {
            declared_wakeup: None,
            has_reactive_inputs: false,
        }
    }

    pub const fn declared_wakeup(self) -> Option<RecurrenceWakeupKind> {
        self.declared_wakeup
    }

    pub const fn has_reactive_inputs(self) -> bool {
        self.has_reactive_inputs
    }

    pub const fn with_declared_wakeup(mut self, kind: RecurrenceWakeupKind) -> Self {
        self.declared_wakeup = Some(kind);
        self
    }

    pub const fn with_reactive_inputs(mut self) -> Self {
        self.has_reactive_inputs = true;
        self
    }
}

/// Focused planning error for recurrent pipes whose explicit wakeup source is still unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceWakeupError {
    MissingExplicitWakeup,
}

impl fmt::Display for RecurrenceWakeupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExplicitWakeup => {
                f.write_str("the compiler cannot yet prove an explicit recurrence wakeup")
            }
        }
    }
}

impl Error for RecurrenceWakeupError {}

/// Stateless entry point used by HIR validation today and typed elaboration later.
pub struct RecurrencePlanner;

impl RecurrencePlanner {
    pub const fn plan(
        evidence: Option<RecurrenceTargetEvidence>,
    ) -> Result<RecurrencePlan, RecurrenceTargetError> {
        match evidence {
            Some(evidence) => Ok(RecurrencePlan::from_evidence(evidence)),
            None => Err(RecurrenceTargetError::MissingExplicitTarget),
        }
    }
}

/// Stateless entry point used by resolved-HIR validation today and scheduler lowering later.
pub struct RecurrenceWakeupPlanner;

impl RecurrenceWakeupPlanner {
    pub const fn plan_non_source(
        cause: NonSourceWakeupCause,
    ) -> Result<RecurrenceWakeupPlan, RecurrenceWakeupError> {
        Ok(RecurrenceWakeupPlan::from_evidence(
            RecurrenceWakeupEvidence::NonSource { cause },
        ))
    }

    pub const fn plan_source(
        context: SourceRecurrenceWakeupContext,
    ) -> Result<RecurrenceWakeupPlan, RecurrenceWakeupError> {
        if let Some(cause) = provider_wakeup_cause(context.provider()) {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::BuiltinSource {
                    provider: context.provider(),
                    cause,
                },
            ));
        }
        if context.has_polling_policy() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::BuiltinSource {
                    provider: context.provider(),
                    cause: BuiltinSourceWakeupCause::PollingPolicy,
                },
            ));
        }
        if context.has_retry_policy() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::BuiltinSource {
                    provider: context.provider(),
                    cause: BuiltinSourceWakeupCause::RetryPolicy,
                },
            ));
        }
        if context.has_signal_trigger() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::BuiltinSource {
                    provider: context.provider(),
                    cause: BuiltinSourceWakeupCause::TriggerSignal,
                },
            ));
        }
        if context.has_reactive_inputs() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::BuiltinSource {
                    provider: context.provider(),
                    cause: BuiltinSourceWakeupCause::ReactiveInputs,
                },
            ));
        }
        Err(RecurrenceWakeupError::MissingExplicitWakeup)
    }

    pub const fn plan_custom_source(
        context: CustomSourceRecurrenceWakeupContext,
    ) -> Result<RecurrenceWakeupPlan, RecurrenceWakeupError> {
        if let Some(kind) = context.declared_wakeup() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::CustomSource {
                    cause: CustomSourceWakeupCause::DeclaredWakeup(kind),
                },
            ));
        }
        if context.has_reactive_inputs() {
            return Ok(RecurrenceWakeupPlan::from_evidence(
                RecurrenceWakeupEvidence::CustomSource {
                    cause: CustomSourceWakeupCause::ReactiveInputs,
                },
            ));
        }
        Err(RecurrenceWakeupError::MissingExplicitWakeup)
    }
}

const fn provider_wakeup_cause(
    provider: BuiltinSourceProvider,
) -> Option<BuiltinSourceWakeupCause> {
    match provider {
        BuiltinSourceProvider::TimerEvery | BuiltinSourceProvider::TimerAfter => {
            Some(BuiltinSourceWakeupCause::ProviderTimer)
        }
        BuiltinSourceProvider::FsWatch
        | BuiltinSourceProvider::SocketConnect
        | BuiltinSourceProvider::MailboxSubscribe
        | BuiltinSourceProvider::ProcessSpawn
        | BuiltinSourceProvider::WindowKeyDown => {
            Some(BuiltinSourceWakeupCause::ProviderDefinedTrigger)
        }
        BuiltinSourceProvider::HttpGet
        | BuiltinSourceProvider::HttpPost
        | BuiltinSourceProvider::FsRead => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext, CustomSourceWakeupCause,
        NonSourceWakeupCause, RecurrencePlan, RecurrencePlanner, RecurrenceTarget,
        RecurrenceTargetError, RecurrenceTargetEvidence, RecurrenceWakeupError,
        RecurrenceWakeupKind, RecurrenceWakeupPlanner, SourceRecurrenceWakeupContext,
    };
    use crate::BuiltinSourceProvider;

    #[test]
    fn signal_item_bodies_plan_signal_targets() {
        let plan = RecurrencePlanner::plan(Some(RecurrenceTargetEvidence::SignalItemBody))
            .expect("signal bodies should provide an explicit recurrence target");
        assert_eq!(
            plan,
            RecurrencePlan::from_evidence(RecurrenceTargetEvidence::SignalItemBody)
        );
        assert_eq!(plan.target(), RecurrenceTarget::Signal);
        assert_eq!(plan.evidence(), RecurrenceTargetEvidence::SignalItemBody);
    }

    #[test]
    fn explicit_signal_annotations_plan_signal_targets() {
        let plan =
            RecurrencePlanner::plan(Some(RecurrenceTargetEvidence::ExplicitSignalAnnotation))
                .expect("explicit Signal annotations should provide an explicit recurrence target");
        assert_eq!(
            plan.target(),
            RecurrenceTargetEvidence::ExplicitSignalAnnotation.target()
        );
        assert_eq!(
            plan.evidence(),
            RecurrenceTargetEvidence::ExplicitSignalAnnotation
        );
    }

    #[test]
    fn explicit_task_annotations_plan_task_targets() {
        let plan = RecurrencePlanner::plan(Some(RecurrenceTargetEvidence::ExplicitTaskAnnotation))
            .expect("explicit Task annotations should provide an explicit recurrence target");
        assert_eq!(plan.target(), RecurrenceTarget::Task);
        assert_eq!(
            plan.evidence(),
            RecurrenceTargetEvidence::ExplicitTaskAnnotation
        );
    }

    #[test]
    fn source_helpers_plan_source_targets() {
        let plan = RecurrencePlanner::plan(Some(RecurrenceTargetEvidence::SourceHelper))
            .expect("source helpers should provide an explicit recurrence target");
        assert_eq!(plan.target(), RecurrenceTarget::SourceHelper);
        assert_eq!(plan.evidence(), RecurrenceTargetEvidence::SourceHelper);
    }

    #[test]
    fn missing_explicit_target_is_rejected() {
        assert_eq!(
            RecurrencePlanner::plan(None),
            Err(RecurrenceTargetError::MissingExplicitTarget)
        );
    }

    #[test]
    fn timer_sources_plan_timer_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(SourceRecurrenceWakeupContext::new(
            BuiltinSourceProvider::TimerEvery,
        ))
        .expect("timer providers should prove explicit recurrence wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::Timer);
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::ProviderTimer)
        );
    }

    #[test]
    fn request_retry_policies_plan_backoff_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(
            SourceRecurrenceWakeupContext::new(BuiltinSourceProvider::HttpGet).with_retry_policy(),
        )
        .expect("request retries should prove explicit backoff wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::Backoff);
        assert_eq!(
            plan.evidence().provider(),
            Some(BuiltinSourceProvider::HttpGet)
        );
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::RetryPolicy)
        );
    }

    #[test]
    fn request_polling_policies_plan_timer_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(
            SourceRecurrenceWakeupContext::new(BuiltinSourceProvider::HttpPost)
                .with_polling_policy(),
        )
        .expect("request polling should prove explicit timer wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::Timer);
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::PollingPolicy)
        );
    }

    #[test]
    fn signal_trigger_options_plan_source_event_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(
            SourceRecurrenceWakeupContext::new(BuiltinSourceProvider::FsRead).with_signal_trigger(),
        )
        .expect("explicit source trigger slots should prove source-event wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::SourceEvent);
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::TriggerSignal)
        );
    }

    #[test]
    fn reactive_source_inputs_plan_source_event_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(
            SourceRecurrenceWakeupContext::new(BuiltinSourceProvider::HttpGet)
                .with_reactive_inputs(),
        )
        .expect("reactive source inputs should prove source-event wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::SourceEvent);
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::ReactiveInputs)
        );
    }

    #[test]
    fn provider_defined_sources_plan_provider_trigger_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_source(SourceRecurrenceWakeupContext::new(
            BuiltinSourceProvider::FsWatch,
        ))
        .expect("event-style source providers should prove provider-defined wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::ProviderDefinedTrigger);
        assert_eq!(
            plan.evidence().builtin_source_cause(),
            Some(BuiltinSourceWakeupCause::ProviderDefinedTrigger)
        );
    }

    #[test]
    fn request_like_sources_without_explicit_trigger_are_rejected() {
        assert_eq!(
            RecurrenceWakeupPlanner::plan_source(SourceRecurrenceWakeupContext::new(
                BuiltinSourceProvider::HttpGet,
            )),
            Err(RecurrenceWakeupError::MissingExplicitWakeup)
        );
        assert_eq!(
            RecurrenceWakeupPlanner::plan_source(SourceRecurrenceWakeupContext::new(
                BuiltinSourceProvider::FsRead,
            )),
            Err(RecurrenceWakeupError::MissingExplicitWakeup)
        );
    }

    #[test]
    fn custom_source_reactive_inputs_plan_source_event_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_custom_source(
            CustomSourceRecurrenceWakeupContext::new().with_reactive_inputs(),
        )
        .expect("reactive custom source inputs should prove explicit source-event wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::SourceEvent);
        assert_eq!(
            plan.evidence().custom_source_cause(),
            Some(CustomSourceWakeupCause::ReactiveInputs)
        );
        assert_eq!(plan.evidence().provider(), None);
        assert_eq!(plan.evidence().builtin_source_cause(), None);
    }

    #[test]
    fn custom_source_declared_wakeups_plan_provider_triggers() {
        let plan = RecurrenceWakeupPlanner::plan_custom_source(
            CustomSourceRecurrenceWakeupContext::new()
                .with_declared_wakeup(RecurrenceWakeupKind::ProviderDefinedTrigger),
        )
        .expect("declared custom wakeup metadata should plan explicit provider triggers");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::ProviderDefinedTrigger);
        assert_eq!(
            plan.evidence().custom_source_cause(),
            Some(CustomSourceWakeupCause::DeclaredWakeup(
                RecurrenceWakeupKind::ProviderDefinedTrigger
            ))
        );
    }

    #[test]
    fn custom_sources_without_explicit_trigger_are_rejected() {
        assert_eq!(
            RecurrenceWakeupPlanner::plan_custom_source(CustomSourceRecurrenceWakeupContext::new()),
            Err(RecurrenceWakeupError::MissingExplicitWakeup)
        );
    }

    #[test]
    fn explicit_non_source_timer_witnesses_plan_timer_wakeups() {
        let plan = RecurrenceWakeupPlanner::plan_non_source(NonSourceWakeupCause::ExplicitTimer)
            .expect("explicit timer witnesses should prove non-source timer wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::Timer);
        assert_eq!(
            plan.evidence().non_source_cause(),
            Some(NonSourceWakeupCause::ExplicitTimer)
        );
        assert_eq!(plan.evidence().provider(), None);
        assert_eq!(plan.evidence().builtin_source_cause(), None);
    }

    #[test]
    fn explicit_non_source_backoff_witnesses_plan_backoff_wakeups() {
        let plan =
            RecurrenceWakeupPlanner::plan_non_source(NonSourceWakeupCause::ExplicitBackoff)
                .expect("explicit backoff witnesses should prove non-source backoff wakeups");
        assert_eq!(plan.kind(), RecurrenceWakeupKind::Backoff);
        assert_eq!(
            plan.evidence().non_source_cause(),
            Some(NonSourceWakeupCause::ExplicitBackoff)
        );
        assert_eq!(plan.evidence().provider(), None);
        assert_eq!(plan.evidence().builtin_source_cause(), None);
    }
}
