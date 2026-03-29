#![forbid(unsafe_code)]

//! Milestone 2 HIR boundary with typed IDs, module-owned arenas, and structural validation.

pub mod arena;
mod decode_elaboration;
mod decode_generation;
mod domain_operator_elaboration;
pub mod exports;
mod fanout_elaboration;
mod gate_elaboration;
mod general_expr_elaboration;
mod hir;
mod ids;
mod lower;
mod recurrence_elaboration;
pub mod resolver;
mod sequence;
mod signal_metadata_elaboration;
mod source_contract_resolution;
mod source_lifecycle_elaboration;
pub mod symbols;
mod truthy_falsy_elaboration;
mod typecheck;
mod validate;

pub use arena::{Arena, ArenaId, ArenaOverflow};
pub use decode_elaboration::{
    BlockedSourceDecodeNode, SourceDecodeDomainBinding, SourceDecodeElaborationBlocker,
    SourceDecodeElaborationReport, SourceDecodeNodeElaboration, SourceDecodeNodeOutcome,
    SourceDecodePlan, SourceDecodeSumBinding, SourceDecodeUnsupportedTypeKind,
    elaborate_source_decodes,
};
pub use decode_generation::{
    BlockedSourceDecodeProgram, DecodeProgramField, DecodeProgramStep, DecodeProgramStepId,
    DecodeProgramVariant, DomainDecodeSurfaceCandidate, DomainDecodeSurfaceKind,
    DomainDecodeSurfacePlan, SourceDecodeProgram, SourceDecodeProgramBlocker,
    SourceDecodeProgramNode, SourceDecodeProgramOutcome, SourceDecodeProgramReport,
    generate_source_decode_programs,
};
pub use exports::{ExportedName, ExportedNameKind, ExportedNames, exports};
pub use fanout_elaboration::{
    BlockedFanoutSegment, FanoutElaborationBlocker, FanoutElaborationReport, FanoutFilterBlocker,
    FanoutFilterPlan, FanoutJoinPlan, FanoutSegmentElaboration, FanoutSegmentOutcome,
    FanoutSegmentPlan, elaborate_fanouts,
};
pub use gate_elaboration::{
    BlockedGateStage, GateCoreExpr, GateCoreExprKind, GateElaborationBlocker,
    GateElaborationReport, GateRuntimeCaseArm, GateRuntimeExpr, GateRuntimeExprKind,
    GateRuntimePipeExpr, GateRuntimePipeStage, GateRuntimePipeStageKind, GateRuntimeProjectionBase,
    GateRuntimeRecordField, GateRuntimeReference, GateRuntimeTextLiteral, GateRuntimeTextSegment,
    GateRuntimeTruthyFalsyBranch, GateRuntimeUnsupportedKind, GateRuntimeUnsupportedPipeStageKind,
    GateStageElaboration, GateStageOutcome, OrdinaryGateStage, SignalGateFilter, elaborate_gates,
};
pub use general_expr_elaboration::{
    BlockedGeneralExpr, GeneralExprBlocker, GeneralExprElaborationReport,
    GeneralExprInstanceMemberElaboration, GeneralExprItemElaboration, GeneralExprOutcome,
    GeneralExprParameter, MarkupRuntimeExprSite, MarkupRuntimeExprSiteError,
    MarkupRuntimeExprSites, collect_markup_runtime_expr_sites, elaborate_general_expressions,
    elaborate_runtime_expr_with_env,
};
pub use hir::{
    ApplicativeCluster, ApplicativeSpine, ApplicativeSpineHead, BigIntLiteral, BinaryOperator,
    Binding, BindingKind, BindingPattern, BuiltinTerm, BuiltinType, CaseControl, ClassItem,
    ClassMember, ClassMemberResolution, ClusterFinalizer, ClusterPresentation, ControlNode,
    ControlNodeKind, CustomSourceArgumentSchema, CustomSourceContractMetadata,
    CustomSourceOptionSchema, CustomSourceRecurrenceWakeup, DebugDecorator, DecimalLiteral,
    Decorator, DecoratorCall, DecoratorPayload, DeprecatedDecorator, DeprecationNotice, DomainItem,
    DomainMember, DomainMemberHandle, DomainMemberKind, DomainMemberResolution, EachControl,
    EmptyControl, ExportItem, ExportResolution, Expr, ExprKind, FloatLiteral, FragmentControl,
    FunctionItem, FunctionParameter, ImportBinding, ImportBindingMetadata, ImportBindingResolution,
    ImportBundleKind, ImportRecordField, ImportValueType, InstanceItem, InstanceMember,
    IntegerLiteral, IntrinsicValue, Item, ItemHeader, ItemKind, LiteralSuffixResolution, MapExpr,
    MapExprEntry, MarkupAttribute, MarkupAttributeValue, MarkupElement, MarkupNode, MarkupNodeKind,
    MatchControl, MockDecorator, Module, ModuleArenas, Name, NameError, NamePath, NamePathError,
    PatchBlock, PatchEntry, PatchInstruction, PatchInstructionKind, PatchSelector,
    PatchSelectorSegment, Pattern, PatternKind, PipeExpr, PipeFanoutSegment,
    PipeRecurrenceShapeError, PipeRecurrenceSuffix, PipeStage, PipeStageKind, PipeTransformMode,
    ProjectionBase, ReactiveUpdateClause, RecordExpr, RecordExprField, RecordFieldSurface,
    RecordPatternField, RecordRowRename, RecordRowTransform, RecurrenceWakeupDecorator,
    RecurrenceWakeupDecoratorKind, RegexLiteral, ResolutionState, RootItemError, ShowControl,
    SignalItem, SourceDecorator, SourceLifecycleDependencies, SourceMetadata,
    SourceProviderContractItem, SourceProviderRef, SuffixedIntegerLiteral, SumConstructorHandle,
    TermReference, TermResolution, TestDecorator, TextFragment, TextInterpolation, TextLiteral,
    TextSegment, TupleConstructorArity, TypeField, TypeItem, TypeItemBody, TypeKind, TypeNode,
    TypeParameter, TypeReference, TypeResolution, TypeVariant, UnaryOperator, UseItem, ValueItem,
    WithControl,
};
pub use ids::{
    BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
    PatternId, TypeId, TypeParameterId,
};
pub use lower::lower_module_with_resolver;
pub use lower::{LoweringResult, lower_module, lower_structure, resolve_imports};
pub use recurrence_elaboration::{
    BlockedRecurrenceNode, RecurrenceElaborationBlocker, RecurrenceElaborationReport,
    RecurrenceGuardPlan, RecurrenceNodeElaboration, RecurrenceNodeOutcome, RecurrenceNodePlan,
    RecurrenceNonSourceWakeupBinding, RecurrenceRuntimeExpr, RecurrenceRuntimeStageBlocker,
    RecurrenceStagePlan, elaborate_recurrences,
};
pub use resolver::{ImportCycle, ImportModuleResolution, ImportResolver, NullImportResolver};
pub use sequence::{AtLeastTwo, NonEmpty, SequenceError};
pub use signal_metadata_elaboration::{
    collect_signal_dependencies_for_expr, collect_signal_dependencies_for_exprs,
    populate_signal_metadata,
};
pub use source_contract_resolution::{
    ResolvedSourceContractType, ResolvedSourceTypeConstructor, SourceContractResolutionError,
    SourceContractResolutionErrorKind, SourceContractTypeResolver,
};
pub use source_lifecycle_elaboration::{
    BlockedSourceLifecycleNode, SourceArgumentValueBinding, SourceInstanceId,
    SourceLifecycleElaborationBlocker, SourceLifecycleElaborationReport,
    SourceLifecycleNodeElaboration, SourceLifecycleNodeOutcome, SourceLifecyclePlan,
    SourceOptionSignalBinding, SourceOptionValueBinding, SourceReplacementPolicy,
    SourceStaleWorkPolicy, SourceTeardownPolicy, elaborate_source_lifecycles,
};
pub use symbols::{LspSymbol, LspSymbolKind, extract_symbols};
pub use truthy_falsy_elaboration::{
    BlockedTruthyFalsyStage, TruthyFalsyBranchKind, TruthyFalsyBranchPlan,
    TruthyFalsyElaborationBlocker, TruthyFalsyElaborationReport, TruthyFalsyStageElaboration,
    TruthyFalsyStageOutcome, TruthyFalsyStagePlan, elaborate_truthy_falsy,
};
pub use typecheck::{
    ClassMemberImplementation, ConstraintClass, ResolvedClassMemberDispatch, TypeCheckReport,
    TypeConstraint, apply_defaults, elaborate_default_record_fields, typecheck_module,
};
pub use validate::{
    GateRecordField, GateType, TypeBinding, TypeConstructorBinding, TypeConstructorHead,
    ValidationMode, ValidationReport, case_pattern_field_types, validate_bindings, validate_module,
    validate_structure, validate_types,
};
