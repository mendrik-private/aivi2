use std::fmt;

use aivi_base::SourceSpan;
use aivi_core::Arena;
use aivi_hir::{DomainMemberHandle, IntrinsicValue, PipeTransformMode, SumConstructorHandle};

use crate::{
    EnvSlotId, InlineSubjectId, ItemId, KernelExprId, LayoutId, PipelineId, SourceId,
    layout::AbiPassMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinTerm {
    True,
    False,
    None,
    Some,
    Ok,
    Err,
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinClassMemberIntrinsic {
    StructuralEq,
    Compare {
        subject: BuiltinOrdSubject,
        ordering_item: aivi_hir::ItemId,
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

impl fmt::Display for BuiltinTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => f.write_str("True"),
            Self::False => f.write_str("False"),
            Self::None => f.write_str("None"),
            Self::Some => f.write_str("Some"),
            Self::Ok => f.write_str("Ok"),
            Self::Err => f.write_str("Err"),
            Self::Valid => f.write_str("Valid"),
            Self::Invalid => f.write_str("Invalid"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    Not,
}

impl fmt::Display for UnaryOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Not => f.write_str("not"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    GreaterThan,
    LessThan,
    GreaterThanOrEqual,
    LessThanOrEqual,
    Equals,
    NotEquals,
    And,
    Or,
}

impl fmt::Display for BinaryOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add => f.write_str("+"),
            Self::Subtract => f.write_str("-"),
            Self::Multiply => f.write_str("*"),
            Self::Divide => f.write_str("/"),
            Self::Modulo => f.write_str("%"),
            Self::GreaterThan => f.write_str(">"),
            Self::LessThan => f.write_str("<"),
            Self::GreaterThanOrEqual => f.write_str(">="),
            Self::LessThanOrEqual => f.write_str("<="),
            Self::Equals => f.write_str("=="),
            Self::NotEquals => f.write_str("!="),
            Self::And => f.write_str("&&"),
            Self::Or => f.write_str("||"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerLiteral {
    pub raw: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatLiteral {
    pub raw: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecimalLiteral {
    pub raw: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigIntLiteral {
    pub raw: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuffixedIntegerLiteral {
    pub raw: Box<str>,
    pub suffix: Box<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubjectRef {
    Input,
    Inline(InlineSubjectId),
}

impl fmt::Display for SubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input => f.write_str("input"),
            Self::Inline(slot) => write!(f, "inline{slot}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallingConventionKind {
    RuntimeKernelV1,
}

impl fmt::Display for CallingConventionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeKernelV1 => f.write_str("runtime-kernel-v1"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterRole {
    InputSubject,
    Environment(EnvSlotId),
}

impl fmt::Display for ParameterRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputSubject => f.write_str("input"),
            Self::Environment(slot) => write!(f, "env{slot}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbiParameter {
    pub role: ParameterRole,
    pub layout: LayoutId,
    pub pass_mode: AbiPassMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbiResult {
    pub layout: LayoutId,
    pub pass_mode: AbiPassMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallingConvention {
    pub kind: CallingConventionKind,
    pub parameters: Vec<AbiParameter>,
    pub result: AbiResult,
}

impl fmt::Display for CallingConvention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.kind)?;
        for (index, parameter) in self.parameters.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(
                f,
                "{}: layout{} [{}]",
                parameter.role, parameter.layout, parameter.pass_mode
            )?;
        }
        write!(
            f,
            ") -> layout{} [{}]",
            self.result.layout, self.result.pass_mode
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelOriginKind {
    /// The kernel was produced from the item body of the item identified by
    /// `item`.  All other `KernelOriginKind` variants identify their producer
    /// through their own fields; this variant makes that identity explicit so
    /// diagnostic messages can name the item without having to look up the
    /// parent `KernelOrigin`.
    ItemBody {
        item: ItemId,
    },
    GateTrue {
        pipeline: PipelineId,
        stage_index: usize,
    },
    GateFalse {
        pipeline: PipelineId,
        stage_index: usize,
    },
    SignalFilterPredicate {
        pipeline: PipelineId,
        stage_index: usize,
    },
    PreviousSeed {
        pipeline: PipelineId,
        stage_index: usize,
    },
    DiffFunction {
        pipeline: PipelineId,
        stage_index: usize,
    },
    DiffSeed {
        pipeline: PipelineId,
        stage_index: usize,
    },
    FanoutMap {
        pipeline: PipelineId,
        stage_index: usize,
    },
    FanoutFilterPredicate {
        pipeline: PipelineId,
        stage_index: usize,
    },
    FanoutJoin {
        pipeline: PipelineId,
        stage_index: usize,
    },
    RecurrenceStart {
        pipeline: PipelineId,
        stage_index: usize,
    },
    RecurrenceStep {
        pipeline: PipelineId,
        stage_index: usize,
    },
    RecurrenceWakeupWitness {
        pipeline: PipelineId,
    },
    RecurrenceSeed {
        pipeline: PipelineId,
    },
    SourceArgument {
        source: SourceId,
        index: usize,
    },
    SourceOption {
        source: SourceId,
        index: usize,
    },
}

impl fmt::Display for KernelOriginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemBody { item } => write!(f, "item-body item{item}"),
            Self::GateTrue {
                pipeline,
                stage_index,
            } => write!(f, "gate-true pipeline{pipeline}[{stage_index}]"),
            Self::GateFalse {
                pipeline,
                stage_index,
            } => write!(f, "gate-false pipeline{pipeline}[{stage_index}]"),
            Self::SignalFilterPredicate {
                pipeline,
                stage_index,
            } => write!(f, "signal-predicate pipeline{pipeline}[{stage_index}]"),
            Self::PreviousSeed {
                pipeline,
                stage_index,
            } => write!(f, "previous-seed pipeline{pipeline}[{stage_index}]"),
            Self::DiffFunction {
                pipeline,
                stage_index,
            } => write!(f, "diff-function pipeline{pipeline}[{stage_index}]"),
            Self::DiffSeed {
                pipeline,
                stage_index,
            } => write!(f, "diff-seed pipeline{pipeline}[{stage_index}]"),
            Self::FanoutMap {
                pipeline,
                stage_index,
            } => write!(f, "fanout-map pipeline{pipeline}[{stage_index}]"),
            Self::FanoutFilterPredicate {
                pipeline,
                stage_index,
            } => write!(f, "fanout-filter pipeline{pipeline}[{stage_index}]"),
            Self::FanoutJoin {
                pipeline,
                stage_index,
            } => write!(f, "fanout-join pipeline{pipeline}[{stage_index}]"),
            Self::RecurrenceStart {
                pipeline,
                stage_index,
            } => write!(f, "recurrence-start pipeline{pipeline}[{stage_index}]"),
            Self::RecurrenceStep {
                pipeline,
                stage_index,
            } => write!(f, "recurrence-step pipeline{pipeline}[{stage_index}]"),
            Self::RecurrenceWakeupWitness { pipeline } => {
                write!(f, "recurrence-witness pipeline{pipeline}")
            }
            Self::RecurrenceSeed { pipeline } => write!(f, "recurrence-seed pipeline{pipeline}"),
            Self::SourceArgument { source, index } => {
                write!(f, "source-argument source{source}[{index}]")
            }
            Self::SourceOption { source, index } => {
                write!(f, "source-option source{source}[{index}]")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelOrigin {
    pub item: ItemId,
    pub span: SourceSpan,
    pub kind: KernelOriginKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kernel {
    pub origin: KernelOrigin,
    pub input_subject: Option<LayoutId>,
    pub inline_subjects: Vec<LayoutId>,
    pub environment: Vec<LayoutId>,
    pub result_layout: LayoutId,
    pub convention: CallingConvention,
    pub global_items: Vec<ItemId>,
    pub root: KernelExprId,
    exprs: Arena<KernelExprId, KernelExpr>,
}

impl Kernel {
    pub fn new(
        origin: KernelOrigin,
        input_subject: Option<LayoutId>,
        inline_subjects: Vec<LayoutId>,
        environment: Vec<LayoutId>,
        result_layout: LayoutId,
        convention: CallingConvention,
        global_items: Vec<ItemId>,
        root: KernelExprId,
        exprs: Arena<KernelExprId, KernelExpr>,
    ) -> Self {
        Self {
            origin,
            input_subject,
            inline_subjects,
            environment,
            result_layout,
            convention,
            global_items,
            root,
            exprs,
        }
    }

    pub fn exprs(&self) -> &Arena<KernelExprId, KernelExpr> {
        &self.exprs
    }

    pub fn exprs_mut(&mut self) -> &mut Arena<KernelExprId, KernelExpr> {
        &mut self.exprs
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelExpr {
    pub span: SourceSpan,
    pub layout: LayoutId,
    pub kind: KernelExprKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelExprKind {
    Subject(SubjectRef),
    OptionSome {
        payload: KernelExprId,
    },
    OptionNone,
    Environment(EnvSlotId),
    Item(ItemId),
    SumConstructor(SumConstructorHandle),
    DomainMember(DomainMemberHandle),
    BuiltinClassMember(BuiltinClassMemberIntrinsic),
    Builtin(BuiltinTerm),
    IntrinsicValue(IntrinsicValue),
    Integer(IntegerLiteral),
    Float(FloatLiteral),
    Decimal(DecimalLiteral),
    BigInt(BigIntLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(TextLiteral),
    Tuple(Vec<KernelExprId>),
    List(Vec<KernelExprId>),
    Map(Vec<MapEntry>),
    Set(Vec<KernelExprId>),
    Record(Vec<RecordExprField>),
    Projection {
        base: ProjectionBase,
        path: Vec<Box<str>>,
    },
    Apply {
        callee: KernelExprId,
        arguments: Vec<KernelExprId>,
    },
    Unary {
        operator: UnaryOperator,
        expr: KernelExprId,
    },
    Binary {
        left: KernelExprId,
        operator: BinaryOperator,
        right: KernelExprId,
    },
    Pipe(InlinePipeExpr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionBase {
    Subject(SubjectRef),
    Expr(KernelExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLiteral {
    pub segments: Vec<TextSegment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSegment {
    Fragment {
        raw: Box<str>,
        span: SourceSpan,
    },
    Interpolation {
        expr: KernelExprId,
        span: SourceSpan,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordExprField {
    pub label: Box<str>,
    pub value: KernelExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEntry {
    pub key: KernelExprId,
    pub value: KernelExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipeExpr {
    pub head: KernelExprId,
    pub stages: Vec<InlinePipeStage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipeStage {
    pub subject: InlineSubjectId,
    pub subject_memo: Option<InlineSubjectId>,
    pub result_memo: Option<InlineSubjectId>,
    pub span: SourceSpan,
    pub input_layout: LayoutId,
    pub result_layout: LayoutId,
    pub kind: InlinePipeStageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlinePipeStageKind {
    Transform {
        mode: PipeTransformMode,
        expr: KernelExprId,
    },
    Tap {
        expr: KernelExprId,
    },
    Debug {
        label: Box<str>,
    },
    Gate {
        predicate: KernelExprId,
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<InlinePipeCaseArm>,
    },
    TruthyFalsy {
        truthy: InlinePipeTruthyFalsyBranch,
        falsy: InlinePipeTruthyFalsyBranch,
    },
    FanOut {
        map_expr: KernelExprId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipeCaseArm {
    pub span: SourceSpan,
    pub pattern: InlinePipePattern,
    pub body: KernelExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipeTruthyFalsyBranch {
    pub span: SourceSpan,
    pub constructor: BuiltinTerm,
    pub payload_subject: Option<InlineSubjectId>,
    pub body: KernelExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipePattern {
    pub span: SourceSpan,
    pub kind: InlinePipePatternKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlinePipeConstructor {
    Builtin(BuiltinTerm),
    Sum(SumConstructorHandle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlinePipePatternKind {
    Wildcard,
    Binding {
        subject: InlineSubjectId,
    },
    Integer(IntegerLiteral),
    Text(Box<str>),
    Tuple(Vec<InlinePipePattern>),
    List {
        elements: Vec<InlinePipePattern>,
        rest: Option<Box<InlinePipePattern>>,
    },
    Record(Vec<InlinePipeRecordPatternField>),
    Constructor {
        constructor: InlinePipeConstructor,
        arguments: Vec<InlinePipePattern>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlinePipeRecordPatternField {
    pub label: Box<str>,
    pub pattern: InlinePipePattern,
}

pub fn describe_expr_kind(kind: &KernelExprKind) -> String {
    match kind {
        KernelExprKind::Subject(subject) => format!("subject {subject}"),
        KernelExprKind::OptionSome { payload } => format!("Some expr{payload}"),
        KernelExprKind::OptionNone => "None".to_owned(),
        KernelExprKind::Environment(slot) => format!("env{slot}"),
        KernelExprKind::Item(item) => format!("item item{item}"),
        KernelExprKind::SumConstructor(handle) => {
            format!(
                "sum-constructor {}.{}",
                handle.type_name, handle.variant_name
            )
        }
        KernelExprKind::DomainMember(handle) => {
            format!(
                "domain-member {}.{}",
                handle.domain_name, handle.member_name
            )
        }
        KernelExprKind::BuiltinClassMember(intrinsic) => {
            format!("builtin-class-member {intrinsic:?}")
        }
        KernelExprKind::Builtin(term) => format!("builtin {term}"),
        KernelExprKind::IntrinsicValue(value) => format!("intrinsic {value}"),
        KernelExprKind::Integer(integer) => integer.raw.to_string(),
        KernelExprKind::Float(float) => float.raw.to_string(),
        KernelExprKind::Decimal(decimal) => decimal.raw.to_string(),
        KernelExprKind::BigInt(bigint) => bigint.raw.to_string(),
        KernelExprKind::SuffixedInteger(integer) => format!("{}{}", integer.raw, integer.suffix),
        KernelExprKind::Text(text) => format!("text segments={}", text.segments.len()),
        KernelExprKind::Tuple(elements) => format!("tuple elems={}", elements.len()),
        KernelExprKind::List(elements) => format!("list elems={}", elements.len()),
        KernelExprKind::Map(entries) => format!("map entries={}", entries.len()),
        KernelExprKind::Set(elements) => format!("set elems={}", elements.len()),
        KernelExprKind::Record(fields) => format!("record fields={}", fields.len()),
        KernelExprKind::Projection { base, path } => {
            let base = match base {
                ProjectionBase::Subject(subject) => format!("subject {subject}"),
                ProjectionBase::Expr(expr) => format!("expr{expr}"),
            };
            format!("projection {base} .{}", path.join("."))
        }
        KernelExprKind::Apply { callee, arguments } => {
            format!("apply expr{callee} args={}", arguments.len())
        }
        KernelExprKind::Unary { operator, expr } => format!("{operator} expr{expr}"),
        KernelExprKind::Binary {
            left,
            operator,
            right,
        } => format!("expr{left} {operator} expr{right}"),
        KernelExprKind::Pipe(pipe) => {
            format!("pipe head=expr{} stages={}", pipe.head, pipe.stages.len())
        }
    }
}
