use std::collections::{HashMap, HashSet, hash_map::Entry};

use aivi_base::{ByteIndex, Diagnostic, DiagnosticCode, DiagnosticLabel, SourceSpan, Span};
use aivi_typing::{
    BuiltinSourceProvider, BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext,
    FanoutPlanner, FanoutStageKind, GatePlanner, Kind, KindCheckError, KindCheckErrorKind,
    KindChecker, KindExprId, KindParameterId as TypingKindParameterId, KindRecordField, KindStore,
    NonSourceWakeupCause, RecurrencePlanner, RecurrenceTargetEvidence, RecurrenceWakeupPlanner,
    SourceContractType, SourceRecurrenceWakeupContext, SourceTypeParameter,
    builtin_source_option_wakeup_cause,
};
use regex_syntax::{
    Error as RegexSyntaxError, ParserBuilder as RegexParserBuilder, ast::Span as RegexSpan,
};

use crate::{
    arena::{Arena, ArenaId},
    hir::{
        ApplicativeSpineHead, BuiltinTerm, BuiltinType, ControlNode, ControlNodeKind,
        DecoratorPayload, DeprecationNotice, DomainMemberKind, DomainMemberResolution,
        ExportResolution, ExprKind, ImportBindingMetadata, ImportBindingResolution, Item,
        LiteralSuffixResolution, MarkupAttributeValue, MarkupNodeKind, Module, Name, NamePath,
        PatternKind, PipeStageKind, RecordExpr, RecurrenceWakeupDecoratorKind, ResolutionState,
        SignalItem, SourceDecorator, SourceMetadata, SourceProviderRef, TermReference,
        TermResolution, TextLiteral, TextSegment, TypeItemBody, TypeKind, TypeReference,
        TypeResolution,
    },
    ids::{
        BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
        PatternId, TypeId, TypeParameterId,
    },
    signal_metadata_elaboration::expr_signal_dependencies,
    source_contract_resolution::{SourceContractResolutionErrorKind, SourceContractTypeResolver},
    typecheck::typecheck_module,
};

pub use crate::type_analysis::GateRecordField;
pub(crate) use crate::type_analysis::{
    CaseConstructorShape, CasePatternCoverage, RecurrenceTargetHint, walk_expr_tree,
};
pub(crate) use crate::typecheck_context::{
    ClassConstraintBinding, ClassMemberCallMatch, DomainMemberSelection, GateEqualityEvidence,
    GateExprEnv, GateIssue, GateProjectionStep, GateTypeContext, PipeFunctionSignatureMatch,
    PipeSubjectStepOutcome, PipeSubjectWalker, PolyTypeBindings, TruthyFalsyPairStages,
    ValidateStageSubject, extend_pipe_env_with_stage_memos, gate_env_for_function,
    pipe_stage_expr_env, truthy_falsy_pair_stages,
};
pub use crate::typecheck_context::{
    GateType, TypeBinding, TypeConstructorBinding, TypeConstructorHead, case_pattern_field_types,
};
use crate::typecheck_context::{
    PendingSourceOptionValue, SourceOptionActualRecordField, SourceOptionActualType,
    SourceOptionConstructorActual, SourceOptionExpectedRecordField, SourceOptionExpectedType,
    SourceOptionGenericConstructorRootCheck, SourceOptionNamedType, SourceOptionTypeBindings,
    SourceOptionTypeCheck, SourceOptionTypeMismatch, SourceOptionTypeSurface,
    custom_source_contract_expected, custom_source_contract_expected_type,
    custom_source_wakeup_kind, is_db_changed_trigger_projection, item_type_name,
    missing_case_label, missing_case_list, source_option_contract_parameter_phrase,
    source_option_expected_matches_actual_type, source_option_expected_to_gate_type,
    source_option_unresolved_contract_parameters, type_argument_phrase,
};

include!("api.rs");

include!("validator.rs");

include!("helpers.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
