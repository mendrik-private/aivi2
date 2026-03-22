use std::collections::HashMap;

use aivi_base::{Diagnostic, DiagnosticCode, DiagnosticLabel, SourceSpan};
use aivi_typing::{
    Kind, KindCheckError, KindCheckErrorKind, KindChecker, KindExprId,
    KindParameterId as TypingKindParameterId, KindRecordField, KindStore,
};

use crate::{
    arena::{Arena, ArenaId},
    hir::{
        BuiltinType, ClusterFinalizer, ControlNode, ControlNodeKind, DecoratorPayload,
        DomainMemberKind, ExprKind, Item, LiteralSuffixResolution, MarkupAttributeValue,
        MarkupNodeKind, Module, Name, NamePath, PatternKind, PipeStageKind, ResolutionState,
        SourceMetadata, TermReference, TermResolution, TextLiteral, TextSegment, TypeKind,
        TypeReference, TypeResolution,
    },
    ids::{
        BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
        PatternId, TypeId, TypeParameterId,
    },
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
            for member in cluster.members.iter() {
                self.require_expr(cluster.span, "cluster", "cluster member", *member);
            }
            if let ClusterFinalizer::Explicit(finalizer) = cluster.finalizer {
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
enum KindBuildFrame {
    Enter(TypeId),
    Exit(TypeId),
}

#[cfg(test)]
mod tests {
    use aivi_base::{ByteIndex, DiagnosticCode, FileId, SourceSpan, Span};

    use crate::{
        ApplicativeCluster, ClusterFinalizer, ClusterPresentation, ControlNode, Expr, ExprKind,
        Item, ItemHeader, MarkupNode, MarkupNodeKind, Module, Name, NamePath, Pattern, PatternKind,
        RecordExpr, ShowControl, TermReference, ValidationMode,
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
