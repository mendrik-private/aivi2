use std::fmt;

use aivi_base::SourceSpan;
use aivi_hir::{
    BuiltinTerm, ExprId as HirExprId, ItemId as HirItemId, SourceProviderRef,
    SourceReplacementPolicy, SourceStaleWorkPolicy, SourceTeardownPolicy, TypeId as HirTypeId,
};
use aivi_typing::{
    DecodeExtraFieldPolicy, DecodeFieldRequirement, DecodeMode, DecodeSumStrategy, FanoutCarrier,
    NonSourceWakeupCause, PrimitiveType, RecurrencePlan, RecurrenceWakeupPlan,
    SourceCancellationPolicy,
};

use crate::{
    Arena,
    expr::{
        Expr, Pattern, PatternBinding, PatternConstructor, PatternKind, PipeCaseArm,
        PipeTruthyFalsyStage, Reference,
    },
    ids::ExprId,
    ids::{DecodeProgramId, DecodeStepId, ItemId, PipeId, SourceId, StageId},
    ty::Type,
};

/// # Expression Closure Invariant
///
/// All expressions stored in this module's expression arena are assumed to be
/// *closed*: they contain no free variable references (`Reference::Local`)
/// that are not bound by the expression's own enclosing patterns or parameters.
///
/// This invariant is NOT enforced at construction time. It is checked
/// post-hoc during lambda lowering by `capture_free_bindings()` in `aivi-lambda`.
/// A violation will only be detected when lambda lowering processes the offending
/// expression.
///
/// # Arena ID Ownership
///
/// Arena IDs (`ExprId`, `ItemId`, etc.) from this module MUST NOT be used to
/// index arenas from a different `Module` instance. The type system does not
/// prevent this — it is enforced by convention only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    items: Arena<ItemId, Item>,
    pipes: Arena<PipeId, Pipe>,
    stages: Arena<StageId, Stage>,
    exprs: Arena<ExprId, Expr>,
    sources: Arena<SourceId, SourceNode>,
    decode_programs: Arena<DecodeProgramId, DecodeProgram>,
}

impl Default for Module {
    fn default() -> Self {
        Self {
            items: Arena::new(),
            pipes: Arena::new(),
            stages: Arena::new(),
            exprs: Arena::new(),
            sources: Arena::new(),
            decode_programs: Arena::new(),
        }
    }
}

impl Module {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &Arena<ItemId, Item> {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Arena<ItemId, Item> {
        &mut self.items
    }

    pub fn pipes(&self) -> &Arena<PipeId, Pipe> {
        &self.pipes
    }

    pub fn pipes_mut(&mut self) -> &mut Arena<PipeId, Pipe> {
        &mut self.pipes
    }

    pub fn stages(&self) -> &Arena<StageId, Stage> {
        &self.stages
    }

    pub fn stages_mut(&mut self) -> &mut Arena<StageId, Stage> {
        &mut self.stages
    }

    pub fn exprs(&self) -> &Arena<ExprId, Expr> {
        &self.exprs
    }

    pub fn exprs_mut(&mut self) -> &mut Arena<ExprId, Expr> {
        &mut self.exprs
    }

    pub fn sources(&self) -> &Arena<SourceId, SourceNode> {
        &self.sources
    }

    pub fn sources_mut(&mut self) -> &mut Arena<SourceId, SourceNode> {
        &mut self.sources
    }

    pub fn decode_programs(&self) -> &Arena<DecodeProgramId, DecodeProgram> {
        &self.decode_programs
    }

    pub fn decode_programs_mut(&mut self) -> &mut Arena<DecodeProgramId, DecodeProgram> {
        &mut self.decode_programs
    }

    pub fn pretty(&self) -> String {
        format!("{self}")
    }

    pub fn item_name(&self, item: ItemId) -> &str {
        &self.items[item].name
    }
}

impl fmt::Display for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (_, item) in self.items.iter() {
            writeln!(f, "{} {}:", item.kind.label(), item.name)?;
            if !item.parameters.is_empty() {
                write!(f, "  params = [")?;
                for (index, parameter) in item.parameters.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", parameter.name, parameter.ty)?;
                }
                writeln!(f, "]")?;
            }
            if let Some(body) = item.body {
                writeln!(f, "  body = {}", ExprPrinter::new(self, body))?;
            }
            if let ItemKind::Signal(info) = &item.kind {
                if !info.dependencies.is_empty() {
                    write!(f, "  dependencies = [")?;
                    for (index, dependency) in info.dependencies.iter().enumerate() {
                        if index > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", self.item_name(*dependency))?;
                    }
                    writeln!(f, "]")?;
                }
                if let Some(source) = info.source {
                    let source = &self.sources[source];
                    writeln!(
                        f,
                        "  source {} provider={} cancellation={:?}",
                        source.instance,
                        source.provider.key().unwrap_or("<missing>"),
                        source.cancellation
                    )?;
                    for (index, argument) in source.arguments.iter().enumerate() {
                        writeln!(f, "    arg[{index}] @expr {}", argument.origin_expr)?;
                    }
                    for option in &source.options {
                        writeln!(
                            f,
                            "    option {} @expr {}",
                            option.option_name, option.origin_expr
                        )?;
                    }
                    if !source.reconfiguration_dependencies.is_empty() {
                        write!(f, "    reconfigure = [")?;
                        for (index, dependency) in
                            source.reconfiguration_dependencies.iter().enumerate()
                        {
                            if index > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", self.item_name(*dependency))?;
                        }
                        writeln!(f, "]")?;
                    }
                    if let Some(active_when) = &source.active_when {
                        writeln!(f, "    activeWhen = {}", active_when.option_name)?;
                    }
                    if let Some(decode) = source.decode {
                        let decode = &self.decode_programs[decode];
                        writeln!(f, "    decode {:?} root={}", decode.mode, decode.root)?;
                    }
                }
            }
            for pipe_id in &item.pipes {
                let pipe = &self.pipes[*pipe_id];
                writeln!(f, "  pipe {} @expr {}:", pipe_id, pipe.origin.pipe_expr)?;
                for stage_id in &pipe.stages {
                    let stage = &self.stages[*stage_id];
                    writeln!(
                        f,
                        "    [{}] {} : {} -> {}",
                        stage.index,
                        stage.kind.label(),
                        stage.input_subject,
                        stage.result_subject
                    )?;
                    match &stage.kind {
                        StageKind::Gate(GateStage::Ordinary {
                            when_true,
                            when_false,
                        }) => {
                            writeln!(f, "      true  = {}", ExprPrinter::new(self, *when_true))?;
                            writeln!(f, "      false = {}", ExprPrinter::new(self, *when_false))?;
                        }
                        StageKind::Gate(GateStage::SignalFilter {
                            predicate,
                            emits_negative_update,
                            ..
                        }) => {
                            writeln!(
                                f,
                                "      predicate = {}  [negative-update={}]",
                                ExprPrinter::new(self, *predicate),
                                emits_negative_update
                            )?;
                        }
                        StageKind::TruthyFalsy(pair) => {
                            writeln!(
                                f,
                                "      truthy[{}/{}] = {} => {}",
                                pair.truthy_stage_index,
                                pair.truthy.constructor_name(),
                                pair.truthy.origin_expr,
                                pair.truthy.result_type
                            )?;
                            writeln!(
                                f,
                                "      falsy [{}/{}] = {} => {}",
                                pair.falsy_stage_index,
                                pair.falsy.constructor_name(),
                                pair.falsy.origin_expr,
                                pair.falsy.result_type
                            )?;
                        }
                        StageKind::Fanout(fanout) => {
                            writeln!(
                                f,
                                "      carrier={:?} element={} mapped={} collection={}",
                                fanout.carrier,
                                fanout.element_subject,
                                fanout.mapped_element_type,
                                fanout.mapped_collection_type
                            )?;
                            writeln!(
                                f,
                                "      map      = {}",
                                ExprPrinter::new(self, fanout.runtime_map)
                            )?;
                            for filter in &fanout.filters {
                                writeln!(
                                    f,
                                    "      filter[{}] = {}",
                                    filter.stage_index,
                                    ExprPrinter::new(self, filter.runtime_predicate)
                                )?;
                            }
                            if let Some(join) = &fanout.join {
                                writeln!(
                                    f,
                                    "      join[{}] = {} => {}",
                                    join.stage_index,
                                    ExprPrinter::new(self, join.runtime_expr),
                                    join.result_type
                                )?;
                            }
                        }
                        StageKind::Temporal(TemporalStage::Previous { seed_expr }) => {
                            writeln!(f, "      previous = {}", ExprPrinter::new(self, *seed_expr))?;
                        }
                        StageKind::Temporal(TemporalStage::DiffFunction { diff_expr }) => {
                            writeln!(f, "      diff = {}", ExprPrinter::new(self, *diff_expr))?;
                        }
                        StageKind::Temporal(TemporalStage::DiffSeed { seed_expr }) => {
                            writeln!(
                                f,
                                "      diff-seed = {}",
                                ExprPrinter::new(self, *seed_expr)
                            )?;
                        }
                    }
                }
                if let Some(recurrence) = &pipe.recurrence {
                    writeln!(
                        f,
                        "    recurrence target={:?} wakeup={:?}",
                        recurrence.target.target(),
                        recurrence.wakeup.kind()
                    )?;
                    writeln!(
                        f,
                        "      start[{}] = {}",
                        recurrence.start.stage_index,
                        ExprPrinter::new(self, recurrence.start.runtime_expr)
                    )?;
                    for guard in &recurrence.guards {
                        writeln!(
                            f,
                            "      guard[{}] = {}",
                            guard.stage_index,
                            ExprPrinter::new(self, guard.runtime_predicate)
                        )?;
                    }
                    for step in &recurrence.steps {
                        writeln!(
                            f,
                            "      step [{}] = {}",
                            step.stage_index,
                            ExprPrinter::new(self, step.runtime_expr)
                        )?;
                    }
                }
            }
            if !item.pipes.is_empty() || matches!(&item.kind, ItemKind::Signal(_)) {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub origin: HirItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub kind: ItemKind,
    pub parameters: Vec<ItemParameter>,
    pub body: Option<ExprId>,
    pub pipes: Vec<PipeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemParameter {
    pub binding: aivi_hir::BindingId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Value,
    Function,
    Signal(SignalInfo),
    Instance,
}

impl ItemKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Function => "func",
            Self::Signal(_) => "signal",
            Self::Instance => "instance",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalInfo {
    pub dependencies: Vec<ItemId>,
    pub source: Option<SourceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeOrigin {
    pub owner: HirItemId,
    pub pipe_expr: HirExprId,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipe {
    pub owner: ItemId,
    pub origin: PipeOrigin,
    pub stages: Vec<StageId>,
    pub recurrence: Option<PipeRecurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub pipe: PipeId,
    pub index: usize,
    pub span: SourceSpan,
    pub input_subject: Type,
    pub result_subject: Type,
    pub kind: StageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageKind {
    Gate(GateStage),
    TruthyFalsy(TruthyFalsyStage),
    Fanout(FanoutStage),
    Temporal(TemporalStage),
}

impl StageKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Gate(_) => "gate",
            Self::TruthyFalsy(_) => "truthy-falsy",
            Self::Fanout(_) => "fanout",
            Self::Temporal(_) => "temporal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateStage {
    Ordinary {
        when_true: ExprId,
        when_false: ExprId,
    },
    SignalFilter {
        payload_type: Type,
        predicate: ExprId,
        emits_negative_update: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruthyFalsyStage {
    pub truthy_stage_index: usize,
    pub truthy_stage_span: SourceSpan,
    pub falsy_stage_index: usize,
    pub falsy_stage_span: SourceSpan,
    pub truthy: TruthyFalsyBranch,
    pub falsy: TruthyFalsyBranch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruthyFalsyBranch {
    pub constructor: BuiltinTerm,
    pub payload_subject: Option<Type>,
    pub result_type: Type,
    pub origin_expr: HirExprId,
}

impl TruthyFalsyBranch {
    pub fn constructor_name(&self) -> &'static str {
        match self.constructor {
            BuiltinTerm::True => "True",
            BuiltinTerm::False => "False",
            BuiltinTerm::None => "None",
            BuiltinTerm::Some => "Some",
            BuiltinTerm::Ok => "Ok",
            BuiltinTerm::Err => "Err",
            BuiltinTerm::Valid => "Valid",
            BuiltinTerm::Invalid => "Invalid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutStage {
    pub carrier: FanoutCarrier,
    pub element_subject: Type,
    pub mapped_element_type: Type,
    pub mapped_collection_type: Type,
    pub runtime_map: ExprId,
    pub filters: Vec<FanoutFilter>,
    pub join: Option<FanoutJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutFilter {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate_expr: HirExprId,
    pub input_subject: Type,
    pub runtime_predicate: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutJoin {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub origin_expr: HirExprId,
    pub input_subject: Type,
    pub collection_subject: Type,
    pub runtime_expr: ExprId,
    pub result_type: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalStage {
    Previous { seed_expr: ExprId },
    DiffFunction { diff_expr: ExprId },
    DiffSeed { seed_expr: ExprId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeRecurrence {
    pub target: RecurrencePlan,
    pub wakeup: RecurrenceWakeupPlan,
    pub seed_expr: ExprId,
    pub start: RecurrenceStage,
    pub guards: Vec<RecurrenceGuard>,
    pub steps: Vec<RecurrenceStage>,
    pub non_source_wakeup: Option<NonSourceWakeup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceStage {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub origin_expr: HirExprId,
    pub input_subject: Type,
    pub result_subject: Type,
    pub runtime_expr: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceGuard {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate_expr: HirExprId,
    pub input_subject: Type,
    pub runtime_predicate: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonSourceWakeup {
    pub cause: NonSourceWakeupCause,
    pub witness_expr: HirExprId,
    pub runtime_witness: ExprId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceInstanceId(u32);

impl SourceInstanceId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

impl fmt::Display for SourceInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOptionBinding {
    pub option_span: SourceSpan,
    pub option_name: Box<str>,
    pub origin_expr: HirExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceArgumentValue {
    pub origin_expr: HirExprId,
    pub runtime_expr: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOptionValue {
    pub option_span: SourceSpan,
    pub option_name: Box<str>,
    pub origin_expr: HirExprId,
    pub runtime_expr: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceNode {
    pub owner: ItemId,
    pub span: SourceSpan,
    pub instance: SourceInstanceId,
    pub provider: SourceProviderRef,
    pub teardown: SourceTeardownPolicy,
    pub replacement: SourceReplacementPolicy,
    pub arguments: Vec<SourceArgumentValue>,
    pub options: Vec<SourceOptionValue>,
    pub reconfiguration_dependencies: Vec<ItemId>,
    pub explicit_triggers: Vec<SourceOptionBinding>,
    pub active_when: Option<SourceOptionBinding>,
    pub cancellation: SourceCancellationPolicy,
    pub stale_work: SourceStaleWorkPolicy,
    pub decode: Option<DecodeProgramId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DomainDecodeSurfaceKind {
    Direct,
    FallibleResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainDecodeSurface {
    pub domain_item: HirItemId,
    pub member_index: usize,
    pub member_name: Box<str>,
    pub kind: DomainDecodeSurfaceKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeProgram {
    pub owner: ItemId,
    pub mode: DecodeMode,
    pub payload_annotation: HirTypeId,
    pub root: DecodeStepId,
    steps: Arena<DecodeStepId, DecodeStep>,
}

impl DecodeProgram {
    pub(crate) fn new(
        owner: ItemId,
        mode: DecodeMode,
        payload_annotation: HirTypeId,
        root: DecodeStepId,
        steps: Arena<DecodeStepId, DecodeStep>,
    ) -> Self {
        Self {
            owner,
            mode,
            payload_annotation,
            root,
            steps,
        }
    }

    pub fn steps(&self) -> &Arena<DecodeStepId, DecodeStep> {
        &self.steps
    }

    pub fn steps_mut(&mut self) -> &mut Arena<DecodeStepId, DecodeStep> {
        &mut self.steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeStep {
    Scalar {
        scalar: PrimitiveType,
    },
    Tuple {
        elements: Vec<DecodeStepId>,
    },
    Record {
        fields: Vec<DecodeField>,
        extra_fields: DecodeExtraFieldPolicy,
    },
    Sum {
        variants: Vec<DecodeVariant>,
        strategy: DecodeSumStrategy,
    },
    Domain {
        carrier: DecodeStepId,
        surface: DomainDecodeSurface,
    },
    List {
        element: DecodeStepId,
    },
    Option {
        element: DecodeStepId,
    },
    Result {
        error: DecodeStepId,
        value: DecodeStepId,
    },
    Validation {
        error: DecodeStepId,
        value: DecodeStepId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeField {
    pub name: Box<str>,
    pub requirement: DecodeFieldRequirement,
    pub step: DecodeStepId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeVariant {
    pub name: Box<str>,
    pub payload: Option<DecodeStepId>,
}

struct ExprPrinter<'a> {
    module: &'a Module,
    root: ExprId,
}

impl<'a> ExprPrinter<'a> {
    fn new(module: &'a Module, root: ExprId) -> Self {
        Self { module, root }
    }
}

impl fmt::Display for ExprPrinter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_expr(self.module, self.root, f)
    }
}

fn format_expr(module: &Module, expr_id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let expr = &module.exprs[expr_id];
    match &expr.kind {
        crate::expr::ExprKind::AmbientSubject => f.write_str("_"),
        crate::expr::ExprKind::OptionSome { payload } => {
            write!(f, "Some ")?;
            format_expr(module, *payload, f)
        }
        crate::expr::ExprKind::OptionNone => f.write_str("None"),
        crate::expr::ExprKind::Reference(reference) => match reference {
            Reference::Local(binding) => write!(f, "#{}", binding.as_raw()),
            Reference::Item(item) => f.write_str(module.item_name(*item)),
            Reference::HirItem(item) => write!(f, "hir-item-{}", item.as_raw()),
            Reference::SumConstructor(handle) => {
                write!(f, "{}.{}", handle.type_name, handle.variant_name)
            }
            Reference::DomainMember(handle) => {
                write!(f, "{}.{}", handle.domain_name, handle.member_name)
            }
            Reference::BuiltinClassMember(intrinsic) => write!(f, "{intrinsic:?}"),
            Reference::Builtin(term) => write!(f, "{term:?}"),
            Reference::IntrinsicValue(value) => write!(f, "{value}"),
        },
        crate::expr::ExprKind::Integer(value) => write!(f, "{}", value.raw),
        crate::expr::ExprKind::Float(value) => write!(f, "{}", value.raw),
        crate::expr::ExprKind::Decimal(value) => write!(f, "{}", value.raw),
        crate::expr::ExprKind::BigInt(value) => write!(f, "{}", value.raw),
        crate::expr::ExprKind::SuffixedInteger(value) => {
            write!(f, "{}{}", value.raw, value.suffix.text())
        }
        crate::expr::ExprKind::Text(text) => {
            f.write_str("\"")?;
            for segment in &text.segments {
                match segment {
                    crate::expr::TextSegment::Fragment { raw, .. } => f.write_str(raw)?,
                    crate::expr::TextSegment::Interpolation { expr, .. } => {
                        f.write_str("{")?;
                        format_expr(module, *expr, f)?;
                        f.write_str("}")?;
                    }
                }
            }
            f.write_str("\"")
        }
        crate::expr::ExprKind::Tuple(elements) => {
            f.write_str("(")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_expr(module, *element, f)?;
            }
            f.write_str(")")
        }
        crate::expr::ExprKind::List(elements) => {
            f.write_str("[")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_expr(module, *element, f)?;
            }
            f.write_str("]")
        }
        crate::expr::ExprKind::Map(entries) => {
            f.write_str("Map {")?;
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_expr(module, entry.key, f)?;
                f.write_str(": ")?;
                format_expr(module, entry.value, f)?;
            }
            f.write_str("}")
        }
        crate::expr::ExprKind::Set(elements) => {
            f.write_str("Set [")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_expr(module, *element, f)?;
            }
            f.write_str("]")
        }
        crate::expr::ExprKind::Record(fields) => {
            f.write_str("{")?;
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{}: ", field.label)?;
                format_expr(module, field.value, f)?;
            }
            f.write_str("}")
        }
        crate::expr::ExprKind::Projection { base, path } => {
            match base {
                crate::expr::ProjectionBase::AmbientSubject => f.write_str("_")?,
                crate::expr::ProjectionBase::Expr(base) => format_expr(module, *base, f)?,
            }
            for segment in path {
                write!(f, ".{segment}")?;
            }
            Ok(())
        }
        crate::expr::ExprKind::Apply { callee, arguments } => {
            format_expr(module, *callee, f)?;
            for argument in arguments {
                f.write_str(" ")?;
                format_expr(module, *argument, f)?;
            }
            Ok(())
        }
        crate::expr::ExprKind::Unary { operator, expr } => {
            write!(f, "{operator:?} ")?;
            format_expr(module, *expr, f)
        }
        crate::expr::ExprKind::Binary {
            left,
            operator,
            right,
        } => {
            format_expr(module, *left, f)?;
            write!(f, " {operator:?} ")?;
            format_expr(module, *right, f)
        }
        crate::expr::ExprKind::Pipe(pipe) => {
            format_expr(module, pipe.head, f)?;
            for stage in &pipe.stages {
                match &stage.kind {
                    crate::expr::PipeStageKind::Transform { expr, .. } => {
                        f.write_str(" |> ")?;
                        format_expr(module, *expr, f)?;
                    }
                    crate::expr::PipeStageKind::Tap { expr } => {
                        f.write_str(" | ")?;
                        format_expr(module, *expr, f)?;
                    }
                    crate::expr::PipeStageKind::Debug { label } => {
                        write!(f, " |debug[{label}]")?;
                    }
                    crate::expr::PipeStageKind::Gate {
                        predicate,
                        emits_negative_update,
                    } => {
                        write!(f, " ?|>{}[", if *emits_negative_update { "!" } else { "" })?;
                        format_expr(module, *predicate, f)?;
                        f.write_str("]")?;
                    }
                    crate::expr::PipeStageKind::Case { arms } => {
                        for PipeCaseArm { pattern, body, .. } in arms {
                            f.write_str(" ||> ")?;
                            format_pattern(pattern, f)?;
                            f.write_str(" => ")?;
                            format_expr(module, *body, f)?;
                        }
                    }
                    crate::expr::PipeStageKind::TruthyFalsy(PipeTruthyFalsyStage {
                        truthy,
                        falsy,
                    }) => {
                        write!(f, " T|>[{}] ", constructor_name(truthy.constructor))?;
                        format_expr(module, truthy.body, f)?;
                        write!(f, " F|>[{}] ", constructor_name(falsy.constructor))?;
                        format_expr(module, falsy.body, f)?;
                    }
                    crate::expr::PipeStageKind::FanOut { map_expr } => {
                        f.write_str(" *|> ")?;
                        format_expr(module, *map_expr, f)?;
                    }
                }
            }
            Ok(())
        }
    }
}

fn format_pattern(pattern: &Pattern, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match &pattern.kind {
        PatternKind::Wildcard => f.write_str("_"),
        PatternKind::Binding(PatternBinding { name, .. }) => f.write_str(name),
        PatternKind::Integer(value) => write!(f, "{}", value.raw),
        PatternKind::Text(raw) => write!(f, "\"{raw}\""),
        PatternKind::Tuple(elements) => {
            f.write_str("(")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_pattern(element, f)?;
            }
            f.write_str(")")
        }
        PatternKind::List { elements, rest } => {
            f.write_str("[")?;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                format_pattern(element, f)?;
            }
            if let Some(rest) = rest {
                if !elements.is_empty() {
                    f.write_str(", ")?;
                }
                f.write_str("...")?;
                format_pattern(rest, f)?;
            }
            f.write_str("]")
        }
        PatternKind::Record(fields) => {
            f.write_str("{")?;
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{}: ", field.label)?;
                format_pattern(&field.pattern, f)?;
            }
            f.write_str("}")
        }
        PatternKind::Constructor {
            callee: PatternConstructor { display, .. },
            arguments,
        } => {
            f.write_str(display)?;
            for argument in arguments {
                f.write_str(" ")?;
                format_pattern(argument, f)?;
            }
            Ok(())
        }
    }
}

fn constructor_name(term: BuiltinTerm) -> &'static str {
    match term {
        BuiltinTerm::True => "True",
        BuiltinTerm::False => "False",
        BuiltinTerm::None => "None",
        BuiltinTerm::Some => "Some",
        BuiltinTerm::Ok => "Ok",
        BuiltinTerm::Err => "Err",
        BuiltinTerm::Valid => "Valid",
        BuiltinTerm::Invalid => "Invalid",
    }
}
