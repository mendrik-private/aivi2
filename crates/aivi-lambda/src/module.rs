use std::{collections::BTreeMap, fmt};

use aivi_base::SourceSpan;
use aivi_core::{self as core, Arena};
use aivi_hir::{BindingId, ExprId as HirExprId};

use crate::{CaptureId, ClosureId};

/// Build a binding→name map from any parameter slice where each element exposes
/// `binding: BindingId` and `name: Box<str>`. Used by both lowering and validation.
pub(crate) fn parameter_name_map<P: HasBindingName>(
    parameters: &[P],
) -> BTreeMap<BindingId, Box<str>> {
    parameters
        .iter()
        .map(|p| (p.binding(), p.name().into()))
        .collect()
}

pub(crate) trait HasBindingName {
    fn binding(&self) -> BindingId;
    fn name(&self) -> &str;
}

impl HasBindingName for Parameter {
    fn binding(&self) -> BindingId {
        self.binding
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasBindingName for core::ItemParameter {
    fn binding(&self) -> BindingId {
        self.binding
    }
    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    core: core::Module,
    items: Arena<core::ItemId, Item>,
    pipes: Arena<core::PipeId, Pipe>,
    stages: Arena<core::StageId, Stage>,
    closures: Arena<ClosureId, Closure>,
    captures: Arena<CaptureId, Capture>,
}

impl Module {
    pub fn new(core: core::Module) -> Self {
        Self {
            core,
            items: Arena::new(),
            pipes: Arena::new(),
            stages: Arena::new(),
            closures: Arena::new(),
            captures: Arena::new(),
        }
    }

    pub fn core(&self) -> &core::Module {
        &self.core
    }

    pub fn exprs(&self) -> &Arena<core::ExprId, core::Expr> {
        self.core.exprs()
    }

    pub fn sources(&self) -> &Arena<core::SourceId, core::SourceNode> {
        self.core.sources()
    }

    pub fn decode_programs(&self) -> &Arena<core::DecodeProgramId, core::DecodeProgram> {
        self.core.decode_programs()
    }

    pub fn items(&self) -> &Arena<core::ItemId, Item> {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Arena<core::ItemId, Item> {
        &mut self.items
    }

    pub fn pipes(&self) -> &Arena<core::PipeId, Pipe> {
        &self.pipes
    }

    pub fn pipes_mut(&mut self) -> &mut Arena<core::PipeId, Pipe> {
        &mut self.pipes
    }

    pub fn stages(&self) -> &Arena<core::StageId, Stage> {
        &self.stages
    }

    pub fn stages_mut(&mut self) -> &mut Arena<core::StageId, Stage> {
        &mut self.stages
    }

    pub fn closures(&self) -> &Arena<ClosureId, Closure> {
        &self.closures
    }

    pub fn closures_mut(&mut self) -> &mut Arena<ClosureId, Closure> {
        &mut self.closures
    }

    pub fn captures(&self) -> &Arena<CaptureId, Capture> {
        &self.captures
    }

    pub fn captures_mut(&mut self) -> &mut Arena<CaptureId, Capture> {
        &mut self.captures
    }

    pub fn item_name(&self, item: core::ItemId) -> &str {
        &self.items[item].name
    }

    pub fn pretty(&self) -> String {
        format!("{self}")
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
                writeln!(f, "  body = closure{body}")?;
            }
            if let core::ItemKind::Signal(info) = &item.kind {
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
                    let source = &self.sources()[source];
                    writeln!(
                        f,
                        "  source {} provider={} cancellation={:?}",
                        source.instance,
                        source.provider.key().unwrap_or("<missing>"),
                        source.cancellation
                    )?;
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
                            writeln!(f, "      true  = closure{when_true}")?;
                            writeln!(f, "      false = closure{when_false}")?;
                        }
                        StageKind::Gate(GateStage::SignalFilter {
                            predicate,
                            emits_negative_update,
                            ..
                        }) => {
                            writeln!(
                                f,
                                "      predicate = closure{predicate}  [negative-update={emits_negative_update}]"
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
                            writeln!(f, "      map      = closure{}", fanout.map)?;
                            for filter in &fanout.filters {
                                writeln!(
                                    f,
                                    "      filter[{}] = closure{}",
                                    filter.stage_index, filter.runtime
                                )?;
                            }
                            if let Some(join) = &fanout.join {
                                writeln!(
                                    f,
                                    "      join[{}] = closure{} => {}",
                                    join.stage_index, join.runtime, join.result_type
                                )?;
                            }
                        }
                        StageKind::Temporal(TemporalStage::Previous { seed }) => {
                            writeln!(f, "      previous = closure{seed}")?;
                        }
                        StageKind::Temporal(TemporalStage::DiffFunction { diff }) => {
                            writeln!(f, "      diff = closure{diff}")?;
                        }
                        StageKind::Temporal(TemporalStage::DiffSeed { seed }) => {
                            writeln!(f, "      diff-seed = closure{seed}")?;
                        }
                        StageKind::Temporal(TemporalStage::Delay { duration }) => {
                            writeln!(f, "      delay = closure{duration}")?;
                        }
                        StageKind::Temporal(TemporalStage::Burst { every, count }) => {
                            writeln!(f, "      burst every=closure{every} count=closure{count}")?;
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
                        "      start[{}] = closure{}",
                        recurrence.start.stage_index, recurrence.start.runtime
                    )?;
                    for step in &recurrence.steps {
                        writeln!(
                            f,
                            "      step [{}] = closure{}",
                            step.stage_index, step.runtime
                        )?;
                    }
                    if let Some(witness) = &recurrence.non_source_wakeup {
                        writeln!(
                            f,
                            "      witness {:?} = closure{}",
                            witness.cause, witness.runtime
                        )?;
                    }
                }
            }
            if !item.pipes.is_empty() || matches!(&item.kind, core::ItemKind::Signal(_)) {
                writeln!(f)?;
            }
        }

        if !self.closures.is_empty() {
            writeln!(f, "closures:")?;
        }
        for (closure_id, closure) in self.closures.iter() {
            writeln!(
                f,
                "  closure{closure_id} {} owner={} root=expr{} kind={}",
                self.item_name(closure.owner),
                closure.owner,
                closure.root,
                closure.kind
            )?;
            if let Some(subject) = &closure.ambient_subject {
                writeln!(f, "    subject = {subject}")?;
            }
            if !closure.parameters.is_empty() {
                write!(f, "    params = [")?;
                for (index, parameter) in closure.parameters.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(
                        f,
                        "{}:#{}:{}",
                        parameter.name,
                        parameter.binding.as_raw(),
                        parameter.ty
                    )?;
                }
                writeln!(f, "]")?;
            }
            if !closure.captures.is_empty() {
                write!(f, "    captures = [")?;
                for (index, capture_id) in closure.captures.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    let capture = &self.captures[*capture_id];
                    write!(
                        f,
                        "capture{capture_id}:#{}:{}",
                        capture.binding.as_raw(),
                        capture.ty
                    )?;
                    if let Some(name) = &capture.name {
                        write!(f, " ({name})")?;
                    }
                }
                writeln!(f, "]")?;
            }
        }
        Ok(())
    }
}

/// A lambda-level function parameter, owned by an [`Item`] or [`Closure`].
///
/// Mirrors [`core::ItemParameter`] but is decoupled from it so that changes to the
/// core representation do not force changes here, and vice-versa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub binding: aivi_hir::BindingId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub ty: core::Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub origin: aivi_hir::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub kind: core::ItemKind,
    pub parameters: Vec<Parameter>,
    pub body: Option<ClosureId>,
    pub pipes: Vec<core::PipeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipe {
    pub owner: core::ItemId,
    pub origin: core::PipeOrigin,
    pub stages: Vec<core::StageId>,
    pub recurrence: Option<PipeRecurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub pipe: core::PipeId,
    pub index: usize,
    pub span: SourceSpan,
    pub input_subject: core::Type,
    pub result_subject: core::Type,
    pub kind: StageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageKind {
    Gate(GateStage),
    TruthyFalsy(core::TruthyFalsyStage),
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
        when_true: ClosureId,
        when_false: ClosureId,
    },
    SignalFilter {
        payload_type: core::Type,
        predicate: ClosureId,
        emits_negative_update: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutStage {
    pub carrier: aivi_typing::FanoutCarrier,
    pub element_subject: core::Type,
    pub mapped_element_type: core::Type,
    pub mapped_collection_type: core::Type,
    pub map: ClosureId,
    pub filters: Vec<FanoutFilter>,
    pub join: Option<FanoutJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutFilter {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate_expr: HirExprId,
    pub input_subject: core::Type,
    pub runtime: ClosureId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutJoin {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub origin_expr: HirExprId,
    pub input_subject: core::Type,
    pub collection_subject: core::Type,
    pub runtime: ClosureId,
    pub result_type: core::Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalStage {
    Previous { seed: ClosureId },
    DiffFunction { diff: ClosureId },
    DiffSeed { seed: ClosureId },
    Delay { duration: ClosureId },
    Burst { every: ClosureId, count: ClosureId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeRecurrence {
    pub target: aivi_typing::RecurrencePlan,
    pub wakeup: aivi_typing::RecurrenceWakeupPlan,
    pub seed: ClosureId,
    pub start: RecurrenceStage,
    pub steps: Vec<RecurrenceStage>,
    pub non_source_wakeup: Option<NonSourceWakeup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceStage {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub origin_expr: HirExprId,
    pub input_subject: core::Type,
    pub result_subject: core::Type,
    pub runtime: ClosureId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonSourceWakeup {
    pub cause: aivi_typing::NonSourceWakeupCause,
    pub witness_expr: HirExprId,
    pub runtime: ClosureId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closure {
    pub owner: core::ItemId,
    pub span: SourceSpan,
    pub kind: ClosureKind,
    /// The type of the "ambient subject" pre-bound for expressions inside this closure, if any.
    ///
    /// - `Some(ty)` for gate and recurrence-stage closures: the stage's `input_subject` type is
    ///   implicitly in scope and can be referenced via `ExprKind::AmbientSubject` without naming
    ///   a binding. This is set during lowering from the owning stage's `input_subject` field.
    ///   The module-level construct that sets this is a pipe stage (gate, signal-filter, or
    ///   recurrence start/step).
    ///
    /// - `None` for `ItemBody` and `RecurrenceWakeupWitness` closures: these closures have no
    ///   implicit ambient type; all values in scope must be explicit item parameters or captures.
    ///
    /// Invariant: if `ambient_subject` is `Some`, the closure body must not reference
    /// `ExprKind::AmbientSubject` with a type that differs from the recorded type.
    pub ambient_subject: Option<core::Type>,
    pub parameters: Vec<Parameter>,
    /// The set of captured bindings from enclosing scopes.
    ///
    /// # Self-recursive closures
    ///
    /// Self-recursive closures are not currently supported. The capture analysis
    /// (`capture_free_bindings`) walks the body expression and collects free bindings, but the
    /// closure itself has not been given a binding ID by the time that analysis runs. A
    /// self-recursive call would therefore appear as an unbound free reference and be rejected
    /// with an error. Supporting self-recursion would require either:
    ///   1. assigning the closure a stable name/binding before the body is analyzed, or
    ///   2. a separate fixed-point pass that patches the closure after the fact.
    pub captures: Vec<CaptureId>,
    pub root: core::ExprId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosureKind {
    ItemBody,
    GateTrue,
    GateFalse,
    SignalFilterPredicate,
    PreviousSeed,
    DiffFunction,
    DiffSeed,
    DelayDuration,
    BurstEvery,
    BurstCount,
    FanoutMap,
    FanoutFilterPredicate,
    FanoutJoin,
    RecurrenceSeed,
    RecurrenceStart,
    RecurrenceStep,
    RecurrenceWakeupWitness,
}

impl fmt::Display for ClosureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemBody => f.write_str("item-body"),
            Self::GateTrue => f.write_str("gate-true"),
            Self::GateFalse => f.write_str("gate-false"),
            Self::SignalFilterPredicate => f.write_str("signal-filter-predicate"),
            Self::PreviousSeed => f.write_str("previous-seed"),
            Self::DiffFunction => f.write_str("diff-function"),
            Self::DiffSeed => f.write_str("diff-seed"),
            Self::DelayDuration => f.write_str("delay-duration"),
            Self::BurstEvery => f.write_str("burst-every"),
            Self::BurstCount => f.write_str("burst-count"),
            Self::FanoutMap => f.write_str("fanout-map"),
            Self::FanoutFilterPredicate => f.write_str("fanout-filter-predicate"),
            Self::FanoutJoin => f.write_str("fanout-join"),
            Self::RecurrenceSeed => f.write_str("recurrence-seed"),
            Self::RecurrenceStart => f.write_str("recurrence-start"),
            Self::RecurrenceStep => f.write_str("recurrence-step"),
            Self::RecurrenceWakeupWitness => f.write_str("recurrence-wakeup-witness"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub closure: ClosureId,
    pub binding: BindingId,
    pub name: Option<Box<str>>,
    pub ty: core::Type,
}
