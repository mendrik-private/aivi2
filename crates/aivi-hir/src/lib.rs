#![forbid(unsafe_code)]

//! Milestone 2 HIR boundary with typed IDs, module-owned arenas, and structural validation.

pub mod arena;
mod decode_elaboration;
mod fanout_elaboration;
mod gate_elaboration;
mod hir;
mod ids;
mod lower;
mod recurrence_elaboration;
mod sequence;
mod source_contract_resolution;
mod source_lifecycle_elaboration;
mod truthy_falsy_elaboration;
mod validate;

pub use arena::{Arena, ArenaId, ArenaOverflow};
pub use decode_elaboration::{
    elaborate_source_decodes, BlockedSourceDecodeNode, SourceDecodeElaborationBlocker,
    SourceDecodeElaborationReport, SourceDecodeNodeElaboration, SourceDecodeNodeOutcome,
    SourceDecodePlan, SourceDecodeUnsupportedTypeKind,
};
pub use fanout_elaboration::{
    elaborate_fanouts, BlockedFanoutSegment, FanoutElaborationBlocker, FanoutElaborationReport,
    FanoutJoinPlan, FanoutSegmentElaboration, FanoutSegmentOutcome, FanoutSegmentPlan,
};
pub use gate_elaboration::{
    elaborate_gates, BlockedGateStage, GateCoreExpr, GateCoreExprKind, GateElaborationBlocker,
    GateElaborationReport, GateRuntimeExpr, GateRuntimeExprKind, GateRuntimePipeExpr,
    GateRuntimePipeStage, GateRuntimePipeStageKind, GateRuntimeProjectionBase,
    GateRuntimeRecordField, GateRuntimeReference, GateRuntimeTextLiteral, GateRuntimeTextSegment,
    GateRuntimeUnsupportedKind, GateRuntimeUnsupportedPipeStageKind, GateStageElaboration,
    GateStageOutcome, OrdinaryGateStage, SignalGateFilter,
};
pub use hir::{
    ApplicativeCluster, ApplicativeSpine, ApplicativeSpineHead, BinaryOperator, Binding,
    BindingKind, BindingPattern, BuiltinTerm, BuiltinType, CaseControl, ClassItem, ClassMember,
    ClusterFinalizer, ClusterPresentation, ControlNode, ControlNodeKind,
    CustomSourceArgumentSchema, CustomSourceContractMetadata, CustomSourceOptionSchema,
    CustomSourceRecurrenceWakeup, Decorator, DecoratorCall, DecoratorPayload, DomainItem,
    DomainMember, DomainMemberKind, EachControl, EmptyControl, ExportItem, Expr, ExprKind,
    FragmentControl, FunctionItem, FunctionParameter, ImportBinding, ImportBindingMetadata,
    ImportBundleKind, ImportRecordField, ImportValueType, InstanceItem, InstanceMember,
    IntegerLiteral, Item, ItemHeader, ItemKind, LiteralSuffixResolution, MarkupAttribute,
    MarkupAttributeValue, MarkupElement, MarkupNode, MarkupNodeKind, MatchControl, Module,
    ModuleArenas, Name, NameError, NamePath, NamePathError, Pattern, PatternKind, PipeExpr,
    PipeRecurrenceShapeError, PipeRecurrenceSuffix, PipeStage, PipeStageKind, ProjectionBase,
    RecordExpr, RecordExprField, RecordFieldSurface, RecordPatternField, RecurrenceWakeupDecorator,
    RecurrenceWakeupDecoratorKind, RegexLiteral, ResolutionState, RootItemError, ShowControl,
    SignalItem, SourceDecorator, SourceLifecycleDependencies, SourceMetadata,
    SourceProviderContractItem, SourceProviderRef, SuffixedIntegerLiteral, TermReference,
    TermResolution, TextFragment, TextInterpolation, TextLiteral, TextSegment,
    TupleConstructorArity, TypeField, TypeItem, TypeItemBody, TypeKind, TypeNode, TypeParameter,
    TypeReference, TypeResolution, TypeVariant, UnaryOperator, UseItem, ValueItem, WithControl,
};
pub use ids::{
    BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
    PatternId, TypeId, TypeParameterId,
};
pub use lower::{lower_module, LoweringResult};
pub use recurrence_elaboration::{
    elaborate_recurrences, BlockedRecurrenceNode, RecurrenceElaborationBlocker,
    RecurrenceElaborationReport, RecurrenceNodeElaboration, RecurrenceNodeOutcome,
    RecurrenceNodePlan, RecurrenceNonSourceWakeupBinding, RecurrenceRuntimeExpr,
    RecurrenceRuntimeStageBlocker, RecurrenceStagePlan,
};
pub use sequence::{AtLeastTwo, NonEmpty, SequenceError};
pub use source_contract_resolution::{
    ResolvedSourceContractType, ResolvedSourceTypeConstructor, SourceContractResolutionError,
    SourceContractResolutionErrorKind, SourceContractTypeResolver,
};
pub use source_lifecycle_elaboration::{
    elaborate_source_lifecycles, BlockedSourceLifecycleNode, SourceInstanceId,
    SourceLifecycleElaborationBlocker, SourceLifecycleElaborationReport,
    SourceLifecycleNodeElaboration, SourceLifecycleNodeOutcome, SourceLifecyclePlan,
    SourceOptionSignalBinding, SourceReplacementPolicy, SourceStaleWorkPolicy,
    SourceTeardownPolicy,
};
pub use truthy_falsy_elaboration::{
    elaborate_truthy_falsy, BlockedTruthyFalsyStage, TruthyFalsyBranchKind, TruthyFalsyBranchPlan,
    TruthyFalsyElaborationBlocker, TruthyFalsyElaborationReport, TruthyFalsyStageElaboration,
    TruthyFalsyStageOutcome, TruthyFalsyStagePlan,
};
pub use validate::{validate_module, GateRecordField, GateType, ValidationMode, ValidationReport};
