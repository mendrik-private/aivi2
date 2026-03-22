#![forbid(unsafe_code)]

//! Milestone 2 HIR boundary with typed IDs, module-owned arenas, and structural validation.

pub mod arena;
mod fanout_elaboration;
mod gate_elaboration;
mod hir;
mod ids;
mod lower;
mod sequence;
mod source_contract_resolution;
mod validate;

pub use arena::{Arena, ArenaId, ArenaOverflow};
pub use fanout_elaboration::{
    BlockedFanoutSegment, FanoutElaborationBlocker, FanoutElaborationReport, FanoutJoinPlan,
    FanoutSegmentElaboration, FanoutSegmentOutcome, FanoutSegmentPlan, elaborate_fanouts,
};
pub use gate_elaboration::{
    BlockedGateStage, GateCoreExpr, GateCoreExprKind, GateElaborationBlocker,
    GateElaborationReport, GateRuntimeExpr, GateRuntimeExprKind, GateRuntimePipeExpr,
    GateRuntimePipeStage, GateRuntimePipeStageKind, GateRuntimeProjectionBase,
    GateRuntimeRecordField, GateRuntimeReference, GateRuntimeTextLiteral, GateRuntimeTextSegment,
    GateRuntimeUnsupportedKind, GateRuntimeUnsupportedPipeStageKind, GateStageElaboration,
    GateStageOutcome, OrdinaryGateStage, SignalGateFilter, elaborate_gates,
};
pub use hir::{
    ApplicativeCluster, ApplicativeSpine, ApplicativeSpineHead, BinaryOperator, Binding,
    BindingKind, BindingPattern, BuiltinTerm, BuiltinType, CaseControl, ClassItem, ClassMember,
    ClusterFinalizer, ClusterPresentation, ControlNode, ControlNodeKind,
    CustomSourceRecurrenceWakeup, Decorator, DecoratorCall, DecoratorPayload, DomainItem,
    DomainMember, DomainMemberKind, EachControl, EmptyControl, ExportItem, Expr, ExprKind,
    FragmentControl, FunctionItem, FunctionParameter, ImportBinding, InstanceItem, InstanceMember,
    IntegerLiteral, Item, ItemHeader, ItemKind, LiteralSuffixResolution, MarkupAttribute,
    MarkupAttributeValue, MarkupElement, MarkupNode, MarkupNodeKind, MatchControl, Module,
    ModuleArenas, Name, NameError, NamePath, NamePathError, Pattern, PatternKind, PipeExpr,
    PipeRecurrenceShapeError, PipeRecurrenceSuffix, PipeStage, PipeStageKind, ProjectionBase,
    RecordExpr, RecordExprField, RecordFieldSurface, RecordPatternField, RecurrenceWakeupDecorator,
    RecurrenceWakeupDecoratorKind, RegexLiteral, ResolutionState, RootItemError, ShowControl,
    SignalItem, SourceDecorator, SourceMetadata, SuffixedIntegerLiteral, TermReference,
    TermResolution, TextFragment, TextInterpolation, TextLiteral, TextSegment,
    TupleConstructorArity, TypeField, TypeItem, TypeItemBody, TypeKind, TypeNode, TypeParameter,
    TypeReference, TypeResolution, TypeVariant, UnaryOperator, UseItem, ValueItem, WithControl,
};
pub use ids::{
    BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
    PatternId, TypeId, TypeParameterId,
};
pub use lower::{LoweringResult, lower_module};
pub use sequence::{AtLeastTwo, NonEmpty, SequenceError};
pub use source_contract_resolution::{
    ResolvedSourceContractType, ResolvedSourceTypeConstructor, SourceContractResolutionError,
    SourceContractResolutionErrorKind, SourceContractTypeResolver,
};
pub use validate::{GateRecordField, GateType, ValidationMode, ValidationReport, validate_module};
