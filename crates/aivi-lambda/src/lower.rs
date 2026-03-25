use std::collections::BTreeMap;

use aivi_base::SourceSpan;
use aivi_core::{self as core, ArenaOverflow};
use aivi_hir::BindingId;

use crate::{
    Capture, Closure, ClosureId, ClosureKind, GateStage, Item,
    LoweringError::*,
    Module, NonSourceWakeup, Pipe, PipeRecurrence, RecurrenceStage, Stage, StageKind,
    analysis::{AnalysisError, capture_free_bindings},
    validate::{ValidationError, validate_module},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoweringErrors {
    errors: Vec<LoweringError>,
}

impl LoweringErrors {
    pub fn new(errors: Vec<LoweringError>) -> Self {
        Self { errors }
    }

    pub fn errors(&self) -> &[LoweringError] {
        &self.errors
    }

    pub fn into_errors(self) -> Vec<LoweringError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl std::fmt::Display for LoweringErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoweringErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoweringError {
    InvalidCoreModule(core::ValidationError),
    UnboundLocalReference {
        item: core::ItemId,
        binding: BindingId,
        span: SourceSpan,
    },
    CaptureTypeConflict {
        item: core::ItemId,
        kind: ClosureKind,
        binding: BindingId,
        previous: core::Type,
        current: core::Type,
        span: SourceSpan,
    },
    ArenaOverflow {
        family: &'static str,
        attempted_len: usize,
    },
    InvalidLambdaModule(ValidationError),
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCoreModule(error) => {
                write!(
                    f,
                    "typed-lambda lowering requires valid typed-core input: {error}"
                )
            }
            Self::UnboundLocalReference { item, binding, .. } => write!(
                f,
                "typed-lambda lowering found unbound local binding #{} in top-level item {}",
                binding.as_raw(),
                item
            ),
            Self::CaptureTypeConflict {
                item,
                kind,
                binding,
                previous,
                current,
                ..
            } => write!(
                f,
                "typed-lambda lowering saw binding #{} change type inside {} closure for item {}: {} then {}",
                binding.as_raw(),
                kind,
                item,
                previous,
                current
            ),
            Self::ArenaOverflow {
                family,
                attempted_len,
            } => write!(
                f,
                "typed-lambda {family} arena overflow after {attempted_len} entries; ids are limited to u32::MAX"
            ),
            Self::InvalidLambdaModule(error) => {
                write!(
                    f,
                    "typed-lambda lowering produced invalid lambda IR: {error}"
                )
            }
        }
    }
}

impl std::error::Error for LoweringError {}

pub fn lower_module(core_module: &core::Module) -> Result<Module, LoweringErrors> {
    if let Err(errors) = core::validate_module(core_module) {
        return Err(LoweringErrors::new(
            errors
                .into_errors()
                .into_iter()
                .map(LoweringError::InvalidCoreModule)
                .collect(),
        ));
    }

    ModuleLowerer::new(core_module).build()
}

struct ModuleLowerer<'a> {
    core: &'a core::Module,
    module: Module,
    errors: Vec<LoweringError>,
}

impl<'a> ModuleLowerer<'a> {
    fn new(core: &'a core::Module) -> Self {
        Self {
            core,
            module: Module::new(core.clone()),
            errors: Vec::new(),
        }
    }

    fn build(mut self) -> Result<Module, LoweringErrors> {
        self.seed_items();
        self.seed_stages();
        self.seed_pipes();

        if !self.errors.is_empty() {
            return Err(LoweringErrors::new(self.errors));
        }

        if let Err(errors) = validate_module(&self.module) {
            return Err(LoweringErrors::new(
                errors
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::InvalidLambdaModule)
                    .collect(),
            ));
        }

        Ok(self.module)
    }

    fn seed_items(&mut self) {
        for (item_id, item) in self.core.items().iter() {
            let body = item.body.and_then(|root| {
                match self.lower_closure(
                    item_id,
                    item.span,
                    ClosureKind::ItemBody,
                    None,
                    item.parameters.clone(),
                    root,
                    false,
                    &parameter_name_map(&item.parameters),
                ) {
                    Some(closure) => Some(closure),
                    None => None,
                }
            });

            let lowered_id = match self.module.items_mut().alloc(Item {
                origin: item.origin,
                span: item.span,
                name: item.name.clone(),
                kind: item.kind.clone(),
                parameters: item.parameters.clone(),
                body,
                pipes: item.pipes.clone(),
            }) {
                Ok(id) => id,
                Err(overflow) => {
                    self.errors.push(arena_overflow("items", overflow));
                    return;
                }
            };
            debug_assert_eq!(lowered_id, item_id);
        }
    }

    fn seed_stages(&mut self) {
        for (stage_id, stage) in self.core.stages().iter() {
            let owner = self.core.pipes()[stage.pipe].owner;
            let runtime_names = parameter_name_map(&self.core.items()[owner].parameters);
            let kind = match &stage.kind {
                core::StageKind::Gate(core::GateStage::Ordinary {
                    when_true,
                    when_false,
                }) => {
                    let Some(when_true) = self.lower_closure(
                        owner,
                        stage.span,
                        ClosureKind::GateTrue,
                        Some(stage.input_subject.clone()),
                        Vec::new(),
                        *when_true,
                        true,
                        &runtime_names,
                    ) else {
                        continue;
                    };
                    let Some(when_false) = self.lower_closure(
                        owner,
                        stage.span,
                        ClosureKind::GateFalse,
                        Some(stage.input_subject.clone()),
                        Vec::new(),
                        *when_false,
                        true,
                        &runtime_names,
                    ) else {
                        continue;
                    };
                    StageKind::Gate(GateStage::Ordinary {
                        when_true,
                        when_false,
                    })
                }
                core::StageKind::Gate(core::GateStage::SignalFilter {
                    payload_type,
                    predicate,
                    emits_negative_update,
                }) => {
                    let Some(predicate) = self.lower_closure(
                        owner,
                        stage.span,
                        ClosureKind::SignalFilterPredicate,
                        Some(payload_type.clone()),
                        Vec::new(),
                        *predicate,
                        true,
                        &runtime_names,
                    ) else {
                        continue;
                    };
                    StageKind::Gate(GateStage::SignalFilter {
                        payload_type: payload_type.clone(),
                        predicate,
                        emits_negative_update: *emits_negative_update,
                    })
                }
                core::StageKind::TruthyFalsy(pair) => StageKind::TruthyFalsy(pair.clone()),
                core::StageKind::Fanout(fanout) => StageKind::Fanout(fanout.clone()),
            };

            let lowered_id = match self.module.stages_mut().alloc(Stage {
                pipe: stage.pipe,
                index: stage.index,
                span: stage.span,
                input_subject: stage.input_subject.clone(),
                result_subject: stage.result_subject.clone(),
                kind,
            }) {
                Ok(id) => id,
                Err(overflow) => {
                    self.errors.push(arena_overflow("stages", overflow));
                    return;
                }
            };
            debug_assert_eq!(lowered_id, stage_id);
        }
    }

    fn seed_pipes(&mut self) {
        for (pipe_id, pipe) in self.core.pipes().iter() {
            let runtime_names = parameter_name_map(&self.core.items()[pipe.owner].parameters);
            let recurrence = pipe.recurrence.as_ref().and_then(|recurrence| {
                let seed = self.lower_closure(
                    pipe.owner,
                    pipe.origin.span,
                    ClosureKind::RecurrenceSeed,
                    None,
                    Vec::new(),
                    recurrence.seed_expr,
                    true,
                    &runtime_names,
                )?;
                let start = self.lower_recurrence_stage(
                    pipe.owner,
                    &runtime_names,
                    ClosureKind::RecurrenceStart,
                    &recurrence.start,
                )?;
                let mut steps = Vec::with_capacity(recurrence.steps.len());
                for step in &recurrence.steps {
                    steps.push(self.lower_recurrence_stage(
                        pipe.owner,
                        &runtime_names,
                        ClosureKind::RecurrenceStep,
                        step,
                    )?);
                }
                let non_source_wakeup = recurrence.non_source_wakeup.as_ref().and_then(|wakeup| {
                    let runtime = self.lower_closure(
                        pipe.owner,
                        pipe.origin.span,
                        ClosureKind::RecurrenceWakeupWitness,
                        None,
                        Vec::new(),
                        wakeup.runtime_witness,
                        true,
                        &runtime_names,
                    )?;
                    Some(NonSourceWakeup {
                        cause: wakeup.cause,
                        witness_expr: wakeup.witness_expr,
                        runtime,
                    })
                });
                Some(PipeRecurrence {
                    target: recurrence.target.clone(),
                    wakeup: recurrence.wakeup.clone(),
                    seed,
                    start,
                    steps,
                    non_source_wakeup,
                })
            });

            let lowered_id = match self.module.pipes_mut().alloc(Pipe {
                owner: pipe.owner,
                origin: pipe.origin.clone(),
                stages: pipe.stages.clone(),
                recurrence,
            }) {
                Ok(id) => id,
                Err(overflow) => {
                    self.errors.push(arena_overflow("pipes", overflow));
                    return;
                }
            };
            debug_assert_eq!(lowered_id, pipe_id);
        }
    }

    fn lower_recurrence_stage(
        &mut self,
        owner: core::ItemId,
        known_names: &BTreeMap<BindingId, Box<str>>,
        kind: ClosureKind,
        stage: &core::RecurrenceStage,
    ) -> Option<RecurrenceStage> {
        let runtime = self.lower_closure(
            owner,
            stage.stage_span,
            kind,
            Some(stage.input_subject.clone()),
            Vec::new(),
            stage.runtime_expr,
            true,
            known_names,
        )?;
        Some(RecurrenceStage {
            stage_index: stage.stage_index,
            stage_span: stage.stage_span,
            origin_expr: stage.origin_expr,
            input_subject: stage.input_subject.clone(),
            result_subject: stage.result_subject.clone(),
            runtime,
        })
    }

    fn lower_closure(
        &mut self,
        owner: core::ItemId,
        span: SourceSpan,
        kind: ClosureKind,
        ambient_subject: Option<core::Type>,
        parameters: Vec<core::ItemParameter>,
        root: core::ExprId,
        allow_captures: bool,
        known_names: &BTreeMap<BindingId, Box<str>>,
    ) -> Option<ClosureId> {
        let local_bindings = parameters
            .iter()
            .map(|parameter| parameter.binding)
            .collect::<Vec<_>>();
        let captures = match capture_free_bindings(self.core, root, &local_bindings, known_names) {
            Ok(captures) => captures,
            Err(AnalysisError::BindingTypeConflict {
                binding,
                previous,
                current,
                span,
            }) => {
                self.errors.push(CaptureTypeConflict {
                    item: owner,
                    kind,
                    binding,
                    previous,
                    current,
                    span,
                });
                return None;
            }
        };

        if !allow_captures {
            if !captures.is_empty() {
                for capture in &captures {
                    self.errors.push(UnboundLocalReference {
                        item: owner,
                        binding: capture.binding,
                        span: capture.span,
                    });
                }
                return None;
            }
        }

        let closure_id = match self.module.closures_mut().alloc(Closure {
            owner,
            span,
            kind,
            ambient_subject,
            parameters,
            captures: Vec::new(),
            root,
        }) {
            Ok(id) => id,
            Err(overflow) => {
                self.errors.push(arena_overflow("closures", overflow));
                return None;
            }
        };

        let mut capture_ids = Vec::with_capacity(captures.len());
        for capture in captures {
            let capture_id = match self.module.captures_mut().alloc(Capture {
                closure: closure_id,
                binding: capture.binding,
                name: capture.name,
                ty: capture.ty,
            }) {
                Ok(id) => id,
                Err(overflow) => {
                    self.errors.push(arena_overflow("captures", overflow));
                    return None;
                }
            };
            capture_ids.push(capture_id);
        }
        self.module
            .closures_mut()
            .get_mut(closure_id)
            .expect("new closure should exist")
            .captures = capture_ids;
        Some(closure_id)
    }
}

fn parameter_name_map(parameters: &[core::ItemParameter]) -> BTreeMap<BindingId, Box<str>> {
    parameters
        .iter()
        .map(|parameter| (parameter.binding, parameter.name.clone()))
        .collect()
}

fn arena_overflow(family: &'static str, overflow: ArenaOverflow) -> LoweringError {
    LoweringError::ArenaOverflow {
        family,
        attempted_len: overflow.attempted_len(),
    }
}
