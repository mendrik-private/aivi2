use std::collections::HashMap;

use aivi_base::{Diagnostic, DiagnosticCode, Severity, SourceSpan};
use aivi_syntax as syn;
use aivi_typing::Kind;

use crate::{
    ApplicativeCluster, ApplicativeSpineHead, AtLeastTwo, BigIntLiteral, BinaryOperator, Binding,
    BindingId, BindingKind, BindingPattern, BuiltinTerm, BuiltinType, CaseControl, ClassItem,
    ClassMember, ClusterFinalizer, ClusterPresentation, ControlNode, ControlNodeId, DebugDecorator,
    DecimalLiteral, Decorator, DecoratorCall, DecoratorId, DecoratorPayload, DeprecatedDecorator,
    DomainItem, DomainMember, DomainMemberKind, DomainMemberResolution, EachControl, EmptyControl,
    ExportItem, ExportResolution, Expr, ExprId, ExprKind, FloatLiteral, FragmentControl,
    FunctionItem, FunctionParameter, HoistItem, HoistKindFilter, ImportBinding,
    ImportBindingMetadata, ImportBindingResolution, ImportBundleKind, ImportId,
    ImportModuleResolution, ImportRecordField, ImportValueType, ImportedDomainLiteralSuffix,
    InstanceItem, InstanceMember, IntegerLiteral, IntrinsicValue, Item, ItemHeader, ItemId,
    ItemKind, LiteralSuffixResolution, MapExpr, MapExprEntry, MarkupAttribute,
    MarkupAttributeValue, MarkupElement, MarkupNode, MarkupNodeId, MarkupNodeKind, MatchControl,
    MockDecorator, Module, Name, NamePath, NonEmpty, PatchBlock, PatchEntry, PatchInstruction,
    PatchInstructionKind, PatchSelector, PatchSelectorSegment, Pattern, PatternId, PatternKind,
    PipeExpr, PipeStage, PipeStageKind, ProjectionBase, ReactiveUpdateBodyMode,
    ReactiveUpdateClause, RecordExpr, RecordExprField, RecordFieldSurface, RecordPatternField,
    RecordRowRename, RecordRowTransform, RecurrenceWakeupDecorator, RecurrenceWakeupDecoratorKind,
    RegexLiteral, ResolutionState, Resolved, ShowControl, SignalItem, SourceDecorator,
    SourceProviderContractItem, SourceProviderRef, SuffixedIntegerLiteral, TermReference,
    TermResolution, TestDecorator, TextFragment, TextInterpolation, TextLiteral, TextSegment,
    TypeField, TypeId, TypeItem, TypeItemBody, TypeKind, TypeNode, TypeParameter, TypeParameterId,
    TypeReference, TypeResolution, TypeVariant, UnaryOperator, Unresolved, UseItem, ValueItem,
    WithControl,
};

include!("api.rs");

include!("ambient.rs");

include!("lowerer.rs");

include!("helpers.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
