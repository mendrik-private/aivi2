use aivi_base::{FileId, SourceSpan};

/// Token index range into the lossless token buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenRange {
    start: usize,
    end: usize,
}

impl TokenRange {
    pub fn new(start: usize, end: usize) -> Self {
        assert!(start <= end, "token range start must not exceed end");
        Self { start, end }
    }

    pub const fn start(self) -> usize {
        self.start
    }

    pub const fn end(self) -> usize {
        self.end
    }

    pub const fn len(self) -> usize {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Identifier copied out of the token buffer for later phases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifier {
    pub text: String,
    pub span: SourceSpan,
}

impl Identifier {
    pub fn is_uppercase_initial(&self) -> bool {
        self.text
            .chars()
            .next()
            .map(char::is_uppercase)
            .unwrap_or(false)
    }
}

/// Dotted name used by decorators and `use` declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualifiedName {
    pub segments: Vec<Identifier>,
    pub span: SourceSpan,
}

impl QualifiedName {
    pub fn as_dotted(&self) -> String {
        self.segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// Shared metadata for all CST nodes that preserve a source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPath {
    pub span: SourceSpan,
    pub fields: Vec<Identifier>,
}

/// Integer literal preserved in surface form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerLiteral {
    pub raw: String,
    pub span: SourceSpan,
}

/// Float literal preserved in surface form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatLiteral {
    pub raw: String,
    pub span: SourceSpan,
}

/// Decimal literal preserved in surface form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecimalLiteral {
    pub raw: String,
    pub span: SourceSpan,
}

/// BigInt literal preserved in surface form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigIntLiteral {
    pub raw: String,
    pub span: SourceSpan,
}

/// Integer literal immediately suffixed by a domain literal name, such as `250ms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuffixedIntegerLiteral {
    pub literal: IntegerLiteral,
    pub suffix: Identifier,
    pub span: SourceSpan,
}

/// Text literal preserved as explicit text fragments plus interpolation holes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLiteral {
    pub span: SourceSpan,
    pub segments: Vec<TextSegment>,
}

impl TextLiteral {
    pub fn has_interpolation(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment, TextSegment::Interpolation(_)))
    }
}

/// One structural text-literal segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSegment {
    Text(TextFragment),
    Interpolation(TextInterpolation),
}

/// Decoded text content between interpolation holes, without the surrounding quotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFragment {
    pub raw: String,
    pub span: SourceSpan,
}

/// One `{ ... }` interpolation hole inside a text literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInterpolation {
    pub expr: Box<Expr>,
    pub span: SourceSpan,
}

/// Regex literal preserved in surface form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexLiteral {
    pub raw: String,
    pub span: SourceSpan,
}

/// Value-level record field preserving shorthand and explicit forms.
///
/// `label_path` holds additional dotted segments beyond `label`.
/// For `{ address.city.name: value }`, `label` is `address` and
/// `label_path` is `[city, name]`.  Plain fields leave `label_path` empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordField {
    pub label: Identifier,
    pub label_path: Vec<Identifier>,
    pub value: Option<Expr>,
    pub span: SourceSpan,
}

/// Value-level closed record literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordExpr {
    pub fields: Vec<RecordField>,
    pub span: SourceSpan,
}

/// One key/value entry in a `Map { ... }` literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapExprEntry {
    pub key: Expr,
    pub value: Expr,
    pub span: SourceSpan,
}

/// Value-level `Map { ... }` literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapExpr {
    pub entries: Vec<MapExprEntry>,
    pub span: SourceSpan,
}

/// Type-level record field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeField {
    pub label: Identifier,
    pub ty: Option<TypeExpr>,
    pub span: SourceSpan,
}

/// One parsed type expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: SourceSpan,
}

/// Surface type forms preserved by the Milestone 1 CST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeExprKind {
    Name(Identifier),
    Group(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Record(Vec<TypeField>),
    Arrow {
        parameter: Box<TypeExpr>,
        result: Box<TypeExpr>,
    },
    Apply {
        callee: Box<TypeExpr>,
        arguments: Vec<TypeExpr>,
    },
}

/// One constructor branch in a `type` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeVariant {
    pub name: Option<Identifier>,
    pub fields: Vec<TypeVariantField>,
    pub span: SourceSpan,
}

/// A positional field in a constructor variant, with an optional label.
///
/// `| Date year:Year month:Month day:Day` has three named fields.
/// `| Vec2 Int Int` has two anonymous fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeVariantField {
    pub label: Option<Identifier>,
    pub ty: TypeExpr,
    pub span: SourceSpan,
}

/// One authored companion member colocated with a closed sum type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeCompanionMember {
    pub name: Identifier,
    pub annotation: Option<TypeExpr>,
    pub function_form: FunctionSurfaceForm,
    pub parameters: Vec<FunctionParam>,
    pub body: Option<Expr>,
    pub span: SourceSpan,
}

/// Body of a closed sum type, optionally including companion members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeSumBody {
    pub variants: Vec<TypeVariant>,
    pub companions: Vec<TypeCompanionMember>,
    pub span: SourceSpan,
}

/// Body of a `type` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeDeclBody {
    Alias(TypeExpr),
    Sum(TypeSumBody),
}

/// Parenthesized operator name preserved for class member declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorName {
    pub text: String,
    pub span: SourceSpan,
}

/// Class member name, which may be ordinary or operator-shaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassMemberName {
    Identifier(Identifier),
    Operator(OperatorName),
}

impl ClassMemberName {
    pub fn text(&self) -> &str {
        match self {
            Self::Identifier(identifier) => identifier.text.as_str(),
            Self::Operator(operator) => operator.text.as_str(),
        }
    }

    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Identifier(identifier) => identifier.span,
            Self::Operator(operator) => operator.span,
        }
    }
}

/// One class member signature preserved by the syntax layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassMember {
    pub name: ClassMemberName,
    pub constraints: Vec<TypeExpr>,
    pub annotation: Option<TypeExpr>,
    pub span: SourceSpan,
}

/// A `with SuperclassName TypeParam` declaration inside a class body.
/// Maps to a superclass in the HIR and is the canonical class-superclass syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassWithDecl {
    pub superclass: TypeExpr,
    pub span: SourceSpan,
}

/// A `require ClassName TypeParam` declaration inside a class body.
/// Constrains a class type parameter: any type instantiated for it must
/// satisfy the given class (e.g. `require Eq K` asserts K has Eq).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassRequireDecl {
    pub constraint: TypeExpr,
    pub span: SourceSpan,
}

/// Body of a `class` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassBody {
    pub with_decls: Vec<ClassWithDecl>,
    pub require_decls: Vec<ClassRequireDecl>,
    pub members: Vec<ClassMember>,
    pub span: SourceSpan,
}

/// One instance member binding preserved by the syntax layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceMember {
    pub name: ClassMemberName,
    pub parameters: Vec<Identifier>,
    pub body: Option<Expr>,
    pub span: SourceSpan,
}

/// Body of an `instance` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceBody {
    pub members: Vec<InstanceMember>,
    pub span: SourceSpan,
}

/// Domain member name, either an ordinary/operator signature or a literal suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainMemberName {
    Signature(ClassMemberName),
    Literal(Identifier),
}

impl DomainMemberName {
    pub fn text(&self) -> &str {
        match self {
            Self::Signature(name) => name.text(),
            Self::Literal(identifier) => identifier.text.as_str(),
        }
    }

    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Signature(name) => name.span(),
            Self::Literal(identifier) => identifier.span,
        }
    }
}

/// One domain-owned signature or literal declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainMember {
    pub name: DomainMemberName,
    pub annotation: Option<TypeExpr>,
    pub parameters: Vec<Identifier>,
    pub body: Option<Expr>,
    pub span: SourceSpan,
}

/// Body of a `domain` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainBody {
    pub members: Vec<DomainMember>,
    pub span: SourceSpan,
}

/// Prefix operators supported by the surface subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    Not,
}

/// Infix operators supported by the surface subset.
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

/// One expression node in the Milestone 1 CST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: SourceSpan,
}

impl Expr {
    /// Returns `true` when the expression tree contains an identifier reference
    /// named `"self"`, used to detect implicit domain receiver usage.
    pub fn contains_self_reference(&self) -> bool {
        expr_contains_self(self)
    }
}

fn expr_contains_self(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(id) => id.text == "self",
        ExprKind::Group(inner) => expr_contains_self(inner),
        ExprKind::Tuple(items) | ExprKind::List(items) | ExprKind::Set(items) => {
            items.iter().any(expr_contains_self)
        }
        ExprKind::Map(map) => map
            .entries
            .iter()
            .any(|e| expr_contains_self(&e.key) || expr_contains_self(&e.value)),
        ExprKind::Record(rec) => rec
            .fields
            .iter()
            .any(|f| f.value.as_ref().is_some_and(expr_contains_self)),
        ExprKind::Range { start, end } => {
            expr_contains_self(start) || expr_contains_self(end)
        }
        ExprKind::Projection { base, .. } => expr_contains_self(base),
        ExprKind::Apply {
            callee, arguments, ..
        } => expr_contains_self(callee) || arguments.iter().any(expr_contains_self),
        ExprKind::Unary { expr, .. } => expr_contains_self(expr),
        ExprKind::Binary { left, right, .. } => {
            expr_contains_self(left) || expr_contains_self(right)
        }
        ExprKind::ResultBlock(block) => {
            block
                .bindings
                .iter()
                .any(|b| expr_contains_self(&b.expr))
                || block.tail.as_deref().is_some_and(expr_contains_self)
        }
        ExprKind::PatchApply { target, patch } => {
            expr_contains_self(target) || patch_contains_self(patch)
        }
        ExprKind::PatchLiteral(patch) => patch_contains_self(patch),
        ExprKind::Pipe(pipe) => {
            pipe.head.as_deref().is_some_and(expr_contains_self)
                || pipe.stages.iter().any(|stage| match &stage.kind {
                    PipeStageKind::Transform { expr }
                    | PipeStageKind::Gate { expr }
                    | PipeStageKind::Map { expr }
                    | PipeStageKind::Apply { expr }
                    | PipeStageKind::ClusterFinalizer { expr }
                    | PipeStageKind::RecurStart { expr }
                    | PipeStageKind::RecurStep { expr }
                    | PipeStageKind::Tap { expr }
                    | PipeStageKind::FanIn { expr }
                    | PipeStageKind::Truthy { expr }
                    | PipeStageKind::Falsy { expr }
                    | PipeStageKind::Validate { expr }
                    | PipeStageKind::Previous { expr }
                    | PipeStageKind::Diff { expr }
                    | PipeStageKind::Delay { duration: expr } => expr_contains_self(expr),
                    PipeStageKind::Case(arm) => expr_contains_self(&arm.body),
                    PipeStageKind::Accumulate { seed, step } => {
                        expr_contains_self(seed) || expr_contains_self(step)
                    }
                    PipeStageKind::Burst { every, count } => {
                        expr_contains_self(every) || expr_contains_self(count)
                    }
                })
        }
        ExprKind::Text(text) => text.segments.iter().any(|seg| {
            matches!(seg, TextSegment::Interpolation(interp) if expr_contains_self(&interp.expr))
        }),
        ExprKind::Markup(node) => markup_contains_self(node),
        ExprKind::Integer(_)
        | ExprKind::Float(_)
        | ExprKind::Decimal(_)
        | ExprKind::BigInt(_)
        | ExprKind::SuffixedInteger(_)
        | ExprKind::Regex(_)
        | ExprKind::SubjectPlaceholder
        | ExprKind::AmbientProjection(_)
        | ExprKind::OperatorSection(_) => false,
    }
}

fn patch_contains_self(patch: &PatchBlock) -> bool {
    patch.entries.iter().any(|entry| {
        entry.selector.segments.iter().any(|seg| {
            matches!(seg, PatchSelectorSegment::BracketExpr { expr, .. } if expr_contains_self(expr))
        }) || match &entry.instruction.kind {
            PatchInstructionKind::Replace(e) | PatchInstructionKind::Store(e) => {
                expr_contains_self(e)
            }
            PatchInstructionKind::Remove => false,
        }
    })
}

fn markup_contains_self(node: &MarkupNode) -> bool {
    node.attributes.iter().any(
        |attr| matches!(&attr.value, Some(MarkupAttributeValue::Expr(e)) if expr_contains_self(e)),
    ) || node.children.iter().any(markup_contains_self)
}

/// One `<-` binding inside a `result { ... }` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultBinding {
    pub name: Identifier,
    pub expr: Expr,
    pub span: SourceSpan,
}

/// Block-shaped `result { ... }` expression preserved before HIR desugaring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultBlockExpr {
    pub bindings: Vec<ResultBinding>,
    pub tail: Option<Box<Expr>>,
    pub span: SourceSpan,
}

/// Reusable structural patch block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchBlock {
    pub entries: Vec<PatchEntry>,
    pub span: SourceSpan,
}

/// One structural patch entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchEntry {
    pub selector: PatchSelector,
    pub instruction: PatchInstruction,
    pub span: SourceSpan,
}

/// Selector path preserved before typing resolves whether named segments focus fields or constructors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchSelector {
    pub segments: Vec<PatchSelectorSegment>,
    pub span: SourceSpan,
}

/// One selector segment inside a patch selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchSelectorSegment {
    Named {
        name: Identifier,
        dotted: bool,
        span: SourceSpan,
    },
    BracketTraverse {
        span: SourceSpan,
    },
    BracketExpr {
        expr: Box<Expr>,
        span: SourceSpan,
    },
}

/// Terminal patch instruction preserved before later typing/lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchInstruction {
    pub kind: PatchInstructionKind,
    pub span: SourceSpan,
}

/// Patch instruction forms from the surface grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchInstructionKind {
    Replace(Box<Expr>),
    Store(Box<Expr>),
    Remove,
}

/// Surface expression forms exercised by the Milestone 1 fixture corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Name(Identifier),
    Integer(IntegerLiteral),
    Float(FloatLiteral),
    Decimal(DecimalLiteral),
    BigInt(BigIntLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(TextLiteral),
    Regex(RegexLiteral),
    Group(Box<Expr>),
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    Map(MapExpr),
    Set(Vec<Expr>),
    Record(RecordExpr),
    SubjectPlaceholder,
    AmbientProjection(ProjectionPath),
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    Projection {
        base: Box<Expr>,
        path: ProjectionPath,
    },
    Apply {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },
    Unary {
        operator: UnaryOperator,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
    /// `(op)` — a binary operator used as a first-class function value.
    OperatorSection(BinaryOperator),
    ResultBlock(ResultBlockExpr),
    PatchApply {
        target: Box<Expr>,
        patch: PatchBlock,
    },
    PatchLiteral(PatchBlock),
    Pipe(PipeExpr),
    Markup(MarkupNode),
}

/// Markup attribute value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupAttributeValue {
    Text(TextLiteral),
    Expr(Expr),
    Pattern(Pattern),
}

/// One markup attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupAttribute {
    pub name: Identifier,
    pub value: Option<MarkupAttributeValue>,
    pub span: SourceSpan,
}

/// Markup/widget node skeleton preserved by the syntax layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupNode {
    pub name: QualifiedName,
    pub attributes: Vec<MarkupAttribute>,
    pub children: Vec<MarkupNode>,
    pub close_name: Option<QualifiedName>,
    pub self_closing: bool,
    pub span: SourceSpan,
}

/// Pipe match arm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeCaseArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: SourceSpan,
}

/// One pipe stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeStage {
    pub subject_memo: Option<Identifier>,
    pub result_memo: Option<Identifier>,
    pub kind: PipeStageKind,
    pub span: SourceSpan,
}

/// Pipe stage variants exercised by the Milestone 1 fixture corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipeStageKind {
    Transform { expr: Expr },
    Gate { expr: Expr },
    Case(PipeCaseArm),
    Map { expr: Expr },
    Apply { expr: Expr },
    ClusterFinalizer { expr: Expr },
    RecurStart { expr: Expr },
    RecurStep { expr: Expr },
    Tap { expr: Expr },
    FanIn { expr: Expr },
    Truthy { expr: Expr },
    Falsy { expr: Expr },
    Validate { expr: Expr },
    Previous { expr: Expr },
    Accumulate { seed: Expr, step: Expr },
    Diff { expr: Expr },
    Delay { duration: Expr },
    Burst { every: Expr, count: Expr },
}

/// Pipe spine with an optional leading subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeExpr {
    pub head: Option<Box<Expr>>,
    pub stages: Vec<PipeStage>,
    pub span: SourceSpan,
}

/// Record-pattern field preserving shorthand and explicit forms.
///
/// `label_path` holds additional dotted segments beyond `label`.
/// For `{ address.city.name }`, `label` is `address` and
/// `label_path` is `[city, name]`.  Plain fields leave `label_path` empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordPatternField {
    pub label: Identifier,
    pub label_path: Vec<Identifier>,
    pub pattern: Option<Pattern>,
    pub span: SourceSpan,
}

/// One pattern node used by pipe cases and markup control nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: SourceSpan,
}

/// Pattern forms required by the Milestone 1 fixture corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Name(Identifier),
    Integer(IntegerLiteral),
    Text(TextLiteral),
    Group(Box<Pattern>),
    Tuple(Vec<Pattern>),
    List {
        elements: Vec<Pattern>,
        rest: Option<Box<Pattern>>,
    },
    Record(Vec<RecordPatternField>),
    Apply {
        callee: Box<Pattern>,
        arguments: Vec<Pattern>,
    },
}

/// Decorator payload with source-specific structure preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecoratorPayload {
    Bare,
    Source(SourceDecorator),
    Arguments(DecoratorArguments),
}

/// Generic decorator invocation payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoratorArguments {
    pub arguments: Vec<Expr>,
    pub options: Option<RecordExpr>,
}

/// `@source` payload preserving provider, arguments, and `with { ... }` options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecorator {
    pub provider: Option<QualifiedName>,
    pub arguments: Vec<Expr>,
    pub options: Option<RecordExpr>,
}

/// Leading decorator header attached to a top-level item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decorator {
    pub name: QualifiedName,
    pub span: SourceSpan,
    pub payload: DecoratorPayload,
}

/// Shared top-level item metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemBase {
    pub span: SourceSpan,
    pub token_range: TokenRange,
    pub decorators: Vec<Decorator>,
    /// Line comments (including `//` prefix) that appear immediately before this item.
    pub leading_comments: Vec<String>,
}

/// Function parameter preserved by the syntax layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionParam {
    pub name: Option<Identifier>,
    pub annotation: Option<TypeExpr>,
    pub span: SourceSpan,
}

/// Surface spelling used for function declarations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FunctionSurfaceForm {
    #[default]
    Explicit,
    /// `func name = .`, `func name = .field`, or `func name = "Hello {.}"`.
    UnarySubjectSugar,
}

/// Shared body forms for named top-level items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedItemBody {
    Expr(Expr),
    Type(TypeDeclBody),
    Class(ClassBody),
    Instance(InstanceBody),
    Merge(SignalMergeBody),
}

/// Shared representation for named top-level items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub name: Option<Identifier>,
    pub type_parameters: Vec<Identifier>,
    pub constraints: Vec<TypeExpr>,
    pub annotation: Option<TypeExpr>,
    pub function_form: FunctionSurfaceForm,
    pub parameters: Vec<FunctionParam>,
    pub body: Option<NamedItemBody>,
}

impl NamedItem {
    pub fn expr_body(&self) -> Option<&Expr> {
        match &self.body {
            Some(NamedItemBody::Expr(expr)) => Some(expr),
            _ => None,
        }
    }

    pub fn type_body(&self) -> Option<&TypeDeclBody> {
        match &self.body {
            Some(NamedItemBody::Type(body)) => Some(body),
            _ => None,
        }
    }

    pub fn class_body(&self) -> Option<&ClassBody> {
        match &self.body {
            Some(NamedItemBody::Class(body)) => Some(body),
            _ => None,
        }
    }

    pub fn instance_body(&self) -> Option<&InstanceBody> {
        match &self.body {
            Some(NamedItemBody::Instance(body)) => Some(body),
            _ => None,
        }
    }

    pub fn merge_body(&self) -> Option<&SignalMergeBody> {
        match &self.body {
            Some(NamedItemBody::Merge(body)) => Some(body),
            _ => None,
        }
    }
}

/// One reactive arm on a signal declaration with merge sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalReactiveArm {
    /// Source signal identifier in multi-source arms, `None` for default/single-source.
    pub source: Option<Identifier>,
    pub pattern: Option<Pattern>,
    pub body: Option<Expr>,
    pub span: SourceSpan,
}

/// Signal body that merges one or more source signals and pattern-matches with `||>` arms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalMergeBody {
    /// Source signals participating in the merge (at least 1).
    pub sources: Vec<Identifier>,
    /// Reactive arms that pattern-match on the merged sources.
    pub arms: Vec<SignalReactiveArm>,
    pub span: SourceSpan,
}

/// `instance` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub context: Vec<TypeExpr>,
    pub class: Option<QualifiedName>,
    pub target: Option<TypeExpr>,
    pub body: Option<InstanceBody>,
}

/// `use` declaration with an optional module path and import list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseImport {
    pub path: QualifiedName,
    pub alias: Option<Identifier>,
}

/// `use` declaration with an optional module path and import list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub path: Option<QualifiedName>,
    pub imports: Vec<UseImport>,
}

/// One entry inside a `from source = { ... }` fan-out block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromEntry {
    pub name: Identifier,
    pub body: Option<Expr>,
    pub span: SourceSpan,
}

/// `from source = { name: expr ... }` sugar for grouped derived signals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub source: Option<Expr>,
    pub entries: Vec<FromEntry>,
}

/// `export` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub targets: Vec<Identifier>,
}

/// Kind filter in a `hoist` declaration (e.g. `func`, `value`, `type`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoistKindFilter {
    pub span: SourceSpan,
    pub text: String,
}

/// `hoist` declaration — lifts this module's own exports into the project-wide global scope.
///
/// Syntax:
/// ```aivi
/// hoist
/// hoist (func)
/// hoist (func, value)
/// hoist hiding (foo)
/// hoist (func) hiding (foo)
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoistItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    /// Optional kind filters — if empty, all kinds are hoisted.
    pub kind_filters: Vec<HoistKindFilter>,
    /// Names explicitly excluded from the hoist.
    pub hiding: Vec<Identifier>,
}

/// `domain` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub name: Option<Identifier>,
    pub type_parameters: Vec<Identifier>,
    pub carrier: Option<TypeExpr>,
    pub body: Option<DomainBody>,
}

/// One untyped `provider` contract field such as `wakeup: providerTrigger`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProviderContractFieldValue {
    pub span: SourceSpan,
    pub name: Option<Identifier>,
    pub value: Option<Identifier>,
}

/// One typed custom-provider contract schema member such as `option timeout: Duration`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProviderContractSchemaMember {
    pub span: SourceSpan,
    pub name: Option<Identifier>,
    pub annotation: Option<TypeExpr>,
}

/// One `provider` contract member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderContractMember {
    FieldValue(SourceProviderContractFieldValue),
    OptionSchema(SourceProviderContractSchemaMember),
    ArgumentSchema(SourceProviderContractSchemaMember),
    OperationSchema(SourceProviderContractSchemaMember),
    CommandSchema(SourceProviderContractSchemaMember),
}

/// Body of a `provider` contract declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProviderContractBody {
    pub span: SourceSpan,
    pub members: Vec<SourceProviderContractMember>,
}

/// `provider` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProviderContractItem {
    pub base: ItemBase,
    pub keyword_span: SourceSpan,
    pub provider: Option<QualifiedName>,
    pub body: Option<SourceProviderContractBody>,
}

/// Error recovery item that still preserves source coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorItem {
    pub base: ItemBase,
    pub message: String,
}

/// Top-level CST for Milestone 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Type(NamedItem),
    Fun(NamedItem),
    Value(NamedItem),
    Signal(NamedItem),
    From(FromItem),
    Class(NamedItem),
    Instance(InstanceItem),
    Domain(DomainItem),
    SourceProviderContract(SourceProviderContractItem),
    Use(UseItem),
    Export(ExportItem),
    Hoist(HoistItem),
    Error(ErrorItem),
}

/// Stable item discriminant used by tooling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Type,
    Fun,
    Value,
    Signal,
    From,
    Class,
    Instance,
    Domain,
    SourceProviderContract,
    Use,
    Export,
    Hoist,
    Error,
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Item::Type(_) => ItemKind::Type,
            Item::Fun(_) => ItemKind::Fun,
            Item::Value(_) => ItemKind::Value,
            Item::Signal(_) => ItemKind::Signal,
            Item::From(_) => ItemKind::From,
            Item::Class(_) => ItemKind::Class,
            Item::Instance(_) => ItemKind::Instance,
            Item::Domain(_) => ItemKind::Domain,
            Item::SourceProviderContract(_) => ItemKind::SourceProviderContract,
            Item::Use(_) => ItemKind::Use,
            Item::Export(_) => ItemKind::Export,
            Item::Hoist(_) => ItemKind::Hoist,
            Item::Error(_) => ItemKind::Error,
        }
    }

    pub fn base(&self) -> &ItemBase {
        match self {
            Item::Type(item)
            | Item::Fun(item)
            | Item::Value(item)
            | Item::Signal(item)
            | Item::Class(item) => &item.base,
            Item::From(item) => &item.base,
            Item::Instance(item) => &item.base,
            Item::Domain(item) => &item.base,
            Item::SourceProviderContract(item) => &item.base,
            Item::Use(item) => &item.base,
            Item::Export(item) => &item.base,
            Item::Hoist(item) => &item.base,
            Item::Error(item) => &item.base,
        }
    }

    pub fn span(&self) -> SourceSpan {
        self.base().span
    }

    pub fn token_range(&self) -> TokenRange {
        self.base().token_range
    }

    pub fn base_mut(&mut self) -> &mut ItemBase {
        match self {
            Item::Type(item)
            | Item::Fun(item)
            | Item::Value(item)
            | Item::Signal(item)
            | Item::Class(item) => &mut item.base,
            Item::From(item) => &mut item.base,
            Item::Instance(item) => &mut item.base,
            Item::Domain(item) => &mut item.base,
            Item::SourceProviderContract(item) => &mut item.base,
            Item::Use(item) => &mut item.base,
            Item::Export(item) => &mut item.base,
            Item::Hoist(item) => &mut item.base,
            Item::Error(item) => &mut item.base,
        }
    }

    pub fn decorators(&self) -> &[Decorator] {
        &self.base().decorators
    }
}

/// Parsed source module coordinated with the lossless token buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub file: FileId,
    pub items: Vec<Item>,
    pub token_count: usize,
}

impl Module {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
