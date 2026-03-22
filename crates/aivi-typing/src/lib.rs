#![forbid(unsafe_code)]

//! Milestone 3 type-side semantics, starting with explicit kind-checking foundations and
//! compiler-derived `Eq` derivation plans.
//!
//! The crate intentionally models only the focused structural fragments needed by the current
//! implementation wave. It does not pretend full instance resolution, typed-core elaboration,
//! or runtime evaluation already exists.

pub mod eq;
pub mod fanout;
pub mod gate;
pub mod kind;
pub mod recurrence;
pub mod source_contracts;

pub use eq::{
    Class, Closedness, DomainShape, EqContext, EqDerivation, EqDerivationError,
    EqDerivationErrorKind, EqDeriver, EqFieldPlan, EqPathSegment, EqPlanId, EqStep, EqVariantPlan,
    ExternalTypeId, FieldName, InstanceHead, PrimitiveType, RecordField, RecordShape, ShapeError,
    ShapeErrorKind, SumShape, SumVariant, TypeId, TypeNode, TypeParameterId, TypeReference,
    TypeStore, VariantName,
};
pub use fanout::{FanoutCarrier, FanoutPlan, FanoutPlanner, FanoutResultKind, FanoutStageKind};
pub use gate::{GateCarrier, GatePlan, GatePlanner, GateResultKind};
pub use kind::{
    Kind, KindCheckError, KindCheckErrorKind, KindChecker, KindExpr, KindExprId, KindParameterId,
    KindRecordField, KindStore, TypeConstructorId,
};
pub use recurrence::{
    BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext, CustomSourceWakeupCause,
    NonSourceWakeupCause, RecurrencePlan, RecurrencePlanner, RecurrenceTarget,
    RecurrenceTargetError, RecurrenceTargetEvidence, RecurrenceWakeupError,
    RecurrenceWakeupEvidence, RecurrenceWakeupKind, RecurrenceWakeupPlan, RecurrenceWakeupPlanner,
    SourceRecurrenceWakeupContext, builtin_source_option_wakeup_cause,
};
pub use source_contracts::{
    BuiltinSourceProvider, SourceCancellationPolicy, SourceContract,
    SourceContractIntrinsicWakeup, SourceContractType, SourceLifecycleContract,
    SourceNominalType, SourceOptionContract, SourceOptionWakeupCause,
    SourceOptionWakeupContract, SourceRecurrenceContract, SourceTypeAtom, SourceTypeParameter,
};
