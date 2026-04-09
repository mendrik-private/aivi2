use std::collections::{BTreeMap, HashMap, HashSet};

use aivi_base::SourceSpan;
use aivi_hir::{
    BlockedFanoutSegment, BlockedGateStage, BlockedGeneralExpr as BlockedGeneralExprBody,
    BlockedRecurrenceNode, BlockedSourceDecodeProgram, BlockedSourceLifecycleNode,
    BlockedTemporalStage, BlockedTruthyFalsyStage, DecoratorPayload, ExprId as HirExprId,
    ExprKind as HirExprKind, GateRuntimeExpr, GateRuntimeExprKind, GateRuntimePipeExpr,
    GateRuntimePipeStageKind, GateRuntimeProjectionBase, GateRuntimeReference,
    GateRuntimeTextLiteral, GateRuntimeTextSegment, GateRuntimeTruthyFalsyBranch, GateStageOutcome,
    GeneralExprInstanceMemberElaboration, GeneralExprOutcome, GeneralExprParameter,
    ImportBindingMetadata, ImportId, ImportValueType, Item as HirItem, ItemId as HirItemId,
    PatternId as HirPatternId, PipeTransformMode, RecurrenceNodeOutcome,
    ResolvedClassMemberDispatch, SourceDecodeProgram, SourceDecodeProgramOutcome,
    SourceLifecycleNodeOutcome, SumConstructorHandle, TemporalStageOutcome, TermResolution,
    TruthyFalsyStageOutcome, TypeBinding, TypeConstructorHead, TypeItemBody,
    elaborate_ambient_items, elaborate_fanouts, elaborate_gates, elaborate_general_expressions,
    elaborate_recurrences, elaborate_source_lifecycles, elaborate_temporal_stages,
    elaborate_truthy_falsy, generate_source_decode_programs,
};

use crate::{
    Arena, ArenaOverflow, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject,
    BuiltinTraversableCarrier, DecodeField, DecodeProgram, DecodeProgramId, DecodeStep,
    DecodeStepId, DomainDecodeSurface, DomainDecodeSurfaceKind, Expr, ExprId, FanoutFilter,
    FanoutJoin, FanoutStage, GateStage, Item, ItemId, ItemKind, ItemParameter, MapEntry, Module,
    NonSourceWakeup, Pattern, PatternBinding, PatternConstructor, PatternKind, Pipe, PipeCaseArm,
    PipeExpr, PipeOrigin, PipeRecurrence, PipeStage, PipeTruthyFalsyBranch, PipeTruthyFalsyStage,
    ProjectionBase, RecordExprField, RecordPatternField, RecurrenceGuard, RecurrenceStage,
    Reference, SignalInfo, SourceArgumentValue, SourceId, SourceInstanceId, SourceNode,
    SourceOptionBinding, SourceOptionValue, Stage, StageKind, TextLiteral, TextSegment,
    TruthyFalsyBranch, TruthyFalsyStage, Type,
    expr::ExprKind,
    module::TemporalStage,
    validate::{ValidationError, validate_module},
};

include!("api.rs");

include!("module_lowerer.rs");

include!("runtime_fragment.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
