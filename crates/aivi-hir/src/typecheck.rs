use std::collections::{HashMap, HashSet};

use aivi_base::{Diagnostic, DiagnosticCode, SourceSpan};

use crate::{
    hir::{
        BuiltinTerm, BuiltinType, ExprKind, FunctionItem, ImportBindingMetadata,
        ImportBundleKind, Item, Module, RecordExpr, SignalItem, TermReference, TermResolution,
        TypeItemBody, ValueItem,
    },
    ids::{ExprId, ItemId, TypeParameterId},
    validate::{
        DomainMemberSelection, GateExprEnv, GateIssue, GateRecordField, GateType, GateTypeContext,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintClass {
    Eq,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeConstraint {
    span: SourceSpan,
    class: ConstraintClass,
    subject: GateType,
}

impl TypeConstraint {
    pub(crate) fn eq(span: SourceSpan, subject: GateType) -> Self {
        Self {
            span,
            class: ConstraintClass::Eq,
            subject,
        }
    }

    pub fn span(&self) -> SourceSpan {
        self.span
    }

    pub fn class(&self) -> &ConstraintClass {
        &self.class
    }

    pub fn subject(&self) -> &GateType {
        &self.subject
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeCheckReport {
    diagnostics: Vec<Diagnostic>,
}

impl TypeCheckReport {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

pub fn typecheck_module(module: &Module) -> TypeCheckReport {
    let mut checker = TypeChecker::new(module);
    checker.run();
    TypeCheckReport::new(checker.diagnostics)
}

pub(crate) fn expression_matches(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    expected: &GateType,
) -> bool {
    let mut checker = TypeChecker::new(module);
    checker.check_expr(expr_id, env, Some(expected), &mut Vec::new()) && checker.diagnostics.is_empty()
}

struct TypeChecker<'a> {
    module: &'a Module,
    typing: GateTypeContext<'a>,
    diagnostics: Vec<Diagnostic>,
    option_default_in_scope: bool,
}

impl<'a> TypeChecker<'a> {
    fn new(module: &'a Module) -> Self {
        let option_default_in_scope = module.imports().iter().any(|(_, import)| {
            matches!(
                import.metadata,
                ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
            )
        });
        Self {
            module,
            typing: GateTypeContext::new(module),
            diagnostics: Vec::new(),
            option_default_in_scope,
        }
    }

    fn run(&mut self) {
        let items = self
            .module
            .items()
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        for item in items {
            match item {
                Item::Value(item) => self.check_value_item(&item),
                Item::Function(item) => self.check_function_item(&item),
                Item::Signal(item) => self.check_signal_item(&item),
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }
    }

    fn check_value_item(&mut self, item: &ValueItem) {
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        self.check_expr(item.body, &GateExprEnv::default(), expected.as_ref(), &mut Vec::new());
    }

    fn check_function_item(&mut self, item: &FunctionItem) {
        let mut env = GateExprEnv::default();
        for parameter in &item.parameters {
            let Some(annotation) = parameter.annotation else {
                continue;
            };
            let Some(parameter_ty) = self.typing.lower_annotation(annotation) else {
                continue;
            };
            env.locals.insert(parameter.binding, parameter_ty);
        }
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        self.check_expr(item.body, &env, expected.as_ref(), &mut Vec::new());
    }

    fn check_signal_item(&mut self, item: &SignalItem) {
        let Some(body) = item.body else {
            return;
        };
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        match expected.as_ref() {
            Some(annotation @ GateType::Signal(payload)) => {
                let checkpoint = self.diagnostics.len();
                if self.check_expr(
                    body,
                    &GateExprEnv::default(),
                    Some(annotation),
                    &mut Vec::new(),
                ) {
                    return;
                }
                self.diagnostics.truncate(checkpoint);
                self.check_expr(
                    body,
                    &GateExprEnv::default(),
                    Some(payload.as_ref()),
                    &mut Vec::new(),
                );
            }
            Some(annotation) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "signal bodies require a `Signal A` annotation, found `{annotation}`"
                    ))
                    .with_code(code("invalid-signal-annotation"))
                    .with_primary_label(
                        self.module.exprs()[body].span,
                        "this signal body is checked against the payload type of `Signal A`",
                    ),
                );
            }
            None => {
                self.check_expr(body, &GateExprEnv::default(), None, &mut Vec::new());
            }
        }
    }

    fn check_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(expected) = expected {
            if let Some(result) =
                self.check_expected_special_case(expr_id, env, expected, value_stack)
            {
                return result;
            }
        }

        let info = self.typing.infer_expr(expr_id, env, None);
        self.emit_expr_issues(&info.issues);
        self.solve_constraints(&info.constraints);

        match (expected, info.ty.as_ref()) {
            (Some(expected), Some(actual)) if actual.same_shape(expected) => true,
            (Some(expected), Some(actual)) => {
                self.emit_type_mismatch(self.module.exprs()[expr_id].span, expected, actual);
                false
            }
            (Some(_), None) => false,
            (None, Some(_)) | (None, None) => true,
        }
    }

    fn check_expected_special_case(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let kind = self.module.exprs()[expr_id].kind.clone();
        match kind {
            ExprKind::Name(reference) => self
                .check_builtin_constructor_name(&reference, expected)
                .or_else(|| self.check_domain_member_name(&reference, expected))
                .or_else(|| self.check_unannotated_value_name(&reference, env, expected, value_stack)),
            ExprKind::Apply { callee, arguments } => {
                let callee_kind = self.module.exprs()[callee].kind.clone();
                if let ExprKind::Name(reference) = callee_kind {
                    if let Some(result) = self.check_builtin_constructor_apply(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                    if let Some(result) = self.check_domain_member_apply(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                }
                self.check_expected_apply(
                    expr_id,
                    callee,
                    &arguments,
                    env,
                    expected,
                    value_stack,
                )
            }
            ExprKind::Record(record) => match expected {
                GateType::Record(fields) => Some(self.check_record_expr(
                    self.module.exprs()[expr_id].span,
                    &record,
                    fields,
                    env,
                    value_stack,
                )),
                _ => None,
            },
            ExprKind::Integer(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::Text(_)
            | ExprKind::Regex(_)
            | ExprKind::Tuple(_)
            | ExprKind::List(_)
            | ExprKind::Map(_)
            | ExprKind::Set(_)
            | ExprKind::Projection { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Pipe(_)
            | ExprKind::Cluster(_)
            | ExprKind::Markup(_) => None,
        }
    }

    fn check_unannotated_value_name(
        &mut self,
        reference: &TermReference,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Item(item_id)) = reference.resolution.as_ref() else {
            return None;
        };
        let (body, annotated) = match &self.module.items()[*item_id] {
            Item::Value(item) => (item.body, item.annotation.is_some()),
            _ => return None,
        };
        if annotated || value_stack.contains(item_id) {
            return None;
        }
        value_stack.push(*item_id);
        let result = self.check_expr(body, env, Some(expected), value_stack);
        let popped = value_stack.pop();
        debug_assert_eq!(popped, Some(*item_id));
        Some(result)
    }

    fn check_builtin_constructor_name(
        &self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) = reference.resolution.as_ref() else {
            return None;
        };
        match (builtin, expected) {
            (BuiltinTerm::None, GateType::Option(_)) => Some(true),
            _ => None,
        }
    }

    fn check_builtin_constructor_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) = reference.resolution.as_ref() else {
            return None;
        };
        if arguments.len() != 1 {
            return None;
        }
        let argument = *arguments.first();
        match (builtin, expected) {
            (BuiltinTerm::Some, GateType::Option(payload)) => {
                Some(self.check_expr(argument, env, Some(payload.as_ref()), value_stack))
            }
            (BuiltinTerm::Ok, GateType::Result { value, .. }) => {
                Some(self.check_expr(argument, env, Some(value.as_ref()), value_stack))
            }
            (BuiltinTerm::Err, GateType::Result { error, .. }) => {
                Some(self.check_expr(argument, env, Some(error.as_ref()), value_stack))
            }
            (BuiltinTerm::Valid, GateType::Validation { value, .. }) => {
                Some(self.check_expr(argument, env, Some(value.as_ref()), value_stack))
            }
            (BuiltinTerm::Invalid, GateType::Validation { error, .. }) => {
                Some(self.check_expr(argument, env, Some(error.as_ref()), value_stack))
            }
            _ => None,
        }
    }

    fn check_domain_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<bool> {
        let labels = self.typing.domain_member_candidate_labels(reference)?;
        match self.typing.select_domain_member_name(reference, expected)? {
            DomainMemberSelection::Unique(_) => Some(true),
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_domain_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let labels = self.typing.domain_member_candidate_labels(reference)?;
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let info = self.typing.infer_expr(*argument, env, None);
            self.emit_expr_issues(&info.issues);
            self.solve_constraints(&info.constraints);
            let Some(argument_ty) = info.ty else {
                return None;
            };
            argument_types.push(argument_ty);
        }
        match self
            .typing
            .select_domain_member_call(reference, &argument_types, Some(expected))?
        {
            DomainMemberSelection::Unique(matched) => {
                for (argument, parameter) in arguments.iter().zip(matched.parameters.iter()) {
                    if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                        return Some(false);
                    }
                }
                Some(true)
            }
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_expected_apply(
        &mut self,
        expr_id: ExprId,
        callee: ExprId,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let checkpoint = self.diagnostics.len();
        let callee_info = self.typing.infer_expr(callee, env, None);
        self.emit_expr_issues(&callee_info.issues);
        self.solve_constraints(&callee_info.constraints);
        if self.diagnostics.len() != checkpoint {
            return Some(false);
        }

        let mut current = callee_info.ty?;
        let mut parameter_types = Vec::with_capacity(arguments.len());
        for _ in arguments.iter() {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameter_types.push(*parameter);
            current = *result;
        }

        if !current.same_shape(expected) {
            self.emit_type_mismatch(self.module.exprs()[expr_id].span, expected, &current);
            return Some(false);
        }

        for (argument, parameter) in arguments.iter().zip(parameter_types.iter()) {
            if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                return Some(false);
            }
        }

        Some(true)
    }

    fn check_record_expr(
        &mut self,
        expr_span: SourceSpan,
        record: &RecordExpr,
        expected_fields: &[GateRecordField],
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let expected = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), &field.ty))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::<String>::new();
        let mut ok = true;

        for field in &record.fields {
            let label = field.label.text();
            let Some(expected_ty) = expected.get(label) else {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "record literal provides unexpected field `{label}`"
                    ))
                    .with_code(code("unexpected-record-field"))
                    .with_primary_label(
                        field.span,
                        "this field is not part of the expected closed record type",
                    ),
                );
                ok = false;
                continue;
            };
            seen.insert(label.to_owned());
            ok &= self.check_expr(field.value, env, Some(*expected_ty), value_stack);
        }

        for field in expected_fields {
            if seen.contains(&field.name) {
                continue;
            }
            if self.has_default_instance(&field.ty) {
                continue;
            }
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "record literal omits field `{}` but no `Default` instance is in scope for `{}`",
                    field.name, field.ty
                ))
                .with_code(code("missing-default-instance"))
                .with_primary_label(
                    expr_span,
                    format!("field `{}` must be provided or defaultable here", field.name),
                ),
            );
            ok = false;
        }

        ok
    }

    fn has_default_instance(&self, ty: &GateType) -> bool {
        matches!(ty, GateType::Option(_)) && self.option_default_in_scope
    }

    fn emit_expr_issues(&mut self, issues: &[GateIssue]) {
        for issue in issues {
            let diagnostic = match issue {
                GateIssue::InvalidProjection {
                    span,
                    path,
                    subject,
                } => Diagnostic::error(format!(
                    "projection `{path}` cannot be applied to `{subject}`"
                ))
                .with_code(code("invalid-projection"))
                .with_primary_label(*span, "this projection target does not support field access"),
                GateIssue::UnknownField {
                    span,
                    path,
                    subject,
                } => Diagnostic::error(format!(
                    "projection `{path}` is not available on `{subject}`"
                ))
                .with_code(code("unknown-projection-field"))
                .with_primary_label(*span, "this projection refers to a missing record field"),
                GateIssue::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                } => Diagnostic::error(format!(
                    "domain member `{name}` is ambiguous in this context"
                ))
                .with_code(code("ambiguous-domain-member"))
                .with_primary_label(
                    *span,
                    "add more type context or rename/import an alias for the desired member",
                )
                .with_note(format!("candidates: {}", candidates.join(", "))),
            };
            self.diagnostics.push(diagnostic);
        }
    }

    fn emit_ambiguous_domain_member(
        &mut self,
        span: SourceSpan,
        reference: &TermReference,
        candidates: &[String],
    ) {
        let name = reference
            .path
            .segments()
            .last()
            .text()
            .to_owned();
        self.diagnostics.push(
            Diagnostic::error(format!("domain member `{name}` is ambiguous in this context"))
                .with_code(code("ambiguous-domain-member"))
                .with_primary_label(
                    span,
                    "add more type context or rename/import an alias for the desired member",
                )
                .with_note(format!("candidates: {}", candidates.join(", "))),
        );
    }

    fn solve_constraints(&mut self, constraints: &[TypeConstraint]) {
        for constraint in constraints {
            match constraint.class() {
                ConstraintClass::Eq => {
                    if let Err(reason) = self.require_eq(constraint.subject(), &mut Vec::new()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "this expression requires `Eq` for `{}`",
                                constraint.subject()
                            ))
                            .with_code(code("missing-eq-instance"))
                            .with_primary_label(
                                constraint.span(),
                                format!(
                                    "`{}` does not currently have `Eq` evidence",
                                    constraint.subject()
                                ),
                            )
                            .with_note(reason),
                        );
                    }
                }
            }
        }
    }

    fn require_eq(&mut self, ty: &GateType, item_stack: &mut Vec<ItemId>) -> Result<(), String> {
        match ty {
            GateType::Primitive(BuiltinType::Bytes) => Err(
                "`Bytes` does not have a compiler-derived `Eq` instance in v1".to_owned(),
            ),
            GateType::Primitive(_) => Ok(()),
            GateType::Tuple(elements) => {
                for element in elements {
                    self.require_eq(element, item_stack)?;
                }
                Ok(())
            }
            GateType::Record(fields) => {
                for field in fields {
                    self.require_eq(&field.ty, item_stack)?;
                }
                Ok(())
            }
            GateType::List(element) | GateType::Option(element) => {
                self.require_eq(element, item_stack)
            }
            GateType::Result { error, value } | GateType::Validation { error, value } => {
                self.require_eq(error, item_stack)?;
                self.require_eq(value, item_stack)
            }
            GateType::Domain {
                item, arguments, ..
            } => {
                if item_stack.contains(item) {
                    return Ok(());
                }
                let (parameters, carrier) = match &self.module.items()[*item] {
                    Item::Domain(domain) => (domain.parameters.clone(), domain.carrier),
                    _ => return Err(format!("`{ty}` does not refer to a domain declaration")),
                };
                let substitutions = parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<TypeParameterId, GateType>>();
                let Some(carrier) = self.typing.lower_hir_type(carrier, &substitutions) else {
                    return Err(format!(
                        "the carrier type for `{ty}` could not be lowered for Eq checking"
                    ));
                };
                item_stack.push(*item);
                let result = self.require_eq(&carrier, item_stack);
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(*item));
                result
            }
            GateType::OpaqueItem {
                item, arguments, ..
            } => {
                if item_stack.contains(item) {
                    return Ok(());
                }
                let (parameters, body) = match &self.module.items()[*item] {
                    Item::Type(item_ty) => (item_ty.parameters.clone(), item_ty.body.clone()),
                    _ => return Err(format!("`{ty}` does not refer to a type declaration")),
                };
                let substitutions = parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<TypeParameterId, GateType>>();
                item_stack.push(*item);
                let result = match body {
                    TypeItemBody::Alias(alias) => {
                        let Some(lowered) = self.typing.lower_hir_type(alias, &substitutions) else {
                            return Err(format!(
                                "the alias body for `{ty}` could not be lowered for Eq checking"
                            ));
                        };
                        self.require_eq(&lowered, item_stack)
                    }
                    TypeItemBody::Sum(variants) => {
                        for variant in variants.iter() {
                            for field in &variant.fields {
                                let Some(lowered) = self.typing.lower_hir_type(*field, &substitutions) else {
                                    return Err(format!(
                                        "constructor payloads for `{ty}` could not be lowered for Eq checking"
                                    ));
                                };
                                self.require_eq(&lowered, item_stack)?;
                            }
                        }
                        Ok(())
                    }
                };
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(*item));
                result
            }
            GateType::Arrow { .. }
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. } => Err(format!(
                "`{ty}` does not have a compiler-derived `Eq` instance in v1"
            )),
        }
    }

    fn emit_type_mismatch(&mut self, span: SourceSpan, expected: &GateType, actual: &GateType) {
        self.diagnostics.push(
            Diagnostic::error(format!("expected `{expected}` but found `{actual}`"))
                .with_code(code("type-mismatch"))
                .with_primary_label(span, "this expression has the wrong type"),
        );
    }
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

#[cfg(test)]
mod tests {
    use aivi_base::{DiagnosticCode, SourceDatabase};
    use aivi_syntax::parse_module;

    use crate::lower_module;

    use super::*;

    fn typecheck_text(path: &str, text: &str) -> TypeCheckReport {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "typecheck input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "typecheck input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        typecheck_module(lowered.module())
    }

    #[test]
    fn typecheck_allows_option_default_record_elision() {
        let report = typecheck_text(
            "record-elision.aivi",
            "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             val name = \"Ada\"\n\
             val nickname = Some \"Countess\"\n\
             val profile:Profile = { name, nickname }\n",
        );
        assert!(
            report.is_ok(),
            "expected defaulted record elision to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_missing_eq_for_map_equality() {
        let report = typecheck_text(
            "map-equality.aivi",
            "val left = Map { \"id\": 1 }\n\
             val right = Map { \"id\": 1 }\n\
             val same:Bool = left == right\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "missing-eq-instance"))
            }),
            "expected missing Eq diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_value_annotation_mismatch() {
        let report = typecheck_text("value-mismatch.aivi", "val answer:Text = 42\n");
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected type mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_function_application_with_expected_builtin_hole_argument() {
        let report = typecheck_text(
            "function-application-expected-hole.aivi",
            "fun keep:Option Int #value:Option Int => value\n\
             val result:Option Int = keep None\n",
        );
        assert!(
            report.is_ok(),
            "expected keep None to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_function_application_result_mismatch() {
        let report = typecheck_text(
            "function-application-result-mismatch.aivi",
            "fun keep:Option Int #value:Option Int => value\n\
             val result:Option Text = keep None\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected type mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}
