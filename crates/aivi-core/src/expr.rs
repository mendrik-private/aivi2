use aivi_base::SourceSpan;
use aivi_hir::{
    BigIntLiteral, BinaryOperator, BindingId as HirBindingId, BuiltinTerm, DecimalLiteral,
    DomainMemberHandle, FloatLiteral, IntegerLiteral, IntrinsicValue, ItemId as HirItemId,
    PipeTransformMode, SuffixedIntegerLiteral, SumConstructorHandle, UnaryOperator,
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
    Float(FloatLiteral),
    Decimal(DecimalLiteral),
    BigInt(BigIntLiteral),
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
    ExecutableEvidence(ExecutableClassMember),
    Builtin(BuiltinTerm),
    IntrinsicValue(IntrinsicValue),
}

pub type ExecutableClassMember = ExecutableEvidence<crate::ItemId, BuiltinClassMemberIntrinsic>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExecutableEvidence<Item, Builtin> {
    Authored(Item),
    Builtin(Builtin),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinClassMemberIntrinsic {
    StructuralEq,
    Compare {
        subject: BuiltinOrdSubject,
        ordering_item: HirItemId,
    },
    Append(BuiltinAppendCarrier),
    Empty(BuiltinAppendCarrier),
    Map(BuiltinFunctorCarrier),
    Bimap(BuiltinBifunctorCarrier),
    Pure(BuiltinApplicativeCarrier),
    Apply(BuiltinApplyCarrier),
    Chain(BuiltinMonadCarrier),
    Join(BuiltinMonadCarrier),
    Reduce(BuiltinFoldableCarrier),
    Traverse {
        traversable: BuiltinTraversableCarrier,
        applicative: BuiltinApplicativeCarrier,
    },
    FilterMap(BuiltinFilterableCarrier),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinFunctorCarrier {
    List,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinBifunctorCarrier {
    Result,
    Validation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinApplicativeCarrier {
    List,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinApplyCarrier {
    List,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinMonadCarrier {
    List,
    Option,
    Result,
    Task,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinFoldableCarrier {
    List,
    Option,
    Result,
    Validation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinTraversableCarrier {
    List,
    Option,
    Result,
    Validation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinFilterableCarrier {
    List,
    Option,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinAppendCarrier {
    Text,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinOrdSubject {
    Int,
    Float,
    Decimal,
    BigInt,
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
    List {
        elements: Vec<Pattern>,
        rest: Option<Box<Pattern>>,
    },
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
    pub field_types: Option<Vec<Type>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeStage {
    pub span: SourceSpan,
    pub subject_memo: Option<HirBindingId>,
    pub result_memo: Option<HirBindingId>,
    pub input_subject: Type,
    pub result_subject: Type,
    pub kind: PipeStageKind,
}

impl PipeStage {
    pub const fn supports_memos(&self) -> bool {
        true
    }
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
        mode: PipeTransformMode,
        expr: ExprId,
    },
    Tap {
        expr: ExprId,
    },
    Debug {
        label: Box<str>,
    },
    Gate {
        predicate: ExprId,
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<PipeCaseArm>,
    },
    TruthyFalsy(PipeTruthyFalsyStage),
    FanOut {
        map_expr: ExprId,
    },
}
