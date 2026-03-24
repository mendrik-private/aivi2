use aivi_base::SourceSpan;
use aivi_hir::{
    BinaryOperator, BindingId as HirBindingId, BuiltinTerm, DomainMemberHandle, IntegerLiteral,
    ItemId as HirItemId, SuffixedIntegerLiteral, SumConstructorHandle, UnaryOperator,
};

use crate::{ids::ExprId, ty::Type};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub span: SourceSpan,
    pub ty: Type,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    AmbientSubject,
    OptionSome {
        payload: ExprId,
    },
    OptionNone,
    Reference(Reference),
    Integer(IntegerLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(TextLiteral),
    Tuple(Vec<ExprId>),
    List(Vec<ExprId>),
    Map(Vec<MapEntry>),
    Set(Vec<ExprId>),
    Record(Vec<RecordExprField>),
    Projection {
        base: ProjectionBase,
        path: Vec<Box<str>>,
    },
    Apply {
        callee: ExprId,
        arguments: Vec<ExprId>,
    },
    Unary {
        operator: UnaryOperator,
        expr: ExprId,
    },
    Binary {
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
    },
    Pipe(PipeExpr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    Local(HirBindingId),
    Item(crate::ItemId),
    HirItem(HirItemId),
    SumConstructor(SumConstructorHandle),
    DomainMember(DomainMemberHandle),
    BuiltinClassMember(BuiltinClassMemberIntrinsic),
    Builtin(BuiltinTerm),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinClassMemberIntrinsic {
    StructuralEq,
    Compare {
        subject: BuiltinOrdSubject,
        ordering_item: HirItemId,
    },
    Append(BuiltinAppendCarrier),
    Empty(BuiltinAppendCarrier),
    Map(BuiltinFunctorCarrier),
    Pure(BuiltinApplicativeCarrier),
    Apply(BuiltinApplyCarrier),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFunctorCarrier {
    List,
    Option,
    Result,
    Validation,
    Signal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinApplicativeCarrier {
    List,
    Option,
    Result,
    Validation,
    Signal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinApplyCarrier {
    List,
    Option,
    Result,
    Signal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinAppendCarrier {
    Text,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinOrdSubject {
    Int,
    Bool,
    Text,
    Ordering,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionBase {
    AmbientSubject,
    Expr(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLiteral {
    pub segments: Vec<TextSegment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSegment {
    Fragment { raw: Box<str>, span: SourceSpan },
    Interpolation { expr: ExprId, span: SourceSpan },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordExprField {
    pub label: Box<str>,
    pub value: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEntry {
    pub key: ExprId,
    pub value: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeExpr {
    pub head: ExprId,
    pub stages: Vec<PipeStage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub span: SourceSpan,
    pub kind: PatternKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Binding(PatternBinding),
    Integer(IntegerLiteral),
    Text(Box<str>),
    Tuple(Vec<Pattern>),
    Record(Vec<RecordPatternField>),
    Constructor {
        callee: PatternConstructor,
        arguments: Vec<Pattern>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternBinding {
    pub binding: HirBindingId,
    pub name: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordPatternField {
    pub label: Box<str>,
    pub pattern: Pattern,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternConstructor {
    pub display: Box<str>,
    pub reference: Reference,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeStage {
    pub span: SourceSpan,
    pub input_subject: Type,
    pub result_subject: Type,
    pub kind: PipeStageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeCaseArm {
    pub span: SourceSpan,
    pub pattern: Pattern,
    pub body: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeTruthyFalsyStage {
    pub truthy: PipeTruthyFalsyBranch,
    pub falsy: PipeTruthyFalsyBranch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeTruthyFalsyBranch {
    pub span: SourceSpan,
    pub constructor: BuiltinTerm,
    pub payload_subject: Option<Type>,
    pub result_type: Type,
    pub body: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipeStageKind {
    Transform {
        expr: ExprId,
    },
    Tap {
        expr: ExprId,
    },
    Gate {
        predicate: ExprId,
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<PipeCaseArm>,
    },
    TruthyFalsy(PipeTruthyFalsyStage),
}
