use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt,
};

use aivi_base::{ByteIndex, Diagnostic, DiagnosticCode, DiagnosticLabel, SourceSpan, Span};
use aivi_typing::{
    BuiltinSourceProvider, BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext,
    FanoutCarrier, FanoutPlan, FanoutPlanner, FanoutResultKind, FanoutStageKind, GateCarrier,
    GatePlanner, GateResultKind, Kind, KindCheckError, KindCheckErrorKind, KindChecker, KindExprId,
    KindParameterId as TypingKindParameterId, KindRecordField, KindStore, NonSourceWakeupCause,
    RecurrencePlanner, RecurrenceTargetEvidence, RecurrenceWakeupKind, RecurrenceWakeupPlanner,
    SourceContractType, SourceRecurrenceWakeupContext, SourceTypeParameter,
    builtin_source_option_wakeup_cause,
};
use regex_syntax::{
    Error as RegexSyntaxError, ParserBuilder as RegexParserBuilder, ast::Span as RegexSpan,
};

use crate::{
    arena::{Arena, ArenaId},
    domain_operator_elaboration::{binary_operator_text, select_domain_binary_operator},
    hir::{
        ApplicativeSpineHead, BuiltinTerm, BuiltinType, ClassMemberResolution, ControlNode,
        ControlNodeKind, CustomSourceRecurrenceWakeup, DecoratorPayload, DomainMemberKind,
        DomainMemberResolution, ExportResolution, ExprKind, ImportBindingMetadata,
        ImportBindingResolution, ImportValueType, Item, LiteralSuffixResolution,
        MarkupAttributeValue, MarkupNodeKind, Module, Name, NamePath, PatternKind, PipeStage,
        PipeStageKind, RecurrenceWakeupDecoratorKind, ResolutionState, SignalItem, SourceDecorator,
        SourceMetadata, SourceProviderRef, TermReference, TermResolution, TextLiteral, TextSegment,
        TypeItemBody, TypeKind, TypeReference, TypeResolution,
    },
    ids::{
        BindingId, ClusterId, ControlNodeId, DecoratorId, ExprId, ImportId, ItemId, MarkupNodeId,
        PatternId, TypeId, TypeParameterId,
    },
    source_contract_resolution::{
        ResolvedSourceContractType, ResolvedSourceTypeConstructor,
        SourceContractResolutionErrorKind, SourceContractTypeResolver,
    },
    typecheck::{TypeConstraint, expression_matches, typecheck_module},
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

const REGEX_LITERAL_PREFIX_LEN: usize = 3;
const REGEX_NEST_LIMIT: u32 = 256;

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
        self.validate_instance_items();
        self.validate_source_contract_types();
        self.validate_expression_types();
        self.validate_constructor_arity();
        self.validate_pipe_semantics();
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
            match (import.resolution, &import.metadata) {
                (ImportBindingResolution::Resolved, ImportBindingMetadata::Unknown)
                | (
                    ImportBindingResolution::UnknownModule
                    | ImportBindingResolution::MissingExport
                    | ImportBindingResolution::Cycle,
                    ImportBindingMetadata::Value { .. }
                    | ImportBindingMetadata::IntrinsicValue { .. }
                    | ImportBindingMetadata::OpaqueValue
                    | ImportBindingMetadata::TypeConstructor { .. }
                    | ImportBindingMetadata::BuiltinType(_)
                    | ImportBindingMetadata::BuiltinTerm(_)
                    | ImportBindingMetadata::AmbientType
                    | ImportBindingMetadata::Bundle(_),
                ) => {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "import binding resolution and metadata are inconsistent",
                        )
                        .with_code(code("invalid-import-resolution"))
                        .with_primary_label(
                            import.span,
                            "resolved imports must carry metadata, while blocked imports must stay unknown",
                        ),
                    );
                }
                _ => {}
            }
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
                DecoratorPayload::RecurrenceWakeup(wakeup) => {
                    self.require_expr(
                        decorator.span,
                        "decorator",
                        "recurrence wakeup witness",
                        wakeup.witness,
                    );
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
                PatternKind::List { elements, rest } => {
                    for element in elements {
                        self.require_pattern(
                            pattern.span,
                            "pattern",
                            "list element pattern",
                            *element,
                        );
                    }
                    if let Some(rest) = rest {
                        self.require_pattern(pattern.span, "pattern", "list rest pattern", *rest);
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
                ExprKind::Integer(_)
                | ExprKind::Float(_)
                | ExprKind::Decimal(_)
                | ExprKind::BigInt(_) => {}
                ExprKind::Regex(regex) => self.check_regex_literal(expr.span, regex),
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
                ExprKind::Map(map) => {
                    for entry in &map.entries {
                        self.check_span("map entry", entry.span);
                        self.require_expr(entry.span, "map entry", "entry key", entry.key);
                        self.require_expr(entry.span, "map entry", "entry value", entry.value);
                    }
                }
                ExprKind::Set(elements) => {
                    for element in elements {
                        self.require_expr(expr.span, "expression", "set element", *element);
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
        for (item_id, item) in self.module.items().iter() {
            if self.module.ambient_items().contains(&item_id) {
                continue;
            }
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
                    for parameter in &item.type_parameters {
                        self.require_type_parameter(
                            item.header.span,
                            "function item",
                            "type parameter",
                            *parameter,
                        );
                    }
                    for constraint in &item.context {
                        self.require_type(
                            item.header.span,
                            "function item",
                            "signature constraint",
                            *constraint,
                        );
                    }
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
                        for parameter in &member.type_parameters {
                            self.require_type_parameter(
                                member.span,
                                "class member",
                                "type parameter",
                                *parameter,
                            );
                        }
                        for constraint in &member.context {
                            self.require_type(
                                member.span,
                                "class member",
                                "signature constraint",
                                *constraint,
                            );
                        }
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
                Item::SourceProviderContract(item) => {
                    for argument in &item.contract.arguments {
                        self.check_span("provider contract argument", argument.span);
                        self.check_name(&argument.name);
                        self.require_type(
                            argument.span,
                            "provider contract argument",
                            "annotation",
                            argument.annotation,
                        );
                    }
                    for option in &item.contract.options {
                        self.check_span("provider contract option", option.span);
                        self.check_name(&option.name);
                        self.require_type(
                            option.span,
                            "provider contract option",
                            "annotation",
                            option.annotation,
                        );
                    }
                }
                Item::Instance(item) => {
                    self.check_type_reference(&item.class);
                    for parameter in &item.type_parameters {
                        self.require_type_parameter(
                            item.header.span,
                            "instance item",
                            "type parameter",
                            *parameter,
                        );
                    }
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
                        for parameter in &member.parameters {
                            self.check_span("instance member parameter", parameter.span);
                            self.require_binding(
                                member.span,
                                "instance member",
                                "parameter binding",
                                parameter.binding,
                            );
                            if let Some(annotation) = parameter.annotation {
                                self.require_type(
                                    parameter.span,
                                    "instance member parameter",
                                    "annotation",
                                    annotation,
                                );
                            }
                        }
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
                            if let ExportResolution::Item(item_id) = resolved {
                                this.require_item(
                                    item.header.span,
                                    "export item",
                                    "resolved target",
                                    *item_id,
                                );
                            }
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
            .root_items()
            .iter()
            .map(|item_id| self.module.items()[*item_id].clone())
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
                    let parameters = item.type_parameters.clone();
                    for constraint in &item.context {
                        self.check_expected_type_kind(
                            *constraint,
                            &parameters,
                            "function signature constraint",
                        );
                    }
                    if let Some(annotation) = item.annotation {
                        self.check_expected_type_kind(
                            annotation,
                            &parameters,
                            "function annotation",
                        );
                    }
                    for parameter in &item.parameters {
                        if let Some(annotation) = parameter.annotation {
                            self.check_expected_type_kind(
                                annotation,
                                &parameters,
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
                        let mut member_parameters = parameters.clone();
                        member_parameters.extend(member.type_parameters.iter().copied());
                        for constraint in &member.context {
                            self.check_expected_type_kind(
                                *constraint,
                                &member_parameters,
                                "class member constraint",
                            );
                        }
                        self.check_expected_type_kind(
                            member.annotation,
                            &member_parameters,
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
                Item::SourceProviderContract(item) => {
                    for argument in &item.contract.arguments {
                        self.check_expected_type_kind(
                            argument.annotation,
                            &[],
                            "provider contract argument annotation",
                        );
                    }
                    for option in &item.contract.options {
                        self.check_expected_type_kind(
                            option.annotation,
                            &[],
                            "provider contract option annotation",
                        );
                    }
                }
                Item::Instance(item) => {
                    let parameters = item.type_parameters.clone();
                    self.check_type_reference_kind(
                        &item.class,
                        &parameters,
                        Kind::constructor(item.arguments.len()),
                        "instance class head",
                    );
                    for argument in item.arguments.iter() {
                        self.check_expected_type_kind(*argument, &parameters, "instance argument");
                    }
                    for context in &item.context {
                        self.check_expected_type_kind(*context, &parameters, "instance context");
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

    fn validate_instance_items(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let instances = self
            .module
            .items()
            .iter()
            .filter_map(|(_, item)| match item {
                Item::Instance(instance) => Some(instance.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut seen_instances = Vec::<(ItemId, TypeId, SourceSpan)>::new();

        for item in instances {
            let Some(class_item_id) = self.instance_class_item_id(&item) else {
                continue;
            };
            let Item::Class(class_item) = &self.module.items()[class_item_id] else {
                unreachable!("instance class helper should only return class items");
            };
            let argument = *item.arguments.first();
            if let Some((_, _, previous_span)) =
                seen_instances
                    .iter()
                    .find(|(seen_class, seen_argument, _)| {
                        *seen_class == class_item_id
                            && self.same_instance_argument_type(*seen_argument, argument)
                    })
            {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "duplicate instance for class `{}`",
                        class_item.name.text()
                    ))
                    .with_code(code("duplicate-instance"))
                    .with_primary_label(
                        item.header.span,
                        "this instance head duplicates an earlier same-module instance",
                    )
                    .with_secondary_label(*previous_span, "previous instance here"),
                );
            }
            seen_instances.push((class_item_id, argument, item.header.span));

            let mut seen_members = HashMap::<String, SourceSpan>::new();
            for member in &item.members {
                let name = member.name.text().to_owned();
                if let Some(previous_span) = seen_members.insert(name.clone(), member.span) {
                    self.diagnostics.push(
                        Diagnostic::error(format!("duplicate instance member `{name}`"))
                            .with_code(code("duplicate-instance-member"))
                            .with_primary_label(
                                member.span,
                                "this instance member repeats an earlier binding",
                            )
                            .with_secondary_label(previous_span, "previous instance member here"),
                    );
                }
                if !class_item
                    .members
                    .iter()
                    .any(|class_member| class_member.name.text() == name)
                {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "instance member `{name}` is not declared by class `{}`",
                            class_item.name.text()
                        ))
                        .with_code(code("unknown-instance-member"))
                        .with_primary_label(
                            member.span,
                            "remove this member or add it to the class declaration first",
                        ),
                    );
                }
            }

            let missing_members = class_item
                .members
                .iter()
                .filter(|class_member| !seen_members.contains_key(class_member.name.text()))
                .map(|class_member| format!("`{}`", class_member.name.text()))
                .collect::<Vec<_>>();
            if !missing_members.is_empty() {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "instance for `{}` is missing {}",
                        class_item.name.text(),
                        missing_members.join(", ")
                    ))
                    .with_code(code("missing-instance-member"))
                    .with_primary_label(
                        item.header.span,
                        "every class member must be implemented exactly once",
                    ),
                );
            }
        }
    }

    fn validate_source_contract_types(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let provider_contracts = self
            .module
            .items()
            .iter()
            .filter_map(|(_, item)| match item {
                Item::SourceProviderContract(item) => Some(item.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let module = self.module;
        let decorators = self
            .module
            .items()
            .iter()
            .flat_map(|(_, item)| {
                let source_metadata = match item {
                    Item::Signal(signal) => signal.source_metadata.clone(),
                    _ => None,
                };
                item.decorators().iter().filter_map(move |decorator_id| {
                    let decorator = &module.decorators()[*decorator_id];
                    let DecoratorPayload::Source(source) = &decorator.payload else {
                        return None;
                    };
                    Some((decorator.span, source.clone(), source_metadata.clone()))
                })
            })
            .collect::<Vec<_>>();
        let mut resolver = SourceContractTypeResolver::new(self.module);
        let mut typing = GateTypeContext::new(self.module);

        for item in &provider_contracts {
            self.validate_custom_source_contract_schema_types(item, &mut typing);
        }

        for (span, source, source_metadata) in decorators {
            self.validate_source_decorator_contract_types(
                span,
                &source,
                source_metadata.as_ref(),
                &mut resolver,
                &mut typing,
            );
        }
    }

    fn validate_source_decorator_contract_types(
        &mut self,
        source_span: SourceSpan,
        source: &crate::hir::SourceDecorator,
        source_metadata: Option<&SourceMetadata>,
        resolver: &mut SourceContractTypeResolver<'_>,
        typing: &mut GateTypeContext<'_>,
    ) {
        let provider = source_metadata
            .map(|metadata| metadata.provider.clone())
            .unwrap_or_else(|| SourceProviderRef::from_path(source.provider.as_ref()));
        match provider {
            SourceProviderRef::Builtin(provider) => {
                self.validate_builtin_source_decorator_contract_types(
                    source, provider, resolver, typing,
                );
            }
            SourceProviderRef::Custom(_) => {
                let Some(source_metadata) = source_metadata else {
                    return;
                };
                let Some(contract) = source_metadata.custom_contract.as_ref() else {
                    return;
                };
                self.validate_custom_source_decorator_contract_types(
                    source_span,
                    &provider,
                    source,
                    contract,
                    typing,
                );
            }
            SourceProviderRef::Missing | SourceProviderRef::InvalidShape(_) => {}
        }
    }

    fn validate_builtin_source_decorator_contract_types(
        &mut self,
        source: &crate::hir::SourceDecorator,
        provider: BuiltinSourceProvider,
        resolver: &mut SourceContractTypeResolver<'_>,
        typing: &mut GateTypeContext<'_>,
    ) {
        let Some(options) = source.options else {
            return;
        };
        let ExprKind::Record(record) = &self.module.exprs()[options].kind else {
            return;
        };
        let mut pending = Vec::new();

        for field in &record.fields {
            let Some(option) = provider.contract().option(field.label.text()) else {
                continue;
            };
            match resolver.resolve(option.ty()) {
                Ok(expected) => {
                    let Some(expected) =
                        SourceOptionExpectedType::from_resolved(self.module, &expected)
                    else {
                        continue;
                    };
                    pending.push(PendingSourceOptionValue {
                        field: field.clone(),
                        expected_surface: option.ty().to_string(),
                        expected,
                    });
                }
                Err(error) => self.emit_source_contract_resolution_error(
                    field.span,
                    provider.key(),
                    field.label.text(),
                    option.ty(),
                    error.kind(),
                ),
            }
        }

        let mut resolved = Vec::new();
        let mut bindings = SourceOptionTypeBindings::default();
        while !pending.is_empty() {
            let mut progress = false;
            let mut remaining = Vec::new();
            for pending_option in pending {
                let mut trial_bindings = bindings.clone();
                match self.check_source_option_expr(
                    pending_option.field.value,
                    &pending_option.expected,
                    typing,
                    &mut trial_bindings,
                ) {
                    SourceOptionTypeCheck::Match => {
                        bindings = trial_bindings;
                        resolved.push(pending_option);
                        progress = true;
                    }
                    SourceOptionTypeCheck::Mismatch(mismatch) => {
                        self.emit_source_option_value_mismatch(
                            &pending_option.field,
                            provider.key(),
                            &pending_option.expected_surface,
                            mismatch,
                        );
                        progress = true;
                    }
                    SourceOptionTypeCheck::Unknown => remaining.push(pending_option),
                }
            }
            if !progress {
                pending = remaining;
                break;
            }
            pending = remaining;
        }

        for option in resolved.iter().chain(&pending) {
            self.emit_source_option_unbound_contract_parameter(
                &option.field,
                provider.key(),
                &option.expected_surface,
                &option.expected,
                &bindings,
            );
        }
    }

    fn validate_custom_source_contract_schema_types(
        &mut self,
        item: &crate::SourceProviderContractItem,
        typing: &mut GateTypeContext<'_>,
    ) {
        let provider_key = item.provider.key().unwrap_or("<provider>");
        for argument in &item.contract.arguments {
            self.validate_custom_source_contract_schema_type(
                provider_key,
                "argument",
                argument.name.text(),
                argument.span,
                argument.annotation,
                typing,
            );
        }
        for option in &item.contract.options {
            self.validate_custom_source_contract_schema_type(
                provider_key,
                "option",
                option.name.text(),
                option.span,
                option.annotation,
                typing,
            );
        }
    }

    fn validate_custom_source_contract_schema_type(
        &mut self,
        provider_key: &str,
        schema_kind: &str,
        schema_name: &str,
        span: SourceSpan,
        annotation: TypeId,
        typing: &mut GateTypeContext<'_>,
    ) {
        if custom_source_contract_expected_type(self.module, annotation).is_some() {
            return;
        }
        let Some(lowered) = typing.lower_annotation(annotation) else {
            return;
        };
        self.diagnostics.push(
            Diagnostic::error(format!(
                "provider contract {schema_kind} `{schema_name}` for `{provider_key}` uses unsupported schema type `{lowered}`"
            ))
            .with_code(code("unsupported-source-provider-contract-type"))
            .with_primary_label(
                span,
                "this custom source contract schema falls outside the current closed proof surface",
            )
            .with_note(
                "custom provider schemas currently support only compiler-known primitive types, same-module `type`/`domain` items, and those shapes under `List` or `Signal`",
            )
            .with_note(
                "records, tuples, arrows, `Option`/`Result`/`Task`, imported type constructors, and other richer schema forms remain later work",
            ),
        );
    }

    fn validate_custom_source_decorator_contract_types(
        &mut self,
        source_span: SourceSpan,
        provider: &SourceProviderRef,
        source: &crate::hir::SourceDecorator,
        contract: &crate::CustomSourceContractMetadata,
        typing: &mut GateTypeContext<'_>,
    ) {
        let provider_key = provider
            .key()
            .expect("custom provider validation requires a preserved provider key");
        if contract.arguments.len() != source.arguments.len() {
            self.emit_source_argument_count_mismatch(
                source_span,
                provider_key,
                contract.arguments.len(),
                source.arguments.len(),
            );
        }

        for (index, (argument, schema)) in source
            .arguments
            .iter()
            .copied()
            .zip(contract.arguments.iter())
            .enumerate()
        {
            let Some((expected, expected_surface)) =
                custom_source_contract_expected(self.module, schema.annotation, typing)
            else {
                continue;
            };
            let mut bindings = SourceOptionTypeBindings::default();
            match self.check_source_option_expr(argument, &expected, typing, &mut bindings) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => self
                    .emit_source_argument_value_mismatch(
                        schema.span,
                        provider_key,
                        index,
                        schema.name.text(),
                        &expected_surface,
                        mismatch,
                    ),
                SourceOptionTypeCheck::Unknown => {}
            }
        }

        let Some(options) = source.options else {
            return;
        };
        let ExprKind::Record(record) = &self.module.exprs()[options].kind else {
            return;
        };
        for field in &record.fields {
            let Some(schema) = contract
                .options
                .iter()
                .find(|schema| schema.name.text() == field.label.text())
            else {
                self.emit_unknown_source_option(field.span, provider_key, field.label.text());
                continue;
            };
            let Some((expected, expected_surface)) =
                custom_source_contract_expected(self.module, schema.annotation, typing)
            else {
                continue;
            };
            let mut bindings = SourceOptionTypeBindings::default();
            match self.check_source_option_expr(field.value, &expected, typing, &mut bindings) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => self
                    .emit_source_option_value_mismatch(
                        field,
                        provider_key,
                        &expected_surface,
                        mismatch,
                    ),
                SourceOptionTypeCheck::Unknown => {}
            }
        }
    }

    fn validate_expression_types(&mut self) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }
        // TODO(perf): `typecheck_module` re-runs full type inference over the entire module from
        // scratch on every call. Because `validate_module` is invoked once per gate context
        // construction (and gate contexts are created for every item being elaborated), this makes
        // elaboration O(n²) in the number of gates: each new `GateTypeContext` starts with an
        // empty `item_types` cache and must re-infer every reachable item. Fixing this requires
        // either sharing a single persistent `GateTypeContext` across all elaboration passes, or
        // memoising the per-module type-check result and reusing it rather than recomputing it.
        self.diagnostics
            .extend(typecheck_module(self.module).into_diagnostics());
    }

    /// Checks that every constructor call site (in both patterns and expressions) supplies
    /// exactly the number of arguments declared by the corresponding variant definition.
    ///
    /// Patterns carry all constructor arguments inline, so arity can be verified directly.
    /// Expression call sites use curried application and are handled by type inference in
    /// [`validate_expression_types`]; this pass covers any structural mismatches that survive
    /// before type inference runs (e.g. in `Structural` validation mode).
    fn validate_constructor_arity(&mut self) {
        for (_, pattern) in self.module.patterns().iter() {
            let PatternKind::Constructor { callee, arguments } = &pattern.kind else {
                continue;
            };
            let ResolutionState::Resolved(TermResolution::Item(item_id)) =
                callee.resolution.as_ref()
            else {
                continue;
            };
            let Item::Type(type_item) = &self.module.items()[*item_id] else {
                continue;
            };
            let TypeItemBody::Sum(variants) = &type_item.body else {
                continue;
            };
            let variant_name = callee.path.segments().last().text();
            let Some(variant) = variants.iter().find(|v| v.name.text() == variant_name) else {
                continue;
            };
            let expected = variant.fields.len();
            let actual = arguments.len();
            if actual != expected {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "constructor `{}` expects {expected} argument{}, but {actual} {} provided",
                        callee.path,
                        if expected == 1 { "" } else { "s" },
                        if actual == 1 { "was" } else { "were" },
                    ))
                    .with_code(code("constructor-arity-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        pattern.span,
                        format!(
                            "this pattern supplies {actual} argument{} to a {expected}-field constructor",
                            if actual == 1 { "" } else { "s" },
                        ),
                    )),
                );
            }
        }
    }

    fn emit_source_option_value_mismatch(
        &mut self,
        field: &crate::hir::RecordExprField,
        provider_key: &str,
        expected_surface: &str,
        mismatch: SourceOptionTypeMismatch,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "source option `{}` for `{provider_key}` expects `{expected_surface}`, but this expression proves `{}`",
                field.label.text(),
                mismatch.actual
            ))
            .with_code(code("source-option-type-mismatch"))
            .with_primary_label(
                mismatch.span,
                format!("this source option expression proves `{}`", mismatch.actual),
            )
            .with_secondary_label(
                field.span,
                format!("`{}` expects `{expected_surface}`", field.label.text()),
            )
            .with_note(
                "current source option typing checks only the resolved-HIR cases it can prove honestly: same-module annotations, same-module unannotated value bodies rechecked through that same proof slice, suffixed domain literals including any generic arguments the resolved literal-member result proves, same-module constructors checked against the expected contract type or re-inferred as bare roots with `_` holes for any still-unproven generic arguments, built-in `Option` / `Result` / `Validation` constructors including bare roots that only prove a local container shape, imported bindings whose compiler-known import metadata lowers into the current closed type surface, tuple/record/list/map/set expressions whose nested values stay within that same slice, and reactive `Signal` payloads used as ordinary source configuration values",
            )
            .with_note(
                "bare contract-parameter roots now also cover nested same-module generic constructor applications, unannotated local value bodies, tuple/record/list literals, `Some` roots, context-free `None` / `Ok` / `Err` / `Valid` / `Invalid` holes carried through local source-option bindings, and constructor fields whose tuple/record or built-in container shape can be proved locally; imports without compiler-known type metadata and otherwise unproven ordinary expressions still wait for fuller expression typing",
            ),
        );
    }

    fn emit_source_option_unbound_contract_parameter(
        &mut self,
        field: &crate::hir::RecordExprField,
        provider_key: &str,
        expected_surface: &str,
        expected: &SourceOptionExpectedType,
        bindings: &SourceOptionTypeBindings,
    ) {
        let unresolved = source_option_unresolved_contract_parameters(expected, bindings);
        if unresolved.is_empty() {
            return;
        }
        let summaries = unresolved
            .iter()
            .map(|parameter| match bindings.parameter(*parameter) {
                Some(actual) => format!("{parameter} = {actual}"),
                None => format!("{parameter} = _"),
            })
            .collect::<Vec<_>>();
        let summary = summaries.join("`, `");

        self.diagnostics.push(
            Diagnostic::error(format!(
                "source option `{}` for `{provider_key}` expects `{expected_surface}`, but local source-option checking leaves {} unbound",
                field.label.text(),
                source_option_contract_parameter_phrase(&unresolved),
            ))
            .with_code(code("source-option-unbound-contract-parameter"))
            .with_primary_label(
                self.module.exprs()[field.value].span,
                format!("current fixed-point proof stops at `{summary}`"),
            )
            .with_secondary_label(
                field.span,
                format!("`{}` expects `{expected_surface}`", field.label.text()),
            )
            .with_note(
                "source option contract parameters must collapse to a closed type after fixed-point refinement across the provided option values",
            )
            .with_note(
                "add a more specific constructor, literal, annotation, or same-module binding so local source-option typing can close the remaining contract parameter holes",
            ),
        );
    }

    fn emit_unknown_source_option(
        &mut self,
        span: SourceSpan,
        provider_key: &str,
        option_name: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "unknown source option `{option_name}` for `{provider_key}`"
            ))
            .with_code(code("unknown-source-option"))
            .with_primary_label(
                span,
                "this option is not supported for the selected source provider",
            ),
        );
    }

    fn emit_source_argument_count_mismatch(
        &mut self,
        span: SourceSpan,
        provider_key: &str,
        expected: usize,
        actual: usize,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "source `{provider_key}` expects {expected} positional argument{}, but this use provides {actual}",
                if expected == 1 { "" } else { "s" }
            ))
            .with_code(code("source-argument-count-mismatch"))
            .with_primary_label(
                span,
                "adjust the `@source` arguments to match the declared provider contract",
            ),
        );
    }

    fn emit_source_argument_value_mismatch(
        &mut self,
        span: SourceSpan,
        provider_key: &str,
        index: usize,
        schema_name: &str,
        expected_surface: &str,
        mismatch: SourceOptionTypeMismatch,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "source argument #{} `{schema_name}` for `{provider_key}` expects `{expected_surface}`, but this expression proves `{}`",
                index + 1,
                mismatch.actual
            ))
            .with_code(code("source-argument-type-mismatch"))
            .with_primary_label(
                mismatch.span,
                format!("this source argument expression proves `{}`", mismatch.actual),
            )
            .with_secondary_label(
                span,
                format!("argument #{} `{schema_name}` expects `{expected_surface}`", index + 1),
            )
            .with_note(
                "current custom source contract typing reuses the same resolved-HIR proof surface as source options: same-module annotations, same-module unannotated value bodies rechecked through that same proof slice, suffixed domain literals including any generic arguments the resolved literal-member result proves, same-module constructors checked against the expected contract type or re-inferred as bare roots with `_` holes for any still-unproven generic arguments, built-in `Option` / `Result` / `Validation` constructors including bare roots that only prove a local container shape, imported bindings whose compiler-known import metadata lowers into the current closed type surface, tuple/record/list/map/set expressions whose nested values stay within that same slice, and reactive `Signal` payloads used as ordinary source configuration values",
            ),
        );
    }

    fn check_source_option_expr(
        &self,
        expr_id: ExprId,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
    ) -> SourceOptionTypeCheck {
        self.check_source_option_expr_inner(expr_id, expected, typing, bindings, &mut Vec::new())
    }

    fn check_source_option_expr_inner(
        &self,
        expr_id: ExprId,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        match &self.module.exprs()[expr_id].kind {
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::Text(_)
            | ExprKind::Regex(_) => self.check_source_option_expr_by_inference_or_unknown(
                expr_id,
                expected,
                typing,
                bindings,
                value_stack,
            ),
            ExprKind::SuffixedInteger(literal) => self
                .check_source_option_suffixed_integer(expr_id, literal, expected, typing, bindings),
            ExprKind::Name(reference) => {
                if let Some(check) = self.check_source_option_expr_by_inference(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                ) {
                    return check;
                }
                self.check_source_option_name(reference, expected, typing, bindings, value_stack)
            }
            ExprKind::Apply { callee, arguments } => {
                if let Some(check) = self.check_source_option_expr_by_inference(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                ) {
                    return check;
                }
                self.check_source_option_apply(
                    self.module.exprs()[expr_id].span,
                    *callee,
                    arguments,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::Tuple(elements) => {
                if let SourceOptionExpectedType::Tuple(expected_elements) = expected {
                    return self.check_source_option_tuple(
                        expr_id,
                        elements,
                        expected_elements,
                        typing,
                        bindings,
                        value_stack,
                    );
                }
                self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::List(elements) => {
                if let SourceOptionExpectedType::List(element_expected) = expected {
                    return self.check_source_option_list(
                        elements,
                        element_expected,
                        typing,
                        bindings,
                        value_stack,
                    );
                }
                self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::Map(map) => {
                if let SourceOptionExpectedType::Map { key, value } = expected {
                    return self.check_source_option_map(
                        map,
                        key,
                        value,
                        typing,
                        bindings,
                        value_stack,
                    );
                }
                self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::Set(elements) => {
                if let SourceOptionExpectedType::Set(element_expected) = expected {
                    return self.check_source_option_set(
                        elements,
                        element_expected,
                        typing,
                        bindings,
                        value_stack,
                    );
                }
                self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::Record(record) => {
                if let SourceOptionExpectedType::Record(expected_fields) = expected {
                    return self.check_source_option_record(
                        expr_id,
                        record,
                        expected_fields,
                        typing,
                        bindings,
                        value_stack,
                    );
                }
                self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::Projection { .. } => SourceOptionTypeCheck::Unknown,
            _ => SourceOptionTypeCheck::Unknown,
        }
    }

    fn check_source_option_expr_by_inference(
        &self,
        expr_id: ExprId,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionTypeCheck> {
        let actual = self.infer_source_option_expr_actual_type_inner(
            expr_id,
            typing,
            bindings,
            value_stack,
        )?;
        Some(
            if source_option_expected_matches_actual_type(expected, &actual, bindings) {
                SourceOptionTypeCheck::Match
            } else {
                SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                    span: self.module.exprs()[expr_id].span,
                    actual: actual.to_string(),
                })
            },
        )
    }

    fn check_source_option_expr_by_inference_or_unknown(
        &self,
        expr_id: ExprId,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        self.check_source_option_expr_by_inference(expr_id, expected, typing, bindings, value_stack)
            .unwrap_or(SourceOptionTypeCheck::Unknown)
    }

    fn check_source_option_suffixed_integer(
        &self,
        expr_id: ExprId,
        literal: &crate::hir::SuffixedIntegerLiteral,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
    ) -> SourceOptionTypeCheck {
        let expected_gate = source_option_expected_to_gate_type(expected, bindings);
        let Some(actual) = self.infer_source_option_suffixed_integer_actual_type(
            literal,
            expected_gate.as_ref(),
            typing,
        ) else {
            return SourceOptionTypeCheck::Unknown;
        };
        if source_option_expected_matches_actual_type(expected, &actual, bindings) {
            SourceOptionTypeCheck::Match
        } else {
            SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: self.module.exprs()[expr_id].span,
                actual: actual.to_string(),
            })
        }
    }

    fn check_source_option_name(
        &self,
        reference: &TermReference,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        if let Some(check) = self.check_source_option_unannotated_value_item(
            reference,
            expected,
            typing,
            bindings,
            value_stack,
        ) {
            return check;
        }
        if let Some(check) = self.check_source_option_builtin_constructor(
            reference,
            &[],
            expected,
            typing,
            bindings,
            value_stack,
        ) {
            return check;
        }
        self.check_source_option_constructor(
            reference,
            &[],
            expected,
            typing,
            bindings,
            value_stack,
        )
    }

    fn check_source_option_apply(
        &self,
        apply_span: SourceSpan,
        callee: ExprId,
        arguments: &crate::NonEmpty<ExprId>,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        let ExprKind::Name(reference) = &self.module.exprs()[callee].kind else {
            return SourceOptionTypeCheck::Unknown;
        };
        let arguments = arguments.iter().copied().collect::<Vec<_>>();
        if let Some(check) = self.check_source_option_builtin_constructor(
            reference,
            &arguments,
            expected,
            typing,
            bindings,
            value_stack,
        ) {
            return check;
        }
        if let Some(check) = self.check_source_option_named_apply(
            apply_span,
            reference,
            &arguments,
            expected,
            typing,
            bindings,
            value_stack,
        ) {
            return check;
        }
        self.check_source_option_constructor(
            reference,
            &arguments,
            expected,
            typing,
            bindings,
            value_stack,
        )
    }

    fn check_source_option_unannotated_value_item(
        &self,
        reference: &TermReference,
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionTypeCheck> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Value(item) = &self.module.items()[*item_id] else {
            return None;
        };
        if item.annotation.is_some() {
            return None;
        }
        if value_stack.contains(item_id) {
            return Some(SourceOptionTypeCheck::Unknown);
        }

        value_stack.push(*item_id);
        let check =
            self.check_source_option_expr_inner(item.body, expected, typing, bindings, value_stack);
        let popped = value_stack.pop();
        debug_assert_eq!(popped, Some(*item_id));
        Some(check)
    }

    fn check_source_option_named_apply(
        &self,
        apply_span: SourceSpan,
        reference: &TermReference,
        arguments: &[ExprId],
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionTypeCheck> {
        let mut current = self.source_option_name_apply_gate_type(reference, typing)?;
        let mut saw_unknown = false;

        for argument in arguments {
            let GateType::Arrow { parameter, result } = current else {
                return Some(SourceOptionTypeCheck::Unknown);
            };
            let Some(parameter_expected) = SourceOptionExpectedType::from_gate_type(
                self.module,
                &parameter,
                SourceOptionTypeSurface::Expression,
            ) else {
                return Some(SourceOptionTypeCheck::Unknown);
            };
            match self.check_source_option_expr_inner(
                *argument,
                &parameter_expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return Some(SourceOptionTypeCheck::Mismatch(mismatch));
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
            current = *result;
        }

        if saw_unknown {
            return Some(SourceOptionTypeCheck::Unknown);
        }

        let actual = SourceOptionActualType::from_gate_type(&current);
        Some(
            if source_option_expected_matches_actual_type(expected, &actual, bindings) {
                SourceOptionTypeCheck::Match
            } else {
                SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                    span: apply_span,
                    actual: actual.to_string(),
                })
            },
        )
    }

    fn source_option_name_apply_gate_type(
        &self,
        reference: &TermReference,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                typing.import_value_type(*import_id)
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => match &self.module.items()
                [*item_id]
            {
                Item::Function(_) => typing.item_value_type(*item_id),
                Item::Value(item) if item.annotation.is_some() => typing.item_value_type(*item_id),
                _ => None,
            },
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => None,
        }
    }

    fn check_source_option_builtin_constructor(
        &self,
        reference: &TermReference,
        arguments: &[ExprId],
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionTypeCheck> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };

        let synthesized_expected = match expected {
            SourceOptionExpectedType::ContractParameter(parameter) => {
                bindings.parameter_gate_type(*parameter).and_then(|bound| {
                    SourceOptionExpectedType::from_gate_type(
                        self.module,
                        &bound,
                        SourceOptionTypeSurface::Expression,
                    )
                })
            }
            _ => None,
        };
        let expected = synthesized_expected.as_ref().unwrap_or(expected);

        let constructor_actual = format!("builtin constructor `{}`", builtin_term_name(*builtin));
        Some(match (builtin, arguments) {
            (BuiltinTerm::None, []) => match expected {
                SourceOptionExpectedType::Option(_) => SourceOptionTypeCheck::Match,
                SourceOptionExpectedType::ContractParameter(_) => return None,
                _ => SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                    span: reference.path.span(),
                    actual: constructor_actual,
                }),
            },
            (BuiltinTerm::None, _) => SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: reference.path.span(),
                actual: constructor_actual,
            }),
            (BuiltinTerm::Some, [argument]) => {
                let SourceOptionExpectedType::Option(payload_expected) = expected else {
                    if matches!(expected, SourceOptionExpectedType::ContractParameter(_)) {
                        return None;
                    }
                    return Some(SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: constructor_actual,
                    }));
                };
                self.check_source_option_expr_inner(
                    *argument,
                    payload_expected,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            (BuiltinTerm::Ok, [argument]) => {
                let SourceOptionExpectedType::Result { value, .. } = expected else {
                    if matches!(expected, SourceOptionExpectedType::ContractParameter(_)) {
                        return None;
                    }
                    return Some(SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: constructor_actual,
                    }));
                };
                self.check_source_option_expr_inner(*argument, value, typing, bindings, value_stack)
            }
            (BuiltinTerm::Err, [argument]) => {
                let SourceOptionExpectedType::Result { error, .. } = expected else {
                    if matches!(expected, SourceOptionExpectedType::ContractParameter(_)) {
                        return None;
                    }
                    return Some(SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: constructor_actual,
                    }));
                };
                self.check_source_option_expr_inner(*argument, error, typing, bindings, value_stack)
            }
            (BuiltinTerm::Valid, [argument]) => {
                let SourceOptionExpectedType::Validation { value, .. } = expected else {
                    if matches!(expected, SourceOptionExpectedType::ContractParameter(_)) {
                        return None;
                    }
                    return Some(SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: constructor_actual,
                    }));
                };
                self.check_source_option_expr_inner(*argument, value, typing, bindings, value_stack)
            }
            (BuiltinTerm::Invalid, [argument]) => {
                let SourceOptionExpectedType::Validation { error, .. } = expected else {
                    if matches!(expected, SourceOptionExpectedType::ContractParameter(_)) {
                        return None;
                    }
                    return Some(SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: constructor_actual,
                    }));
                };
                self.check_source_option_expr_inner(*argument, error, typing, bindings, value_stack)
            }
            (BuiltinTerm::True | BuiltinTerm::False, _) => return None,
            (
                BuiltinTerm::Some
                | BuiltinTerm::Ok
                | BuiltinTerm::Err
                | BuiltinTerm::Valid
                | BuiltinTerm::Invalid,
                _,
            ) => SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: reference.path.span(),
                actual: constructor_actual,
            }),
        })
    }

    fn check_source_option_constructor(
        &self,
        reference: &TermReference,
        arguments: &[ExprId],
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        let Some(actual) = self.source_constructor_actual(reference) else {
            return SourceOptionTypeCheck::Unknown;
        };

        let mut bind_parameter = None;
        let synthesized_expected = match expected {
            SourceOptionExpectedType::ContractParameter(parameter) => {
                if let Some(bound) = bindings.parameter_gate_type(*parameter) {
                    let Some(bound_expected) = SourceOptionExpectedType::from_gate_type(
                        self.module,
                        &bound,
                        SourceOptionTypeSurface::Expression,
                    ) else {
                        return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                            span: reference.path.span(),
                            actual: actual.parent_name.clone(),
                        });
                    };
                    Some(bound_expected)
                } else {
                    let Some(monomorphic) =
                        self.source_option_monomorphic_constructor_expected(actual.parent_item)
                    else {
                        return self
                            .check_source_option_unbound_contract_parameter_constructor_root(
                                *parameter,
                                reference.path.span(),
                                &actual,
                                arguments,
                                typing,
                                bindings,
                                value_stack,
                            );
                    };
                    bind_parameter = Some((
                        *parameter,
                        SourceOptionActualType::OpaqueItem {
                            item: actual.parent_item,
                            name: actual.parent_name.clone(),
                            arguments: Vec::new(),
                        },
                    ));
                    Some(monomorphic)
                }
            }
            _ => None,
        };
        let expected = synthesized_expected.as_ref().unwrap_or(expected);

        if arguments.len() != actual.field_types.len() {
            return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: reference.path.span(),
                actual: format!("constructor `{}`", actual.constructor_name),
            });
        }

        if !expected.matches_named_item(actual.parent_item) {
            return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: reference.path.span(),
                actual: actual.parent_name.clone(),
            });
        }

        if actual.field_types.is_empty() {
            if let Some((parameter, bound_type)) = bind_parameter.as_ref() {
                if !bindings.bind_or_match_actual(*parameter, bound_type) {
                    return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: actual.parent_name.clone(),
                    });
                }
            }
            return SourceOptionTypeCheck::Match;
        }

        let Some(expected_named) = expected.as_named() else {
            return SourceOptionTypeCheck::Unknown;
        };
        let Some(field_expectations) = self.source_option_constructor_field_expectations(
            actual.parent_item,
            expected_named,
            &actual.field_types,
        ) else {
            return SourceOptionTypeCheck::Unknown;
        };

        let mut saw_unknown = false;
        for (argument, field_expected) in arguments.iter().zip(&field_expectations) {
            match self.check_source_option_expr_inner(
                *argument,
                field_expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return SourceOptionTypeCheck::Mismatch(mismatch);
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            if let Some((parameter, bound_type)) = bind_parameter.as_ref() {
                if !bindings.bind_or_match_actual(*parameter, bound_type) {
                    return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: actual.parent_name.clone(),
                    });
                }
            }
            SourceOptionTypeCheck::Match
        }
    }

    fn check_source_option_unbound_contract_parameter_constructor_root(
        &self,
        parameter: SourceTypeParameter,
        constructor_span: SourceSpan,
        actual: &SourceOptionConstructorActual,
        arguments: &[ExprId],
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        match self.infer_source_option_generic_constructor_root(
            constructor_span,
            actual,
            arguments,
            typing,
            bindings,
            value_stack,
        ) {
            SourceOptionGenericConstructorRootCheck::Match(actual_type) => {
                if !bindings.bind_or_match_actual(parameter, &actual_type) {
                    return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: constructor_span,
                        actual: actual_type.to_string(),
                    });
                }
                SourceOptionTypeCheck::Match
            }
            SourceOptionGenericConstructorRootCheck::Mismatch(mismatch) => {
                SourceOptionTypeCheck::Mismatch(mismatch)
            }
            SourceOptionGenericConstructorRootCheck::Unknown => SourceOptionTypeCheck::Unknown,
        }
    }

    fn infer_source_option_generic_constructor_root(
        &self,
        constructor_span: SourceSpan,
        actual: &SourceOptionConstructorActual,
        arguments: &[ExprId],
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionGenericConstructorRootCheck {
        let Item::Type(item) = &self.module.items()[actual.parent_item] else {
            return SourceOptionGenericConstructorRootCheck::Unknown;
        };
        if arguments.len() != actual.field_types.len() {
            return SourceOptionGenericConstructorRootCheck::Mismatch(SourceOptionTypeMismatch {
                span: constructor_span,
                actual: format!("constructor `{}`", actual.constructor_name),
            });
        }

        let mut parameter_substitutions = HashMap::<TypeParameterId, SourceOptionActualType>::new();
        let mut trial_bindings = bindings.clone();
        let mut pending = arguments
            .iter()
            .copied()
            .zip(actual.field_types.iter().copied())
            .collect::<Vec<_>>();
        while !pending.is_empty() {
            let mut progress = false;
            let mut remaining = Vec::new();
            for (argument, field_type) in std::mem::take(&mut pending) {
                if let Some(expected) = self.source_option_constructor_field_expected_type(
                    field_type,
                    &parameter_substitutions,
                ) {
                    match self.check_source_option_expr_inner(
                        argument,
                        &expected,
                        typing,
                        &mut trial_bindings,
                        value_stack,
                    ) {
                        SourceOptionTypeCheck::Match => {
                            progress = true;
                            continue;
                        }
                        SourceOptionTypeCheck::Mismatch(mismatch) => {
                            return SourceOptionGenericConstructorRootCheck::Mismatch(mismatch);
                        }
                        SourceOptionTypeCheck::Unknown => {}
                    }
                }

                let actual_argument = match self.infer_source_option_expr_actual_type_inner(
                    argument,
                    typing,
                    &trial_bindings,
                    value_stack,
                ) {
                    Some(actual_argument) => actual_argument,
                    None => {
                        remaining.push((argument, field_type));
                        continue;
                    }
                };
                match self.source_option_hir_type_matches_actual_type(
                    field_type,
                    &actual_argument,
                    &mut parameter_substitutions,
                ) {
                    Some(true) => progress = true,
                    Some(false) => {
                        return SourceOptionGenericConstructorRootCheck::Mismatch(
                            SourceOptionTypeMismatch {
                                span: self.module.exprs()[argument].span,
                                actual: actual_argument.to_string(),
                            },
                        );
                    }
                    None => remaining.push((argument, field_type)),
                }
            }
            if !progress {
                break;
            }
            pending = remaining;
        }

        if !pending.is_empty() {
            return SourceOptionGenericConstructorRootCheck::Unknown;
        }

        let arguments = item
            .parameters
            .iter()
            .map(|parameter| {
                parameter_substitutions
                    .get(parameter)
                    .cloned()
                    .unwrap_or(SourceOptionActualType::Hole)
            })
            .collect::<Vec<_>>();
        SourceOptionGenericConstructorRootCheck::Match(SourceOptionActualType::OpaqueItem {
            item: actual.parent_item,
            name: actual.parent_name.clone(),
            arguments,
        })
    }

    fn infer_source_option_suffixed_integer_actual_type(
        &self,
        literal: &crate::hir::SuffixedIntegerLiteral,
        expected: Option<&GateType>,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<SourceOptionActualType> {
        let ResolutionState::Resolved(resolution) = literal.resolution.as_ref() else {
            return None;
        };
        let resolution = *resolution;
        let root_actual = self.source_option_literal_suffix_root_actual_type(resolution)?;
        let Some(result_type) = self.source_option_literal_suffix_result_type(resolution) else {
            return Some(root_actual);
        };

        let mut fallback = root_actual.clone();
        if let Some(actual) = self
            .source_option_hir_type_to_actual_type(result_type, &HashMap::new())
            .and_then(|actual| root_actual.unify(&actual))
        {
            fallback = actual;
        }

        let Some(expected) = expected else {
            return Some(fallback);
        };

        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !typing.match_hir_type(result_type, expected, &mut substitutions, &mut item_stack) {
            return Some(fallback);
        }
        let substitutions = substitutions
            .into_iter()
            .map(|(parameter, ty)| (parameter, SourceOptionActualType::from_gate_type(&ty)))
            .collect::<HashMap<_, _>>();
        self.source_option_hir_type_to_actual_type(result_type, &substitutions)
            .and_then(|actual| root_actual.unify(&actual))
            .or(Some(fallback))
    }

    fn source_option_literal_suffix_root_actual_type(
        &self,
        resolution: LiteralSuffixResolution,
    ) -> Option<SourceOptionActualType> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        Some(SourceOptionActualType::Domain {
            item: resolution.domain,
            name: domain.name.text().to_owned(),
            arguments: vec![SourceOptionActualType::Hole; domain.parameters.len()],
        })
    }

    fn source_option_literal_suffix_result_type(
        &self,
        resolution: LiteralSuffixResolution,
    ) -> Option<TypeId> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        let member = domain.members.get(resolution.member_index)?;
        if member.kind != DomainMemberKind::Literal {
            return None;
        }
        let TypeKind::Arrow { result, .. } = &self.module.types()[member.annotation].kind else {
            return None;
        };
        Some(*result)
    }

    fn infer_source_option_expr_actual_type_inner(
        &self,
        expr_id: ExprId,
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionActualType> {
        match &self.module.exprs()[expr_id].kind {
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::Text(_)
            | ExprKind::Regex(_) => typing
                .infer_expr(expr_id, &GateExprEnv::default(), None)
                .actual(),
            ExprKind::SuffixedInteger(literal) => {
                self.infer_source_option_suffixed_integer_actual_type(literal, None, typing)
            }
            ExprKind::Name(reference) => {
                self.infer_source_option_name_actual_type(reference, typing, bindings, value_stack)
            }
            ExprKind::Apply { callee, arguments } => {
                let ExprKind::Name(reference) = &self.module.exprs()[*callee].kind else {
                    return None;
                };
                let arguments = arguments.iter().copied().collect::<Vec<_>>();
                self.infer_source_option_constructor_like_actual_type(
                    reference,
                    &arguments,
                    typing,
                    bindings,
                    value_stack,
                )
            }
            ExprKind::List(elements) => {
                let mut element_type = None::<SourceOptionActualType>;
                for element in elements {
                    let child = self.infer_source_option_expr_actual_type_inner(
                        *element,
                        typing,
                        bindings,
                        value_stack,
                    )?;
                    element_type = Some(match element_type.take() {
                        None => child,
                        Some(current) => current.unify(&child)?,
                    });
                }
                Some(SourceOptionActualType::List(Box::new(element_type?)))
            }
            ExprKind::Map(map) => {
                let mut key_type = None::<SourceOptionActualType>;
                let mut value_type = None::<SourceOptionActualType>;
                for entry in &map.entries {
                    let key = self.infer_source_option_expr_actual_type_inner(
                        entry.key,
                        typing,
                        bindings,
                        value_stack,
                    )?;
                    key_type = Some(match key_type.take() {
                        None => key,
                        Some(current) => current.unify(&key)?,
                    });

                    let value = self.infer_source_option_expr_actual_type_inner(
                        entry.value,
                        typing,
                        bindings,
                        value_stack,
                    )?;
                    value_type = Some(match value_type.take() {
                        None => value,
                        Some(current) => current.unify(&value)?,
                    });
                }
                Some(SourceOptionActualType::Map {
                    key: Box::new(key_type?),
                    value: Box::new(value_type?),
                })
            }
            ExprKind::Set(elements) => {
                let mut element_type = None::<SourceOptionActualType>;
                for element in elements {
                    let child = self.infer_source_option_expr_actual_type_inner(
                        *element,
                        typing,
                        bindings,
                        value_stack,
                    )?;
                    element_type = Some(match element_type.take() {
                        None => child,
                        Some(current) => current.unify(&child)?,
                    });
                }
                Some(SourceOptionActualType::Set(Box::new(element_type?)))
            }
            ExprKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.infer_source_option_expr_actual_type_inner(
                        *element,
                        typing,
                        bindings,
                        value_stack,
                    )?);
                }
                Some(SourceOptionActualType::Tuple(lowered))
            }
            ExprKind::Record(record) => {
                let mut fields = Vec::with_capacity(record.fields.len());
                for field in &record.fields {
                    fields.push(SourceOptionActualRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.infer_source_option_expr_actual_type_inner(
                            field.value,
                            typing,
                            bindings,
                            value_stack,
                        )?,
                    });
                }
                Some(SourceOptionActualType::Record(fields))
            }
            _ => None,
        }
    }

    fn infer_source_option_name_actual_type(
        &self,
        reference: &TermReference,
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionActualType> {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => None,
            ResolutionState::Resolved(TermResolution::Local(_)) => None,
            ResolutionState::Resolved(TermResolution::Import(import_id)) => typing
                .import_value_type(*import_id)
                .map(|actual| SourceOptionActualType::from_gate_type(&actual)),
            ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => None,
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => self
                .infer_source_option_builtin_actual_type(
                    *builtin,
                    &[],
                    typing,
                    bindings,
                    value_stack,
                ),
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                match &self.module.items()[*item_id] {
                    Item::Value(item) if item.annotation.is_none() => {
                        if value_stack.contains(item_id) {
                            return None;
                        }
                        value_stack.push(*item_id);
                        let actual = self.infer_source_option_expr_actual_type_inner(
                            item.body,
                            typing,
                            bindings,
                            value_stack,
                        );
                        let popped = value_stack.pop();
                        debug_assert_eq!(popped, Some(*item_id));
                        actual
                    }
                    Item::Value(item) => item
                        .annotation
                        .and_then(|annotation| typing.lower_annotation(annotation))
                        .map(|actual| SourceOptionActualType::from_gate_type(&actual)),
                    Item::Function(_) | Item::Signal(_) => typing
                        .item_value_type(*item_id)
                        .map(|actual| SourceOptionActualType::from_gate_type(&actual)),
                    _ => self.infer_source_option_constructor_actual_type(
                        reference,
                        &[],
                        typing,
                        bindings,
                        value_stack,
                    ),
                }
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(_)) => None,
        }
    }

    fn infer_source_option_constructor_like_actual_type(
        &self,
        reference: &TermReference,
        arguments: &[ExprId],
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionActualType> {
        if let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        {
            return self.infer_source_option_builtin_actual_type(
                *builtin,
                arguments,
                typing,
                bindings,
                value_stack,
            );
        }
        self.infer_source_option_constructor_actual_type(
            reference,
            arguments,
            typing,
            bindings,
            value_stack,
        )
    }

    fn infer_source_option_builtin_actual_type(
        &self,
        builtin: BuiltinTerm,
        arguments: &[ExprId],
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionActualType> {
        match (builtin, arguments) {
            (BuiltinTerm::True | BuiltinTerm::False, []) => {
                Some(SourceOptionActualType::Primitive(BuiltinType::Bool))
            }
            (BuiltinTerm::None, []) => Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
            (BuiltinTerm::Some, [argument]) => Some(SourceOptionActualType::Option(Box::new(
                self.infer_source_option_expr_actual_type_inner(
                    *argument,
                    typing,
                    bindings,
                    value_stack,
                )?,
            ))),
            (BuiltinTerm::Ok, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(self.infer_source_option_expr_actual_type_inner(
                    *argument,
                    typing,
                    bindings,
                    value_stack,
                )?),
            }),
            (BuiltinTerm::Err, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(self.infer_source_option_expr_actual_type_inner(
                    *argument,
                    typing,
                    bindings,
                    value_stack,
                )?),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            (BuiltinTerm::Valid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(self.infer_source_option_expr_actual_type_inner(
                    *argument,
                    typing,
                    bindings,
                    value_stack,
                )?),
            }),
            (BuiltinTerm::Invalid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(self.infer_source_option_expr_actual_type_inner(
                    *argument,
                    typing,
                    bindings,
                    value_stack,
                )?),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            _ => None,
        }
    }

    fn infer_source_option_constructor_actual_type(
        &self,
        reference: &TermReference,
        arguments: &[ExprId],
        typing: &mut GateTypeContext<'_>,
        bindings: &SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<SourceOptionActualType> {
        let actual = self.source_constructor_actual(reference)?;
        match self.infer_source_option_generic_constructor_root(
            reference.path.span(),
            &actual,
            arguments,
            typing,
            bindings,
            value_stack,
        ) {
            SourceOptionGenericConstructorRootCheck::Match(actual_type) => Some(actual_type),
            SourceOptionGenericConstructorRootCheck::Mismatch(_)
            | SourceOptionGenericConstructorRootCheck::Unknown => None,
        }
    }

    fn source_option_constructor_field_expected_type(
        &self,
        field_type: TypeId,
        substitutions: &HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<SourceOptionExpectedType> {
        let substitutions = substitutions
            .iter()
            .filter_map(|(parameter, ty)| {
                ty.to_gate_type().and_then(|ty| {
                    SourceOptionExpectedType::from_gate_type(
                        self.module,
                        &ty,
                        SourceOptionTypeSurface::Expression,
                    )
                    .map(|expected| (*parameter, expected))
                })
            })
            .collect::<HashMap<_, _>>();
        SourceOptionExpectedType::from_hir_type(
            self.module,
            field_type,
            &substitutions,
            SourceOptionTypeSurface::Expression,
        )
    }

    fn source_option_hir_type_to_actual_type(
        &self,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<SourceOptionActualType> {
        match &self.module.types()[ty].kind {
            TypeKind::Name(reference) => {
                self.source_option_hir_type_reference_to_actual_type(reference, substitutions)
            }
            TypeKind::Apply { callee, arguments } => {
                let arguments = arguments.iter().copied().collect::<Vec<_>>();
                self.source_option_hir_type_application_to_actual_type(
                    *callee,
                    &arguments,
                    substitutions,
                )
            }
            TypeKind::Tuple(elements) => Some(SourceOptionActualType::Tuple(
                elements
                    .iter()
                    .copied()
                    .map(|element| {
                        self.source_option_hir_type_to_actual_type(element, substitutions)
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Record(fields) => Some(SourceOptionActualType::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionActualRecordField {
                            name: field.label.text().to_owned(),
                            ty: self
                                .source_option_hir_type_to_actual_type(field.ty, substitutions)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Arrow { parameter, result } => Some(SourceOptionActualType::Arrow {
                parameter: Box::new(
                    self.source_option_hir_type_to_actual_type(*parameter, substitutions)?,
                ),
                result: Box::new(
                    self.source_option_hir_type_to_actual_type(*result, substitutions)?,
                ),
            }),
        }
    }

    fn source_option_hir_type_reference_to_actual_type(
        &self,
        reference: &TypeReference,
        substitutions: &HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<SourceOptionActualType> {
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
            )) => Some(SourceOptionActualType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => Some(
                substitutions
                    .get(parameter)
                    .cloned()
                    .unwrap_or(SourceOptionActualType::Hole),
            ),
            ResolutionState::Resolved(TypeResolution::Item(item)) => {
                self.source_option_named_item_actual_type(*item, Vec::new())
            }
            ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Resolved(TypeResolution::Import(_)) => None,
        }
    }

    fn source_option_hir_type_application_to_actual_type(
        &self,
        callee: TypeId,
        arguments: &[TypeId],
        substitutions: &HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<SourceOptionActualType> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Some(SourceOptionActualType::List(Box::new(
                    self.source_option_hir_type_to_actual_type(*arguments.first()?, substitutions)?,
                )))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                Some(SourceOptionActualType::Map {
                    key: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.first()?,
                        substitutions,
                    )?),
                    value: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.get(1)?,
                        substitutions,
                    )?),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                Some(SourceOptionActualType::Set(Box::new(
                    self.source_option_hir_type_to_actual_type(*arguments.first()?, substitutions)?,
                )))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                Some(SourceOptionActualType::Option(Box::new(
                    self.source_option_hir_type_to_actual_type(*arguments.first()?, substitutions)?,
                )))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                Some(SourceOptionActualType::Result {
                    error: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.first()?,
                        substitutions,
                    )?),
                    value: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.get(1)?,
                        substitutions,
                    )?),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                Some(SourceOptionActualType::Validation {
                    error: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.first()?,
                        substitutions,
                    )?),
                    value: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.get(1)?,
                        substitutions,
                    )?),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                Some(SourceOptionActualType::Signal(Box::new(
                    self.source_option_hir_type_to_actual_type(*arguments.first()?, substitutions)?,
                )))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                Some(SourceOptionActualType::Task {
                    error: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.first()?,
                        substitutions,
                    )?),
                    value: Box::new(self.source_option_hir_type_to_actual_type(
                        *arguments.get(1)?,
                        substitutions,
                    )?),
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item)) => {
                let arguments = arguments
                    .iter()
                    .copied()
                    .map(|argument| {
                        self.source_option_hir_type_to_actual_type(argument, substitutions)
                    })
                    .collect::<Option<Vec<_>>>()?;
                self.source_option_named_item_actual_type(*item, arguments)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Unresolved => None,
        }
    }

    fn source_option_named_item_actual_type(
        &self,
        item: ItemId,
        arguments: Vec<SourceOptionActualType>,
    ) -> Option<SourceOptionActualType> {
        match &self.module.items()[item] {
            Item::Domain(domain) => Some(SourceOptionActualType::Domain {
                item,
                name: domain.name.text().to_owned(),
                arguments,
            }),
            Item::Type(item_ref) => Some(SourceOptionActualType::OpaqueItem {
                item,
                name: item_ref.name.text().to_owned(),
                arguments,
            }),
            _ => None,
        }
    }

    fn source_option_hir_type_matches_actual_type(
        &self,
        expected: TypeId,
        actual: &SourceOptionActualType,
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
        if !self.source_option_hir_type_is_signal_contract(expected) {
            if let SourceOptionActualType::Signal(inner) = actual {
                return self.source_option_hir_type_matches_actual_type_inner(
                    expected,
                    inner,
                    substitutions,
                );
            }
        }

        self.source_option_hir_type_matches_actual_type_inner(expected, actual, substitutions)
    }

    fn source_option_hir_type_matches_actual_type_inner(
        &self,
        expected: TypeId,
        actual: &SourceOptionActualType,
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
        match &self.module.types()[expected].kind {
            TypeKind::Name(reference) => self.source_option_hir_type_reference_matches_actual_type(
                reference,
                actual,
                substitutions,
            ),
            TypeKind::Apply { callee, arguments } => {
                let arguments = arguments.iter().copied().collect::<Vec<_>>();
                self.source_option_hir_type_application_matches_actual_type(
                    *callee,
                    &arguments,
                    actual,
                    substitutions,
                )
            }
            TypeKind::Tuple(elements) => {
                let SourceOptionActualType::Tuple(actual_elements) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                if elements.len() != actual_elements.len() {
                    return Some(false);
                }
                for (expected, actual) in elements.iter().copied().zip(actual_elements) {
                    match self.source_option_hir_type_matches_actual_type(
                        expected,
                        actual,
                        substitutions,
                    ) {
                        Some(true) => {}
                        Some(false) => return Some(false),
                        None => return None,
                    }
                }
                Some(true)
            }
            TypeKind::Record(fields) => {
                let SourceOptionActualType::Record(actual_fields) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                if fields.len() != actual_fields.len() {
                    return Some(false);
                }
                let actual_fields = actual_fields
                    .iter()
                    .map(|field| (field.name.as_str(), &field.ty))
                    .collect::<HashMap<_, _>>();
                for expected in fields {
                    let Some(actual) = actual_fields.get(expected.label.text()) else {
                        return Some(false);
                    };
                    match self.source_option_hir_type_matches_actual_type(
                        expected.ty,
                        actual,
                        substitutions,
                    ) {
                        Some(true) => {}
                        Some(false) => return Some(false),
                        None => return None,
                    }
                }
                Some(true)
            }
            TypeKind::Arrow { parameter, result } => {
                let SourceOptionActualType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                match self.source_option_hir_type_matches_actual_type(
                    *parameter,
                    actual_parameter,
                    substitutions,
                ) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => return None,
                }
                self.source_option_hir_type_matches_actual_type(
                    *result,
                    actual_result,
                    substitutions,
                )
            }
        }
    }

    fn source_option_hir_type_reference_matches_actual_type(
        &self,
        reference: &TypeReference,
        actual: &SourceOptionActualType,
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
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
            )) => Some(match actual {
                SourceOptionActualType::Hole => true,
                SourceOptionActualType::Primitive(actual_builtin) => actual_builtin == builtin,
                _ => false,
            }),
            ResolutionState::Resolved(TypeResolution::Builtin(_)) => None,
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                Some(match substitutions.entry(*parameter) {
                    Entry::Occupied(mut entry) => match entry.get().unify(actual) {
                        Some(unified) => {
                            entry.insert(unified);
                            true
                        }
                        None => false,
                    },
                    Entry::Vacant(entry) => {
                        entry.insert(actual.clone());
                        true
                    }
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => self
                .source_option_hir_item_matches_actual_type(*item_id, &[], actual, substitutions),
            ResolutionState::Resolved(TypeResolution::Import(_)) => None,
        }
    }

    fn source_option_hir_type_application_matches_actual_type(
        &self,
        callee: TypeId,
        arguments: &[TypeId],
        actual: &SourceOptionActualType,
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                let SourceOptionActualType::List(actual) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                let SourceOptionActualType::Map {
                    key: actual_key,
                    value: actual_value,
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                match self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual_key,
                    substitutions,
                ) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => return None,
                }
                self.source_option_hir_type_matches_actual_type(
                    *arguments.get(1)?,
                    actual_value,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                let SourceOptionActualType::Set(actual) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                let SourceOptionActualType::Option(actual) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                let SourceOptionActualType::Result {
                    error: actual_error,
                    value: actual_value,
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                match self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual_error,
                    substitutions,
                ) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => return None,
                }
                self.source_option_hir_type_matches_actual_type(
                    *arguments.get(1)?,
                    actual_value,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                let SourceOptionActualType::Validation {
                    error: actual_error,
                    value: actual_value,
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                match self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual_error,
                    substitutions,
                ) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => return None,
                }
                self.source_option_hir_type_matches_actual_type(
                    *arguments.get(1)?,
                    actual_value,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                let SourceOptionActualType::Signal(actual) = actual else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                let SourceOptionActualType::Task {
                    error: actual_error,
                    value: actual_value,
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                match self.source_option_hir_type_matches_actual_type(
                    *arguments.first()?,
                    actual_error,
                    substitutions,
                ) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => return None,
                }
                self.source_option_hir_type_matches_actual_type(
                    *arguments.get(1)?,
                    actual_value,
                    substitutions,
                )
            }
            ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Unresolved => None,
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => self
                .source_option_hir_item_matches_actual_type(
                    *item_id,
                    arguments,
                    actual,
                    substitutions,
                ),
        }
    }

    fn source_option_hir_item_matches_actual_type(
        &self,
        item_id: ItemId,
        expected_arguments: &[TypeId],
        actual: &SourceOptionActualType,
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
        match &self.module.items()[item_id] {
            Item::Domain(_) => {
                let SourceOptionActualType::Domain {
                    item, arguments, ..
                } = actual
                else {
                    return Some(matches!(actual, SourceOptionActualType::Hole));
                };
                if *item != item_id {
                    return Some(false);
                }
                self.source_option_hir_type_arguments_match_actual_types(
                    expected_arguments,
                    arguments,
                    substitutions,
                )
            }
            Item::Type(item) => match &item.body {
                TypeItemBody::Alias(_) => None,
                TypeItemBody::Sum(_) => {
                    let SourceOptionActualType::OpaqueItem {
                        item, arguments, ..
                    } = actual
                    else {
                        return Some(matches!(actual, SourceOptionActualType::Hole));
                    };
                    if *item != item_id {
                        return Some(false);
                    }
                    self.source_option_hir_type_arguments_match_actual_types(
                        expected_arguments,
                        arguments,
                        substitutions,
                    )
                }
            },
            Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::Class(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        }
    }

    fn source_option_hir_type_arguments_match_actual_types(
        &self,
        expected: &[TypeId],
        actual: &[SourceOptionActualType],
        substitutions: &mut HashMap<TypeParameterId, SourceOptionActualType>,
    ) -> Option<bool> {
        if expected.len() != actual.len() {
            return Some(false);
        }
        for (expected, actual) in expected.iter().copied().zip(actual) {
            match self.source_option_hir_type_matches_actual_type(expected, actual, substitutions) {
                Some(true) => {}
                Some(false) => return Some(false),
                None => return None,
            }
        }
        Some(true)
    }

    fn source_option_hir_type_is_signal_contract(&self, ty: TypeId) -> bool {
        let TypeKind::Apply { callee, .. } = &self.module.types()[ty].kind else {
            return false;
        };
        let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
            return false;
        };
        matches!(
            reference.resolution.as_ref(),
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal))
        )
    }

    fn source_option_constructor_field_expectations(
        &self,
        parent_item: ItemId,
        expected_parent: &SourceOptionNamedType,
        field_types: &[TypeId],
    ) -> Option<Vec<SourceOptionExpectedType>> {
        let Item::Type(item) = &self.module.items()[parent_item] else {
            return None;
        };
        if item.parameters.len() != expected_parent.arguments.len() {
            return None;
        }

        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(expected_parent.arguments.iter().cloned())
            .collect::<HashMap<_, _>>();

        field_types
            .iter()
            .map(|field| {
                SourceOptionExpectedType::from_hir_type(
                    self.module,
                    *field,
                    &substitutions,
                    SourceOptionTypeSurface::Expression,
                )
            })
            .collect()
    }

    fn check_source_option_list(
        &self,
        elements: &[ExprId],
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        let mut saw_unknown = false;

        for element in elements {
            match self.check_source_option_expr_inner(
                *element,
                expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return SourceOptionTypeCheck::Mismatch(mismatch);
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            SourceOptionTypeCheck::Match
        }
    }

    fn check_source_option_map(
        &self,
        map: &crate::MapExpr,
        key_expected: &SourceOptionExpectedType,
        value_expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        let mut saw_unknown = false;

        for entry in &map.entries {
            for (expr_id, expected) in [(entry.key, key_expected), (entry.value, value_expected)] {
                match self.check_source_option_expr_inner(
                    expr_id,
                    expected,
                    typing,
                    bindings,
                    value_stack,
                ) {
                    SourceOptionTypeCheck::Match => {}
                    SourceOptionTypeCheck::Mismatch(mismatch) => {
                        return SourceOptionTypeCheck::Mismatch(mismatch);
                    }
                    SourceOptionTypeCheck::Unknown => saw_unknown = true,
                }
            }
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            SourceOptionTypeCheck::Match
        }
    }

    fn check_source_option_set(
        &self,
        elements: &[ExprId],
        expected: &SourceOptionExpectedType,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        let mut saw_unknown = false;

        for element in elements {
            match self.check_source_option_expr_inner(
                *element,
                expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return SourceOptionTypeCheck::Mismatch(mismatch);
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            SourceOptionTypeCheck::Match
        }
    }

    fn check_source_option_tuple(
        &self,
        expr_id: ExprId,
        elements: &crate::AtLeastTwo<ExprId>,
        expected: &[SourceOptionExpectedType],
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        if elements.len() != expected.len() {
            return self.check_source_option_expr_by_inference_or_unknown(
                expr_id,
                &SourceOptionExpectedType::Tuple(expected.to_vec()),
                typing,
                bindings,
                value_stack,
            );
        }

        let mut saw_unknown = false;

        for (element, expected) in elements.iter().zip(expected) {
            match self.check_source_option_expr_inner(
                *element,
                expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return SourceOptionTypeCheck::Mismatch(mismatch);
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            SourceOptionTypeCheck::Match
        }
    }

    fn check_source_option_record(
        &self,
        expr_id: ExprId,
        record: &crate::RecordExpr,
        expected: &[SourceOptionExpectedRecordField],
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
        value_stack: &mut Vec<ItemId>,
    ) -> SourceOptionTypeCheck {
        if record.fields.len() != expected.len() {
            return self.check_source_option_expr_by_inference_or_unknown(
                expr_id,
                &SourceOptionExpectedType::Record(expected.to_vec()),
                typing,
                bindings,
                value_stack,
            );
        }

        let expected_fields = expected
            .iter()
            .map(|field| (field.name.as_str(), &field.ty))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::<String>::new();
        let mut saw_unknown = false;

        for field in &record.fields {
            let Some(field_expected) = expected_fields.get(field.label.text()) else {
                return self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    &SourceOptionExpectedType::Record(expected.to_vec()),
                    typing,
                    bindings,
                    value_stack,
                );
            };
            seen.insert(field.label.text().to_owned());
            match self.check_source_option_expr_inner(
                field.value,
                field_expected,
                typing,
                bindings,
                value_stack,
            ) {
                SourceOptionTypeCheck::Match => {}
                SourceOptionTypeCheck::Mismatch(mismatch) => {
                    return SourceOptionTypeCheck::Mismatch(mismatch);
                }
                SourceOptionTypeCheck::Unknown => saw_unknown = true,
            }
        }

        if seen.len() != expected.len() {
            return self.check_source_option_expr_by_inference_or_unknown(
                expr_id,
                &SourceOptionExpectedType::Record(expected.to_vec()),
                typing,
                bindings,
                value_stack,
            );
        }

        if saw_unknown {
            SourceOptionTypeCheck::Unknown
        } else {
            SourceOptionTypeCheck::Match
        }
    }

    fn source_option_monomorphic_constructor_expected(
        &self,
        parent_item: ItemId,
    ) -> Option<SourceOptionExpectedType> {
        let Item::Type(item) = &self.module.items()[parent_item] else {
            return None;
        };
        if !item.parameters.is_empty() {
            return None;
        }
        Some(SourceOptionExpectedType::Named(
            SourceOptionNamedType::from_item(self.module, parent_item, Vec::new())?,
        ))
    }

    fn source_constructor_actual(
        &self,
        reference: &TermReference,
    ) -> Option<SourceOptionConstructorActual> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Type(item) = &self.module.items()[*item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let constructor_name = reference.path.segments().iter().last()?.text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == constructor_name)?;
        Some(SourceOptionConstructorActual {
            parent_item: *item_id,
            parent_name: item.name.text().to_owned(),
            constructor_name: constructor_name.to_owned(),
            field_types: variant.fields.clone(),
        })
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

    /// Merged pipe-semantics validation pass.
    ///
    /// Replaces the five formerly separate passes (`validate_fanout_semantics`,
    /// `validate_gate_semantics`, `validate_truthy_falsy_semantics`,
    /// `validate_case_exhaustiveness`, `validate_recurrence_targets`).
    ///
    /// The five passes shared identical item-cloning and `GateTypeContext`
    /// construction steps, causing every item body to be traversed five times
    /// with five freshly-built typing contexts on every validation run.  This
    /// single pass:
    ///  - Clones items once.
    ///  - Builds one `GateTypeContext` (interning tables are preserved across
    ///    all five per-operator dispatch calls at each pipe expression site).
    ///  - Walks each item body once, dispatching to each operator's private
    ///    validation method inside the same `walk_expr_tree` callback.
    ///
    /// The public interface of `Validator::run` is unchanged.
    fn validate_pipe_semantics(&mut self) {
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
                    let env = GateExprEnv::default();
                    let target = item.annotation.and_then(|annotation| {
                        typing.recurrence_target_hint_for_annotation(annotation)
                    });
                    let wakeup =
                        self.recurrence_wakeup_hint_for_decorators(&item.header.decorators);
                    self.validate_fanout_expr_tree(item.body, &env, &mut typing);
                    self.validate_gate_expr_tree(item.body, &env, &mut typing);
                    self.validate_truthy_falsy_expr_tree(item.body, &env, &mut typing);
                    self.validate_case_exhaustiveness_expr_tree(item.body, &env, &mut typing);
                    self.validate_recurrence_expr_tree(
                        item.body,
                        target,
                        wakeup,
                        &env,
                        &mut typing,
                    );
                }
                Item::Function(item) => {
                    let env = self.gate_env_for_function(&item, &mut typing);
                    let target = item.annotation.and_then(|annotation| {
                        typing.recurrence_target_hint_for_annotation(annotation)
                    });
                    let wakeup =
                        self.recurrence_wakeup_hint_for_decorators(&item.header.decorators);
                    self.validate_fanout_expr_tree(item.body, &env, &mut typing);
                    self.validate_gate_expr_tree(item.body, &env, &mut typing);
                    self.validate_truthy_falsy_expr_tree(item.body, &env, &mut typing);
                    self.validate_case_exhaustiveness_expr_tree(item.body, &env, &mut typing);
                    self.validate_recurrence_expr_tree(
                        item.body,
                        target,
                        wakeup,
                        &env,
                        &mut typing,
                    );
                }
                Item::Signal(item) => {
                    if let Some(body) = item.body {
                        let env = GateExprEnv::default();
                        let wakeup = self.recurrence_wakeup_hint_for_signal(&item);
                        self.validate_fanout_expr_tree(body, &env, &mut typing);
                        self.validate_gate_expr_tree(body, &env, &mut typing);
                        self.validate_truthy_falsy_expr_tree(body, &env, &mut typing);
                        self.validate_case_exhaustiveness_expr_tree(body, &env, &mut typing);
                        self.validate_recurrence_expr_tree(
                            body,
                            Some(RecurrenceTargetHint::Evidence(
                                RecurrenceTargetEvidence::SignalItemBody,
                            )),
                            wakeup,
                            &env,
                            &mut typing,
                        );
                    }
                }
                Item::Instance(item) => {
                    for member in item.members {
                        let env = GateExprEnv::default();
                        let target = member.annotation.and_then(|annotation| {
                            typing.recurrence_target_hint_for_annotation(annotation)
                        });
                        self.validate_fanout_expr_tree(member.body, &env, &mut typing);
                        self.validate_gate_expr_tree(member.body, &env, &mut typing);
                        self.validate_truthy_falsy_expr_tree(member.body, &env, &mut typing);
                        self.validate_case_exhaustiveness_expr_tree(member.body, &env, &mut typing);
                        self.validate_recurrence_expr_tree(
                            member.body,
                            target,
                            None,
                            &env,
                            &mut typing,
                        );
                    }
                }
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }

        // Decorator expressions: only case-exhaustiveness and recurrence need them.
        for decorator in decorators {
            match decorator.payload {
                DecoratorPayload::Bare => {}
                DecoratorPayload::Call(call) => {
                    let env = GateExprEnv::default();
                    for argument in &call.arguments {
                        self.validate_case_exhaustiveness_expr_tree(*argument, &env, &mut typing);
                        self.validate_recurrence_expr_tree(
                            *argument,
                            None,
                            None,
                            &env,
                            &mut typing,
                        );
                    }
                    if let Some(options) = call.options {
                        self.validate_case_exhaustiveness_expr_tree(options, &env, &mut typing);
                        self.validate_recurrence_expr_tree(options, None, None, &env, &mut typing);
                    }
                }
                DecoratorPayload::RecurrenceWakeup(wakeup) => {
                    let env = GateExprEnv::default();
                    self.validate_case_exhaustiveness_expr_tree(wakeup.witness, &env, &mut typing);
                    self.validate_recurrence_expr_tree(
                        wakeup.witness,
                        None,
                        None,
                        &env,
                        &mut typing,
                    );
                }
                DecoratorPayload::Source(source) => {
                    let env = GateExprEnv::default();
                    for argument in &source.arguments {
                        self.validate_case_exhaustiveness_expr_tree(*argument, &env, &mut typing);
                        self.validate_recurrence_expr_tree(
                            *argument,
                            None,
                            None,
                            &env,
                            &mut typing,
                        );
                    }
                    if let Some(options) = source.options {
                        self.validate_case_exhaustiveness_expr_tree(options, &env, &mut typing);
                        self.validate_recurrence_expr_tree(options, None, None, &env, &mut typing);
                    }
                }
            }
        }
    }

    fn validate_case_exhaustiveness_expr_tree(
        &mut self,
        root: ExprId,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let mut work = vec![CaseExhaustivenessWork::Expr {
            expr: root,
            env: env.clone(),
        }];

        while let Some(frame) = work.pop() {
            match frame {
                CaseExhaustivenessWork::Expr { expr, env } => {
                    let Some(expr) = self.module.exprs().get(expr).cloned() else {
                        continue;
                    };
                    match expr.kind {
                        ExprKind::Name(_)
                        | ExprKind::Integer(_)
                        | ExprKind::Float(_)
                        | ExprKind::Decimal(_)
                        | ExprKind::BigInt(_)
                        | ExprKind::SuffixedInteger(_)
                        | ExprKind::Regex(_) => {}
                        ExprKind::Text(text) => {
                            for segment in text.segments.into_iter().rev() {
                                if let TextSegment::Interpolation(interpolation) = segment {
                                    work.push(CaseExhaustivenessWork::Expr {
                                        expr: interpolation.expr,
                                        env: env.clone(),
                                    });
                                }
                            }
                        }
                        ExprKind::Tuple(elements) => {
                            for element in elements.iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: *element,
                                    env: env.clone(),
                                });
                            }
                        }
                        ExprKind::List(elements) => {
                            for element in elements.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: element,
                                    env: env.clone(),
                                });
                            }
                        }
                        ExprKind::Map(map) => {
                            for entry in map.entries.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: entry.value,
                                    env: env.clone(),
                                });
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: entry.key,
                                    env: env.clone(),
                                });
                            }
                        }
                        ExprKind::Set(elements) => {
                            for element in elements.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: element,
                                    env: env.clone(),
                                });
                            }
                        }
                        ExprKind::Record(record) => {
                            for field in record.fields.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: field.value,
                                    env: env.clone(),
                                });
                            }
                        }
                        ExprKind::Projection {
                            base: crate::hir::ProjectionBase::Expr(base),
                            ..
                        } => work.push(CaseExhaustivenessWork::Expr { expr: base, env }),
                        ExprKind::Projection { .. } => {}
                        ExprKind::Apply { callee, arguments } => {
                            for argument in arguments.iter().rev() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: *argument,
                                    env: env.clone(),
                                });
                            }
                            work.push(CaseExhaustivenessWork::Expr { expr: callee, env });
                        }
                        ExprKind::Unary { expr, .. } => {
                            work.push(CaseExhaustivenessWork::Expr { expr, env });
                        }
                        ExprKind::Binary { left, right, .. } => {
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: right,
                                env: env.clone(),
                            });
                            work.push(CaseExhaustivenessWork::Expr { expr: left, env });
                        }
                        ExprKind::Pipe(pipe) => {
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: pipe.head,
                                env: env.clone(),
                            });
                            let stages = pipe.stages.iter().collect::<Vec<_>>();
                            let mut current = self.infer_case_expr_type(pipe.head, &env, typing);
                            let mut stage_index = 0usize;
                            while stage_index < stages.len() {
                                let stage = stages[stage_index];
                                match &stage.kind {
                                    PipeStageKind::Transform { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        current = current.as_ref().and_then(|subject| {
                                            typing.infer_transform_stage(*expr, &env, subject)
                                        });
                                        stage_index += 1;
                                    }
                                    PipeStageKind::Tap { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        stage_index += 1;
                                    }
                                    PipeStageKind::Gate { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        current = current.as_ref().and_then(|subject| {
                                            typing.infer_gate_stage(*expr, &env, subject)
                                        });
                                        stage_index += 1;
                                    }
                                    PipeStageKind::Map { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        current = current.as_ref().and_then(|subject| {
                                            typing.infer_fanout_map_stage(*expr, &env, subject)
                                        });
                                        stage_index += 1;
                                    }
                                    PipeStageKind::FanIn { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        current = current.as_ref().and_then(|subject| {
                                            typing.infer_fanin_stage(*expr, &env, subject)
                                        });
                                        stage_index += 1;
                                    }
                                    PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                                        let Some(pair) =
                                            truthy_falsy_pair_stages(&stages, stage_index)
                                        else {
                                            let branch_expr = match &stage.kind {
                                                PipeStageKind::Truthy { expr }
                                                | PipeStageKind::Falsy { expr } => *expr,
                                                _ => unreachable!(
                                                    "truthy/falsy branch extraction should stay aligned"
                                                ),
                                            };
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: branch_expr,
                                                env: env.clone(),
                                            });
                                            current = None;
                                            stage_index += 1;
                                            continue;
                                        };
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: pair.falsy_expr,
                                            env: env.clone(),
                                        });
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: pair.truthy_expr,
                                            env: env.clone(),
                                        });
                                        current = current.as_ref().and_then(|subject| {
                                            typing.infer_truthy_falsy_pair(&pair, &env, subject)
                                        });
                                        stage_index = pair.next_index;
                                    }
                                    PipeStageKind::Case { .. } => {
                                        let case_start = stage_index;
                                        while stage_index < stages.len()
                                            && matches!(
                                                stages[stage_index].kind,
                                                PipeStageKind::Case { .. }
                                            )
                                        {
                                            stage_index += 1;
                                        }
                                        let case_stages = &stages[case_start..stage_index];
                                        if let Some(subject) = current.clone() {
                                            self.validate_pipe_case_run(
                                                case_stages,
                                                &subject,
                                                typing,
                                            );
                                            for case_stage in case_stages.iter().rev() {
                                                let PipeStageKind::Case { pattern, body } =
                                                    &case_stage.kind
                                                else {
                                                    continue;
                                                };
                                                work.push(CaseExhaustivenessWork::Expr {
                                                    expr: *body,
                                                    env: self.case_branch_env(
                                                        &env, *pattern, &subject, typing,
                                                    ),
                                                });
                                            }
                                        } else {
                                            for case_stage in case_stages.iter().rev() {
                                                let PipeStageKind::Case { body, .. } =
                                                    &case_stage.kind
                                                else {
                                                    continue;
                                                };
                                                work.push(CaseExhaustivenessWork::Expr {
                                                    expr: *body,
                                                    env: env.clone(),
                                                });
                                            }
                                        }
                                        current = None;
                                    }
                                    PipeStageKind::Apply { expr }
                                    | PipeStageKind::RecurStart { expr }
                                    | PipeStageKind::RecurStep { expr } => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr: *expr,
                                            env: env.clone(),
                                        });
                                        current = None;
                                        stage_index += 1;
                                    }
                                }
                            }
                        }
                        ExprKind::Cluster(cluster_id) => {
                            let Some(cluster) = self.module.clusters().get(cluster_id).cloned()
                            else {
                                continue;
                            };
                            let spine = cluster.normalized_spine();
                            for member in spine.apply_arguments() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: member,
                                    env: env.clone(),
                                });
                            }
                            if let ApplicativeSpineHead::Expr(finalizer) = spine.pure_head() {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: finalizer,
                                    env,
                                });
                            }
                        }
                        ExprKind::Markup(node_id) => {
                            work.push(CaseExhaustivenessWork::Markup { node: node_id, env });
                        }
                    }
                }
                CaseExhaustivenessWork::Markup { node, env } => {
                    let Some(node) = self.module.markup_nodes().get(node).cloned() else {
                        continue;
                    };
                    match node.kind {
                        MarkupNodeKind::Element(element) => {
                            for child in element.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                            for attribute in element.attributes.into_iter().rev() {
                                match attribute.value {
                                    MarkupAttributeValue::Expr(expr) => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr,
                                            env: env.clone(),
                                        });
                                    }
                                    MarkupAttributeValue::Text(text) => {
                                        for segment in text.segments.into_iter().rev() {
                                            if let TextSegment::Interpolation(interpolation) =
                                                segment
                                            {
                                                work.push(CaseExhaustivenessWork::Expr {
                                                    expr: interpolation.expr,
                                                    env: env.clone(),
                                                });
                                            }
                                        }
                                    }
                                    MarkupAttributeValue::ImplicitTrue => {}
                                }
                            }
                        }
                        MarkupNodeKind::Control(control_id) => {
                            work.push(CaseExhaustivenessWork::Control {
                                node: control_id,
                                env,
                            });
                        }
                    }
                }
                CaseExhaustivenessWork::Control { node, env } => {
                    let Some(control) = self.module.control_nodes().get(node).cloned() else {
                        continue;
                    };
                    match control {
                        ControlNode::Show(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                            if let Some(keep_mounted) = node.keep_mounted {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: keep_mounted,
                                    env: env.clone(),
                                });
                            }
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: node.when,
                                env,
                            });
                        }
                        ControlNode::Each(node) => {
                            if let Some(empty) = node.empty {
                                work.push(CaseExhaustivenessWork::Control {
                                    node: empty,
                                    env: env.clone(),
                                });
                            }
                            let child_env = self.each_child_env(&env, &node, typing);
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: child_env.clone(),
                                });
                            }
                            if let Some(key) = node.key {
                                work.push(CaseExhaustivenessWork::Expr {
                                    expr: key,
                                    env: child_env,
                                });
                            }
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: node.collection,
                                env,
                            });
                        }
                        ControlNode::Empty(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        ControlNode::Match(node) => {
                            let subject = self.infer_case_expr_type(node.scrutinee, &env, typing);
                            if let Some(subject) = subject.as_ref() {
                                self.validate_match_control_exhaustiveness(&node, subject, typing);
                            }
                            for case in node.cases.iter().rev() {
                                let case_env = subject
                                    .as_ref()
                                    .and_then(|subject| {
                                        match self.module.control_nodes().get(*case) {
                                            Some(ControlNode::Case(case_node)) => {
                                                Some(self.case_branch_env(
                                                    &env,
                                                    case_node.pattern,
                                                    subject,
                                                    typing,
                                                ))
                                            }
                                            _ => None,
                                        }
                                    })
                                    .unwrap_or_else(|| env.clone());
                                work.push(CaseExhaustivenessWork::Control {
                                    node: *case,
                                    env: case_env,
                                });
                            }
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: node.scrutinee,
                                env,
                            });
                        }
                        ControlNode::Case(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        ControlNode::Fragment(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        ControlNode::With(node) => {
                            let child_env = self.with_child_env(&env, &node, typing);
                            for child in node.children.into_iter().rev() {
                                work.push(CaseExhaustivenessWork::Markup {
                                    node: child,
                                    env: child_env.clone(),
                                });
                            }
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: node.value,
                                env,
                            });
                        }
                    }
                }
            }
        }
    }

    fn validate_pipe_case_run(
        &mut self,
        case_stages: &[&crate::hir::PipeStage],
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) {
        let Some(shape) = typing.case_subject_shape(subject) else {
            return;
        };
        let mut covered = HashSet::new();
        let mut has_catch_all = false;
        for stage in case_stages {
            let PipeStageKind::Case { pattern, .. } = &stage.kind else {
                continue;
            };
            match typing.case_pattern_coverage(*pattern, &shape) {
                CasePatternCoverage::CatchAll => {
                    has_catch_all = true;
                    break;
                }
                CasePatternCoverage::Constructor(key) => {
                    covered.insert(key);
                }
                CasePatternCoverage::None => {}
            }
        }
        if has_catch_all {
            return;
        }
        let missing = shape
            .constructors
            .iter()
            .filter(|constructor| !covered.contains(&constructor.key))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        let span = case_stages
            .first()
            .map(|stage| stage.span)
            .unwrap_or_else(SourceSpan::default);
        self.emit_non_exhaustive_case_diagnostic(CaseSiteKind::PipeCase, span, subject, &missing);
    }

    fn validate_match_control_exhaustiveness(
        &mut self,
        match_node: &crate::hir::MatchControl,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) {
        let Some(shape) = typing.case_subject_shape(subject) else {
            return;
        };
        let mut covered = HashSet::new();
        let mut has_catch_all = false;
        for case in match_node.cases.iter() {
            let Some(ControlNode::Case(case_node)) = self.module.control_nodes().get(*case) else {
                continue;
            };
            match typing.case_pattern_coverage(case_node.pattern, &shape) {
                CasePatternCoverage::CatchAll => {
                    has_catch_all = true;
                    break;
                }
                CasePatternCoverage::Constructor(key) => {
                    covered.insert(key);
                }
                CasePatternCoverage::None => {}
            }
        }
        if has_catch_all {
            return;
        }
        let missing = shape
            .constructors
            .iter()
            .filter(|constructor| !covered.contains(&constructor.key))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        self.emit_non_exhaustive_case_diagnostic(
            CaseSiteKind::MatchControl,
            match_node.span,
            subject,
            &missing,
        );
    }

    fn emit_non_exhaustive_case_diagnostic(
        &mut self,
        site_kind: CaseSiteKind,
        span: SourceSpan,
        subject: &GateType,
        missing: &[CaseConstructorShape],
    ) {
        let missing_list = missing_case_list(missing);
        let mut diagnostic = Diagnostic::error(format!(
            "{} over `{subject}` is not exhaustive; missing {missing_list}",
            site_kind.display_name()
        ))
        .with_code(code("non-exhaustive-case-pattern"))
        .with_primary_label(span, missing_case_label(missing));

        for constructor in missing {
            if let Some(declared_at) = constructor.span {
                diagnostic = diagnostic.with_secondary_label(
                    declared_at,
                    format!("`{}` is declared here", constructor.display),
                );
            }
        }

        diagnostic = diagnostic.with_note(
            "current resolved-HIR exhaustiveness checking covers only ordinary `Bool`, `Option`, `Result`, `Validation`, and same-module closed sums whose scrutinee type is already known here; signal-lifted case splits, imported sums, and harder unannotated scrutinee inference remain later work",
        );
        self.diagnostics.push(diagnostic);
    }

    fn infer_case_expr_type(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        if !self.module.exprs().contains(expr_id) {
            return None;
        }
        typing.infer_expr(expr_id, env, None).ty
    }

    fn case_branch_env(
        &mut self,
        env: &GateExprEnv,
        pattern: PatternId,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> GateExprEnv {
        let mut branch_env = env.clone();
        branch_env
            .locals
            .extend(typing.case_pattern_bindings(pattern, subject).locals);
        branch_env
    }

    fn each_child_env(
        &mut self,
        env: &GateExprEnv,
        each: &crate::hir::EachControl,
        typing: &mut GateTypeContext<'_>,
    ) -> GateExprEnv {
        let mut child_env = env.clone();
        if let Some(element_ty) = self
            .infer_case_expr_type(each.collection, env, typing)
            .and_then(|collection| collection.fanout_element().cloned())
        {
            child_env.locals.insert(each.binding, element_ty);
        }
        child_env
    }

    fn with_child_env(
        &mut self,
        env: &GateExprEnv,
        with_node: &crate::hir::WithControl,
        typing: &mut GateTypeContext<'_>,
    ) -> GateExprEnv {
        let mut child_env = env.clone();
        if let Some(value_ty) = self.infer_case_expr_type(with_node.value, env, typing) {
            child_env.locals.insert(with_node.binding, value_ty);
        }
        child_env
    }

    fn validate_recurrence_expr_tree(
        &mut self,
        root: ExprId,
        root_target: Option<RecurrenceTargetHint>,
        root_wakeup: Option<RecurrenceWakeupHint>,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, is_root| {
            if let ExprKind::Pipe(pipe) = &expr.kind {
                let target = if is_root { root_target.as_ref() } else { None };
                let wakeup = if is_root { root_wakeup.as_ref() } else { None };
                self.validate_recurrence_pipe(pipe, target, wakeup, is_root, env, typing);
            }
        });
    }

    fn validate_recurrence_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        target: Option<&RecurrenceTargetHint>,
        wakeup: Option<&RecurrenceWakeupHint>,
        is_root: bool,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
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
            Some(RecurrenceWakeupHint::CustomSource { context, .. }) => {
                if RecurrenceWakeupPlanner::plan_custom_source(*context).is_err() {
                    self.emit_missing_recurrence_wakeup(start_span, wakeup);
                }
            }
            Some(RecurrenceWakeupHint::NonSource(cause)) => {
                if RecurrenceWakeupPlanner::plan_non_source(*cause).is_err() {
                    self.emit_missing_recurrence_wakeup(start_span, wakeup);
                }
            }
            None => {
                self.emit_missing_recurrence_wakeup(start_span, wakeup);
            }
        }

        let Some(input_subject) = self.infer_recurrence_input_subject_for_validation(
            pipe,
            suffix.prefix_stage_count(),
            env,
            typing,
        ) else {
            return;
        };
        let start_info = typing.infer_pipe_body(suffix.start_expr(), env, &input_subject);
        if !start_info.issues.is_empty() {
            return;
        }
        let Some(start_subject) = start_info.ty else {
            return;
        };
        for stage in suffix.guard_stages() {
            let PipeStageKind::Gate { expr } = stage.kind else {
                unreachable!("validated recurrence guards must use `?|>`");
            };
            let _ = self.validate_gate_stage(stage.span, expr, env, &start_subject, typing);
        }
    }

    fn validate_joined_fanout_segment(
        &mut self,
        segment: &crate::PipeFanoutSegment<'_>,
        env: &GateExprEnv,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let Some(carrier) = typing.fanout_carrier(subject) else {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "fan-out `*|>` requires `List A` or `Signal (List A)`, found `{subject}`"
                ))
                .with_code(code("fanout-subject-not-list"))
                .with_label(DiagnosticLabel::primary(
                    segment.map_stage().span,
                    "map over a list-valued subject or transform to `List` first",
                )),
            );
            return None;
        };
        let Some(element_subject) = subject.fanout_element().cloned() else {
            return None;
        };
        let body_info = typing.infer_pipe_body(segment.map_expr(), env, &element_subject);
        let mut saw_error = false;
        for issue in body_info.issues {
            self.emit_fanout_issue(FanoutIssueContext::MapElement, issue);
            saw_error = true;
        }
        if saw_error {
            return None;
        }
        let mapped_element_type = body_info.ty?;
        for stage in segment.filter_stages() {
            let PipeStageKind::Gate { expr } = stage.kind else {
                unreachable!("validated fan-out filters must use `?|>`");
            };
            if self
                .validate_fanout_filter_stage(stage.span, expr, env, &mapped_element_type, typing)
                .is_none()
            {
                return None;
            }
        }
        let mapped_collection_type = typing.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Map, carrier),
            mapped_element_type,
        );
        match segment.join_expr() {
            Some(join_expr) => self.validate_fanin_stage(
                segment
                    .join_stage()
                    .expect("join expression implies join stage")
                    .span,
                join_expr,
                env,
                &mapped_collection_type,
                typing,
            ),
            None => Some(mapped_collection_type),
        }
    }

    fn validate_fanout_filter_stage(
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
                        "compute a `Bool` from the current fan-out element instead of sampling a signal here",
                    )),
            );
            saw_error = true;
        }
        // Gate predicates (fan-out filters) must evaluate to `Bool`. This invariant is fundamental
        // to the gate/filter semantic: the runtime uses the predicate result to decide whether to
        // forward or discard each fan-out element. Any other result type is a type error that must
        // be reported here rather than deferred to a later pass.
        if let Some(predicate_ty) = predicate_info.ty {
            if !predicate_ty.is_bool() {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate must produce `Bool`, found `{predicate_ty}`"
                    ))
                    .with_code(code("gate-predicate-not-bool"))
                    .with_label(DiagnosticLabel::primary(
                        self.module.exprs()[predicate].span,
                        "this fan-out filter does not evaluate to `Bool` for the current element",
                    )),
                );
                saw_error = true;
            }
        } else {
            saw_error = true;
        }
        if saw_error {
            return None;
        }
        Some(subject.clone())
    }

    fn infer_recurrence_input_subject_for_validation(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        prefix_stage_count: usize,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let stages = pipe
            .stages
            .iter()
            .take(prefix_stage_count)
            .collect::<Vec<_>>();
        let mut current = typing.infer_expr(pipe.head, env, None).ty?;
        let mut stage_index = 0usize;
        while stage_index < stages.len() {
            let stage = stages[stage_index];
            match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    current = typing.infer_transform_stage(*expr, env, &current)?;
                    stage_index += 1;
                }
                PipeStageKind::Tap { expr } => {
                    let _ = typing.infer_pipe_body(*expr, env, &current);
                    stage_index += 1;
                }
                PipeStageKind::Gate { expr } => {
                    current = typing.infer_gate_stage(*expr, env, &current)?;
                    stage_index += 1;
                }
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        let outcome = crate::fanout_elaboration::elaborate_fanout_segment(
                            self.module,
                            &segment,
                            Some(&current),
                            env,
                            typing,
                        );
                        current = match outcome {
                            crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                plan.result_type
                            }
                            crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => {
                                return None;
                            }
                        };
                        stage_index = segment.next_stage_index();
                    } else {
                        current = typing.infer_fanout_map_stage(*expr, env, &current)?;
                        stage_index += 1;
                    }
                }
                PipeStageKind::FanIn { expr } => {
                    current = typing.infer_fanin_stage(*expr, env, &current)?;
                    stage_index += 1;
                }
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let pair = truthy_falsy_pair_stages(&stages, stage_index)?;
                    current = typing.infer_truthy_falsy_pair(&pair, env, &current)?;
                    stage_index = pair.next_index;
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => return None,
            }
        }
        Some(current)
    }

    fn recurrence_wakeup_hint_for_decorators(
        &self,
        decorators: &[DecoratorId],
    ) -> Option<RecurrenceWakeupHint> {
        decorators.iter().find_map(|decorator_id| {
            let decorator = self.module.decorators().get(*decorator_id)?;
            let DecoratorPayload::RecurrenceWakeup(ref wakeup) = decorator.payload else {
                return None;
            };
            Some(RecurrenceWakeupHint::NonSource(match wakeup.kind {
                RecurrenceWakeupDecoratorKind::Timer => NonSourceWakeupCause::ExplicitTimer,
                RecurrenceWakeupDecoratorKind::Backoff => NonSourceWakeupCause::ExplicitBackoff,
            }))
        })
    }

    fn recurrence_wakeup_hint_for_signal(&self, item: &SignalItem) -> Option<RecurrenceWakeupHint> {
        let Some(source) = self.signal_source_decorator(item) else {
            return self.recurrence_wakeup_hint_for_decorators(&item.header.decorators);
        };
        let provider = source.provider.as_ref()?;
        let metadata = item.source_metadata.as_ref();
        let provider_ref = metadata
            .map(|metadata| metadata.provider.clone())
            .unwrap_or_else(|| SourceProviderRef::from_path(Some(provider)));
        let provider = match provider_ref.builtin() {
            Some(provider) => provider,
            None => {
                let mut context = CustomSourceRecurrenceWakeupContext::new();
                if metadata.is_some_and(SourceMetadata::has_reactive_wakeup_inputs) {
                    context = context.with_reactive_inputs();
                }
                if let Some(wakeup) = metadata
                    .and_then(|metadata| metadata.custom_contract.clone())
                    .and_then(|contract| contract.recurrence_wakeup)
                {
                    context = context.with_declared_wakeup(custom_source_wakeup_kind(wakeup));
                }
                return Some(RecurrenceWakeupHint::CustomSource {
                    provider_path: provider.clone(),
                    context,
                });
            }
        };
        let mut context = SourceRecurrenceWakeupContext::new(provider);
        if metadata.is_some_and(SourceMetadata::has_reactive_wakeup_inputs) {
            context = context.with_reactive_inputs();
        }
        let contract = provider.contract();
        if let Some(options) = source.options {
            if let ExprKind::Record(record) = &self.module.exprs()[options].kind {
                for field in &record.fields {
                    let Some(cause) = contract
                        .wakeup_option(field.label.text())
                        .map(|option| builtin_source_option_wakeup_cause(option.cause()))
                    else {
                        continue;
                    };
                    context = match cause {
                        BuiltinSourceWakeupCause::RetryPolicy => context.with_retry_policy(),
                        BuiltinSourceWakeupCause::PollingPolicy => context.with_polling_policy(),
                        BuiltinSourceWakeupCause::TriggerSignal => context.with_signal_trigger(),
                        BuiltinSourceWakeupCause::ProviderTimer
                        | BuiltinSourceWakeupCause::ReactiveInputs
                        | BuiltinSourceWakeupCause::ProviderDefinedTrigger => context,
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
            Some(RecurrenceWakeupHint::CustomSource {
                provider_path,
                context: _,
            }) => {
                diagnostic = diagnostic
                    .with_primary_label(
                        span,
                        "this custom `@source` recurrence still needs reactive source inputs or explicit provider wakeup metadata",
                    )
                    .with_secondary_label(
                        provider_path.span(),
                        "custom providers do not inherit built-in `retry` / `refreshEvery` / `refreshOn` semantics without their own wakeup contract",
                    )
                    .with_note(
                        "reactive source arguments/options already prove source-event wakeups for any provider; timer/backoff/provider-trigger proof now comes only from a matching same-module `provider qualified.name` declaration such as `provider custom.feed` with `wakeup: ...`",
                    );
            }
            Some(RecurrenceWakeupHint::NonSource(cause)) => {
                let note = match cause {
                    NonSourceWakeupCause::ExplicitTimer => {
                        "this declaration already carries an explicit non-source timer witness; if this diagnostic appears, keep the failing fixture because the recurrence wakeup adapter is inconsistent"
                    }
                    NonSourceWakeupCause::ExplicitBackoff => {
                        "this declaration already carries an explicit non-source backoff witness; if this diagnostic appears, keep the failing fixture because the recurrence wakeup adapter is inconsistent"
                    }
                };
                diagnostic = diagnostic
                    .with_primary_label(
                        span,
                        "this recurrent pipe already carries an explicit non-source wakeup witness",
                    )
                    .with_note(note);
            }
            None => {
                diagnostic = diagnostic
                    .with_primary_label(
                        span,
                        "this recurrent pipe needs an explicit timer, backoff policy, source event, or provider-defined trigger",
                    )
                    .with_note(
                        "add a compiler-known non-source wakeup witness such as `@recur.timer 5s` or `@recur.backoff 3x`, or use a compiler-known `@source` provider with explicit wakeup proof",
                    );
            }
        }
        self.diagnostics.push(diagnostic);
    }

    fn validate_fanout_expr_tree(
        &mut self,
        root: ExprId,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, _| {
            if let ExprKind::Pipe(pipe) = &expr.kind {
                self.validate_fanout_pipe(pipe, env, typing);
            }
        });
    }

    fn validate_fanout_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let all_stages = pipe.stages.iter().collect::<Vec<_>>();
        PipeSubjectWalker::new(pipe, env, typing).walk(
            typing,
            |stage_index, stage, current, typing| match &stage.kind {
                PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_gate_stage(*expr, env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Map { .. } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        let next = segment.next_stage_index();
                        let new_subject = current.and_then(|s| {
                            self.validate_joined_fanout_segment(&segment, env, s, typing)
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject,
                            advance_by: next.saturating_sub(stage_index).max(1),
                        }
                    } else {
                        let new_subject = current.and_then(|s| {
                            self.validate_fanout_map_stage(
                                stage.span,
                                segment.map_expr(),
                                env,
                                s,
                                typing,
                            )
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject,
                            advance_by: 1,
                        }
                    }
                }
                PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| self.validate_fanin_stage(stage.span, *expr, env, s, typing)),
                    advance_by: 1,
                },
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&all_stages, stage_index) else {
                        return PipeSubjectStepOutcome::Continue {
                            new_subject: None,
                            advance_by: 1,
                        };
                    };
                    let new_subject =
                        current.and_then(|s| typing.infer_truthy_falsy_pair(&pair, env, s));
                    let advance = pair.next_index.saturating_sub(stage_index).max(1);
                    PipeSubjectStepOutcome::Continue {
                        new_subject,
                        advance_by: advance,
                    }
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                    advance_by: 1,
                },
                // Transform and Tap are handled by PipeSubjectWalker before the
                // callback is invoked; they can never reach this arm.
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!(
                        "Transform/Tap are consumed by PipeSubjectWalker before the callback"
                    )
                }
            },
        );
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
        // Limit the walk to the prefix before any recurrence suffix so
        // RecurStart/RecurStep are never seen by this pass (PA-M1).
        let limit = pipe
            .recurrence_suffix()
            .ok()
            .flatten()
            .map(|suffix| suffix.prefix_stage_count())
            .unwrap_or(pipe.stages.len());
        let all_stages = pipe.stages.iter().take(limit).collect::<Vec<_>>();
        PipeSubjectWalker::new_with_limit(pipe, env, typing, limit).walk(
            typing,
            |stage_index, stage, current, typing| match &stage.kind {
                PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| self.validate_gate_stage(stage.span, *expr, env, s, typing)),
                    advance_by: 1,
                },
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        // Use infer_fanout_segment_result_type instead of the
                        // full elaborate_fanout_segment to avoid re-running
                        // filter/join plan building that validate_fanout_semantics
                        // already performed (PA-H2).
                        let next = segment.next_stage_index();
                        let new_subject = current.and_then(|s| {
                            typing.infer_fanout_segment_result_type(&segment, env, s)
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject,
                            advance_by: next.saturating_sub(stage_index).max(1),
                        }
                    } else {
                        PipeSubjectStepOutcome::Continue {
                            new_subject: current
                                .and_then(|s| typing.infer_fanout_map_stage(*expr, env, s)),
                            advance_by: 1,
                        }
                    }
                }
                PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_fanin_stage(*expr, env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&all_stages, stage_index) else {
                        return PipeSubjectStepOutcome::Continue {
                            new_subject: None,
                            advance_by: 1,
                        };
                    };
                    let new_subject =
                        current.and_then(|s| typing.infer_truthy_falsy_pair(&pair, env, s));
                    let advance = pair.next_index.saturating_sub(stage_index).max(1);
                    PipeSubjectStepOutcome::Continue {
                        new_subject,
                        advance_by: advance,
                    }
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                    advance_by: 1,
                },
                // Transform and Tap are handled by PipeSubjectWalker before the
                // callback is invoked; they can never reach this arm.
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!(
                        "Transform/Tap are consumed by PipeSubjectWalker before the callback"
                    )
                }
            },
        );
    }

    fn validate_truthy_falsy_expr_tree(
        &mut self,
        root: ExprId,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, _| {
            if let ExprKind::Pipe(pipe) = &expr.kind {
                self.validate_truthy_falsy_pipe(pipe, env, typing);
            }
        });
    }

    fn validate_truthy_falsy_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        let all_stages = pipe.stages.iter().collect::<Vec<_>>();
        PipeSubjectWalker::new(pipe, env, typing).walk(
            typing,
            |stage_index, stage, current, typing| match &stage.kind {
                PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_gate_stage(*expr, env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_fanout_map_stage(*expr, env, s)),
                    advance_by: 1,
                },
                PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_fanin_stage(*expr, env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&all_stages, stage_index) else {
                        return PipeSubjectStepOutcome::Continue {
                            new_subject: None,
                            advance_by: 1,
                        };
                    };
                    let new_subject = current
                        .and_then(|s| self.validate_truthy_falsy_pair(&pair, env, s, typing));
                    let advance = pair.next_index.saturating_sub(stage_index).max(1);
                    PipeSubjectStepOutcome::Continue {
                        new_subject,
                        advance_by: advance,
                    }
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                    advance_by: 1,
                },
                // Transform and Tap are handled by PipeSubjectWalker before the
                // callback is invoked; they can never reach this arm.
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!(
                        "Transform/Tap are consumed by PipeSubjectWalker before the callback"
                    )
                }
            },
        );
    }

    fn validate_truthy_falsy_pair(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let Some(subject_plan) = typing.truthy_falsy_subject_plan(subject) else {
            self.emit_unsupported_truthy_falsy_subject(pair, subject);
            return None;
        };
        let truthy_has_payload = subject_plan.truthy_payload.is_some();
        let falsy_has_payload = subject_plan.falsy_payload.is_some();
        let truthy_info = typing.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
        );
        let falsy_info = typing.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
        );
        let truthy_ty = truthy_info.ty.clone();
        let falsy_ty = falsy_info.ty.clone();
        let mut saw_error = false;
        for issue in truthy_info.issues {
            self.emit_truthy_falsy_issue(
                crate::TruthyFalsyBranchKind::Truthy,
                truthy_has_payload,
                issue,
            );
            saw_error = true;
        }
        for issue in falsy_info.issues {
            self.emit_truthy_falsy_issue(
                crate::TruthyFalsyBranchKind::Falsy,
                falsy_has_payload,
                issue,
            );
            saw_error = true;
        }
        if saw_error {
            return None;
        }

        let Some(truthy_ty) = truthy_ty else {
            return None;
        };
        let Some(falsy_ty) = falsy_ty else {
            return None;
        };
        if !truthy_ty.same_shape(&falsy_ty) {
            self.emit_truthy_falsy_branch_type_mismatch(pair, &truthy_ty, &falsy_ty);
            return None;
        }

        Some(typing.apply_truthy_falsy_result_type(subject, truthy_ty))
    }

    fn validate_fanout_map_stage(
        &mut self,
        stage_span: SourceSpan,
        expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let Some(carrier) = typing.fanout_carrier(subject) else {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "fan-out `*|>` requires `List A` or `Signal (List A)`, found `{subject}`"
                ))
                .with_code(code("fanout-subject-not-list"))
                .with_label(DiagnosticLabel::primary(
                    stage_span,
                    "map over a list-valued subject or transform to `List` first",
                )),
            );
            return None;
        };
        let Some(element_subject) = subject.fanout_element().cloned() else {
            return None;
        };
        let body_info = typing.infer_pipe_body(expr, env, &element_subject);
        let mut saw_error = false;
        for issue in body_info.issues {
            self.emit_fanout_issue(FanoutIssueContext::MapElement, issue);
            saw_error = true;
        }
        if saw_error {
            return None;
        }
        let body_ty = body_info.ty?;
        Some(typing.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Map, carrier), body_ty))
    }

    fn validate_fanin_stage(
        &mut self,
        _stage_span: SourceSpan,
        expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let Some(carrier) = typing.fanout_carrier(subject) else {
            return None;
        };
        let body_info = typing.infer_pipe_body(expr, env, subject);
        let mut saw_error = false;
        for issue in body_info.issues {
            self.emit_fanout_issue(FanoutIssueContext::JoinCollection, issue);
            saw_error = true;
        }
        if saw_error {
            return None;
        }
        let body_ty = body_info.ty?;
        Some(typing.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Join, carrier), body_ty))
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
        // Gate predicates must evaluate to `Bool`. This is a hard semantic invariant: the `?|>`
        // stage passes each subject element through only when the predicate returns `true`. Any
        // other result type cannot meaningfully drive the keep/discard decision, so it must be
        // rejected here. The check is performed against the inferred type rather than delegated
        // to a downstream pass so that the error is anchored to the predicate expression itself.
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
        gate_env_for_function(item, typing)
    }

    fn emit_gate_issue(&mut self, issue: GateIssue) {
        match issue {
            GateIssue::InvalidPipeStageInput {
                span,
                expected,
                actual,
                ..
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate pipe stage expects `{actual}` but the current subject is `{expected}`"
                    ))
                    .with_code(code("invalid-pipe-stage-input"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make this staged predicate accept the current gate subject",
                    )),
                );
            }
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
            GateIssue::AmbiguousDomainMember {
                span,
                name,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate cannot resolve domain member `{name}` unambiguously"
                    ))
                    .with_code(code("ambiguous-domain-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add more type context or use a distinct local/import alias for the member you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
            GateIssue::UnsupportedApplicativeClusterMember { span, actual } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate contains an `&|>` cluster member with unsupported type `{actual}`"
                    ))
                    .with_code(code("unsupported-applicative-cluster-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use one resolved applicative family for every cluster member inside this predicate",
                    )),
                );
            }
            GateIssue::ApplicativeClusterMismatch {
                span,
                expected,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate mixes `{expected}` with `{actual}` inside one `&|>` cluster"
                    ))
                    .with_code(code("applicative-cluster-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "all members in one cluster must share the same outer applicative constructor",
                    )),
                );
            }
            GateIssue::InvalidClusterFinalizer {
                span,
                expected_inputs,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate uses an `&|>` cluster finalizer that cannot consume {} (found `{actual}`)",
                        expected_inputs
                            .iter()
                            .map(|input| format!("`{input}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .with_code(code("invalid-cluster-finalizer"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "adjust the finalizer so it accepts the member payload types in order",
                    )),
                );
            }
            GateIssue::CaseBranchTypeMismatch {
                span,
                expected,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "gate predicate contains a case split whose branches produce `{expected}` and `{actual}`"
                    ))
                    .with_code(code("case-branch-type-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make every branch in this case split produce the same type",
                    )),
                );
            }
            GateIssue::AmbiguousDomainOperator {
                span,
                operator,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "binary operator `{operator}` is ambiguous: multiple domain implementations match"
                    ))
                    .with_code(code("ambiguous-domain-operator"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add a type annotation on one operand to disambiguate which domain operator to use",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
        }
    }

    fn emit_unsupported_truthy_falsy_subject(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        subject: &GateType,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "`T|>` / `F|>` currently requires `Bool`, `Option A`, `Result E A`, `Validation E A`, or exactly one outer `Signal (...)` around those same canonical carriers, found `{subject}`"
            ))
            .with_code(code("truthy-falsy-subject-not-canonical"))
            .with_primary_label(
                pair.truthy_stage.span,
                "this branch pair cannot choose one of the RFC's canonical builtin truthy/falsy constructor pairs",
            )
            .with_secondary_label(pair.falsy_stage.span, "paired truthy/falsy stage involved here")
            .with_note(
                "current resolved-HIR truthy/falsy elaboration proves only the RFC's builtin canonical carriers plus one pointwise `Signal (...)` lift; user-defined truthy/falsy overloads remain later work",
            ),
        );
    }

    fn emit_truthy_falsy_issue(
        &mut self,
        branch: crate::TruthyFalsyBranchKind,
        has_payload: bool,
        issue: GateIssue,
    ) {
        let branch_name = match branch {
            crate::TruthyFalsyBranchKind::Truthy => "truthy",
            crate::TruthyFalsyBranchKind::Falsy => "falsy",
        };
        match issue {
            GateIssue::InvalidPipeStageInput {
                span,
                expected,
                actual,
                ..
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch pipe stage expects `{actual}` but the matched payload is `{expected}`"
                    ))
                    .with_code(code("invalid-pipe-stage-input"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make this staged branch expression accept the current payload",
                    )),
                );
            }
            GateIssue::InvalidProjection {
                span,
                path,
                subject,
            } => {
                let diagnostic = if !has_payload && subject == "unknown subject" {
                    Diagnostic::error(format!(
                        "{branch_name} branch cannot use ambient projection `{path}` because this branch matches a constructor with no payload"
                    ))
                    .with_code(code("invalid-truthy-falsy-projection"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "this branch has no matched payload subject; use a literal or named value here, or switch to `||>` for an explicit pattern",
                    ))
                } else {
                    Diagnostic::error(format!(
                        "{branch_name} branch cannot project `{path}` from matched payload subject `{subject}`"
                    ))
                    .with_code(code("invalid-truthy-falsy-projection"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "project from the matched payload or transform it before branching",
                    ))
                };
                self.diagnostics.push(diagnostic);
            }
            GateIssue::UnknownField {
                span,
                path,
                subject,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch cannot find field `{path}` on matched payload subject `{subject}`"
                    ))
                    .with_code(code("unknown-truthy-falsy-field"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use a field that exists on the matched branch payload",
                    )),
                );
            }
            GateIssue::AmbiguousDomainMember {
                span,
                name,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch cannot resolve domain member `{name}` unambiguously"
                    ))
                    .with_code(code("ambiguous-domain-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add more type context or use a distinct local/import alias for the member you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
            GateIssue::UnsupportedApplicativeClusterMember { span, actual } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch contains an `&|>` cluster member with unsupported type `{actual}`"
                    ))
                    .with_code(code("unsupported-applicative-cluster-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use one resolved applicative family for every cluster member in this branch",
                    )),
                );
            }
            GateIssue::ApplicativeClusterMismatch {
                span,
                expected,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch mixes `{expected}` with `{actual}` inside one `&|>` cluster"
                    ))
                    .with_code(code("applicative-cluster-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "all members in one cluster must share the same outer applicative constructor",
                    )),
                );
            }
            GateIssue::InvalidClusterFinalizer {
                span,
                expected_inputs,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch uses an `&|>` cluster finalizer that cannot consume {} (found `{actual}`)",
                        expected_inputs
                            .iter()
                            .map(|input| format!("`{input}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .with_code(code("invalid-cluster-finalizer"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "adjust the finalizer so it accepts the member payload types in order",
                    )),
                );
            }
            GateIssue::CaseBranchTypeMismatch {
                span,
                expected,
                actual,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch contains a case split whose branches produce `{expected}` and `{actual}`"
                    ))
                    .with_code(code("case-branch-type-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make every branch in this nested case split produce the same type",
                    )),
                );
            }
            GateIssue::AmbiguousDomainOperator {
                span,
                operator,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch: binary operator `{operator}` is ambiguous: multiple domain implementations match"
                    ))
                    .with_code(code("ambiguous-domain-operator"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add a type annotation on one operand to disambiguate which domain operator to use",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
        }
    }

    fn emit_truthy_falsy_branch_type_mismatch(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        truthy: &GateType,
        falsy: &GateType,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "`T|>` and `F|>` must elaborate to one shared branch result type, found `{truthy}` and `{falsy}`"
            ))
            .with_code(code("truthy-falsy-branch-type-mismatch"))
            .with_primary_label(
                pair.truthy_stage.span,
                format!("the `T|>` branch proves `{truthy}`"),
            )
            .with_secondary_label(
                pair.falsy_stage.span,
                format!("the `F|>` branch proves `{falsy}`"),
            )
            .with_note(
                "truthy/falsy shorthand is surface sugar over one deterministic two-arm case split, so both branches must agree on one result type",
            ),
        );
    }

    fn emit_fanout_issue(&mut self, context: FanoutIssueContext, issue: GateIssue) {
        match (context, issue) {
            (
                context,
                GateIssue::InvalidPipeStageInput {
                    span,
                    expected,
                    actual,
                    ..
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} pipe stage expects `{actual}` but the current subject is `{expected}`"
                    ))
                    .with_code(code("invalid-pipe-stage-input"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make this staged expression accept the current fan-out subject",
                    )),
                );
            }
            (
                FanoutIssueContext::MapElement,
                GateIssue::InvalidProjection {
                    span,
                    path,
                    subject,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-out body cannot project `{path}` from non-record element subject `{subject}`"
                    ))
                    .with_code(code("invalid-fanout-projection"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "project from each record element or transform to the desired field first",
                    )),
                );
            }
            (
                FanoutIssueContext::MapElement,
                GateIssue::UnknownField {
                    span,
                    path,
                    subject,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-out body cannot find field `{path}` on element subject `{subject}`"
                    ))
                    .with_code(code("unknown-fanout-field"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use a field that exists on each mapped element",
                    )),
                );
            }
            (
                FanoutIssueContext::JoinCollection,
                GateIssue::InvalidProjection {
                    span,
                    path,
                    subject,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-in body cannot project `{path}` from non-record collection subject `{subject}`"
                    ))
                    .with_code(code("invalid-fanin-projection"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "reduce the mapped collection directly or transform it before projecting",
                    )),
                );
            }
            (
                FanoutIssueContext::JoinCollection,
                GateIssue::UnknownField {
                    span,
                    path,
                    subject,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-in body cannot find field `{path}` on collection subject `{subject}`"
                    ))
                    .with_code(code("unknown-fanin-field"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use a field that exists on the mapped collection subject",
                    )),
                );
            }
            (
                FanoutIssueContext::MapElement,
                GateIssue::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-out body cannot resolve domain member `{name}` unambiguously"
                    ))
                    .with_code(code("ambiguous-domain-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add more type context or use a distinct local/import alias for the member you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
            (
                FanoutIssueContext::JoinCollection,
                GateIssue::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                },
            ) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "fan-in body cannot resolve domain member `{name}` unambiguously"
                    ))
                    .with_code(code("ambiguous-domain-member"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add more type context or use a distinct local/import alias for the member you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
            (context, GateIssue::UnsupportedApplicativeClusterMember { span, actual }) => {
                let (subject, code_name, label) = match context {
                    FanoutIssueContext::MapElement => (
                        "fan-out body",
                        "unsupported-applicative-cluster-member",
                        "use one resolved applicative family for every cluster member in this mapped body",
                    ),
                    FanoutIssueContext::JoinCollection => (
                        "fan-in body",
                        "unsupported-applicative-cluster-member",
                        "use one resolved applicative family for every cluster member in this reduction body",
                    ),
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} contains an `&|>` cluster member with unsupported type `{actual}`"
                    ))
                    .with_code(code(code_name))
                    .with_label(DiagnosticLabel::primary(span, label)),
                );
            }
            (
                context,
                GateIssue::ApplicativeClusterMismatch {
                    span,
                    expected,
                    actual,
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} mixes `{expected}` with `{actual}` inside one `&|>` cluster"
                    ))
                    .with_code(code("applicative-cluster-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "all members in one cluster must share the same outer applicative constructor",
                    )),
                );
            }
            (
                context,
                GateIssue::InvalidClusterFinalizer {
                    span,
                    expected_inputs,
                    actual,
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} uses an `&|>` cluster finalizer that cannot consume {} (found `{actual}`)",
                        expected_inputs
                            .iter()
                            .map(|input| format!("`{input}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .with_code(code("invalid-cluster-finalizer"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "adjust the finalizer so it accepts the member payload types in order",
                    )),
                );
            }
            (
                context,
                GateIssue::CaseBranchTypeMismatch {
                    span,
                    expected,
                    actual,
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} contains a case split whose branches produce `{expected}` and `{actual}`"
                    ))
                    .with_code(code("case-branch-type-mismatch"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "make every branch in this nested case split produce the same type",
                    )),
                );
            }
            (
                context,
                GateIssue::AmbiguousDomainOperator {
                    span,
                    operator,
                    candidates,
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject}: binary operator `{operator}` is ambiguous: multiple domain implementations match"
                    ))
                    .with_code(code("ambiguous-domain-operator"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add a type annotation on one operand to disambiguate which domain operator to use",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
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

    fn instance_class_item_id(&mut self, item: &crate::hir::InstanceItem) -> Option<ItemId> {
        match item.class.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                if matches!(self.module.items()[*item_id], Item::Class(_)) {
                    Some(*item_id)
                } else {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "instance class `{}` must resolve to a same-module `class` declaration",
                            item.class.path
                        ))
                        .with_code(code("invalid-instance-class"))
                        .with_primary_label(
                            item.class.span(),
                            "this instance head does not name a class declaration",
                        ),
                    );
                    None
                }
            }
            ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_)) => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "instance class `{}` must resolve to a same-module `class` declaration",
                        item.class.path
                    ))
                    .with_code(code("invalid-instance-class"))
                    .with_primary_label(
                        item.class.span(),
                        "instance heads cannot target imported, builtin, or type-parameter classes in this slice",
                    ),
                );
                None
            }
            ResolutionState::Unresolved => None,
        }
    }

    fn same_instance_argument_type(&self, left: TypeId, right: TypeId) -> bool {
        let mut work = vec![(left, right)];
        while let Some((left, right)) = work.pop() {
            let left = &self.module.types()[left].kind;
            let right = &self.module.types()[right].kind;
            match (left, right) {
                (TypeKind::Name(left), TypeKind::Name(right)) => {
                    if left.resolution != right.resolution
                        || left.path.segments().len() != right.path.segments().len()
                        || left
                            .path
                            .segments()
                            .iter()
                            .zip(right.path.segments().iter())
                            .any(|(left, right)| left.text() != right.text())
                    {
                        return false;
                    }
                }
                (TypeKind::Tuple(left), TypeKind::Tuple(right)) => {
                    if left.len() != right.len() {
                        return false;
                    }
                    work.extend(
                        left.iter()
                            .zip(right.iter())
                            .map(|(left, right)| (*left, *right)),
                    );
                }
                (TypeKind::Record(left), TypeKind::Record(right)) => {
                    if left.len() != right.len() {
                        return false;
                    }
                    for (left, right) in left.iter().zip(right.iter()) {
                        if left.label.text() != right.label.text() {
                            return false;
                        }
                        work.push((left.ty, right.ty));
                    }
                }
                (
                    TypeKind::Arrow {
                        parameter: left_parameter,
                        result: left_result,
                    },
                    TypeKind::Arrow {
                        parameter: right_parameter,
                        result: right_result,
                    },
                ) => {
                    work.push((*left_parameter, *right_parameter));
                    work.push((*left_result, *right_result));
                }
                (
                    TypeKind::Apply {
                        callee: left_callee,
                        arguments: left_arguments,
                    },
                    TypeKind::Apply {
                        callee: right_callee,
                        arguments: right_arguments,
                    },
                ) => {
                    if left_arguments.len() != right_arguments.len() {
                        return false;
                    }
                    work.push((*left_callee, *right_callee));
                    work.extend(
                        left_arguments
                            .iter()
                            .zip(right_arguments.iter())
                            .map(|(left, right)| (*left, *right)),
                    );
                }
                _ => return false,
            }
        }
        true
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
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                let constructor = store.add_constructor(
                    self.module.imports()[*import_id]
                        .local_name
                        .text()
                        .to_owned(),
                    self.import_type_kind(*import_id)?,
                );
                store.constructor_expr(constructor)
            }
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

    fn import_type_kind(&self, import_id: ImportId) -> Option<Kind> {
        let import = &self.module.imports()[import_id];
        match &import.metadata {
            ImportBindingMetadata::TypeConstructor { kind } => Some(kind.clone()),
            ImportBindingMetadata::BuiltinType(builtin) => Some(builtin_kind(*builtin)),
            ImportBindingMetadata::Value { .. }
            | ImportBindingMetadata::IntrinsicValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_)
            | ImportBindingMetadata::Unknown => None,
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
                TermResolution::DomainMember(resolution) => {
                    this.require_domain_member_resolution(reference.span(), *resolution);
                }
                TermResolution::AmbiguousDomainMembers(candidates) => {
                    for resolution in candidates.iter().copied() {
                        this.require_domain_member_resolution(reference.span(), resolution);
                    }
                }
                TermResolution::ClassMember(resolution) => {
                    this.require_item(
                        reference.span(),
                        "term reference",
                        "class",
                        resolution.class,
                    );
                }
                TermResolution::AmbiguousClassMembers(candidates) => {
                    for resolution in candidates.iter().copied() {
                        this.require_item(
                            reference.span(),
                            "term reference",
                            "class",
                            resolution.class,
                        );
                    }
                }
                TermResolution::Builtin(_) => {}
                TermResolution::IntrinsicValue(_) => {}
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

    fn check_regex_literal(&mut self, literal_span: SourceSpan, regex: &crate::hir::RegexLiteral) {
        let Some(pattern) = regex_literal_body(&regex.raw) else {
            self.diagnostics.push(
                Diagnostic::error("regex literal lost its `rx\"...\"` wrapper before validation")
                    .with_code(code("malformed-regex-literal"))
                    .with_primary_label(
                        literal_span,
                        "preserve the original surface literal while lowering into HIR",
                    ),
            );
            return;
        };

        let mut builder = RegexParserBuilder::new();
        builder.nest_limit(REGEX_NEST_LIMIT);
        let mut parser = builder.build();
        if let Err(error) = parser.parse(pattern) {
            self.diagnostics.push(invalid_regex_literal_diagnostic(
                literal_span,
                &regex.raw,
                &error,
            ));
        }
    }

    fn check_source_metadata(&mut self, span: SourceSpan, metadata: &SourceMetadata) {
        self.check_source_dependency_list(span, "source metadata", &metadata.signal_dependencies);
        self.check_source_dependency_list(
            span,
            "source lifecycle reconfiguration",
            &metadata.lifecycle_dependencies.reconfiguration,
        );
        self.check_source_dependency_list(
            span,
            "source lifecycle trigger",
            &metadata.lifecycle_dependencies.explicit_triggers,
        );
        self.check_source_dependency_list(
            span,
            "source lifecycle activeWhen",
            &metadata.lifecycle_dependencies.active_when,
        );
        if metadata.lifecycle_dependencies.merged() != metadata.signal_dependencies {
            self.diagnostics.push(
                Diagnostic::error(
                    "source lifecycle dependency roles must stay consistent with source metadata dependencies",
                )
                .with_code(code("inconsistent-source-lifecycle-dependencies"))
                .with_label(DiagnosticLabel::primary(
                    span,
                    "recompute source lifecycle dependency roles after name resolution",
                )),
            );
        }
        if metadata.custom_contract.is_some() {
            match &metadata.provider {
                SourceProviderRef::Custom(_) => {}
                SourceProviderRef::Builtin(_) => self.diagnostics.push(
                    Diagnostic::error(
                        "built-in source metadata must not carry custom provider contract hooks",
                    )
                    .with_code(code("invalid-custom-source-wakeup"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "remove the custom contract hook and rely on the built-in source planner instead",
                    )),
                ),
                SourceProviderRef::Missing => self.diagnostics.push(
                    Diagnostic::error(
                        "custom source contract metadata requires a preserved source provider key",
                    )
                    .with_code(code("invalid-custom-source-wakeup"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "store the source provider key before attaching custom contract metadata",
                    )),
                ),
                SourceProviderRef::InvalidShape(_) => self.diagnostics.push(
                    Diagnostic::error(
                        "custom source contract metadata requires a provider variant such as `custom.feed`",
                    )
                    .with_code(code("invalid-custom-source-wakeup"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "fix the provider path before attaching custom contract metadata",
                    )),
                ),
            }
        }
    }

    fn check_source_dependency_list(
        &mut self,
        span: SourceSpan,
        role: &str,
        dependencies: &[ItemId],
    ) {
        let mut previous = None;
        for dependency in dependencies {
            self.require_item(span, "source lifecycle", "signal dependency", *dependency);
            if let Some(item) = self.module.items().get(*dependency) {
                if !matches!(item, Item::Signal(_)) {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "{role} dependency must point at a signal item"
                        ))
                        .with_code(code("invalid-source-lifecycle-dependency"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "update the source lifecycle dependency list to reference only signal items",
                        )),
                    );
                }
            }
            if let Some(previous) = previous {
                if previous >= *dependency {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "{role} dependency lists must stay sorted and duplicate-free"
                        ))
                        .with_code(code("unordered-source-lifecycle-dependencies"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "normalize source lifecycle dependency ordering after resolution",
                        )),
                    );
                    break;
                }
            }
            previous = Some(*dependency);
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

    fn require_domain_member_resolution(
        &mut self,
        span: SourceSpan,
        resolution: DomainMemberResolution,
    ) {
        let Some(item) = self.module.items().get(resolution.domain) else {
            self.diagnostics.push(
                Diagnostic::error("domain member resolution points at a missing domain item")
                    .with_code(code("invalid-domain-member-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "update this resolution to target an existing domain item",
                    )),
            );
            return;
        };
        let Item::Domain(domain) = item else {
            self.diagnostics.push(
                Diagnostic::error("domain member resolution does not target a domain item")
                    .with_code(code("invalid-domain-member-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "domain member resolutions must point at a domain declaration",
                    )),
            );
            return;
        };
        let Some(member) = domain.members.get(resolution.member_index) else {
            self.diagnostics.push(
                Diagnostic::error("domain member resolution points at a missing domain member")
                    .with_code(code("invalid-domain-member-resolution"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "update this resolution to target an existing domain member",
                    )),
            );
            return;
        };
        if member.kind != DomainMemberKind::Method {
            self.diagnostics.push(
                Diagnostic::error(
                    "domain member resolution does not target a callable domain member",
                )
                .with_code(code("invalid-domain-member-resolution"))
                .with_label(DiagnosticLabel::primary(
                    span,
                    "only callable identifier members participate in unqualified term resolution",
                )),
            );
        }
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

fn regex_literal_body(raw: &str) -> Option<&str> {
    raw.strip_prefix("rx\"")
        .and_then(|pattern| pattern.strip_suffix('\"'))
}

fn invalid_regex_literal_diagnostic(
    literal_span: SourceSpan,
    raw: &str,
    error: &RegexSyntaxError,
) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "regex literal is not valid under the current compile-time regex grammar",
    )
    .with_code(code("invalid-regex-literal"));
    match error {
        RegexSyntaxError::Parse(error) => {
            let mut diagnostic = diagnostic.with_primary_label(
                regex_span_in_literal(literal_span, raw, error.span()),
                error.kind().to_string(),
            );
            if let Some(auxiliary) = error.auxiliary_span() {
                diagnostic = diagnostic.with_secondary_label(
                    regex_span_in_literal(literal_span, raw, auxiliary),
                    "the original conflicting regex fragment is here",
                );
            }
            diagnostic
        }
        RegexSyntaxError::Translate(error) => diagnostic.with_primary_label(
            regex_span_in_literal(literal_span, raw, error.span()),
            error.kind().to_string(),
        ),
        _ => diagnostic.with_primary_label(
            literal_span,
            "this regex literal failed compile-time validation",
        ),
    }
}

fn regex_span_in_literal(
    literal_span: SourceSpan,
    raw: &str,
    regex_span: &RegexSpan,
) -> SourceSpan {
    let body_len = regex_literal_body(raw).map_or(0, str::len);
    let start_offset = regex_span.start.offset.min(body_len);
    let end_offset = regex_span.end.offset.max(start_offset).min(body_len);
    let literal_start = literal_span.span().start().as_usize();
    let body_start = literal_start + REGEX_LITERAL_PREFIX_LEN;
    let start = body_start + start_offset;
    let end = body_start + end_offset;
    let start = u32::try_from(start).expect("regex literal start offset should fit in ByteIndex");
    let end = u32::try_from(end).expect("regex literal end offset should fit in ByteIndex");
    SourceSpan::new(
        literal_span.file(),
        Span::new(ByteIndex::new(start), ByteIndex::new(end)),
    )
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
        BuiltinType::List | BuiltinType::Set | BuiltinType::Option | BuiltinType::Signal => {
            Kind::constructor(1)
        }
        BuiltinType::Map | BuiltinType::Result | BuiltinType::Validation | BuiltinType::Task => {
            Kind::constructor(2)
        }
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
        BuiltinType::Map => "Map",
        BuiltinType::Set => "Set",
        BuiltinType::Option => "Option",
        BuiltinType::Result => "Result",
        BuiltinType::Validation => "Validation",
        BuiltinType::Signal => "Signal",
        BuiltinType::Task => "Task",
    }
}

fn builtin_type_arity(builtin: BuiltinType) -> usize {
    match builtin {
        BuiltinType::Int
        | BuiltinType::Float
        | BuiltinType::Decimal
        | BuiltinType::BigInt
        | BuiltinType::Bool
        | BuiltinType::Text
        | BuiltinType::Unit
        | BuiltinType::Bytes => 0,
        BuiltinType::List | BuiltinType::Set | BuiltinType::Option | BuiltinType::Signal => 1,
        BuiltinType::Map | BuiltinType::Result | BuiltinType::Validation | BuiltinType::Task => 2,
    }
}

fn type_constructor_arity(head: TypeConstructorHead, module: &Module) -> usize {
    match head {
        TypeConstructorHead::Builtin(builtin) => builtin_type_arity(builtin),
        TypeConstructorHead::Item(item_id) => match &module.items()[item_id] {
            Item::Type(item) => item.parameters.len(),
            Item::Domain(item) => item.parameters.len(),
            _ => 0,
        },
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

fn item_type_name(item: &Item) -> String {
    match item {
        Item::Type(item) => item.name.text().to_owned(),
        Item::Class(item) => item.name.text().to_owned(),
        Item::Domain(item) => item.name.text().to_owned(),
        Item::SourceProviderContract(item) => {
            item.provider.key().unwrap_or("<provider>").to_owned()
        }
        other => format!("{:?}", other.kind()),
    }
}

#[derive(Clone, Copy, Debug)]
enum ExprWalkWork {
    Expr { expr: ExprId, is_root: bool },
    Markup(MarkupNodeId),
    Control(ControlNodeId),
}

#[derive(Clone, Debug)]
enum CaseExhaustivenessWork {
    Expr {
        expr: ExprId,
        env: GateExprEnv,
    },
    Markup {
        node: MarkupNodeId,
        env: GateExprEnv,
    },
    Control {
        node: ControlNodeId,
        env: GateExprEnv,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprEnv {
    pub(crate) locals: HashMap<BindingId, GateType>,
}

/// Outcome of one step in a `PipeSubjectWalker` iteration.
///
/// Returned by per-stage callbacks to tell the walker how to advance and what
/// the new subject type is after the stage (PA-M1).
pub(crate) enum PipeSubjectStepOutcome {
    /// The stage was handled; `new_subject` is the subject type after the
    /// stage and `advance_by` is how many stage slots to skip (usually 1, but
    /// fanout segments span multiple slots).
    Continue {
        new_subject: Option<GateType>,
        advance_by: usize,
    },
    /// Stop walking at this stage index (e.g. when hitting a recurrence
    /// boundary or a stage kind the caller cannot handle).
    Stop,
}

/// Iterator-style helper that walks a pipe expression's stages left-to-right,
/// maintaining the subject type across `|>` transform and `|` tap stages.
///
/// Callers supply a callback that handles operator-specific stages (gate,
/// fanout, truthy/falsy, recurrence, …).  The walker handles the common
/// `Transform` and `Tap` stages so every pass doesn't duplicate that logic
/// (PA-M1).
///
/// # Usage
/// ```ignore
/// PipeSubjectWalker::new(pipe, env, typing).walk(|stage_index, stage, current, typing| {
///     match &stage.kind {
///         PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue { … },
///         _ => PipeSubjectStepOutcome::Stop,
///     }
/// });
/// ```
pub(crate) struct PipeSubjectWalker<'pipe, 'env> {
    stages: Vec<&'pipe PipeStage>,
    current: Option<GateType>,
    env: &'env GateExprEnv,
}

impl<'pipe, 'env> PipeSubjectWalker<'pipe, 'env> {
    pub(crate) fn new(
        pipe: &'pipe crate::hir::PipeExpr,
        env: &'env GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) -> Self {
        let stages = pipe.stages.iter().collect::<Vec<_>>();
        let current = typing.infer_expr(pipe.head, env, None).ty;
        Self {
            stages,
            current,
            env,
        }
    }

    /// Like `new`, but only considers the first `limit` stages of the pipe.
    ///
    /// Used by passes (e.g. recurrence elaboration) that must stop before the
    /// recurrence boundary stages (`RecurStart`/`RecurStep`) which appear at a
    /// known prefix position (PA-M1).
    pub(crate) fn new_with_limit(
        pipe: &'pipe crate::hir::PipeExpr,
        env: &'env GateExprEnv,
        typing: &mut GateTypeContext<'_>,
        limit: usize,
    ) -> Self {
        let stages = pipe.stages.iter().take(limit).collect::<Vec<_>>();
        let current = typing.infer_expr(pipe.head, env, None).ty;
        Self {
            stages,
            current,
            env,
        }
    }

    /// Walk all stages, calling `on_stage` for each stage that is not a plain
    /// `Transform` or `Tap`.  Iteration stops when `on_stage` returns
    /// `PipeSubjectStepOutcome::Stop` or when all stages are exhausted.
    ///
    /// Returns the subject type at the point where walking stopped.
    pub(crate) fn walk<F>(
        mut self,
        typing: &mut GateTypeContext<'_>,
        mut on_stage: F,
    ) -> Option<GateType>
    where
        F: FnMut(
            usize,             // stage_index
            &'pipe PipeStage,  // stage
            Option<&GateType>, // current subject (before this stage)
            &mut GateTypeContext<'_>,
        ) -> PipeSubjectStepOutcome,
    {
        let mut stage_index = 0usize;
        while stage_index < self.stages.len() {
            let stage = self.stages[stage_index];
            match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    self.current = self
                        .current
                        .as_ref()
                        .and_then(|subject| typing.infer_transform_stage(*expr, self.env, subject));
                    stage_index += 1;
                }
                PipeStageKind::Tap { expr } => {
                    if let Some(subject) = self.current.clone() {
                        let _ = typing.infer_pipe_body(*expr, self.env, &subject);
                        self.current = Some(subject);
                    }
                    stage_index += 1;
                }
                _ => match on_stage(stage_index, stage, self.current.as_ref(), typing) {
                    PipeSubjectStepOutcome::Continue {
                        new_subject,
                        advance_by,
                    } => {
                        self.current = new_subject;
                        stage_index += advance_by;
                    }
                    PipeSubjectStepOutcome::Stop => break,
                },
            }
        }
        self.current
    }
}

/// Build a `GateExprEnv` from a function item's annotated parameters.
///
/// Shared by all gate/truthy-falsy/recurrence elaboration passes so the
/// parameter-to-type wiring is defined exactly once (PA-I2).
pub(crate) fn gate_env_for_function(
    item: &crate::hir::FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> GateExprEnv {
    let mut env = GateExprEnv::default();
    for parameter in &item.parameters {
        let Some(annotation) = parameter.annotation else {
            continue;
        };
        if let Some(ty) = typing.lower_open_annotation(annotation) {
            env.locals.insert(parameter.binding, ty);
        }
    }
    env
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprInfo {
    pub(crate) ty: Option<GateType>,
    actual: Option<SourceOptionActualType>,
    pub(crate) contains_signal: bool,
    pub(crate) issues: Vec<GateIssue>,
    pub(crate) constraints: Vec<TypeConstraint>,
}

impl GateExprInfo {
    fn merge(&mut self, other: Self) {
        self.contains_signal |= other.contains_signal;
        self.issues.extend(other.issues);
        self.constraints.extend(other.constraints);
    }

    fn actual(&self) -> Option<SourceOptionActualType> {
        self.actual
            .clone()
            .or_else(|| self.ty.as_ref().map(SourceOptionActualType::from_gate_type))
    }

    pub(crate) fn actual_gate_type(&self) -> Option<GateType> {
        self.actual().and_then(|actual| actual.to_gate_type())
    }

    fn set_actual(&mut self, actual: SourceOptionActualType) {
        self.contains_signal |= actual.is_signal();
        self.ty = actual.to_gate_type();
        self.actual = Some(actual);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PipeFunctionSignatureMatch {
    pub(crate) callee_expr: ExprId,
    pub(crate) explicit_arguments: Vec<ExprId>,
    pub(crate) signal_payload_arguments: Vec<bool>,
    pub(crate) parameter_types: Vec<GateType>,
    pub(crate) result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateIssue {
    InvalidPipeStageInput {
        span: SourceSpan,
        stage: &'static str,
        expected: String,
        actual: String,
    },
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
    AmbiguousDomainMember {
        span: SourceSpan,
        name: String,
        candidates: Vec<String>,
    },
    UnsupportedApplicativeClusterMember {
        span: SourceSpan,
        actual: String,
    },
    ApplicativeClusterMismatch {
        span: SourceSpan,
        expected: String,
        actual: String,
    },
    InvalidClusterFinalizer {
        span: SourceSpan,
        expected_inputs: Vec<String>,
        actual: String,
    },
    CaseBranchTypeMismatch {
        span: SourceSpan,
        expected: String,
        actual: String,
    },
    /// Two or more domain operator implementations match the given binary expression.
    /// The caller must emit this issue as a diagnostic and treat the operator result type
    /// as unknown so downstream checking can continue without cascading false errors.
    AmbiguousDomainOperator {
        span: SourceSpan,
        operator: String,
        candidates: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DomainMemberSelection<T> {
    Unique(T),
    Ambiguous,
    NoMatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DomainMemberCallMatch {
    pub(crate) parameters: Vec<GateType>,
    pub(crate) result: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassConstraintBinding {
    pub(crate) class_item: ItemId,
    pub(crate) subject: TypeBinding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassMemberCallMatch {
    pub(crate) resolution: ClassMemberResolution,
    pub(crate) parameters: Vec<GateType>,
    pub(crate) result: GateType,
    pub(crate) evidence: ClassConstraintBinding,
    pub(crate) constraints: Vec<ClassConstraintBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TruthyFalsySubjectPlan {
    pub(crate) truthy_constructor: BuiltinTerm,
    pub(crate) truthy_payload: Option<GateType>,
    pub(crate) falsy_constructor: BuiltinTerm,
    pub(crate) falsy_payload: Option<GateType>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TruthyFalsyPairStages<'a> {
    pub(crate) truthy_index: usize,
    pub(crate) truthy_stage: &'a crate::hir::PipeStage,
    pub(crate) truthy_expr: ExprId,
    pub(crate) falsy_index: usize,
    pub(crate) falsy_stage: &'a crate::hir::PipeStage,
    pub(crate) falsy_expr: ExprId,
    pub(crate) next_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FanoutIssueContext {
    MapElement,
    JoinCollection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecurrenceTargetHint {
    Evidence(RecurrenceTargetEvidence),
    UnsupportedType { ty: GateType, span: SourceSpan },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecurrenceWakeupHint {
    BuiltinSource(SourceRecurrenceWakeupContext),
    NonSource(NonSourceWakeupCause),
    CustomSource {
        provider_path: NamePath,
        context: CustomSourceRecurrenceWakeupContext,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRecordField {
    pub name: String,
    pub ty: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CaseConstructorKey {
    Builtin(BuiltinTerm),
    SameModuleVariant { item: ItemId, name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaseConstructorShape {
    key: CaseConstructorKey,
    display: String,
    span: Option<SourceSpan>,
    field_types: Option<Vec<GateType>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaseSubjectShape {
    constructors: Vec<CaseConstructorShape>,
}

impl CaseSubjectShape {
    fn constructor(&self, key: &CaseConstructorKey) -> Option<&CaseConstructorShape> {
        self.constructors
            .iter()
            .find(|constructor| &constructor.key == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CasePatternCoverage {
    CatchAll,
    Constructor(CaseConstructorKey),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseSiteKind {
    PipeCase,
    MatchControl,
}

impl CaseSiteKind {
    const fn display_name(self) -> &'static str {
        match self {
            Self::PipeCase => "case split",
            Self::MatchControl => "match control",
        }
    }
}

pub fn case_pattern_field_types(
    module: &Module,
    callee: &TermReference,
    subject: &GateType,
) -> Option<Vec<GateType>> {
    fn same_module_constructor_fields(
        module: &Module,
        item_id: ItemId,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let Item::Type(item) = &module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let GateType::OpaqueItem {
            item: subject_item,
            arguments,
            ..
        } = subject
        else {
            return None;
        };
        if *subject_item != item_id || item.parameters.len() != arguments.len() {
            return None;
        }
        let variant_name = callee.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut typing = GateTypeContext::new(module);
        variant
            .fields
            .iter()
            .map(|field| typing.lower_hir_type(*field, &substitutions))
            .collect()
    }

    match callee.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::True))
        | ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::False)) => {
            matches!(subject, GateType::Primitive(BuiltinType::Bool)).then(Vec::new)
        }
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Some)) => match subject {
            GateType::Option(payload) => Some(vec![payload.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::None)) => {
            matches!(subject, GateType::Option(_)).then(Vec::new)
        }
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Ok)) => match subject {
            GateType::Result { value, .. } => Some(vec![value.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Err)) => match subject {
            GateType::Result { error, .. } => Some(vec![error.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Valid)) => match subject {
            GateType::Validation { value, .. } => Some(vec![value.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Invalid)) => match subject {
            GateType::Validation { error, .. } => Some(vec![error.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            same_module_constructor_fields(module, *item_id, callee, subject)
        }
        ResolutionState::Resolved(_) | ResolutionState::Unresolved => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateType {
    Primitive(BuiltinType),
    TypeParameter {
        parameter: TypeParameterId,
        name: String,
    },
    Tuple(Vec<GateType>),
    Record(Vec<GateRecordField>),
    Arrow {
        parameter: Box<GateType>,
        result: Box<GateType>,
    },
    List(Box<GateType>),
    Map {
        key: Box<GateType>,
        value: Box<GateType>,
    },
    Set(Box<GateType>),
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

    pub(crate) fn fanout_carrier(&self) -> Option<FanoutCarrier> {
        match self {
            Self::List(_) => Some(FanoutCarrier::Ordinary),
            Self::Signal(inner) if matches!(inner.as_ref(), Self::List(_)) => {
                Some(FanoutCarrier::Signal)
            }
            _ => None,
        }
    }

    pub(crate) fn fanout_element(&self) -> Option<&Self> {
        match self {
            Self::List(element) => Some(element),
            Self::Signal(inner) => match inner.as_ref() {
                Self::List(element) => Some(element),
                _ => None,
            },
            _ => None,
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
        let mut left_to_right = HashMap::new();
        let mut right_to_left = HashMap::new();
        Self::same_shape_inner(self, other, &mut left_to_right, &mut right_to_left)
    }

    fn same_shape_inner(
        left: &Self,
        right: &Self,
        left_to_right: &mut HashMap<TypeParameterId, TypeParameterId>,
        right_to_left: &mut HashMap<TypeParameterId, TypeParameterId>,
    ) -> bool {
        match (left, right) {
            (Self::Primitive(left), Self::Primitive(right)) => left == right,
            (
                Self::TypeParameter {
                    parameter: left_parameter,
                    ..
                },
                Self::TypeParameter {
                    parameter: right_parameter,
                    ..
                },
            ) => match (
                left_to_right.get(left_parameter),
                right_to_left.get(right_parameter),
            ) {
                (Some(mapped_right), Some(mapped_left)) => {
                    mapped_right == right_parameter && mapped_left == left_parameter
                }
                (None, None) => {
                    left_to_right.insert(*left_parameter, *right_parameter);
                    right_to_left.insert(*right_parameter, *left_parameter);
                    true
                }
                _ => false,
            },
            (Self::Tuple(left), Self::Tuple(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        Self::same_shape_inner(left, right, left_to_right, right_to_left)
                    })
            }
            (Self::Record(left), Self::Record(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        left.name == right.name
                            && Self::same_shape_inner(
                                &left.ty,
                                &right.ty,
                                left_to_right,
                                right_to_left,
                            )
                    })
            }
            (
                Self::Arrow {
                    parameter: left_parameter,
                    result: left_result,
                },
                Self::Arrow {
                    parameter: right_parameter,
                    result: right_result,
                },
            ) => {
                Self::same_shape_inner(
                    left_parameter,
                    right_parameter,
                    left_to_right,
                    right_to_left,
                ) && Self::same_shape_inner(left_result, right_result, left_to_right, right_to_left)
            }
            (Self::List(left), Self::List(right))
            | (Self::Set(left), Self::Set(right))
            | (Self::Option(left), Self::Option(right))
            | (Self::Signal(left), Self::Signal(right)) => {
                Self::same_shape_inner(left, right, left_to_right, right_to_left)
            }
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_key, right_key, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
            }
            (
                Self::Result {
                    error: left_error,
                    value: left_value,
                },
                Self::Result {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_error, right_error, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
            }
            (
                Self::Domain {
                    item: left_item,
                    arguments: left_arguments,
                    ..
                },
                Self::Domain {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            )
            | (
                Self::OpaqueItem {
                    item: left_item,
                    arguments: left_arguments,
                    ..
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) => {
                left_item == right_item
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            _ => false,
        }
    }

    pub(crate) fn constructor_view(&self) -> Option<(TypeConstructorHead, Vec<GateType>)> {
        match self {
            Self::List(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::List),
                vec![element.as_ref().clone()],
            )),
            Self::Map { key, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Map),
                vec![key.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Set(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Set),
                vec![element.as_ref().clone()],
            )),
            Self::Option(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Option),
                vec![element.as_ref().clone()],
            )),
            Self::Result { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Result),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Validation { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Validation),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Signal(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Signal),
                vec![element.as_ref().clone()],
            )),
            Self::Task { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Task),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Domain {
                item, arguments, ..
            }
            | Self::OpaqueItem {
                item, arguments, ..
            } => Some((TypeConstructorHead::Item(*item), arguments.clone())),
            Self::Primitive(_)
            | Self::TypeParameter { .. }
            | Self::Tuple(_)
            | Self::Record(_)
            | Self::Arrow { .. } => None,
        }
    }
}

impl fmt::Display for GateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateType::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            GateType::TypeParameter { name, .. } => write!(f, "{name}"),
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
            GateType::Map { key, value } => write!(f, "Map {key} {value}"),
            GateType::Set(element) => write!(f, "Set {element}"),
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum ApplicativeClusterKind {
    List,
    Option,
    Result { error: SourceOptionActualType },
    Validation { error: SourceOptionActualType },
    Signal,
    Task { error: SourceOptionActualType },
}

impl ApplicativeClusterKind {
    fn from_member_actual(
        actual: &SourceOptionActualType,
    ) -> Option<(Self, SourceOptionActualType)> {
        match actual {
            SourceOptionActualType::List(element) => Some((Self::List, element.as_ref().clone())),
            SourceOptionActualType::Option(element) => {
                Some((Self::Option, element.as_ref().clone()))
            }
            SourceOptionActualType::Result { error, value } => Some((
                Self::Result {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Validation { error, value } => Some((
                Self::Validation {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Signal(element) => {
                Some((Self::Signal, element.as_ref().clone()))
            }
            SourceOptionActualType::Task { error, value } => Some((
                Self::Task {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Hole
            | SourceOptionActualType::Primitive(_)
            | SourceOptionActualType::Tuple(_)
            | SourceOptionActualType::Record(_)
            | SourceOptionActualType::Arrow { .. }
            | SourceOptionActualType::Map { .. }
            | SourceOptionActualType::Set(_)
            | SourceOptionActualType::Domain { .. }
            | SourceOptionActualType::OpaqueItem { .. } => None,
        }
    }

    fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::List, Self::List) => Some(Self::List),
            (Self::Option, Self::Option) => Some(Self::Option),
            (Self::Signal, Self::Signal) => Some(Self::Signal),
            (Self::Result { error: left }, Self::Result { error: right }) => Some(Self::Result {
                error: left.unify(right)?,
            }),
            (Self::Validation { error: left }, Self::Validation { error: right }) => {
                Some(Self::Validation {
                    error: left.unify(right)?,
                })
            }
            (Self::Task { error: left }, Self::Task { error: right }) => Some(Self::Task {
                error: left.unify(right)?,
            }),
            _ => None,
        }
    }

    fn wrap_actual(&self, payload: SourceOptionActualType) -> SourceOptionActualType {
        match self {
            Self::List => SourceOptionActualType::List(Box::new(payload)),
            Self::Option => SourceOptionActualType::Option(Box::new(payload)),
            Self::Result { error } => SourceOptionActualType::Result {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Validation { error } => SourceOptionActualType::Validation {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Signal => SourceOptionActualType::Signal(Box::new(payload)),
            Self::Task { error } => SourceOptionActualType::Task {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
        }
    }

    fn surface(&self) -> String {
        match self {
            Self::List => "List _".to_owned(),
            Self::Option => "Option _".to_owned(),
            Self::Result { error } => format!("Result {error} _"),
            Self::Validation { error } => format!("Validation {error} _"),
            Self::Signal => "Signal _".to_owned(),
            Self::Task { error } => format!("Task {error} _"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeBinding {
    Type(GateType),
    Constructor(TypeConstructorBinding),
}

impl TypeBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Type(left), Self::Type(right)) => left.same_shape(right),
            (Self::Constructor(left), Self::Constructor(right)) => left.matches(right),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeConstructorBinding {
    head: TypeConstructorHead,
    arguments: Vec<GateType>,
}

impl TypeConstructorBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.head == other.head
            && self.arguments.len() == other.arguments.len()
            && self
                .arguments
                .iter()
                .zip(other.arguments.iter())
                .all(|(left, right)| left.same_shape(right))
    }

    pub fn head(&self) -> TypeConstructorHead {
        self.head
    }

    pub fn arguments(&self) -> &[GateType] {
        &self.arguments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeConstructorHead {
    Builtin(BuiltinType),
    Item(ItemId),
}

pub(crate) type PolyTypeBindings = HashMap<TypeParameterId, TypeBinding>;

pub(crate) struct GateTypeContext<'a> {
    module: &'a Module,
    item_types: HashMap<ItemId, Option<GateType>>,
    item_actuals: HashMap<ItemId, Option<SourceOptionActualType>>,
}

impl<'a> GateTypeContext<'a> {
    pub(crate) fn new(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
            item_actuals: HashMap::new(),
        }
    }

    pub(crate) fn fanout_carrier(&self, subject: &GateType) -> Option<FanoutCarrier> {
        subject.fanout_carrier()
    }

    pub(crate) fn gate_carrier(&self, subject: &GateType) -> GateCarrier {
        subject.gate_carrier()
    }

    pub(crate) fn truthy_falsy_subject_plan(
        &self,
        subject: &GateType,
    ) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Signal(inner) => Self::truthy_falsy_ordinary_subject_plan(inner),
            other => Self::truthy_falsy_ordinary_subject_plan(other),
        }
    }

    fn truthy_falsy_ordinary_subject_plan(subject: &GateType) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::True,
                truthy_payload: None,
                falsy_constructor: BuiltinTerm::False,
                falsy_payload: None,
            }),
            GateType::Option(payload) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Some,
                truthy_payload: Some(payload.as_ref().clone()),
                falsy_constructor: BuiltinTerm::None,
                falsy_payload: None,
            }),
            GateType::Result { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Ok,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Err,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Validation { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Valid,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Invalid,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. }
            | GateType::OpaqueItem { .. } => None,
        }
    }

    pub(crate) fn apply_truthy_falsy_result_type(
        &self,
        subject: &GateType,
        result: GateType,
    ) -> GateType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => GateType::Signal(Box::new(result)),
        }
    }

    fn apply_truthy_falsy_result_actual(
        &self,
        subject: &GateType,
        result: SourceOptionActualType,
    ) -> SourceOptionActualType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(result)),
        }
    }

    fn case_subject_shape(&mut self, subject: &GateType) -> Option<CaseSubjectShape> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::True),
                        display: "True".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::False),
                        display: "False".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Option(payload) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Some),
                        display: "Some".to_owned(),
                        span: None,
                        field_types: Some(vec![payload.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::None),
                        display: "None".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Result { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Ok),
                        display: "Ok".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Err),
                        display: "Err".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::Validation { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Valid),
                        display: "Valid".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Invalid),
                        display: "Invalid".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::OpaqueItem {
                item, arguments, ..
            } => self.same_module_case_subject_shape(*item, arguments),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. } => None,
        }
    }

    fn same_module_case_subject_shape(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
    ) -> Option<CaseSubjectShape> {
        let Item::Type(item) = &self.module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        if item.parameters.len() != arguments.len() {
            return None;
        }
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let constructors = variants
            .iter()
            .map(|variant| CaseConstructorShape {
                key: CaseConstructorKey::SameModuleVariant {
                    item: item_id,
                    name: variant.name.text().to_owned(),
                },
                display: variant.name.text().to_owned(),
                span: Some(variant.span),
                field_types: self.lower_case_variant_fields(&variant.fields, &substitutions),
            })
            .collect::<Vec<_>>();
        Some(CaseSubjectShape { constructors })
    }

    fn lower_case_variant_fields(
        &mut self,
        fields: &[TypeId],
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<Vec<GateType>> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            lowered.push(self.lower_type(*field, substitutions, &mut Vec::new(), false)?);
        }
        Some(lowered)
    }

    fn case_pattern_coverage(
        &mut self,
        pattern_id: PatternId,
        subject: &CaseSubjectShape,
    ) -> CasePatternCoverage {
        let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
            return CasePatternCoverage::None;
        };
        match pattern.kind {
            PatternKind::Wildcard | PatternKind::Binding(_) => CasePatternCoverage::CatchAll,
            PatternKind::Constructor { callee, .. } | PatternKind::UnresolvedName(callee) => {
                let Some(key) = case_constructor_key(&callee) else {
                    return CasePatternCoverage::None;
                };
                if subject.constructor(&key).is_some() {
                    CasePatternCoverage::Constructor(key)
                } else {
                    CasePatternCoverage::None
                }
            }
            PatternKind::Integer(_)
            | PatternKind::Text(_)
            | PatternKind::Tuple(_)
            | PatternKind::List { .. }
            | PatternKind::Record(_) => CasePatternCoverage::None,
        }
    }

    fn case_pattern_bindings(&mut self, pattern_id: PatternId, subject: &GateType) -> GateExprEnv {
        let mut env = GateExprEnv::default();
        let mut work = vec![(pattern_id, subject.clone())];
        while let Some((pattern_id, subject_ty)) = work.pop() {
            let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
                continue;
            };
            match pattern.kind {
                PatternKind::Wildcard
                | PatternKind::Integer(_)
                | PatternKind::Text(_)
                | PatternKind::UnresolvedName(_) => {}
                PatternKind::Binding(binding) => {
                    env.locals.insert(binding.binding, subject_ty);
                }
                PatternKind::Tuple(elements) => {
                    let GateType::Tuple(subject_elements) = &subject_ty else {
                        continue;
                    };
                    if elements.len() != subject_elements.len() {
                        continue;
                    }
                    let element_pairs = elements
                        .iter()
                        .zip(subject_elements.iter())
                        .collect::<Vec<_>>();
                    for (element, element_ty) in element_pairs.into_iter().rev() {
                        work.push((*element, element_ty.clone()));
                    }
                }
                PatternKind::List { elements, rest } => {
                    let GateType::List(element_ty) = &subject_ty else {
                        continue;
                    };
                    for element in elements.into_iter().rev() {
                        work.push((element, element_ty.as_ref().clone()));
                    }
                    if let Some(rest) = rest {
                        work.push((rest, subject_ty));
                    }
                }
                PatternKind::Record(fields) => {
                    let GateType::Record(subject_fields) = &subject_ty else {
                        continue;
                    };
                    for field in fields.into_iter().rev() {
                        let Some(field_ty) = subject_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                            .map(|field_ty| field_ty.ty.clone())
                        else {
                            continue;
                        };
                        work.push((field.pattern, field_ty));
                    }
                }
                PatternKind::Constructor { callee, arguments } => {
                    let Some(field_types) = self.case_pattern_field_types(&callee, &subject_ty)
                    else {
                        continue;
                    };
                    if field_types.len() != arguments.len() {
                        continue;
                    }
                    for (argument, field_ty) in arguments.into_iter().zip(field_types).rev() {
                        work.push((argument, field_ty));
                    }
                }
            }
        }
        env
    }

    fn case_pattern_field_types(
        &mut self,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let key = case_constructor_key(callee)?;
        let subject = self.case_subject_shape(subject)?;
        subject.constructor(&key)?.field_types.clone()
    }

    pub(crate) fn apply_fanout_plan(&self, plan: FanoutPlan, subject: GateType) -> GateType {
        match plan.result() {
            FanoutResultKind::MappedCollection => {
                let mapped_collection = GateType::List(Box::new(subject));
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(mapped_collection))
                } else {
                    mapped_collection
                }
            }
            FanoutResultKind::JoinedValue => {
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(subject))
                } else {
                    subject
                }
            }
        }
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
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), false)
    }

    pub(crate) fn lower_open_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), true)
    }

    pub(crate) fn lower_hir_type(
        &mut self,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        self.lower_type(ty, substitutions, &mut Vec::new(), false)
    }

    pub(crate) fn poly_type_binding(&mut self, ty: TypeId) -> Option<TypeBinding> {
        if let Some(lowered) = self.lower_annotation(ty) {
            return Some(TypeBinding::Type(lowered));
        }
        let mut item_stack = Vec::new();
        self.partial_type_constructor_binding(ty, &mut item_stack)
            .map(TypeBinding::Constructor)
    }

    pub(crate) fn instantiate_poly_hir_type(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type(ty, bindings, &mut Vec::new())
    }

    pub(crate) fn instantiate_poly_hir_type_partially(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type_partially(ty, bindings, &mut Vec::new())
    }

    fn lower_poly_type_partially(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return Some(lowered);
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.get(parameter) {
                        Some(TypeBinding::Type(ty)) => Some(ty.clone()),
                        Some(TypeBinding::Constructor(binding)) => {
                            let arity = type_constructor_arity(binding.head, self.module);
                            (binding.arguments.len() == arity)
                                .then(|| {
                                    self.apply_type_constructor(
                                        binding.head,
                                        &binding.arguments,
                                        item_stack,
                                    )
                                })
                                .flatten()
                        }
                        None => Some(GateType::TypeParameter {
                            parameter: *parameter,
                            name: self.module.type_parameters()[*parameter]
                                .name
                                .text()
                                .to_owned(),
                        }),
                    }
                }
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
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    self.lower_type_item(*item_id, &[], item_stack, true)
                }
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type_partially(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type_partially(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(
                    self.lower_poly_type_partially(*parameter, bindings, item_stack)?,
                ),
                result: Box::new(self.lower_poly_type_partially(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(
                                    binding.head,
                                    &all_arguments,
                                    item_stack,
                                )
                            })
                            .flatten()
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() != arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        self.apply_type_constructor(head, &lowered_arguments, item_stack)
                    }
                }
            }
        }
    }

    pub(crate) fn match_poly_hir_type(
        &mut self,
        ty: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        self.match_poly_hir_type_inner(ty, actual, bindings, &mut Vec::new())
    }

    pub(crate) fn match_poly_type_binding(
        &mut self,
        ty: TypeId,
        actual: &TypeBinding,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        if let Some(candidate) = self.instantiate_poly_type_binding(ty, bindings) {
            return candidate.matches(actual);
        }
        match (&self.module.types()[ty].kind, actual) {
            (TypeKind::Name(reference), _) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            (TypeKind::Apply { callee, arguments }, TypeBinding::Constructor(actual_binding)) => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return false;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Item(_)) => {
                        let Some((head, _)) = self.type_constructor_head_and_arity(*callee) else {
                            return false;
                        };
                        if head != actual_binding.head()
                            || arguments.len() != actual_binding.arguments.len()
                        {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments.iter().zip(actual_binding.arguments.iter()).all(
                            |(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            },
                        )
                    }
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let Some(prefix_len) =
                            actual_binding.arguments.len().checked_sub(arguments.len())
                        else {
                            return false;
                        };
                        let prefix = TypeBinding::Constructor(TypeConstructorBinding {
                            head: actual_binding.head(),
                            arguments: actual_binding.arguments[..prefix_len].to_vec(),
                        });
                        let matches_prefix = match bindings.entry(*parameter) {
                            Entry::Occupied(entry) => entry.get().matches(&prefix),
                            Entry::Vacant(entry) => {
                                entry.insert(prefix);
                                true
                            }
                        };
                        if !matches_prefix {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments
                            .iter()
                            .zip(actual_binding.arguments[prefix_len..].iter())
                            .all(|(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            })
                    }
                    _ => false,
                }
            }
            (TypeKind::Tuple(_), _)
            | (TypeKind::Record(_), _)
            | (TypeKind::Arrow { .. }, _)
            | (TypeKind::Apply { .. }, TypeBinding::Type(_)) => false,
        }
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
        self.item_types.insert(item_id, None);
        let ty = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| self.infer_expr(item.body, &GateExprEnv::default(), None).ty),
            Item::Function(item) => {
                let mut env = GateExprEnv::default();
                let mut parameters = Vec::with_capacity(item.parameters.len());
                for parameter in &item.parameters {
                    let annotation = parameter.annotation?;
                    let parameter_ty = self.lower_open_annotation(annotation)?;
                    env.locals.insert(parameter.binding, parameter_ty.clone());
                    parameters.push(parameter_ty);
                }
                let result = item
                    .annotation
                    .and_then(|annotation| self.lower_open_annotation(annotation))
                    .or_else(|| self.infer_expr(item.body, &env, None).ty)?;
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
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    Some(GateType::Signal(Box::new(
                        self.infer_expr(body, &GateExprEnv::default(), None).ty?,
                    )))
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        };
        self.item_types.insert(item_id, ty.clone());
        ty
    }

    fn item_value_actual(&mut self, item_id: ItemId) -> Option<SourceOptionActualType> {
        if let Some(cached) = self.item_actuals.get(&item_id) {
            return cached.clone();
        }
        self.item_actuals.insert(item_id, None);
        let actual = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    self.infer_expr(item.body, &GateExprEnv::default(), None)
                        .actual()
                }),
            Item::Function(_) => self
                .item_value_type(item_id)
                .map(|ty| SourceOptionActualType::from_gate_type(&ty)),
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    let body_actual = self
                        .infer_expr(body, &GateExprEnv::default(), None)
                        .actual()?;
                    Some(SourceOptionActualType::Signal(Box::new(body_actual)))
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        };
        self.item_actuals.insert(item_id, actual.clone());
        actual
    }

    fn finalize_expr_info(&self, mut info: GateExprInfo) -> GateExprInfo {
        if let Some(ty) = info.ty.as_ref() {
            let actual_matches_ty = info
                .actual
                .as_ref()
                .and_then(SourceOptionActualType::to_gate_type)
                .is_some_and(|actual| actual.same_shape(ty));
            if !actual_matches_ty {
                info.actual = Some(SourceOptionActualType::from_gate_type(ty));
            }
        }
        info.contains_signal |= info.ty.as_ref().is_some_and(GateType::is_signal)
            || info
                .actual
                .as_ref()
                .is_some_and(SourceOptionActualType::is_signal);
        info
    }

    fn import_value_type(&self, import_id: ImportId) -> Option<GateType> {
        let import = &self.module.imports()[import_id];
        match &import.metadata {
            ImportBindingMetadata::Value { ty }
            | ImportBindingMetadata::IntrinsicValue { ty, .. } => {
                Some(self.lower_import_value_type(ty))
            }
            ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::BuiltinType(_)
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_)
            | ImportBindingMetadata::Unknown => None,
        }
    }

    fn lower_import_value_type(&self, ty: &ImportValueType) -> GateType {
        match ty {
            ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
            ImportValueType::Tuple(elements) => GateType::Tuple(
                elements
                    .iter()
                    .map(|element| self.lower_import_value_type(element))
                    .collect(),
            ),
            ImportValueType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .map(|field| GateRecordField {
                        name: field.name.to_string(),
                        ty: self.lower_import_value_type(&field.ty),
                    })
                    .collect(),
            ),
            ImportValueType::Arrow { parameter, result } => GateType::Arrow {
                parameter: Box::new(self.lower_import_value_type(parameter)),
                result: Box::new(self.lower_import_value_type(result)),
            },
            ImportValueType::List(element) => {
                GateType::List(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Map { key, value } => GateType::Map {
                key: Box::new(self.lower_import_value_type(key)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Set(element) => {
                GateType::Set(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Option(element) => {
                GateType::Option(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Result { error, value } => GateType::Result {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Validation { error, value } => GateType::Validation {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Signal(element) => {
                GateType::Signal(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Task { error, value } => GateType::Task {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
        }
    }

    fn domain_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<DomainMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn domain_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.domain_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.domain_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn infer_domain_member_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let candidates = self.domain_member_candidates(reference)?;
        if candidates.len() != 1 {
            return None;
        }
        self.lower_domain_member_annotation(candidates[0], &HashMap::new())
    }

    pub(crate) fn select_domain_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<DomainMemberSelection<GateType>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_name_candidate(resolution, expected)
            }),
        )
    }

    pub(crate) fn select_domain_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<DomainMemberCallMatch>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_call_candidate(resolution, argument_types, expected_result)
            }),
        )
    }

    fn class_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<ClassMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::ClassMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn class_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.class_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.class_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn select_class_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<ClassMemberCallMatch>> {
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) =
                self.match_class_member_call_candidate(candidate, argument_types, expected_result)
            {
                matches.push(matched);
            }
        }
        Some(match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one class member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        })
    }

    fn match_class_member_call_candidate(
        &mut self,
        resolution: ClassMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<ClassMemberCallMatch> {
        let (class_parameter, member_annotation, member_context) =
            self.class_member_signature(resolution)?;
        let mut bindings = PolyTypeBindings::new();
        let mut current = member_annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            if !self.match_poly_hir_type(parameter, argument, &mut bindings) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result
            && !self.match_poly_hir_type(current, expected, &mut bindings)
        {
            return None;
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            parameters.push(self.instantiate_poly_hir_type(parameter, &bindings)?);
        }
        let result = self.instantiate_poly_hir_type(current, &bindings)?;
        if let Some(expected) = expected_result
            && !result.same_shape(expected)
        {
            return None;
        }

        let evidence = ClassConstraintBinding {
            class_item: resolution.class,
            subject: bindings.get(&class_parameter)?.clone(),
        };
        let constraints = member_context
            .iter()
            .map(|constraint| self.class_constraint_binding(*constraint, &bindings))
            .collect::<Option<Vec<_>>>()?;
        Some(ClassMemberCallMatch {
            resolution,
            parameters,
            result,
            evidence,
            constraints,
        })
    }

    fn class_member_signature(
        &self,
        resolution: ClassMemberResolution,
    ) -> Option<(TypeParameterId, TypeId, Vec<TypeId>)> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        Some((
            *class_item.parameters.first(),
            member.annotation,
            member.context.clone(),
        ))
    }

    fn class_member_label(&self, resolution: ClassMemberResolution) -> Option<String> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        Some(format!("{}.{}", class_item.name.text(), member.name.text()))
    }

    pub(crate) fn class_constraint_binding(
        &mut self,
        constraint: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<ClassConstraintBinding> {
        let (class_item, subject) = self.class_constraint_parts(constraint)?;
        Some(ClassConstraintBinding {
            class_item,
            subject: self.instantiate_poly_type_binding(subject, bindings)?,
        })
    }

    fn class_constraint_parts(&self, constraint: TypeId) -> Option<(ItemId, TypeId)> {
        let ty = self.module.types().get(constraint)?;
        match &ty.kind {
            TypeKind::Apply { callee, arguments } if arguments.len() == 1 => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
                    reference.resolution.as_ref()
                else {
                    return None;
                };
                matches!(self.module.items()[*item_id], Item::Class(_))
                    .then_some((*item_id, *arguments.first()))
            }
            _ => None,
        }
    }

    fn select_domain_member_candidate<T>(
        &mut self,
        candidates: Vec<DomainMemberResolution>,
        mut matcher: impl FnMut(&mut Self, DomainMemberResolution) -> Option<T>,
    ) -> DomainMemberSelection<T> {
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) = matcher(self, candidate) {
                matches.push(matched);
            }
        }
        match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one domain member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        }
    }

    fn match_domain_member_name_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        expected: &GateType,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !self.match_hir_type(annotation, expected, &mut substitutions, &mut item_stack) {
            return None;
        }
        let lowered = self.lower_domain_member_annotation(resolution, &substitutions)?;
        lowered.same_shape(expected).then_some(lowered)
    }

    fn match_domain_member_call_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberCallMatch> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut current = annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            let mut item_stack = Vec::new();
            if !self.match_hir_type(parameter, argument, &mut substitutions, &mut item_stack) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(current, expected, &mut substitutions, &mut item_stack) {
                return None;
            }
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            let mut item_stack = Vec::new();
            parameters.push(self.lower_type(parameter, &substitutions, &mut item_stack, false)?);
        }
        let mut item_stack = Vec::new();
        let result = self.lower_type(current, &substitutions, &mut item_stack, false)?;
        if let Some(expected) = expected_result {
            if !result.same_shape(expected) {
                return None;
            }
        }
        Some(DomainMemberCallMatch { parameters, result })
    }

    fn match_hir_type(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_type(type_id, substitutions, item_stack, false) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match substitutions.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().same_shape(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_hir_type(*element, actual, substitutions, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_hir_type(field.ty, &actual_field.ty, substitutions, item_stack)
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_hir_type(parameter, actual_parameter, substitutions, item_stack)
                    && self.match_hir_type(result, actual_result, substitutions, item_stack)
            }
            TypeKind::Apply { callee, arguments } => self.match_hir_type_application(
                callee,
                &arguments,
                actual,
                substitutions,
                item_stack,
            ),
        }
    }

    fn match_hir_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        let arguments = arguments.iter().copied().collect::<Vec<_>>();
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                let GateType::List(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                let GateType::Map { key, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], key, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                let GateType::Set(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                let GateType::Option(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                let GateType::Result { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                let GateType::Validation { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                let GateType::Signal(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                let GateType::Task { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => match actual {
                GateType::Domain {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                GateType::OpaqueItem {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                _ => false,
            },
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_)) => false,
        }
    }

    fn lower_domain_member_annotation(
        &mut self,
        resolution: DomainMemberResolution,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut item_stack = Vec::new();
        self.lower_type(annotation, substitutions, &mut item_stack, false)
    }

    fn domain_member_annotation(&self, resolution: DomainMemberResolution) -> Option<TypeId> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        domain
            .members
            .get(resolution.member_index)
            .filter(|member| member.kind == DomainMemberKind::Method)
            .map(|member| member.annotation)
    }

    fn domain_member_label(&self, resolution: DomainMemberResolution) -> Option<String> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        let member = domain.members.get(resolution.member_index)?;
        Some(format!("{}.{}", domain.name.text(), member.name.text()))
    }

    fn lower_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => self.lower_type_reference(
                reference,
                substitutions,
                item_stack,
                allow_open_type_parameters,
            ),
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_type(
                        *element,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_type(
                            field.ty,
                            substitutions,
                            item_stack,
                            allow_open_type_parameters,
                        )?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_type(
                    *parameter,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
                result: Box::new(self.lower_type(
                    *result,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                self.lower_type_application(
                    *callee,
                    &lowered_arguments,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )
            }
        }
    }

    fn lower_type_reference(
        &mut self,
        reference: &TypeReference,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
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
                substitutions.get(parameter).cloned().or_else(|| {
                    allow_open_type_parameters.then(|| GateType::TypeParameter {
                        parameter: *parameter,
                        name: self.module.type_parameters()[*parameter]
                            .name
                            .text()
                            .to_owned(),
                    })
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, allow_open_type_parameters)
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
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Some(GateType::List(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                Some(GateType::Map {
                    key: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                Some(GateType::Set(Box::new(arguments.first()?.clone())))
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
                self.lower_type_item(*item_id, arguments, item_stack, allow_open_type_parameters)
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
        allow_open_type_parameters: bool,
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
                            self.lower_type(
                                *alias,
                                &substitutions,
                                item_stack,
                                allow_open_type_parameters,
                            )
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
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => None,
        };
        let popped = item_stack.pop();
        debug_assert_eq!(popped, Some(item_id));
        lowered
    }

    fn lower_poly_type(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => {
                self.lower_poly_type_reference(reference, bindings, item_stack)
            }
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_poly_type(*parameter, bindings, item_stack)?),
                result: Box::new(self.lower_poly_type(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        self.apply_type_constructor(binding.head, &all_arguments, item_stack)
                    }
                    _ => {
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        (lowered_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(head, &lowered_arguments, item_stack)
                            })
                            .flatten()
                    }
                }
            }
        }
    }

    fn lower_poly_type_reference(
        &mut self,
        reference: &TypeReference,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                match bindings.get(parameter)? {
                    TypeBinding::Type(ty) => Some(ty.clone()),
                    TypeBinding::Constructor(binding) => {
                        self.apply_type_constructor(binding.head, &binding.arguments, item_stack)
                    }
                }
            }
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
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, false)
            }
            ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Unresolved => None,
        }
    }

    fn instantiate_poly_type_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<TypeBinding> {
        let mut item_stack = Vec::new();
        if let Some(ty) = self.lower_poly_type(type_id, bindings, &mut item_stack) {
            return Some(TypeBinding::Type(ty));
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    bindings.get(parameter).cloned()
                }
                _ => self
                    .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                    .map(TypeBinding::Constructor),
            },
            TypeKind::Apply { .. } => self
                .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                .map(TypeBinding::Constructor),
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
        }
    }

    fn partial_poly_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                        return None;
                    };
                    Some(binding.clone())
                }
                _ => {
                    let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                    (arity > 0).then_some(TypeConstructorBinding {
                        head,
                        arguments: Vec::new(),
                    })
                }
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() < arity).then_some(TypeConstructorBinding {
                            head: binding.head,
                            arguments: all_arguments,
                        })
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() >= arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        Some(TypeConstructorBinding {
                            head,
                            arguments: lowered_arguments,
                        })
                    }
                }
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
        }
    }

    fn match_poly_hir_type_inner(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let candidate = TypeBinding::Type(actual.clone());
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(&candidate),
                        Entry::Vacant(entry) => {
                            entry.insert(candidate);
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_poly_hir_type_inner(*element, actual, bindings, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_poly_hir_type_inner(
                            field.ty,
                            &actual_field.ty,
                            bindings,
                            item_stack,
                        )
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_poly_hir_type_inner(parameter, actual_parameter, bindings, item_stack)
                    && self.match_poly_hir_type_inner(result, actual_result, bindings, item_stack)
            }
            TypeKind::Apply { callee, arguments } => {
                self.match_poly_type_application(callee, &arguments, actual, bindings, item_stack)
            }
        }
    }

    fn match_poly_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                let Some((head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                let pattern_arguments = arguments.iter().copied().collect::<Vec<_>>();
                if actual_arguments.len() < pattern_arguments.len() {
                    return false;
                }
                let prefix_count = actual_arguments.len() - pattern_arguments.len();
                let candidate = TypeBinding::Constructor(TypeConstructorBinding {
                    head,
                    arguments: actual_arguments[..prefix_count].to_vec(),
                });
                match bindings.entry(*parameter) {
                    Entry::Occupied(entry) if !entry.get().matches(&candidate) => return false,
                    Entry::Occupied(_) => {}
                    Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                }
                pattern_arguments
                    .iter()
                    .zip(actual_arguments[prefix_count..].iter())
                    .all(|(argument, actual_argument)| {
                        self.match_poly_hir_type_inner(
                            *argument,
                            actual_argument,
                            bindings,
                            item_stack,
                        )
                    })
            }
            _ => {
                let Some((expected_head, _)) = self.type_constructor_head_and_arity(callee) else {
                    return false;
                };
                let Some((actual_head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                expected_head == actual_head
                    && actual_arguments.len() >= arguments.len()
                    && arguments.iter().zip(actual_arguments.iter()).all(
                        |(argument, actual_argument)| {
                            self.match_poly_hir_type_inner(
                                *argument,
                                actual_argument,
                                bindings,
                                item_stack,
                            )
                        },
                    )
            }
        }
    }

    fn partial_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(_) => {
                let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                (arity > 0).then_some(TypeConstructorBinding {
                    head,
                    arguments: Vec::new(),
                })
            }
            TypeKind::Apply { callee, arguments } => {
                let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                if arguments.len() >= arity {
                    return None;
                }
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        &HashMap::new(),
                        item_stack,
                        false,
                    )?);
                }
                Some(TypeConstructorBinding {
                    head,
                    arguments: lowered_arguments,
                })
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
        }
    }

    fn type_constructor_head_and_arity(
        &self,
        type_id: TypeId,
    ) -> Option<(TypeConstructorHead, usize)> {
        let TypeKind::Name(reference) = &self.module.types()[type_id].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => Some((
                TypeConstructorHead::Builtin(*builtin),
                builtin_type_arity(*builtin),
            )),
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                let arity = match &self.module.items()[*item_id] {
                    Item::Type(item) => item.parameters.len(),
                    Item::Domain(item) => item.parameters.len(),
                    _ => return None,
                };
                Some((TypeConstructorHead::Item(*item_id), arity))
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Unresolved => None,
        }
    }

    fn apply_type_constructor(
        &mut self,
        head: TypeConstructorHead,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match head {
            TypeConstructorHead::Builtin(builtin) => {
                self.apply_builtin_type_constructor(builtin, arguments)
            }
            TypeConstructorHead::Item(item_id) => {
                self.lower_type_item(item_id, arguments, item_stack, false)
            }
        }
    }

    fn apply_builtin_type_constructor(
        &self,
        builtin: BuiltinType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        if arguments.len() != builtin_type_arity(builtin) {
            return None;
        }
        match builtin {
            BuiltinType::Int
            | BuiltinType::Float
            | BuiltinType::Decimal
            | BuiltinType::BigInt
            | BuiltinType::Bool
            | BuiltinType::Text
            | BuiltinType::Unit
            | BuiltinType::Bytes => Some(GateType::Primitive(builtin)),
            BuiltinType::List => Some(GateType::List(Box::new(arguments.first()?.clone()))),
            BuiltinType::Map => Some(GateType::Map {
                key: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Set => Some(GateType::Set(Box::new(arguments.first()?.clone()))),
            BuiltinType::Option => Some(GateType::Option(Box::new(arguments.first()?.clone()))),
            BuiltinType::Result => Some(GateType::Result {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Validation => Some(GateType::Validation {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Signal => Some(GateType::Signal(Box::new(arguments.first()?.clone()))),
            BuiltinType::Task => Some(GateType::Task {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
        }
    }

    pub(crate) fn infer_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> GateExprInfo {
        let expr = self.module.exprs()[expr_id].clone();
        let info = match expr.kind {
            ExprKind::Name(reference) => self.infer_name(&reference, env),
            ExprKind::Integer(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Int)),
                ..GateExprInfo::default()
            },
            ExprKind::Float(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Float)),
                ..GateExprInfo::default()
            },
            ExprKind::Decimal(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Decimal)),
                ..GateExprInfo::default()
            },
            ExprKind::BigInt(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::BigInt)),
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
                for element in elements.iter() {
                    let child = self.infer_expr(*element, env, ambient);
                    if let Some(ty) = child.actual() {
                        lowered.push(ty);
                    }
                    info.merge(child);
                }
                if lowered.len() == elements.len() {
                    info.set_actual(SourceOptionActualType::Tuple(lowered));
                }
                info
            }
            ExprKind::List(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut element_gate_type = None::<GateType>;
                let mut consistent = true;
                for element in &elements {
                    let child = self.infer_expr(*element, env, ambient);
                    if consistent {
                        if let Some(child_ty) = child.actual_gate_type().or(child.ty.clone()) {
                            element_gate_type = match element_gate_type.take() {
                                None => Some(child_ty),
                                Some(current) => {
                                    if current.same_shape(&child_ty) {
                                        Some(current)
                                    } else {
                                        consistent = false;
                                        None
                                    }
                                }
                            };
                        }
                    }
                    if consistent {
                        if let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    } else {
                        let _ = child.actual();
                    }
                    info.merge(child);
                }
                if consistent {
                    if let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::List(Box::new(element_type)));
                        if info.ty.is_none() {
                            if let Some(element_gate_type) = element_gate_type {
                                info.ty = Some(GateType::List(Box::new(element_gate_type)));
                            }
                        }
                    } else if let Some(element_gate_type) = element_gate_type {
                        info.ty = Some(GateType::List(Box::new(element_gate_type)));
                    }
                }
                info
            }
            ExprKind::Map(map) => {
                let mut info = GateExprInfo::default();
                let mut key_type = None::<SourceOptionActualType>;
                let mut value_type = None::<SourceOptionActualType>;
                let mut keys_consistent = true;
                let mut values_consistent = true;
                for entry in &map.entries {
                    let key = self.infer_expr(entry.key, env, ambient);
                    if keys_consistent {
                        if let Some(child_ty) = key.actual() {
                            key_type = match key_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        keys_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(key);

                    let value = self.infer_expr(entry.value, env, ambient);
                    if values_consistent {
                        if let Some(child_ty) = value.actual() {
                            value_type = match value_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        values_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(value);
                }
                if keys_consistent && values_consistent {
                    if let (Some(key), Some(value)) = (key_type, value_type) {
                        info.set_actual(SourceOptionActualType::Map {
                            key: Box::new(key),
                            value: Box::new(value),
                        });
                    }
                }
                info
            }
            ExprKind::Set(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut consistent = true;
                for element in elements {
                    let child = self.infer_expr(element, env, ambient);
                    if consistent {
                        if let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(child);
                }
                if consistent {
                    if let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::Set(Box::new(element_type)));
                    }
                }
                info
            }
            ExprKind::Record(record) => {
                let mut info = GateExprInfo::default();
                let field_count = record.fields.len();
                let mut fields = Vec::with_capacity(field_count);
                for field in record.fields {
                    let child = self.infer_expr(field.value, env, ambient);
                    if let Some(ty) = child.actual() {
                        fields.push(SourceOptionActualRecordField {
                            name: field.label.text().to_owned(),
                            ty,
                        });
                    }
                    info.merge(child);
                }
                if fields.len() == field_count {
                    info.set_actual(SourceOptionActualType::Record(fields));
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
                if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind {
                    if let Some(info) = self
                        .infer_builtin_constructor_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) =
                        self.infer_domain_member_apply(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) =
                        self.infer_class_member_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) = self.infer_same_module_constructor_apply_expr(
                        reference, &arguments, env, ambient,
                    ) {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) = self
                        .infer_polymorphic_function_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                }
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
                info.ty = if let (Some(left), Some(right)) = (left_ty.as_ref(), right_ty.as_ref()) {
                    match select_domain_binary_operator(self.module, self, operator, left, right) {
                        Ok(maybe_matched) => maybe_matched.map(|matched| matched.result_type),
                        Err(candidates) => {
                            // Multiple domain operator implementations match: emit an ambiguity
                            // diagnostic and leave the result type unknown so downstream checking
                            // can continue without cascading false errors.
                            info.issues.push(GateIssue::AmbiguousDomainOperator {
                                span: expr.span,
                                operator: binary_operator_text(operator).to_owned(),
                                candidates: candidates
                                    .into_iter()
                                    .map(|c| {
                                        format!("{}.{}", c.callee.domain_name, c.callee.member_name)
                                    })
                                    .collect(),
                            });
                            None
                        }
                    }
                } else {
                    None
                };
                if info.ty.is_some() {
                    return self.finalize_expr_info(info);
                }
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
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Multiply)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Divide)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Modulo)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(left.clone())
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Equals)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::NotEquals)
                        if left.same_shape(right) =>
                    {
                        info.constraints
                            .push(TypeConstraint::eq(expr.span, left.clone()));
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Pipe(pipe) => self.infer_pipe_expr(&pipe, env),
            ExprKind::Cluster(cluster) => self.infer_cluster_expr(cluster, env),
            ExprKind::Markup(_) => GateExprInfo::default(),
        };
        self.finalize_expr_info(info)
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
                let constructor_ty = self.infer_same_module_constructor_name_type(reference);
                let ty = constructor_ty
                    .clone()
                    .or_else(|| self.item_value_type(*item_id));
                let actual = constructor_ty
                    .as_ref()
                    .map(SourceOptionActualType::from_gate_type)
                    .or_else(|| self.item_value_actual(*item_id));
                GateExprInfo {
                    actual,
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                let ty = self.import_value_type(*import_id);
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::DomainMember(_)) => GateExprInfo {
                ty: self.infer_domain_member_name_type(reference),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_)) => GateExprInfo {
                issues: vec![GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                }],
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => {
                GateExprInfo::default()
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(_)) => GateExprInfo::default(),
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                let (ty, actual) = match builtin {
                    crate::hir::BuiltinTerm::True | crate::hir::BuiltinTerm::False => {
                        (Some(GateType::Primitive(BuiltinType::Bool)), None)
                    }
                    crate::hir::BuiltinTerm::None => (
                        None,
                        Some(SourceOptionActualType::Option(Box::new(
                            SourceOptionActualType::Hole,
                        ))),
                    ),
                    crate::hir::BuiltinTerm::Some
                    | crate::hir::BuiltinTerm::Ok
                    | crate::hir::BuiltinTerm::Err
                    | crate::hir::BuiltinTerm::Valid
                    | crate::hir::BuiltinTerm::Invalid => (None, None),
                };
                GateExprInfo {
                    actual,
                    ty,
                    ..GateExprInfo::default()
                }
            }
        }
    }

    fn same_module_constructor(
        &self,
        reference: &TermReference,
    ) -> Option<(ItemId, String, Vec<TypeParameterId>, Vec<TypeId>)> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Type(item) = &self.module.items()[*item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let variant_name = reference.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        Some((
            *item_id,
            item.name.text().to_owned(),
            item.parameters.clone(),
            variant.fields.clone(),
        ))
    }

    fn infer_builtin_constructor_actual(
        &self,
        builtin: BuiltinTerm,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        match (builtin, arguments) {
            (BuiltinTerm::True | BuiltinTerm::False, []) => {
                Some(SourceOptionActualType::Primitive(BuiltinType::Bool))
            }
            (BuiltinTerm::None, []) => Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
            (BuiltinTerm::Some, [argument]) => {
                Some(SourceOptionActualType::Option(Box::new(argument.clone())))
            }
            (BuiltinTerm::Ok, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Err, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            (BuiltinTerm::Valid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Invalid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            _ => None,
        }
    }

    fn infer_builtin_constructor_actual_from_reference(
        &self,
        reference: &TermReference,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        self.infer_builtin_constructor_actual(*builtin, arguments)
    }

    fn infer_builtin_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let mut info = GateExprInfo::default();
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            info.merge(argument_info);
            let Some(argument_actual) = argument_actual else {
                return Some(info);
            };
            argument_actuals.push(argument_actual);
        }
        let actual = self.infer_builtin_constructor_actual(*builtin, &argument_actuals)?;
        info.set_actual(actual);
        Some(info)
    }

    fn infer_same_module_constructor_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if !parameters.is_empty() {
            return None;
        }
        let substitutions = HashMap::new();
        let field_types = fields
            .into_iter()
            .map(|field| self.lower_hir_type(field, &substitutions))
            .collect::<Option<Vec<_>>>()?;
        let mut ty = GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments: Vec::new(),
        };
        for field_ty in field_types.into_iter().rev() {
            ty = GateType::Arrow {
                parameter: Box::new(field_ty),
                result: Box::new(ty),
            };
        }
        Some(ty)
    }

    fn infer_same_module_constructor_apply(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_types.len() {
            return None;
        }
        let mut substitutions = HashMap::new();
        for (field, actual) in fields.iter().zip(argument_types.iter()) {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(*field, actual, &mut substitutions, &mut item_stack) {
                return None;
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| substitutions.get(parameter).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    fn infer_same_module_constructor_apply_actual(
        &mut self,
        reference: &TermReference,
        argument_actuals: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_actuals.len() {
            return None;
        }
        let validator = Validator {
            module: self.module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut substitutions = HashMap::<TypeParameterId, SourceOptionActualType>::new();
        for (field, actual) in fields.iter().zip(argument_actuals.iter()) {
            match validator.source_option_hir_type_matches_actual_type_inner(
                *field,
                actual,
                &mut substitutions,
            ) {
                Some(true) => {}
                Some(false) | None => return None,
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| {
                substitutions
                    .get(parameter)
                    .cloned()
                    .unwrap_or(SourceOptionActualType::Hole)
            })
            .collect();
        Some(SourceOptionActualType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    fn infer_same_module_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.same_module_constructor(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
            argument_actuals.push(argument_actual);
        }
        if let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() {
            info.ty = self.infer_same_module_constructor_apply(reference, &argument_types);
        }
        if info.ty.is_none() {
            let Some(argument_actuals) = argument_actuals.into_iter().collect::<Option<Vec<_>>>()
            else {
                return Some(info);
            };
            if let Some(actual) =
                self.infer_same_module_constructor_apply_actual(reference, &argument_actuals)
            {
                info.set_actual(actual);
            }
        }
        Some(info)
    }

    fn match_function_signature(
        &mut self,
        function: &crate::hir::FunctionItem,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<(Vec<GateType>, GateType)> {
        if function.parameters.len() != argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(function.parameters.len());
        for (parameter, actual) in function.parameters.iter().zip(argument_types.iter()) {
            let annotation = parameter.annotation?;
            if let Some(lowered) = self.lower_annotation(annotation) {
                if !lowered.same_shape(actual) {
                    return None;
                }
                instantiated_parameters.push(lowered);
                continue;
            }
            if !self.match_poly_hir_type(annotation, actual, &mut bindings) {
                return None;
            }
            instantiated_parameters.push(self.instantiate_poly_hir_type(annotation, &bindings)?);
        }
        let result_annotation = function.annotation?;
        if let Some(expected) = expected_result {
            if let Some(lowered) = self.lower_annotation(result_annotation) {
                if !lowered.same_shape(expected) {
                    return None;
                }
            } else if !self.match_poly_hir_type(result_annotation, expected, &mut bindings) {
                return None;
            }
        }
        let result = self
            .lower_annotation(result_annotation)
            .or_else(|| self.instantiate_poly_hir_type(result_annotation, &bindings))?;
        Some((instantiated_parameters, result))
    }

    fn flatten_apply_expr(&self, expr_id: ExprId) -> (ExprId, Vec<ExprId>) {
        let mut callee = expr_id;
        let mut segments = Vec::new();
        while let ExprKind::Apply {
            callee: next_callee,
            arguments: next_arguments,
        } = &self.module.exprs()[callee].kind
        {
            segments.push(next_arguments.iter().copied().collect::<Vec<_>>());
            callee = *next_callee;
        }
        let mut arguments = Vec::new();
        for segment in segments.into_iter().rev() {
            arguments.extend(segment);
        }
        (callee, arguments)
    }

    fn match_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<()> {
        if let Some(lowered) = self.lower_annotation(annotation) {
            lowered.same_shape(actual).then_some(())
        } else {
            self.match_poly_hir_type(annotation, actual, bindings)
                .then_some(())
        }
    }

    fn match_pipe_argument_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<bool> {
        if self
            .match_function_parameter_annotation(annotation, actual, bindings)
            .is_some()
        {
            return Some(false);
        }
        let GateType::Signal(payload) = actual else {
            return None;
        };
        self.match_function_parameter_annotation(annotation, payload, bindings)
            .map(|_| true)
    }

    fn instantiate_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_annotation(annotation)
            .or_else(|| self.instantiate_poly_hir_type(annotation, bindings))
            .or_else(|| self.instantiate_poly_hir_type_partially(annotation, bindings))
    }

    pub(crate) fn match_pipe_function_signature(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let (callee_expr, explicit_arguments) = self.flatten_apply_expr(expr_id);
        let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind else {
            return None;
        };
        if let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        {
            let Item::Function(function) = &self.module.items()[*item_id] else {
                return None;
            };
            if function.parameters.len() != explicit_arguments.len() + 1 {
                return None;
            }
            if function.annotation.is_none()
                || function.parameters.iter().any(|parameter| parameter.annotation.is_none())
            {
                return self.match_pipe_unannotated_function_signature(
                    callee_expr,
                    &explicit_arguments,
                    function,
                    env,
                    ambient,
                    expected_result,
                );
            }

            let mut bindings = PolyTypeBindings::new();
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
                let annotation = parameter.annotation?;
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                let Some(argument_ty) = argument_info.actual_gate_type().or(argument_info.ty)
                else {
                    signal_payload_arguments.push(false);
                    continue;
                };
                let Some(reads_signal_payload) = self.match_pipe_argument_parameter_annotation(
                    annotation,
                    &argument_ty,
                    &mut bindings,
                ) else {
                    return None;
                };
                signal_payload_arguments.push(reads_signal_payload);
            }

            let ambient_parameter = function
                .parameters
                .last()
                .expect("checked pipe arity above");
            let ambient_annotation = ambient_parameter.annotation?;
            self.match_function_parameter_annotation(ambient_annotation, ambient, &mut bindings)?;

            let result_annotation = function.annotation?;
            if let Some(expected) = expected_result {
                self.match_function_parameter_annotation(
                    result_annotation,
                    expected,
                    &mut bindings,
                )?;
            }

            let mut parameter_types = Vec::with_capacity(function.parameters.len());
            for parameter in &function.parameters {
                let annotation = parameter.annotation?;
                parameter_types
                    .push(self.instantiate_function_parameter_annotation(annotation, &bindings)?);
            }
            let result_type =
                self.instantiate_function_parameter_annotation(result_annotation, &bindings)?;

            return Some(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments,
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }

        let explicit_argument_types = explicit_arguments
            .iter()
            .map(|argument| {
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                argument_info.actual_gate_type().or(argument_info.ty)
            })
            .collect::<Vec<_>>();
        if let Some(mut full_argument_types) = explicit_argument_types
            .iter()
            .cloned()
            .collect::<Option<Vec<_>>>()
        {
            full_argument_types.push(ambient.clone());
            if let DomainMemberSelection::Unique(matched) =
                self.select_class_member_call(reference, &full_argument_types, expected_result)?
            {
                return Some(PipeFunctionSignatureMatch {
                    callee_expr,
                    explicit_arguments,
                    signal_payload_arguments: vec![
                        false;
                        matched.parameters.len().saturating_sub(1)
                    ],
                    parameter_types: matched.parameters,
                    result_type: matched.result,
                });
            }
        }
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            let (_, member_annotation, _) = self.class_member_signature(candidate)?;
            let mut bindings = PolyTypeBindings::new();
            let mut current = member_annotation;
            let mut parameter_type_ids = Vec::with_capacity(explicit_arguments.len() + 1);
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for argument_ty in explicit_argument_types.iter().cloned() {
                let TypeKind::Arrow { parameter, result } =
                    self.module.types()[current].kind.clone()
                else {
                    continue;
                };
                if let Some(argument_ty) = argument_ty.as_ref() {
                    if self.match_poly_hir_type(parameter, argument_ty, &mut bindings) {
                        signal_payload_arguments.push(false);
                    } else if let GateType::Signal(payload) = argument_ty {
                        if self.match_poly_hir_type(parameter, payload, &mut bindings) {
                            signal_payload_arguments.push(true);
                        } else {
                            parameter_type_ids.clear();
                            signal_payload_arguments.clear();
                            break;
                        }
                    } else {
                        parameter_type_ids.clear();
                        signal_payload_arguments.clear();
                        break;
                    }
                } else {
                    signal_payload_arguments.push(false);
                }
                parameter_type_ids.push(parameter);
                current = result;
            }
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                continue;
            };
            if !self.match_poly_hir_type(parameter, ambient, &mut bindings) {
                continue;
            }
            parameter_type_ids.push(parameter);
            current = result;
            if parameter_type_ids.len() != explicit_arguments.len() + 1 {
                continue;
            }
            if let Some(expected) = expected_result
                && !self.match_poly_hir_type(current, expected, &mut bindings)
            {
                continue;
            }
            let Some(parameter_types) = parameter_type_ids
                .into_iter()
                .map(|parameter| self.instantiate_poly_hir_type_partially(parameter, &bindings))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Some(result_type) = self.instantiate_poly_hir_type_partially(current, &bindings)
            else {
                continue;
            };
            if let Some(expected) = expected_result
                && !result_type.same_shape(expected)
            {
                continue;
            }
            let explicit_arguments_match = explicit_arguments
                .iter()
                .zip(parameter_types.iter().take(explicit_arguments.len()))
                .zip(signal_payload_arguments.iter())
                .all(|((argument, expected_parameter), reads_signal_payload)| {
                    let argument_info = self.infer_expr(*argument, env, Some(ambient));
                    argument_info
                        .actual_gate_type()
                        .or(argument_info.ty.clone())
                        .as_ref()
                        .is_some_and(|actual| {
                            actual.same_shape(expected_parameter)
                                || (*reads_signal_payload
                                    && matches!(
                                        actual,
                                        GateType::Signal(payload)
                                            if payload.same_shape(expected_parameter)
                                    ))
                        })
                        || expression_matches(self.module, *argument, env, expected_parameter)
                });
            if !explicit_arguments_match {
                continue;
            }
            matches.push(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments: explicit_arguments.clone(),
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }
        if matches.len() != 1 {
            return None;
        }
        matches.pop()
    }

    fn match_pipe_unannotated_function_signature(
        &mut self,
        callee_expr: ExprId,
        explicit_arguments: &[ExprId],
        function: &crate::hir::FunctionItem,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let mut bindings = PolyTypeBindings::new();
        let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
        let mut explicit_argument_types = Vec::with_capacity(explicit_arguments.len());

        for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_ty = argument_info.actual_gate_type().or(argument_info.ty);
            if let Some(annotation) = parameter.annotation {
                if let Some(argument_ty) = argument_ty.as_ref() {
                    let reads_signal_payload = self.match_pipe_argument_parameter_annotation(
                        annotation,
                        argument_ty,
                        &mut bindings,
                    )?;
                    signal_payload_arguments.push(reads_signal_payload);
                } else {
                    signal_payload_arguments.push(false);
                }
            } else {
                signal_payload_arguments.push(false);
            }
            explicit_argument_types.push(argument_ty);
        }

        let ambient_parameter = function
            .parameters
            .last()
            .expect("checked pipe arity above");
        if let Some(annotation) = ambient_parameter.annotation {
            self.match_function_parameter_annotation(annotation, ambient, &mut bindings)?;
        }

        if let Some(result_annotation) = function.annotation
            && let Some(expected) = expected_result
        {
            self.match_function_parameter_annotation(result_annotation, expected, &mut bindings)?;
        }

        let mut parameter_types = Vec::with_capacity(function.parameters.len());
        for (index, parameter) in function.parameters.iter().enumerate() {
            let parameter_ty = if let Some(annotation) = parameter.annotation {
                self.instantiate_function_parameter_annotation(annotation, &bindings)?
            } else if index < explicit_argument_types.len() {
                explicit_argument_types[index].clone()?
            } else {
                ambient.clone()
            };
            parameter_types.push(parameter_ty);
        }

        let mut function_env = GateExprEnv::default();
        for (parameter, parameter_ty) in function.parameters.iter().zip(parameter_types.iter()) {
            function_env
                .locals
                .insert(parameter.binding, parameter_ty.clone());
        }

        let result_type = if let Some(result_annotation) = function.annotation {
            self.instantiate_function_parameter_annotation(result_annotation, &bindings)?
        } else {
            let body_info = self.infer_expr(function.body, &function_env, None);
            body_info.actual_gate_type().or(body_info.ty)?
        };
        if let Some(expected) = expected_result
            && !result_type.same_shape(expected)
        {
            return None;
        }

        Some(PipeFunctionSignatureMatch {
            callee_expr,
            explicit_arguments: explicit_arguments.to_vec(),
            signal_payload_arguments,
            parameter_types,
            result_type,
        })
    }

    fn infer_polymorphic_function_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return None;
        };
        if function.type_parameters.is_empty() {
            return None;
        }
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let Some((_, result)) = self.match_function_signature(function, &argument_types, None) {
            info.ty = Some(result);
        }
        Some(info)
    }

    fn infer_class_member_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.class_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let DomainMemberSelection::Unique(matched) =
            self.select_class_member_call(reference, &argument_types, None)?
        {
            info.ty = Some(matched.result);
        }
        Some(info)
    }

    fn infer_domain_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.domain_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        match self.select_domain_member_call(reference, &argument_types, None)? {
            DomainMemberSelection::Unique(matched) => {
                info.ty = Some(matched.result);
            }
            DomainMemberSelection::Ambiguous => {
                info.issues.push(GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                });
            }
            DomainMemberSelection::NoMatch => {}
        }
        Some(info)
    }

    pub(crate) fn infer_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let ambient = subject.gate_payload().clone();
        let mut info = self.infer_expr(expr_id, env, Some(&ambient));
        if let Some(function_body) = self.infer_function_pipe_body(expr_id, env, &ambient, None) {
            info = function_body;
        }
        if let Some(GateType::Arrow { parameter, result }) = info.ty.clone() {
            if parameter.same_shape(&ambient) {
                info.ty = Some(*result);
            } else {
                info.issues.push(GateIssue::InvalidPipeStageInput {
                    span: self.module.exprs()[expr_id].span,
                    stage: "pipe",
                    expected: ambient.to_string(),
                    actual: parameter.to_string(),
                });
                info.ty = None;
            }
        }
        self.finalize_expr_info(info)
    }

    fn infer_tap_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        info.ty = Some(subject.clone());
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_truthy_falsy_branch(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        payload_subject: Option<&GateType>,
    ) -> GateExprInfo {
        match payload_subject {
            Some(subject) => self.infer_pipe_body(expr_id, env, subject),
            None => self.infer_expr(expr_id, env, None),
        }
    }

    pub(crate) fn infer_truthy_falsy_pair(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let subject_plan = self.truthy_falsy_subject_plan(subject)?;
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
        );
        if !truthy.issues.is_empty() {
            return None;
        }
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
        );
        if !falsy.issues.is_empty() {
            return None;
        }
        let truthy_ty = truthy.actual()?;
        let falsy_ty = falsy.actual()?;
        self.apply_truthy_falsy_result_actual(subject, truthy_ty.unify(&falsy_ty)?)
            .to_gate_type()
    }

    fn infer_truthy_falsy_pair_info(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(subject_plan) = self.truthy_falsy_subject_plan(subject) else {
            return GateExprInfo::default();
        };
        let mut info = GateExprInfo::default();
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
        );
        let truthy_ty = truthy.actual();
        info.merge(truthy);
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
        );
        let falsy_ty = falsy.actual();
        info.merge(falsy);
        if info.issues.is_empty() {
            if let (Some(truthy_ty), Some(falsy_ty)) = (truthy_ty, falsy_ty) {
                if let Some(branch_ty) = truthy_ty.unify(&falsy_ty) {
                    info.set_actual(self.apply_truthy_falsy_result_actual(subject, branch_ty));
                }
            }
        }
        self.finalize_expr_info(info)
    }

    fn infer_function_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let plan = self.match_pipe_function_signature(expr_id, env, ambient, expected_result)?;
        let mut info = GateExprInfo::default();
        for ((argument, expected), reads_signal_payload) in plan
            .explicit_arguments
            .iter()
            .zip(
                plan.parameter_types
                    .iter()
                    .take(plan.explicit_arguments.len()),
            )
            .zip(plan.signal_payload_arguments.iter())
        {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_ty = argument_info
                .actual_gate_type()
                .or(argument_info.ty.clone());
            info.merge(argument_info);
            let matches_expected = argument_ty.as_ref().is_some_and(|actual| {
                actual.same_shape(expected)
                    || (*reads_signal_payload
                        && matches!(
                            actual,
                            GateType::Signal(payload) if payload.same_shape(expected)
                        ))
            }) || expression_matches(self.module, *argument, env, expected);
            if !matches_expected {
                return Some(info);
            }
        }
        if info.issues.is_empty() {
            info.ty = Some(plan.result_type);
        }
        Some(info)
    }

    pub(crate) fn infer_gate_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_gate_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_fanout_map_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanout_map_stage_info(expr_id, env, subject).ty
    }

    /// Infer only the result type for a joined fanout segment, without building
    /// filter plans or join plans.  Used by `validate_gate_pipe` to advance the
    /// subject type past a `*|> … <|*` segment without re-running the full
    /// `elaborate_fanout_segment` pass that `validate_fanout_semantics` already
    /// performed (PA-H2).
    pub(crate) fn infer_fanout_segment_result_type(
        &mut self,
        segment: &crate::PipeFanoutSegment<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let carrier = self.fanout_carrier(subject)?;
        let element_subject = subject.fanout_element().cloned()?;
        let mapped_element_type = self
            .infer_pipe_body(segment.map_expr(), env, &element_subject)
            .ty?;
        let mapped_collection_type = self.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Map, carrier),
            mapped_element_type,
        );
        if let Some(join_expr) = segment.join_expr() {
            let join_value_type = self
                .infer_pipe_body(join_expr, env, &mapped_collection_type)
                .ty?;
            Some(self.apply_fanout_plan(
                FanoutPlanner::plan(FanoutStageKind::Join, carrier),
                join_value_type,
            ))
        } else {
            Some(mapped_collection_type)
        }
    }

    pub(crate) fn infer_fanin_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanin_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_transform_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_transform_stage_info(expr_id, env, subject).ty
    }

    fn infer_transform_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        info.ty = info.ty.map(|body_ty| match subject {
            GateType::Signal(_) => GateType::Signal(Box::new(body_ty)),
            _ => body_ty,
        });
        self.finalize_expr_info(info)
    }

    fn infer_gate_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        let is_valid = info.issues.is_empty()
            && !info.contains_signal
            && !info.ty.as_ref().is_some_and(GateType::is_signal)
            && info.ty.as_ref().is_some_and(GateType::is_bool);
        info.ty = is_valid
            .then(|| self.apply_gate_plan(GatePlanner::plan(subject.gate_carrier()), subject));
        self.finalize_expr_info(info)
    }

    fn infer_fanout_map_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let Some(element) = subject.fanout_element() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, element);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Map, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    fn infer_fanin_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Join, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    fn infer_case_stage_run_info(
        &mut self,
        case_stages: &[&crate::hir::PipeStage],
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let mut branch_result = None::<SourceOptionActualType>;
        let branch_subject = subject.gate_payload().clone();
        for stage in case_stages {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                continue;
            };
            let mut branch_env = env.clone();
            branch_env
                .locals
                .extend(self.case_pattern_bindings(*pattern, &branch_subject).locals);
            let branch = self.infer_pipe_body(*body, &branch_env, &branch_subject);
            let branch_ty = branch.actual();
            info.merge(branch);
            let Some(branch_ty) = branch_ty else {
                branch_result = None;
                continue;
            };
            match branch_result.as_ref() {
                None => branch_result = Some(branch_ty),
                Some(current) => {
                    let Some(unified) = current.unify(&branch_ty) else {
                        info.issues.push(GateIssue::CaseBranchTypeMismatch {
                            span: stage.span,
                            expected: current.to_string(),
                            actual: branch_ty.to_string(),
                        });
                        branch_result = None;
                        break;
                    };
                    branch_result = Some(unified);
                }
            }
        }
        if info.issues.is_empty() {
            if let Some(branch_result) = branch_result {
                info.set_actual(match subject.gate_carrier() {
                    GateCarrier::Ordinary => branch_result,
                    GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(branch_result)),
                });
            }
        }
        self.finalize_expr_info(info)
    }

    fn infer_cluster_expr(&mut self, cluster_id: ClusterId, env: &GateExprEnv) -> GateExprInfo {
        let Some(cluster) = self.module.clusters().get(cluster_id).cloned() else {
            return GateExprInfo::default();
        };
        let spine = cluster.normalized_spine();
        let mut info = GateExprInfo::default();
        let mut cluster_kind = None::<ApplicativeClusterKind>;
        let mut payloads = Vec::new();

        for member in spine.apply_arguments() {
            let member_info = self.infer_expr(member, env, None);
            let member_ty = member_info.actual();
            info.merge(member_info);
            let Some(member_ty) = member_ty else {
                return self.finalize_expr_info(info);
            };
            let Some((member_kind, payload)) =
                ApplicativeClusterKind::from_member_actual(&member_ty)
            else {
                info.issues
                    .push(GateIssue::UnsupportedApplicativeClusterMember {
                        span: self.module.exprs()[member].span,
                        actual: member_ty.to_string(),
                    });
                return self.finalize_expr_info(info);
            };
            match cluster_kind.as_ref() {
                None => {
                    cluster_kind = Some(member_kind);
                    payloads.push(payload);
                }
                Some(expected) => {
                    let Some(unified) = expected.unify(&member_kind) else {
                        info.issues.push(GateIssue::ApplicativeClusterMismatch {
                            span: self.module.exprs()[member].span,
                            expected: expected.surface(),
                            actual: member_kind.surface(),
                        });
                        return self.finalize_expr_info(info);
                    };
                    cluster_kind = Some(unified);
                    payloads.push(payload);
                }
            }
        }

        let Some(cluster_kind) = cluster_kind else {
            return self.finalize_expr_info(info);
        };
        let payload_result = match spine.pure_head() {
            ApplicativeSpineHead::TupleConstructor(_) => SourceOptionActualType::Tuple(payloads),
            ApplicativeSpineHead::Expr(finalizer) => {
                let finalizer_info = self.infer_expr(finalizer, env, None);
                let finalizer_ty = finalizer_info.ty.clone();
                let finalizer_had_issues = !finalizer_info.issues.is_empty();
                info.merge(finalizer_info);

                let closed_payloads = payloads
                    .iter()
                    .map(SourceOptionActualType::to_gate_type)
                    .collect::<Option<Vec<_>>>();
                let applied_from_type = finalizer_ty
                    .as_ref()
                    .zip(closed_payloads.as_ref())
                    .and_then(|(ty, payloads)| self.apply_function_chain(ty, payloads));
                let applied_from_constructor = match &self.module.exprs()[finalizer].kind {
                    ExprKind::Name(reference) => {
                        let from_builtin = self
                            .infer_builtin_constructor_actual_from_reference(reference, &payloads);
                        let from_same_module =
                            self.infer_same_module_constructor_apply_actual(reference, &payloads);
                        from_builtin.or(from_same_module).or_else(|| {
                            closed_payloads.as_ref().and_then(|payloads| {
                                self.infer_same_module_constructor_apply(reference, payloads)
                                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                            })
                        })
                    }
                    _ => None,
                };

                match applied_from_type
                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                    .or(applied_from_constructor)
                {
                    Some(result) => result,
                    None => {
                        if !finalizer_had_issues {
                            info.issues.push(GateIssue::InvalidClusterFinalizer {
                                span: self.module.exprs()[finalizer].span,
                                expected_inputs: payloads.iter().map(ToString::to_string).collect(),
                                actual: finalizer_ty
                                    .map(|ty| ty.to_string())
                                    .unwrap_or_else(|| "unknown type".to_owned()),
                            });
                        }
                        return self.finalize_expr_info(info);
                    }
                }
            }
        };
        info.set_actual(cluster_kind.wrap_actual(payload_result));
        self.finalize_expr_info(info)
    }

    fn infer_pipe_expr(&mut self, pipe: &crate::hir::PipeExpr, env: &GateExprEnv) -> GateExprInfo {
        let stages = pipe.stages.iter().collect::<Vec<_>>();
        let mut info = self.infer_expr(pipe.head, env, None);
        let mut current = info.ty.clone();
        let mut stage_index = 0usize;
        while stage_index < stages.len() {
            let stage = stages[stage_index];
            let Some(subject) = current.clone() else {
                break;
            };
            let stage_info = match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    stage_index += 1;
                    self.infer_transform_stage_info(*expr, env, &subject)
                }
                PipeStageKind::Tap { expr } => {
                    stage_index += 1;
                    self.infer_tap_stage_info(*expr, env, &subject)
                }
                PipeStageKind::Gate { expr } => {
                    stage_index += 1;
                    self.infer_gate_stage_info(*expr, env, &subject)
                }
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        stage_index = segment.next_stage_index();
                        match crate::fanout_elaboration::elaborate_fanout_segment(
                            self.module,
                            &segment,
                            Some(&subject),
                            env,
                            self,
                        ) {
                            crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                let mut info = GateExprInfo::default();
                                info.ty = Some(plan.result_type);
                                info
                            }
                            crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => {
                                GateExprInfo::default()
                            }
                        }
                    } else {
                        stage_index += 1;
                        self.infer_fanout_map_stage_info(*expr, env, &subject)
                    }
                }
                PipeStageKind::FanIn { expr } => {
                    stage_index += 1;
                    self.infer_fanin_stage_info(*expr, env, &subject)
                }
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&stages, stage_index) else {
                        break;
                    };
                    stage_index = pair.next_index;
                    self.infer_truthy_falsy_pair_info(&pair, env, &subject)
                }
                PipeStageKind::Case { .. } => {
                    let case_start = stage_index;
                    while stage_index < stages.len()
                        && matches!(stages[stage_index].kind, PipeStageKind::Case { .. })
                    {
                        stage_index += 1;
                    }
                    self.infer_case_stage_run_info(&stages[case_start..stage_index], env, &subject)
                }
                PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => {
                    stage_index += 1;
                    GateExprInfo::default()
                }
            };
            current = stage_info.ty.clone();
            info.merge(stage_info);
        }
        info.ty = current;
        info
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

    fn apply_function_chain(&self, callee: &GateType, arguments: &[GateType]) -> Option<GateType> {
        let mut current = callee.clone();
        for argument in arguments {
            current = self.apply_function(&current, argument)?;
        }
        Some(current)
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

pub(crate) fn truthy_falsy_pair_stages<'a>(
    stages: &[&'a crate::hir::PipeStage],
    index: usize,
) -> Option<TruthyFalsyPairStages<'a>> {
    let first = *stages.get(index)?;
    let second = *stages.get(index + 1)?;
    match (&first.kind, &second.kind) {
        (
            PipeStageKind::Truthy { expr: truthy_expr },
            PipeStageKind::Falsy { expr: falsy_expr },
        ) => Some(TruthyFalsyPairStages {
            truthy_index: index,
            truthy_stage: first,
            truthy_expr: *truthy_expr,
            falsy_index: index + 1,
            falsy_stage: second,
            falsy_expr: *falsy_expr,
            next_index: index + 2,
        }),
        (
            PipeStageKind::Falsy { expr: falsy_expr },
            PipeStageKind::Truthy { expr: truthy_expr },
        ) => Some(TruthyFalsyPairStages {
            truthy_index: index + 1,
            truthy_stage: second,
            truthy_expr: *truthy_expr,
            falsy_index: index,
            falsy_stage: first,
            falsy_expr: *falsy_expr,
            next_index: index + 2,
        }),
        _ => None,
    }
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

fn case_constructor_key(reference: &TermReference) -> Option<CaseConstructorKey> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
            Some(CaseConstructorKey::Builtin(*builtin))
        }
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            Some(CaseConstructorKey::SameModuleVariant {
                item: *item_id,
                name: reference.path.segments().iter().last()?.text().to_owned(),
            })
        }
        ResolutionState::Unresolved
        | ResolutionState::Resolved(TermResolution::Local(_))
        | ResolutionState::Resolved(TermResolution::Import(_))
        | ResolutionState::Resolved(TermResolution::DomainMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
        | ResolutionState::Resolved(TermResolution::ClassMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
        | ResolutionState::Resolved(TermResolution::IntrinsicValue(_)) => None,
    }
}

fn missing_case_list(missing: &[CaseConstructorShape]) -> String {
    missing
        .iter()
        .map(|constructor| format!("`{}`", constructor.display))
        .collect::<Vec<_>>()
        .join(", ")
}

fn missing_case_label(missing: &[CaseConstructorShape]) -> String {
    let cases = missing_case_list(missing);
    if missing.len() == 1 {
        format!("add a case for {cases}, or use `_` to make the catch-all explicit")
    } else {
        format!("add cases for {cases}, or use `_` to make the catch-all explicit")
    }
}

fn custom_source_wakeup_kind(wakeup: CustomSourceRecurrenceWakeup) -> RecurrenceWakeupKind {
    match wakeup {
        CustomSourceRecurrenceWakeup::Timer => RecurrenceWakeupKind::Timer,
        CustomSourceRecurrenceWakeup::Backoff => RecurrenceWakeupKind::Backoff,
        CustomSourceRecurrenceWakeup::SourceEvent => RecurrenceWakeupKind::SourceEvent,
        CustomSourceRecurrenceWakeup::ProviderDefinedTrigger => {
            RecurrenceWakeupKind::ProviderDefinedTrigger
        }
    }
}

fn type_argument_phrase(count: usize) -> String {
    format!("{count} type argument{}", if count == 1 { "" } else { "s" })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceOptionTypeCheck {
    Match,
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceOptionGenericConstructorRootCheck {
    Match(SourceOptionActualType),
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceOptionTypeMismatch {
    span: SourceSpan,
    actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSourceOptionValue {
    field: crate::hir::RecordExprField,
    expected_surface: String,
    expected: SourceOptionExpectedType,
}

fn custom_source_contract_expected(
    module: &Module,
    annotation: TypeId,
    typing: &mut GateTypeContext<'_>,
) -> Option<(SourceOptionExpectedType, String)> {
    let expected = custom_source_contract_expected_type(module, annotation)?;
    let surface = typing.lower_annotation(annotation)?.to_string();
    Some((expected, surface))
}

fn custom_source_contract_expected_type(
    module: &Module,
    annotation: TypeId,
) -> Option<SourceOptionExpectedType> {
    SourceOptionExpectedType::from_hir_type(
        module,
        annotation,
        &HashMap::new(),
        SourceOptionTypeSurface::Contract,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceOptionExpectedType {
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionExpectedRecordField>),
    List(Box<Self>),
    Map { key: Box<Self>, value: Box<Self> },
    Set(Box<Self>),
    Signal(Box<Self>),
    Option(Box<Self>),
    Result { error: Box<Self>, value: Box<Self> },
    Validation { error: Box<Self>, value: Box<Self> },
    Named(SourceOptionNamedType),
    ContractParameter(SourceTypeParameter),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceOptionExpectedRecordField {
    name: String,
    ty: SourceOptionExpectedType,
}

/// Local proof type that keeps builtin container holes explicit until later
/// ordinary-expression or source-option evidence refines them into closed `GateType`s.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceOptionActualType {
    Hole,
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionActualRecordField>),
    Arrow {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    List(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
    },
    Set(Box<Self>),
    Option(Box<Self>),
    Result {
        error: Box<Self>,
        value: Box<Self>,
    },
    Validation {
        error: Box<Self>,
        value: Box<Self>,
    },
    Signal(Box<Self>),
    Task {
        error: Box<Self>,
        value: Box<Self>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceOptionActualRecordField {
    name: String,
    ty: SourceOptionActualType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceOptionTypeSurface {
    Contract,
    Expression,
}

impl SourceOptionExpectedType {
    fn from_resolved(module: &Module, ty: &ResolvedSourceContractType) -> Option<Self> {
        match ty {
            ResolvedSourceContractType::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            ) => Some(Self::Primitive(*builtin)),
            ResolvedSourceContractType::Builtin(_) => None,
            ResolvedSourceContractType::ContractParameter(parameter) => {
                Some(Self::ContractParameter(*parameter))
            }
            ResolvedSourceContractType::Item(item) => Some(Self::Named(
                SourceOptionNamedType::from_item(module, *item, Vec::new())?,
            )),
            ResolvedSourceContractType::Apply { callee, arguments } => match callee {
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::List) => Some(Self::List(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Map) => Some(Self::Map {
                    key: Box::new(Self::from_resolved(module, arguments.first()?)?),
                    value: Box::new(Self::from_resolved(module, arguments.get(1)?)?),
                }),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Set) => Some(Self::Set(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal) => Some(Self::Signal(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(_) => None,
                ResolvedSourceTypeConstructor::Item(item) => {
                    let arguments = arguments
                        .iter()
                        .map(|argument| Self::from_resolved(module, argument))
                        .collect::<Option<Vec<_>>>()?;
                    Some(Self::Named(SourceOptionNamedType::from_item(
                        module, *item, arguments,
                    )?))
                }
            },
        }
    }

    fn from_hir_type(
        module: &Module,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, SourceOptionExpectedType>,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match &module.types()[ty].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Builtin(
                    builtin @ (BuiltinType::Int
                    | BuiltinType::Float
                    | BuiltinType::Decimal
                    | BuiltinType::BigInt
                    | BuiltinType::Bool
                    | BuiltinType::Text
                    | BuiltinType::Unit
                    | BuiltinType::Bytes),
                )) => Some(Self::Primitive(*builtin)),
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    substitutions.get(parameter).cloned()
                }
                ResolutionState::Resolved(TypeResolution::Item(item)) => Some(Self::Named(
                    SourceOptionNamedType::from_item(module, *item, Vec::new())?,
                )),
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                        Some(Self::List(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Map {
                            key: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Set(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                        Some(Self::Signal(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Option(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Result {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Validation {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Item(item)) => {
                        let arguments = arguments
                            .iter()
                            .map(|argument| {
                                Self::from_hir_type(module, *argument, substitutions, surface)
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some(Self::Named(SourceOptionNamedType::from_item(
                            module, *item, arguments,
                        )?))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(_))
                    | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
                    | ResolutionState::Resolved(TypeResolution::Import(_))
                    | ResolutionState::Unresolved => None,
                }
            }
            TypeKind::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .copied()
                    .map(|element| Self::from_hir_type(module, element, substitutions, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.label.text().to_owned(),
                            ty: Self::from_hir_type(module, field.ty, substitutions, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Arrow { .. } => None,
        }
    }

    fn from_gate_type(
        module: &Module,
        ty: &GateType,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match ty {
            GateType::Primitive(builtin) => Some(Self::Primitive(*builtin)),
            GateType::TypeParameter { .. } => None,
            GateType::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .map(|element| Self::from_gate_type(module, element, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.name.clone(),
                            ty: Self::from_gate_type(module, &field.ty, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::List(element) => Some(Self::List(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Map { key, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Map {
                    key: Box::new(Self::from_gate_type(module, key, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Set(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Set(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Signal(element) => Some(Self::Signal(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Option(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Option(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Result { error, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Result {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Validation { error, value }
                if surface == SourceOptionTypeSurface::Expression =>
            {
                Some(Self::Validation {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Domain {
                item, arguments, ..
            }
            | GateType::OpaqueItem {
                item, arguments, ..
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| Self::from_gate_type(module, argument, surface))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Named(SourceOptionNamedType::from_item(
                    module, *item, arguments,
                )?))
            }
            GateType::Arrow { .. }
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Option(_)
            | GateType::Result { .. }
            | GateType::Validation { .. }
            | GateType::Task { .. } => None,
        }
    }

    fn is_signal_contract(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    fn matches_named_item(&self, item: ItemId) -> bool {
        matches!(self, Self::Named(named) if named.item == item)
    }

    fn as_named(&self) -> Option<&SourceOptionNamedType> {
        let Self::Named(named) = self else {
            return None;
        };
        Some(named)
    }
}

impl SourceOptionActualType {
    fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    fn from_gate_type(ty: &GateType) -> Self {
        match ty {
            GateType::Primitive(builtin) => Self::Primitive(*builtin),
            GateType::TypeParameter { .. } => Self::Hole,
            GateType::Tuple(elements) => {
                Self::Tuple(elements.iter().map(Self::from_gate_type).collect())
            }
            GateType::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|field| SourceOptionActualRecordField {
                        name: field.name.clone(),
                        ty: Self::from_gate_type(&field.ty),
                    })
                    .collect(),
            ),
            GateType::Arrow { parameter, result } => Self::Arrow {
                parameter: Box::new(Self::from_gate_type(parameter)),
                result: Box::new(Self::from_gate_type(result)),
            },
            GateType::List(element) => Self::List(Box::new(Self::from_gate_type(element))),
            GateType::Map { key, value } => Self::Map {
                key: Box::new(Self::from_gate_type(key)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Set(element) => Self::Set(Box::new(Self::from_gate_type(element))),
            GateType::Option(element) => Self::Option(Box::new(Self::from_gate_type(element))),
            GateType::Result { error, value } => Self::Result {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Validation { error, value } => Self::Validation {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Signal(element) => Self::Signal(Box::new(Self::from_gate_type(element))),
            GateType::Task { error, value } => Self::Task {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Domain {
                item,
                name,
                arguments,
            } => Self::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
            GateType::OpaqueItem {
                item,
                name,
                arguments,
            } => Self::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
        }
    }

    fn to_gate_type(&self) -> Option<GateType> {
        match self {
            Self::Hole => None,
            Self::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
            Self::Tuple(elements) => Some(GateType::Tuple(
                elements
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Record(fields) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(GateRecordField {
                            name: field.name.clone(),
                            ty: field.ty.to_gate_type()?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(parameter.to_gate_type()?),
                result: Box::new(result.to_gate_type()?),
            }),
            Self::List(element) => Some(GateType::List(Box::new(element.to_gate_type()?))),
            Self::Map { key, value } => Some(GateType::Map {
                key: Box::new(key.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Set(element) => Some(GateType::Set(Box::new(element.to_gate_type()?))),
            Self::Option(element) => Some(GateType::Option(Box::new(element.to_gate_type()?))),
            Self::Result { error, value } => Some(GateType::Result {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Validation { error, value } => Some(GateType::Validation {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Signal(element) => Some(GateType::Signal(Box::new(element.to_gate_type()?))),
            Self::Task { error, value } => Some(GateType::Task {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Domain {
                item,
                name,
                arguments,
            } => Some(GateType::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
            Self::OpaqueItem {
                item,
                name,
                arguments,
            } => Some(GateType::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
        }
    }

    fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Hole, actual) | (actual, Self::Hole) => Some(actual.clone()),
            (Self::Primitive(left), Self::Primitive(right)) if left == right => {
                Some(Self::Primitive(*left))
            }
            (Self::Tuple(left), Self::Tuple(right)) if left.len() == right.len() => {
                Some(Self::Tuple(
                    left.iter()
                        .zip(right)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                ))
            }
            (Self::Record(left), Self::Record(right)) if left.len() == right.len() => {
                let right_fields = right
                    .iter()
                    .map(|field| (field.name.as_str(), field))
                    .collect::<HashMap<_, _>>();
                let mut fields = Vec::with_capacity(left.len());
                for left in left {
                    let right = right_fields.get(left.name.as_str())?;
                    fields.push(SourceOptionActualRecordField {
                        name: left.name.clone(),
                        ty: left.ty.unify(&right.ty)?,
                    });
                }
                Some(Self::Record(fields))
            }
            (
                Self::Arrow {
                    parameter: left_parameter,
                    result: left_result,
                },
                Self::Arrow {
                    parameter: right_parameter,
                    result: right_result,
                },
            ) => Some(Self::Arrow {
                parameter: Box::new(left_parameter.unify(right_parameter)?),
                result: Box::new(left_result.unify(right_result)?),
            }),
            (Self::List(left), Self::List(right)) => Some(Self::List(Box::new(left.unify(right)?))),
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => Some(Self::Map {
                key: Box::new(left_key.unify(right_key)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Set(left), Self::Set(right)) => Some(Self::Set(Box::new(left.unify(right)?))),
            (Self::Option(left), Self::Option(right)) => {
                Some(Self::Option(Box::new(left.unify(right)?)))
            }
            (
                Self::Result {
                    error: left_error,
                    value: left_value,
                },
                Self::Result {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Result {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Validation {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Signal(left), Self::Signal(right)) => {
                Some(Self::Signal(Box::new(left.unify(right)?)))
            }
            (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Task {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Domain {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::Domain {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::Domain {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            (
                Self::OpaqueItem {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::OpaqueItem {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            _ => None,
        }
    }
}

impl fmt::Display for SourceOptionActualType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hole => write!(f, "_"),
            Self::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            Self::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            Self::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            Self::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            Self::List(element) => write!(f, "List {element}"),
            Self::Map { key, value } => write!(f, "Map {key} {value}"),
            Self::Set(element) => write!(f, "Set {element}"),
            Self::Option(element) => write!(f, "Option {element}"),
            Self::Result { error, value } => write!(f, "Result {error} {value}"),
            Self::Validation { error, value } => write!(f, "Validation {error} {value}"),
            Self::Signal(element) => write!(f, "Signal {element}"),
            Self::Task { error, value } => write!(f, "Task {error} {value}"),
            Self::Domain {
                name, arguments, ..
            }
            | Self::OpaqueItem {
                name, arguments, ..
            } => {
                if arguments.is_empty() {
                    write!(f, "{name}")
                } else {
                    write!(f, "{name}")?;
                    for argument in arguments {
                        write!(f, " {argument}")?;
                    }
                    Ok(())
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceOptionNamedType {
    item: ItemId,
    name: String,
    kind: SourceOptionNamedKind,
    arguments: Vec<SourceOptionExpectedType>,
}

impl SourceOptionNamedType {
    fn from_item(
        module: &Module,
        item: ItemId,
        arguments: Vec<SourceOptionExpectedType>,
    ) -> Option<Self> {
        let item_ref = &module.items()[item];
        let kind = match item_ref {
            Item::Domain(_) => SourceOptionNamedKind::Domain,
            Item::Type(_) => SourceOptionNamedKind::Type,
            Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::Class(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => return None,
        };
        Some(Self {
            item,
            name: item_type_name(item_ref),
            kind,
            arguments,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceOptionNamedKind {
    Domain,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceOptionConstructorActual {
    parent_item: ItemId,
    parent_name: String,
    constructor_name: String,
    field_types: Vec<TypeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SourceOptionTypeBindings {
    parameters: HashMap<SourceTypeParameter, SourceOptionActualType>,
}

impl SourceOptionTypeBindings {
    fn parameter(&self, parameter: SourceTypeParameter) -> Option<&SourceOptionActualType> {
        self.parameters.get(&parameter)
    }

    fn parameter_gate_type(&self, parameter: SourceTypeParameter) -> Option<GateType> {
        self.parameter(parameter)?.to_gate_type()
    }

    fn bind_or_match_actual(
        &mut self,
        parameter: SourceTypeParameter,
        actual: &SourceOptionActualType,
    ) -> bool {
        match self.parameters.entry(parameter) {
            Entry::Occupied(mut entry) => {
                let Some(unified) = entry.get().unify(actual) else {
                    return false;
                };
                entry.insert(unified);
                true
            }
            Entry::Vacant(entry) => {
                entry.insert(actual.clone());
                true
            }
        }
    }
}

fn source_option_contract_parameters(
    expected: &SourceOptionExpectedType,
) -> Vec<SourceTypeParameter> {
    fn collect(expected: &SourceOptionExpectedType, parameters: &mut Vec<SourceTypeParameter>) {
        match expected {
            SourceOptionExpectedType::Primitive(_) => {}
            SourceOptionExpectedType::Tuple(elements) => {
                for element in elements {
                    collect(element, parameters);
                }
            }
            SourceOptionExpectedType::Record(fields) => {
                for field in fields {
                    collect(&field.ty, parameters);
                }
            }
            SourceOptionExpectedType::List(element)
            | SourceOptionExpectedType::Set(element)
            | SourceOptionExpectedType::Signal(element)
            | SourceOptionExpectedType::Option(element) => collect(element, parameters),
            SourceOptionExpectedType::Map { key, value }
            | SourceOptionExpectedType::Result { error: key, value }
            | SourceOptionExpectedType::Validation { error: key, value } => {
                collect(key, parameters);
                collect(value, parameters);
            }
            SourceOptionExpectedType::Named(named) => {
                for argument in &named.arguments {
                    collect(argument, parameters);
                }
            }
            SourceOptionExpectedType::ContractParameter(parameter) => {
                if !parameters.contains(parameter) {
                    parameters.push(*parameter);
                }
            }
        }
    }

    let mut parameters = Vec::new();
    collect(expected, &mut parameters);
    parameters
}

fn source_option_unresolved_contract_parameters(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Vec<SourceTypeParameter> {
    source_option_contract_parameters(expected)
        .into_iter()
        .filter(|parameter| bindings.parameter_gate_type(*parameter).is_none())
        .collect()
}

fn source_option_contract_parameter_phrase(parameters: &[SourceTypeParameter]) -> String {
    let quoted = parameters
        .iter()
        .map(|parameter| format!("`{parameter}`"))
        .collect::<Vec<_>>();
    match quoted.as_slice() {
        [] => "contract parameters".to_owned(),
        [single] => format!("contract parameter {single}"),
        [left, right] => format!("contract parameters {left} and {right}"),
        _ => format!(
            "contract parameters {}, and {}",
            quoted[..quoted.len() - 1].join(", "),
            quoted
                .last()
                .expect("non-empty parameter list should keep a tail"),
        ),
    }
}

fn source_option_expected_to_gate_type(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Option<GateType> {
    match expected {
        SourceOptionExpectedType::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
        SourceOptionExpectedType::Tuple(elements) => Some(GateType::Tuple(
            elements
                .iter()
                .map(|element| source_option_expected_to_gate_type(element, bindings))
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::Record(fields) => Some(GateType::Record(
            fields
                .iter()
                .map(|field| {
                    Some(GateRecordField {
                        name: field.name.clone(),
                        ty: source_option_expected_to_gate_type(&field.ty, bindings)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::List(element) => Some(GateType::List(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Map { key, value } => Some(GateType::Map {
            key: Box::new(source_option_expected_to_gate_type(key, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Set(element) => Some(GateType::Set(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Signal(element) => Some(GateType::Signal(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Option(element) => Some(GateType::Option(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Result { error, value } => Some(GateType::Result {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Validation { error, value } => Some(GateType::Validation {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Named(named) => {
            let arguments = named
                .arguments
                .iter()
                .map(|argument| source_option_expected_to_gate_type(argument, bindings))
                .collect::<Option<Vec<_>>>()?;
            Some(match named.kind {
                SourceOptionNamedKind::Domain => GateType::Domain {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
                SourceOptionNamedKind::Type => GateType::OpaqueItem {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
            })
        }
        SourceOptionExpectedType::ContractParameter(parameter) => {
            bindings.parameter_gate_type(*parameter)
        }
    }
}

fn source_option_expected_matches_actual_type(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if !expected.is_signal_contract() {
        if let SourceOptionActualType::Signal(inner) = actual {
            return source_option_expected_matches_actual_type_inner(expected, inner, bindings);
        }
    }

    source_option_expected_matches_actual_type_inner(expected, actual, bindings)
}

fn source_option_expected_matches_actual_type_inner(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    match (expected, actual) {
        (SourceOptionExpectedType::ContractParameter(parameter), _) => {
            bindings.bind_or_match_actual(*parameter, actual)
        }
        (SourceOptionExpectedType::Primitive(_), SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Primitive(expected),
            SourceOptionActualType::Primitive(actual),
        ) => expected == actual,
        (SourceOptionExpectedType::Tuple(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Tuple(expected), SourceOptionActualType::Tuple(actual)) => {
            source_option_expected_args_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Record(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Record(expected), SourceOptionActualType::Record(actual)) => {
            source_option_expected_record_fields_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::List(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::List(expected), SourceOptionActualType::List(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Map { .. }, SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Map { key, value },
            SourceOptionActualType::Map {
                key: actual_key,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(key, actual_key, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Set(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Set(expected), SourceOptionActualType::Set(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Signal(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Signal(expected), SourceOptionActualType::Signal(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Option(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Option(expected), SourceOptionActualType::Option(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Result { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Result { error, value },
            SourceOptionActualType::Result {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Validation { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Validation { error, value },
            SourceOptionActualType::Validation {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Named(expected), SourceOptionActualType::Hole) => {
            let _ = expected;
            true
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::Domain {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Domain && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::OpaqueItem {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Type && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        _ => false,
    }
}

fn source_option_expected_args_match(
    expected: &[SourceOptionExpectedType],
    actual: &[SourceOptionActualType],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(expected, actual)| {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        })
}

fn source_option_expected_record_fields_match(
    expected: &[SourceOptionExpectedRecordField],
    actual: &[SourceOptionActualRecordField],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let actual_fields = actual
        .iter()
        .map(|field| (field.name.as_str(), &field.ty))
        .collect::<HashMap<_, _>>();
    expected.iter().all(|field| {
        actual_fields
            .get(field.name.as_str())
            .is_some_and(|actual| {
                source_option_expected_matches_actual_type(&field.ty, actual, bindings)
            })
    })
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
                    | ExprKind::Float(_)
                    | ExprKind::Decimal(_)
                    | ExprKind::BigInt(_)
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
                    ExprKind::Map(map) => {
                        for entry in map.entries.into_iter().rev() {
                            work.push(ExprWalkWork::Expr {
                                expr: entry.value,
                                is_root: false,
                            });
                            work.push(ExprWalkWork::Expr {
                                expr: entry.key,
                                is_root: false,
                            });
                        }
                    }
                    ExprKind::Set(elements) => {
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
    use aivi_base::{
        ByteIndex, DiagnosticCode, FileId, LabelStyle, SourceDatabase, SourceSpan, Span,
    };
    use aivi_syntax::parse_module;
    use aivi_typing::SourceTypeParameter;

    use crate::{
        ApplicativeCluster, Binding, BindingKind, BuiltinTerm, BuiltinType, ClusterFinalizer,
        ClusterPresentation, ControlNode, Expr, ExprKind, FunctionItem, FunctionParameter,
        ImportBinding, ImportBindingMetadata, IntegerLiteral, Item, ItemHeader, MarkupNode,
        MarkupNodeKind, Module, Name, NamePath, NonEmpty, Pattern, PatternKind, PipeExpr,
        PipeStage, PipeStageKind, RecordExpr, ShowControl, TermReference, TermResolution, TypeItem,
        TypeItemBody, TypeKind, TypeNode, TypeParameter, TypeReference, TypeResolution,
        TypeVariant, ValidationMode,
    };

    use super::*;

    fn span(file: u32, start: u32, end: u32) -> SourceSpan {
        SourceSpan::new(
            FileId::new(file),
            Span::new(ByteIndex::new(start), ByteIndex::new(end)),
        )
    }

    fn unit_span() -> SourceSpan {
        span(0, 0, 1)
    }

    fn validate_text(path: &str, text: &str) -> ValidationReport {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "test input should parse before HIR validation: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "test input should lower before HIR validation: {:?}",
            lowered.diagnostics()
        );
        validate_module(lowered.module(), ValidationMode::Structural)
    }

    fn validate_resolved_text(path: &str, text: &str) -> ValidationReport {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "test input should parse before resolved HIR validation: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "test input should lower before resolved HIR validation: {:?}",
            lowered.diagnostics()
        );
        validate_module(lowered.module(), ValidationMode::RequireResolvedNames)
    }

    #[test]
    fn gate_typing_infers_map_and_set_literals() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "map-set-literal-types.aivi",
            "val headers = Map { \"Authorization\": \"Bearer demo\", \"Accept\": \"application/json\" }\nval tags = Set [\"news\", \"featured\"]\n",
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "map/set typing input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "map/set typing input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let headers_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "headers" => Some(item.body),
                _ => None,
            })
            .expect("expected headers value");
        let tags_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "tags" => Some(item.body),
                _ => None,
            })
            .expect("expected tags value");

        let mut typing = GateTypeContext::new(module);
        assert_eq!(
            typing
                .infer_expr(headers_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Map {
                key: Box::new(GateType::Primitive(BuiltinType::Text)),
                value: Box::new(GateType::Primitive(BuiltinType::Text)),
            }),
        );
        assert_eq!(
            typing
                .infer_expr(tags_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Set(Box::new(GateType::Primitive(
                BuiltinType::Text,
            )))),
        );
    }

    #[test]
    fn gate_typing_infers_applicative_clusters() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "cluster-types.aivi",
            "type NamePair = NamePair Text Text\n\
             val first:(Option Text) = Some \"Ada\"\n\
             val last:(Option Text) = Some \"Lovelace\"\n\
             val pair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             val tupled =\n\
              &|> first\n\
              &|> last\n",
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "cluster typing input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "cluster typing input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let pair_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "pair" => Some(item.body),
                _ => None,
            })
            .expect("expected pair value");
        let tupled_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "tupled" => Some(item.body),
                _ => None,
            })
            .expect("expected tupled value");

        let mut typing = GateTypeContext::new(module);
        assert_eq!(
            typing
                .infer_expr(pair_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Option(Box::new(GateType::OpaqueItem {
                item: module
                    .root_items()
                    .iter()
                    .find_map(|item_id| match &module.items()[*item_id] {
                        Item::Type(item) if item.name.text() == "NamePair" => Some(*item_id),
                        _ => None,
                    })
                    .expect("expected NamePair type item"),
                name: "NamePair".to_owned(),
                arguments: Vec::new(),
            }))),
        );
        assert_eq!(
            typing
                .infer_expr(tupled_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Option(Box::new(GateType::Tuple(vec![
                GateType::Primitive(BuiltinType::Text),
                GateType::Primitive(BuiltinType::Text),
            ])))),
        );
    }

    #[test]
    fn gate_typing_tracks_partial_builtin_constructor_roots_and_applications() {
        let mut module = Module::new(FileId::new(0));
        let int_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("integer allocation should fit");
        let bool_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let some_expr = builtin_apply_expr(&mut module, BuiltinTerm::Some, "Some", vec![int_expr]);
        let ok_expr = builtin_apply_expr(&mut module, BuiltinTerm::Ok, "Ok", vec![int_expr]);
        let err_expr = builtin_apply_expr(&mut module, BuiltinTerm::Err, "Err", vec![bool_expr]);
        let valid_expr =
            builtin_apply_expr(&mut module, BuiltinTerm::Valid, "Valid", vec![int_expr]);
        let invalid_expr = builtin_apply_expr(
            &mut module,
            BuiltinTerm::Invalid,
            "Invalid",
            vec![bool_expr],
        );

        let mut typing = GateTypeContext::new(&module);

        let none_info = typing.infer_expr(none_expr, &GateExprEnv::default(), None);
        assert_eq!(
            none_info.actual,
            Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
        );
        assert_eq!(none_info.ty, None);

        let some_info = typing.infer_expr(some_expr, &GateExprEnv::default(), None);
        assert_eq!(
            some_info.actual,
            Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Primitive(BuiltinType::Int),
            ))),
        );
        assert_eq!(
            some_info.ty,
            Some(GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Int,
            )))),
        );

        let ok_info = typing.infer_expr(ok_expr, &GateExprEnv::default(), None);
        assert_eq!(
            ok_info.actual,
            Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(SourceOptionActualType::Primitive(BuiltinType::Int)),
            }),
        );
        assert_eq!(ok_info.ty, None);

        let err_info = typing.infer_expr(err_expr, &GateExprEnv::default(), None);
        assert_eq!(
            err_info.actual,
            Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Primitive(BuiltinType::Bool)),
                value: Box::new(SourceOptionActualType::Hole),
            }),
        );
        assert_eq!(err_info.ty, None);

        let valid_info = typing.infer_expr(valid_expr, &GateExprEnv::default(), None);
        assert_eq!(
            valid_info.actual,
            Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(SourceOptionActualType::Primitive(BuiltinType::Int)),
            }),
        );
        assert_eq!(valid_info.ty, None);

        let invalid_info = typing.infer_expr(invalid_expr, &GateExprEnv::default(), None);
        assert_eq!(
            invalid_info.actual,
            Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Primitive(BuiltinType::Bool)),
                value: Box::new(SourceOptionActualType::Hole),
            }),
        );
        assert_eq!(invalid_info.ty, None);
    }

    #[test]
    fn gate_typing_infers_partial_builtin_applicative_clusters() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "partial-builtin-clusters.aivi",
            "type NamePair = NamePair Text Text\n\
             val first = Some \"Ada\"\n\
             val last = None\n\
             val maybePair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             val okFirst = Ok \"Ada\"\n\
             val errLast = Err \"missing\"\n\
             val resultPair =\n\
              &|> okFirst\n\
              &|> errLast\n\
               |> NamePair\n",
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "partial builtin cluster input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "partial builtin cluster input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let maybe_pair_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "maybePair" => Some(item.body),
                _ => None,
            })
            .expect("expected maybePair value");
        let result_pair_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "resultPair" => Some(item.body),
                _ => None,
            })
            .expect("expected resultPair value");
        let name_pair_item = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Type(item) if item.name.text() == "NamePair" => Some(*item_id),
                _ => None,
            })
            .expect("expected NamePair type item");

        let mut typing = GateTypeContext::new(module);
        assert_eq!(
            typing
                .infer_expr(maybe_pair_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Option(Box::new(GateType::OpaqueItem {
                item: name_pair_item,
                name: "NamePair".to_owned(),
                arguments: Vec::new(),
            }))),
        );
        assert_eq!(
            typing
                .infer_expr(result_pair_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Result {
                error: Box::new(GateType::Primitive(BuiltinType::Text)),
                value: Box::new(GateType::OpaqueItem {
                    item: name_pair_item,
                    name: "NamePair".to_owned(),
                    arguments: Vec::new(),
                }),
            }),
        );
    }

    #[test]
    fn gate_typing_infers_pipe_case_split_result() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "case-pipe-types.aivi",
            r#"type Screen =
  | Loading
  | Ready Text
  | Failed Text
val current:Screen = Loading
val label =
    current
     ||> Loading => "loading"
     ||> Ready title => title
     ||> Failed reason => reason
"#,
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "case typing input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "case typing input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let label_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "label" => Some(item.body),
                _ => None,
            })
            .expect("expected label value");

        let mut typing = GateTypeContext::new(module);
        assert_eq!(
            typing
                .infer_expr(label_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Primitive(BuiltinType::Text)),
        );
    }

    #[test]
    fn gate_typing_infers_partial_builtin_case_runs() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "partial-builtin-cases.aivi",
            r#"type Screen =
  | Loading
  | Ready Text
  | Failed Text
val current:Screen = Loading
val maybeLabel =
    current
     ||> Loading => None
     ||> Ready title => Some title
     ||> Failed reason => Some reason
val resultLabel =
    current
     ||> Loading => Ok "loading"
     ||> Ready title => Ok title
     ||> Failed reason => Err reason
"#,
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "partial builtin case input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "partial builtin case input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let maybe_label_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "maybeLabel" => Some(item.body),
                _ => None,
            })
            .expect("expected maybeLabel value");
        let result_label_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "resultLabel" => Some(item.body),
                _ => None,
            })
            .expect("expected resultLabel value");

        let mut typing = GateTypeContext::new(module);
        assert_eq!(
            typing
                .infer_expr(maybe_label_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Text,
            )))),
        );
        assert_eq!(
            typing
                .infer_expr(result_label_expr, &GateExprEnv::default(), None)
                .ty,
            Some(GateType::Result {
                error: Box::new(GateType::Primitive(BuiltinType::Text)),
                value: Box::new(GateType::Primitive(BuiltinType::Text)),
            }),
        );
    }

    fn name(text: &str) -> Name {
        Name::new(text, unit_span()).expect("test name should stay valid")
    }

    fn builtin_name(builtin: BuiltinType) -> &'static str {
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

    fn builtin_type(module: &mut Module, builtin: BuiltinType) -> crate::TypeId {
        let path = NamePath::from_vec(vec![name(builtin_name(builtin))])
            .expect("builtin path should stay valid");
        module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    path,
                    TypeResolution::Builtin(builtin),
                )),
            })
            .expect("builtin type allocation should fit")
    }

    fn imported_type(module: &mut Module, text: &str, kind: Kind) -> crate::TypeId {
        let import_id = module
            .alloc_import(ImportBinding {
                span: unit_span(),
                imported_name: name(text),
                local_name: name(text),
                resolution: ImportBindingResolution::Resolved,
                metadata: ImportBindingMetadata::TypeConstructor { kind },
            })
            .expect("import allocation should fit");
        let path = NamePath::from_vec(vec![name(text)]).expect("single-segment path");
        module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    path,
                    TypeResolution::Import(import_id),
                )),
            })
            .expect("imported type allocation should fit")
    }

    fn type_parameter(module: &mut Module, text: &str) -> crate::TypeParameterId {
        module
            .alloc_type_parameter(TypeParameter {
                span: unit_span(),
                name: name(text),
            })
            .expect("type parameter allocation should fit")
    }

    fn push_sum_type(
        module: &mut Module,
        item_name: &str,
        parameters: Vec<crate::TypeParameterId>,
        variant_name: &str,
        fields: Vec<crate::TypeId>,
    ) -> crate::ItemId {
        module
            .push_item(Item::Type(TypeItem {
                header: ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: name(item_name),
                parameters,
                body: TypeItemBody::Sum(NonEmpty::new(
                    TypeVariant {
                        span: unit_span(),
                        name: name(variant_name),
                        fields,
                    },
                    Vec::new(),
                )),
            }))
            .expect("type item allocation should fit")
    }

    fn constructor_expr(
        module: &mut Module,
        parent_item: crate::ItemId,
        variant_name: &str,
        arguments: Vec<crate::ExprId>,
    ) -> crate::ExprId {
        let path = NamePath::from_vec(vec![name(variant_name)])
            .expect("constructor path should stay valid");
        let callee = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Name(TermReference::resolved(
                    path,
                    TermResolution::Item(parent_item),
                )),
            })
            .expect("constructor callee allocation should fit");
        match arguments.split_first() {
            None => callee,
            Some((first, rest)) => module
                .alloc_expr(Expr {
                    span: unit_span(),
                    kind: ExprKind::Apply {
                        callee,
                        arguments: NonEmpty::new(*first, rest.to_vec()),
                    },
                })
                .expect("constructor application allocation should fit"),
        }
    }

    fn builtin_expr(module: &mut Module, builtin: BuiltinTerm, text: &str) -> crate::ExprId {
        let path = NamePath::from_vec(vec![name(text)]).expect("builtin path should stay valid");
        module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Name(TermReference::resolved(
                    path,
                    TermResolution::Builtin(builtin),
                )),
            })
            .expect("builtin expression allocation should fit")
    }

    fn builtin_apply_expr(
        module: &mut Module,
        builtin: BuiltinTerm,
        text: &str,
        arguments: Vec<crate::ExprId>,
    ) -> crate::ExprId {
        let callee = builtin_expr(module, builtin, text);
        match arguments.split_first() {
            None => callee,
            Some((first, rest)) => module
                .alloc_expr(Expr {
                    span: unit_span(),
                    kind: ExprKind::Apply {
                        callee,
                        arguments: NonEmpty::new(*first, rest.to_vec()),
                    },
                })
                .expect("builtin constructor application should fit"),
        }
    }

    fn item_expr(module: &mut Module, item: crate::ItemId, text: &str) -> crate::ExprId {
        let path = NamePath::from_vec(vec![name(text)]).expect("item path should stay valid");
        module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Name(TermReference::resolved(path, TermResolution::Item(item))),
            })
            .expect("item expression allocation should fit")
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
    fn imported_type_constructor_metadata_participates_in_kind_validation() {
        let mut module = Module::new(FileId::new(0));
        let request = imported_type(&mut module, "Request", Kind::constructor(1));
        let text = builtin_type(&mut module, BuiltinType::Text);
        let broken_alias = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Apply {
                    callee: request,
                    arguments: NonEmpty::new(text, vec![text]),
                },
            })
            .expect("type application allocation should fit");
        let _ = module
            .push_item(Item::Type(TypeItem {
                header: ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: name("Broken"),
                parameters: Vec::new(),
                body: TypeItemBody::Alias(broken_alias),
            }))
            .expect("type item allocation should fit");

        let report = validate_module(&module, ValidationMode::RequireResolvedNames);
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-type-application"))
            }),
            "expected imported constructor kind metadata to trigger over-application diagnostics, got {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn regex_literal_validation_reports_hir_diagnostics() {
        let report = validate_text(
            "regex_invalid_quantifier.aivi",
            "val brokenPattern = rx\"a{2,1}\"\n",
        );
        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-regex-literal"))
            })
            .expect("invalid regex literal should produce a HIR diagnostic");

        assert_eq!(
            diagnostic.message,
            "regex literal is not valid under the current compile-time regex grammar"
        );
        assert!(
            diagnostic
                .labels
                .iter()
                .any(|label| label.style == LabelStyle::Primary && !label.message.is_empty()),
            "expected regex validation to keep the parser-provided primary error span",
        );
    }

    #[test]
    fn case_exhaustiveness_reports_missing_same_module_sum_constructors() {
        let report = validate_resolved_text(
            "pattern_non_exhaustive_sum.aivi",
            r#"type Status =
  | Paid
  | Pending
  | Failed Text

fun statusLabel:Text #status:Status =>
    status
     ||> Paid => "paid"
"#,
        );
        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "non-exhaustive-case-pattern"))
            })
            .expect("non-exhaustive sum cases should produce a HIR diagnostic");

        assert_eq!(
            diagnostic.message,
            "case split over `Status` is not exhaustive; missing `Pending`, `Failed`"
        );
        assert!(
            diagnostic.labels.iter().any(|label| {
                label.style == LabelStyle::Primary
                    && label.message.contains("add cases for `Pending`, `Failed`")
            }),
            "expected a primary label listing the missing constructors, got {:?}",
            diagnostic.labels
        );
    }

    #[test]
    fn case_exhaustiveness_accepts_builtin_case_pairs() {
        let report = validate_resolved_text(
            "builtin_exhaustive_cases.aivi",
            r#"fun boolLabel:Text #ready:Bool =>
    ready
     ||> True => "ready"
     ||> False => "waiting"

fun maybeLabel:Text #maybeUser:(Option Text) =>
    maybeUser
     ||> Some name => name
     ||> None => "login"

fun resultLabel:Text #status:(Result Text Text) =>
    status
     ||> Ok body => body
     ||> Err message => message

fun validationLabel:Text #status:(Validation Text Text) =>
    status
     ||> Valid body => body
     ||> Invalid message => message
"#,
        );

        assert!(
            report.is_ok(),
            "expected builtin case pairs to validate cleanly, got {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn match_control_exhaustiveness_uses_with_binding_types() {
        let report = validate_resolved_text(
            "non_exhaustive_match_control.aivi",
            r#"type Screen =
  | Loading
  | Ready Text
  | Failed Text

val current:Screen =
    Loading

val screenView =
    <with value={current} as={screen}>
        <match on={screen}>
            <case pattern={Loading}>
                <Label text="Loading..." />
            </case>
            <case pattern={Ready title}>
                <Label text={title} />
            </case>
        </match>
    </with>
"#,
        );
        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "non-exhaustive-case-pattern"))
            })
            .expect("non-exhaustive markup match should produce a HIR diagnostic");

        assert_eq!(
            diagnostic.message,
            "match control over `Screen` is not exhaustive; missing `Failed`"
        );
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

    #[test]
    fn source_option_expected_types_preserve_contract_parameter_holes() {
        let module = Module::new(FileId::new(0));

        assert_eq!(
            SourceOptionExpectedType::from_resolved(
                &module,
                &ResolvedSourceContractType::ContractParameter(SourceTypeParameter::A),
            ),
            Some(SourceOptionExpectedType::ContractParameter(
                SourceTypeParameter::A,
            ))
        );
        assert_eq!(
            SourceOptionExpectedType::from_resolved(
                &module,
                &ResolvedSourceContractType::Apply {
                    callee: ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal),
                    arguments: vec![ResolvedSourceContractType::ContractParameter(
                        SourceTypeParameter::B,
                    )],
                },
            ),
            Some(SourceOptionExpectedType::Signal(Box::new(
                SourceOptionExpectedType::ContractParameter(SourceTypeParameter::B),
            )))
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_inferable_expression_types() {
        let mut module = Module::new(FileId::new(0));
        let expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Primitive(BuiltinType::Int)),
        );
    }

    #[test]
    fn source_option_concrete_expected_types_accept_function_applications_with_builtin_holes() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "source-option-concrete-application.aivi",
            "fun keep:Option Int #value:Option Int => value\n\
             val chosen = keep None\n",
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "test input should parse before source option checking: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "test input should lower before source option checking: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let chosen_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "chosen" => Some(item.body),
                _ => None,
            })
            .expect("expected chosen value");
        let validator = Validator {
            module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                chosen_expr,
                &SourceOptionExpectedType::Option(Box::new(SourceOptionExpectedType::Primitive(
                    BuiltinType::Int,
                ))),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
    }

    #[test]
    fn source_option_concrete_expected_types_accept_tuple_literals() {
        let mut module = Module::new(FileId::new(0));
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let bool_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let tuple_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Tuple(crate::AtLeastTwo::new(value_expr, bool_expr, Vec::new())),
            })
            .expect("tuple expression allocation should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                tuple_expr,
                &SourceOptionExpectedType::Tuple(vec![
                    SourceOptionExpectedType::Primitive(BuiltinType::Int),
                    SourceOptionExpectedType::Primitive(BuiltinType::Bool),
                ]),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
    }

    #[test]
    fn source_option_concrete_expected_types_accept_record_literals() {
        let mut module = Module::new(FileId::new(0));
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let bool_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let record_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("value"),
                            value: value_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("enabled"),
                            value: bool_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                    ],
                }),
            })
            .expect("record expression allocation should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                record_expr,
                &SourceOptionExpectedType::Record(vec![
                    SourceOptionExpectedRecordField {
                        name: "value".to_owned(),
                        ty: SourceOptionExpectedType::Primitive(BuiltinType::Int),
                    },
                    SourceOptionExpectedRecordField {
                        name: "enabled".to_owned(),
                        ty: SourceOptionExpectedType::Primitive(BuiltinType::Bool),
                    },
                ]),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
    }

    #[test]
    fn source_option_concrete_expected_types_accept_empty_map_literals() {
        let mut module = Module::new(FileId::new(0));
        let map_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Map(crate::MapExpr {
                    entries: Vec::new(),
                }),
            })
            .expect("map expression allocation should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                map_expr,
                &SourceOptionExpectedType::Map {
                    key: Box::new(SourceOptionExpectedType::Primitive(BuiltinType::Text)),
                    value: Box::new(SourceOptionExpectedType::Primitive(BuiltinType::Int)),
                },
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
    }

    #[test]
    fn source_option_projection_expressions_remain_unproven() {
        let mut module = Module::new(FileId::new(0));
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let record_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![crate::RecordExprField {
                        span: unit_span(),
                        label: name("value"),
                        value: value_expr,
                        surface: crate::RecordFieldSurface::Explicit,
                    }],
                }),
            })
            .expect("record expression allocation should fit");
        let projection_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Projection {
                    base: crate::ProjectionBase::Expr(record_expr),
                    path: NamePath::from_vec(vec![name("value")])
                        .expect("projection path should stay valid"),
                },
            })
            .expect("projection expression allocation should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                projection_expr,
                &SourceOptionExpectedType::Primitive(BuiltinType::Int),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Unknown,
        );
    }

    #[test]
    fn resolved_validation_accepts_custom_source_tuple_and_record_options() {
        let report = validate_resolved_text(
            "source-option-tuple-record-options.aivi",
            "provider custom.feed\n\
             \x20\x20\x20\x20option pair: (Int, Bool)\n\
             \x20\x20\x20\x20option config: { value: Int, enabled: Bool }\n\
             \x20\x20\x20\x20wakeup: providerTrigger\n\
             \n\
             @source custom.feed with {\n\
             \x20\x20\x20\x20pair: (1, True),\n\
             \x20\x20\x20\x20config: { value: 1, enabled: True }\n\
             }\n\
             sig updates : Signal Int\n",
        );

        assert!(
            report.is_ok(),
            "unexpected diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn resolved_validation_accepts_custom_source_parameterized_domain_literal_options() {
        let report = validate_resolved_text(
            "source-option-parameterized-domain-literal-options.aivi",
            "domain Tagged A B over Int\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             \n\
             provider custom.feed\n\
             \x20\x20\x20\x20option tag: Tagged Int Bool\n\
             \x20\x20\x20\x20wakeup: providerTrigger\n\
             \n\
             @source custom.feed with {\n\
             \x20\x20\x20\x20tag: 1tg\n\
             }\n\
             sig updates : Signal Int\n",
        );

        assert!(
            report.is_ok(),
            "unexpected diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn resolved_validation_rejects_custom_source_domain_literal_constraint_mismatches() {
        let report = validate_resolved_text(
            "source-option-domain-literal-constraint-mismatch.aivi",
            "domain Tagged A B over Int\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             \n\
             provider custom.feed\n\
             \x20\x20\x20\x20option tag: Tagged Text Bool\n\
             \x20\x20\x20\x20wakeup: providerTrigger\n\
             \n\
             @source custom.feed with {\n\
             \x20\x20\x20\x20tag: 1tg\n\
             }\n\
             sig updates : Signal Int\n",
        );

        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "source-option-type-mismatch"))
            })
            .expect("expected source option mismatch diagnostic");
        assert_eq!(
            diagnostic.message,
            "source option `tag` for `custom.feed` expects `Tagged Text Bool`, but this expression proves `Tagged Int _`"
        );
    }

    #[test]
    fn resolved_validation_rejects_unbound_source_option_contract_parameters() {
        let report = validate_resolved_text(
            "source-option-unbound-contract-parameter.aivi",
            r#"type HttpError =
  | Timeout

type Session = {
    token: Text
}

type Box A =
  | Box A

val emptyBody =
    Box None

@source http.post "/login" with {
    body: emptyBody
}
sig login : Signal (Result HttpError Session)
"#,
        );

        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code
                    == Some(DiagnosticCode::new(
                        "hir",
                        "source-option-unbound-contract-parameter",
                    ))
            })
            .expect("expected unbound source option contract parameter diagnostic");
        assert_eq!(
            diagnostic.message,
            "source option `body` for `http.post` expects `A`, but local source-option checking leaves contract parameter `A` unbound"
        );
        assert!(
            diagnostic
                .labels
                .iter()
                .any(|label| label.message.contains("A = Box Option _")),
            "expected the diagnostic to report the partial fixed-point proof, got {:?}",
            diagnostic.labels
        );
    }

    #[test]
    fn builtin_source_option_validation_refines_contract_parameters_across_multiple_values() {
        let mut module = Module::new(FileId::new(0));
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let some_expr =
            builtin_apply_expr(&mut module, BuiltinTerm::Some, "Some", vec![value_expr]);
        let options = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("body"),
                            value: none_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("body"),
                            value: some_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                    ],
                }),
            })
            .expect("record expression allocation should fit");
        let source = SourceDecorator {
            provider: None,
            arguments: Vec::new(),
            options: Some(options),
        };
        let mut validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut resolver = SourceContractTypeResolver::new(&module);
        let mut typing = GateTypeContext::new(&module);

        validator.validate_builtin_source_decorator_contract_types(
            &source,
            BuiltinSourceProvider::HttpPost,
            &mut resolver,
            &mut typing,
        );

        assert!(
            validator.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            validator.diagnostics
        );
    }

    #[test]
    fn builtin_source_option_validation_reports_conflicting_partial_bindings() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let box_item = push_sum_type(&mut module, "Box", vec![payload], "Box", vec![payload_ref]);
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let first_body = constructor_expr(&mut module, box_item, "Box", vec![none_expr]);
        let true_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let second_body = constructor_expr(&mut module, box_item, "Box", vec![true_expr]);
        let options = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("body"),
                            value: first_body,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("body"),
                            value: second_body,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                    ],
                }),
            })
            .expect("record expression allocation should fit");
        let source = SourceDecorator {
            provider: None,
            arguments: Vec::new(),
            options: Some(options),
        };
        let mut validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut resolver = SourceContractTypeResolver::new(&module);
        let mut typing = GateTypeContext::new(&module);

        validator.validate_builtin_source_decorator_contract_types(
            &source,
            BuiltinSourceProvider::HttpPost,
            &mut resolver,
            &mut typing,
        );

        let diagnostic = validator
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "source-option-type-mismatch"))
            })
            .expect("expected conflicting source option binding mismatch");
        assert_eq!(
            diagnostic.message,
            "source option `body` for `http.post` expects `A`, but this expression proves `Box Bool`"
        );
    }

    #[test]
    fn source_option_root_contract_parameters_reuse_existing_bindings() {
        let mut module = Module::new(FileId::new(0));
        let int_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let true_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                int_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert!(matches!(
            validator.check_source_option_expr(
                true_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Mismatch(_),
        ));
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Primitive(BuiltinType::Int)),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_monomorphic_constructor_roots() {
        let mut module = Module::new(FileId::new(0));
        let mode = push_sum_type(&mut module, "Mode", Vec::new(), "On", Vec::new());
        let expr = constructor_expr(&mut module, mode, "On", Vec::new());
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: mode,
                name: "Mode".to_owned(),
                arguments: Vec::new(),
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_reuse_bindings_for_generic_constructors() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let box_item = push_sum_type(&mut module, "Box", vec![payload], "Box", vec![payload_ref]);
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let first_expr = constructor_expr(&mut module, box_item, "Box", vec![element]);
        let second_element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "2".into() }),
            })
            .expect("expression allocation should fit");
        let second_expr = constructor_expr(&mut module, box_item, "Box", vec![second_element]);
        let bool_element = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let mismatched_expr = constructor_expr(&mut module, box_item, "Box", vec![bool_element]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                first_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: box_item,
                name: "Box".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
        assert_eq!(
            validator.check_source_option_expr(
                second_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert!(matches!(
            validator.check_source_option_expr(
                mismatched_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Mismatch(_),
        ));
    }

    #[test]
    fn source_option_root_contract_parameters_bind_generic_constructor_roots_from_concrete_fields()
    {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let mode_item = push_sum_type(&mut module, "Mode", Vec::new(), "On", Vec::new());
        let mode_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("Mode")]).expect("type path should stay valid"),
                    TypeResolution::Item(mode_item),
                )),
            })
            .expect("mode type allocation should fit");
        let box_item = push_sum_type(
            &mut module,
            "Box",
            vec![payload],
            "Box",
            vec![mode_ref, payload_ref],
        );
        let mode_expr = constructor_expr(&mut module, mode_item, "On", Vec::new());
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let expr = constructor_expr(&mut module, box_item, "Box", vec![mode_expr, element]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: box_item,
                name: "Box".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_nested_generic_constructor_arguments() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let inner_item = push_sum_type(
            &mut module,
            "Inner",
            vec![payload],
            "Inner",
            vec![payload_ref],
        );
        let outer_item = push_sum_type(
            &mut module,
            "Outer",
            vec![payload],
            "Outer",
            vec![payload_ref],
        );
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let inner_expr = constructor_expr(&mut module, inner_item, "Inner", vec![element]);
        let outer_expr = constructor_expr(&mut module, outer_item, "Outer", vec![inner_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                outer_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: outer_item,
                name: "Outer".to_owned(),
                arguments: vec![GateType::OpaqueItem {
                    item: inner_item,
                    name: "Inner".to_owned(),
                    arguments: vec![GateType::Primitive(BuiltinType::Int)],
                }],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_unannotated_local_value_bodies() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let box_item = push_sum_type(&mut module, "Box", vec![payload], "Box", vec![payload_ref]);
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let boxed_expr = constructor_expr(&mut module, box_item, "Box", vec![element]);
        let boxed_item = module
            .push_item(Item::Value(crate::ValueItem {
                header: ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: name("boxed"),
                annotation: None,
                body: boxed_expr,
            }))
            .expect("value item allocation should fit");
        let expr = item_expr(&mut module, boxed_item, "boxed");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: box_item,
                name: "Box".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_builtin_some_constructor_roots() {
        let mut module = Module::new(FileId::new(0));
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let some_callee = builtin_expr(&mut module, BuiltinTerm::Some, "Some");
        let some_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Apply {
                    callee: some_callee,
                    arguments: NonEmpty::new(element, Vec::new()),
                },
            })
            .expect("builtin constructor application should fit");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                some_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Int,
            )))),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_context_free_builtin_none_roots() {
        let mut module = Module::new(FileId::new(0));
        let expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter(SourceTypeParameter::A),
            Some(&SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_refine_context_free_builtin_none_roots() {
        let mut module = Module::new(FileId::new(0));
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let some_expr = builtin_apply_expr(&mut module, BuiltinTerm::Some, "Some", vec![element]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                none_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            validator.check_source_option_expr(
                some_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Option(Box::new(GateType::Primitive(
                BuiltinType::Int,
            )))),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_refine_context_free_builtin_result_roots() {
        let mut module = Module::new(FileId::new(0));
        let ok_value = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let ok_expr = builtin_apply_expr(&mut module, BuiltinTerm::Ok, "Ok", vec![ok_value]);
        let err_value = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let err_expr = builtin_apply_expr(&mut module, BuiltinTerm::Err, "Err", vec![err_value]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                ok_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter(SourceTypeParameter::A),
            Some(&SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(SourceOptionActualType::Primitive(BuiltinType::Int)),
            }),
        );
        assert_eq!(
            validator.check_source_option_expr(
                err_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Result {
                error: Box::new(GateType::Primitive(BuiltinType::Bool)),
                value: Box::new(GateType::Primitive(BuiltinType::Int)),
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_refine_context_free_builtin_validation_roots() {
        let mut module = Module::new(FileId::new(0));
        let valid_value = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let valid_expr =
            builtin_apply_expr(&mut module, BuiltinTerm::Valid, "Valid", vec![valid_value]);
        let invalid_value = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let invalid_expr = builtin_apply_expr(
            &mut module,
            BuiltinTerm::Invalid,
            "Invalid",
            vec![invalid_value],
        );
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                valid_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            validator.check_source_option_expr(
                invalid_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Validation {
                error: Box::new(GateType::Primitive(BuiltinType::Bool)),
                value: Box::new(GateType::Primitive(BuiltinType::Int)),
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_generic_constructor_roots_with_builtin_holes() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let box_item = push_sum_type(&mut module, "Box", vec![payload], "Box", vec![payload_ref]);
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let first_expr = constructor_expr(&mut module, box_item, "Box", vec![none_expr]);
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let some_expr = builtin_apply_expr(&mut module, BuiltinTerm::Some, "Some", vec![element]);
        let second_expr = constructor_expr(&mut module, box_item, "Box", vec![some_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                first_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter(SourceTypeParameter::A),
            Some(&SourceOptionActualType::OpaqueItem {
                item: box_item,
                name: "Box".to_owned(),
                arguments: vec![SourceOptionActualType::Option(Box::new(
                    SourceOptionActualType::Hole,
                ))],
            }),
        );
        assert_eq!(
            validator.check_source_option_expr(
                second_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: box_item,
                name: "Box".to_owned(),
                arguments: vec![GateType::Option(Box::new(GateType::Primitive(
                    BuiltinType::Int,
                )))],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_fixed_point_domain_literal_fields() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "source-option-domain-literal-constructor-root.aivi",
            "domain Tagged A B over Int\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             \n\
             type Wrap B =\n\
             \x20\x20| Wrap (Tagged Int B) B\n\
             \n\
             val chosen =\n\
             \x20\x20\x20\x20Wrap 1tg True\n",
        );
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "test input should parse before source option checking: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "test input should lower before source option checking: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        let chosen_expr = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Value(item) if item.name.text() == "chosen" => Some(item.body),
                _ => None,
            })
            .expect("expected chosen value");
        let wrap_item = module
            .root_items()
            .iter()
            .find_map(|item_id| match &module.items()[*item_id] {
                Item::Type(item) if item.name.text() == "Wrap" => Some(*item_id),
                _ => None,
            })
            .expect("expected Wrap type");
        let validator = Validator {
            module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                chosen_expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: wrap_item,
                name: "Wrap".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Bool)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_preserve_generic_constructor_holes_for_unproven_arguments()
     {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let int_ref = builtin_type(&mut module, BuiltinType::Int);
        let phantom_item = push_sum_type(
            &mut module,
            "Phantom",
            vec![payload],
            "Phantom",
            vec![int_ref],
        );
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let expr = constructor_expr(&mut module, phantom_item, "Phantom", vec![value_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter(SourceTypeParameter::A),
            Some(&SourceOptionActualType::OpaqueItem {
                item: phantom_item,
                name: "Phantom".to_owned(),
                arguments: vec![SourceOptionActualType::Hole],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_fixed_point_builtin_none_fields() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let option_callee = builtin_type(&mut module, BuiltinType::Option);
        let option_payload = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Apply {
                    callee: option_callee,
                    arguments: NonEmpty::new(payload_ref, Vec::new()),
                },
            })
            .expect("option type allocation should fit");
        let pair_item = push_sum_type(
            &mut module,
            "Pair",
            vec![payload],
            "Pair",
            vec![payload_ref, option_payload],
        );
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let none_expr = builtin_expr(&mut module, BuiltinTerm::None, "None");
        let expr = constructor_expr(&mut module, pair_item, "Pair", vec![element, none_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: pair_item,
                name: "Pair".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_fixed_point_builtin_result_fields() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let text_ref = builtin_type(&mut module, BuiltinType::Text);
        let result_callee = builtin_type(&mut module, BuiltinType::Result);
        let result_payload = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Apply {
                    callee: result_callee,
                    arguments: NonEmpty::new(text_ref, vec![payload_ref]),
                },
            })
            .expect("result type allocation should fit");
        let outcome_item = push_sum_type(
            &mut module,
            "OutcomeBox",
            vec![payload],
            "OutcomeBox",
            vec![payload_ref, result_payload],
        );
        let element = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let ok_value = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "2".into() }),
            })
            .expect("expression allocation should fit");
        let ok_callee = builtin_expr(&mut module, BuiltinTerm::Ok, "Ok");
        let ok_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Apply {
                    callee: ok_callee,
                    arguments: NonEmpty::new(ok_value, Vec::new()),
                },
            })
            .expect("builtin constructor application should fit");
        let expr = constructor_expr(
            &mut module,
            outcome_item,
            "OutcomeBox",
            vec![element, ok_expr],
        );
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: outcome_item,
                name: "OutcomeBox".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_tuple_constructor_fields() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let bool_ref = builtin_type(&mut module, BuiltinType::Bool);
        let tuple_field = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Tuple(crate::AtLeastTwo::new(payload_ref, bool_ref, Vec::new())),
            })
            .expect("tuple type allocation should fit");
        let pair_box = push_sum_type(
            &mut module,
            "PairBox",
            vec![payload],
            "PairBox",
            vec![tuple_field],
        );
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let bool_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let tuple_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Tuple(crate::AtLeastTwo::new(value_expr, bool_expr, Vec::new())),
            })
            .expect("tuple expression allocation should fit");
        let expr = constructor_expr(&mut module, pair_box, "PairBox", vec![tuple_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: pair_box,
                name: "PairBox".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_record_constructor_fields() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let bool_ref = builtin_type(&mut module, BuiltinType::Bool);
        let record_field = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Record(vec![
                    crate::TypeField {
                        span: unit_span(),
                        label: name("value"),
                        ty: payload_ref,
                    },
                    crate::TypeField {
                        span: unit_span(),
                        label: name("enabled"),
                        ty: bool_ref,
                    },
                ]),
            })
            .expect("record type allocation should fit");
        let config_box = push_sum_type(
            &mut module,
            "ConfigBox",
            vec![payload],
            "ConfigBox",
            vec![record_field],
        );
        let value_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Integer(IntegerLiteral { raw: "1".into() }),
            })
            .expect("expression allocation should fit");
        let bool_expr = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let record_expr = module
            .alloc_expr(Expr {
                span: unit_span(),
                kind: ExprKind::Record(RecordExpr {
                    fields: vec![
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("value"),
                            value: value_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                        crate::RecordExprField {
                            span: unit_span(),
                            label: name("enabled"),
                            value: bool_expr,
                            surface: crate::RecordFieldSurface::Explicit,
                        },
                    ],
                }),
            })
            .expect("record expression allocation should fit");
        let expr = constructor_expr(&mut module, config_box, "ConfigBox", vec![record_expr]);
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: config_box,
                name: "ConfigBox".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_root_contract_parameters_bind_arrow_constructor_fields() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let int_ref = builtin_type(&mut module, BuiltinType::Int);
        let bool_ref = builtin_type(&mut module, BuiltinType::Bool);
        let arrow_field = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Arrow {
                    parameter: payload_ref,
                    result: bool_ref,
                },
            })
            .expect("arrow type allocation should fit");
        let function_box = push_sum_type(
            &mut module,
            "FunctionBox",
            vec![payload],
            "FunctionBox",
            vec![arrow_field],
        );
        let parameter_binding = module
            .alloc_binding(Binding {
                span: unit_span(),
                name: name("value"),
                kind: BindingKind::FunctionParameter,
            })
            .expect("binding allocation should fit");
        let body = builtin_expr(&mut module, BuiltinTerm::True, "True");
        let function_item = module
            .push_item(Item::Function(FunctionItem {
                header: ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: name("keepTrue"),
                type_parameters: Vec::new(),
                context: Vec::new(),
                parameters: vec![FunctionParameter {
                    span: unit_span(),
                    binding: parameter_binding,
                    annotation: Some(int_ref),
                }],
                annotation: Some(bool_ref),
                body,
            }))
            .expect("function item allocation should fit");
        let function_expr = item_expr(&mut module, function_item, "keepTrue");
        let expr = constructor_expr(
            &mut module,
            function_box,
            "FunctionBox",
            vec![function_expr],
        );
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };
        let mut typing = GateTypeContext::new(&module);
        let mut bindings = SourceOptionTypeBindings::default();

        assert_eq!(
            validator.check_source_option_expr(
                expr,
                &SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
                &mut typing,
                &mut bindings,
            ),
            SourceOptionTypeCheck::Match,
        );
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::OpaqueItem {
                item: function_box,
                name: "FunctionBox".to_owned(),
                arguments: vec![GateType::Primitive(BuiltinType::Int)],
            }),
        );
    }

    #[test]
    fn source_option_constructor_field_expectations_preserve_contract_parameter_substitutions() {
        let mut module = Module::new(FileId::new(0));
        let payload = type_parameter(&mut module, "A");
        let payload_ref = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Name(TypeReference::resolved(
                    NamePath::from_vec(vec![name("A")]).expect("parameter path should stay valid"),
                    TypeResolution::TypeParameter(payload),
                )),
            })
            .expect("parameter type allocation should fit");
        let signal_callee = builtin_type(&mut module, BuiltinType::Signal);
        let signal_payload = module
            .alloc_type(TypeNode {
                span: unit_span(),
                kind: TypeKind::Apply {
                    callee: signal_callee,
                    arguments: NonEmpty::new(payload_ref, Vec::new()),
                },
            })
            .expect("signal type allocation should fit");
        let trigger_box = module
            .push_item(Item::Type(TypeItem {
                header: ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: name("TriggerBox"),
                parameters: vec![payload],
                body: TypeItemBody::Sum(NonEmpty::new(
                    TypeVariant {
                        span: unit_span(),
                        name: name("TriggerBox"),
                        fields: vec![signal_payload],
                    },
                    Vec::new(),
                )),
            }))
            .expect("type item allocation should fit");
        let expected_parent = SourceOptionNamedType::from_item(
            &module,
            trigger_box,
            vec![SourceOptionExpectedType::ContractParameter(
                SourceTypeParameter::B,
            )],
        )
        .expect("named type should stay valid");
        let validator = Validator {
            module: &module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
        };

        assert_eq!(
            validator.source_option_constructor_field_expectations(
                trigger_box,
                &expected_parent,
                &[signal_payload],
            ),
            Some(vec![SourceOptionExpectedType::Signal(Box::new(
                SourceOptionExpectedType::ContractParameter(SourceTypeParameter::B),
            ))]),
        );
    }

    #[test]
    fn source_option_signal_contract_parameters_still_check_outer_signal_shape() {
        let expected = SourceOptionExpectedType::Signal(Box::new(
            SourceOptionExpectedType::ContractParameter(SourceTypeParameter::A),
        ));
        let mut bindings = SourceOptionTypeBindings::default();

        assert!(source_option_expected_matches_actual_type(
            &expected,
            &SourceOptionActualType::from_gate_type(&GateType::Signal(Box::new(
                GateType::Primitive(BuiltinType::Bool),
            ))),
            &mut bindings,
        ));
        assert_eq!(
            bindings.parameter_gate_type(SourceTypeParameter::A),
            Some(GateType::Primitive(BuiltinType::Bool)),
        );
        let mut bindings = SourceOptionTypeBindings::default();
        assert!(!source_option_expected_matches_actual_type(
            &expected,
            &SourceOptionActualType::from_gate_type(&GateType::Primitive(BuiltinType::Bool)),
            &mut bindings,
        ));
    }
}
