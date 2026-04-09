use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::fmt;

use aivi_base::SourceSpan;
use aivi_typing::{
    FanoutCarrier, FanoutPlan, FanoutPlanner, FanoutResultKind, FanoutStageKind, GateCarrier,
    GatePlanner, GateResultKind, RecurrenceTargetEvidence, RecurrenceWakeupKind,
    SourceTypeParameter,
};

use crate::{
    domain_operator_elaboration::{binary_operator_text, select_domain_binary_operator},
    function_inference::{
        FunctionCallEvidence, FunctionSignatureEvidence, infer_same_module_function_types,
        supports_same_module_function_inference,
    },
    hir::{
        ApplicativeSpineHead, BuiltinTerm, BuiltinType, ClassMemberResolution,
        CustomSourceRecurrenceWakeup, DomainMemberHandle, DomainMemberKind, DomainMemberResolution,
        ExprKind, ImportBindingMetadata, ImportTypeDefinition, ImportValueType, IntrinsicValue,
        Item, Module, Name, NamePath, PatternKind, PipeStage, PipeStageKind, PipeTransformMode,
        ProjectionBase, ResolutionState, TermReference, TermResolution, TextSegment, TypeItemBody,
        TypeKind, TypeReference, TypeResolution, TypeVariantField,
    },
    ids::{BindingId, ClusterId, ExprId, ImportId, ItemId, PatternId, TypeId, TypeParameterId},
    source_contract_resolution::{ResolvedSourceContractType, ResolvedSourceTypeConstructor},
    type_analysis::{
        CaseConstructorKey, CaseConstructorShape, CasePatternCoverage, CaseSubjectShape,
        GateRecordField, RecurrenceTargetHint,
    },
    typecheck::{TypeConstraint, expression_matches},
    validate::{ValidationMode, Validator, builtin_type_name},
};

include!("helpers.rs");
include!("gate_type.rs");
include!("bindings.rs");
include!("context.rs");
include!("source_options.rs");
