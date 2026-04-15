impl Validator<'_> {
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
                DecoratorPayload::Test(_) | DecoratorPayload::Debug(_) => {}
                DecoratorPayload::Deprecated(deprecated) => {
                    if let Some(message) = deprecated.message {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "deprecation message",
                            message,
                        );
                    }
                    if let Some(options) = deprecated.options {
                        self.require_expr(
                            decorator.span,
                            "decorator",
                            "deprecation options expression",
                            options,
                        );
                    }
                }
                DecoratorPayload::Mock(mock) => {
                    self.require_expr(
                        decorator.span,
                        "decorator",
                        "mock target expression",
                        mock.target,
                    );
                    self.require_expr(
                        decorator.span,
                        "decorator",
                        "mock replacement expression",
                        mock.replacement,
                    );
                }
            }
        }
    }

    fn validate_types(&mut self) {
        let mut typing = GateTypeContext::new(self.module);
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
                TypeKind::RecordTransform { transform, source } => {
                    self.require_type(ty.span, "type node", "record row transform source", *source);
                    self.validate_record_row_transform_type(
                        ty.span,
                        transform,
                        *source,
                        &mut typing,
                    );
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

    fn validate_record_row_transform_type(
        &mut self,
        span: SourceSpan,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        typing: &mut GateTypeContext<'_>,
    ) {
        let Some(source_ty) = typing.lower_annotation(source) else {
            return;
        };
        let GateType::Record(fields) = &source_ty else {
            self.diagnostics.push(
                Diagnostic::error("record row transforms require a closed record source type")
                    .with_code(code("record-row-transform-source"))
                    .with_primary_label(
                        span,
                        "this transform does not target a closed record type",
                    ),
            );
            return;
        };
        let field_names = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<HashSet<_>>();
        match transform {
            crate::RecordRowTransform::Pick(labels)
            | crate::RecordRowTransform::Omit(labels)
            | crate::RecordRowTransform::Optional(labels)
            | crate::RecordRowTransform::Required(labels)
            | crate::RecordRowTransform::Defaulted(labels) => {
                let mut seen = HashSet::new();
                for label in labels {
                    if !seen.insert(label.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform references field `{}` more than once",
                                label.text()
                            ))
                            .with_code(code("duplicate-record-row-field"))
                            .with_primary_label(label.span(), "remove the duplicate field label"),
                        );
                    } else if !field_names.contains(label.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform references unknown field `{}`",
                                label.text()
                            ))
                            .with_code(code("unknown-record-row-field"))
                            .with_primary_label(
                                label.span(),
                                "this field does not exist on the source record",
                            ),
                        );
                    }
                }
            }
            crate::RecordRowTransform::Rename(renames) => {
                let mut seen_sources = HashSet::new();
                let mut seen_targets = HashSet::new();
                for rename in renames {
                    if !seen_sources.insert(rename.from.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform renames field `{}` more than once",
                                rename.from.text()
                            ))
                            .with_code(code("duplicate-record-row-field"))
                            .with_primary_label(
                                rename.from.span(),
                                "each source field may be renamed at most once",
                            ),
                        );
                        continue;
                    }
                    if !field_names.contains(rename.from.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform references unknown field `{}`",
                                rename.from.text()
                            ))
                            .with_code(code("unknown-record-row-field"))
                            .with_primary_label(
                                rename.from.span(),
                                "this field does not exist on the source record",
                            ),
                        );
                    }
                    if !seen_targets.insert(rename.to.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform renames multiple fields to `{}`",
                                rename.to.text()
                            ))
                            .with_code(code("record-row-rename-collision"))
                            .with_primary_label(rename.to.span(), "rename targets must be unique"),
                        );
                    }
                }
                let retained_names = field_names
                    .iter()
                    .filter(|name| !seen_sources.contains(**name))
                    .copied()
                    .collect::<HashSet<_>>();
                for rename in renames {
                    if retained_names.contains(rename.to.text()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "record row transform renames `{}` to `{}`, which collides with an existing field",
                                rename.from.text(),
                                rename.to.text()
                            ))
                            .with_code(code("record-row-rename-collision"))
                            .with_primary_label(rename.to.span(), "this renamed field collides with a retained field"),
                        );
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
                    // TODO: constructor arity validation requires resolved type info — deferred to type checking
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
                ExprKind::Lambda(lambda) => {
                    for parameter in &lambda.parameters {
                        self.check_span("lambda parameter", parameter.span);
                        self.require_binding(
                            parameter.span,
                            "lambda parameter",
                            "binding",
                            parameter.binding,
                        );
                        if let Some(annotation) = parameter.annotation {
                            self.require_type(
                                parameter.span,
                                "lambda parameter",
                                "annotation",
                                annotation,
                            );
                        }
                    }
                    self.require_expr(expr.span, "expression", "lambda body", lambda.body);
                }
                ExprKind::Record(record) => {
                    for field in &record.fields {
                        self.check_span("record field", field.span);
                        self.check_name(&field.label);
                        self.require_expr(field.span, "record field", "field value", field.value);
                    }
                }
                ExprKind::AmbientSubject => {}
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
                ExprKind::PatchApply { target, patch } => {
                    self.require_expr(expr.span, "expression", "patch target", *target);
                    self.validate_patch_block(expr.span, patch);
                }
                ExprKind::PatchLiteral(patch) => self.validate_patch_block(expr.span, patch),
                ExprKind::Pipe(pipe) => {
                    self.require_expr(expr.span, "expression", "pipe head", pipe.head);
                    for stage in pipe.stages.iter() {
                        self.check_span("pipe stage", stage.span);
                        for pattern in stage.pattern_inputs() {
                            self.require_pattern(
                                stage.span,
                                "pipe stage",
                                pattern.role.validator_label(),
                                pattern.pattern,
                            );
                        }
                        for input in stage.expr_inputs() {
                            self.require_expr(
                                stage.span,
                                "pipe stage",
                                input.role.validator_label(),
                                input.expr,
                            );
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

    fn validate_patch_block(&mut self, owner_span: SourceSpan, patch: &crate::PatchBlock) {
        for entry in &patch.entries {
            self.check_span("patch entry", entry.span);
            self.check_span("patch selector", entry.selector.span);
            for segment in &entry.selector.segments {
                match segment {
                    crate::PatchSelectorSegment::Named { name, span, .. } => {
                        self.check_span("patch selector segment", *span);
                        self.check_name(name);
                    }
                    crate::PatchSelectorSegment::BracketTraverse { span } => {
                        self.check_span("patch selector segment", *span);
                    }
                    crate::PatchSelectorSegment::BracketExpr { expr, span } => {
                        self.check_span("patch selector segment", *span);
                        self.require_expr(*span, "patch selector", "bracket expression", *expr);
                    }
                }
            }
            self.check_span("patch instruction", entry.instruction.span);
            match entry.instruction.kind {
                crate::PatchInstructionKind::Replace(expr)
                | crate::PatchInstructionKind::Store(expr) => self.require_expr(
                    entry.instruction.span,
                    "patch instruction",
                    "instruction expression",
                    expr,
                ),
                crate::PatchInstructionKind::Remove => {
                    let _ = owner_span;
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
                    if let Some(control) = self.module.control_nodes().get(*control)
                        && matches!(
                            control.kind(),
                            ControlNodeKind::Empty | ControlNodeKind::Case
                        ) {
                            self.illegal_direct_control(node.span, control.kind());
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
                        if let Some(node) = self.module.control_nodes().get(empty)
                            && node.kind() != ControlNodeKind::Empty {
                                self.wrong_control_kind(
                                    each.span,
                                    "each empty branch",
                                    ControlNodeKind::Empty,
                                    node.kind(),
                                );
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
                        if let Some(node) = self.module.control_nodes().get(*case)
                            && node.kind() != ControlNodeKind::Case {
                                self.wrong_control_kind(
                                    match_node.span,
                                    "match case",
                                    ControlNodeKind::Case,
                                    node.kind(),
                                );
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
            if !self.module.root_items().contains(&item_id) {
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
                                        field.ty,
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
                    for update in &item.reactive_updates {
                        self.check_span("reactive update clause", update.span);
                        self.require_expr(
                            update.span,
                            "reactive update clause",
                            "guard",
                            update.guard,
                        );
                        self.require_expr(
                            update.span,
                            "reactive update clause",
                            "body",
                            update.body,
                        );
                    }
                    self.check_signal_dependencies(item.header.span, &item.signal_dependencies);
                    self.validate_reactive_update_dependencies(item_id, item);
                    let has_source_decorator = item.header.decorators.iter().any(|decorator_id| {
                        matches!(
                            self.module
                                .decorators()
                                .get(*decorator_id)
                                .map(|decorator| &decorator.payload),
                            Some(DecoratorPayload::Source(_))
                        )
                    });
                    if has_source_decorator
                        && item.body.is_some()
                        && !item.is_source_capability_handle
                    {
                        self.diagnostics.push(
                            Diagnostic::error("`@source` signals must be bodyless")
                                .with_code(code("source-signals-must-be-bodyless"))
                                .with_label(DiagnosticLabel::primary(
                                    item.header.span,
                                    "declare the raw source as a bodyless `sig` and derive from it separately",
                                )),
                        );
                    }
                    match (
                        has_source_decorator,
                        item.source_metadata.as_ref(),
                        item.is_source_capability_handle,
                    ) {
                        (true, None, true) => {}
                        (true, Some(_), true) => self.diagnostics.push(
                            Diagnostic::error(
                                "source capability handles must not carry executable source metadata",
                            )
                            .with_code(code("unexpected-source-capability-metadata"))
                            .with_label(DiagnosticLabel::primary(
                                item.header.span,
                                "lower capability handle use sites into concrete source bindings instead",
                            )),
                        ),
                        (true, Some(metadata), false) => {
                            self.check_source_metadata(item.header.span, metadata)
                        }
                        (true, None, false) => self.diagnostics.push(
                            Diagnostic::error("source-backed signal is missing source metadata")
                                .with_code(code("missing-source-metadata"))
                                .with_label(DiagnosticLabel::primary(
                                    item.header.span,
                                    "populate source metadata after name resolution",
                                )),
                        ),
                        (false, Some(_), _) => self.diagnostics.push(
                            Diagnostic::error(
                                "non-source signal unexpectedly carries source metadata",
                            )
                            .with_code(code("unexpected-source-metadata"))
                            .with_label(DiagnosticLabel::primary(
                                item.header.span,
                                "only `@source` signals should carry source metadata",
                            )),
                        ),
                        (false, None, _) => {}
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
                    for constraint in &item.param_constraints {
                        self.require_type(
                            item.header.span,
                            "class item",
                            "parameter constraint",
                            *constraint,
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
                        if member.kind == DomainMemberKind::Literal
                            && member.name.text().chars().count() < 2
                        {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    "domain literal suffixes must be at least two characters long",
                                )
                                .with_code(code("literal-suffix-too-short"))
                                .with_primary_label(
                                    member.name.span(),
                                    "use a suffix with at least two characters",
                                ),
                            );
                        }
                        self.require_type(
                            member.span,
                            "domain member",
                            "annotation",
                            member.annotation,
                        );
                        for parameter in &member.parameters {
                            self.check_span("domain member parameter", parameter.span);
                            self.require_binding(
                                member.span,
                                "domain member",
                                "parameter binding",
                                parameter.binding,
                            );
                            if let Some(annotation) = parameter.annotation {
                                self.require_type(
                                    parameter.span,
                                    "domain member parameter",
                                    "annotation",
                                    annotation,
                                );
                            }
                        }
                        if let Some(body) = member.body {
                            self.require_expr(member.span, "domain member", "body", body);
                        }
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
                    for operation in &item.contract.operations {
                        self.check_span("provider contract operation", operation.span);
                        self.check_name(&operation.name);
                        self.require_type(
                            operation.span,
                            "provider contract operation",
                            "annotation",
                            operation.annotation,
                        );
                    }
                    for command in &item.contract.commands {
                        self.check_span("provider contract command", command.span);
                        self.check_name(&command.name);
                        self.require_type(
                            command.span,
                            "provider contract command",
                            "annotation",
                            command.annotation,
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
                        Some(item.target.segments().last().text()),
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
                Item::Hoist(_item) => {
                    // No module path to validate — hoist is a self-declaration.
                }
            }
        }
    }

    fn validate_decorator_semantics(&mut self) {
        let mut typing = GateTypeContext::new(self.module);
        for (item_id, item) in self.module.items().iter() {
            if self.module.ambient_items().contains(&item_id) {
                continue;
            }
            self.validate_item_decorator_semantics(item_id, item, &mut typing);
        }
    }

    fn validate_item_decorator_semantics(
        &mut self,
        item_id: ItemId,
        item: &Item,
        typing: &mut GateTypeContext<'_>,
    ) {
        let mut test_count = 0usize;
        let mut debug_count = 0usize;
        let mut deprecated_count = 0usize;
        let has_test = self.item_has_test_decorator(item);
        let mut mocked_imports = HashSet::new();

        for decorator_id in item.decorators() {
            let decorator = &self.module.decorators()[*decorator_id];
            match &decorator.payload {
                DecoratorPayload::Test(_) => {
                    test_count += 1;
                    self.validate_test_decorator(item_id, decorator.span, typing);
                }
                DecoratorPayload::Debug(_) => {
                    debug_count += 1;
                }
                DecoratorPayload::Deprecated(deprecated) => {
                    deprecated_count += 1;
                    self.validate_deprecated_decorator(decorator.span, deprecated);
                }
                DecoratorPayload::Mock(mock) => {
                    self.validate_mock_decorator(
                        item_id,
                        has_test,
                        decorator.span,
                        mock,
                        &mut mocked_imports,
                        typing,
                    );
                }
                DecoratorPayload::Bare
                | DecoratorPayload::Call(_)
                | DecoratorPayload::RecurrenceWakeup(_)
                | DecoratorPayload::Source(_) => {}
            }
        }

        if test_count > 1 {
            self.diagnostics.push(
                Diagnostic::error("duplicate `@test` decorator")
                    .with_code(code("duplicate-test-decorator"))
                    .with_primary_label(item.span(), "keep only one `@test` decorator"),
            );
        }
        if debug_count > 1 {
            self.diagnostics.push(
                Diagnostic::error("duplicate `@debug` decorator")
                    .with_code(code("duplicate-debug-decorator"))
                    .with_primary_label(item.span(), "keep only one `@debug` decorator"),
            );
        }
        if deprecated_count > 1 {
            self.diagnostics.push(
                Diagnostic::error("duplicate `@deprecated` decorator")
                    .with_code(code("duplicate-deprecated-decorator"))
                    .with_primary_label(item.span(), "keep only one `@deprecated` decorator"),
            );
        }

        if let Item::Export(export) = item
            && let ResolutionState::Resolved(ExportResolution::Item(target)) = export.resolution
            && let Some(target_item) = self.module.items().get(target)
            && self.item_has_test_decorator(target_item)
        {
            self.diagnostics.push(
                Diagnostic::error("`@test` items cannot be exported")
                    .with_code(code("test-export"))
                    .with_primary_label(
                        export.header.span,
                        "remove this export or move the test into a non-exported declaration",
                    ),
            );
        }
    }

    fn validate_test_decorator(
        &mut self,
        item_id: ItemId,
        span: SourceSpan,
        typing: &mut GateTypeContext<'_>,
    ) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }
        let Some(ty) = typing.item_value_type(item_id) else {
            return;
        };
        let GateType::Task { value, .. } = ty else {
            self.diagnostics.push(
                Diagnostic::error("`@test` values must have a `Task ...` type")
                    .with_code(code("invalid-test-type"))
                    .with_primary_label(span, "annotate or infer this test as a task value"),
            );
            return;
        };
        if !test_result_type_supported(value.as_ref()) {
            self.diagnostics.push(
                Diagnostic::error(
                    "`@test` tasks must produce `Unit`, `Bool`, `Result ...`, or `Validation ...`",
                )
                .with_code(code("invalid-test-result-type"))
                .with_primary_label(span, "return one of the supported test result shapes"),
            );
        }
    }

    fn validate_deprecated_decorator(
        &mut self,
        span: SourceSpan,
        deprecated: &crate::DeprecatedDecorator,
    ) {
        if let Some(message) = deprecated.message
            && self.module.expr_static_text(message).is_none()
        {
            self.diagnostics.push(
                Diagnostic::error("`@deprecated` message must be a plain text literal")
                    .with_code(code("invalid-deprecated-message"))
                    .with_primary_label(
                        message_span(self.module, message),
                        "use a plain text literal",
                    ),
            );
        }
        let Some(options) = deprecated.options else {
            return;
        };
        let Some(expr) = self.module.exprs().get(options) else {
            return;
        };
        let ExprKind::Record(RecordExpr { fields }) = &expr.kind else {
            self.diagnostics.push(
                Diagnostic::error("`@deprecated` options must use `with { replacement: \"...\" }`")
                    .with_code(code("invalid-deprecated-options"))
                    .with_primary_label(span, "use a record literal in `with { ... }`"),
            );
            return;
        };
        let mut seen_replacement = false;
        for field in fields {
            if field.label.text() != "replacement" {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "`@deprecated` does not support option `{}`",
                        field.label.text()
                    ))
                    .with_code(code("unknown-deprecated-option"))
                    .with_primary_label(field.span, "remove this option"),
                );
                continue;
            }
            if seen_replacement {
                self.diagnostics.push(
                    Diagnostic::error("duplicate `replacement` option in `@deprecated`")
                        .with_code(code("duplicate-deprecated-replacement"))
                        .with_primary_label(field.span, "keep only one `replacement` option"),
                );
                continue;
            }
            seen_replacement = true;
            if self.module.expr_static_text(field.value).is_none() {
                self.diagnostics.push(
                    Diagnostic::error("`@deprecated` replacement must be a plain text literal")
                        .with_code(code("invalid-deprecated-replacement"))
                        .with_primary_label(field.span, "use a plain text literal"),
                );
            }
        }
    }

    fn validate_mock_decorator(
        &mut self,
        item_id: ItemId,
        has_test: bool,
        span: SourceSpan,
        mock: &crate::MockDecorator,
        seen_imports: &mut HashSet<ImportId>,
        typing: &mut GateTypeContext<'_>,
    ) {
        if !has_test {
            self.diagnostics.push(
                Diagnostic::error("`@mock` is only valid on `@test` values")
                    .with_code(code("mock-outside-test"))
                    .with_primary_label(span, "add `@test` to this declaration or remove `@mock`"),
            );
        }
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }

        let Some(target_import) = self.mock_target_import(mock.target) else {
            self.diagnostics.push(
                Diagnostic::error(
                    "the first `@mock` argument must name an imported top-level function",
                )
                .with_code(code("invalid-mock-target"))
                .with_primary_label(
                    message_span(self.module, mock.target),
                    "reference an imported function binding here",
                ),
            );
            return;
        };
        if !seen_imports.insert(target_import) {
            self.diagnostics.push(
                Diagnostic::error("duplicate `@mock` target on one test")
                    .with_code(code("duplicate-mock-target"))
                    .with_primary_label(
                        message_span(self.module, mock.target),
                        "mock each imported binding at most once per test",
                    ),
            );
            return;
        }

        let Some(target_ty) = self.mock_target_type(target_import, typing) else {
            self.diagnostics.push(
                Diagnostic::error("`@mock` targets must be imported functions")
                    .with_code(code("mock-non-function-target"))
                    .with_primary_label(
                        message_span(self.module, mock.target),
                        "this imported binding is not callable",
                    ),
            );
            return;
        };

        let Some(replacement_ty) = self.mock_replacement_type(mock.replacement, typing) else {
            self.diagnostics.push(
                Diagnostic::error(
                    "the second `@mock` argument must name a top-level replacement value or function",
                )
                .with_code(code("invalid-mock-replacement"))
                .with_primary_label(
                    message_span(self.module, mock.replacement),
                    "reference a top-level replacement binding here",
                ),
            );
            return;
        };

        if target_ty != replacement_ty {
            self.diagnostics.push(
                Diagnostic::error("`@mock` replacement type must exactly match the mocked import")
                    .with_code(code("mock-type-mismatch"))
                    .with_primary_label(
                        message_span(self.module, mock.replacement),
                        format!(
                            "replacement has type `{replacement_ty}` but the mocked import has type `{target_ty}`"
                        ),
                    ),
            );
        }

        let Some(test_ty) = typing.item_value_type(item_id) else {
            return;
        };
        if !matches!(test_ty, GateType::Task { .. }) {
            self.diagnostics.push(
                Diagnostic::error("`@mock` can only decorate executable `@test` task values")
                    .with_code(code("mock-non-task-test"))
                    .with_primary_label(span, "make this test value a `Task ...`"),
            );
        }
    }

    fn item_has_test_decorator(&self, item: &Item) -> bool {
        item.decorators().iter().any(|decorator_id| {
            self.module
                .decorators()
                .get(*decorator_id)
                .is_some_and(|decorator| matches!(decorator.payload, DecoratorPayload::Test(_)))
        })
    }

    fn item_deprecation_notice(&self, item_id: ItemId) -> Option<DeprecationNotice> {
        let item = self.module.items().get(item_id)?;
        item.decorators().iter().find_map(|decorator_id| {
            let decorator = self.module.decorators().get(*decorator_id)?;
            let DecoratorPayload::Deprecated(deprecated) = &decorator.payload else {
                return None;
            };
            Some(DeprecationNotice {
                message: deprecated
                    .message
                    .and_then(|message| self.module.expr_static_text(message)),
                replacement: deprecated.options.and_then(|options| {
                    let ExprKind::Record(RecordExpr { fields }) =
                        &self.module.exprs().get(options)?.kind
                    else {
                        return None;
                    };
                    fields
                        .iter()
                        .find(|field| field.label.text() == "replacement")
                        .and_then(|field| self.module.expr_static_text(field.value))
                }),
            })
        })
    }

    fn mock_target_import(&self, expr_id: ExprId) -> Option<ImportId> {
        let ExprKind::Name(reference) = &self.module.exprs().get(expr_id)?.kind else {
            return None;
        };
        let ResolutionState::Resolved(TermResolution::Import(import_id)) = reference.resolution
        else {
            return None;
        };
        Some(import_id)
    }

    fn mock_replacement_type(
        &mut self,
        expr_id: ExprId,
        typing: &mut GateTypeContext<'_>,
    ) -> Option<GateType> {
        let ExprKind::Name(reference) = &self.module.exprs().get(expr_id)?.kind else {
            return None;
        };
        match reference.resolution {
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                typing.item_value_type(item_id)
            }
            ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                typing.import_value_type(import_id)
            }
            _ => None,
        }
    }

    fn mock_target_type(
        &self,
        import_id: ImportId,
        typing: &GateTypeContext<'_>,
    ) -> Option<GateType> {
        self.module.imports()[import_id]
            .callable_type
            .as_ref()
            .map(|ty| typing.lower_import_value_type(ty))
    }

    fn emit_item_deprecation_warning(&mut self, span: SourceSpan, item_id: ItemId) {
        let Some(notice) = self.item_deprecation_notice(item_id) else {
            return;
        };
        let name = item_name(self.module.items().get(item_id));
        self.emit_deprecation_warning(span, name.as_deref().unwrap_or("item"), &notice);
    }

    fn emit_import_deprecation_warning(&mut self, span: SourceSpan, import_id: ImportId) {
        let Some(import) = self.module.imports().get(import_id) else {
            return;
        };
        let Some(notice) = import.deprecation.as_ref() else {
            return;
        };
        self.emit_deprecation_warning(span, import.local_name.text(), notice);
    }

    fn emit_deprecation_warning(
        &mut self,
        span: SourceSpan,
        name: &str,
        notice: &DeprecationNotice,
    ) {
        let mut message = format!("`{name}` is deprecated");
        if let Some(detail) = notice.message.as_deref()
            && !detail.is_empty()
        {
            message.push_str(": ");
            message.push_str(detail);
        }
        let mut diagnostic = Diagnostic::warning(message)
            .with_code(code("deprecated-use"))
            .with_primary_label(span, "this reference uses a deprecated declaration");
        if let Some(replacement) = notice.replacement.as_deref()
            && !replacement.is_empty()
        {
            diagnostic = diagnostic.with_note(format!("replacement: {replacement}"));
        }
        self.diagnostics.push(diagnostic);
    }

    fn validate_signal_cycles(&mut self) {
        // Collect all signal items and their declared dependency edges.
        let mut signal_deps: HashMap<ItemId, Vec<ItemId>> = HashMap::new();
        let mut signal_names: HashMap<ItemId, String> = HashMap::new();
        for (item_id, item) in self.module.items().iter() {
            if let Item::Signal(signal) = item {
                let deps = {
                    let mut excluded: HashSet<ItemId> = HashSet::new();
                    if let Some(metadata) = signal.source_metadata.as_ref() {
                        excluded.extend(metadata.lifecycle_dependencies.merged());
                    }
                    excluded.extend(signal.temporal_input_dependencies.iter().copied());
                    if excluded.is_empty() {
                        signal.signal_dependencies.clone()
                    } else {
                        signal
                            .signal_dependencies
                            .iter()
                            .copied()
                            .filter(|dep| !excluded.contains(dep))
                            .collect()
                    }
                };
                signal_deps.insert(item_id, deps);
                signal_names.insert(item_id, signal.name.text().to_owned());
            }
        }

        // DFS cycle detection: for each unvisited signal, walk the dependency
        // graph and report any back-edge (cycle).
        let mut visited: HashSet<ItemId> = HashSet::new();
        let signal_ids: Vec<ItemId> = signal_deps.keys().copied().collect();
        for start in signal_ids {
            if visited.contains(&start) {
                continue;
            }
            // path holds the current DFS stack for cycle reconstruction.
            let mut path: Vec<ItemId> = Vec::new();
            let mut on_stack: HashSet<ItemId> = HashSet::new();
            let mut stack: Vec<(ItemId, usize)> = vec![(start, 0)];
            while !stack.is_empty() {
                let (node, dep_index) = *stack.last().unwrap();
                if dep_index == 0 {
                    // First time visiting this node in this DFS path.
                    if visited.contains(&node) {
                        stack.pop();
                        continue;
                    }
                    on_stack.insert(node);
                    path.push(node);
                }
                let deps = signal_deps.get(&node).map(|v| v.as_slice()).unwrap_or(&[]);
                if dep_index < deps.len() {
                    let dep = deps[dep_index];
                    stack.last_mut().unwrap().1 = dep_index + 1;
                    if on_stack.contains(&dep) {
                        // Found a cycle — reconstruct the cycle path from the DFS path.
                        let cycle_start = path.iter().position(|&id| id == dep).unwrap_or(0);
                        let cycle_names: Vec<String> = path[cycle_start..]
                            .iter()
                            .map(|id| {
                                signal_names
                                    .get(id)
                                    .cloned()
                                    .unwrap_or_else(|| "<unknown>".to_owned())
                            })
                            .collect();
                        let cycle_path =
                            format!("{} -> {}", cycle_names.join(" -> "), cycle_names[0]);
                        let offending_name = signal_names
                            .get(&dep)
                            .cloned()
                            .unwrap_or_else(|| "<unknown>".to_owned());
                        // Report on the span of the signal where the cycle was detected.
                        let dep_span = self.module.items().get(dep).and_then(|item| {
                            if let Item::Signal(signal_item) = item {
                                Some(signal_item.header.span)
                            } else {
                                None
                            }
                        });
                        if let Some(span) = dep_span {
                            self.diagnostics.push(
                                Diagnostic::error(format!(
                                    "signal '{offending_name}' has a circular dependency: {cycle_path}"
                                ))
                                .with_code(code("circular-signal-dependency"))
                                .with_label(DiagnosticLabel::primary(
                                    span,
                                    "this signal is part of a circular dependency chain",
                                ))
                                .with_help("break the cycle by using a reactive update (`when`) or restructuring signal dependencies"),
                            );
                        }
                        // Skip further DFS from this branch to avoid duplicate reports.
                        stack.pop();
                        path.pop();
                        on_stack.remove(&node);
                        visited.insert(node);
                        continue;
                    }
                    if !visited.contains(&dep) && signal_deps.contains_key(&dep) {
                        stack.push((dep, 0));
                    }
                } else {
                    // All dependencies of this node have been explored.
                    stack.pop();
                    path.pop();
                    on_stack.remove(&node);
                    visited.insert(node);
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
                                        field.ty,
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
                    for constraint in &item.param_constraints {
                        self.check_expected_type_kind(
                            *constraint,
                            &parameters,
                            "class parameter constraint",
                        );
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
                    let _ = self.class_parameter_kinds(&item);
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
                    for operation in &item.contract.operations {
                        self.check_expected_type_kind(
                            operation.annotation,
                            &[],
                            "provider contract operation annotation",
                        );
                    }
                    for command in &item.contract.commands {
                        self.check_expected_type_kind(
                            command.annotation,
                            &[],
                            "provider contract command annotation",
                        );
                    }
                }
                Item::Instance(item) => {
                    let parameters = item.type_parameters.clone();
                    let class_kind = self
                        .instance_class_item_id(&item)
                        .and_then(|class_item_id| self.kind_for_item(class_item_id));
                    self.check_type_reference_kind(
                        &item.class,
                        &parameters,
                        class_kind.unwrap_or_else(|| Kind::constructor(item.arguments.len())),
                        "instance class head",
                    );
                    let parameter_kinds =
                        self.instance_class_item_id(&item)
                            .and_then(|class_item_id| match &self.module.items()[class_item_id] {
                                Item::Class(class_item) => self.class_parameter_kinds(class_item),
                                _ => None,
                            });
                    for (index, argument) in item.arguments.iter().enumerate() {
                        let expected = parameter_kinds
                            .as_ref()
                            .and_then(|kinds| kinds.get(index).cloned())
                            .unwrap_or(Kind::Type);
                        self.check_type_kind(*argument, &parameters, expected, "instance argument");
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
                Item::Use(_) | Item::Export(_) | Item::Hoist(_) => {}
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
                match self
                    .check_builtin_source_trigger_projection(
                        provider,
                        &pending_option,
                        typing,
                        &mut trial_bindings,
                    )
                    .unwrap_or_else(|| {
                        self.check_source_option_expr(
                            pending_option.field.value,
                            &pending_option.expected,
                            typing,
                            &mut trial_bindings,
                        )
                    }) {
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

    fn check_builtin_source_trigger_projection(
        &self,
        provider: BuiltinSourceProvider,
        option: &PendingSourceOptionValue,
        typing: &mut GateTypeContext<'_>,
        bindings: &mut SourceOptionTypeBindings,
    ) -> Option<SourceOptionTypeCheck> {
        if provider != BuiltinSourceProvider::DbLive
            || option.field.label.text() != "refreshOn"
            || !is_db_changed_trigger_projection(self.module, option.field.value)
        {
            return None;
        }

        if expr_signal_dependencies(self.module, [option.field.value]).len() != 1 {
            return Some(SourceOptionTypeCheck::Unknown);
        }

        let actual = typing
            .infer_expr(option.field.value, &GateExprEnv::default(), None)
            .actual();
        Some(match actual {
            Some(actual)
                if source_option_expected_matches_actual_type(
                    &option.expected,
                    &actual,
                    bindings,
                ) =>
            {
                SourceOptionTypeCheck::Match
            }
            Some(actual) => SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                span: self.module.exprs()[option.field.value].span,
                actual: actual.to_string(),
            }),
            None => SourceOptionTypeCheck::Unknown,
        })
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
            | ResolutionState::Resolved(TermResolution::DomainConstructor(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_)) => None,
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
            if let Some((parameter, bound_type)) = bind_parameter.as_ref()
                && !bindings.bind_or_match_actual(*parameter, bound_type) {
                    return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: actual.parent_name.clone(),
                    });
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
            if let Some((parameter, bound_type)) = bind_parameter.as_ref()
                && !bindings.bind_or_match_actual(*parameter, bound_type) {
                    return SourceOptionTypeCheck::Mismatch(SourceOptionTypeMismatch {
                        span: reference.path.span(),
                        actual: actual.parent_name.clone(),
                    });
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
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                Some(SourceOptionActualType::Domain {
                    item: resolution.domain,
                    name: domain.name.text().to_owned(),
                    arguments: vec![SourceOptionActualType::Hole; domain.parameters.len()],
                })
            }
            LiteralSuffixResolution::Import(_) => None,
        }
    }

    fn source_option_literal_suffix_result_type(
        &self,
        resolution: LiteralSuffixResolution,
    ) -> Option<TypeId> {
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
                let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
                    return None;
                };
                let member = domain.members.get(resolution.member_index)?;
                if member.kind != DomainMemberKind::Literal {
                    return None;
                }
                let TypeKind::Arrow { result, .. } = &self.module.types()[member.annotation].kind
                else {
                    return None;
                };
                Some(*result)
            }
            LiteralSuffixResolution::Import(_) => None,
        }
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
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_)) => None,
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
            ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::DomainConstructor(_)) => None,
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
            TypeKind::RecordTransform { .. } => None,
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
        if !self.source_option_hir_type_is_signal_contract(expected)
            && let SourceOptionActualType::Signal(inner) = actual {
                return self.source_option_hir_type_matches_actual_type_inner(
                    expected,
                    inner,
                    substitutions,
                );
            }

        self.source_option_hir_type_matches_actual_type_inner(expected, actual, substitutions)
    }

    pub(crate) fn source_option_hir_type_matches_actual_type_inner(
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
            TypeKind::RecordTransform { .. } => None,
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
            | Item::Export(_)
            | Item::Hoist(_) => None,
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
            if !seen.insert(field.label.text().to_owned()) {
                return self.check_source_option_expr_by_inference_or_unknown(
                    expr_id,
                    &SourceOptionExpectedType::Record(expected.to_vec()),
                    typing,
                    bindings,
                    value_stack,
                );
            }
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
            field_types: variant.fields.iter().map(|f| f.ty).collect(),
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
                    self.validate_no_nested_pipes(item.body);
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
                    self.validate_no_nested_pipes(item.body);
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
                        self.validate_no_nested_pipes(body);
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
                        self.validate_no_nested_pipes(member.body);
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
                | Item::Export(_)
                | Item::Hoist(_) => {}
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
                DecoratorPayload::Test(_) | DecoratorPayload::Debug(_) => {}
                DecoratorPayload::Deprecated(deprecated) => {
                    let env = GateExprEnv::default();
                    if let Some(message) = deprecated.message {
                        self.validate_case_exhaustiveness_expr_tree(message, &env, &mut typing);
                        self.validate_recurrence_expr_tree(message, None, None, &env, &mut typing);
                    }
                    if let Some(options) = deprecated.options {
                        self.validate_case_exhaustiveness_expr_tree(options, &env, &mut typing);
                        self.validate_recurrence_expr_tree(options, None, None, &env, &mut typing);
                    }
                }
                DecoratorPayload::Mock(mock) => {
                    let env = GateExprEnv::default();
                    for expr in [mock.target, mock.replacement] {
                        self.validate_case_exhaustiveness_expr_tree(expr, &env, &mut typing);
                        self.validate_recurrence_expr_tree(expr, None, None, &env, &mut typing);
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
                        | ExprKind::AmbientSubject
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
                        ExprKind::Lambda(lambda) => {
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: lambda.body,
                                env,
                            });
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
                        ExprKind::PatchApply { target, patch } => {
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: target,
                                env: env.clone(),
                            });
                            for entry in patch.entries.into_iter().rev() {
                                match entry.instruction.kind {
                                    crate::PatchInstructionKind::Replace(expr)
                                    | crate::PatchInstructionKind::Store(expr) => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr,
                                            env: env.clone(),
                                        });
                                    }
                                    crate::PatchInstructionKind::Remove => {}
                                }
                                for segment in entry.selector.segments.into_iter().rev() {
                                    if let crate::PatchSelectorSegment::BracketExpr {
                                        expr, ..
                                    } = segment
                                    {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr,
                                            env: env.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        ExprKind::PatchLiteral(patch) => {
                            for entry in patch.entries.into_iter().rev() {
                                match entry.instruction.kind {
                                    crate::PatchInstructionKind::Replace(expr)
                                    | crate::PatchInstructionKind::Store(expr) => {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr,
                                            env: env.clone(),
                                        });
                                    }
                                    crate::PatchInstructionKind::Remove => {}
                                }
                                for segment in entry.selector.segments.into_iter().rev() {
                                    if let crate::PatchSelectorSegment::BracketExpr {
                                        expr, ..
                                    } = segment
                                    {
                                        work.push(CaseExhaustivenessWork::Expr {
                                            expr,
                                            env: env.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        ExprKind::Pipe(pipe) => {
                            work.push(CaseExhaustivenessWork::Expr {
                                expr: pipe.head,
                                env: env.clone(),
                            });
                            let mut current = self.infer_case_expr_type(pipe.head, &env, typing);
                            for semantic_stage in pipe.semantic_stages() {
                                match semantic_stage {
                                    crate::PipeSemanticStage::Single { stage, .. } => match &stage.kind {
                                        PipeStageKind::Transform { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing.infer_transform_stage(*expr, &env, subject)
                                            });
                                        }
                                        PipeStageKind::Tap { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                        }
                                        PipeStageKind::Gate { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing.infer_gate_stage(*expr, &env, subject)
                                            });
                                        }
                                        PipeStageKind::Map { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing.infer_fanout_map_stage(*expr, &env, subject)
                                            });
                                        }
                                        PipeStageKind::FanIn { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing.infer_fanin_stage(*expr, &env, subject)
                                            });
                                        }
                                        PipeStageKind::Truthy { expr }
                                        | PipeStageKind::Falsy { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = None;
                                        }
                                        PipeStageKind::Apply { .. } => {
                                            unreachable!("semantic stage iterator groups apply runs")
                                        }
                                        PipeStageKind::RecurStart { expr }
                                        | PipeStageKind::RecurStep { expr }
                                        | PipeStageKind::Validate { expr }
                                        | PipeStageKind::Previous { expr }
                                        | PipeStageKind::Diff { expr } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *expr,
                                                env: env.clone(),
                                            });
                                            current = None;
                                        }
                                        PipeStageKind::Delay { duration } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *duration,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing
                                                    .infer_delay_stage_info(*duration, &env, subject)
                                                    .ty
                                            });
                                        }
                                        PipeStageKind::Burst { every, count } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *every,
                                                env: env.clone(),
                                            });
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *count,
                                                env: env.clone(),
                                            });
                                            current = current.as_ref().and_then(|subject| {
                                                typing
                                                    .infer_burst_stage_info(
                                                        *every, *count, &env, subject,
                                                    )
                                                    .ty
                                            });
                                        }
                                        PipeStageKind::Accumulate { seed, step } => {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *seed,
                                                env: env.clone(),
                                            });
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr: *step,
                                                env: env.clone(),
                                            });
                                            current = None;
                                        }
                                        PipeStageKind::Case { .. } => {
                                            unreachable!("semantic stage iterator groups case runs")
                                        }
                                    },
                                    crate::PipeSemanticStage::ApplyRun(apply_run) => {
                                        for expr in apply_run.exprs().rev() {
                                            work.push(CaseExhaustivenessWork::Expr {
                                                expr,
                                                env: env.clone(),
                                            });
                                        }
                                        current = None;
                                    }
                                    crate::PipeSemanticStage::TruthyFalsyPair(pair) => {
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
                                    }
                                    crate::PipeSemanticStage::CaseRun(case_run) => {
                                        if let Some(subject) = current.clone() {
                                            self.validate_pipe_case_run(
                                                &case_run,
                                                &subject,
                                                typing,
                                            );
                                            for case_stage in case_run.stages().rev() {
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
                                            for case_stage in case_run.stages().rev() {
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
        case_run: &crate::PipeCaseStageRun<'_>,
        subject: &GateType,
        typing: &mut GateTypeContext<'_>,
    ) {
        let Some(shape) = typing.case_subject_shape(subject) else {
            return;
        };
        let mut covered = HashSet::new();
        let mut has_catch_all = false;
        for stage in case_run.stages() {
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
        let span = case_run.start_stage().span;
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
        diagnostic = diagnostic.with_help(format!(
            "add branches for {missing_list} to make the pattern exhaustive"
        ));
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
        // `recurrence_suffix()` returns `Err(PipeRecurrenceShapeError)` for pipes that violate
        // stage-ordering constraints:
        //   - `OrphanStep`    — a `<|@` step with no preceding `@|>` start
        //   - `MissingStep`   — a `@|>` start with no following `<|@` step
        //   - `TrailingStage` — any non-`<|@` stage after the recurrence suffix has begun
        //
        // All three cases are diagnosed during the lowering phase in `lower.rs` via
        // `emit_orphan_recur_step`, `emit_unfinished_recurrence`, and
        // `emit_illegal_recurrence_continuation`.  By the time this validation pass runs, the
        // compiler has already emitted the relevant diagnostics; silently returning here is
        // correct — we must not attempt to validate semantics on malformed pipe structure.
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

    fn validate_fanout_segment(
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
        let element_subject = subject.fanout_element().cloned()?;
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
        let mapped_collection_type = typing.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Map, carrier),
            mapped_element_type.clone(),
        );
        let mut segment_env = env.clone();
        extend_pipe_env_with_stage_result_memo(
            &mut segment_env,
            segment.map_stage(),
            &mapped_collection_type,
        );
        for stage in segment.filter_stages() {
            let PipeStageKind::Gate { expr } = stage.kind else {
                unreachable!("validated fan-out filters must use `?|>`");
            };
            let filter_env = pipe_stage_expr_env(&segment_env, stage, &mapped_collection_type);
            self.validate_fanout_filter_stage(
                stage.span,
                expr,
                &filter_env,
                &mapped_element_type,
                typing,
            )?;
            extend_pipe_env_with_stage_result_memo(
                &mut segment_env,
                stage,
                &mapped_collection_type,
            );
        }
        match segment.join_expr() {
            Some(join_expr) => self.validate_fanin_stage(
                {
                    let join_stage = segment
                        .join_stage()
                        .expect("join expression implies join stage");
                    join_stage.span
                },
                join_expr,
                &pipe_stage_expr_env(
                    &segment_env,
                    segment
                        .join_stage()
                        .expect("join expression implies join stage"),
                    &mapped_collection_type,
                ),
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
        PipeSubjectWalker::new_with_limit(pipe, env, typing, prefix_stage_count).walk(
            typing,
            |stage, current, current_env, typing| match stage {
                crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanin_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Apply { .. } => {
                        unreachable!("subject walker groups apply runs")
                    }
                    PipeStageKind::Case { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. }
                    | PipeStageKind::Validate { .. }
                    | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Stop,
                    PipeStageKind::Previous { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_previous_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Diff { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_diff_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Delay { duration } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_delay_stage_info(*duration, current_env, s).ty),
                    },
                    PipeStageKind::Burst { every, count } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            typing
                                .infer_burst_stage_info(*every, *count, current_env, s)
                                .ty
                        }),
                    },
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Transform { .. }
                    | PipeStageKind::Tap { .. } => {
                        unreachable!("subject walker groups truthy/falsy pairs and consumes transform/tap")
                    }
                },
                crate::PipeSubjectStage::FanoutSegment(segment) => {
                    let new_subject = current.and_then(|s| {
                        match crate::fanout_elaboration::elaborate_fanout_segment(
                            self.module,
                            segment,
                            Some(s),
                            current_env,
                            typing,
                        ) {
                            crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                Some(plan.result_type)
                            }
                            crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => None,
                        }
                    });
                    PipeSubjectStepOutcome::Continue { new_subject }
                },
                crate::PipeSubjectStage::TruthyFalsyPair(pair) => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_truthy_falsy_pair(pair, current_env, s)),
                },
                crate::PipeSubjectStage::ApplyRun(_) => PipeSubjectStepOutcome::Stop,
                crate::PipeSubjectStage::CaseRun(_) => PipeSubjectStepOutcome::Stop,
            },
        )
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
        if let Some(options) = source.options
            && let ExprKind::Record(record) = &self.module.exprs()[options].kind {
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
                    BuiltinSourceProvider::DbLive => (
                        "this live query source still needs an explicit recurrence wakeup such as `refreshOn` or reactive source inputs",
                        "`db.live` issues one query when subscribed; table-change refresh and debounce stay explicit at the current recurrence boundary",
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
                            | BuiltinSourceProvider::DbusOwnName
                            | BuiltinSourceProvider::DbusSignal
                            | BuiltinSourceProvider::DbusMethod
                            | BuiltinSourceProvider::WindowKeyDown
                            | BuiltinSourceProvider::GtkDarkMode
                            | BuiltinSourceProvider::GtkClipboard
                            | BuiltinSourceProvider::GtkWindowSize
                            | BuiltinSourceProvider::GtkWindowFocus
                            | BuiltinSourceProvider::ImapIdle
                            | BuiltinSourceProvider::ImapFetchBody
                            | BuiltinSourceProvider::SmtpSend
                            | BuiltinSourceProvider::DbExec => {
                                "this built-in source should already have planned a wakeup; if you hit this diagnostic, keep the failing fixture because the recurrence wakeup adapter is inconsistent"
                            }
                            BuiltinSourceProvider::ProcessArgs
                            | BuiltinSourceProvider::ProcessCwd
                            | BuiltinSourceProvider::EnvGet
                            | BuiltinSourceProvider::StdioRead
                            | BuiltinSourceProvider::PathHome
                            | BuiltinSourceProvider::PathConfigHome
                            | BuiltinSourceProvider::PathDataHome
                            | BuiltinSourceProvider::PathCacheHome
                            | BuiltinSourceProvider::PathTempDir
                            | BuiltinSourceProvider::TimeNowMs => {
                                "this built-in source publishes one host-context snapshot when subscribed; add an explicit recurrence wakeup or switch to a non-recurrent signal"
                            }
                            BuiltinSourceProvider::DbConnect
                            | BuiltinSourceProvider::ImapConnect => {
                                "this connection source publishes one connection snapshot when subscribed; add an explicit recurrence wakeup or switch to a non-recurrent signal"
                            }
                            BuiltinSourceProvider::HttpGet
                            | BuiltinSourceProvider::HttpPost
                            | BuiltinSourceProvider::DbLive
                            | BuiltinSourceProvider::FsRead
                            | BuiltinSourceProvider::ApiGet
                            | BuiltinSourceProvider::ApiPost
                            | BuiltinSourceProvider::ApiPut
                            | BuiltinSourceProvider::ApiPatch
                            | BuiltinSourceProvider::ApiDelete => {
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
                        "add a compiler-known non-source wakeup witness such as `@recur.timer 5sec` or `@recur.backoff 3times`, or use a compiler-known `@source` provider with explicit wakeup proof",
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
        PipeSubjectWalker::new(pipe, env, typing).walk(
            typing,
            |stage, current, current_env, typing| match stage {
                crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            self.validate_fanout_map_stage(
                                stage.span,
                                *expr,
                                current_env,
                                s,
                                typing,
                            )
                        }),
                    },
                    PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            self.validate_fanin_stage(stage.span, *expr, current_env, s, typing)
                        }),
                    },
                    PipeStageKind::Apply { .. } => {
                        unreachable!("subject walker groups apply runs")
                    }
                    PipeStageKind::Case { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. }
                    | PipeStageKind::Validate { .. }
                    | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Continue {
                        new_subject: None,
                    },
                    PipeStageKind::Previous { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_previous_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Diff { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_diff_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Delay { duration } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_delay_stage_info(*duration, current_env, s).ty),
                    },
                    PipeStageKind::Burst { every, count } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            typing
                                .infer_burst_stage_info(*every, *count, current_env, s)
                                .ty
                        }),
                    },
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Transform { .. }
                    | PipeStageKind::Tap { .. } => {
                        unreachable!("subject walker groups truthy/falsy pairs and consumes transform/tap")
                    }
                },
                crate::PipeSubjectStage::FanoutSegment(segment) => {
                    let new_subject = current.and_then(|s| {
                        self.validate_fanout_segment(segment, current_env, s, typing)
                    });
                    PipeSubjectStepOutcome::Continue { new_subject }
                },
                crate::PipeSubjectStage::TruthyFalsyPair(pair) => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_truthy_falsy_pair(pair, current_env, s)),
                },
                crate::PipeSubjectStage::ApplyRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
                crate::PipeSubjectStage::CaseRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
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
        PipeSubjectWalker::new_with_limit(pipe, env, typing, limit).walk(
            typing,
            |stage, current, current_env, typing| match stage {
                crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            self.validate_gate_stage(stage.span, *expr, current_env, s, typing)
                        }),
                    },
                    PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanin_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Apply { .. } => {
                        unreachable!("subject walker groups apply runs")
                    }
                    PipeStageKind::Case { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. }
                    | PipeStageKind::Validate { .. }
                    | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Continue {
                        new_subject: None,
                    },
                    PipeStageKind::Previous { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_previous_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Diff { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_diff_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Delay { duration } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_delay_stage_info(*duration, current_env, s).ty),
                    },
                    PipeStageKind::Burst { every, count } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            typing
                                .infer_burst_stage_info(*every, *count, current_env, s)
                                .ty
                        }),
                    },
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Transform { .. }
                    | PipeStageKind::Tap { .. } => {
                        unreachable!("subject walker groups truthy/falsy pairs and consumes transform/tap")
                    }
                },
                crate::PipeSubjectStage::FanoutSegment(segment) => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_fanout_segment_result_type(segment, current_env, s)),
                },
                crate::PipeSubjectStage::TruthyFalsyPair(pair) => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| typing.infer_truthy_falsy_pair(pair, current_env, s)),
                },
                crate::PipeSubjectStage::ApplyRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
                crate::PipeSubjectStage::CaseRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
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

    /// Reject pipe expressions that appear nested inside another expression.
    ///
    /// # Invariant
    /// A pipe expression may only appear as the direct body of a top-level declaration
    /// (`func`, `value`, `signal`, `source`, `view`, `result`).  Placing a pipe inside
    /// a function argument, list literal, record field, or any other sub-expression is
    /// forbidden: pipes must be written as separate named declarations.
    fn validate_no_nested_pipes(&mut self, root: ExprId) {
        let module = self.module;
        walk_expr_tree(module, root, |_, expr, is_root| {
            if !is_root
                && let ExprKind::Pipe(pipe) = &expr.kind {
                    // Result-block desugaring legitimately nests PipeExprs (each
                    // `a <- result { … }` binding produces an inner pipe as the head of
                    // the outer result-block pipe).  Skip the diagnostic for those
                    // synthetic pipes; user-authored nested pipes are still flagged.
                    if !pipe.result_block_desugaring {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "pipe expression cannot be nested inside another expression",
                            )
                            .with_code(code("nested-pipe"))
                            .with_primary_label(
                                expr.span,
                                "move this pipe into a separate named declaration",
                            ),
                        );
                    }
                }
        });
    }

    fn validate_truthy_falsy_pipe(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) {
        PipeSubjectWalker::new(pipe, env, typing).walk(
            typing,
            |stage, current, current_env, typing| match stage {
                crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanin_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Apply { .. } => {
                        unreachable!("subject walker groups apply runs")
                    }
                    PipeStageKind::Case { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. }
                    | PipeStageKind::Validate { .. }
                    | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Continue {
                        new_subject: None,
                    },
                    PipeStageKind::Previous { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_previous_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Diff { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_diff_stage_info(*expr, current_env, s).ty),
                    },
                    PipeStageKind::Delay { duration } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_delay_stage_info(*duration, current_env, s).ty),
                    },
                    PipeStageKind::Burst { every, count } => PipeSubjectStepOutcome::Continue {
                        new_subject: current.and_then(|s| {
                            typing
                                .infer_burst_stage_info(*every, *count, current_env, s)
                                .ty
                        }),
                    },
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Transform { .. }
                    | PipeStageKind::Tap { .. } => {
                        unreachable!("subject walker groups truthy/falsy pairs and consumes transform/tap")
                    }
                },
                crate::PipeSubjectStage::TruthyFalsyPair(pair) => PipeSubjectStepOutcome::Continue {
                    new_subject: current.and_then(|s| {
                        self.validate_truthy_falsy_pair(pair, current_env, s, typing)
                    }),
                },
                crate::PipeSubjectStage::FanoutSegment(segment) => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_fanout_segment_result_type(segment, current_env, s)),
                },
                crate::PipeSubjectStage::ApplyRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
                crate::PipeSubjectStage::CaseRun(_) => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                },
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
            subject,
        );
        let falsy_info = typing.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
            subject,
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

        let truthy_ty = truthy_ty?;
        let falsy_ty = falsy_ty?;
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
        let element_subject = subject.fanout_element().cloned()?;
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
        let carrier = typing.fanout_carrier(subject)?;
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
        if let Some(predicate_ty) = predicate_info.ty
            && !predicate_ty.is_bool() {
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
            GateIssue::UnknownLiteralSuffix { span, suffix } => {
                self.diagnostics.push(
                    Diagnostic::error(format!("unknown literal suffix `{suffix}`"))
                        .with_code(code("unknown-literal-suffix"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "this suffixed literal does not match any visible domain suffix",
                        )),
                );
            }
            GateIssue::AmbiguousLiteralSuffix {
                span,
                suffix,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!("literal suffix `{suffix}` is ambiguous"))
                        .with_code(code("ambiguous-literal-suffix"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "add a type annotation to disambiguate which domain suffix you want",
                        ))
                        .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
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
            GateIssue::AmbientSubjectOutsidePipe { span } => {
                self.diagnostics.push(
                    Diagnostic::error(
                        "`.` is only available when a pipe stage provides an ambient subject",
                    )
                    .with_code(code("ambient-subject-outside-pipe"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use `.` inside a pipe stage or bind the value to a name first",
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
            GateIssue::UnknownLiteralSuffix { span, suffix } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch uses unknown literal suffix `{suffix}`"
                    ))
                    .with_code(code("unknown-literal-suffix"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "this suffixed literal does not match any visible domain suffix",
                    )),
                );
            }
            GateIssue::AmbiguousLiteralSuffix {
                span,
                suffix,
                candidates,
            } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch literal suffix `{suffix}` is ambiguous"
                    ))
                    .with_code(code("ambiguous-literal-suffix"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add a type annotation to disambiguate which domain suffix you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
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
            GateIssue::AmbientSubjectOutsidePipe { span } => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{branch_name} branch cannot use `.` because this branch has no matched payload subject"
                    ))
                    .with_code(code("ambient-subject-outside-pipe"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "use a literal or named value here, or switch to `||>` for an explicit pattern",
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
                GateIssue::UnknownLiteralSuffix { span, suffix },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} uses unknown literal suffix `{suffix}`"
                    ))
                    .with_code(code("unknown-literal-suffix"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "this suffixed literal does not match any visible domain suffix",
                    )),
                );
            }
            (
                context,
                GateIssue::AmbiguousLiteralSuffix {
                    span,
                    suffix,
                    candidates,
                },
            ) => {
                let subject = match context {
                    FanoutIssueContext::MapElement => "fan-out body",
                    FanoutIssueContext::JoinCollection => "fan-in body",
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} literal suffix `{suffix}` is ambiguous"
                    ))
                    .with_code(code("ambiguous-literal-suffix"))
                    .with_label(DiagnosticLabel::primary(
                        span,
                        "add a type annotation to disambiguate which domain suffix you want",
                    ))
                    .with_note(format!("candidates: {}", candidates.join(", "))),
                );
            }
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
            (context, GateIssue::AmbientSubjectOutsidePipe { span }) => {
                let (subject, label) = match context {
                    FanoutIssueContext::MapElement => (
                        "fan-out body",
                        "use `.` only where each mapped element is the current ambient subject",
                    ),
                    FanoutIssueContext::JoinCollection => (
                        "fan-in body",
                        "use `.` only where the joined collection is the current ambient subject",
                    ),
                };
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "{subject} cannot use `.` without an ambient pipe subject"
                    ))
                    .with_code(code("ambient-subject-outside-pipe"))
                    .with_label(DiagnosticLabel::primary(span, label)),
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
        self.build_kind_graph_for_type_with_parameter_map(root, parameters)
            .map(|(store, root, spans, _)| (store, root, spans))
    }

    fn build_kind_graph_for_type_with_parameter_map(
        &mut self,
        root: TypeId,
        parameters: &[TypeParameterId],
    ) -> Option<(
        KindStore,
        KindExprId,
        HashMap<KindExprId, SourceSpan>,
        HashMap<TypeParameterId, TypingKindParameterId>,
    )> {
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
                        TypeKind::RecordTransform { source, .. } => {
                            stack.push(KindBuildFrame::Exit(type_id));
                            stack.push(KindBuildFrame::Enter(*source));
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
                        TypeKind::RecordTransform { source, .. } => lowered[source],
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

        Some((store, lowered[&root], spans, parameter_map))
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
        if let Some(cached) = self.kind_item_cache.get(&item_id) {
            return cached.clone();
        }
        if !self.kind_item_stack.insert(item_id) {
            return None;
        }
        let inferred = match &self.module.items()[item_id] {
            Item::Type(item) => Some(Kind::constructor(item.parameters.len())),
            Item::Class(item) => self.class_parameter_kinds(item).map(|parameter_kinds| {
                parameter_kinds
                    .into_iter()
                    .rev()
                    .fold(Kind::Type, |result, parameter| {
                        Kind::arrow(parameter, result)
                    })
            }),
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
        };
        self.kind_item_stack.remove(&item_id);
        self.kind_item_cache.insert(item_id, inferred.clone());
        inferred
    }

    fn class_parameter_kinds(&mut self, item: &crate::hir::ClassItem) -> Option<Vec<Kind>> {
        let mut inferred = HashMap::<TypeParameterId, Kind>::new();
        let parameters = item.parameters.iter().copied().collect::<Vec<_>>();

        for superclass in &item.superclasses {
            self.merge_type_parameter_kinds(*superclass, &parameters, &Kind::Type, &mut inferred)?;
        }
        for constraint in &item.param_constraints {
            self.merge_type_parameter_kinds(*constraint, &parameters, &Kind::Type, &mut inferred)?;
        }
        for member in &item.members {
            let mut parameters = item.parameters.iter().copied().collect::<Vec<_>>();
            parameters.extend(member.type_parameters.iter().copied());
            for constraint in &member.context {
                self.merge_type_parameter_kinds(
                    *constraint,
                    &parameters,
                    &Kind::Type,
                    &mut inferred,
                )?;
            }
            self.merge_type_parameter_kinds(
                member.annotation,
                &parameters,
                &Kind::Type,
                &mut inferred,
            )?;
        }

        Some(
            item.parameters
                .iter()
                .map(|parameter| inferred.remove(parameter).unwrap_or(Kind::Type))
                .collect(),
        )
    }

    fn merge_type_parameter_kinds(
        &mut self,
        root: TypeId,
        parameters: &[TypeParameterId],
        expected: &Kind,
        inferred: &mut HashMap<TypeParameterId, Kind>,
    ) -> Option<()> {
        let (store, root_expr, spans, parameter_map) =
            self.build_kind_graph_for_type_with_parameter_map(root, parameters)?;
        let solution = match KindChecker.expect_kind_with_solution(&store, root_expr, expected) {
            Ok(solution) => solution,
            Err(error) => {
                self.emit_kind_error("type", &spans, error);
                return None;
            }
        };
        for parameter in parameters {
            let Some(kind_parameter) = parameter_map.get(parameter).copied() else {
                continue;
            };
            let inferred_kind = solution.parameter_kind(kind_parameter).clone();
            match inferred.entry(*parameter) {
                Entry::Vacant(entry) => {
                    entry.insert(inferred_kind);
                }
                Entry::Occupied(entry) if entry.get() == &inferred_kind => {}
                Entry::Occupied(entry) => {
                    let parameter_meta = &self.module.type_parameters()[*parameter];
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "type parameter `{}` is used at incompatible kinds",
                            parameter_meta.name.text()
                        ))
                        .with_code(code("type-parameter-kind-mismatch"))
                        .with_primary_label(
                            parameter_meta.span,
                            format!(
                                "this parameter was inferred as both `{}` and `{inferred_kind}`",
                                entry.get()
                            ),
                        ),
                    );
                    return None;
                }
            }
        }
        Some(())
    }

    fn import_type_kind(&self, import_id: ImportId) -> Option<Kind> {
        let import = &self.module.imports()[import_id];
        match &import.metadata {
            ImportBindingMetadata::TypeConstructor { kind, .. } => Some(kind.clone()),
            ImportBindingMetadata::Domain { kind, .. } => Some(kind.clone()),
            ImportBindingMetadata::BuiltinType(builtin) => Some(builtin_kind(*builtin)),
            ImportBindingMetadata::Value { .. }
            | ImportBindingMetadata::IntrinsicValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_)
            | ImportBindingMetadata::DomainSuffix { .. }
            | ImportBindingMetadata::InstanceMember { .. }
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
            Some(reference.path.segments().last().text()),
            |this, resolution| match resolution {
                TermResolution::Local(binding) => {
                    this.require_binding(reference.span(), "term reference", "binding", *binding);
                }
                TermResolution::Item(item) => {
                    this.require_item(reference.span(), "term reference", "item", *item);
                    this.emit_item_deprecation_warning(reference.span(), *item);
                }
                TermResolution::Import(import) => {
                    this.require_import(reference.span(), "term reference", "import", *import);
                    this.emit_import_deprecation_warning(reference.span(), *import);
                }
                TermResolution::DomainConstructor(item_id) => {
                    this.require_item(reference.span(), "term reference", "item", *item_id);
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
                TermResolution::AmbiguousHoistedImports(candidates) => {
                    for import_id in candidates.iter().copied() {
                        this.require_import(
                            reference.span(),
                            "term reference",
                            "import",
                            import_id,
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
        if let ResolutionState::Resolved(resolution) = literal.resolution.as_ref() {
            self.require_literal_suffix_resolution(literal.suffix.span(), &literal.suffix, *resolution);
        }
    }

    fn validate_reactive_update_dependencies(&mut self, item_id: ItemId, item: &SignalItem) {
        if self.mode != ValidationMode::RequireResolvedNames {
            return;
        }
        let target_name = item.name.text();
        for update in &item.reactive_updates {
            if expr_signal_dependencies(self.module, [update.guard]).contains(&item_id) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "reactive update guard for `{target_name}` cannot read the target signal itself"
                    ))
                    .with_code(code("reactive-update-self-reference"))
                    .with_label(DiagnosticLabel::primary(
                        self.module.exprs()[update.guard].span,
                        "guards must not create feedback through the signal they update",
                    ))
                    .with_note(
                        "use an explicit recurrence form instead when feedback is intentional",
                    ),
                );
            }
            if expr_signal_dependencies(self.module, [update.body]).contains(&item_id) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "reactive update body for `{target_name}` cannot read the target signal itself"
                    ))
                    .with_code(code("reactive-update-self-reference"))
                    .with_label(DiagnosticLabel::primary(
                        self.module.exprs()[update.body].span,
                        "reactive update bodies must not depend on the current value they overwrite",
                    ))
                    .with_note(
                        "use an explicit recurrence form instead when feedback is intentional",
                    ),
                );
            }
        }
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
            if let Some(item) = self.module.items().get(*dependency)
                && !matches!(item, Item::Signal(_)) {
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
            if let Some(previous) = previous
                && previous >= *dependency {
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
            previous = Some(*dependency);
        }
    }

    fn check_signal_dependencies(&mut self, span: SourceSpan, dependencies: &[ItemId]) {
        let mut previous = None;
        for dependency in dependencies {
            self.require_item(span, "signal item", "signal dependency", *dependency);
            if let Some(item) = self.module.items().get(*dependency)
                && !matches!(item, Item::Signal(_)) {
                    self.diagnostics.push(
                        Diagnostic::error("signal dependency must point at a signal item")
                            .with_code(code("invalid-signal-dependency"))
                            .with_label(DiagnosticLabel::primary(
                                span,
                                "update the signal dependency list to reference only signal items",
                            )),
                    );
                }
            if let Some(previous) = previous
                && previous >= *dependency {
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
            previous = Some(*dependency);
        }
    }

    fn check_type_reference(&mut self, reference: &TypeReference) {
        self.check_name_path(&reference.path);
        self.check_resolution(
            reference.span(),
            "type reference",
            reference.resolution.as_ref(),
            Some(reference.path.segments().last().text()),
            |this, resolution| match resolution {
                TypeResolution::Item(item) => {
                    this.require_item(reference.span(), "type reference", "item", *item);
                    this.emit_item_deprecation_warning(reference.span(), *item);
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
                    this.emit_import_deprecation_warning(reference.span(), *import);
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
        name_hint: Option<&str>,
        on_resolved: impl FnOnce(&mut Self, &T),
    ) {
        match resolution {
            ResolutionState::Resolved(value) => on_resolved(self, value),
            ResolutionState::Unresolved if self.mode == ValidationMode::RequireResolvedNames => {
                let mut diag =
                    Diagnostic::error(format!("{subject} remains unresolved in resolved HIR mode"))
                        .with_code(code("unresolved-name"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "Milestone 2 HIR should resolve this reference before validation",
                        ));
                if let Some(name) = name_hint
                    && let Some(suggestion) = suggest_similar_name(self.module, name) {
                        diag = diag.with_help(format!("did you mean `{suggestion}`?"));
                    }
                self.diagnostics.push(diag);
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
        match resolution {
            LiteralSuffixResolution::DomainMember(resolution) => {
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
                                "literal suffixes must resolve to domain suffix declarations",
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
                                "update the literal suffix resolution to target an existing suffix member",
                            )),
                    );
                    return;
                };
                if member.kind != DomainMemberKind::Literal || member.name.text() != suffix.text() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "literal suffix resolution does not match the target domain suffix",
                        )
                        .with_code(code("invalid-literal-suffix-resolution"))
                        .with_label(DiagnosticLabel::primary(
                            span,
                            "the resolved domain suffix must match the suffix spelling used here",
                        )),
                    );
                }
            }
            LiteralSuffixResolution::Import(import_id) => {
                let Some(import) = self.module.imports().get(import_id) else {
                    self.diagnostics.push(
                        Diagnostic::error("literal suffix resolution points at a missing import")
                            .with_code(code("invalid-literal-suffix-resolution"))
                            .with_label(DiagnosticLabel::primary(
                                span,
                                "update the literal suffix resolution to target an existing import",
                            )),
                    );
                    return;
                };
                match &import.metadata {
                    ImportBindingMetadata::DomainSuffix { suffix_name, .. }
                        if suffix_name.as_ref() == suffix.text() => {}
                    _ => {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "literal suffix import resolution does not target a suffix constructor",
                            )
                            .with_code(code("invalid-literal-suffix-resolution"))
                            .with_label(DiagnosticLabel::primary(
                                span,
                                "this resolved import does not match the suffix spelling used here",
                            )),
                        );
                    }
                }
            }
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
