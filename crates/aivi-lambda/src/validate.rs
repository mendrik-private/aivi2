use std::collections::{BTreeMap, BTreeSet};

use aivi_core::{self as core};
use aivi_hir::BindingId;

use crate::{
    CaptureId, ClosureId, ClosureKind, GateStage, Item, Module, Parameter, Pipe, RecurrenceStage,
    Stage, StageKind, TemporalStage,
    analysis::{AnalysisError, capture_free_bindings},
    module::parameter_name_map,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidCoreModule(core::ValidationError),
    ItemMirrorCount {
        expected: usize,
        found: usize,
    },
    PipeMirrorCount {
        expected: usize,
        found: usize,
    },
    StageMirrorCount {
        expected: usize,
        found: usize,
    },
    ItemMirrorMismatch {
        item: core::ItemId,
    },
    PipeMirrorMismatch {
        pipe: core::PipeId,
    },
    StageMirrorMismatch {
        stage: core::StageId,
    },
    UnknownClosure {
        closure: ClosureId,
    },
    UnknownCapture {
        closure: ClosureId,
        capture: CaptureId,
    },
    CaptureOwnerMismatch {
        closure: ClosureId,
        capture: CaptureId,
        owner: ClosureId,
    },
    DuplicateClosureParameterBinding {
        closure: ClosureId,
        binding: BindingId,
    },
    DuplicateClosureCaptureBinding {
        closure: ClosureId,
        binding: BindingId,
    },
    ClosureMetadataMismatch(ClosureMetadataMismatch),
    MissingClosureCapture {
        closure: ClosureId,
        binding: BindingId,
    },
    UnexpectedClosureCapture {
        closure: ClosureId,
        binding: BindingId,
    },
    CaptureTypeMismatch {
        closure: ClosureId,
        capture: CaptureId,
        expected: core::Type,
        found: core::Type,
    },
    ClosureCaptureTypeConflict {
        closure: ClosureId,
        binding: BindingId,
        previous: core::Type,
        current: core::Type,
    },
}

/// Sub-variant of [`ValidationError::ClosureMetadataMismatch`] indicating which metadata field
/// on a closure does not match the typed-core site that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosureMetadataMismatch {
    /// The `owner` item ID recorded on the closure does not match the expected owner.
    Owner {
        closure: ClosureId,
        expected: core::ItemId,
        found: core::ItemId,
    },
    /// The `kind` of the closure does not match the expected kind for this site.
    Kind {
        closure: ClosureId,
        expected: crate::ClosureKind,
        found: crate::ClosureKind,
    },
    /// The `root` expression ID does not match the expected root for this site.
    Root {
        closure: ClosureId,
        expected: core::ExprId,
        found: core::ExprId,
    },
    /// The number of parameters does not match the expected count.
    ParameterCount {
        closure: ClosureId,
        expected: usize,
        found: usize,
    },
    /// The `ambient_subject` type does not match the expected subject for this site.
    AmbientSubject {
        closure: ClosureId,
        expected: Option<core::Type>,
        found: Option<core::Type>,
    },
    /// A basic structural invariant failed (e.g. the owner item or root expression does not
    /// exist in the module). This is reported when the closure cannot be checked at all.
    Invalid { closure: ClosureId },
}

impl std::fmt::Display for ClosureMetadataMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owner {
                closure,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} owner mismatch: expected item {expected}, found item {found}"
            ),
            Self::Kind {
                closure,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} kind mismatch: expected {expected}, found {found}"
            ),
            Self::Root {
                closure,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} root expression mismatch: expected expr {expected}, found expr {found}"
            ),
            Self::ParameterCount {
                closure,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} parameter count mismatch: expected {expected}, found {found}"
            ),
            Self::AmbientSubject {
                closure,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} ambient subject mismatch: expected {expected:?}, found {found:?}"
            ),
            Self::Invalid { closure } => write!(
                f,
                "typed-lambda closure {closure} refers to an owner or root that does not exist in the module"
            ),
        }
    }
}

impl std::error::Error for ClosureMetadataMismatch {}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCoreModule(error) => {
                write!(
                    f,
                    "typed-lambda module requires valid typed-core storage: {error}"
                )
            }
            Self::ItemMirrorCount { expected, found } => write!(
                f,
                "typed-lambda item mirror count disagrees with typed-core: expected {expected}, found {found}"
            ),
            Self::PipeMirrorCount { expected, found } => write!(
                f,
                "typed-lambda pipe mirror count disagrees with typed-core: expected {expected}, found {found}"
            ),
            Self::StageMirrorCount { expected, found } => write!(
                f,
                "typed-lambda stage mirror count disagrees with typed-core: expected {expected}, found {found}"
            ),
            Self::ItemMirrorMismatch { item } => {
                write!(
                    f,
                    "typed-lambda item mirror does not match typed-core item {item}"
                )
            }
            Self::PipeMirrorMismatch { pipe } => {
                write!(
                    f,
                    "typed-lambda pipe mirror does not match typed-core pipe {pipe}"
                )
            }
            Self::StageMirrorMismatch { stage } => {
                write!(
                    f,
                    "typed-lambda stage mirror does not match typed-core stage {stage}"
                )
            }
            Self::UnknownClosure { closure } => {
                write!(f, "typed-lambda references unknown closure {closure}")
            }
            Self::UnknownCapture { closure, capture } => write!(
                f,
                "typed-lambda closure {closure} references unknown capture {capture}"
            ),
            Self::CaptureOwnerMismatch {
                closure,
                capture,
                owner,
            } => write!(
                f,
                "typed-lambda closure {closure} references capture {capture} owned by closure {owner}"
            ),
            Self::DuplicateClosureParameterBinding { closure, binding } => write!(
                f,
                "typed-lambda closure {closure} lists local binding #{} more than once as a parameter",
                binding.as_raw()
            ),
            Self::DuplicateClosureCaptureBinding { closure, binding } => write!(
                f,
                "typed-lambda closure {closure} captures binding #{} more than once",
                binding.as_raw()
            ),
            Self::ClosureMetadataMismatch(mismatch) => write!(f, "{mismatch}"),
            Self::MissingClosureCapture { closure, binding } => write!(
                f,
                "typed-lambda closure {closure} is missing explicit capture metadata for binding #{}",
                binding.as_raw()
            ),
            Self::UnexpectedClosureCapture { closure, binding } => write!(
                f,
                "typed-lambda closure {closure} captures binding #{} even though it is not free in the body",
                binding.as_raw()
            ),
            Self::CaptureTypeMismatch {
                closure,
                capture,
                expected,
                found,
            } => write!(
                f,
                "typed-lambda closure {closure} capture {capture} changed type: expected {expected}, found {found}"
            ),
            Self::ClosureCaptureTypeConflict {
                closure,
                binding,
                previous,
                current,
            } => write!(
                f,
                "typed-lambda closure {closure} sees binding #{} at conflicting types: {} then {}",
                binding.as_raw(),
                previous,
                current
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

pub type ValidationErrors = aivi_base::ErrorCollection<ValidationError>;

pub fn validate_module(module: &Module) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();

    if let Err(core_errors) = core::validate_module(module.core()) {
        errors.extend(
            core_errors
                .into_errors()
                .into_iter()
                .map(ValidationError::InvalidCoreModule),
        );
    }

    if module.items().len() != module.core().items().len() {
        errors.push(ValidationError::ItemMirrorCount {
            expected: module.core().items().len(),
            found: module.items().len(),
        });
    }
    if module.pipes().len() != module.core().pipes().len() {
        errors.push(ValidationError::PipeMirrorCount {
            expected: module.core().pipes().len(),
            found: module.pipes().len(),
        });
    }
    if module.stages().len() != module.core().stages().len() {
        errors.push(ValidationError::StageMirrorCount {
            expected: module.core().stages().len(),
            found: module.stages().len(),
        });
    }

    for (item_id, core_item) in module.core().items().iter() {
        match module.items().get(item_id) {
            Some(item)
                if item.origin == core_item.origin
                    && item.span == core_item.span
                    && item.name == core_item.name
                    && item.kind == core_item.kind
                    && parameters_mirror(&item.parameters, &core_item.parameters)
                    && item.pipes == core_item.pipes =>
            {
                validate_item_body(module, item_id, item, core_item, &mut errors);
            }
            Some(_) | None => errors.push(ValidationError::ItemMirrorMismatch { item: item_id }),
        }
    }

    for (stage_id, core_stage) in module.core().stages().iter() {
        match module.stages().get(stage_id) {
            Some(stage)
                if stage.pipe == core_stage.pipe
                    && stage.index == core_stage.index
                    && stage.span == core_stage.span
                    && stage.input_subject == core_stage.input_subject
                    && stage.result_subject == core_stage.result_subject =>
            {
                validate_stage(module, stage_id, stage, core_stage, &mut errors);
            }
            Some(_) | None => errors.push(ValidationError::StageMirrorMismatch { stage: stage_id }),
        }
    }

    for (pipe_id, core_pipe) in module.core().pipes().iter() {
        match module.pipes().get(pipe_id) {
            Some(pipe)
                if pipe.owner == core_pipe.owner
                    && pipe.origin == core_pipe.origin
                    && pipe.stages == core_pipe.stages =>
            {
                validate_pipe(module, pipe_id, pipe, core_pipe, &mut errors);
            }
            Some(_) | None => errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id }),
        }
    }

    for (closure_id, closure) in module.closures().iter() {
        validate_closure(module, closure_id, closure, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors::new(errors))
    }
}

fn validate_item_body(
    module: &Module,
    item_id: core::ItemId,
    item: &Item,
    core_item: &core::Item,
    errors: &mut Vec<ValidationError>,
) {
    match (item.body, core_item.body) {
        (None, None) => {}
        (Some(closure), Some(root)) => validate_expected_closure(
            module,
            closure,
            item_id,
            ClosureKind::ItemBody,
            None,
            &item.parameters,
            root,
            errors,
        ),
        _ => errors.push(ValidationError::ItemMirrorMismatch { item: item_id }),
    }
}

fn validate_pipe(
    module: &Module,
    pipe_id: core::PipeId,
    pipe: &Pipe,
    core_pipe: &core::Pipe,
    errors: &mut Vec<ValidationError>,
) {
    match (&pipe.recurrence, &core_pipe.recurrence) {
        (None, None) => {}
        (Some(recurrence), Some(core_recurrence)) => {
            if recurrence.target != core_recurrence.target
                || recurrence.wakeup != core_recurrence.wakeup
            {
                errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id });
                return;
            }
            validate_recurrence_stage(
                module,
                pipe_id,
                pipe.owner,
                ClosureKind::RecurrenceStart,
                &recurrence.start,
                &core_recurrence.start,
                errors,
            );
            if recurrence.steps.len() != core_recurrence.steps.len() {
                errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id });
            }
            for (step, core_step) in recurrence.steps.iter().zip(core_recurrence.steps.iter()) {
                validate_recurrence_stage(
                    module,
                    pipe_id,
                    pipe.owner,
                    ClosureKind::RecurrenceStep,
                    step,
                    core_step,
                    errors,
                );
            }
            match (
                &recurrence.non_source_wakeup,
                &core_recurrence.non_source_wakeup,
            ) {
                (None, None) => {}
                (Some(wakeup), Some(core_wakeup)) => {
                    if wakeup.cause != core_wakeup.cause
                        || wakeup.witness_expr != core_wakeup.witness_expr
                    {
                        errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id });
                    } else {
                        validate_expected_closure(
                            module,
                            wakeup.runtime,
                            pipe.owner,
                            ClosureKind::RecurrenceWakeupWitness,
                            None,
                            &[],
                            core_wakeup.runtime_witness,
                            errors,
                        );
                    }
                }
                _ => errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id }),
            }
        }
        _ => errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id }),
    }
}

fn validate_recurrence_stage(
    module: &Module,
    pipe_id: core::PipeId,
    owner: core::ItemId,
    kind: ClosureKind,
    stage: &RecurrenceStage,
    core_stage: &core::RecurrenceStage,
    errors: &mut Vec<ValidationError>,
) {
    if stage.stage_index != core_stage.stage_index
        || stage.stage_span != core_stage.stage_span
        || stage.origin_expr != core_stage.origin_expr
        || stage.input_subject != core_stage.input_subject
        || stage.result_subject != core_stage.result_subject
    {
        errors.push(ValidationError::PipeMirrorMismatch { pipe: pipe_id });
        return;
    }
    validate_expected_closure(
        module,
        stage.runtime,
        owner,
        kind,
        Some(&stage.input_subject),
        &[],
        core_stage.runtime_expr,
        errors,
    );
}

fn validate_stage(
    module: &Module,
    stage_id: core::StageId,
    stage: &Stage,
    core_stage: &core::Stage,
    errors: &mut Vec<ValidationError>,
) {
    match (&stage.kind, &core_stage.kind) {
        (
            StageKind::Gate(GateStage::Ordinary {
                when_true,
                when_false,
            }),
            core::StageKind::Gate(core::GateStage::Ordinary {
                when_true: core_true,
                when_false: core_false,
            }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *when_true,
                owner,
                ClosureKind::GateTrue,
                Some(&stage.input_subject),
                &[],
                *core_true,
                errors,
            );
            validate_expected_closure(
                module,
                *when_false,
                owner,
                ClosureKind::GateFalse,
                Some(&stage.input_subject),
                &[],
                *core_false,
                errors,
            );
        }
        (
            StageKind::Gate(GateStage::SignalFilter {
                payload_type,
                predicate,
                emits_negative_update,
            }),
            core::StageKind::Gate(core::GateStage::SignalFilter {
                payload_type: core_payload,
                predicate: core_predicate,
                emits_negative_update: core_negative,
            }),
        ) if payload_type == core_payload && emits_negative_update == core_negative => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *predicate,
                owner,
                ClosureKind::SignalFilterPredicate,
                Some(payload_type),
                &[],
                *core_predicate,
                errors,
            );
        }
        (
            StageKind::Temporal(TemporalStage::Previous { seed }),
            core::StageKind::Temporal(core::TemporalStage::Previous { seed_expr }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *seed,
                owner,
                ClosureKind::PreviousSeed,
                None,
                &[],
                *seed_expr,
                errors,
            );
        }
        (
            StageKind::Temporal(TemporalStage::DiffFunction { diff }),
            core::StageKind::Temporal(core::TemporalStage::DiffFunction { diff_expr }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *diff,
                owner,
                ClosureKind::DiffFunction,
                None,
                &[],
                *diff_expr,
                errors,
            );
        }
        (
            StageKind::Temporal(TemporalStage::DiffSeed { seed }),
            core::StageKind::Temporal(core::TemporalStage::DiffSeed { seed_expr }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *seed,
                owner,
                ClosureKind::DiffSeed,
                None,
                &[],
                *seed_expr,
                errors,
            );
        }
        (
            StageKind::Temporal(TemporalStage::Delay { duration }),
            core::StageKind::Temporal(core::TemporalStage::Delay { duration_expr }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *duration,
                owner,
                ClosureKind::DelayDuration,
                None,
                &[],
                *duration_expr,
                errors,
            );
        }
        (
            StageKind::Temporal(TemporalStage::Burst { every, count }),
            core::StageKind::Temporal(core::TemporalStage::Burst {
                every_expr,
                count_expr,
            }),
        ) => {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                *every,
                owner,
                ClosureKind::BurstEvery,
                None,
                &[],
                *every_expr,
                errors,
            );
            validate_expected_closure(
                module,
                *count,
                owner,
                ClosureKind::BurstCount,
                None,
                &[],
                *count_expr,
                errors,
            );
        }
        (StageKind::TruthyFalsy(pair), core::StageKind::TruthyFalsy(core_pair))
            if pair == core_pair => {}
        (StageKind::Fanout(fanout), core::StageKind::Fanout(core_fanout))
            if fanout.carrier == core_fanout.carrier
                && fanout.element_subject == core_fanout.element_subject
                && fanout.mapped_element_type == core_fanout.mapped_element_type
                && fanout.mapped_collection_type == core_fanout.mapped_collection_type
                && fanout.filters.len() == core_fanout.filters.len()
                && fanout.join.is_some() == core_fanout.join.is_some() =>
        {
            let owner = module.core().pipes()[stage.pipe].owner;
            validate_expected_closure(
                module,
                fanout.map,
                owner,
                ClosureKind::FanoutMap,
                Some(&core_fanout.element_subject),
                &[],
                core_fanout.runtime_map,
                errors,
            );
            for (filter, core_filter) in fanout.filters.iter().zip(&core_fanout.filters) {
                if filter.stage_index != core_filter.stage_index
                    || filter.stage_span != core_filter.stage_span
                    || filter.predicate_expr != core_filter.predicate_expr
                    || filter.input_subject != core_filter.input_subject
                {
                    errors.push(ValidationError::StageMirrorMismatch { stage: stage_id });
                    return;
                }
                validate_expected_closure(
                    module,
                    filter.runtime,
                    owner,
                    ClosureKind::FanoutFilterPredicate,
                    Some(&core_filter.input_subject),
                    &[],
                    core_filter.runtime_predicate,
                    errors,
                );
            }
            match (&fanout.join, &core_fanout.join) {
                (Some(join), Some(core_join)) => {
                    if join.stage_index != core_join.stage_index
                        || join.stage_span != core_join.stage_span
                        || join.origin_expr != core_join.origin_expr
                        || join.input_subject != core_join.input_subject
                        || join.collection_subject != core_join.collection_subject
                        || join.result_type != core_join.result_type
                    {
                        errors.push(ValidationError::StageMirrorMismatch { stage: stage_id });
                        return;
                    }
                    validate_expected_closure(
                        module,
                        join.runtime,
                        owner,
                        ClosureKind::FanoutJoin,
                        Some(&core_join.collection_subject),
                        &[],
                        core_join.runtime_expr,
                        errors,
                    );
                }
                (None, None) => {}
                _ => errors.push(ValidationError::StageMirrorMismatch { stage: stage_id }),
            }
        }
        _ => errors.push(ValidationError::StageMirrorMismatch { stage: stage_id }),
    }
}

fn validate_expected_closure(
    module: &Module,
    closure_id: ClosureId,
    expected_owner: core::ItemId,
    expected_kind: ClosureKind,
    expected_subject: Option<&core::Type>,
    expected_parameters: &[Parameter],
    expected_root: core::ExprId,
    errors: &mut Vec<ValidationError>,
) {
    let Some(closure) = module.closures().get(closure_id) else {
        errors.push(ValidationError::UnknownClosure {
            closure: closure_id,
        });
        return;
    };
    if closure.owner != expected_owner {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::Owner {
                closure: closure_id,
                expected: expected_owner,
                found: closure.owner,
            },
        ));
    }
    if closure.kind != expected_kind {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::Kind {
                closure: closure_id,
                expected: expected_kind,
                found: closure.kind,
            },
        ));
    }
    if closure.root != expected_root {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::Root {
                closure: closure_id,
                expected: expected_root,
                found: closure.root,
            },
        ));
    }
    if closure.parameters.len() != expected_parameters.len() {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::ParameterCount {
                closure: closure_id,
                expected: expected_parameters.len(),
                found: closure.parameters.len(),
            },
        ));
    }
    if closure.ambient_subject.as_ref() != expected_subject {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::AmbientSubject {
                closure: closure_id,
                expected: expected_subject.cloned(),
                found: closure.ambient_subject.clone(),
            },
        ));
    }
}

fn validate_closure(
    module: &Module,
    closure_id: ClosureId,
    closure: &crate::Closure,
    errors: &mut Vec<ValidationError>,
) {
    if !module.items().contains(closure.owner) || !module.exprs().contains(closure.root) {
        errors.push(ValidationError::ClosureMetadataMismatch(
            ClosureMetadataMismatch::Invalid {
                closure: closure_id,
            },
        ));
        return;
    }

    let mut parameter_bindings = BTreeSet::new();
    for parameter in &closure.parameters {
        if !parameter_bindings.insert(parameter.binding) {
            errors.push(ValidationError::DuplicateClosureParameterBinding {
                closure: closure_id,
                binding: parameter.binding,
            });
        }
    }

    let known_names = parameter_name_map(&closure.parameters);
    let expected_captures = match capture_free_bindings(
        module.core(),
        closure.root,
        &closure
            .parameters
            .iter()
            .map(|parameter| parameter.binding)
            .collect::<Vec<_>>(),
        &known_names,
    ) {
        Ok(captures) => captures,
        Err(AnalysisError::BindingTypeConflict {
            binding,
            previous,
            current,
            ..
        }) => {
            errors.push(ValidationError::ClosureCaptureTypeConflict {
                closure: closure_id,
                binding,
                previous,
                current,
            });
            return;
        }
    };

    let mut actual_by_binding = BTreeMap::<BindingId, (CaptureId, core::Type)>::new();
    for capture_id in &closure.captures {
        let Some(capture) = module.captures().get(*capture_id) else {
            errors.push(ValidationError::UnknownCapture {
                closure: closure_id,
                capture: *capture_id,
            });
            continue;
        };
        if capture.closure != closure_id {
            errors.push(ValidationError::CaptureOwnerMismatch {
                closure: closure_id,
                capture: *capture_id,
                owner: capture.closure,
            });
            continue;
        }
        if let Some((_, _)) =
            actual_by_binding.insert(capture.binding, (*capture_id, capture.ty.clone()))
        {
            errors.push(ValidationError::DuplicateClosureCaptureBinding {
                closure: closure_id,
                binding: capture.binding,
            });
        }
    }

    let expected_by_binding = expected_captures
        .into_iter()
        .map(|capture| (capture.binding, capture.ty))
        .collect::<BTreeMap<_, _>>();

    for (binding, expected_ty) in &expected_by_binding {
        match actual_by_binding.get(binding) {
            Some((capture_id, found_ty)) if found_ty == expected_ty => {
                let _ = capture_id;
            }
            Some((capture_id, found_ty)) => errors.push(ValidationError::CaptureTypeMismatch {
                closure: closure_id,
                capture: *capture_id,
                expected: expected_ty.clone(),
                found: found_ty.clone(),
            }),
            None => errors.push(ValidationError::MissingClosureCapture {
                closure: closure_id,
                binding: *binding,
            }),
        }
    }

    for binding in actual_by_binding.keys() {
        if !expected_by_binding.contains_key(binding) {
            errors.push(ValidationError::UnexpectedClosureCapture {
                closure: closure_id,
                binding: *binding,
            });
        }
    }
}

fn parameters_mirror(lambda: &[Parameter], core: &[core::ItemParameter]) -> bool {
    lambda.len() == core.len()
        && lambda.iter().zip(core.iter()).all(|(l, c)| {
            l.binding == c.binding && l.span == c.span && l.name == c.name && l.ty == c.ty
        })
}
