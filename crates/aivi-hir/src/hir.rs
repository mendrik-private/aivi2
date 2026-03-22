use std::{error::Error, fmt};

use aivi_base::{FileId, SourceSpan};

use crate::{
    arena::{Arena, ArenaOverflow},
    ids::{
        BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
        PatternId, TypeId, TypeParameterId,
    },
    sequence::{AtLeastTwo, NonEmpty, SequenceError},
    validate::{ValidationMode, ValidationReport, validate_module},
};

/// One source-stable surface name preserved into HIR for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Name {
    text: Box<str>,
    span: SourceSpan,
}

/// Name construction error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("HIR names must not be empty"),
        }
    }
}

impl Error for NameError {}

impl Name {
    pub fn new(text: impl Into<String>, span: SourceSpan) -> Result<Self, NameError> {
        let text = text.into();
        if text.is_empty() {
            return Err(NameError::Empty);
        }

        Ok(Self {
            text: text.into_boxed_str(),
            span,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// Non-empty dotted path used by references, decorators, projections, and markup names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamePath {
    segments: NonEmpty<Name>,
    span: SourceSpan,
}

/// Path construction error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamePathError {
    Empty,
    MixedFiles { expected: FileId, found: FileId },
}

impl fmt::Display for NamePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("HIR name paths must contain at least one segment"),
            Self::MixedFiles { expected, found } => write!(
                f,
                "HIR name path segments must stay in one file, expected {expected} but found {found}"
            ),
        }
    }
}

impl Error for NamePathError {}

impl NamePath {
    pub fn new(segments: NonEmpty<Name>) -> Result<Self, NamePathError> {
        let mut iter = segments.iter();
        let first = iter
            .next()
            .expect("NonEmpty always contains at least one segment");
        let mut span = first.span();
        let expected = span.file();

        for segment in iter {
            let found = segment.span().file();
            if found != expected {
                return Err(NamePathError::MixedFiles { expected, found });
            }
            span = span
                .join(segment.span())
                .expect("segments already guaranteed to come from the same file");
        }

        Ok(Self { segments, span })
    }

    pub fn from_vec(segments: Vec<Name>) -> Result<Self, NamePathError> {
        let segments = NonEmpty::from_vec(segments).map_err(|error| match error {
            SequenceError::Empty => NamePathError::Empty,
            SequenceError::TooShort { .. } => NamePathError::Empty,
        })?;
        Self::new(segments)
    }

    pub fn segments(&self) -> &NonEmpty<Name> {
        &self.segments
    }

    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

impl fmt::Display for NamePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut segments = self.segments.iter();
        if let Some(first) = segments.next() {
            f.write_str(first.text())?;
        }
        for segment in segments {
            write!(f, ".{}", segment.text())?;
        }
        Ok(())
    }
}

/// Resolution marker used until Milestone 2 lowering populates every reference honestly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResolutionState<T> {
    #[default]
    Unresolved,
    Resolved(T),
}

impl<T> ResolutionState<T> {
    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved(_))
    }

    pub fn as_ref(&self) -> ResolutionState<&T> {
        match self {
            Self::Unresolved => ResolutionState::Unresolved,
            Self::Resolved(value) => ResolutionState::Resolved(value),
        }
    }
}

/// Local binding introduced by parameters, patterns, and markup control nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub span: SourceSpan,
    pub name: Name,
    pub kind: BindingKind,
}

/// Distinguishes how a binding entered scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingKind {
    FunctionParameter,
    Pattern,
    MarkupEach,
    MarkupWith,
}

/// HIR-level type parameter identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParameter {
    pub span: SourceSpan,
    pub name: Name,
}

/// One imported binding surfaced by a `use` item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportBinding {
    pub span: SourceSpan,
    pub imported_name: Name,
    pub local_name: Name,
}

/// Compiler-known builtin term references that live outside the current module graph.
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

/// Compiler-known builtin type references that live outside the current module graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinType {
    Int,
    Float,
    Decimal,
    BigInt,
    Bool,
    Text,
    Unit,
    Bytes,
    List,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

/// Resolved destination for a term-level reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TermResolution {
    Local(BindingId),
    Item(ItemId),
    Import(ImportId),
    Builtin(BuiltinTerm),
}

/// Resolved destination for a type-level reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeResolution {
    Item(ItemId),
    TypeParameter(TypeParameterId),
    Import(ImportId),
    Builtin(BuiltinType),
}

/// Term-level name reference preserved with dotted spelling for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TermReference {
    pub path: NamePath,
    pub resolution: ResolutionState<TermResolution>,
}

impl TermReference {
    pub fn unresolved(path: NamePath) -> Self {
        Self {
            path,
            resolution: ResolutionState::Unresolved,
        }
    }

    pub fn resolved(path: NamePath, resolution: TermResolution) -> Self {
        Self {
            path,
            resolution: ResolutionState::Resolved(resolution),
        }
    }

    pub const fn span(&self) -> SourceSpan {
        self.path.span()
    }
}

/// Type-level name reference preserved with dotted spelling for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeReference {
    pub path: NamePath,
    pub resolution: ResolutionState<TypeResolution>,
}

impl TypeReference {
    pub fn unresolved(path: NamePath) -> Self {
        Self {
            path,
            resolution: ResolutionState::Unresolved,
        }
    }

    pub fn resolved(path: NamePath, resolution: TypeResolution) -> Self {
        Self {
            path,
            resolution: ResolutionState::Resolved(resolution),
        }
    }

    pub const fn span(&self) -> SourceSpan {
        self.path.span()
    }
}

/// Resolved destination for a domain literal suffix use site such as `250ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiteralSuffixResolution {
    pub domain: ItemId,
    pub member_index: usize,
}

/// Shared top-level metadata attached to every HIR item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemHeader {
    pub span: SourceSpan,
    pub decorators: Vec<DecoratorId>,
}

/// Stable item discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Type,
    Value,
    Function,
    Signal,
    Class,
    Domain,
    Instance,
    Use,
    Export,
}

/// One module-level declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Type(TypeItem),
    Value(ValueItem),
    Function(FunctionItem),
    Signal(SignalItem),
    Class(ClassItem),
    Domain(DomainItem),
    Instance(InstanceItem),
    Use(UseItem),
    Export(ExportItem),
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Type(_) => ItemKind::Type,
            Self::Value(_) => ItemKind::Value,
            Self::Function(_) => ItemKind::Function,
            Self::Signal(_) => ItemKind::Signal,
            Self::Class(_) => ItemKind::Class,
            Self::Domain(_) => ItemKind::Domain,
            Self::Instance(_) => ItemKind::Instance,
            Self::Use(_) => ItemKind::Use,
            Self::Export(_) => ItemKind::Export,
        }
    }

    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Type(item) => item.header.span,
            Self::Value(item) => item.header.span,
            Self::Function(item) => item.header.span,
            Self::Signal(item) => item.header.span,
            Self::Class(item) => item.header.span,
            Self::Domain(item) => item.header.span,
            Self::Instance(item) => item.header.span,
            Self::Use(item) => item.header.span,
            Self::Export(item) => item.header.span,
        }
    }

    pub fn decorators(&self) -> &[DecoratorId] {
        match self {
            Self::Type(item) => &item.header.decorators,
            Self::Value(item) => &item.header.decorators,
            Self::Function(item) => &item.header.decorators,
            Self::Signal(item) => &item.header.decorators,
            Self::Class(item) => &item.header.decorators,
            Self::Domain(item) => &item.header.decorators,
            Self::Instance(item) => &item.header.decorators,
            Self::Use(item) => &item.header.decorators,
            Self::Export(item) => &item.header.decorators,
        }
    }
}

/// One `type` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeItem {
    pub header: ItemHeader,
    pub name: Name,
    pub parameters: Vec<TypeParameterId>,
    pub body: TypeItemBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeItemBody {
    Alias(TypeId),
    Sum(NonEmpty<TypeVariant>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeVariant {
    pub span: SourceSpan,
    pub name: Name,
    pub fields: Vec<TypeId>,
}

/// One `val` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueItem {
    pub header: ItemHeader,
    pub name: Name,
    pub annotation: Option<TypeId>,
    pub body: ExprId,
}

/// One `fun` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionItem {
    pub header: ItemHeader,
    pub name: Name,
    pub parameters: Vec<FunctionParameter>,
    pub annotation: Option<TypeId>,
    pub body: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionParameter {
    pub span: SourceSpan,
    pub binding: BindingId,
    pub annotation: Option<TypeId>,
}

/// One `sig` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalItem {
    pub header: ItemHeader,
    pub name: Name,
    pub annotation: Option<TypeId>,
    pub body: Option<ExprId>,
    pub signal_dependencies: Vec<ItemId>,
    pub source_metadata: Option<SourceMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMetadata {
    pub provider_key: Option<Box<str>>,
    pub signal_dependencies: Vec<ItemId>,
    pub is_reactive: bool,
}

/// One `class` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassItem {
    pub header: ItemHeader,
    pub name: Name,
    pub parameters: NonEmpty<TypeParameterId>,
    pub superclasses: Vec<TypeId>,
    pub members: Vec<ClassMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassMember {
    pub span: SourceSpan,
    pub name: Name,
    pub annotation: TypeId,
}

/// One `domain` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainItem {
    pub header: ItemHeader,
    pub name: Name,
    pub parameters: Vec<TypeParameterId>,
    pub carrier: TypeId,
    pub members: Vec<DomainMember>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DomainMemberKind {
    Method,
    Operator,
    Literal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainMember {
    pub span: SourceSpan,
    pub kind: DomainMemberKind,
    pub name: Name,
    pub annotation: TypeId,
}

/// One `instance` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceItem {
    pub header: ItemHeader,
    pub class: TypeReference,
    pub arguments: NonEmpty<TypeId>,
    pub context: Vec<TypeId>,
    pub members: Vec<InstanceMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceMember {
    pub span: SourceSpan,
    pub name: Name,
    pub annotation: Option<TypeId>,
    pub body: ExprId,
}

/// One `use` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseItem {
    pub header: ItemHeader,
    pub module: NamePath,
    pub imports: NonEmpty<ImportId>,
}

/// One `export` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportItem {
    pub header: ItemHeader,
    pub target: NamePath,
    pub resolution: ResolutionState<ItemId>,
}

/// One integer literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerLiteral {
    pub raw: Box<str>,
}

/// One integer literal immediately suffixed by a resolved-or-resolvable domain suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuffixedIntegerLiteral {
    pub raw: Box<str>,
    pub suffix: Name,
    pub resolution: ResolutionState<LiteralSuffixResolution>,
}

/// One text literal preserved as explicit text fragments plus interpolation holes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLiteral {
    pub segments: Vec<TextSegment>,
}

impl TextLiteral {
    pub fn has_interpolation(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment, TextSegment::Interpolation(_)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSegment {
    Text(TextFragment),
    Interpolation(TextInterpolation),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFragment {
    pub raw: Box<str>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInterpolation {
    pub span: SourceSpan,
    pub expr: ExprId,
}

/// One regex literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexLiteral {
    pub raw: Box<str>,
}

/// Unary operators preserved through HIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    Not,
}

/// Binary operators preserved through HIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    Add,
    Subtract,
    GreaterThan,
    LessThan,
    Equals,
    NotEquals,
    And,
    Or,
}

/// One expression node owned by the module expression arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub span: SourceSpan,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Name(TermReference),
    Integer(IntegerLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(TextLiteral),
    Regex(RegexLiteral),
    Tuple(AtLeastTwo<ExprId>),
    List(Vec<ExprId>),
    Record(RecordExpr),
    Projection {
        base: ProjectionBase,
        path: NamePath,
    },
    Apply {
        callee: ExprId,
        arguments: NonEmpty<ExprId>,
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
    Cluster(ClusterId),
    Markup(MarkupNodeId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordExpr {
    pub fields: Vec<RecordExprField>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordFieldSurface {
    Explicit,
    Shorthand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordExprField {
    pub span: SourceSpan,
    pub label: Name,
    pub value: ExprId,
    pub surface: RecordFieldSurface,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionBase {
    Ambient,
    Expr(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeExpr {
    pub head: ExprId,
    pub stages: NonEmpty<PipeStage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeStage {
    pub span: SourceSpan,
    pub kind: PipeStageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipeStageKind {
    Transform { expr: ExprId },
    Gate { expr: ExprId },
    Case { pattern: PatternId, body: ExprId },
    Map { expr: ExprId },
    Apply { expr: ExprId },
    Tap { expr: ExprId },
    FanIn { expr: ExprId },
    Truthy { expr: ExprId },
    Falsy { expr: ExprId },
    RecurStart { expr: ExprId },
    RecurStep { expr: ExprId },
}

/// Presentation-free structural view of one trailing recurrence suffix inside a pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeRecurrenceSuffix<'a> {
    prefix_stage_count: usize,
    start_stage: &'a PipeStage,
    stages: &'a NonEmpty<PipeStage>,
}

impl<'a> PipeRecurrenceSuffix<'a> {
    pub fn prefix_stage_count(&self) -> usize {
        self.prefix_stage_count
    }

    pub fn prefix_stages(&self) -> impl Iterator<Item = &'a PipeStage> + 'a {
        self.stages.iter().take(self.prefix_stage_count)
    }

    pub fn start_stage(&self) -> &'a PipeStage {
        self.start_stage
    }

    pub fn start_expr(&self) -> ExprId {
        match &self.start_stage.kind {
            PipeStageKind::RecurStart { expr } => *expr,
            other => {
                unreachable!("validated recurrence suffixes must start with `@|>`, found {other:?}")
            }
        }
    }

    pub fn step_count(&self) -> usize {
        self.stages.len() - self.prefix_stage_count - 1
    }

    pub fn step_stages(&self) -> impl Iterator<Item = &'a PipeStage> + 'a {
        self.stages.iter().skip(self.prefix_stage_count + 1)
    }

    pub fn step_exprs(&self) -> impl Iterator<Item = ExprId> + 'a {
        self.step_stages().map(|stage| match &stage.kind {
            PipeStageKind::RecurStep { expr } => *expr,
            other => {
                unreachable!("validated recurrence suffix steps must use `<|@`, found {other:?}")
            }
        })
    }
}

/// Structural recurrence-shape error for raw HIR pipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeRecurrenceShapeError {
    OrphanStep {
        step_span: SourceSpan,
    },
    MissingStep {
        start_span: SourceSpan,
        continuation_span: Option<SourceSpan>,
    },
    TrailingStage {
        start_span: SourceSpan,
        stage_span: SourceSpan,
    },
}

impl fmt::Display for PipeRecurrenceShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrphanStep { .. } => {
                f.write_str("`<|@` appears without a preceding `@|>` recurrence start")
            }
            Self::MissingStep { .. } => {
                f.write_str("`@|>` is not followed by one or more `<|@` recurrence steps")
            }
            Self::TrailingStage { .. } => {
                f.write_str("a recurrent pipe suffix is followed by a non-`<|@` stage")
            }
        }
    }
}

impl Error for PipeRecurrenceShapeError {}

impl PipeExpr {
    pub fn recurrence_suffix(
        &self,
    ) -> Result<Option<PipeRecurrenceSuffix<'_>>, PipeRecurrenceShapeError> {
        let mut suffix_start: Option<(usize, &PipeStage)> = None;
        let mut saw_step = false;

        for (index, stage) in self.stages.iter().enumerate() {
            match (suffix_start, &stage.kind) {
                (None, PipeStageKind::RecurStart { .. }) => {
                    suffix_start = Some((index, stage));
                }
                (None, PipeStageKind::RecurStep { .. }) => {
                    return Err(PipeRecurrenceShapeError::OrphanStep {
                        step_span: stage.span,
                    });
                }
                (None, _) => {}
                (Some(_), PipeStageKind::RecurStep { .. }) => {
                    saw_step = true;
                }
                (Some((_, start_stage)), _) if !saw_step => {
                    return Err(PipeRecurrenceShapeError::MissingStep {
                        start_span: start_stage.span,
                        continuation_span: Some(stage.span),
                    });
                }
                (Some((_, start_stage)), _) => {
                    return Err(PipeRecurrenceShapeError::TrailingStage {
                        start_span: start_stage.span,
                        stage_span: stage.span,
                    });
                }
            }
        }

        match suffix_start {
            None => Ok(None),
            Some((_, start_stage)) if !saw_step => Err(PipeRecurrenceShapeError::MissingStep {
                start_span: start_stage.span,
                continuation_span: None,
            }),
            Some((prefix_stage_count, start_stage)) => Ok(Some(PipeRecurrenceSuffix {
                prefix_stage_count,
                start_stage,
                stages: &self.stages,
            })),
        }
    }
}

/// One pattern node owned by the module pattern arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub span: SourceSpan,
    pub kind: PatternKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Binding(BindingPattern),
    Integer(IntegerLiteral),
    Text(TextLiteral),
    Tuple(AtLeastTwo<PatternId>),
    Record(Vec<RecordPatternField>),
    Constructor {
        callee: TermReference,
        arguments: Vec<PatternId>,
    },
    UnresolvedName(TermReference),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingPattern {
    pub binding: BindingId,
    pub name: Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordPatternField {
    pub span: SourceSpan,
    pub label: Name,
    pub pattern: PatternId,
    pub surface: RecordFieldSurface,
}

/// One type expression node owned by the module type arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeNode {
    pub span: SourceSpan,
    pub kind: TypeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Name(TypeReference),
    Tuple(AtLeastTwo<TypeId>),
    Record(Vec<TypeField>),
    Arrow {
        parameter: TypeId,
        result: TypeId,
    },
    Apply {
        callee: TypeId,
        arguments: NonEmpty<TypeId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeField {
    pub span: SourceSpan,
    pub label: Name,
    pub ty: TypeId,
}

/// One attached decorator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decorator {
    pub span: SourceSpan,
    pub name: NamePath,
    pub payload: DecoratorPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecoratorPayload {
    Bare,
    Call(DecoratorCall),
    Source(SourceDecorator),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoratorCall {
    pub arguments: Vec<ExprId>,
    pub options: Option<ExprId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecorator {
    pub provider: Option<NamePath>,
    pub arguments: Vec<ExprId>,
    pub options: Option<ExprId>,
}

/// One markup node in the explicit HIR view tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupNode {
    pub span: SourceSpan,
    pub kind: MarkupNodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupNodeKind {
    Element(MarkupElement),
    Control(ControlNodeId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupElement {
    pub name: NamePath,
    pub attributes: Vec<MarkupAttribute>,
    pub children: Vec<MarkupNodeId>,
    pub close_name: Option<NamePath>,
    pub self_closing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupAttribute {
    pub span: SourceSpan,
    pub name: Name,
    pub value: MarkupAttributeValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupAttributeValue {
    ImplicitTrue,
    Text(TextLiteral),
    Expr(ExprId),
}

/// Explicit markup control-node family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlNode {
    Show(ShowControl),
    Each(EachControl),
    Empty(EmptyControl),
    Match(MatchControl),
    Case(CaseControl),
    Fragment(FragmentControl),
    With(WithControl),
}

/// Stable control-node discriminant used by validation and later lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlNodeKind {
    Show,
    Each,
    Empty,
    Match,
    Case,
    Fragment,
    With,
}

impl ControlNode {
    pub fn kind(&self) -> ControlNodeKind {
        match self {
            Self::Show(_) => ControlNodeKind::Show,
            Self::Each(_) => ControlNodeKind::Each,
            Self::Empty(_) => ControlNodeKind::Empty,
            Self::Match(_) => ControlNodeKind::Match,
            Self::Case(_) => ControlNodeKind::Case,
            Self::Fragment(_) => ControlNodeKind::Fragment,
            Self::With(_) => ControlNodeKind::With,
        }
    }

    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Show(node) => node.span,
            Self::Each(node) => node.span,
            Self::Empty(node) => node.span,
            Self::Match(node) => node.span,
            Self::Case(node) => node.span,
            Self::Fragment(node) => node.span,
            Self::With(node) => node.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowControl {
    pub span: SourceSpan,
    pub when: ExprId,
    pub keep_mounted: Option<ExprId>,
    pub children: Vec<MarkupNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EachControl {
    pub span: SourceSpan,
    pub collection: ExprId,
    pub binding: BindingId,
    pub key: Option<ExprId>,
    pub children: Vec<MarkupNodeId>,
    pub empty: Option<ControlNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchControl {
    pub span: SourceSpan,
    pub scrutinee: ExprId,
    pub cases: NonEmpty<ControlNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyControl {
    pub span: SourceSpan,
    pub children: Vec<MarkupNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseControl {
    pub span: SourceSpan,
    pub pattern: PatternId,
    pub children: Vec<MarkupNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentControl {
    pub span: SourceSpan,
    pub children: Vec<MarkupNodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithControl {
    pub span: SourceSpan,
    pub value: ExprId,
    pub binding: BindingId,
    pub children: Vec<MarkupNodeId>,
}

/// Explicit applicative-cluster node preserved through HIR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicativeCluster {
    pub span: SourceSpan,
    pub presentation: ClusterPresentation,
    pub members: AtLeastTwo<ExprId>,
    pub finalizer: ClusterFinalizer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClusterPresentation {
    ExpressionHeaded,
    Leading,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterFinalizer {
    Explicit(ExprId),
    ImplicitTuple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TupleConstructorArity(usize);

impl TupleConstructorArity {
    pub fn new(member_count: usize) -> Option<Self> {
        (member_count >= 2).then_some(Self(member_count))
    }

    pub fn get(self) -> usize {
        self.0
    }

    fn from_member_count(member_count: usize) -> Self {
        Self::new(member_count)
            .expect("applicative clusters always normalize to tuple arities of at least two")
    }
}

/// Presentation-free exact RFC §12.5/§12.6 normalization view of one `&|>` cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicativeSpine<'a> {
    pure_head: ApplicativeSpineHead,
    apply_arguments: &'a AtLeastTwo<ExprId>,
}

impl<'a> ApplicativeSpine<'a> {
    pub fn pure_head(&self) -> ApplicativeSpineHead {
        self.pure_head
    }

    pub fn apply_arguments(&self) -> impl Iterator<Item = ExprId> + '_ {
        self.apply_arguments.iter().copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplicativeSpineHead {
    Expr(ExprId),
    TupleConstructor(TupleConstructorArity),
}

impl ApplicativeCluster {
    pub fn normalized_spine(&self) -> ApplicativeSpine<'_> {
        let pure_head = match self.finalizer {
            ClusterFinalizer::Explicit(expr) => ApplicativeSpineHead::Expr(expr),
            ClusterFinalizer::ImplicitTuple => ApplicativeSpineHead::TupleConstructor(
                TupleConstructorArity::from_member_count(self.members.len()),
            ),
        };
        ApplicativeSpine {
            pure_head,
            apply_arguments: &self.members,
        }
    }
}

/// Grouped node arenas owned by one HIR module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleArenas {
    pub(crate) items: Arena<ItemId, Item>,
    pub(crate) exprs: Arena<ExprId, Expr>,
    pub(crate) patterns: Arena<PatternId, Pattern>,
    pub(crate) types: Arena<TypeId, TypeNode>,
    pub(crate) decorators: Arena<DecoratorId, Decorator>,
    pub(crate) markup_nodes: Arena<MarkupNodeId, MarkupNode>,
    pub(crate) control_nodes: Arena<ControlNodeId, ControlNode>,
    pub(crate) clusters: Arena<ClusterId, ApplicativeCluster>,
    pub(crate) bindings: Arena<BindingId, Binding>,
    pub(crate) type_parameters: Arena<TypeParameterId, TypeParameter>,
    pub(crate) imports: Arena<ImportId, ImportBinding>,
}

/// One validated-or-validatable HIR module boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub(crate) file: FileId,
    pub(crate) root_items: Vec<ItemId>,
    pub(crate) arenas: ModuleArenas,
}

/// Error returned when attempting to attach an invalid item as a root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootItemError {
    UnknownItem(ItemId),
    DuplicateItem(ItemId),
}

impl fmt::Display for RootItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownItem(id) => write!(f, "cannot attach unknown item {id} as a module root"),
            Self::DuplicateItem(id) => {
                write!(f, "item {id} is already present in the module root list")
            }
        }
    }
}

impl Error for RootItemError {}

impl Module {
    pub fn new(file: FileId) -> Self {
        Self {
            file,
            root_items: Vec::new(),
            arenas: ModuleArenas::default(),
        }
    }

    pub const fn file(&self) -> FileId {
        self.file
    }

    pub fn root_items(&self) -> &[ItemId] {
        &self.root_items
    }

    pub fn items(&self) -> &Arena<ItemId, Item> {
        &self.arenas.items
    }

    pub fn exprs(&self) -> &Arena<ExprId, Expr> {
        &self.arenas.exprs
    }

    pub fn patterns(&self) -> &Arena<PatternId, Pattern> {
        &self.arenas.patterns
    }

    pub fn types(&self) -> &Arena<TypeId, TypeNode> {
        &self.arenas.types
    }

    pub fn decorators(&self) -> &Arena<DecoratorId, Decorator> {
        &self.arenas.decorators
    }

    pub fn markup_nodes(&self) -> &Arena<MarkupNodeId, MarkupNode> {
        &self.arenas.markup_nodes
    }

    pub fn control_nodes(&self) -> &Arena<ControlNodeId, ControlNode> {
        &self.arenas.control_nodes
    }

    pub fn clusters(&self) -> &Arena<ClusterId, ApplicativeCluster> {
        &self.arenas.clusters
    }

    pub fn bindings(&self) -> &Arena<BindingId, Binding> {
        &self.arenas.bindings
    }

    pub fn type_parameters(&self) -> &Arena<TypeParameterId, TypeParameter> {
        &self.arenas.type_parameters
    }

    pub fn imports(&self) -> &Arena<ImportId, ImportBinding> {
        &self.arenas.imports
    }

    pub fn alloc_item(&mut self, item: Item) -> Result<ItemId, ArenaOverflow> {
        self.arenas.items.alloc(item)
    }

    pub fn push_item(&mut self, item: Item) -> Result<ItemId, ArenaOverflow> {
        let id = self.alloc_item(item)?;
        self.root_items.push(id);
        Ok(id)
    }

    pub fn append_root_item(&mut self, id: ItemId) -> Result<(), RootItemError> {
        if !self.arenas.items.contains(id) {
            return Err(RootItemError::UnknownItem(id));
        }
        if self.root_items.contains(&id) {
            return Err(RootItemError::DuplicateItem(id));
        }
        self.root_items.push(id);
        Ok(())
    }

    pub fn alloc_expr(&mut self, expr: Expr) -> Result<ExprId, ArenaOverflow> {
        self.arenas.exprs.alloc(expr)
    }

    pub fn alloc_pattern(&mut self, pattern: Pattern) -> Result<PatternId, ArenaOverflow> {
        self.arenas.patterns.alloc(pattern)
    }

    pub fn alloc_type(&mut self, ty: TypeNode) -> Result<TypeId, ArenaOverflow> {
        self.arenas.types.alloc(ty)
    }

    pub fn alloc_decorator(&mut self, decorator: Decorator) -> Result<DecoratorId, ArenaOverflow> {
        self.arenas.decorators.alloc(decorator)
    }

    pub fn alloc_markup_node(&mut self, node: MarkupNode) -> Result<MarkupNodeId, ArenaOverflow> {
        self.arenas.markup_nodes.alloc(node)
    }

    pub fn alloc_control_node(
        &mut self,
        node: ControlNode,
    ) -> Result<ControlNodeId, ArenaOverflow> {
        self.arenas.control_nodes.alloc(node)
    }

    pub fn alloc_cluster(
        &mut self,
        cluster: ApplicativeCluster,
    ) -> Result<ClusterId, ArenaOverflow> {
        self.arenas.clusters.alloc(cluster)
    }

    pub fn alloc_binding(&mut self, binding: Binding) -> Result<BindingId, ArenaOverflow> {
        self.arenas.bindings.alloc(binding)
    }

    pub fn alloc_type_parameter(
        &mut self,
        parameter: TypeParameter,
    ) -> Result<TypeParameterId, ArenaOverflow> {
        self.arenas.type_parameters.alloc(parameter)
    }

    pub fn alloc_import(&mut self, import: ImportBinding) -> Result<ImportId, ArenaOverflow> {
        self.arenas.imports.alloc(import)
    }

    pub fn validate(&self, mode: ValidationMode) -> ValidationReport {
        validate_module(self, mode)
    }
}
