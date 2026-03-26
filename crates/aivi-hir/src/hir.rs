use std::{error::Error, fmt};

use aivi_base::{FileId, SourceSpan};
use aivi_typing::{BuiltinSourceProvider, Kind};

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
    pub resolution: ImportBindingResolution,
    pub metadata: ImportBindingMetadata,
}

/// Resolution outcome for one imported binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportBindingResolution {
    Resolved,
    UnknownModule,
    MissingExport,
    Cycle,
}

/// Resolved destination for one domain-owned term member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DomainMemberResolution {
    pub domain: ItemId,
    pub member_index: usize,
}

/// Stable semantic handle for one domain-owned callable surfaced past HIR elaboration.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DomainMemberHandle {
    pub domain_name: Box<str>,
    pub member_name: Box<str>,
    pub member_index: usize,
}

/// Stable semantic handle for one class-owned callable surfaced past HIR elaboration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassMemberResolution {
    pub class: ItemId,
    pub member_index: usize,
}

/// Stable semantic handle for one same-module closed-sum constructor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SumConstructorHandle {
    pub item: ItemId,
    pub type_name: Box<str>,
    pub variant_name: Box<str>,
    pub field_count: usize,
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

/// Compiler-known stdlib values that lower through dedicated runtime seams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntrinsicValue {
    RandomInt,
    RandomBytes,
    StdoutWrite,
    StderrWrite,
    FsWriteText,
    FsWriteBytes,
    FsCreateDirAll,
    FsDeleteFile,
}

impl fmt::Display for IntrinsicValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RandomInt => f.write_str("aivi.random.randomInt"),
            Self::RandomBytes => f.write_str("aivi.random.randomBytes"),
            Self::StdoutWrite => f.write_str("aivi.stdio.stdoutWrite"),
            Self::StderrWrite => f.write_str("aivi.stdio.stderrWrite"),
            Self::FsWriteText => f.write_str("aivi.fs.writeText"),
            Self::FsWriteBytes => f.write_str("aivi.fs.writeBytes"),
            Self::FsCreateDirAll => f.write_str("aivi.fs.createDirAll"),
            Self::FsDeleteFile => f.write_str("aivi.fs.deleteFile"),
        }
    }
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
    Map,
    Set,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

/// Compiler-known facts preserved for one imported binding.
///
/// Milestone 2 imports still resolve through a small closed catalog. This metadata keeps the
/// proven value/type surface explicit so later validation can use import facts without re-parsing
/// module/member strings or guessing missing type information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportBindingMetadata {
    Unknown,
    Value {
        ty: ImportValueType,
    },
    IntrinsicValue {
        value: IntrinsicValue,
        ty: ImportValueType,
    },
    OpaqueValue,
    AmbientValue {
        name: Box<str>,
    },
    TypeConstructor {
        kind: Kind,
    },
    BuiltinType(BuiltinType),
    BuiltinTerm(BuiltinTerm),
    AmbientType,
    Bundle(ImportBundleKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportBundleKind {
    BuiltinOption,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRecordField {
    pub name: Box<str>,
    pub ty: ImportValueType,
}

/// Closed imported value-type surface that HIR can use before real module-linked nominal typing
/// exists.
///
/// This intentionally mirrors only the shapes the current validator can lower directly into
/// `GateType` without cross-module item identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportValueType {
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<ImportRecordField>),
    Arrow {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    List(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
    },
    Set(Box<Self>),
    Option(Box<Self>),
    Result {
        error: Box<Self>,
        value: Box<Self>,
    },
    Validation {
        error: Box<Self>,
        value: Box<Self>,
    },
    Signal(Box<Self>),
    Task {
        error: Box<Self>,
        value: Box<Self>,
    },
}

/// Resolved destination for a term-level reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TermResolution {
    Local(BindingId),
    Item(ItemId),
    Import(ImportId),
    IntrinsicValue(IntrinsicValue),
    DomainMember(DomainMemberResolution),
    AmbiguousDomainMembers(NonEmpty<DomainMemberResolution>),
    ClassMember(ClassMemberResolution),
    AmbiguousClassMembers(NonEmpty<ClassMemberResolution>),
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
    SourceProviderContract,
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
    SourceProviderContract(SourceProviderContractItem),
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
            Self::SourceProviderContract(_) => ItemKind::SourceProviderContract,
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
            Self::SourceProviderContract(item) => item.header.span,
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
            Self::SourceProviderContract(item) => &item.header.decorators,
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
    pub type_parameters: Vec<TypeParameterId>,
    pub context: Vec<TypeId>,
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
    pub provider: SourceProviderRef,
    pub signal_dependencies: Vec<ItemId>,
    pub lifecycle_dependencies: SourceLifecycleDependencies,
    pub is_reactive: bool,
    pub custom_contract: Option<CustomSourceContractMetadata>,
}

impl SourceMetadata {
    pub fn has_reactive_wakeup_inputs(&self) -> bool {
        self.lifecycle_dependencies.has_reactive_wakeup_inputs()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceLifecycleDependencies {
    pub reconfiguration: Vec<ItemId>,
    pub explicit_triggers: Vec<ItemId>,
    pub active_when: Vec<ItemId>,
}

impl SourceLifecycleDependencies {
    pub fn has_reactive_wakeup_inputs(&self) -> bool {
        !self.reconfiguration.is_empty() || !self.active_when.is_empty()
    }

    pub fn merged(&self) -> Vec<ItemId> {
        let mut dependencies = Vec::new();
        dependencies.extend(self.reconfiguration.iter().copied());
        dependencies.extend(self.explicit_triggers.iter().copied());
        dependencies.extend(self.active_when.iter().copied());
        dependencies.sort();
        dependencies.dedup();
        dependencies
    }
}

/// Typed `@source` provider identity preserved into HIR.
///
/// This keeps built-in vs custom vs malformed provider paths explicit so later validation and
/// future contract resolution do not have to repeatedly re-parse raw strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceProviderRef {
    Missing,
    Builtin(BuiltinSourceProvider),
    Custom(Box<str>),
    InvalidShape(Box<str>),
}

impl SourceProviderRef {
    pub fn from_path(path: Option<&NamePath>) -> Self {
        let Some(path) = path else {
            return Self::Missing;
        };
        let key = path
            .segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".")
            .into_boxed_str();
        if path.segments().len() < 2 {
            return Self::InvalidShape(key);
        }
        match BuiltinSourceProvider::parse(key.as_ref()) {
            Some(provider) => Self::Builtin(provider),
            None => Self::Custom(key),
        }
    }

    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Missing => None,
            Self::Builtin(provider) => Some(provider.key()),
            Self::Custom(key) | Self::InvalidShape(key) => Some(key.as_ref()),
        }
    }

    pub fn builtin(&self) -> Option<BuiltinSourceProvider> {
        match self {
            Self::Builtin(provider) => Some(*provider),
            Self::Missing | Self::Custom(_) | Self::InvalidShape(_) => None,
        }
    }

    pub fn custom_key(&self) -> Option<&str> {
        match self {
            Self::Custom(key) => Some(key.as_ref()),
            Self::Missing | Self::Builtin(_) | Self::InvalidShape(_) => None,
        }
    }
}

/// Resolved custom-provider facts carried at one `@source` use site.
///
/// This keeps provider-local recurrence and schema facts together so later custom-provider typing
/// can reuse one explicit carrier instead of extending source metadata with ad-hoc optional fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomSourceContractMetadata {
    pub recurrence_wakeup: Option<CustomSourceRecurrenceWakeup>,
    pub arguments: Vec<CustomSourceArgumentSchema>,
    pub options: Vec<CustomSourceOptionSchema>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CustomSourceRecurrenceWakeup {
    Timer,
    Backoff,
    SourceEvent,
    ProviderDefinedTrigger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSourceArgumentSchema {
    pub span: SourceSpan,
    pub name: Name,
    pub annotation: TypeId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSourceOptionSchema {
    pub span: SourceSpan,
    pub name: Name,
    pub annotation: TypeId,
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
    pub type_parameters: Vec<TypeParameterId>,
    pub context: Vec<TypeId>,
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

/// One `provider` contract declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProviderContractItem {
    pub header: ItemHeader,
    pub provider: SourceProviderRef,
    pub contract: CustomSourceContractMetadata,
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
    pub type_parameters: Vec<TypeParameterId>,
    pub context: Vec<TypeId>,
    pub members: Vec<InstanceMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceMember {
    pub span: SourceSpan,
    pub name: Name,
    pub parameters: Vec<FunctionParameter>,
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
    pub resolution: ResolutionState<ExportResolution>,
}

/// Resolved destination for one explicit `export` target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportResolution {
    Item(ItemId),
    BuiltinTerm(BuiltinTerm),
    BuiltinType(BuiltinType),
}

/// One integer literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerLiteral {
    pub raw: Box<str>,
}

/// One float literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatLiteral {
    pub raw: Box<str>,
}

/// One decimal literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecimalLiteral {
    pub raw: Box<str>,
}

/// One BigInt literal preserved in raw form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigIntLiteral {
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
    Multiply,
    Divide,
    Modulo,
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
    Float(FloatLiteral),
    Decimal(DecimalLiteral),
    BigInt(BigIntLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(TextLiteral),
    Regex(RegexLiteral),
    Tuple(AtLeastTwo<ExprId>),
    List(Vec<ExprId>),
    Map(MapExpr),
    Set(Vec<ExprId>),
    Record(RecordExpr),
    AmbientSubject,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapExpr {
    pub entries: Vec<MapExprEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapExprEntry {
    pub span: SourceSpan,
    pub key: ExprId,
    pub value: ExprId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordFieldSurface {
    Explicit,
    Shorthand,
    Defaulted,
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

/// Typed runtime semantics for one `|>` transform stage after elaboration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PipeTransformMode {
    /// Evaluate the stage as a callable transform over the current subject.
    Apply,
    /// Evaluate the stage as a value and replace the current subject with it.
    Replace,
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

/// Presentation-free structural view of one fan-out segment inside a pipe.
///
/// The current supported joined segment shape is:
/// - one `*|>` map stage,
/// - followed by zero or more `?|>` filter stages,
/// - optionally closed by one `<|*` join stage.
///
/// If no `<|*` follows the filter run, the segment remains a plain one-stage `*|>` map and the
/// filter count is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeFanoutSegment<'a> {
    map_stage_index: usize,
    map_stage: &'a PipeStage,
    filter_stage_count: usize,
    join_stage: Option<(usize, &'a PipeStage)>,
    stages: &'a NonEmpty<PipeStage>,
}

impl<'a> PipeFanoutSegment<'a> {
    pub fn map_stage_index(&self) -> usize {
        self.map_stage_index
    }

    pub fn map_stage(&self) -> &'a PipeStage {
        self.map_stage
    }

    pub fn map_expr(&self) -> ExprId {
        match &self.map_stage.kind {
            PipeStageKind::Map { expr } => *expr,
            other => {
                unreachable!("validated fan-out segments must start with `*|>`, found {other:?}")
            }
        }
    }

    pub fn filter_stage_count(&self) -> usize {
        self.filter_stage_count
    }

    pub fn filter_stages(&self) -> impl Iterator<Item = &'a PipeStage> + 'a {
        self.stages
            .iter()
            .skip(self.map_stage_index + 1)
            .take(self.filter_stage_count)
    }

    pub fn filter_exprs(&self) -> impl Iterator<Item = ExprId> + 'a {
        self.filter_stages().map(|stage| match &stage.kind {
            PipeStageKind::Gate { expr } => *expr,
            other => unreachable!("validated fan-out filters must use `?|>`, found {other:?}"),
        })
    }

    pub fn join_stage_index(&self) -> Option<usize> {
        self.join_stage.map(|(index, _)| index)
    }

    pub fn join_stage(&self) -> Option<&'a PipeStage> {
        self.join_stage.map(|(_, stage)| stage)
    }

    pub fn join_expr(&self) -> Option<ExprId> {
        self.join_stage().map(|stage| match &stage.kind {
            PipeStageKind::FanIn { expr } => *expr,
            other => unreachable!("validated fan-out joins must use `<|*`, found {other:?}"),
        })
    }

    pub fn next_stage_index(&self) -> usize {
        self.join_stage_index()
            .map_or(self.map_stage_index + 1, |index| index + 1)
    }
}

/// Presentation-free structural view of one trailing recurrence suffix inside a pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeRecurrenceSuffix<'a> {
    prefix_stage_count: usize,
    start_stage: &'a PipeStage,
    guard_stage_count: usize,
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

    pub fn guard_stage_count(&self) -> usize {
        self.guard_stage_count
    }

    pub fn guard_stages(&self) -> impl Iterator<Item = &'a PipeStage> + 'a {
        self.stages
            .iter()
            .skip(self.prefix_stage_count + 1)
            .take(self.guard_stage_count)
    }

    pub fn guard_exprs(&self) -> impl Iterator<Item = ExprId> + 'a {
        self.guard_stages().map(|stage| match &stage.kind {
            PipeStageKind::Gate { expr } => *expr,
            other => unreachable!("validated recurrence guards must use `?|>`, found {other:?}"),
        })
    }

    pub fn step_count(&self) -> usize {
        self.stages.len() - self.prefix_stage_count - 1 - self.guard_stage_count
    }

    pub fn step_stages(&self) -> impl Iterator<Item = &'a PipeStage> + 'a {
        self.stages
            .iter()
            .skip(self.prefix_stage_count + 1 + self.guard_stage_count)
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
    pub fn fanout_segment(&self, map_stage_index: usize) -> Option<PipeFanoutSegment<'_>> {
        let stages = self.stages.iter().collect::<Vec<_>>();
        let map_stage = stages.get(map_stage_index).copied()?;
        if !matches!(map_stage.kind, PipeStageKind::Map { .. }) {
            return None;
        }

        let mut index = map_stage_index + 1;
        while index < stages.len() && matches!(stages[index].kind, PipeStageKind::Gate { .. }) {
            index += 1;
        }
        let join_stage = stages.get(index).and_then(|stage| match &stage.kind {
            PipeStageKind::FanIn { .. } => Some((index, *stage)),
            _ => None,
        });

        Some(PipeFanoutSegment {
            map_stage_index,
            map_stage,
            filter_stage_count: if join_stage.is_some() {
                index - (map_stage_index + 1)
            } else {
                0
            },
            join_stage,
            stages: &self.stages,
        })
    }

    pub fn recurrence_suffix(
        &self,
    ) -> Result<Option<PipeRecurrenceSuffix<'_>>, PipeRecurrenceShapeError> {
        let mut suffix_start: Option<(usize, &PipeStage)> = None;
        let mut guard_stage_count = 0usize;
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
                (Some(_), PipeStageKind::Gate { .. }) if !saw_step => {
                    guard_stage_count += 1;
                }
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
                guard_stage_count,
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
    List {
        elements: Vec<PatternId>,
        rest: Option<PatternId>,
    },
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
    RecurrenceWakeup(RecurrenceWakeupDecorator),
    Source(SourceDecorator),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoratorCall {
    pub arguments: Vec<ExprId>,
    pub options: Option<ExprId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceWakeupDecoratorKind {
    Timer,
    Backoff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceWakeupDecorator {
    pub kind: RecurrenceWakeupDecoratorKind,
    pub witness: ExprId,
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
    pub(crate) ambient_items: Vec<ItemId>,
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
            ambient_items: Vec::new(),
            arenas: ModuleArenas::default(),
        }
    }

    /// Create a valid but empty module. Used as a cycle-recovery placeholder.
    pub fn empty() -> Self {
        Self::default()
    }

    pub const fn file(&self) -> FileId {
        self.file
    }

    pub fn root_items(&self) -> &[ItemId] {
        &self.root_items
    }

    pub fn ambient_items(&self) -> &[ItemId] {
        &self.ambient_items
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

    pub fn domain_member_handle(
        &self,
        resolution: DomainMemberResolution,
    ) -> Option<DomainMemberHandle> {
        let Item::Domain(domain) = self.arenas.items.get(resolution.domain)? else {
            return None;
        };
        let member = domain.members.get(resolution.member_index)?;
        Some(DomainMemberHandle {
            domain_name: domain.name.text().into(),
            member_name: member.name.text().into(),
            member_index: resolution.member_index,
        })
    }

    pub fn sum_constructor_handle(
        &self,
        item: ItemId,
        variant_name: &str,
    ) -> Option<SumConstructorHandle> {
        let Item::Type(type_item) = self.arenas.items.get(item)? else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &type_item.body else {
            return None;
        };
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        Some(SumConstructorHandle {
            item,
            type_name: type_item.name.text().into(),
            variant_name: variant.name.text().into(),
            field_count: variant.fields.len(),
        })
    }

    pub fn alloc_item(&mut self, item: Item) -> Result<ItemId, ArenaOverflow> {
        self.arenas.items.alloc(item)
    }

    pub fn push_item(&mut self, item: Item) -> Result<ItemId, ArenaOverflow> {
        let id = self.alloc_item(item)?;
        self.root_items.push(id);
        Ok(id)
    }

    pub fn push_ambient_item(&mut self, item: Item) -> Result<ItemId, ArenaOverflow> {
        let id = self.alloc_item(item)?;
        self.ambient_items.push(id);
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
