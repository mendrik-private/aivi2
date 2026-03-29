#![forbid(unsafe_code)]

//! First backend-facing foundations for the AIVI compiler.
//!
//! This crate consumes the validated `aivi-lambda` slice and re-expresses it as backend-owned,
//! layout-aware contracts:
//! - backend-owned items, pipelines, kernels, sources, and decode plans,
//! - explicit ABI/layout tables,
//! - explicit kernel calling conventions and captured environments,
//! - source/decode/runtime-facing plans with no remaining typed-lambda-only capture analysis,
//! - structural validation plus stable debug output,
//! - and a first Cranelift/object-code path for backend-owned scalar kernels.
//!
//! The current Cranelift slice is intentionally narrow. It consumes explicit lambda closures and
//! turns them into closed backend kernels with explicit input subjects, environment slots, layout
//! tables, and global dependencies, then lowers the subset of runtime-kernel ABI contracts that
//! are already backend-owned into real Cranelift functions and object bytes.

mod codegen;
mod gc;
mod ids;
mod kernel;
mod layout;
mod lower;
mod numeric;
mod program;
mod runtime;
mod validate;

pub use aivi_core::{Arena, ArenaId, ArenaOverflow};
pub use codegen::{CodegenError, CodegenErrors, CompiledKernel, CompiledProgram, compile_program};
pub use gc::{
    CommittedValueStore, InlineCommittedValueStore, MovingRuntimeValueStore, RuntimeGcHandle,
};
pub use ids::{
    DecodePlanId, DecodeStepId, EnvSlotId, InlineSubjectId, ItemId, KernelExprId, KernelId,
    LayoutId, PipelineId, SourceId,
};
pub use kernel::{
    AbiParameter, AbiResult, BigIntLiteral, BinaryOperator, BuiltinAppendCarrier,
    BuiltinApplicativeCarrier, BuiltinApplyCarrier, BuiltinBifunctorCarrier,
    BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier, BuiltinFoldableCarrier,
    BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject, BuiltinTerm,
    BuiltinTraversableCarrier, CallingConvention, CallingConventionKind, DecimalLiteral,
    FloatLiteral, InlinePipeCaseArm, InlinePipeConstructor, InlinePipeExpr, InlinePipePattern,
    InlinePipePatternKind, InlinePipeRecordPatternField, InlinePipeStage, InlinePipeStageKind,
    InlinePipeTruthyFalsyBranch, IntegerLiteral, Kernel, KernelExpr, KernelExprKind, KernelOrigin,
    KernelOriginKind, MapEntry, ParameterRole, ProjectionBase, RecordExprField, SubjectRef,
    SuffixedIntegerLiteral, TextLiteral, TextSegment, UnaryOperator, describe_expr_kind,
};
pub use layout::{
    AbiPassMode, Layout, LayoutKind, PrimitiveType, RecordFieldLayout, VariantLayout,
};
pub use lower::{LoweringError, LoweringErrors, lower_module};
pub use numeric::{RuntimeBigInt, RuntimeDecimal, RuntimeFloat};
pub use program::{
    DecodeExtraFieldPolicy, DecodeField, DecodeFieldRequirement, DecodeMode, DecodePlan,
    DecodeStep, DecodeStepKind, DecodeSumStrategy, DecodeVariant, DomainDecodeSurface,
    DomainDecodeSurfaceKind, FanoutCarrier, FanoutFilter, FanoutJoin, FanoutStage, GateStage, Item,
    ItemKind, NonSourceWakeup, NonSourceWakeupCause, Pipeline, PipelineOrigin, Program, Recurrence,
    RecurrenceStage, RecurrenceTarget, RecurrenceWakeupKind, SignalInfo, SourceArgumentKernel,
    SourceCancellationPolicy, SourceInstanceId, SourceOptionBinding, SourceOptionKernel,
    SourcePlan, SourceProvider, SourceReplacementPolicy, SourceStaleWorkPolicy,
    SourceTeardownPolicy, Stage, StageKind, TruthyFalsyBranch, TruthyFalsyStage,
};
pub use runtime::{
    DetachedRuntimeValue, EvaluationError, KernelEvaluator, RuntimeCallable, RuntimeConstructor,
    RuntimeMap, RuntimeMapEntry, RuntimeRecordField, RuntimeSumValue, RuntimeTaskPlan,
    RuntimeValue,
};
pub use validate::{ValidationError, ValidationErrors, validate_program};
