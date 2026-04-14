use std::collections::BTreeMap;

use aivi_base::SourceSpan;
use aivi_typing::BuiltinSourceProvider;

use crate::{
    BuiltinTerm, BuiltinType, DecoratorPayload, Expr, ExprId, ExprKind, GateType, Item, ItemHeader,
    ItemId, Module, Name, NamePath, NonEmpty, PipeExpr, PipeStage, PipeStageKind, ProjectionBase,
    ReactiveUpdateBodyMode, ReactiveUpdateClause, RecordExpr, RecordExprField, RecordFieldSurface,
    ResolutionState, SignalItem, SourceDecorator, TermReference, TermResolution, TypeKind,
    TypeNode, TypeReference, TypeResolution, signal_payload_type,
};

pub(crate) fn elaborate_resource_signal_companions(module: &mut Module) {
    let candidates = collect_resource_signal_candidates(module);
    if candidates.is_empty() {
        return;
    }

    let mut rewrites = BTreeMap::new();
    for candidate in candidates {
        let targets = apply_candidate(module, &candidate);
        rewrites.insert(candidate.raw_signal, targets);
    }

    rewrite_resource_signal_projections(module, &rewrites);
}

#[derive(Clone, Debug)]
struct ResourceSignalCandidate {
    raw_signal: ItemId,
    span: SourceSpan,
    name: Name,
    decorator_id: crate::DecoratorId,
    trigger_option_name: &'static str,
    existing_trigger_signal: Option<ItemId>,
    active_when_expr: Option<ExprId>,
    active_when_signal: Option<ItemId>,
    value_payload: GateType,
    error_payload: GateType,
}

#[derive(Clone, Copy, Debug)]
struct CompanionTargets {
    run: ItemId,
    success: ItemId,
    error: ItemId,
    loading: ItemId,
}

fn collect_resource_signal_candidates(module: &Module) -> Vec<ResourceSignalCandidate> {
    let item_ids = module
        .items()
        .iter()
        .map(|(item_id, _)| item_id)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for item_id in item_ids {
        let Some(candidate) = collect_resource_signal_candidate(module, item_id) else {
            continue;
        };
        candidates.push(candidate);
    }
    candidates
}

fn collect_resource_signal_candidate(
    module: &Module,
    item_id: ItemId,
) -> Option<ResourceSignalCandidate> {
    let Item::Signal(signal) = &module.items()[item_id] else {
        return None;
    };
    if signal.is_source_capability_handle {
        return None;
    }
    let metadata = signal.source_metadata.as_ref()?;
    let provider = metadata.provider.builtin()?;
    let (error_payload, value_payload) = match signal_payload_type(module, signal)? {
        GateType::Result { error, value } => ((*error).clone(), (*value).clone()),
        _ => return None,
    };
    let trigger_option_name = provider.contract().trigger_signal_option()?.name();
    let (decorator_id, source) = signal_source_decorator(module, signal)?;
    let existing_trigger_expr =
        source_option_expr(module, source, trigger_option_name).map(|(expr, _)| expr);
    let existing_trigger_signal = match existing_trigger_expr {
        Some(expr) => {
            let signal =
                resolve_resource_option_signal_binding(module, provider, trigger_option_name, expr);
            if signal.is_none() {
                return None;
            }
            signal
        }
        None => None,
    };
    let active_when_expr = source_option_expr(module, source, "activeWhen").map(|(expr, _)| expr);
    let active_when_signal = active_when_expr.and_then(|expr| {
        resolve_resource_option_signal_binding(module, provider, "activeWhen", expr)
    });
    Some(ResourceSignalCandidate {
        raw_signal: item_id,
        span: signal.header.span,
        name: signal.name.clone(),
        decorator_id,
        trigger_option_name,
        existing_trigger_signal,
        active_when_expr,
        active_when_signal,
        value_payload,
        error_payload,
    })
}

fn resolve_resource_option_signal_binding(
    module: &Module,
    provider: BuiltinSourceProvider,
    option_name: &str,
    expr: ExprId,
) -> Option<ItemId> {
    let allow_db_changed_projection =
        matches!(provider, BuiltinSourceProvider::DbLive) && option_name == "refreshOn";
    crate::source_lifecycle_elaboration::resolve_source_option_signal_binding(
        module,
        expr,
        allow_db_changed_projection,
    )
}

fn apply_candidate(module: &mut Module, candidate: &ResourceSignalCandidate) -> CompanionTargets {
    let run = synthesize_run_signal(module, candidate);
    let trigger = synthesize_trigger_signal(module, candidate, run);
    bind_source_trigger_option(module, candidate, trigger);
    let success = synthesize_success_signal(module, candidate);
    let error = synthesize_error_signal(module, candidate);
    let loading = synthesize_loading_signal(module, candidate, trigger);
    CompanionTargets {
        run,
        success,
        error,
        loading,
    }
}

fn synthesize_run_signal(module: &mut Module, candidate: &ResourceSignalCandidate) -> ItemId {
    let unit_signal = synthesize_signal_annotation(module, candidate.span, BuiltinType::Unit);
    module
        .push_item(Item::Signal(SignalItem {
            header: ItemHeader {
                span: candidate.span,
                decorators: Vec::new(),
            },
            name: hidden_signal_name(candidate, "run"),
            annotation: Some(unit_signal),
            body: None,
            reactive_updates: Vec::new(),
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            temporal_input_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }))
        .expect("resource companion run signal should fit in the item arena")
}

fn synthesize_trigger_signal(
    module: &mut Module,
    candidate: &ResourceSignalCandidate,
    run: ItemId,
) -> ItemId {
    let bool_signal = synthesize_signal_annotation(module, candidate.span, BuiltinType::Bool);
    let false_expr = builtin_term_expr(module, BuiltinTerm::False, candidate.span);
    let mut reactive_updates = Vec::new();
    if let Some(existing_trigger) = candidate.existing_trigger_signal {
        reactive_updates.push(constant_reactive_update(
            module,
            candidate.span,
            existing_trigger,
            BuiltinTerm::True,
        ));
    }
    reactive_updates.push(constant_reactive_update(
        module,
        candidate.span,
        run,
        BuiltinTerm::True,
    ));
    module
        .push_item(Item::Signal(SignalItem {
            header: ItemHeader {
                span: candidate.span,
                decorators: Vec::new(),
            },
            name: hidden_signal_name(candidate, "trigger"),
            annotation: Some(bool_signal),
            body: Some(false_expr),
            reactive_updates,
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            temporal_input_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }))
        .expect("resource companion trigger signal should fit in the item arena")
}

fn synthesize_success_signal(module: &mut Module, candidate: &ResourceSignalCandidate) -> ItemId {
    let body = success_pipe_expr(module, candidate.raw_signal, candidate.span);
    let option_payload = gate_type_to_type_id(
        module,
        candidate.span,
        &GateType::Option(Box::new(candidate.value_payload.clone())),
    )
    .expect("resource success companions should convert their payload type back into HIR");
    let annotation =
        synthesize_signal_annotation_with_payload(module, candidate.span, option_payload);
    module
        .push_item(Item::Signal(SignalItem {
            header: ItemHeader {
                span: candidate.span,
                decorators: Vec::new(),
            },
            name: hidden_signal_name(candidate, "success"),
            annotation: Some(annotation),
            body: Some(body),
            reactive_updates: Vec::new(),
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            temporal_input_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }))
        .expect("resource companion success signal should fit in the item arena")
}

fn synthesize_error_signal(module: &mut Module, candidate: &ResourceSignalCandidate) -> ItemId {
    let body = error_pipe_expr(module, candidate.raw_signal, candidate.span);
    let option_payload = gate_type_to_type_id(
        module,
        candidate.span,
        &GateType::Option(Box::new(candidate.error_payload.clone())),
    )
    .expect("resource error companions should convert their payload type back into HIR");
    let annotation =
        synthesize_signal_annotation_with_payload(module, candidate.span, option_payload);
    module
        .push_item(Item::Signal(SignalItem {
            header: ItemHeader {
                span: candidate.span,
                decorators: Vec::new(),
            },
            name: hidden_signal_name(candidate, "error"),
            annotation: Some(annotation),
            body: Some(body),
            reactive_updates: Vec::new(),
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            temporal_input_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }))
        .expect("resource companion error signal should fit in the item arena")
}

fn synthesize_loading_signal(
    module: &mut Module,
    candidate: &ResourceSignalCandidate,
    trigger: ItemId,
) -> ItemId {
    let bool_signal = synthesize_signal_annotation(module, candidate.span, BuiltinType::Bool);
    let body = candidate
        .active_when_expr
        .unwrap_or_else(|| builtin_term_expr(module, BuiltinTerm::True, candidate.span));
    let mut reactive_updates = Vec::new();
    if let Some(active_when_expr) = candidate.active_when_expr {
        reactive_updates.push(ReactiveUpdateClause {
            span: candidate.span,
            keyword_span: candidate.span,
            target_span: candidate.span,
            guard: builtin_term_expr(module, BuiltinTerm::True, candidate.span),
            body: active_when_expr,
            body_mode: ReactiveUpdateBodyMode::Payload,
            trigger_source: candidate.active_when_signal,
        });
    }
    let trigger_body = candidate
        .active_when_expr
        .unwrap_or_else(|| builtin_term_expr(module, BuiltinTerm::True, candidate.span));
    reactive_updates.push(ReactiveUpdateClause {
        span: candidate.span,
        keyword_span: candidate.span,
        target_span: candidate.span,
        guard: builtin_term_expr(module, BuiltinTerm::True, candidate.span),
        body: trigger_body,
        body_mode: ReactiveUpdateBodyMode::Payload,
        trigger_source: Some(trigger),
    });
    reactive_updates.push(constant_reactive_update(
        module,
        candidate.span,
        candidate.raw_signal,
        BuiltinTerm::False,
    ));
    module
        .push_item(Item::Signal(SignalItem {
            header: ItemHeader {
                span: candidate.span,
                decorators: Vec::new(),
            },
            name: hidden_signal_name(candidate, "loading"),
            annotation: Some(bool_signal),
            body: Some(body),
            reactive_updates,
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            temporal_input_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }))
        .expect("resource companion loading signal should fit in the item arena")
}

fn constant_reactive_update(
    module: &mut Module,
    span: SourceSpan,
    trigger_source: ItemId,
    value: BuiltinTerm,
) -> ReactiveUpdateClause {
    ReactiveUpdateClause {
        span,
        keyword_span: span,
        target_span: span,
        guard: builtin_term_expr(module, BuiltinTerm::True, span),
        body: builtin_term_expr(module, value, span),
        body_mode: ReactiveUpdateBodyMode::Payload,
        trigger_source: Some(trigger_source),
    }
}

fn bind_source_trigger_option(
    module: &mut Module,
    candidate: &ResourceSignalCandidate,
    trigger: ItemId,
) {
    let trigger_expr = item_name_expr(module, trigger, candidate.span);
    let Some(source_options) =
        module
            .decorators()
            .get(candidate.decorator_id)
            .and_then(|decorator| match &decorator.payload {
                DecoratorPayload::Source(source) => Some(source.options),
                _ => None,
            })
    else {
        return;
    };
    match source_options {
        Some(options_expr) => {
            let Some(expr) = module.arenas.exprs.get_mut(options_expr) else {
                return;
            };
            let ExprKind::Record(record) = &mut expr.kind else {
                return;
            };
            if let Some(field) = record
                .fields
                .iter_mut()
                .find(|field| field.label.text() == candidate.trigger_option_name)
            {
                field.value = trigger_expr;
            } else {
                record.fields.push(RecordExprField {
                    span: candidate.span,
                    label: name(candidate.trigger_option_name, candidate.span),
                    value: trigger_expr,
                    surface: RecordFieldSurface::Explicit,
                });
            }
        }
        None => {
            let options = module
                .alloc_expr(Expr {
                    span: candidate.span,
                    kind: ExprKind::Record(RecordExpr {
                        fields: vec![RecordExprField {
                            span: candidate.span,
                            label: name(candidate.trigger_option_name, candidate.span),
                            value: trigger_expr,
                            surface: RecordFieldSurface::Explicit,
                        }],
                    }),
                })
                .expect("resource companion source options should fit in the expression arena");
            let Some(decorator) = module.arenas.decorators.get_mut(candidate.decorator_id) else {
                return;
            };
            let DecoratorPayload::Source(source) = &mut decorator.payload else {
                return;
            };
            source.options = Some(options);
        }
    }
}

fn rewrite_resource_signal_projections(
    module: &mut Module,
    rewrites: &BTreeMap<ItemId, CompanionTargets>,
) {
    let expr_ids = module
        .exprs()
        .iter()
        .map(|(expr_id, _)| expr_id)
        .collect::<Vec<_>>();
    for expr_id in expr_ids {
        let Some((base_expr, path)) = projection_rewrite_target(module, expr_id) else {
            continue;
        };
        let ExprKind::Name(reference) = &module.exprs()[base_expr].kind else {
            continue;
        };
        let ResolutionState::Resolved(TermResolution::Item(item_id)) = reference.resolution else {
            continue;
        };
        let Some(targets) = rewrites.get(&item_id).copied() else {
            continue;
        };
        let Some((target_item, remaining_path)) = projection_target_for_path(&path, targets) else {
            continue;
        };
        let span = module.exprs()[expr_id].span;
        let target_expr = item_name_expr(module, target_item, span);
        let target_kind = module.exprs()[target_expr].kind.clone();
        let Some(expr) = module.arenas.exprs.get_mut(expr_id) else {
            continue;
        };
        expr.kind = if let Some(path) = remaining_path {
            ExprKind::Projection {
                base: ProjectionBase::Expr(target_expr),
                path,
            }
        } else {
            target_kind
        };
    }
}

fn projection_rewrite_target(module: &Module, expr_id: ExprId) -> Option<(ExprId, NamePath)> {
    let ExprKind::Projection {
        base: ProjectionBase::Expr(base_expr),
        path,
    } = &module.exprs()[expr_id].kind
    else {
        return None;
    };
    Some((*base_expr, path.clone()))
}

fn projection_target_for_path(
    path: &NamePath,
    targets: CompanionTargets,
) -> Option<(ItemId, Option<NamePath>)> {
    let mut segments = path.segments().iter();
    let first = segments.next()?;
    let target = match first.text() {
        "run" => targets.run,
        "success" => targets.success,
        "error" => targets.error,
        "loading" => targets.loading,
        _ => return None,
    };
    let remaining = segments.cloned().collect::<Vec<_>>();
    let remaining = (!remaining.is_empty()).then(|| {
        NamePath::from_vec(remaining)
            .expect("resource companion projection tails should stay non-empty and same-file")
    });
    Some((target, remaining))
}

fn success_pipe_expr(module: &mut Module, raw_signal: ItemId, span: SourceSpan) -> ExprId {
    let head = item_name_expr(module, raw_signal, span);
    let ambient = ambient_subject_expr(module, span);
    let some_payload = some_expr(module, ambient, span);
    let none_expr = builtin_term_expr(module, BuiltinTerm::None, span);
    pipe_expr(
        module,
        head,
        vec![
            PipeStage {
                span,
                subject_memo: None,
                result_memo: None,
                kind: PipeStageKind::Truthy { expr: some_payload },
            },
            PipeStage {
                span,
                subject_memo: None,
                result_memo: None,
                kind: PipeStageKind::Falsy { expr: none_expr },
            },
        ],
    )
}

fn error_pipe_expr(module: &mut Module, raw_signal: ItemId, span: SourceSpan) -> ExprId {
    let head = item_name_expr(module, raw_signal, span);
    let none_expr = builtin_term_expr(module, BuiltinTerm::None, span);
    let ambient = ambient_subject_expr(module, span);
    let some_payload = some_expr(module, ambient, span);
    pipe_expr(
        module,
        head,
        vec![
            PipeStage {
                span,
                subject_memo: None,
                result_memo: None,
                kind: PipeStageKind::Truthy { expr: none_expr },
            },
            PipeStage {
                span,
                subject_memo: None,
                result_memo: None,
                kind: PipeStageKind::Falsy { expr: some_payload },
            },
        ],
    )
}

fn pipe_expr(module: &mut Module, head: ExprId, stages: Vec<PipeStage>) -> ExprId {
    let stages = NonEmpty::from_vec(stages)
        .expect("resource companion helper pipes should always contain at least one stage");
    module
        .alloc_expr(Expr {
            span: module.exprs()[head].span,
            kind: ExprKind::Pipe(PipeExpr {
                head,
                stages,
                result_block_desugaring: false,
            }),
        })
        .expect("resource companion helper pipe should fit in the expression arena")
}

fn ambient_subject_expr(module: &mut Module, span: SourceSpan) -> ExprId {
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::AmbientSubject,
        })
        .expect("resource companion ambient subject should fit in the expression arena")
}

fn some_expr(module: &mut Module, payload: ExprId, span: SourceSpan) -> ExprId {
    let callee = builtin_term_expr(module, BuiltinTerm::Some, span);
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Apply {
                callee,
                arguments: NonEmpty::new(payload, Vec::new()),
            },
        })
        .expect("resource companion Some application should fit in the expression arena")
}

fn synthesize_signal_annotation(
    module: &mut Module,
    span: SourceSpan,
    payload: BuiltinType,
) -> crate::TypeId {
    let payload = builtin_type(module, payload, span);
    synthesize_signal_annotation_with_payload(module, span, payload)
}

fn synthesize_signal_annotation_with_payload(
    module: &mut Module,
    span: SourceSpan,
    payload: crate::TypeId,
) -> crate::TypeId {
    let signal_callee = builtin_type(module, BuiltinType::Signal, span);
    module
        .alloc_type(TypeNode {
            span,
            kind: TypeKind::Apply {
                callee: signal_callee,
                arguments: NonEmpty::new(payload, Vec::new()),
            },
        })
        .expect("resource companion signal annotation should fit in the type arena")
}

fn gate_type_to_type_id(
    module: &mut Module,
    span: SourceSpan,
    ty: &GateType,
) -> Option<crate::TypeId> {
    match ty {
        GateType::Primitive(builtin) => Some(builtin_type(module, *builtin, span)),
        GateType::TypeParameter {
            parameter,
            name: parameter_name,
        } => module
            .alloc_type(TypeNode {
                span,
                kind: TypeKind::Name(TypeReference::resolved(
                    name_path(&[name(parameter_name, span)]),
                    TypeResolution::TypeParameter(*parameter),
                )),
            })
            .ok(),
        GateType::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(|element| gate_type_to_type_id(module, span, element))
                .collect::<Option<Vec<_>>>()?;
            let elements = crate::AtLeastTwo::from_vec(elements).ok()?;
            module
                .alloc_type(TypeNode {
                    span,
                    kind: TypeKind::Tuple(elements),
                })
                .ok()
        }
        GateType::Record(fields) => {
            let fields = fields
                .iter()
                .map(|field| {
                    Some(crate::TypeField {
                        span,
                        label: name(&field.name, span),
                        ty: gate_type_to_type_id(module, span, &field.ty)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            module
                .alloc_type(TypeNode {
                    span,
                    kind: TypeKind::Record(fields),
                })
                .ok()
        }
        GateType::Arrow { parameter, result } => {
            let parameter = gate_type_to_type_id(module, span, parameter)?;
            let result = gate_type_to_type_id(module, span, result)?;
            module
                .alloc_type(TypeNode {
                    span,
                    kind: TypeKind::Arrow { parameter, result },
                })
                .ok()
        }
        GateType::List(element) => {
            builtin_apply_type(module, span, BuiltinType::List, &[element.as_ref()])
        }
        GateType::Map { key, value } => builtin_apply_type(
            module,
            span,
            BuiltinType::Map,
            &[key.as_ref(), value.as_ref()],
        ),
        GateType::Set(element) => {
            builtin_apply_type(module, span, BuiltinType::Set, &[element.as_ref()])
        }
        GateType::Option(element) => {
            builtin_apply_type(module, span, BuiltinType::Option, &[element.as_ref()])
        }
        GateType::Result { error, value } => builtin_apply_type(
            module,
            span,
            BuiltinType::Result,
            &[error.as_ref(), value.as_ref()],
        ),
        GateType::Validation { error, value } => builtin_apply_type(
            module,
            span,
            BuiltinType::Validation,
            &[error.as_ref(), value.as_ref()],
        ),
        GateType::Signal(element) => {
            builtin_apply_type(module, span, BuiltinType::Signal, &[element.as_ref()])
        }
        GateType::Task { error, value } => builtin_apply_type(
            module,
            span,
            BuiltinType::Task,
            &[error.as_ref(), value.as_ref()],
        ),
        GateType::Domain {
            item,
            name: domain_name,
            arguments,
        }
        | GateType::OpaqueItem {
            item,
            name: domain_name,
            arguments,
        } => resolved_apply_type(
            module,
            span,
            TypeReference::resolved(
                name_path(&[name(domain_name, span)]),
                TypeResolution::Item(*item),
            ),
            arguments,
        ),
        GateType::OpaqueImport {
            import,
            name: import_name,
            arguments,
            ..
        } => resolved_apply_type(
            module,
            span,
            TypeReference::resolved(
                name_path(&[name(import_name, span)]),
                TypeResolution::Import(*import),
            ),
            arguments,
        ),
    }
}

fn builtin_apply_type(
    module: &mut Module,
    span: SourceSpan,
    builtin: BuiltinType,
    arguments: &[&GateType],
) -> Option<crate::TypeId> {
    let callee = builtin_type(module, builtin, span);
    let arguments = arguments
        .iter()
        .map(|argument| gate_type_to_type_id(module, span, argument))
        .collect::<Option<Vec<_>>>()?;
    let arguments = NonEmpty::from_vec(arguments).ok()?;
    module
        .alloc_type(TypeNode {
            span,
            kind: TypeKind::Apply { callee, arguments },
        })
        .ok()
}

fn resolved_apply_type(
    module: &mut Module,
    span: SourceSpan,
    reference: TypeReference,
    arguments: &[GateType],
) -> Option<crate::TypeId> {
    let callee = module
        .alloc_type(TypeNode {
            span,
            kind: TypeKind::Name(reference),
        })
        .ok()?;
    if arguments.is_empty() {
        return Some(callee);
    }
    let arguments = arguments
        .iter()
        .map(|argument| gate_type_to_type_id(module, span, argument))
        .collect::<Option<Vec<_>>>()?;
    let arguments = NonEmpty::from_vec(arguments).ok()?;
    module
        .alloc_type(TypeNode {
            span,
            kind: TypeKind::Apply { callee, arguments },
        })
        .ok()
}

fn builtin_type(module: &mut Module, builtin: BuiltinType, span: SourceSpan) -> crate::TypeId {
    module
        .alloc_type(TypeNode {
            span,
            kind: TypeKind::Name(TypeReference::resolved(
                name_path(&[name(builtin_type_name(builtin), span)]),
                TypeResolution::Builtin(builtin),
            )),
        })
        .expect("resource companion builtin type should fit in the type arena")
}

fn builtin_term_expr(module: &mut Module, builtin: BuiltinTerm, span: SourceSpan) -> ExprId {
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                name_path(&[name(builtin_term_name(builtin), span)]),
                TermResolution::Builtin(builtin),
            )),
        })
        .expect("resource companion builtin term should fit in the expression arena")
}

fn item_name_expr(module: &mut Module, item: ItemId, span: SourceSpan) -> ExprId {
    let item_name = match &module.items()[item] {
        Item::Signal(signal) => signal.name.clone(),
        other => panic!("resource companions only name signal items, found {other:?}"),
    };
    module
        .alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                name_path(&[item_name]),
                TermResolution::Item(item),
            )),
        })
        .expect("resource companion item reference should fit in the expression arena")
}

fn hidden_signal_name(candidate: &ResourceSignalCandidate, suffix: &str) -> Name {
    name(
        &format!("{}#{suffix}", candidate.name.text()),
        candidate.span,
    )
}

fn source_option_expr(
    module: &Module,
    source: &SourceDecorator,
    option_name: &str,
) -> Option<(ExprId, SourceSpan)> {
    let options = source.options?;
    let ExprKind::Record(record) = &module.exprs()[options].kind else {
        return None;
    };
    record
        .fields
        .iter()
        .find(|field| field.label.text() == option_name)
        .map(|field| (field.value, field.span))
}

fn signal_source_decorator<'a>(
    module: &'a Module,
    item: &'a SignalItem,
) -> Option<(crate::DecoratorId, &'a SourceDecorator)> {
    item.header.decorators.iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some((*decorator_id, source)),
            _ => None,
        }
    })
}

fn builtin_type_name(builtin: BuiltinType) -> &'static str {
    match builtin {
        BuiltinType::Int => "Int",
        BuiltinType::Float => "Float",
        BuiltinType::Decimal => "Decimal",
        BuiltinType::BigInt => "BigInt",
        BuiltinType::Bool => "Bool",
        BuiltinType::Text => "Text",
        BuiltinType::Unit => "Unit",
        BuiltinType::Bytes => "Bytes",
        BuiltinType::List => "List",
        BuiltinType::Map => "Map",
        BuiltinType::Set => "Set",
        BuiltinType::Option => "Option",
        BuiltinType::Result => "Result",
        BuiltinType::Validation => "Validation",
        BuiltinType::Signal => "Signal",
        BuiltinType::Task => "Task",
    }
}

fn builtin_term_name(builtin: BuiltinTerm) -> &'static str {
    match builtin {
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

fn name(text: &str, span: SourceSpan) -> Name {
    Name::new(text.to_owned(), span).expect("resource companion names should never be empty")
}

fn name_path(names: &[Name]) -> NamePath {
    NamePath::from_vec(names.to_vec())
        .expect("resource companion name paths should stay non-empty and same-file")
}
