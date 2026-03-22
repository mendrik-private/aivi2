use std::{collections::HashMap, fmt};

use aivi_base::{Diagnostic, DiagnosticCode, DiagnosticLabel, SourceSpan};
use aivi_typing::{
    BuiltinSourceProvider, GateCarrier, GatePlanner, GateResultKind, Kind, KindCheckError,
    KindCheckErrorKind, KindChecker, KindExprId, KindParameterId as TypingKindParameterId,
    KindRecordField, KindStore, RecurrencePlanner, RecurrenceTargetEvidence,
    RecurrenceWakeupPlanner, SourceContractType, SourceRecurrenceWakeupContext,
};

use crate::{
    arena::{Arena, ArenaId},
    hir::{
        ApplicativeSpineHead, BuiltinType, ControlNode, ControlNodeKind, DecoratorPayload,
        DomainMemberKind, ExprKind, Item, LiteralSuffixResolution, MarkupAttributeValue,
        MarkupNodeKind, Module, Name, NamePath, PatternKind, PipeStageKind, ResolutionState,
        SignalItem, SourceDecorator, SourceMetadata, TermReference, TermResolution, TextLiteral,
        TextSegment, TypeKind, TypeReference, TypeResolution,
    },
    ids::{
        BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
        PatternId, TypeId, TypeParameterId,
    },
    source_contract_resolution::{SourceContractResolutionErrorKind, SourceContractTypeResolver},
};

/// Validation strictness for HIR modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Structural,
    RequireResolvedNames,
}

/// Aggregated HIR validation result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationReport {
    diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
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

pub fn validate_module(module: &Module, mode: ValidationMode) -> ValidationReport {
    let mut validator = Validator {
        module,
        mode,
        diagnostics: Vec::new(),
    };
    validator.run();
    ValidationReport::new(validator.diagnostics)
}

struct Validator<'a> {
    module: &'a Module,
    mode: ValidationMode,
    diagnostics: Vec<Diagnostic>,
}

impl Validator<'_> {
    fn run(&mut self) {
        self.validate_roots();
        self.validate_bindings();
        self.validate_type_parameters();
        self.validate_imports();
        self.validate_decorators();
        self.validate_types();
        self.validate_patterns();
        self.validate_exprs();
        self.validate_markup_nodes();
        self.validate_control_nodes();
        self.validate_clusters();
        self.validate_items();
        self.validate_type_kinds();
        self.validate_source_contract_types();
        self.validate_gate_semantics();
        self.validate_recurrence_targets();
    }

    fn validate_roots(&mut self) {
        for item in &self.module.root_items {
            self.require_item(SourceSpan::default(), "module root", "item", *item);
        }
    }

    fn validate_bindings(&mut self) {
        for (_, binding) in self.module.bindings().iter() {
            self.check_span("binding", binding.span);
            self.check_name(&binding.name);
        }
    }

    fn validate_type_parameters(&mut self) {
        for (_, parameter) in self.module.type_parameters().iter() {
            self.check_span("type parameter", parameter.span);
            self.check_name(&parameter.name);
        }
    }

    fn validate_imports(&mut self) {
        for (_, import) in self.module.imports().iter() {
            self.check_span("import binding", import.span);
            self.check_name(&import.imported_name);
            self.check_name(&import.local_name);
        }
    }

    fn validate_decorators(&mut self) {
        for (_, decorator) in self.module.decorators().iter() {
            self.check_span("decorator", decorator.span);
            self.check_name_path(&decorator.name);
            match &decorator.payload {
                DecoratorPayload::Bare => {}
                DecoratorPayload::Call(call) => {
                    for argument in &call.arguments {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "argument expression",
                            *argument,
                        );
                    }
                    if let Some(options) = call.options {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "options expression",
                            options,
                        );
                    }
                }
                DecoratorPayload::Source(source) => {
                    if let Some(provider) = &source.provider {
                        self.check_name_path(provider);
                    }
                    for argument in &source.arguments {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "source argument",
                            *argument,
                        );
                    }
                    if let Some(options) = source.options {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "source options expression",
                            options,
                        );
                    }
                }
            }
        }
    }

    fn validate_types(&mut self) {
        for (_, ty) in self.module.types().iter() {
            self.check_span("type node", ty.span);
            match &ty.kind {
                TypeKind::Name(reference) => self.check_type_reference(reference),
                TypeKind::Tuple(elements) => {
                    for element in elements.iter() {
                        self.require_type(ty.span, "type node", "tuple element type", *element);
                    }
                }
                TypeKind::Record(fields) => {
                    for field in fields {
                        self.check_span("type field", field.span);
                        self.check_name(&field.label);
                        self.require_type(field.span, "type field", "field type", field.ty);
                    }
                }
                TypeKind::Arrow { parameter, result } => {
                    self.require_type(ty.span, "type node", "parameter type", *parameter);
                    self.require_type(ty.span, "type node", "result type", *result);
                }
                TypeKind::Apply { callee, arguments } => {
                    self.require_type(ty.span, "type node", "type callee", *callee);
                    for argument in arguments.iter() {
                        self.require_type(ty.span, "type node", "type argument", *argument);
                    }
                }
            }
        }
    }

    fn validate_patterns(&mut self) {
        for (_, pattern) in self.module.patterns().iter() {
            self.check_span("pattern", pattern.span);
            match &pattern.kind {
                PatternKind::Wildcard | PatternKind::Integer(_) => {}
                PatternKind::Text(text) => self.check_text_literal(pattern.span, text),
                PatternKind::Binding(binding) => {
                    self.check_name(&binding.name);
                    self.require_binding(pattern.span, "pattern", "binding", binding.binding);
                }
                PatternKind::Tuple(elements) => {
                    for element in elements.iter() {
                        self.require_pattern(
                            pattern.span,
                            "pattern",
                            "tuple element pattern",
                            *element,
                        );
                    }
                }
                PatternKind::Record(fields) => {
                    for field in fields {
                        self.check_span("record pattern field", field.span);
                        self.check_name(&field.label);
                        self.require_pattern(
                            field.span,
                            "record pattern field",
                            "field pattern",
                            field.pattern,
                        );
                    }
                }
                PatternKind::Constructor { callee, arguments } => {
                    self.check_term_reference(callee);
                    for argument in arguments {
                        self.require_pattern(
                            pattern.span,
                            "pattern",
                            "constructor argument",
                            *argument,
                        );
                    }
                }
                PatternKind::UnresolvedName(reference) => {
                    self.check_term_reference(reference);
                }
            }
        }
    }

    fn validate_exprs(&mut self) {
        for (_, expr) in self.module.exprs().iter() {
            self.check_span("expression", expr.span);
            match &expr.kind {
                ExprKind::Name(reference) => self.check_term_reference(reference),
                ExprKind::Integer(_) | ExprKind::Regex(_) => {}
                ExprKind::Text(text) => self.check_text_literal(expr.span, text),
                ExprKind::SuffixedInteger(literal) => self.check_suffixed_integer(literal),
                ExprKind::Tuple(elements) => {
                    for element in elements.iter() {
                        self.require_expr(expr.span, "expression", "tuple element", *element);
                    }
                }
                ExprKind::List(elements) => {
                    for element in elements {
                        self.require_expr(expr.span, "expression", "list element", *element);
                    }
                }
                ExprKind::Record(record) => {
                    for field in &record.fields {
                        self.check_span("record field", field.span);
                        self.check_name(&field.label);
                        self.require_expr(field.span, "record field", "field value", field.value);
                    }
                }
                ExprKind::Projection { base, path } => {
                    if let crate::hir::ProjectionBase::Expr(base) = base {
                        self.require_expr(expr.span, "expression", "projection base", *base);
                    }
                    self.check_name_path(path);
                }
                ExprKind::Apply { callee, arguments } => {
                    self.require_expr(expr.span, "expression", "application callee", *callee);
                    for argument in arguments.iter() {
                        self.require_expr(
                            expr.span,
                            "expression",
                            "application argument",
                            *argument,
                        );
                    }
                }
                ExprKind::Unary { expr: inner, .. } => {
                    self.require_expr(expr.span, "expression", "unary operand", *inner);
                }
                ExprKind::Binary { left, right, .. } => {
                    self.require_expr(expr.span, "expression", "binary left operand", *left);
                    self.require_expr(expr.span, "expression", "binary right operand", *right);
                }
                ExprKind::Pipe(pipe) => {
                    self.require_expr(expr.span, "expression", "pipe head", pipe.head);
                    for stage in pipe.stages.iter() {
                        self.check_span("pipe stage", stage.span);
                        match &stage.kind {
                            PipeStageKind::Transform { expr }
                            | PipeStageKind::Gate { expr }
                            | PipeStageKind::Map { expr }
                            | PipeStageKind::Apply { expr }
                            | PipeStageKind::Tap { expr }
                            | PipeStageKind::FanIn { expr }
                            | PipeStageKind::Truthy { expr }
                            | PipeStageKind::Falsy { expr }
                            | PipeStageKind::RecurStart { expr }
                            | PipeStageKind::RecurStep { expr } => {
                                self.require_expr(
                                    stage.span,
                                    "pipe stage",
                                    "stage expression",
                                    *expr,
                                );
                            }
                            PipeStageKind::Case { pattern, body } => {
                                self.require_pattern(
                                    stage.span,
                                    "pipe stage",
                                    "case pattern",
                                    *pattern,
                                );
                                self.require_expr(stage.span, "pipe stage", "case body", *body);
                            }
                        }
                    }
                }
                ExprKind::Cluster(cluster) => {
                    self.require_cluster(expr.span, "expression", "cluster", *cluster);
                }
                ExprKind::Markup(node) => {
                    self.require_markup_node(expr.span, "expression", "markup node", *node);
                }
            }
        }
    }

    fn validate_markup_nodes(&mut self) {
        for (_, node) in self.module.markup_nodes().iter() {
            self.check_span("markup node", node.span);
            match &node.kind {
                MarkupNodeKind::Element(element) => {
                    self.check_name_path(&element.name);
                    if let Some(close_name) = &element.close_name {
                        self.check_name_path(close_name);
                    }
                    for attribute in &element.attributes {
                        self.check_span("markup attribute", attribute.span);
                        self.check_name(&attribute.name);
                        match &attribute.value {
                            MarkupAttributeValue::Expr(expr) => self.require_expr(
                                attribute.span,
                                "markup attribute",
                                "attribute expression",
                                *expr,
                            ),
                            MarkupAttributeValue::Text(text) => {
                                self.check_text_literal(attribute.span, text)
                            }
                            MarkupAttributeValue::ImplicitTrue => {}
                        }
                    }
                    for child in &element.children {
                        self.require_markup_node(node.span, "markup node", "child node", *child);
                    }
                }
                MarkupNodeKind::Control(control) => {
                    self.require_control_node(node.span, "markup node", "control node", *control);
                    if let Some(control) = self.module.control_nodes().get(*control) {
                        if matches!(
                            control.kind(),
                            ControlNodeKind::Empty | ControlNodeKind::Case
                        ) {
                            self.illegal_direct_control(node.span, control.kind());
                        }
                    }
                }
            }
        }
    }

    fn validate_control_nodes(&mut self) {
        for (_, node) in self.module.control_nodes().iter() {
            self.check_span("control node", node.span());
            match node {
                ControlNode::Show(show) => {
                    self.require_expr(show.span, "control node", "show condition", show.when);
                    if let Some(keep_mounted) = show.keep_mounted {
                        self.require_expr(
                            show.span,
                            "control node",
                            "keepMounted expression",
                            keep_mounted,
                        );
                    }
                    for child in &show.children {
                        self.require_markup_node(show.span, "control node", "show child", *child);
                    }
                }
                ControlNode::Each(each) => {
                    self.require_expr(
                        each.span,
                        "control node",
                        "each collection",
                        each.collection,
                    );
                    self.require_binding(each.span, "control node", "each binding", each.binding);
                    if let Some(key) = each.key {
                        self.require_expr(each.span, "control node", "each key", key);
                    }
                    for child in &each.children {
                        self.require_markup_node(each.span, "control node", "each child", *child);
                    }
                    if let Some(empty) = each.empty {
                        self.require_control_node(each.span, "control node", "empty branch", empty);
                        if let Some(node) = self.module.control_nodes().get(empty) {
                            if node.kind() != ControlNodeKind::Empty {
                                self.wrong_control_kind(
                                    each.span,
                                    "each empty branch",
                                    ControlNodeKind::Empty,
                                    node.kind(),
                                );
                            }
                        }
                    }
                }
                ControlNode::Empty(empty) => {
                    for child in &empty.children {
                        self.require_markup_node(empty.span, "control node", "empty child", *child);
                    }
                }
                ControlNode::Match(match_node) => {
                    self.require_expr(
                        match_node.span,
                        "control node",
                        "match scrutinee",
                        match_node.scrutinee,
                    );
                    for case in match_node.cases.iter() {
                        self.require_control_node(
                            match_node.span,
                            "control node",
                            "match case",
                            *case,
                        );
                        if let Some(node) = self.module.control_nodes().get(*case) {
                            if node.kind() != ControlNodeKind::Case {
                                self.wrong_control_kind(
                                    match_node.span,
                                    "match case",
                                    ControlNodeKind::Case,
                                    node.kind(),
                                );
                            }
                        }
                    }
                }
                ControlNode::Case(case) => {
                    self.require_pattern(case.span, "control node", "case pattern", case.pattern);
                    for child in &case.children {
                        self.require_markup_node(case.span, "control node", "case child", *child);
                    }
                }
                ControlNode::Fragment(fragment) => {
                    for child in &fragment.children {
                        self.require_markup_node(
                            fragment.span,
                            "control node",
                            "fragment child",
                            *child,
                        );
                    }
                }
                ControlNode::With(with_node) => {
                    self.require_expr(
                        with_node.span,
                        "control node",
                        "with value",
                        with_node.value,
                    );
                    self.require_binding(
                        with_node.span,
                        "control node",
                        "with binding",
                        with_node.binding,
                    );
                    for child in &with_node.children {
                        self.require_markup_node(
                            with_node.span,
                            "control node",
                            "with child",
                            *child,
                        );
                    }
                }
            }
        }
    }

    fn validate_clusters(&mut self) {
        for (_, cluster) in self.module.clusters().iter() {
            self.check_span("cluster", cluster.span);
            let spine = cluster.normalized_spine();
            for member in spine.apply_arguments() {
                self.require_expr(cluster.span, "cluster", "cluster member", member);
            }
            if let ApplicativeSpineHead::Expr(finalizer) = spine.pure_head() {
                self.require_expr(cluster.span, "cluster", "cluster finalizer", finalizer);
            }
        }
    }

    fn validate_items(&mut self) {
        for (_, item) in self.module.items().iter() {
            self.check_span("item", item.span());
            for decorator in item.decorators() {
                self.require_decorator(item.span(), "item", "decorator", *decorator);
            }

            match item {
                Item::Type(item) => {
                    self.check_name(&item.name);
                    for parameter in &item.parameters {
                        self.require_type_parameter(
                            item.header.span,
                            "type item",
                            "type parameter",
                            *parameter,
                        );
                    }
                    match &item.body {
                        crate::hir::TypeItemBody::Alias(alias) => {
                            self.require_type(
                                item.header.span,
                                "type item",
                                "alias target",
                                *alias,
                            );
                        }
                        crate::hir::TypeItemBody::Sum(variants) => {
                            for variant in variants.iter() {
                                self.check_span("type variant", variant.span);
                                self.check_name(&variant.name);
                                for field in &variant.fields {
                                    self.require_type(
                                        variant.span,
                                        "type variant",
                                        "variant field type",
                                        *field,
                                    );
                                }
                            }
                        }
                    }
                }
                Item::Value(item) => {
                    self.check_name(&item.name);
                    if let Some(annotation) = item.annotation {
                        self.require_type(item.header.span, "value item", "annotation", annotation);
                    }
                    self.require_expr(item.header.span, "value item", "body", item.body);
                }
                Item::Function(item) => {
                    self.check_name(&item.name);
                    if let Some(annotation) = item.annotation {
                        self.require_type(
                            item.header.span,
                            "function item",
                            "annotation",
                            annotation,
                        );
                    }
                    for parameter in &item.parameters {
                        self.check_span("function parameter", parameter.span);
                        self.require_binding(
                            parameter.span,
                            "function parameter",
                            "binding",
                            parameter.binding,
                        );
                        if let Some(annotation) = parameter.annotation {
                            self.require_type(
                                parameter.span,
                                "function parameter",
                                "annotation",
                                annotation,
                            );
                        }
                    }
                    self.require_expr(item.header.span, "function item", "body", item.body);
                }
                Item::Signal(item) => {
                    self.check_name(&item.name);
                    if let Some(annotation) = item.annotation {
                        self.require_type(
                            item.header.span,
                            "signal item",
                            "annotation",
                            annotation,
                        );
                    }
                    if let Some(body) = item.body {
                        self.require_expr(item.header.span, "signal item", "body", body);
                    }
                    self.check_signal_dependencies(item.header.span, &item.signal_dependencies);
                    let has_source_decorator = item.header.decorators.iter().any(|decorator_id| {
                        matches!(
                            self.module
                                .decorators()
                                .get(*decorator_id)
                                .map(|decorator| &decorator.payload),
                            Some(DecoratorPayload::Source(_))
                        )
                    });
                    match (has_source_decorator, item.source_metadata.as_ref()) {
                        (true, Some(metadata)) => {
                            self.check_source_metadata(item.header.span, metadata)
                        }
                        (true, None) => self.diagnostics.push(
                            Diagnostic::error("source-backed signal is missing source metadata")
                                .with_code(code("missing-source-metadata"))
                                .with_label(DiagnosticLabel::primary(
                                    item.header.span,
                                    "populate source metadata after name resolution",
                                )),
                        ),
                        (false, Some(_)) => self.diagnostics.push(
                            Diagnostic::error(
                                "non-source signal unexpectedly carries source metadata",
                            )
                            .with_code(code("unexpected-source-metadata"))
                            .with_label(DiagnosticLabel::primary(
                                item.header.span,
                                "only `@source` signals should carry source metadata",
                            )),
                        ),
                        (false, None) => {}
                    }
                }
                Item::Class(item) => {
                    self.check_name(&item.name);
                    for parameter in item.parameters.iter() {
                        self.require_type_parameter(
                            item.header.span,
                            "class item",
                            "type parameter",
                            *parameter,
                        );
                    }
                    for superclass in &item.superclasses {
                        self.require_type(
                            item.header.span,
                            "class item",
                            "superclass",
                            *superclass,
                        );
                    }
                    for member in &item.members {
                        self.check_span("class member", member.span);
                        self.check_name(&member.name);
                        self.require_type(
                            member.span,
                            "class member",
                            "annotation",
                            member.annotation,
                        );
                    }
                }
                Item::Domain(item) => {
                    self.check_name(&item.name);
                    for parameter in &item.parameters {
                        self.require_type_parameter(
                            item.header.span,
                            "domain item",
                            "type parameter",
                            *parameter,
                        );
                    }
                    self.require_type(item.header.span, "domain item", "carrier", item.carrier);
                    for member in &item.members {
                        self.check_span("domain member", member.span);
                        self.check_name(&member.name);
                        self.require_type(
                            member.span,
                            "domain member",
                            "annotation",
                            member.annotation,
                        );
                    }
                }
                Item::Instance(item) => {
                    self.check_type_reference(&item.class);
                    for argument in item.arguments.iter() {
                        self.require_type(
                            item.header.span,
                            "instance item",
                            "instance argument",
                            *argument,
                        );
                    }
                    for context in &item.context {
                        self.require_type(
                            item.header.span,
                            "instance item",
                            "instance context",
                            *context,
                        );
                    }
                    for member in &item.members {
                        self.check_span("instance member", member.span);
                        self.check_name(&member.name);
                        if let Some(annotation) = member.annotation {
                            self.require_type(
                                member.span,
                                "instance member",
                                "annotation",
                                annotation,
                            );
                        }
                        self.require_expr(member.span, "instance member", "body", member.body);
                    }
                }
                Item::Use(item) => {
                    self.check_name_path(&item.module);
                    for import in item.imports.iter() {
                        self.require_import(
                            item.header.span,
                            "use item",
                            "import binding",
                            *import,
                        );
                    }
                }
                Item::Export(item) => {
                    self.check_name_path(&item.target);
                    self.check_resolution(
                        item.header.span,
                        "export target",
                        item.resolution.as_ref(),
                        |this, resolved| {
                            this.require_item(
                                item.header.span,
                                "export item",
                                "resolved target",
                                *resolved,
                            );
                        },
                    );
                }
            }
        }
    }

    fn validate_type_kinds(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let items = self
            .module
            .items()
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();

        for item in items {
            match item {
                Item::Type(item) => {
                    let parameters = item.parameters.clone();
                    match item.body {
                        crate::hir::TypeItemBody::Alias(alias) => {
                            self.check_expected_type_kind(alias, &parameters, "type alias body");
                        }
                        crate::hir::TypeItemBody::Sum(variants) => {
                            for variant in variants.iter() {
                                for field in &variant.fields {
                                    self.check_expected_type_kind(
                                        *field,
                                        &parameters,
                                        "type variant field",
                                    );
                                }
                            }
                        }
                    }
                }
                Item::Value(item) => {
                    if let Some(annotation) = item.annotation {
                        self.check_expected_type_kind(annotation, &[], "value annotation");
                    }
                }
                Item::Function(item) => {
                    if let Some(annotation) = item.annotation {
                        self.check_expected_type_kind(annotation, &[], "function annotation");
                    }
                    for parameter in &item.parameters {
                        if let Some(annotation) = parameter.annotation {
                            self.check_expected_type_kind(
                                annotation,
                                &[],
                                "function parameter annotation",
                            );
                        }
                    }
                }
                Item::Signal(item) => {
                    if let Some(annotation) = item.annotation {
                        self.check_expected_type_kind(annotation, &[], "signal annotation");
                    }
                }
                Item::Class(item) => {
                    let parameters = item.parameters.iter().copied().collect::<Vec<_>>();
                    for superclass in &item.superclasses {
                        self.check_expected_type_kind(*superclass, &parameters, "class superclass");
                    }
                    for member in &item.members {
                        self.check_expected_type_kind(
                            member.annotation,
                            &parameters,
                            "class member annotation",
                        );
                    }
                }
                Item::Domain(item) => {
                    let parameters = item.parameters.clone();
                    self.check_expected_type_kind(item.carrier, &parameters, "domain carrier");
                    for member in &item.members {
                        self.check_expected_type_kind(
                            member.annotation,
                            &parameters,
                            "domain member annotation",
                        );
                    }
                }
                Item::Instance(item) => {
                    self.check_type_reference_kind(
                        &item.class,
                        &[],
                        Kind::constructor(item.arguments.len()),
                        "instance class head",
                    );
                    for argument in item.arguments.iter() {
                        self.check_expected_type_kind(*argument, &[], "instance argument");
                    }
                    for context in &item.context {
                        self.check_expected_type_kind(*context, &[], "instance context");
                    }
                    for member in &item.members {
                        if let Some(annotation) = member.annotation {
                            self.check_expected_type_kind(
                                annotation,
                                &[],
                                "instance member annotation",
                            );
                        }
                    }
                }
                Item::Use(_) | Item::Export(_) => {}
            }
        }
    }

    fn validate_source_contract_types(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let decorators = self
            .module
            .decorators()
            .iter()
            .map(|(_, decorator)| decorator.clone())
            .collect::<Vec<_>>();
        let mut resolver = SourceContractTypeResolver::new(self.module);

        for decorator in decorators {
            let DecoratorPayload::Source(source) = decorator.payload else {
                continue;
            };
            self.validate_source_decorator_contract_types(&source, &mut resolver);
        }
    }

    fn validate_source_decorator_contract_types(
        &mut self,
        source: &crate::hir::SourceDecorator,
        resolver: &mut SourceContractTypeResolver<'_>,
    ) {
        let Some(provider) = source.provider.as_ref() else {
            return;
        };
        if provider.segments().len() < 2 {
            return;
        }
        let provider_key = provider_key_text(provider);
        let Some(provider) = BuiltinSourceProvider::parse(&provider_key) else {
            return;
        };
        let Some(options) = source.options else {
            return;
        };
        let ExprKind::Record(record) = &self.module.exprs()[options].kind else {
            return;
        };

        for field in &record.fields {
            let Some(option) = provider.contract().option(field.label.text()) else {
                continue;
            };
            if let Err(error) = resolver.resolve(option.ty()) {
                self.emit_source_contract_resolution_error(
                    field.span,
                    provider.key(),
                    field.label.text(),
                    option.ty(),
                    error.kind(),
                );
            }
        }
    }

    fn emit_source_contract_resolution_error(
        &mut self,
        span: SourceSpan,
        provider_key: &str,
        option_name: &str,
        expected: SourceContractType,
        error: &SourceContractResolutionErrorKind,
    ) {
        match error {
            SourceContractResolutionErrorKind::MissingType { name } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "source option `{option_name}` for `{provider_key}` expects `{expected}`, but `{name}` is not available as a same-module type"
                    ))
                    .with_code(code("missing-source-contract-type"))
                    .with_primary_label(
                        span,
                        format!(
                            "declare a same-module `type` or `domain` named `{name}` to satisfy this source contract"
                        ),
                    )
                    .with_note(
                        "current source-contract type resolution maps RFC helper names only through compiler builtins plus unique same-module `type`/`domain` items; imported helpers and option-value typing remain later work",
                    ),
                );
            }
            SourceContractResolutionErrorKind::AmbiguousType { name } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "source option `{option_name}` for `{provider_key}` expects `{expected}`, but `{name}` is ambiguous in this module"
                    ))
                    .with_code(code("ambiguous-source-contract-type"))
                    .with_primary_label(
                        span,
                        format!(
                            "this source contract cannot choose a unique same-module `type` or `domain` named `{name}`"
                        ),
                    ),
                );
            }
            SourceContractResolutionErrorKind::ArityMismatch {
                name,
                expected,
                actual,
                item,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "source option `{option_name}` for `{provider_key}` expects `{expected}`, but `{name}` has {}",
                        type_argument_phrase(*actual)
                    ))
                    .with_code(code("source-contract-type-arity"))
                    .with_primary_label(
                        span,
                        format!(
                            "this source contract needs `{name}` to accept {}",
                            type_argument_phrase(*expected)
                        ),
                    )
                    .with_secondary_label(
                        self.module.items()[*item].span(),
                        format!("`{name}` is declared here with {}", type_argument_phrase(*actual)),
                    )
                    .with_note(
                        "current source-contract type resolution checks only builtins and same-module type/domain arities before ordinary option expression typing exists",
                    ),
                );
            }
        }
    }

    fn validate_gate_semantics(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let items = self
            .module
            .items()
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        let mut typing = GateTypeContext::new(self.module);

        for item in items {
            match item {
                Item::Value(item) => {
                    self.validate_gate_expr_tree(item.body, &GateExprEnv::default(), &mut typing);
                }
                Item::Function(item) => {
                    let env = self.gate_env_for_function(&item, &mut typing);
                    self.validate_gate_expr_tree(item.body, &env, &mut typing);
                }
                Item::Signal(item) => {
                    if let Some(body) = item.body {
                        self.validate_gate_expr_tree(body, &GateExprEnv::default(), &mut typing);
                    }
                }
                Item::Instance(item) => {
                    for member in item.members {
                        self.validate_gate_expr_tree(
                            member.body,
                            &GateExprEnv::default(),
                            &mut typing,
                        );
                    }
                }
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }
    }

    fn validate_recurrence_targets(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let items = self
            .module
            .items()
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        let decorators = self
            .module
            .decorators()
            .iter()
            .map(|(_, decorator)| decorator.clone())
            .collect::<Vec<_>>();
        let mut typing = GateTypeContext::new(self.module);

        for item in items {
            match item {
                Item::Value(item) => {
                    let target = item.annotation.and_then(|annotation| {
                        typing.recurrence_target_hint_for_annotation(annotation)
                    });
                    self.validate_recurrence_expr_tree(item.body, target, None);
                }
                Item::Function(item) => {
                    let target = item.annotation.and_then(|annotation| {
                        typing.recurrence_target_hint_for_annotation(annotation)
                    });
                    self.validate_recurrence_expr_tree(item.body, target, None);
                }
                Item::Signal(item) => {
                    if let Some(body) = item.body {
                        let wakeup = self.recurrence_wakeup_hint_for_signal(&item);
                        self.validate_recurrence_expr_tree(
                            body,
                            Some(RecurrenceTargetHint::Evidence(
                                RecurrenceTargetEvidence::SignalItemBody,
                            )),
                            wakeup,
                        );
                    }
                }
                Item::Instance(item) => {
                    for member in item.members {
                        let target = member.annotation.and_then(|annotation| {
                            typing.recurrence_target_hint_for_annotation(annotation)
                        });
                        self.validate_recurrence_expr_tree(member.body, target, None);
                    }
                }
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }

        for decorator in decorators {
            match decorator.payload {
                DecoratorPayload::Bare => {}
                DecoratorPayload::Call(call) => {
                    for argument in call.arguments {
                        self.validate_recurrence_expr_tree(argument, None, None);
                    }
                    if let Some(options) = call.options {
                        self.validate_recurrence_expr_tree(options, None, None);
                    }
                }
                DecoratorPayload::Source(source) => {
                    for argument in source.arguments {
                        self.validate_recurrence_expr_tree(argument, None, None);
                    }
                    if let Some(options) = source.options {
                        self.validate_recurrence_expr_tree(options, None, None);
                    }
                }
            }
        }
    }

    fn validate_recurrence_expr_tree(
        &mut self,
        root: ExprId,
        root_target: Option<RecurrenceTargetHint>,
        root_wakeup: Option<RecurrenceWakeupHint>,
    ) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, is_root| {
            if let ExprKind::Pipe(pipe) = &expr.kind {
                let target = if is_root { root_target.as_ref() } else { None };
                let wakeup = if is_root { root_wakeup.as_ref() } else { None };
                self.validate_recurrence_pipe(pipe, target, wakeup, is_root);
            }
        });
    }

    fn validate_recurrence_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        target: Option<&RecurrenceTargetHint>,
        wakeup: Option<&RecurrenceWakeupHint>,
        is_root: bool,
    ) {
        let suffix = match pipe.recurrence_suffix() {
            Ok(Some(suffix)) => suffix,
            Ok(None) | Err(_) => return,
        };
        let start_span = suffix.start_stage().span;
        let target_valid = match target {
            Some(RecurrenceTargetHint::Evidence(evidence)) => {
                let _plan = RecurrencePlanner::plan(Some(*evidence))
                    .expect("explicit recurrence target evidence should always plan");
                true
            }
            Some(RecurrenceTargetHint::UnsupportedType { ty, span }) => {
                self.emit_unsupported_recurrence_target(start_span, *span, ty);
                false
            }
            None => {
                self.emit_unknown_recurrence_target(start_span, is_root);
                false
            }
        };
        if !target_valid {
            return;
        }
        match wakeup {
            Some(RecurrenceWakeupHint::BuiltinSource(context)) => {
                if RecurrenceWakeupPlanner::plan_source(*context).is_err() {
                    self.emit_missing_recurrence_wakeup(start_span, wakeup);
                }
            }
            Some(RecurrenceWakeupHint::CustomSource { .. }) | None => {
                self.emit_missing_recurrence_wakeup(start_span, wakeup);
            }
        }
    }

    fn recurrence_wakeup_hint_for_signal(&self, item: &SignalItem) -> Option<RecurrenceWakeupHint> {
        let source = self.signal_source_decorator(item)?;
        let provider = source.provider.as_ref()?;
        let provider_key = provider
            .segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".");
        let provider = match BuiltinSourceProvider::parse(&provider_key) {
            Some(provider) => provider,
            None => {
                return Some(RecurrenceWakeupHint::CustomSource {
                    provider_path: provider.clone(),
                });
            }
        };
        let mut context = SourceRecurrenceWakeupContext::new(provider);
        if item
            .source_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_reactive)
        {
            context = context.with_reactive_inputs();
        }
        if let Some(options) = source.options {
            if let ExprKind::Record(record) = &self.module.exprs()[options].kind {
                for field in &record.fields {
                    context = match field.label.text() {
                        "retry" => context.with_retry_policy(),
                        "refreshEvery" => context.with_polling_policy(),
                        "refreshOn" | "reloadOn" | "restartOn" => context.with_signal_trigger(),
                        _ => context,
                    };
                }
            }
        }
        Some(RecurrenceWakeupHint::BuiltinSource(context))
    }

    fn signal_source_decorator<'a>(&'a self, item: &SignalItem) -> Option<&'a SourceDecorator> {
        item.header.decorators.iter().find_map(|decorator_id| {
            let decorator = self.module.decorators().get(*decorator_id)?;
            match &decorator.payload {
                DecoratorPayload::Source(source) => Some(source),
                _ => None,
            }
        })
    }

    fn emit_unknown_recurrence_target(&mut self, span: SourceSpan, is_root: bool) {
        let label = if is_root {
            "annotate this declaration as `Signal ...` or `Task ...`, or move the recurrence into a `sig` body"
        } else {
            "move this recurrent pipe to a declaration body with explicit `Signal ...` or `Task ...` target evidence"
        };
        let note = if is_root {
            "the current recurrence-target slice accepts only direct signal item bodies plus explicit `Signal` or `Task` result annotations"
        } else {
            "nested recurrence target inference stays deferred until the compiler has fuller expression typing"
        };
        self.diagnostics.push(
            Diagnostic::error(
                "the compiler cannot determine a valid recurrence lowering target for this recurrent pipe",
            )
            .with_code(code("unknown-recurrence-target"))
            .with_primary_label(span, label)
            .with_note(note),
        );
    }

    fn emit_unsupported_recurrence_target(
        &mut self,
        span: SourceSpan,
        target_span: SourceSpan,
        ty: &GateType,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!("recurrent pipes cannot currently lower into `{ty}`"))
                .with_code(code("unsupported-recurrence-target"))
                .with_primary_label(
                    span,
                    "this recurrent suffix needs a `Signal`, `Task`, or future `@source` helper target",
                )
                .with_secondary_label(
                    target_span,
                    format!("the enclosing result annotation resolves to `{ty}`"),
                )
                .with_note(
                    "current recurrence-target checks accept only direct signal item bodies plus explicit `Signal` or `Task` result annotations",
                ),
        );
    }

    fn emit_missing_recurrence_wakeup(
        &mut self,
        span: SourceSpan,
        hint: Option<&RecurrenceWakeupHint>,
    ) {
        let mut diagnostic = Diagnostic::error(
            "the compiler cannot determine an explicit recurrence wakeup for this recurrent pipe",
        )
        .with_code(code("missing-recurrence-wakeup"));
        match hint {
            Some(RecurrenceWakeupHint::BuiltinSource(context)) => {
                let (label, note) = match context.provider() {
                    BuiltinSourceProvider::HttpGet | BuiltinSourceProvider::HttpPost => (
                        "this request-like source still needs an explicit recurrence wakeup such as `retry`, `refreshEvery`, `refreshOn`, or reactive source inputs",
                        "plain `http.get` / `http.post` sources issue one request when subscribed; polling, backoff, and refresh proof stay explicit at the current recurrence boundary",
                    ),
                    BuiltinSourceProvider::FsRead => (
                        "this snapshot source still needs an explicit recurrence wakeup such as `reloadOn` or reactive source inputs",
                        "`fs.read` publishes one snapshot and may be retriggered only explicitly; debounce and read-on-start do not by themselves prove recurrence wakeups",
                    ),
                    other => (
                        "this recurrent pipe still needs an explicit source-backed wakeup proof",
                        match other {
                            BuiltinSourceProvider::TimerEvery
                            | BuiltinSourceProvider::TimerAfter
                            | BuiltinSourceProvider::FsWatch
                            | BuiltinSourceProvider::SocketConnect
                            | BuiltinSourceProvider::MailboxSubscribe
                            | BuiltinSourceProvider::ProcessSpawn
                            | BuiltinSourceProvider::WindowKeyDown => {
                                "this built-in source should already have planned a wakeup; if you hit this diagnostic, keep the failing fixture because the recurrence wakeup adapter is inconsistent"
                            }
                            BuiltinSourceProvider::HttpGet
                            | BuiltinSourceProvider::HttpPost
                            | BuiltinSourceProvider::FsRead => {
                                unreachable!("request-like providers are handled above")
                            }
                        },
                    ),
                };
                diagnostic = diagnostic.with_primary_label(span, label).with_note(note);
            }
            Some(RecurrenceWakeupHint::CustomSource { provider_path }) => {
                diagnostic = diagnostic
                    .with_primary_label(
                        span,
                        "the compiler cannot yet prove a recurrence wakeup from this custom `@source` provider",
                    )
                    .with_secondary_label(
                        provider_path.span(),
                        "current recurrence wakeup checks know only compiler-built-in source provider semantics",
                    )
                    .with_note(
                        "custom provider-defined triggers remain deferred until source-provider contracts carry explicit wakeup metadata",
                    );
            }
            None => {
                diagnostic = diagnostic
                    .with_primary_label(
                        span,
                        "this recurrent pipe needs an explicit timer, backoff policy, source event, or provider-defined trigger",
                    )
                    .with_note(
                        "the current wakeup slice can prove recurrence wakeups only from compiler-known `@source` contexts; plain `Signal` / `Task` bodies still need future explicit timer/backoff evidence",
                    );
            }
        }
        self.diagnostics.push(diagnostic);
    }

    fn validate_gate_expr_tree(
        &mut self,
        root: ExprId,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, _| {
            if let ExprKind::Pipe(pipe) = &expr.kind {
                self.validate_gate_pipe(pipe, env, typing);
            }
        });
    }

    fn validate_gate_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let mut current = typing.infer_expr(pipe.head, env, None).ty;
        for stage in pipe.stages.iter() {
            let Some(subject) = current.clone() else {
                break;
            };
            match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    current = typing.infer_transform_stage(*expr, env, &subject);
                }
                PipeStageKind::Tap { expr } => {
                    let _ = typing.infer_pipe_body(*expr, env, &subject);
                    current = Some(subject);
                }
                PipeStageKind::Gate { expr } => {
                    current = self.validate_gate_stage(stage.span, *expr, env, &subject, typing);
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Map { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::FanIn { .. }
                | PipeStageKind::Truthy { .. }
                | PipeStageKind::Falsy { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => {
                    current = None;
                }
            }
        }
    }

    fn validate_gate_stage(
        &mut self,
        _stage_span: SourceSpan,
        predicate: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let predicate_info = typing.infer_pipe_body(predicate, env, subject);
        let mut saw_error = false;
        for issue in predicate_info.issues {
            self.emit_gate_issue(issue);
            saw_error = true;
        }
        if predicate_info.contains_signal
            || predicate_info.ty.as_ref().is_some_and(GateType::is_signal)
        {
            self.diagnostics.push(
                Diagnostic::error("gate predicate must be pure and cannot read a signal directly")
                    .with_code(code("impure-gate-predicate"))
                    .with_label(DiagnosticLabel::primary(
                        self.module.exprs()[predicate].span,
                        "compute a `Bool` from the current subject instead of sampling a signal here",
                    )),
            );
            saw_error = true;
        }
        if let Some(predicate_ty) = predicate_info.ty {
            if !predicate_ty.is_bool() {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate must produce `Bool`, found `{predicate_ty}`"
                    ))
                    .with_code(code("gate-predicate-not-bool"))
                    .with_label(DiagnosticLabel::primary(
                        self.module.exprs()[predicate].span,
                        "this gate body does not evaluate to `Bool` for the current subject",
                    )),
                );
                saw_error = true;
            }
        }
        if saw_error {
            return None;
        }

        let plan = GatePlanner::plan(typing.gate_carrier(subject));
        Some(typing.apply_gate_plan(plan, subject))
    }

    fn gate_env_for_function(
        &self,
        item: &crate::hir::FunctionItem,
        typing: &mut GateTypeContext<'_>,
    ) -> GateExprEnv {
        let mut env = GateExprEnv::default();
        for parameter in &item.parameters {
            let Some(annotation) = parameter.annotation else {
                continue;
            };
            if let Some(ty) = typing.lower_annotation(annotation) {
                env.locals.insert(parameter.binding, ty);
            }
        }
        env
    }

    fn emit_gate_issue(&mut self, issue: GateIssue) {
        match issue {
            GateIssue::InvalidProjection {
                span,
                path,
                subject,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate cannot project `{path}` from non-record subject `{subject}`"
                    ))
                    .with_code(code("invalid-gate-projection"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "project from a record-valued subject or transform to the desired field first",
                    )),
                );
            }
            GateIssue::UnknownField {
                span,
                path,
                subject,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate cannot find field `{path}` on subject `{subject}`"
                    ))
                    .with_code(code("unknown-gate-field"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use a field that exists on the current subject",
                    )),
                );
            }
        }
    }

    fn check_expected_type_kind(
        &mut self,
        ty: TypeId,
        parameters: &[TypeParameterId],
        subject: &'static str,
    ) {
        self.check_type_kind(ty, parameters, Kind::Type, subject);
    }

    fn check_type_kind(
        &mut self,
        ty: TypeId,
        parameters: &[TypeParameterId],
        expected: Kind,
        subject: &'static str,
    ) {
        let Some((store, root, spans)) = self.build_kind_graph_for_type(ty, parameters) else {
            return;
        };
        if let Err(error) = KindChecker.expect_kind(&store, root, &expected) {
            self.emit_kind_error(subject, &spans, error);
        }
    }

    fn check_type_reference_kind(
        &mut self,
        reference: &TypeReference,
        parameters: &[TypeParameterId],
        expected: Kind,
        subject: &'static str,
    ) {
        let Some((store, root, spans)) = self.build_kind_graph_for_reference(reference, parameters)
        else {
            return;
        };
        if let Err(error) = KindChecker.expect_kind(&store, root, &expected) {
            self.emit_kind_error(subject, &spans, error);
        }
    }

    fn build_kind_graph_for_type(
        &mut self,
        root: TypeId,
        parameters: &[TypeParameterId],
    ) -> Option<(KindStore, KindExprId, HashMap<KindExprId, SourceSpan>)> {
        let mut store = KindStore::default();
        let mut spans = HashMap::new();
        let mut parameter_map = self.kind_parameter_map(parameters, &mut store);
        let mut lowered = HashMap::new();
        let mut stack = vec![KindBuildFrame::Enter(root)];

        while let Some(frame) = stack.pop() {
            match frame {
                KindBuildFrame::Enter(type_id) => {
                    if lowered.contains_key(&type_id) {
                        continue;
                    }
                    match &self.module.types()[type_id].kind {
                        TypeKind::Name(reference) => {
                            let expr = self.kind_expr_for_reference(
                                reference,
                                &mut store,
                                &mut spans,
                                &mut parameter_map,
                            )?;
                            lowered.insert(type_id, expr);
                        }
                        TypeKind::Tuple(elements) => {
                            stack.push(KindBuildFrame::Exit(type_id));
                            let elements = elements.iter().copied().collect::<Vec<_>>();
                            for element in elements.into_iter().rev() {
                                stack.push(KindBuildFrame::Enter(element));
                            }
                        }
                        TypeKind::Record(fields) => {
                            stack.push(KindBuildFrame::Exit(type_id));
                            for field in fields.iter().rev() {
                                stack.push(KindBuildFrame::Enter(field.ty));
                            }
                        }
                        TypeKind::Arrow { parameter, result } => {
                            stack.push(KindBuildFrame::Exit(type_id));
                            stack.push(KindBuildFrame::Enter(*result));
                            stack.push(KindBuildFrame::Enter(*parameter));
                        }
                        TypeKind::Apply { callee, arguments } => {
                            stack.push(KindBuildFrame::Exit(type_id));
                            let arguments = arguments.iter().copied().collect::<Vec<_>>();
                            for argument in arguments.into_iter().rev() {
                                stack.push(KindBuildFrame::Enter(argument));
                            }
                            stack.push(KindBuildFrame::Enter(*callee));
                        }
                    }
                }
                KindBuildFrame::Exit(type_id) => {
                    let expr = match &self.module.types()[type_id].kind {
                        TypeKind::Name(_) => unreachable!("name nodes lower during enter"),
                        TypeKind::Tuple(elements) => store.tuple_expr(
                            elements
                                .iter()
                                .map(|element| lowered[element])
                                .collect::<Vec<_>>(),
                        ),
                        TypeKind::Record(fields) => store.record_expr(
                            fields
                                .iter()
                                .map(|field| {
                                    KindRecordField::new(field.label.text(), lowered[&field.ty])
                                })
                                .collect::<Vec<_>>(),
                        ),
                        TypeKind::Arrow { parameter, result } => {
                            store.arrow_expr(lowered[parameter], lowered[result])
                        }
                        TypeKind::Apply { callee, arguments } => {
                            let mut expr = lowered[callee];
                            for argument in arguments.iter() {
                                expr = store.apply_expr(expr, lowered[argument]);
                                spans.insert(expr, self.module.types()[type_id].span);
                            }
                            expr
                        }
                    };
                    spans
                        .entry(expr)
                        .or_insert(self.module.types()[type_id].span);
                    lowered.insert(type_id, expr);
                }
            }
        }

        Some((store, lowered[&root], spans))
    }

    fn build_kind_graph_for_reference(
        &mut self,
        reference: &TypeReference,
        parameters: &[TypeParameterId],
    ) -> Option<(KindStore, KindExprId, HashMap<KindExprId, SourceSpan>)> {
        let mut store = KindStore::default();
        let mut spans = HashMap::new();
        let mut parameter_map = self.kind_parameter_map(parameters, &mut store);
        let root =
            self.kind_expr_for_reference(reference, &mut store, &mut spans, &mut parameter_map)?;
        Some((store, root, spans))
    }

    fn kind_parameter_map(
        &self,
        parameters: &[TypeParameterId],
        store: &mut KindStore,
    ) -> HashMap<TypeParameterId, TypingKindParameterId> {
        let mut parameter_map = HashMap::new();
        for parameter in parameters {
            let kind_parameter = store.add_parameter(
                self.module.type_parameters()[*parameter]
                    .name
                    .text()
                    .to_owned(),
            );
            parameter_map.insert(*parameter, kind_parameter);
        }
        parameter_map
    }

    fn kind_expr_for_reference(
        &mut self,
        reference: &TypeReference,
        store: &mut KindStore,
        spans: &mut HashMap<KindExprId, SourceSpan>,
        parameters: &mut HashMap<TypeParameterId, TypingKindParameterId>,
    ) -> Option<KindExprId> {
        let expr = match reference.resolution.as_ref() {
            ResolutionState::Unresolved => return None,
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => {
                let constructor =
                    store.add_constructor(builtin_type_name(*builtin), builtin_kind(*builtin));
                store.constructor_expr(constructor)
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                let parameter = *parameters.entry(*parameter).or_insert_with(|| {
                    store.add_parameter(
                        self.module.type_parameters()[*parameter]
                            .name
                            .text()
                            .to_owned(),
                    )
                });
                store.parameter_expr(parameter)
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                let constructor = store.add_constructor(
                    item_type_name(&self.module.items()[*item_id]),
                    self.kind_for_item(*item_id)?,
                );
                store.constructor_expr(constructor)
            }
            ResolutionState::Resolved(TypeResolution::Import(_)) => return None,
        };
        spans.insert(expr, reference.span());
        Some(expr)
    }

    fn kind_for_item(&mut self, item_id: ItemId) -> Option<Kind> {
        match &self.module.items()[item_id] {
            Item::Type(item) => Some(Kind::constructor(item.parameters.len())),
            Item::Class(item) => Some(Kind::constructor(item.parameters.len())),
            Item::Domain(item) => Some(Kind::constructor(item.parameters.len())),
            other => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "type resolution unexpectedly points at non-type item kind {:?}",
                        other.kind()
                    ))
                    .with_code(code("invalid-type-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        other.span(),
                        "only type, class, and domain items may appear in type resolution",
                    )),
                );
                None
            }
        }
    }

    fn emit_kind_error(
        &mut self,
        subject: &'static str,
        spans: &HashMap<KindExprId, SourceSpan>,
        error: KindCheckError,
    ) {
        match error.kind() {
            KindCheckErrorKind::CannotApplyNonConstructor { callee_kind, .. } => {
                let span = spans.get(&error.expr()).copied().unwrap_or_default();
                self.diagnostics.push(
                    Diagnostic::error("type application is over-saturated")
                        .with_code(code("invalid-type-application"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            format!(
                                "this application already has kind `{callee_kind}` and cannot take another type argument"
                            ),
                        )),
                );
            }
            KindCheckErrorKind::ArgumentKindMismatch {
                expected,
                argument,
                found,
                ..
            } => {
                let span = spans.get(argument).copied().unwrap_or_default();
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "type argument kind mismatch: expected `{expected}`, found `{found}`"
                    ))
                    .with_code(code("invalid-type-argument-kind"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "this type argument does not match the constructor's expected kind",
                    )),
                );
            }
            KindCheckErrorKind::ExpectedType { child, found } => {
                let span = spans.get(child).copied().unwrap_or_default();
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} requires a concrete type, found kind `{found}`"
                    ))
                    .with_code(code("expected-type-kind"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "fully apply this type constructor before using it here",
                    )),
                );
            }
            KindCheckErrorKind::ExpectedKind { expected, found } => {
                let span = spans.get(&error.expr()).copied().unwrap_or_default();
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} has kind `{found}`, expected `{expected}`"
                    ))
                    .with_code(code("expected-kind-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "adjust the applied type arguments to match the expected constructor kind",
                    )),
                );
            }
        }
    }

    fn check_name(&mut self, name: &Name) {
        self.check_span("name", name.span());
    }

    fn check_name_path(&mut self, path: &NamePath) {
        self.check_span("name path", path.span());
        for segment in path.segments().iter() {
            self.check_name(segment);
        }
    }

    fn check_term_reference(&mut self, reference: &TermReference) {
        self.check_name_path(&reference.path);
        self.check_resolution(
            reference.span(),
            "term reference",
            reference.resolution.as_ref(),
            |this, resolution| match resolution {
                TermResolution::Local(binding) => {
                    this.require_binding(reference.span(), "term reference", "binding", *binding);
                }
                TermResolution::Item(item) => {
                    this.require_item(reference.span(), "term reference", "item", *item);
                }
                TermResolution::Import(import) => {
                    this.require_import(reference.span(), "term reference", "import", *import);
                }
                TermResolution::Builtin(_) => {}
            },
        );
    }

    fn check_suffixed_integer(&mut self, literal: &crate::hir::SuffixedIntegerLiteral) {
        self.check_name(&literal.suffix);
        self.check_resolution(
            literal.suffix.span(),
            "literal suffix",
            literal.resolution.as_ref(),
            |this, resolution| {
                this.require_literal_suffix_resolution(
                    literal.suffix.span(),
                    &literal.suffix,
                    *resolution,
                );
            },
        );
    }

    fn check_text_literal(&mut self, owner_span: SourceSpan, text: &TextLiteral) {
        for segment in &text.segments {
            match segment {
                TextSegment::Text(fragment) => self.check_span("text fragment", fragment.span),
                TextSegment::Interpolation(interpolation) => {
                    self.check_span("text interpolation", interpolation.span);
                    self.require_expr(
                        owner_span,
                        "text literal",
                        "interpolation expression",
                        interpolation.expr,
                    );
                }
            }
        }
    }

    fn check_source_metadata(&mut self, span: SourceSpan, metadata: &SourceMetadata) {
        for dependency in &metadata.signal_dependencies {
            self.require_item(span, "source metadata", "signal dependency", *dependency);
            if let Some(item) = self.module.items().get(*dependency) {
                if !matches!(item, Item::Signal(_)) {
                    self.diagnostics.push(
                        Diagnostic::error("source metadata dependency must point at a signal item")
                            .with_code(code("invalid-source-dependency"))
                            .with_label(DiagnosticLabel::primary(
                                span,
                                "update the source metadata to reference only signal items",
                            )),
                    );
                }
            }
        }
    }

    fn check_signal_dependencies(&mut self, span: SourceSpan, dependencies: &[ItemId]) {
        let mut previous = None;
        for dependency in dependencies {
            self.require_item(span, "signal item", "signal dependency", *dependency);
            if let Some(item) = self.module.items().get(*dependency) {
                if !matches!(item, Item::Signal(_)) {
                    self.diagnostics.push(
                        Diagnostic::error("signal dependency must point at a signal item")
                            .with_code(code("invalid-signal-dependency"))
                            .with_label(DiagnosticLabel::primary(
                                span,
                                "update the signal dependency list to reference only signal items",
                            )),
                    );
                }
            }
            if let Some(previous) = previous {
                if previous >= *dependency {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "signal dependency lists must stay sorted and duplicate-free",
                        )
                        .with_code(code("unordered-signal-dependencies"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "normalize signal dependency ordering after resolution",
                        )),
                    );
                    break;
                }
            }
            previous = Some(*dependency);
        }
    }

    fn check_type_reference(&mut self, reference: &TypeReference) {
        self.check_name_path(&reference.path);
        self.check_resolution(
            reference.span(),
            "type reference",
            reference.resolution.as_ref(),
            |this, resolution| match resolution {
                TypeResolution::Item(item) => {
                    this.require_item(reference.span(), "type reference", "item", *item);
                }
                TypeResolution::TypeParameter(parameter) => {
                    this.require_type_parameter(
                        reference.span(),
                        "type reference",
                        "type parameter",
                        *parameter,
                    );
                }
                TypeResolution::Import(import) => {
                    this.require_import(reference.span(), "type reference", "import", *import);
                }
                TypeResolution::Builtin(_) => {}
            },
        );
    }

    fn check_resolution<T>(
        &mut self,
        span: SourceSpan,
        subject: &'static str,
        resolution: ResolutionState<&T>,
        on_resolved: impl FnOnce(&mut Self, &T),
    ) {
        match resolution {
            ResolutionState::Resolved(value) => on_resolved(self, value),
            ResolutionState::Unresolved if self.mode == ValidationMode::RequireResolvedNames => {
                self.diagnostics.push(
                    Diagnostic::error(format!("{subject} remains unresolved in resolved HIR mode"))
                        .with_code(code("unresolved-name"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "Milestone 2 HIR should resolve this reference before validation",
                        )),
                );
            }
            ResolutionState::Unresolved => {}
        }
    }

    fn check_span(&mut self, subject: &'static str, span: SourceSpan) {
        if span.file() != self.module.file() {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "{subject} belongs to file {} but module owns file {}",
                    span.file(),
                    self.module.file()
                ))
                .with_code(code("foreign-span"))
                .with_label(DiagnosticLabel::primary(
                    span,
                    "all HIR nodes in one module must point at that module's file",
                )),
            );
        }
    }

    fn illegal_direct_control(&mut self, span: SourceSpan, kind: ControlNodeKind) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "markup nodes cannot directly reference branch-only control node kind {kind:?}"
            ))
            .with_code(code("illegal-control-kind"))
            .with_label(DiagnosticLabel::primary(
                span,
                "only show/each/match/fragment/with are renderable control nodes",
            )),
        );
    }

    fn wrong_control_kind(
        &mut self,
        span: SourceSpan,
        subject: &'static str,
        expected: ControlNodeKind,
        found: ControlNodeKind,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "{subject} expected {expected:?} but found {found:?}"
            ))
            .with_code(code("wrong-control-kind"))
            .with_label(DiagnosticLabel::primary(
                span,
                "use the dedicated control-node form required by this parent",
            )),
        );
    }

    fn require_item(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: ItemId,
    ) {
        self.require_node(self.module.items(), span, owner, field, id, "item");
    }

    fn require_literal_suffix_resolution(
        &mut self,
        span: SourceSpan,
        suffix: &Name,
        resolution: LiteralSuffixResolution,
    ) {
        let Some(item) = self.module.items().get(resolution.domain) else {
            self.diagnostics.push(
                Diagnostic::error("literal suffix resolution points at a missing domain item")
                    .with_code(code("invalid-literal-suffix-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "update the literal suffix resolution to target an existing domain item",
                    )),
            );
            return;
        };
        let Item::Domain(domain) = item else {
            self.diagnostics.push(
                Diagnostic::error("literal suffix resolution does not target a domain item")
                    .with_code(code("invalid-literal-suffix-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "literal suffixes must resolve to domain literal declarations",
                    )),
            );
            return;
        };
        let Some(member) = domain.members.get(resolution.member_index) else {
            self.diagnostics.push(
                Diagnostic::error("literal suffix resolution points at a missing domain member")
                    .with_code(code("invalid-literal-suffix-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "update the literal suffix resolution to target an existing literal member",
                    )),
            );
            return;
        };
        if member.kind != DomainMemberKind::Literal || member.name.text() != suffix.text() {
            self.diagnostics.push(
                Diagnostic::error(
                    "literal suffix resolution does not match the target domain literal",
                )
                .with_code(code("invalid-literal-suffix-resolution"))
                .with_label(DiagnosticLabel::primary(
                    span,
                    "the resolved domain literal must match the suffix spelling used here",
                )),
            );
        }
    }

    fn require_expr(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: ExprId,
    ) {
        self.require_node(self.module.exprs(), span, owner, field, id, "expression");
    }

    fn require_pattern(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: PatternId,
    ) {
        self.require_node(self.module.patterns(), span, owner, field, id, "pattern");
    }

    fn require_type(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: TypeId,
    ) {
        self.require_node(self.module.types(), span, owner, field, id, "type");
    }

    fn require_decorator(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: DecoratorId,
    ) {
        self.require_node(
            self.module.decorators(),
            span,
            owner,
            field,
            id,
            "decorator",
        );
    }

    fn require_markup_node(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: MarkupNodeId,
    ) {
        self.require_node(
            self.module.markup_nodes(),
            span,
            owner,
            field,
            id,
            "markup node",
        );
    }

    fn require_control_node(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: ControlNodeId,
    ) {
        self.require_node(
            self.module.control_nodes(),
            span,
            owner,
            field,
            id,
            "control node",
        );
    }

    fn require_cluster(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: ClusterId,
    ) {
        self.require_node(self.module.clusters(), span, owner, field, id, "cluster");
    }

    fn require_binding(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: BindingId,
    ) {
        self.require_node(self.module.bindings(), span, owner, field, id, "binding");
    }

    fn require_type_parameter(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: TypeParameterId,
    ) {
        self.require_node(
            self.module.type_parameters(),
            span,
            owner,
            field,
            id,
            "type parameter",
        );
    }

    fn require_import(
        &mut self,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: ImportId,
    ) {
        self.require_node(self.module.imports(), span, owner, field, id, "import");
    }

    fn require_node<Id, T>(
        &mut self,
        arena: &Arena<Id, T>,
        span: SourceSpan,
        owner: &'static str,
        field: &'static str,
        id: Id,
        family: &'static str,
    ) where
        Id: ArenaId,
    {
        if !arena.contains(id) {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "{owner} references missing {family} {id} through {field}"
                ))
                .with_code(code("missing-node"))
                .with_label(DiagnosticLabel::primary(
                    span,
                    format!("expected a valid {family} id for {field}"),
                )),
            );
        }
    }
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

fn builtin_kind(builtin: BuiltinType) -> Kind {
    match builtin {
        BuiltinType::Int
        | BuiltinType::Float
        | BuiltinType::Decimal
        | BuiltinType::BigInt
        | BuiltinType::Bool
        | BuiltinType::Text
        | BuiltinType::Unit
        | BuiltinType::Bytes => Kind::Type,
        BuiltinType::List | BuiltinType::Option | BuiltinType::Signal => Kind::constructor(1),
        BuiltinType::Result | BuiltinType::Validation | BuiltinType::Task => Kind::constructor(2),
    }
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
        BuiltinType::Option => "Option",
        BuiltinType::Result => "Result",
        BuiltinType::Validation => "Validation",
        BuiltinType::Signal => "Signal",
        BuiltinType::Task => "Task",
    }
}

fn item_type_name(item: &Item) -> String {
    match item {
        Item::Type(item) => item.name.text().to_owned(),
        Item::Class(item) => item.name.text().to_owned(),
        Item::Domain(item) => item.name.text().to_owned(),
        other => format!("{:?}", other.kind()),
    }
}

#[derive(Clone, Copy, Debug)]
enum ExprWalkWork {
    Expr { expr: ExprId, is_root: bool },
    Markup(MarkupNodeId),
    Control(ControlNodeId),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprEnv {
    pub(crate) locals: HashMap<BindingId, GateType>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprInfo {
    pub(crate) ty: Option<GateType>,
    pub(crate) contains_signal: bool,
    pub(crate) issues: Vec<GateIssue>,
}

impl GateExprInfo {
    fn merge(&mut self, other: Self) {
        self.contains_signal |= other.contains_signal;
        self.issues.extend(other.issues);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateIssue {
    InvalidProjection {
        span: SourceSpan,
        path: String,
        subject: String,
    },
    UnknownField {
        span: SourceSpan,
        path: String,
        subject: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecurrenceTargetHint {
    Evidence(RecurrenceTargetEvidence),
    UnsupportedType { ty: GateType, span: SourceSpan },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecurrenceWakeupHint {
    BuiltinSource(SourceRecurrenceWakeupContext),
    CustomSource { provider_path: NamePath },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRecordField {
    pub name: String,
    pub ty: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateType {
    Primitive(BuiltinType),
    Tuple(Vec<GateType>),
    Record(Vec<GateRecordField>),
    Arrow {
        parameter: Box<GateType>,
        result: Box<GateType>,
    },
    List(Box<GateType>),
    Option(Box<GateType>),
    Result {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Validation {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Signal(Box<GateType>),
    Task {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
}

impl GateType {
    pub(crate) fn is_bool(&self) -> bool {
        matches!(self, Self::Primitive(BuiltinType::Bool))
    }

    pub(crate) fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn gate_carrier(&self) -> GateCarrier {
        match self {
            Self::Signal(_) => GateCarrier::Signal,
            _ => GateCarrier::Ordinary,
        }
    }

    pub(crate) fn gate_payload(&self) -> &Self {
        match self {
            Self::Signal(inner) => inner,
            other => other,
        }
    }

    pub(crate) fn recurrence_target_evidence(&self) -> Option<RecurrenceTargetEvidence> {
        match self {
            Self::Signal(_) => Some(RecurrenceTargetEvidence::ExplicitSignalAnnotation),
            Self::Task { .. } => Some(RecurrenceTargetEvidence::ExplicitTaskAnnotation),
            _ => None,
        }
    }

    pub(crate) fn same_shape(&self, other: &Self) -> bool {
        self == other
    }
}

impl fmt::Display for GateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateType::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            GateType::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            GateType::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            GateType::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            GateType::List(element) => write!(f, "List {element}"),
            GateType::Option(element) => write!(f, "Option {element}"),
            GateType::Result { error, value } => write!(f, "Result {error} {value}"),
            GateType::Validation { error, value } => {
                write!(f, "Validation {error} {value}")
            }
            GateType::Signal(element) => write!(f, "Signal {element}"),
            GateType::Task { error, value } => write!(f, "Task {error} {value}"),
            GateType::Domain {
                name, arguments, ..
            }
            | GateType::OpaqueItem {
                name, arguments, ..
            } => {
                write!(f, "{name}")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) struct GateTypeContext<'a> {
    module: &'a Module,
    item_types: HashMap<ItemId, Option<GateType>>,
}

impl<'a> GateTypeContext<'a> {
    pub(crate) fn new(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
        }
    }

    pub(crate) fn gate_carrier(&self, subject: &GateType) -> GateCarrier {
        subject.gate_carrier()
    }

    pub(crate) fn apply_gate_plan(
        &self,
        plan: aivi_typing::GatePlan,
        subject: &GateType,
    ) -> GateType {
        match plan.result() {
            GateResultKind::OptionWrappedSubject => GateType::Option(Box::new(subject.clone())),
            GateResultKind::PreservedSignalSubject => match subject {
                GateType::Signal(_) => subject.clone(),
                other => GateType::Signal(Box::new(other.clone())),
            },
        }
    }

    pub(crate) fn lower_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new())
    }

    fn recurrence_target_hint_for_annotation(
        &mut self,
        annotation: TypeId,
    ) -> Option<RecurrenceTargetHint> {
        let ty = self.lower_annotation(annotation)?;
        Some(match ty.recurrence_target_evidence() {
            Some(evidence) => RecurrenceTargetHint::Evidence(evidence),
            None => RecurrenceTargetHint::UnsupportedType {
                ty,
                span: self.module.types()[annotation].span,
            },
        })
    }

    fn item_value_type(&mut self, item_id: ItemId) -> Option<GateType> {
        if let Some(cached) = self.item_types.get(&item_id) {
            return cached.clone();
        }
        let ty = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation)),
            Item::Function(item) => {
                let result = item
                    .annotation
                    .and_then(|annotation| self.lower_annotation(annotation))?;
                let mut parameters = Vec::with_capacity(item.parameters.len());
                for parameter in &item.parameters {
                    let annotation = parameter.annotation?;
                    parameters.push(self.lower_annotation(annotation)?);
                }
                let mut ty = result;
                for parameter in parameters.into_iter().rev() {
                    ty = GateType::Arrow {
                        parameter: Box::new(parameter),
                        result: Box::new(ty),
                    };
                }
                Some(ty)
            }
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation)),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        };
        self.item_types.insert(item_id, ty.clone());
        ty
    }

    fn lower_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => {
                self.lower_type_reference(reference, substitutions, item_stack)
            }
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_type(*element, substitutions, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_type(field.ty, substitutions, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_type(*parameter, substitutions, item_stack)?),
                result: Box::new(self.lower_type(*result, substitutions, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        substitutions,
                        item_stack,
                    )?);
                }
                self.lower_type_application(*callee, &lowered_arguments, substitutions, item_stack)
            }
        }
    }

    fn lower_type_reference(
        &mut self,
        reference: &TypeReference,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => None,
            ResolutionState::Resolved(TypeResolution::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            )) => Some(GateType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::Builtin(_)) => None,
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned()
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Import(_)) => None,
        }
    }

    fn lower_type_application(
        &mut self,
        callee: TypeId,
        arguments: &[GateType],
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Some(GateType::List(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                Some(GateType::Option(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                Some(GateType::Result {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                Some(GateType::Validation {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                Some(GateType::Signal(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                Some(GateType::Task {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, arguments, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned()
            }
            ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(
                BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes,
            ))
            | ResolutionState::Unresolved => None,
        }
    }

    fn lower_type_item(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let item = &self.module.items()[item_id];
        let name = item_type_name(item);
        if item_stack.contains(&item_id) {
            return Some(GateType::OpaqueItem {
                item: item_id,
                name,
                arguments: arguments.to_vec(),
            });
        }
        item_stack.push(item_id);
        let lowered = match item {
            Item::Type(item) => {
                if item.parameters.len() != arguments.len() {
                    None
                } else {
                    match &item.body {
                        crate::hir::TypeItemBody::Alias(alias) => {
                            let substitutions = item
                                .parameters
                                .iter()
                                .copied()
                                .zip(arguments.iter().cloned())
                                .collect::<HashMap<_, _>>();
                            self.lower_type(*alias, &substitutions, item_stack)
                        }
                        crate::hir::TypeItemBody::Sum(_) => Some(GateType::OpaqueItem {
                            item: item_id,
                            name: item.name.text().to_owned(),
                            arguments: arguments.to_vec(),
                        }),
                    }
                }
            }
            Item::Domain(item) => Some(GateType::Domain {
                item: item_id,
                name: item.name.text().to_owned(),
                arguments: arguments.to_vec(),
            }),
            Item::Class(_)
            | Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        };
        let popped = item_stack.pop();
        debug_assert_eq!(popped, Some(item_id));
        lowered
    }

    pub(crate) fn infer_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> GateExprInfo {
        let expr = self.module.exprs()[expr_id].clone();
        match expr.kind {
            ExprKind::Name(reference) => self.infer_name(&reference, env),
            ExprKind::Integer(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Int)),
                ..GateExprInfo::default()
            },
            ExprKind::SuffixedInteger(literal) => GateExprInfo {
                ty: match literal.resolution.as_ref() {
                    ResolutionState::Resolved(resolution) => {
                        let domain = &self.module.items()[resolution.domain];
                        Some(GateType::Domain {
                            item: resolution.domain,
                            name: item_type_name(domain),
                            arguments: Vec::new(),
                        })
                    }
                    ResolutionState::Unresolved => None,
                },
                ..GateExprInfo::default()
            },
            ExprKind::Text(text) => {
                let mut info = GateExprInfo {
                    ty: Some(GateType::Primitive(BuiltinType::Text)),
                    ..GateExprInfo::default()
                };
                for segment in text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        info.merge(self.infer_expr(interpolation.expr, env, ambient));
                    }
                }
                info
            }
            ExprKind::Regex(_) => GateExprInfo::default(),
            ExprKind::Tuple(elements) => {
                let mut info = GateExprInfo::default();
                let mut lowered = Vec::with_capacity(elements.len());
                let mut complete = true;
                for element in elements.iter() {
                    let child = self.infer_expr(*element, env, ambient);
                    complete &= child.ty.is_some();
                    if let Some(ty) = child.ty.clone() {
                        lowered.push(ty);
                    }
                    info.merge(child);
                }
                if complete {
                    info.ty = Some(GateType::Tuple(lowered));
                }
                info
            }
            ExprKind::List(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<GateType>;
                for element in &elements {
                    let child = self.infer_expr(*element, env, ambient);
                    if let Some(child_ty) = child.ty.as_ref() {
                        match &element_type {
                            None => element_type = Some(child_ty.clone()),
                            Some(current) if current.same_shape(child_ty) => {}
                            Some(_) => element_type = None,
                        }
                    }
                    info.merge(child);
                }
                if let Some(element_type) = element_type {
                    info.ty = Some(GateType::List(Box::new(element_type)));
                }
                info
            }
            ExprKind::Record(record) => {
                let mut info = GateExprInfo::default();
                let mut fields = Vec::with_capacity(record.fields.len());
                let mut complete = true;
                for field in record.fields {
                    let child = self.infer_expr(field.value, env, ambient);
                    complete &= child.ty.is_some();
                    if let Some(ty) = child.ty.clone() {
                        fields.push(GateRecordField {
                            name: field.label.text().to_owned(),
                            ty,
                        });
                    }
                    info.merge(child);
                }
                if complete {
                    info.ty = Some(GateType::Record(fields));
                }
                info
            }
            ExprKind::Projection { base, path } => {
                let mut info = GateExprInfo::default();
                let subject = match base {
                    crate::hir::ProjectionBase::Ambient => ambient.cloned(),
                    crate::hir::ProjectionBase::Expr(base) => {
                        let base_info = self.infer_expr(base, env, ambient);
                        let ty = base_info.ty.clone();
                        info.merge(base_info);
                        ty
                    }
                };
                if let Some(subject) = subject {
                    match self.project_type(&subject, &path) {
                        Ok(projected) => info.ty = Some(projected),
                        Err(issue) => info.issues.push(issue),
                    }
                } else {
                    info.issues.push(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(&path),
                        subject: "unknown subject".to_owned(),
                    });
                }
                info
            }
            ExprKind::Apply { callee, arguments } => {
                let mut info = self.infer_expr(callee, env, ambient);
                let mut current = info.ty.clone();
                for argument in arguments.iter() {
                    let argument_info = self.infer_expr(*argument, env, ambient);
                    let argument_ty = argument_info.ty.clone();
                    info.merge(argument_info);
                    current = match (current.as_ref(), argument_ty.as_ref()) {
                        (Some(callee_ty), Some(argument_ty)) => {
                            self.apply_function(callee_ty, argument_ty)
                        }
                        _ => None,
                    };
                }
                info.ty = current;
                info
            }
            ExprKind::Unary { operator, expr } => {
                let mut info = self.infer_expr(expr, env, ambient);
                info.ty = match (operator, info.ty.as_ref()) {
                    (crate::hir::UnaryOperator::Not, Some(ty)) if ty.is_bool() => {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let mut info = self.infer_expr(left, env, ambient);
                let left_ty = info.ty.clone();
                let right_info = self.infer_expr(right, env, ambient);
                let right_ty = right_info.ty.clone();
                info.merge(right_info);
                info.ty = match (left_ty.as_ref(), right_ty.as_ref(), operator) {
                    (Some(left), Some(right), crate::hir::BinaryOperator::And)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Or)
                        if left.is_bool() && right.is_bool() =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::GreaterThan)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::LessThan)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Add)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Subtract)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(left.clone())
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Equals)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::NotEquals)
                        if left.same_shape(right) =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Pipe(pipe) => GateExprInfo {
                ty: self.infer_pipe_result(&pipe, env),
                ..GateExprInfo::default()
            },
            ExprKind::Cluster(_) | ExprKind::Markup(_) => GateExprInfo::default(),
        }
    }

    fn infer_name(&mut self, reference: &TermReference, env: &GateExprEnv) -> GateExprInfo {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => GateExprInfo::default(),
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                let ty = env.locals.get(binding).cloned();
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                let ty = self.item_value_type(*item_id);
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Import(_)) => GateExprInfo::default(),
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                let ty = match builtin {
                    crate::hir::BuiltinTerm::True | crate::hir::BuiltinTerm::False => {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    crate::hir::BuiltinTerm::None
                    | crate::hir::BuiltinTerm::Some
                    | crate::hir::BuiltinTerm::Ok
                    | crate::hir::BuiltinTerm::Err
                    | crate::hir::BuiltinTerm::Valid
                    | crate::hir::BuiltinTerm::Invalid => None,
                };
                GateExprInfo {
                    ty,
                    ..GateExprInfo::default()
                }
            }
        }
    }

    pub(crate) fn infer_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let ambient = subject.gate_payload().clone();
        let mut info = self.infer_expr(expr_id, env, Some(&ambient));
        if let Some(GateType::Arrow { parameter, result }) = info.ty.clone() {
            if parameter.same_shape(&ambient) {
                info.ty = Some(*result);
            }
        }
        info
    }

    pub(crate) fn infer_transform_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let body = self.infer_pipe_body(expr_id, env, subject);
        let body_ty = body.ty?;
        Some(match subject {
            GateType::Signal(_) => GateType::Signal(Box::new(body_ty)),
            _ => body_ty,
        })
    }

    fn infer_pipe_result(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
    ) -> Option<GateType> {
        let mut current = self.infer_expr(pipe.head, env, None).ty?;
        for stage in pipe.stages.iter() {
            match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    current = self.infer_transform_stage(*expr, env, &current)?;
                }
                PipeStageKind::Tap { .. } => {}
                PipeStageKind::Gate { expr } => {
                    let predicate = self.infer_pipe_body(*expr, env, &current);
                    if !predicate.issues.is_empty()
                        || predicate.contains_signal
                        || predicate.ty.as_ref().is_some_and(GateType::is_signal)
                    {
                        return None;
                    }
                    if let Some(predicate_ty) = predicate.ty.as_ref() {
                        if !predicate_ty.is_bool() {
                            return None;
                        }
                    }
                    current =
                        self.apply_gate_plan(GatePlanner::plan(current.gate_carrier()), &current);
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Map { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::FanIn { .. }
                | PipeStageKind::Truthy { .. }
                | PipeStageKind::Falsy { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => return None,
            }
        }
        Some(current)
    }

    fn project_type(&self, subject: &GateType, path: &NamePath) -> Result<GateType, GateIssue> {
        let mut current = subject.clone();
        for segment in path.segments().iter() {
            let GateType::Record(fields) = &current else {
                return Err(GateIssue::InvalidProjection {
                    span: path.span(),
                    path: name_path_text(path),
                    subject: current.to_string(),
                });
            };
            let Some(field) = fields.iter().find(|field| field.name == segment.text()) else {
                return Err(GateIssue::UnknownField {
                    span: path.span(),
                    path: name_path_text(path),
                    subject: current.to_string(),
                });
            };
            current = field.ty.clone();
        }
        Ok(current)
    }

    fn apply_function(&self, callee: &GateType, argument: &GateType) -> Option<GateType> {
        let GateType::Arrow { parameter, result } = callee else {
            return None;
        };
        parameter
            .same_shape(argument)
            .then(|| result.as_ref().clone())
    }
}

fn is_numeric_gate_type(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(
            BuiltinType::Int | BuiltinType::Float | BuiltinType::Decimal | BuiltinType::BigInt
        )
    )
}

fn name_path_text(path: &NamePath) -> String {
    format!(
        ".{}",
        path.segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".")
    )
}

fn provider_key_text(path: &NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}

fn type_argument_phrase(count: usize) -> String {
    format!("{count} type argument{}", if count == 1 { "" } else { "s" })
}

pub(crate) fn walk_expr_tree(
    module: &Module,
    root: ExprId,
    mut on_expr: impl FnMut(ExprId, &crate::hir::Expr, bool),
) {
    let mut work = vec![ExprWalkWork::Expr {
        expr: root,
        is_root: true,
    }];
    while let Some(task) = work.pop() {
        match task {
            ExprWalkWork::Expr {
                expr: expr_id,
                is_root,
            } => {
                let expr = module.exprs()[expr_id].clone();
                on_expr(expr_id, &expr, is_root);
                match expr.kind {
                    ExprKind::Name(_)
                    | ExprKind::Integer(_)
                    | ExprKind::SuffixedInteger(_)
                    | ExprKind::Regex(_) => {}
                    ExprKind::Text(text) => {
                        for segment in text.segments.into_iter().rev() {
                            if let TextSegment::Interpolation(interpolation) = segment {
                                work.push(ExprWalkWork::Expr {
                                    expr: interpolation.expr,
                                    is_root: false,
                                });
                            }
                        }
                    }
                    ExprKind::Tuple(elements) => {
                        for element in elements.iter().rev() {
                            work.push(ExprWalkWork::Expr {
                                expr: *element,
                                is_root: false,
                            });
                        }
                    }
                    ExprKind::List(elements) => {
                        for element in elements.into_iter().rev() {
                            work.push(ExprWalkWork::Expr {
                                expr: element,
                                is_root: false,
                            });
                        }
                    }
                    ExprKind::Record(record) => {
                        for field in record.fields.into_iter().rev() {
                            work.push(ExprWalkWork::Expr {
                                expr: field.value,
                                is_root: false,
                            });
                        }
                    }
                    ExprKind::Projection {
                        base: crate::hir::ProjectionBase::Expr(base),
                        ..
                    } => work.push(ExprWalkWork::Expr {
                        expr: base,
                        is_root: false,
                    }),
                    ExprKind::Projection { .. } => {}
                    ExprKind::Apply { callee, arguments } => {
                        for argument in arguments.iter().rev() {
                            work.push(ExprWalkWork::Expr {
                                expr: *argument,
                                is_root: false,
                            });
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: callee,
                            is_root: false,
                        });
                    }
                    ExprKind::Unary { expr, .. } => work.push(ExprWalkWork::Expr {
                        expr,
                        is_root: false,
                    }),
                    ExprKind::Binary { left, right, .. } => {
                        work.push(ExprWalkWork::Expr {
                            expr: right,
                            is_root: false,
                        });
                        work.push(ExprWalkWork::Expr {
                            expr: left,
                            is_root: false,
                        });
                    }
                    ExprKind::Pipe(pipe) => {
                        for stage in pipe.stages.iter().rev() {
                            match &stage.kind {
                                PipeStageKind::Transform { expr }
                                | PipeStageKind::Gate { expr }
                                | PipeStageKind::Map { expr }
                                | PipeStageKind::Apply { expr }
                                | PipeStageKind::Tap { expr }
                                | PipeStageKind::FanIn { expr }
                                | PipeStageKind::Truthy { expr }
                                | PipeStageKind::Falsy { expr }
                                | PipeStageKind::RecurStart { expr }
                                | PipeStageKind::RecurStep { expr } => {
                                    work.push(ExprWalkWork::Expr {
                                        expr: *expr,
                                        is_root: false,
                                    });
                                }
                                PipeStageKind::Case { body, .. } => {
                                    work.push(ExprWalkWork::Expr {
                                        expr: *body,
                                        is_root: false,
                                    });
                                }
                            }
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: pipe.head,
                            is_root: false,
                        });
                    }
                    ExprKind::Cluster(cluster_id) => {
                        let cluster = module.clusters()[cluster_id].clone();
                        let spine = cluster.normalized_spine();
                        for member in spine.apply_arguments() {
                            work.push(ExprWalkWork::Expr {
                                expr: member,
                                is_root: false,
                            });
                        }
                        if let ApplicativeSpineHead::Expr(finalizer) = spine.pure_head() {
                            work.push(ExprWalkWork::Expr {
                                expr: finalizer,
                                is_root: false,
                            });
                        }
                    }
                    ExprKind::Markup(node_id) => work.push(ExprWalkWork::Markup(node_id)),
                }
            }
            ExprWalkWork::Markup(node_id) => {
                let node = module.markup_nodes()[node_id].clone();
                match node.kind {
                    MarkupNodeKind::Element(element) => {
                        for child in element.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                        for attribute in element.attributes.into_iter().rev() {
                            match attribute.value {
                                MarkupAttributeValue::Expr(expr) => {
                                    work.push(ExprWalkWork::Expr {
                                        expr,
                                        is_root: false,
                                    });
                                }
                                MarkupAttributeValue::Text(text) => {
                                    for segment in text.segments.into_iter().rev() {
                                        if let TextSegment::Interpolation(interpolation) = segment {
                                            work.push(ExprWalkWork::Expr {
                                                expr: interpolation.expr,
                                                is_root: false,
                                            });
                                        }
                                    }
                                }
                                MarkupAttributeValue::ImplicitTrue => {}
                            }
                        }
                    }
                    MarkupNodeKind::Control(control_id) => {
                        work.push(ExprWalkWork::Control(control_id));
                    }
                }
            }
            ExprWalkWork::Control(control_id) => {
                let control = module.control_nodes()[control_id].clone();
                match control {
                    ControlNode::Show(node) => {
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                        if let Some(keep_mounted) = node.keep_mounted {
                            work.push(ExprWalkWork::Expr {
                                expr: keep_mounted,
                                is_root: false,
                            });
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: node.when,
                            is_root: false,
                        });
                    }
                    ControlNode::Each(node) => {
                        if let Some(empty) = node.empty {
                            work.push(ExprWalkWork::Control(empty));
                        }
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                        if let Some(key) = node.key {
                            work.push(ExprWalkWork::Expr {
                                expr: key,
                                is_root: false,
                            });
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: node.collection,
                            is_root: false,
                        });
                    }
                    ControlNode::Empty(node) => {
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                    }
                    ControlNode::Match(node) => {
                        for case in node.cases.iter().rev() {
                            work.push(ExprWalkWork::Control(*case));
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: node.scrutinee,
                            is_root: false,
                        });
                    }
                    ControlNode::Case(node) => {
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                    }
                    ControlNode::Fragment(node) => {
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                    }
                    ControlNode::With(node) => {
                        for child in node.children.into_iter().rev() {
                            work.push(ExprWalkWork::Markup(child));
                        }
                        work.push(ExprWalkWork::Expr {
                            expr: node.value,
                            is_root: false,
                        });
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum KindBuildFrame {
    Enter(TypeId),
    Exit(TypeId),
}

#[cfg(test)]
mod tests {
    use aivi_base::{ByteIndex, DiagnosticCode, FileId, SourceSpan, Span};

    use crate::{
        ApplicativeCluster, ClusterFinalizer, ClusterPresentation, ControlNode, Expr, ExprKind,
        IntegerLiteral, Item, ItemHeader, MarkupNode, MarkupNodeKind, Module, Name, NamePath,
        NonEmpty, Pattern, PatternKind, PipeExpr, PipeStage, PipeStageKind, RecordExpr,
        ShowControl, TermReference, ValidationMode,
    };

    use super::validate_module;

    fn span(file: u32, start: u32, end: u32) -> SourceSpan {
        SourceSpan::new(
            FileId::new(file),
            Span::new(ByteIndex::new(start), ByteIndex::new(end)),
        )
    }

    #[test]
    fn name_path_rejects_mixed_files() {
        let first = Name::new("app", span(0, 0, 3)).expect("valid name");
        let second = Name::new("ui", span(1, 4, 6)).expect("valid name");

        let error = NamePath::from_vec(vec![first, second]).expect_err("files differ");
        assert!(matches!(error, crate::NamePathError::MixedFiles { .. }));
    }

    #[test]
    fn module_validation_reports_missing_references() {
        let module_span = span(0, 0, 10);
        let mut module = Module::new(FileId::new(0));

        let item = Item::Value(crate::ValueItem {
            header: ItemHeader {
                span: module_span,
                decorators: Vec::new(),
            },
            name: Name::new("answer", span(0, 0, 6)).expect("valid name"),
            annotation: None,
            body: crate::ExprId::from_raw(99),
        });
        let _ = module.push_item(item).expect("item allocation should fit");

        let report = validate_module(&module, ValidationMode::Structural);
        assert!(!report.is_ok());
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message.contains("missing expression 99"))
        );
    }

    #[test]
    fn require_resolved_mode_rejects_unresolved_names() {
        let module_span = span(0, 0, 12);
        let mut module = Module::new(FileId::new(0));

        let path = NamePath::from_vec(vec![Name::new("value", span(0, 0, 5)).expect("valid name")])
            .expect("single-segment path");
        let expr = module
            .alloc_expr(Expr {
                span: module_span,
                kind: ExprKind::Name(TermReference::unresolved(path)),
            })
            .expect("expression allocation should fit");

        let item = Item::Value(crate::ValueItem {
            header: ItemHeader {
                span: module_span,
                decorators: Vec::new(),
            },
            name: Name::new("result", span(0, 0, 6)).expect("valid name"),
            annotation: None,
            body: expr,
        });
        let _ = module.push_item(item).expect("item allocation should fit");

        let report = validate_module(&module, ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(
            |diagnostic| diagnostic.code == Some(DiagnosticCode::new("hir", "unresolved-name"))
        ));
    }

    #[test]
    fn recurrence_suffix_reports_malformed_manual_hir() {
        let pipe_span = span(0, 0, 12);
        let mut module = Module::new(FileId::new(0));

        let head = module
            .alloc_expr(Expr {
                span: span(0, 0, 1),
                kind: ExprKind::Integer(IntegerLiteral { raw: "0".into() }),
            })
            .expect("expression allocation should fit");
        let start_expr = module
            .alloc_expr(Expr {
                span: span(0, 4, 5),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let follow_expr = module
            .alloc_expr(Expr {
                span: span(0, 8, 9),
                kind: ExprKind::Integer(IntegerLiteral { raw: "2".into() }),
            })
            .expect("expression allocation should fit");
        let pipe = module
            .alloc_expr(Expr {
                span: pipe_span,
                kind: ExprKind::Pipe(PipeExpr {
                    head,
                    stages: NonEmpty::new(
                        PipeStage {
                            span: span(0, 2, 5),
                            kind: PipeStageKind::RecurStart { expr: start_expr },
                        },
                        vec![PipeStage {
                            span: span(0, 6, 9),
                            kind: PipeStageKind::Transform { expr: follow_expr },
                        }],
                    ),
                }),
            })
            .expect("expression allocation should fit");

        let _ = module
            .push_item(Item::Value(crate::ValueItem {
                header: ItemHeader {
                    span: pipe_span,
                    decorators: Vec::new(),
                },
                name: Name::new("broken", span(0, 0, 6)).expect("valid name"),
                annotation: None,
                body: pipe,
            }))
            .expect("item allocation should fit");

        let ExprKind::Pipe(pipe) = &module.exprs()[pipe].kind else {
            panic!("expected manual test expression to stay a pipe");
        };
        assert!(
            matches!(
                pipe.recurrence_suffix(),
                Err(crate::PipeRecurrenceShapeError::MissingStep { .. })
            ),
            "manual malformed HIR should report a missing recurrence step, got {:?}",
            pipe.recurrence_suffix()
        );
    }

    #[test]
    fn validation_rejects_branch_only_control_nodes_as_markup_roots() {
        let node_span = span(0, 0, 8);
        let mut module = Module::new(FileId::new(0));

        let pattern = module
            .alloc_pattern(Pattern {
                span: node_span,
                kind: PatternKind::Wildcard,
            })
            .expect("pattern allocation should fit");
        let case = module
            .alloc_control_node(ControlNode::Case(crate::CaseControl {
                span: node_span,
                pattern,
                children: Vec::new(),
            }))
            .expect("control node allocation should fit");
        let markup = module
            .alloc_markup_node(MarkupNode {
                span: node_span,
                kind: MarkupNodeKind::Control(case),
            })
            .expect("markup allocation should fit");
        let expr = module
            .alloc_expr(Expr {
                span: node_span,
                kind: ExprKind::Markup(markup),
            })
            .expect("expression allocation should fit");
        let _ = module
            .push_item(Item::Value(crate::ValueItem {
                header: ItemHeader {
                    span: node_span,
                    decorators: Vec::new(),
                },
                name: Name::new("view", span(0, 0, 4)).expect("valid name"),
                annotation: None,
                body: expr,
            }))
            .expect("item allocation should fit");

        let report = validate_module(&module, ValidationMode::Structural);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message.contains("branch-only control node kind"))
        );
    }

    #[test]
    fn structural_validation_accepts_explicit_cluster_and_show_nodes() {
        let shared_span = span(0, 0, 10);
        let mut module = Module::new(FileId::new(0));

        let bool_name = Name::new("flag", span(0, 0, 4)).expect("valid name");
        let bool_path = NamePath::from_vec(vec![bool_name.clone()]).expect("single segment path");
        let condition = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Name(TermReference::unresolved(bool_path)),
            })
            .expect("expression allocation should fit");

        let child_markup = module
            .alloc_markup_node(MarkupNode {
                span: shared_span,
                kind: MarkupNodeKind::Element(crate::MarkupElement {
                    name: NamePath::from_vec(vec![
                        Name::new("Label", span(0, 0, 5)).expect("valid name"),
                    ])
                    .expect("single segment path"),
                    attributes: Vec::new(),
                    children: Vec::new(),
                    close_name: None,
                    self_closing: true,
                }),
            })
            .expect("markup allocation should fit");

        let show = module
            .alloc_control_node(ControlNode::Show(ShowControl {
                span: shared_span,
                when: condition,
                keep_mounted: None,
                children: vec![child_markup],
            }))
            .expect("control node allocation should fit");
        let markup = module
            .alloc_markup_node(MarkupNode {
                span: shared_span,
                kind: MarkupNodeKind::Control(show),
            })
            .expect("markup allocation should fit");
        let markup_expr = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Markup(markup),
            })
            .expect("expression allocation should fit");

        let left = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let right = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Integer(crate::IntegerLiteral { raw: "2".into() }),
            })
            .expect("expression allocation should fit");
        let cluster = module
            .alloc_cluster(ApplicativeCluster {
                span: shared_span,
                presentation: ClusterPresentation::Leading,
                members: crate::AtLeastTwo::new(left, right, Vec::new()),
                finalizer: ClusterFinalizer::ImplicitTuple,
            })
            .expect("cluster allocation should fit");
        let cluster_expr = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Cluster(cluster),
            })
            .expect("expression allocation should fit");

        let record_expr = module
            .alloc_expr(Expr {
                span: shared_span,
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![crate::RecordExprField {
                        span: shared_span,
                        label: Name::new("view", span(0, 0, 4)).expect("valid name"),
                        value: markup_expr,
                        surface: crate::RecordFieldSurface::Explicit,
                    }],
                }),
            })
            .expect("expression allocation should fit");

        let _ = module
            .push_item(Item::Value(crate::ValueItem {
                header: ItemHeader {
                    span: shared_span,
                    decorators: Vec::new(),
                },
                name: Name::new("ui", span(0, 0, 2)).expect("valid name"),
                annotation: None,
                body: record_expr,
            }))
            .expect("item allocation should fit");
        let _ = module
            .push_item(Item::Value(crate::ValueItem {
                header: ItemHeader {
                    span: shared_span,
                    decorators: Vec::new(),
                },
                name: Name::new("pair", span(0, 0, 4)).expect("valid name"),
                annotation: None,
                body: cluster_expr,
            }))
            .expect("item allocation should fit");

        let report = validate_module(&module, ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "unexpected diagnostics: {:?}",
            report.diagnostics()
        );
    }
}
