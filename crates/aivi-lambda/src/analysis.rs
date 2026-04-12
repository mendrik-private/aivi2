use std::collections::{BTreeMap, BTreeSet};

use aivi_base::SourceSpan;
use aivi_core::{
    self as core,
    expr::{ExprKind, PatternKind, PipeStageKind, ProjectionBase, Reference, TextSegment},
};
use aivi_hir::BindingId;
use aivi_typing::StructuralWalker;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapturedBinding {
    pub binding: BindingId,
    pub name: Option<Box<str>>,
    pub ty: core::Type,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnalysisError {
    BindingTypeConflict {
        binding: BindingId,
        previous: core::Type,
        current: core::Type,
        span: SourceSpan,
    },
}

pub(crate) fn capture_free_bindings(
    core: &core::Module,
    root: core::ExprId,
    initial_locals: &[BindingId],
    known_names: &BTreeMap<BindingId, Box<str>>,
) -> Result<Vec<CapturedBinding>, AnalysisError> {
    let mut captures = BTreeMap::<BindingId, CapturedBinding>::new();
    let mut root_scope = BTreeSet::new();
    root_scope.extend(initial_locals.iter().copied());
    let mut walker = StructuralWalker::<_, ()>::new((root, root_scope));

    while let Some((expr_id, scope)) = walker.next_frame() {
        let expr = &core.exprs()[expr_id];
        match &expr.kind {
            ExprKind::AmbientSubject
            | ExprKind::OptionNone
            | ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            // NOTE: If this closure's body contains a reference to the item that owns it
            // (i.e., the item calls itself), that reference appears here as a captured
            // variable rather than a self-reference. This means truly self-recursive
            // closures are not representable: the closure would appear to capture itself,
            // which is unsound. Self-recursive functions must be expressed as non-closure
            // items. This limitation is intentional for the current IR design but is not
            // validated — a self-recursive closure will produce incorrect capture metadata.
            | ExprKind::Reference(Reference::Item(_))
            | ExprKind::Reference(Reference::HirItem(_))
            | ExprKind::Reference(Reference::SumConstructor(_))
            | ExprKind::Reference(Reference::DomainMember(_))
            | ExprKind::Reference(Reference::ExecutableEvidence(_))
            | ExprKind::Reference(Reference::Builtin(_))
            | ExprKind::Reference(Reference::IntrinsicValue(_)) => {}
            ExprKind::Reference(Reference::Local(binding)) => {
                if scope.contains(binding) {
                    continue;
                }
                let expr_ty = expr.ty.clone();
                match captures.get_mut(binding) {
                    Some(existing) if existing.ty != expr_ty => {
                        return Err(AnalysisError::BindingTypeConflict {
                            binding: *binding,
                            previous: existing.ty.clone(),
                            current: expr_ty,
                            span: expr.span,
                        });
                    }
                    Some(existing) => {
                        if existing.name.is_none() {
                            existing.name = known_names.get(binding).cloned();
                        }
                    }
                    None => {
                        captures.insert(
                            *binding,
                            CapturedBinding {
                                binding: *binding,
                                name: known_names.get(binding).cloned(),
                                ty: expr_ty,
                                span: expr.span,
                            },
                        );
                    }
                }
            }
            ExprKind::OptionSome { payload } => walker.push_frame((*payload, scope)),
            ExprKind::Text(text) => {
                for segment in text.segments.iter().rev() {
                    if let TextSegment::Interpolation { expr, .. } = segment {
                        walker.push_frame((*expr, scope.clone()));
                    }
                }
            }
            ExprKind::Tuple(elements) | ExprKind::List(elements) | ExprKind::Set(elements) => {
                for child in elements.iter().rev() {
                    walker.push_frame((*child, scope.clone()));
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries.iter().rev() {
                    walker.push_frame((entry.value, scope.clone()));
                    walker.push_frame((entry.key, scope.clone()));
                }
            }
            ExprKind::Record(fields) => {
                for field in fields.iter().rev() {
                    walker.push_frame((field.value, scope.clone()));
                }
            }
            ExprKind::Projection { base, .. } => {
                if let ProjectionBase::Expr(base) = base {
                    walker.push_frame((*base, scope));
                }
            }
            ExprKind::Apply { callee, arguments } => {
                for argument in arguments.iter().rev() {
                    walker.push_frame((*argument, scope.clone()));
                }
                walker.push_frame((*callee, scope));
            }
            ExprKind::Unary { expr, .. } => walker.push_frame((*expr, scope)),
            ExprKind::Binary { left, right, .. } => {
                walker.push_frame((*right, scope.clone()));
                walker.push_frame((*left, scope));
            }
            ExprKind::Pipe(pipe) => {
                let mut pipe_scope = scope.clone();
                let mut stage_frames = Vec::new();
                for stage in &pipe.stages {
                    let mut stage_scope = pipe_scope.clone();
                    if stage.supports_memos()
                        && let Some(binding) = stage.subject_memo
                    {
                        stage_scope.insert(binding);
                    }
                    match &stage.kind {
                        PipeStageKind::Transform { expr, .. }
                        | PipeStageKind::Tap { expr }
                        | PipeStageKind::Gate {
                            predicate: expr, ..
                        }
                        | PipeStageKind::FanOut { map_expr: expr } => {
                            stage_frames.push((*expr, stage_scope.clone()))
                        }
                        PipeStageKind::Debug { .. } => {}
                        PipeStageKind::Case { arms } => {
                            for arm in arms {
                                let mut arm_scope = stage_scope.clone();
                                extend_scope_with_pattern(&mut arm_scope, &arm.pattern);
                                stage_frames.push((arm.body, arm_scope));
                            }
                        }
                        PipeStageKind::TruthyFalsy(pair) => {
                            stage_frames.push((pair.truthy.body, stage_scope.clone()));
                            stage_frames.push((pair.falsy.body, stage_scope));
                        }
                    }
                    if stage.supports_memos() {
                        if let Some(binding) = stage.subject_memo {
                            pipe_scope.insert(binding);
                        }
                        if let Some(binding) = stage.result_memo {
                            pipe_scope.insert(binding);
                        }
                    }
                }
                for (stage_expr, stage_scope) in stage_frames.into_iter().rev() {
                    walker.push_frame((stage_expr, stage_scope));
                }
                walker.push_frame((pipe.head, scope));
            }
        }
    }

    Ok(captures.into_values().collect())
}

fn extend_scope_with_pattern(scope: &mut BTreeSet<BindingId>, pattern: &core::Pattern) {
    let mut work = vec![pattern];
    while let Some(pattern) = work.pop() {
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Integer(_) | PatternKind::Text(_) => {}
            PatternKind::Binding(binding) => {
                scope.insert(binding.binding);
            }
            PatternKind::Tuple(elements) => {
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            PatternKind::List { elements, rest } => {
                if let Some(rest) = rest {
                    work.push(rest);
                }
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            PatternKind::Record(fields) => {
                for field in fields.iter().rev() {
                    work.push(&field.pattern);
                }
            }
            PatternKind::Constructor { arguments, .. } => {
                for argument in arguments.iter().rev() {
                    work.push(argument);
                }
            }
        }
    }
}
