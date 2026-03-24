use std::collections::{BTreeMap, BTreeSet};

use aivi_base::SourceSpan;
use aivi_core::{
    self as core,
    expr::{ExprKind, PatternKind, PipeStageKind, ProjectionBase, Reference, TextSegment},
};
use aivi_hir::BindingId;

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
    let mut work = vec![(root, root_scope)];

    while let Some((expr_id, scope)) = work.pop() {
        let expr = &core.exprs()[expr_id];
        match &expr.kind {
            ExprKind::AmbientSubject
            | ExprKind::OptionNone
            | ExprKind::Integer(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::Reference(Reference::Item(_))
            | ExprKind::Reference(Reference::HirItem(_))
            | ExprKind::Reference(Reference::SumConstructor(_))
            | ExprKind::Reference(Reference::DomainMember(_))
            | ExprKind::Reference(Reference::BuiltinClassMember(_))
            | ExprKind::Reference(Reference::Builtin(_)) => {}
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
            ExprKind::OptionSome { payload } => work.push((*payload, scope)),
            ExprKind::Text(text) => {
                for segment in text.segments.iter().rev() {
                    if let TextSegment::Interpolation { expr, .. } = segment {
                        work.push((*expr, scope.clone()));
                    }
                }
            }
            ExprKind::Tuple(elements) | ExprKind::List(elements) | ExprKind::Set(elements) => {
                for child in elements.iter().rev() {
                    work.push((*child, scope.clone()));
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries.iter().rev() {
                    work.push((entry.value, scope.clone()));
                    work.push((entry.key, scope.clone()));
                }
            }
            ExprKind::Record(fields) => {
                for field in fields.iter().rev() {
                    work.push((field.value, scope.clone()));
                }
            }
            ExprKind::Projection { base, .. } => {
                if let ProjectionBase::Expr(base) = base {
                    work.push((*base, scope));
                }
            }
            ExprKind::Apply { callee, arguments } => {
                for argument in arguments.iter().rev() {
                    work.push((*argument, scope.clone()));
                }
                work.push((*callee, scope));
            }
            ExprKind::Unary { expr, .. } => work.push((*expr, scope)),
            ExprKind::Binary { left, right, .. } => {
                work.push((*right, scope.clone()));
                work.push((*left, scope));
            }
            ExprKind::Pipe(pipe) => {
                for stage in pipe.stages.iter().rev() {
                    match &stage.kind {
                        PipeStageKind::Transform { expr }
                        | PipeStageKind::Tap { expr }
                        | PipeStageKind::Gate {
                            predicate: expr, ..
                        } => work.push((*expr, scope.clone())),
                        PipeStageKind::Case { arms } => {
                            for arm in arms.iter().rev() {
                                let mut arm_scope = scope.clone();
                                extend_scope_with_pattern(&mut arm_scope, &arm.pattern);
                                work.push((arm.body, arm_scope));
                            }
                        }
                        PipeStageKind::TruthyFalsy(pair) => {
                            work.push((pair.falsy.body, scope.clone()));
                            work.push((pair.truthy.body, scope.clone()));
                        }
                    }
                }
                work.push((pipe.head, scope));
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
