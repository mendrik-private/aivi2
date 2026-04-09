use std::collections::{HashMap, HashSet};

use aivi_base::{Diagnostic, DiagnosticCode, SourceSpan};

use crate::{
    domain_operator_elaboration::select_domain_binary_operator,
    function_inference::{FunctionSignatureEvidence, supports_same_module_function_inference},
    hir::{
        BinaryOperator, BuiltinTerm, BuiltinType, ClassMemberResolution, DomainMember, ExprKind,
        FunctionItem, ImportBindingMetadata, ImportBundleKind, InstanceItem, InstanceMember, Item,
        MapExpr, Module, Name, NamePath, PatternKind, PipeExpr, PipeStageKind, ProjectionBase,
        ReactiveUpdateBodyMode, RecordExpr, RecordExprField, RecordFieldSurface, ResolutionState,
        SignalItem, TermReference, TermResolution, TypeItemBody, TypeResolution, UnaryOperator,
        ValueItem,
    },
    ids::{BindingId, ExprId, ImportId, ItemId, PatternId, TypeId, TypeParameterId},
    validate::{
        ClassConstraintBinding, ClassMemberCallMatch, DomainMemberSelection, GateExprEnv,
        GateIssue, GateRecordField, GateType, GateTypeContext, PolyTypeBindings, TypeBinding,
    },
};

include!("api.rs");

include!("checker.rs");

include!("helpers.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
