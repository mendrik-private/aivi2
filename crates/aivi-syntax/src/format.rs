use crate::cst::{
    BinaryOperator, ClassMember, ClassMemberName, Decorator, DecoratorArguments, DecoratorPayload,
    DomainItem, DomainMember, DomainMemberName, ExportItem, Expr, ExprKind, FunctionParam,
    Identifier, InstanceItem, InstanceMember, Item, MapExpr, MarkupAttribute, MarkupAttributeValue,
    MarkupNode, Module, NamedItem, Pattern, PatternKind, PipeExpr, PipeStage, PipeStageKind,
    ProjectionPath, QualifiedName, RecordExpr, RecordField, RecordPatternField, SourceDecorator,
    SourceProviderContractItem, SourceProviderContractMember, SourceProviderContractSchemaMember,
    SuffixedIntegerLiteral, TextLiteral, TextSegment, TypeDeclBody, TypeExpr, TypeExprKind,
    TypeField, TypeVariant, UnaryOperator, UseItem,
};

const INDENT_WIDTH: usize = 4;
const INLINE_LIMIT: usize = 32;
const TYPE_VARIANT_INDENT: usize = 2;

const EXPR_PIPE_PREC: u8 = 0;
const EXPR_RANGE_PREC: u8 = 1;
const EXPR_OR_PREC: u8 = 2;
const EXPR_AND_PREC: u8 = 3;
const EXPR_COMPARE_PREC: u8 = 4;
const EXPR_ADD_PREC: u8 = 5;
const EXPR_MUL_PREC: u8 = 6;
const EXPR_APPLY_PREC: u8 = 7;
const EXPR_PROJECTION_PREC: u8 = 8;
const EXPR_PREFIX_PREC: u8 = 9;
const TYPE_ARROW_PREC: u8 = 0;
const TYPE_APPLY_PREC: u8 = 1;
const PATTERN_APPLY_PREC: u8 = 1;

/// Canonical formatter for the supported Milestone 1 surface subset.
#[derive(Clone, Copy, Debug, Default)]
pub struct Formatter;

impl Formatter {
    pub fn format(&self, module: &Module) -> String {
        let formatted_items: Vec<_> = module
            .items()
            .iter()
            .map(|item| self.format_item(item))
            .collect();
        let mut lines = Vec::new();
        for (index, item) in module.items().iter().enumerate() {
            lines.extend(formatted_items[index].iter().cloned());
            if index + 1 < module.items().len()
                && self.needs_blank_line_between(
                    item,
                    &formatted_items[index],
                    &module.items()[index + 1],
                    &formatted_items[index + 1],
                )
            {
                lines.push(String::new());
            }
        }

        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        }
    }

    fn format_item(&self, item: &Item) -> Vec<String> {
        let mut lines = Vec::new();
        for decorator in item.decorators() {
            lines.extend(self.format_decorator(decorator).into_lines());
        }

        match item {
            Item::Type(item) => lines.extend(self.format_type_item(item)),
            Item::Value(item) => lines.extend(self.format_value_item("val", item, true)),
            Item::Function(item) => lines.extend(self.format_function_item(item)),
            Item::Signal(item) => lines.extend(self.format_value_item("sig", item, true)),
            Item::Class(item) => lines.extend(self.format_class_item(item)),
            Item::Instance(item) => lines.extend(self.format_instance_item(item)),
            Item::Domain(item) => lines.extend(self.format_domain_item(item)),
            Item::SourceProviderContract(item) => {
                lines.extend(self.format_source_provider_contract_item(item))
            }
            Item::Use(item) => lines.extend(self.format_use_item(item)),
            Item::Export(item) => lines.extend(self.format_export_item(item)),
            Item::Error(_) => {
                lines.push("# <unparseable item>".to_owned());
            }
        }

        lines
    }

    fn needs_blank_line_between(
        &self,
        left_item: &Item,
        left_lines: &[String],
        right_item: &Item,
        right_lines: &[String],
    ) -> bool {
        !self.compacts_with_next_item(left_item, left_lines)
            || !self.compacts_with_next_item(right_item, right_lines)
            || left_item.kind() != right_item.kind()
    }

    fn compacts_with_next_item(&self, item: &Item, lines: &[String]) -> bool {
        item.decorators().is_empty()
            && lines.len() == 1
            && matches!(item, Item::Value(_) | Item::Signal(_) | Item::Export(_))
    }

    fn format_type_item(&self, item: &NamedItem) -> Vec<String> {
        let mut header = format!("type {}", self.item_name(&item.name));
        for parameter in &item.type_parameters {
            header.push(' ');
            header.push_str(&parameter.text);
        }

        match item.type_body() {
            Some(TypeDeclBody::Alias(ty)) => {
                let force_break =
                    self.should_force_type_break(display_width(&format!("{header} = ")), ty);
                let block = self.format_type_block(ty, force_break);
                if block.is_inline() {
                    vec![format!(
                        "{header} = {}",
                        block.inline_text().expect("inline block")
                    )]
                } else {
                    block.prefixed(&format!("{header} = ")).into_lines()
                }
            }
            Some(TypeDeclBody::Sum(variants)) => {
                let inline = self.format_sum_inline(variants);
                let line = format!("{header} = {inline}");
                if display_width(&line) <= INLINE_LIMIT {
                    vec![line]
                } else {
                    let mut lines = vec![format!("{header} =")];
                    lines.extend(self.format_sum_block(variants).into_lines());
                    lines
                }
            }
            None => vec![format!("{header} =")],
        }
    }

    fn format_value_item(
        &self,
        keyword: &str,
        item: &NamedItem,
        spaced_annotation: bool,
    ) -> Vec<String> {
        let mut header = format!("{keyword} {}", self.item_name(&item.name));
        if let Some(annotation) = &item.annotation {
            header.push_str(if spaced_annotation { ": " } else { ":" });
            header.push_str(&self.format_type_inline(annotation, 0));
        }

        let Some(body) = item.expr_body() else {
            return vec![header];
        };

        let force_break =
            self.should_force_expr_break(display_width(&format!("{header} = ")), body);
        let block = self.format_expr_block(body, force_break);
        if block.is_inline() {
            vec![format!(
                "{header} = {}",
                block.inline_text().expect("inline block")
            )]
        } else if block.starts_with_delimiter() {
            block.prefixed(&format!("{header} = ")).into_lines()
        } else {
            let indent = if matches!(&body.kind, ExprKind::Pipe(pipe) if pipe.head.is_none())
                && matches!(keyword, "val" | "sig")
            {
                1
            } else {
                INDENT_WIDTH
            };
            let mut lines = vec![format!("{header} =")];
            lines.extend(block.indented(indent).into_lines());
            lines
        }
    }

    fn format_function_item(&self, item: &NamedItem) -> Vec<String> {
        let mut header = format!("fun {}", self.item_name(&item.name));
        if let Some(annotation) = &item.annotation {
            header.push(':');
            header.push_str(&self.format_type_inline(annotation, 0));
        }
        for parameter in &item.parameters {
            header.push(' ');
            header.push_str(&self.format_function_param(parameter));
        }
        header.push_str(" =>");

        let Some(body) = item.expr_body() else {
            return vec![header];
        };

        let force_break = self.should_force_expr_break(INDENT_WIDTH, body);
        let block = self.format_expr_block(body, force_break);
        let mut lines = vec![header];
        lines.extend(block.indented(INDENT_WIDTH).into_lines());
        lines
    }

    fn format_class_item(&self, item: &NamedItem) -> Vec<String> {
        let mut header = format!("class {}", self.item_name(&item.name));
        for parameter in &item.type_parameters {
            header.push(' ');
            header.push_str(&parameter.text);
        }

        let Some(body) = item.class_body() else {
            return vec![header];
        };

        let mut lines = vec![header];
        for member in &body.members {
            lines.extend(self.format_class_member(member));
        }
        lines
    }

    fn format_instance_item(&self, item: &InstanceItem) -> Vec<String> {
        let class = item
            .class
            .as_ref()
            .map(|class| self.format_qualified_name(class))
            .unwrap_or_else(|| "_".to_owned());
        let mut header = format!("instance {class}");
        if let Some(target) = &item.target {
            header.push(' ');
            header.push_str(&self.format_type_inline(target, 0));
        }

        let Some(body) = &item.body else {
            return vec![header];
        };

        let mut lines = vec![header];
        for member in &body.members {
            lines.extend(self.format_instance_member(member));
        }
        lines
    }

    fn format_use_item(&self, item: &UseItem) -> Vec<String> {
        let path = item
            .path
            .as_ref()
            .map(|path| path.as_dotted())
            .unwrap_or_else(|| "_".to_owned());

        if item.imports.is_empty() {
            return vec![format!("use {path}")];
        }

        if item.imports.len() == 1 {
            return vec![format!(
                "use {path} ({})",
                self.format_use_import(&item.imports[0])
            )];
        }

        let mut lines = vec![format!("use {path} (")];
        for import in &item.imports {
            lines.push(format!(
                "{}{}",
                spaces(INDENT_WIDTH),
                self.format_use_import(import)
            ));
        }
        lines.push(")".to_owned());
        lines
    }

    fn format_use_import(&self, import: &crate::UseImport) -> String {
        let mut text = self.format_qualified_name(&import.path);
        if let Some(alias) = &import.alias {
            text.push_str(" as ");
            text.push_str(&alias.text);
        }
        text
    }

    fn format_export_item(&self, item: &ExportItem) -> Vec<String> {
        match item.targets.as_slice() {
            [] => vec!["export _".to_owned()],
            [target] => vec![format!("export {}", target.text)],
            targets => vec![format!(
                "export ({})",
                targets
                    .iter()
                    .map(|target| target.text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )],
        }
    }

    fn format_domain_item(&self, item: &DomainItem) -> Vec<String> {
        let mut header = format!("domain {}", self.item_name(&item.name));
        for parameter in &item.type_parameters {
            header.push(' ');
            header.push_str(&parameter.text);
        }
        header.push_str(" over ");
        if let Some(carrier) = &item.carrier {
            header.push_str(&self.format_type_inline(carrier, 0));
        }

        let Some(body) = &item.body else {
            return vec![header];
        };

        let mut lines = vec![header];
        for member in &body.members {
            lines.extend(self.format_domain_member(member));
        }
        lines
    }

    fn format_source_provider_contract_item(
        &self,
        item: &SourceProviderContractItem,
    ) -> Vec<String> {
        let provider = item
            .provider
            .as_ref()
            .map(|provider| self.format_qualified_name(provider))
            .unwrap_or_else(|| "_".to_owned());
        let header = format!("provider {provider}");
        let Some(body) = &item.body else {
            return vec![header];
        };

        let mut lines = vec![header];
        for member in &body.members {
            lines.extend(self.format_source_provider_contract_member(member));
        }
        lines
    }

    fn format_source_provider_contract_member(
        &self,
        member: &SourceProviderContractMember,
    ) -> Vec<String> {
        match member {
            SourceProviderContractMember::FieldValue(member) => {
                let mut line = format!("{}{}:", spaces(INDENT_WIDTH), self.item_name(&member.name));
                if let Some(value) = &member.value {
                    line.push(' ');
                    line.push_str(&value.text);
                }
                vec![line]
            }
            SourceProviderContractMember::OptionSchema(member) => {
                self.format_source_provider_contract_schema_member("option", member)
            }
            SourceProviderContractMember::ArgumentSchema(member) => {
                self.format_source_provider_contract_schema_member("argument", member)
            }
        }
    }

    fn format_source_provider_contract_schema_member(
        &self,
        keyword: &str,
        member: &SourceProviderContractSchemaMember,
    ) -> Vec<String> {
        let prefix = format!(
            "{}{keyword} {}: ",
            spaces(INDENT_WIDTH),
            self.item_name(&member.name)
        );
        let Some(annotation) = &member.annotation else {
            return vec![prefix.trim_end().to_owned()];
        };
        let force_break = self.should_force_type_break(display_width(&prefix), annotation);
        let block = self.format_type_block(annotation, force_break);
        if block.is_inline() {
            vec![format!(
                "{prefix}{}",
                block.inline_text().expect("inline block")
            )]
        } else {
            block.prefixed(&prefix).into_lines()
        }
    }

    fn format_class_member(&self, member: &ClassMember) -> Vec<String> {
        let prefix = format!(
            "{}{}: ",
            spaces(INDENT_WIDTH),
            self.format_class_member_name(&member.name)
        );
        let Some(annotation) = &member.annotation else {
            return vec![prefix.trim_end().to_owned()];
        };
        let force_break = self.should_force_type_break(display_width(&prefix), annotation);
        let block = self.format_type_block(annotation, force_break);
        if block.is_inline() {
            vec![format!(
                "{prefix}{}",
                block.inline_text().expect("inline block")
            )]
        } else {
            block.prefixed(&prefix).into_lines()
        }
    }

    fn format_domain_member(&self, member: &DomainMember) -> Vec<String> {
        let prefix = format!(
            "{}{}: ",
            spaces(INDENT_WIDTH),
            self.format_domain_member_name(&member.name)
        );
        let Some(annotation) = &member.annotation else {
            return vec![prefix.trim_end().to_owned()];
        };
        let force_break = self.should_force_type_break(display_width(&prefix), annotation);
        let block = self.format_type_block(annotation, force_break);
        if block.is_inline() {
            vec![format!(
                "{prefix}{}",
                block.inline_text().expect("inline block")
            )]
        } else {
            block.prefixed(&prefix).into_lines()
        }
    }

    fn format_instance_member(&self, member: &InstanceMember) -> Vec<String> {
        let mut header = format!(
            "{}{}",
            spaces(INDENT_WIDTH),
            self.format_class_member_name(&member.name)
        );
        for parameter in &member.parameters {
            header.push(' ');
            header.push_str(&parameter.text);
        }

        let Some(body) = &member.body else {
            return vec![format!("{header} =")];
        };

        let force_break =
            self.should_force_expr_break(display_width(&format!("{header} = ")), body);
        let block = self.format_expr_block(body, force_break);
        if block.is_inline() {
            vec![format!(
                "{header} = {}",
                block.inline_text().expect("inline block")
            )]
        } else if block.starts_with_delimiter() {
            block.prefixed(&format!("{header} = ")).into_lines()
        } else {
            let mut lines = vec![format!("{header} =")];
            lines.extend(block.indented(INDENT_WIDTH * 2).into_lines());
            lines
        }
    }

    fn format_decorator(&self, decorator: &Decorator) -> Block {
        let mut head = format!("@{}", self.format_qualified_name(&decorator.name));
        match &decorator.payload {
            DecoratorPayload::Bare => Block::inline(head),
            DecoratorPayload::Source(payload) => {
                self.append_source_payload(&mut head, payload);
                self.append_decorator_options(head, payload.options.as_ref())
            }
            DecoratorPayload::Arguments(payload) => {
                self.append_argument_payload(&mut head, payload);
                self.append_decorator_options(head, payload.options.as_ref())
            }
        }
    }

    fn append_source_payload(&self, head: &mut String, payload: &SourceDecorator) {
        if let Some(provider) = &payload.provider {
            head.push(' ');
            head.push_str(&self.format_qualified_name(provider));
        }
        for argument in &payload.arguments {
            head.push(' ');
            head.push_str(&self.format_expr_inline(argument, 0));
        }
    }

    fn append_argument_payload(&self, head: &mut String, payload: &DecoratorArguments) {
        for argument in &payload.arguments {
            head.push(' ');
            head.push_str(&self.format_expr_inline(argument, 0));
        }
    }

    fn append_decorator_options(&self, head: String, options: Option<&RecordExpr>) -> Block {
        let Some(options) = options else {
            return Block::inline(head);
        };

        let prefix = format!("{head} with ");
        let force_break = self.should_force_record_break(display_width(&prefix), options);
        self.format_record_block(options, force_break)
            .prefixed(&prefix)
    }

    fn format_function_param(&self, parameter: &FunctionParam) -> String {
        let mut rendered = self.item_name(&parameter.name).to_owned();
        if let Some(annotation) = &parameter.annotation {
            rendered.push(':');
            rendered.push_str(&self.format_type_inline(annotation, 0));
        }
        rendered
    }

    fn format_sum_inline(&self, variants: &[TypeVariant]) -> String {
        variants
            .iter()
            .map(|variant| self.format_variant_inline(variant))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn format_sum_block(&self, variants: &[TypeVariant]) -> Block {
        Block::from_lines(
            variants
                .iter()
                .map(|variant| {
                    format!(
                        "{}| {}",
                        spaces(TYPE_VARIANT_INDENT),
                        self.format_variant_inline(variant)
                    )
                })
                .collect(),
        )
    }

    fn format_variant_inline(&self, variant: &TypeVariant) -> String {
        let mut rendered = variant
            .name
            .as_ref()
            .map(|name| name.text.clone())
            .unwrap_or_else(|| "_".to_owned());
        for field in &variant.fields {
            rendered.push(' ');
            rendered.push_str(&self.format_type_inline(field, TYPE_APPLY_PREC + 1));
        }
        rendered
    }

    fn format_type_block(&self, ty: &TypeExpr, force_multiline: bool) -> Block {
        match &ty.kind {
            TypeExprKind::Record(fields) => self.format_type_record_block(fields, force_multiline),
            TypeExprKind::Tuple(elements) => {
                self.format_type_tuple_block(elements, force_multiline)
            }
            TypeExprKind::Arrow { .. } => Block::inline(self.format_type_inline(ty, 0)),
            TypeExprKind::Apply { callee, arguments } => {
                self.format_type_apply_block(callee, arguments, force_multiline)
            }
            TypeExprKind::Group(inner) => self.format_type_group_block(inner, force_multiline),
            _ => Block::inline(self.format_type_inline(ty, 0)),
        }
    }

    fn format_type_inline(&self, ty: &TypeExpr, parent_prec: u8) -> String {
        match &ty.kind {
            TypeExprKind::Name(name) => name.text.clone(),
            TypeExprKind::Group(inner) => format!("({})", self.format_type_inline(inner, 0)),
            TypeExprKind::Tuple(elements) => self.format_type_tuple_inline(elements),
            TypeExprKind::Record(fields) => self.format_type_record_inline(fields),
            TypeExprKind::Arrow { parameter, result } => wrap_if_needed(
                format!(
                    "{} -> {}",
                    self.format_type_inline(parameter, TYPE_ARROW_PREC + 1),
                    self.format_type_inline(result, TYPE_ARROW_PREC)
                ),
                TYPE_ARROW_PREC,
                parent_prec,
            ),
            TypeExprKind::Apply { callee, arguments } => {
                let mut rendered = self.format_type_inline(callee, TYPE_APPLY_PREC);
                for argument in arguments {
                    rendered.push(' ');
                    rendered.push_str(&self.format_type_inline(argument, TYPE_APPLY_PREC + 1));
                }
                wrap_if_needed(rendered, TYPE_APPLY_PREC, parent_prec)
            }
        }
    }

    fn format_type_apply_block(
        &self,
        callee: &TypeExpr,
        arguments: &[TypeExpr],
        force_multiline: bool,
    ) -> Block {
        let inline = self.format_type_apply_inline(callee, arguments);
        if !force_multiline && display_width(&inline) <= INLINE_LIMIT {
            return Block::inline(inline);
        }

        let mut prefix = self.format_type_inline(callee, TYPE_APPLY_PREC);
        for (index, argument) in arguments.iter().enumerate() {
            let is_last = index + 1 == arguments.len();
            let block = self.format_type_block(
                argument,
                force_multiline && is_last && self.type_can_break(argument),
            );
            if block.is_inline() {
                prefix.push(' ');
                prefix.push_str(block.inline_text().expect("inline block"));
                continue;
            }
            if is_last {
                return block.prefixed(&format!("{prefix} "));
            }
            return Block::inline(inline);
        }

        Block::inline(prefix)
    }

    fn format_type_apply_inline(&self, callee: &TypeExpr, arguments: &[TypeExpr]) -> String {
        let mut rendered = self.format_type_inline(callee, TYPE_APPLY_PREC);
        for argument in arguments {
            rendered.push(' ');
            rendered.push_str(&self.format_type_inline(argument, TYPE_APPLY_PREC + 1));
        }
        rendered
    }

    fn format_type_group_block(&self, inner: &TypeExpr, force_multiline: bool) -> Block {
        let inline = format!("({})", self.format_type_inline(inner, 0));
        if !force_multiline {
            return Block::inline(inline);
        }

        let block = self.format_type_block(inner, true);
        if block.is_inline() {
            Block::inline(inline)
        } else {
            let mut lines = vec!["(".to_owned()];
            lines.extend(block.indented(INDENT_WIDTH).into_lines());
            lines.push(")".to_owned());
            Block::from_lines(lines)
        }
    }

    fn format_type_tuple_block(&self, elements: &[TypeExpr], force_multiline: bool) -> Block {
        let inline = self.format_type_tuple_inline(elements);
        if elements.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["(".to_owned()];
        for (index, element) in elements.iter().enumerate() {
            let suffix = if index + 1 < elements.len() || elements.len() == 1 {
                ","
            } else {
                ""
            };
            lines.extend(
                self.format_type_block(element, false)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push(")".to_owned());
        Block::from_lines(lines)
    }

    fn format_type_tuple_inline(&self, elements: &[TypeExpr]) -> String {
        format_tuple_like(
            elements
                .iter()
                .map(|element| self.format_type_inline(element, 0))
                .collect(),
        )
    }

    fn format_type_record_block(&self, fields: &[TypeField], force_multiline: bool) -> Block {
        let inline = self.format_type_record_inline(fields);
        if fields.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["{".to_owned()];
        for (index, field) in fields.iter().enumerate() {
            let suffix = if index + 1 < fields.len() { "," } else { "" };
            lines.extend(
                self.format_type_field_block(field)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push("}".to_owned());
        Block::from_lines(lines)
    }

    fn format_type_record_inline(&self, fields: &[TypeField]) -> String {
        format_record_like(
            fields
                .iter()
                .map(|field| self.format_type_field_inline(field))
                .collect(),
        )
    }

    fn format_type_field_block(&self, field: &TypeField) -> Block {
        match &field.ty {
            Some(ty) => {
                let block = self.format_type_block(ty, false);
                if block.is_inline() {
                    Block::inline(format!(
                        "{}: {}",
                        field.label.text,
                        block.inline_text().expect("inline block")
                    ))
                } else {
                    block.prefixed(&format!("{}: ", field.label.text))
                }
            }
            None => Block::inline(field.label.text.clone()),
        }
    }

    fn format_type_field_inline(&self, field: &TypeField) -> String {
        match &field.ty {
            Some(ty) => format!("{}: {}", field.label.text, self.format_type_inline(ty, 0)),
            None => field.label.text.clone(),
        }
    }

    fn format_expr_block(&self, expr: &Expr, force_multiline: bool) -> Block {
        match &expr.kind {
            ExprKind::Pipe(pipe) => self.format_pipe_block(pipe),
            ExprKind::Markup(node) => self.format_markup_block(node),
            ExprKind::Tuple(elements) => self.format_expr_tuple_block(elements, force_multiline),
            ExprKind::List(elements) => self.format_list_block(elements, force_multiline),
            ExprKind::Map(map) => self.format_map_block(map, force_multiline),
            ExprKind::Set(elements) => self.format_set_block(elements, force_multiline),
            ExprKind::Record(record) => self.format_record_block(record, force_multiline),
            ExprKind::Apply { callee, arguments } => {
                self.format_expr_apply_block(callee, arguments, force_multiline)
            }
            ExprKind::Group(inner) => self.format_expr_group_block(inner, force_multiline),
            _ => Block::inline(self.format_expr_inline(expr, 0)),
        }
    }

    fn format_expr_inline(&self, expr: &Expr, parent_prec: u8) -> String {
        match &expr.kind {
            ExprKind::Name(name) => name.text.clone(),
            ExprKind::Integer(integer) => integer.raw.clone(),
            ExprKind::Float(float) => float.raw.clone(),
            ExprKind::Decimal(decimal) => decimal.raw.clone(),
            ExprKind::BigInt(bigint) => bigint.raw.clone(),
            ExprKind::SuffixedInteger(literal) => self.format_suffixed_integer_inline(literal),
            ExprKind::Text(text) => self.format_text_literal(text),
            ExprKind::Regex(regex) => regex.raw.clone(),
            ExprKind::Group(inner) => format!("({})", self.format_expr_inline(inner, 0)),
            ExprKind::Tuple(elements) => self.format_expr_tuple_inline(elements),
            ExprKind::List(elements) => self.format_list_inline(elements),
            ExprKind::Map(map) => self.format_map_inline(map),
            ExprKind::Set(elements) => self.format_set_inline(elements),
            ExprKind::Record(record) => self.format_record_inline(record),
            ExprKind::SubjectPlaceholder => ".".to_owned(),
            ExprKind::AmbientProjection(path) => self.format_projection_path(path),
            ExprKind::Range { start, end } => wrap_if_needed(
                format!(
                    "{}..{}",
                    self.format_expr_inline(start, EXPR_RANGE_PREC + 1),
                    self.format_expr_inline(end, EXPR_RANGE_PREC + 1)
                ),
                EXPR_RANGE_PREC,
                parent_prec,
            ),
            ExprKind::Projection { base, path } => wrap_if_needed(
                format!(
                    "{}{}",
                    self.format_expr_inline(base, EXPR_PROJECTION_PREC),
                    self.format_projection_path(path)
                ),
                EXPR_PROJECTION_PREC,
                parent_prec,
            ),
            ExprKind::Apply { callee, arguments } => {
                let mut rendered = self.format_expr_inline(callee, EXPR_APPLY_PREC);
                for argument in arguments {
                    rendered.push(' ');
                    rendered.push_str(&self.format_expr_inline(argument, EXPR_APPLY_PREC + 1));
                }
                wrap_if_needed(rendered, EXPR_APPLY_PREC, parent_prec)
            }
            ExprKind::Unary { operator, expr } => wrap_if_needed(
                format!(
                    "{} {}",
                    self.format_unary_operator(*operator),
                    self.format_expr_inline(expr, EXPR_PREFIX_PREC)
                ),
                EXPR_PREFIX_PREC,
                parent_prec,
            ),
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let precedence = self.binary_precedence(*operator);
                let rendered = format!(
                    "{} {} {}",
                    self.format_expr_inline(left, precedence),
                    self.format_binary_operator(*operator),
                    self.format_expr_inline(right, precedence + 1)
                );
                wrap_if_needed(rendered, precedence, parent_prec)
            }
            ExprKind::Pipe(pipe) => {
                wrap_if_needed(self.format_pipe_inline(pipe), EXPR_PIPE_PREC, parent_prec)
            }
            ExprKind::Markup(node) => self.format_markup_inline(node),
        }
    }

    fn format_suffixed_integer_inline(&self, literal: &SuffixedIntegerLiteral) -> String {
        format!("{}{}", literal.literal.raw, literal.suffix.text)
    }

    fn format_expr_group_block(&self, inner: &Expr, force_multiline: bool) -> Block {
        let inline = format!("({})", self.format_expr_inline(inner, 0));
        if !force_multiline {
            return Block::inline(inline);
        }

        let block = self.format_expr_block(inner, true);
        if block.is_inline() {
            Block::inline(inline)
        } else {
            let mut lines = vec!["(".to_owned()];
            lines.extend(block.indented(INDENT_WIDTH).into_lines());
            lines.push(")".to_owned());
            Block::from_lines(lines)
        }
    }

    fn format_expr_tuple_block(&self, elements: &[Expr], force_multiline: bool) -> Block {
        let inline = self.format_expr_tuple_inline(elements);
        if elements.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["(".to_owned()];
        for (index, element) in elements.iter().enumerate() {
            let suffix = if index + 1 < elements.len() || elements.len() == 1 {
                ","
            } else {
                ""
            };
            lines.extend(
                self.format_expr_block(element, false)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push(")".to_owned());
        Block::from_lines(lines)
    }

    fn format_expr_tuple_inline(&self, elements: &[Expr]) -> String {
        format_tuple_like(
            elements
                .iter()
                .map(|element| self.format_expr_inline(element, 0))
                .collect(),
        )
    }

    fn format_list_block(&self, elements: &[Expr], force_multiline: bool) -> Block {
        let inline = self.format_list_inline(elements);
        if elements.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["[".to_owned()];
        for (index, element) in elements.iter().enumerate() {
            let suffix = if index + 1 < elements.len() { "," } else { "" };
            lines.extend(
                self.format_expr_block(element, false)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push("]".to_owned());
        Block::from_lines(lines)
    }

    fn format_list_inline(&self, elements: &[Expr]) -> String {
        if elements.is_empty() {
            "[]".to_owned()
        } else {
            format!(
                "[{}]",
                elements
                    .iter()
                    .map(|element| self.format_expr_inline(element, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    fn format_set_block(&self, elements: &[Expr], force_multiline: bool) -> Block {
        let inline = self.format_set_inline(elements);
        if elements.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["Set [".to_owned()];
        for (index, element) in elements.iter().enumerate() {
            let suffix = if index + 1 < elements.len() { "," } else { "" };
            lines.extend(
                self.format_expr_block(element, false)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push("]".to_owned());
        Block::from_lines(lines)
    }

    fn format_set_inline(&self, elements: &[Expr]) -> String {
        format_prefixed_list_like(
            "Set",
            elements
                .iter()
                .map(|element| self.format_expr_inline(element, 0))
                .collect(),
        )
    }

    fn format_map_block(&self, map: &MapExpr, force_multiline: bool) -> Block {
        let inline = self.format_map_inline(map);
        if map.entries.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT) {
            return Block::inline(inline);
        }

        let mut lines = vec!["Map {".to_owned()];
        for (index, entry) in map.entries.iter().enumerate() {
            let suffix = if index + 1 < map.entries.len() {
                ","
            } else {
                ""
            };
            lines.extend(
                self.format_map_entry_block(entry)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push("}".to_owned());
        Block::from_lines(lines)
    }

    fn format_map_inline(&self, map: &MapExpr) -> String {
        format_prefixed_record_like(
            "Map",
            map.entries
                .iter()
                .map(|entry| self.format_map_entry_inline(entry))
                .collect(),
        )
    }

    fn format_map_entry_block(&self, entry: &crate::cst::MapExprEntry) -> Block {
        let key = self.format_expr_inline(&entry.key, 0);
        let block = self.format_expr_block(&entry.value, false);
        if block.is_inline() {
            Block::inline(format!(
                "{key}: {}",
                block.inline_text().expect("inline block")
            ))
        } else {
            block.prefixed(&format!("{key}: "))
        }
    }

    fn format_map_entry_inline(&self, entry: &crate::cst::MapExprEntry) -> String {
        format!(
            "{}: {}",
            self.format_expr_inline(&entry.key, 0),
            self.format_expr_inline(&entry.value, 0)
        )
    }

    fn format_record_block(&self, record: &RecordExpr, force_multiline: bool) -> Block {
        let inline = self.format_record_inline(record);
        if record.fields.is_empty() || (!force_multiline && display_width(&inline) <= INLINE_LIMIT)
        {
            return Block::inline(inline);
        }

        let mut lines = vec!["{".to_owned()];
        for (index, field) in record.fields.iter().enumerate() {
            let suffix = if index + 1 < record.fields.len() {
                ","
            } else {
                ""
            };
            lines.extend(
                self.format_record_field_block(field)
                    .with_suffix_on_last_line(suffix)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push("}".to_owned());
        Block::from_lines(lines)
    }

    fn format_record_inline(&self, record: &RecordExpr) -> String {
        format_record_like(
            record
                .fields
                .iter()
                .map(|field| self.format_record_field_inline(field))
                .collect(),
        )
    }

    fn format_record_field_block(&self, field: &RecordField) -> Block {
        match &field.value {
            Some(value) => {
                let block = self.format_expr_block(value, false);
                if block.is_inline() {
                    Block::inline(format!(
                        "{}: {}",
                        field.label.text,
                        block.inline_text().expect("inline block")
                    ))
                } else {
                    block.prefixed(&format!("{}: ", field.label.text))
                }
            }
            None => Block::inline(field.label.text.clone()),
        }
    }

    fn format_record_field_inline(&self, field: &RecordField) -> String {
        match &field.value {
            Some(value) => format!(
                "{}: {}",
                field.label.text,
                self.format_expr_inline(value, 0)
            ),
            None => field.label.text.clone(),
        }
    }

    fn format_expr_apply_block(
        &self,
        callee: &Expr,
        arguments: &[Expr],
        force_multiline: bool,
    ) -> Block {
        let inline = self.format_expr_apply_inline(callee, arguments);
        if !force_multiline && display_width(&inline) <= INLINE_LIMIT {
            return Block::inline(inline);
        }

        let mut prefix = self.format_expr_inline(callee, EXPR_APPLY_PREC);
        for (index, argument) in arguments.iter().enumerate() {
            let is_last = index + 1 == arguments.len();
            let block = self.format_expr_block(
                argument,
                force_multiline && is_last && self.expr_can_break(argument),
            );
            if block.is_inline() {
                prefix.push(' ');
                prefix.push_str(block.inline_text().expect("inline block"));
                continue;
            }
            if is_last {
                return block.prefixed(&format!("{prefix} "));
            }
            return Block::inline(inline);
        }

        Block::inline(prefix)
    }

    fn format_expr_apply_inline(&self, callee: &Expr, arguments: &[Expr]) -> String {
        let mut rendered = self.format_expr_inline(callee, EXPR_APPLY_PREC);
        for argument in arguments {
            rendered.push(' ');
            rendered.push_str(&self.format_expr_inline(argument, EXPR_APPLY_PREC + 1));
        }
        rendered
    }

    fn format_pipe_block(&self, pipe: &PipeExpr) -> Block {
        let mut lines = Vec::new();
        if let Some(head) = &pipe.head {
            lines.push(self.format_expr_inline(head, 0));
        }

        let mut index = 0usize;
        while index < pipe.stages.len() {
            if matches!(pipe.stages[index].kind, PipeStageKind::Case(_)) {
                let start = index;
                while index < pipe.stages.len()
                    && matches!(pipe.stages[index].kind, PipeStageKind::Case(_))
                {
                    index += 1;
                }
                lines.extend(self.format_pipe_case_group(&pipe.stages[start..index]));
            } else {
                lines.push(self.format_pipe_stage_line(&pipe.stages[index]));
                index += 1;
            }
        }

        Block::from_lines(lines)
    }

    fn format_pipe_inline(&self, pipe: &PipeExpr) -> String {
        let mut parts = Vec::new();
        if let Some(head) = &pipe.head {
            parts.push(self.format_expr_inline(head, 0));
        }
        for stage in &pipe.stages {
            parts.push(self.format_pipe_stage_inline(stage));
        }
        parts.join(" ")
    }

    fn format_pipe_stage_inline(&self, stage: &PipeStage) -> String {
        match &stage.kind {
            PipeStageKind::Case(arm) => format!(
                "||> {} => {}",
                self.format_pattern_inline(&arm.pattern, 0),
                self.format_expr_inline(&arm.body, 0)
            ),
            PipeStageKind::Transform { expr } => self.format_pipe_expr_stage("|>", expr),
            PipeStageKind::Gate { expr } => self.format_pipe_expr_stage("?|>", expr),
            PipeStageKind::Map { expr } => self.format_pipe_expr_stage("*|>", expr),
            PipeStageKind::Apply { expr } => self.format_pipe_expr_stage("&|>", expr),
            PipeStageKind::ClusterFinalizer { expr } => self.format_pipe_expr_stage("|>", expr),
            PipeStageKind::RecurStart { expr } => self.format_pipe_expr_stage("@|>", expr),
            PipeStageKind::RecurStep { expr } => self.format_pipe_expr_stage("<|@", expr),
            PipeStageKind::Tap { expr } => self.format_pipe_expr_stage("|", expr),
            PipeStageKind::FanIn { expr } => self.format_pipe_expr_stage("<|*", expr),
            PipeStageKind::Truthy { expr } => self.format_pipe_expr_stage("T|>", expr),
            PipeStageKind::Falsy { expr } => self.format_pipe_expr_stage("F|>", expr),
        }
    }

    fn format_pipe_stage_line(&self, stage: &PipeStage) -> String {
        match &stage.kind {
            PipeStageKind::Case(_) => {
                todo!("format PipeStageKind::Case as a standalone stage line")
            }
            PipeStageKind::Transform { expr } => self.format_aligned_pipe_stage("|>", expr),
            PipeStageKind::Gate { expr } => self.format_aligned_pipe_stage("?|>", expr),
            PipeStageKind::Map { expr } => self.format_aligned_pipe_stage("*|>", expr),
            PipeStageKind::Apply { expr } => self.format_aligned_pipe_stage("&|>", expr),
            PipeStageKind::ClusterFinalizer { expr } => self.format_aligned_pipe_stage("|>", expr),
            PipeStageKind::RecurStart { expr } => self.format_aligned_pipe_stage("@|>", expr),
            PipeStageKind::RecurStep { expr } => self.format_aligned_pipe_stage("<|@", expr),
            PipeStageKind::Tap { expr } => self.format_aligned_pipe_stage("|", expr),
            PipeStageKind::FanIn { expr } => self.format_aligned_pipe_stage("<|*", expr),
            PipeStageKind::Truthy { expr } => self.format_aligned_pipe_stage("T|>", expr),
            PipeStageKind::Falsy { expr } => self.format_aligned_pipe_stage("F|>", expr),
        }
    }

    fn format_pipe_case_group(&self, stages: &[PipeStage]) -> Vec<String> {
        let patterns: Vec<_> = stages
            .iter()
            .map(|stage| match &stage.kind {
                PipeStageKind::Case(arm) => self.format_pattern_inline(&arm.pattern, 0),
                _ => "// <error: unformattable node>".to_owned(),
            })
            .collect();
        let width = patterns
            .iter()
            .map(|pattern| display_width(pattern))
            .max()
            .unwrap_or(0);

        stages
            .iter()
            .zip(patterns)
            .map(|(stage, pattern)| match &stage.kind {
                PipeStageKind::Case(arm) => {
                    let padding = spaces(width.saturating_sub(display_width(&pattern)));
                    format!(
                        " ||> {pattern}{padding} => {}",
                        self.format_expr_inline(&arm.body, 0)
                    )
                }
                _ => "// <error: unformattable node>".to_owned(),
            })
            .collect()
    }

    fn format_pipe_expr_stage(&self, operator: &str, expr: &Expr) -> String {
        format!("{operator} {}", self.format_expr_inline(expr, 0))
    }

    fn format_aligned_pipe_stage(&self, operator: &str, expr: &Expr) -> String {
        format!(
            "{}{} {}",
            self.pipe_alignment_prefix(operator),
            operator,
            self.format_expr_inline(expr, 0)
        )
    }

    fn pipe_alignment_prefix(&self, _operator: &str) -> &'static str {
        " "
    }

    fn format_markup_block(&self, node: &MarkupNode) -> Block {
        if node.self_closing || node.children.is_empty() {
            return Block::inline(self.format_markup_inline(node));
        }

        let mut lines = vec![self.format_markup_open_tag(node, false)];
        for child in &node.children {
            lines.extend(
                self.format_markup_block(child)
                    .indented(INDENT_WIDTH)
                    .into_lines(),
            );
        }
        lines.push(self.format_markup_close_tag(node));
        Block::from_lines(lines)
    }

    fn format_markup_inline(&self, node: &MarkupNode) -> String {
        if node.self_closing {
            return self.format_markup_open_tag(node, true);
        }

        let open = self.format_markup_open_tag(node, false);
        let close = self.format_markup_close_tag(node);
        if node.children.is_empty() {
            format!("{open}{close}")
        } else {
            let children = node
                .children
                .iter()
                .map(|child| self.format_markup_inline(child))
                .collect::<String>();
            format!("{open}{children}{close}")
        }
    }

    fn format_markup_open_tag(&self, node: &MarkupNode, self_closing: bool) -> String {
        let mut rendered = format!("<{}", self.format_qualified_name(&node.name));
        for attribute in &node.attributes {
            rendered.push(' ');
            rendered.push_str(&self.format_markup_attribute(attribute));
        }
        if self_closing {
            rendered.push_str(" />");
        } else {
            rendered.push('>');
        }
        rendered
    }

    fn format_markup_close_tag(&self, node: &MarkupNode) -> String {
        let name = node.close_name.as_ref().unwrap_or(&node.name);
        format!("</{}>", self.format_qualified_name(name))
    }

    fn format_markup_attribute(&self, attribute: &MarkupAttribute) -> String {
        match &attribute.value {
            Some(MarkupAttributeValue::Text(text)) => {
                format!("{}={}", attribute.name.text, self.format_text_literal(text))
            }
            Some(MarkupAttributeValue::Expr(expr)) => format!(
                "{}={{{}}}",
                attribute.name.text,
                self.format_expr_inline(expr, 0)
            ),
            Some(MarkupAttributeValue::Pattern(pattern)) => format!(
                "{}={{{}}}",
                attribute.name.text,
                self.format_pattern_inline(pattern, 0)
            ),
            None => attribute.name.text.clone(),
        }
    }

    fn format_pattern_inline(&self, pattern: &Pattern, parent_prec: u8) -> String {
        match &pattern.kind {
            PatternKind::Wildcard => "_".to_owned(),
            PatternKind::Name(name) => name.text.clone(),
            PatternKind::Integer(integer) => integer.raw.clone(),
            PatternKind::Text(text) => self.format_text_literal(text),
            PatternKind::Group(inner) => format!("({})", self.format_pattern_inline(inner, 0)),
            PatternKind::Tuple(elements) => self.format_pattern_tuple_inline(elements),
            PatternKind::List { elements, rest } => {
                self.format_pattern_list_inline(elements, rest.as_deref())
            }
            PatternKind::Record(fields) => self.format_pattern_record_inline(fields),
            PatternKind::Apply { callee, arguments } => {
                let mut rendered = self.format_pattern_inline(callee, PATTERN_APPLY_PREC);
                for argument in arguments {
                    rendered.push(' ');
                    rendered
                        .push_str(&self.format_pattern_inline(argument, PATTERN_APPLY_PREC + 1));
                }
                wrap_if_needed(rendered, PATTERN_APPLY_PREC, parent_prec)
            }
        }
    }

    fn format_text_literal(&self, text: &TextLiteral) -> String {
        let mut rendered = String::from("\"");
        for segment in &text.segments {
            match segment {
                TextSegment::Text(fragment) => rendered.push_str(&fragment.raw),
                TextSegment::Interpolation(interpolation) => {
                    rendered.push('{');
                    rendered.push_str(&self.format_expr_inline(&interpolation.expr, 0));
                    rendered.push('}');
                }
            }
        }
        rendered.push('"');
        rendered
    }

    fn format_pattern_tuple_inline(&self, elements: &[Pattern]) -> String {
        format_tuple_like(
            elements
                .iter()
                .map(|element| self.format_pattern_inline(element, 0))
                .collect(),
        )
    }

    fn format_pattern_list_inline(&self, elements: &[Pattern], rest: Option<&Pattern>) -> String {
        if elements.is_empty() && rest.is_none() {
            return "[]".to_owned();
        }

        let mut parts = elements
            .iter()
            .map(|element| self.format_pattern_inline(element, 0))
            .collect::<Vec<_>>();
        if let Some(rest) = rest {
            parts.push(format!("...{}", self.format_pattern_inline(rest, 0)));
        }
        format!("[{}]", parts.join(", "))
    }

    fn format_pattern_record_inline(&self, fields: &[RecordPatternField]) -> String {
        format_record_like(
            fields
                .iter()
                .map(|field| self.format_pattern_field_inline(field))
                .collect(),
        )
    }

    fn format_pattern_field_inline(&self, field: &RecordPatternField) -> String {
        match &field.pattern {
            Some(pattern) => format!(
                "{}: {}",
                field.label.text,
                self.format_pattern_inline(pattern, 0)
            ),
            None => field.label.text.clone(),
        }
    }

    fn format_projection_path(&self, path: &ProjectionPath) -> String {
        let mut rendered = String::new();
        for field in &path.fields {
            rendered.push('.');
            rendered.push_str(&field.text);
        }
        rendered
    }

    fn format_qualified_name(&self, name: &QualifiedName) -> String {
        name.as_dotted()
    }

    fn format_class_member_name(&self, name: &ClassMemberName) -> String {
        match name {
            ClassMemberName::Identifier(identifier) => identifier.text.clone(),
            ClassMemberName::Operator(operator) => format!("({})", operator.text),
        }
    }

    fn format_domain_member_name(&self, name: &DomainMemberName) -> String {
        match name {
            DomainMemberName::Signature(name) => self.format_class_member_name(name),
            DomainMemberName::Literal(identifier) => format!("literal {}", identifier.text),
        }
    }

    fn item_name<'a>(&self, name: &'a Option<Identifier>) -> &'a str {
        name.as_ref().map(|name| name.text.as_str()).unwrap_or("_")
    }

    fn format_unary_operator(&self, operator: UnaryOperator) -> &'static str {
        match operator {
            UnaryOperator::Not => "not",
        }
    }

    fn format_binary_operator(&self, operator: BinaryOperator) -> &'static str {
        match operator {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::GreaterThan => ">",
            BinaryOperator::LessThan => "<",
            BinaryOperator::Equals => "==",
            BinaryOperator::NotEquals => "!=",
            BinaryOperator::And => "and",
            BinaryOperator::Or => "or",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Modulo => "%",
        }
    }

    fn binary_precedence(&self, operator: BinaryOperator) -> u8 {
        match operator {
            BinaryOperator::Or => EXPR_OR_PREC,
            BinaryOperator::And => EXPR_AND_PREC,
            BinaryOperator::GreaterThan
            | BinaryOperator::LessThan
            | BinaryOperator::Equals
            | BinaryOperator::NotEquals => EXPR_COMPARE_PREC,
            BinaryOperator::Add | BinaryOperator::Subtract => EXPR_ADD_PREC,
            BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo => {
                EXPR_MUL_PREC
            }
        }
    }

    fn should_force_expr_break(&self, prefix_width: usize, expr: &Expr) -> bool {
        self.expr_can_break(expr)
            && prefix_width + display_width(&self.format_expr_inline(expr, 0)) > INLINE_LIMIT
    }

    fn should_force_type_break(&self, prefix_width: usize, ty: &TypeExpr) -> bool {
        self.type_can_break(ty)
            && prefix_width + display_width(&self.format_type_inline(ty, 0)) > INLINE_LIMIT
    }

    fn should_force_record_break(&self, prefix_width: usize, record: &RecordExpr) -> bool {
        !record.fields.is_empty()
            && prefix_width + display_width(&self.format_record_inline(record)) > INLINE_LIMIT
    }

    fn expr_can_break(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Tuple(elements) => !elements.is_empty(),
            ExprKind::List(elements) => !elements.is_empty(),
            ExprKind::Map(map) => !map.entries.is_empty(),
            ExprKind::Set(elements) => !elements.is_empty(),
            ExprKind::Record(record) => !record.fields.is_empty(),
            ExprKind::Pipe(_) | ExprKind::Markup(_) => true,
            ExprKind::Range { start, end } => {
                self.expr_can_break(start) || self.expr_can_break(end)
            }
            ExprKind::Apply {
                callee: _,
                arguments,
            } => arguments
                .iter()
                .any(|argument| self.expr_can_break(argument)),
            ExprKind::Group(inner) => self.expr_can_break(inner),
            _ => false,
        }
    }

    fn type_can_break(&self, ty: &TypeExpr) -> bool {
        match &ty.kind {
            TypeExprKind::Tuple(elements) => !elements.is_empty(),
            TypeExprKind::Record(fields) => !fields.is_empty(),
            TypeExprKind::Arrow { parameter, result } => {
                self.type_can_break(parameter) || self.type_can_break(result)
            }
            TypeExprKind::Apply {
                callee: _,
                arguments,
            } => arguments
                .iter()
                .any(|argument| self.type_can_break(argument)),
            TypeExprKind::Group(inner) => self.type_can_break(inner),
            TypeExprKind::Name(_) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Block {
    lines: Vec<String>,
}

impl Block {
    fn inline(text: impl Into<String>) -> Self {
        Self {
            lines: vec![text.into()],
        }
    }

    fn from_lines(lines: Vec<String>) -> Self {
        debug_assert!(!lines.is_empty(), "blocks must contain at least one line");
        Self { lines }
    }

    fn is_inline(&self) -> bool {
        self.lines.len() == 1
    }

    fn inline_text(&self) -> Option<&str> {
        self.is_inline().then(|| self.lines[0].as_str())
    }

    fn starts_with_delimiter(&self) -> bool {
        self.lines
            .first()
            .and_then(|line| line.chars().next())
            .is_some_and(|character| matches!(character, '{' | '[' | '('))
    }

    fn prefixed(mut self, prefix: &str) -> Self {
        if let Some(first) = self.lines.first_mut() {
            *first = format!("{prefix}{first}");
        }
        self
    }

    fn indented(mut self, spaces_count: usize) -> Self {
        let prefix = spaces(spaces_count);
        for line in &mut self.lines {
            if !line.is_empty() {
                *line = format!("{prefix}{line}");
            }
        }
        self
    }

    fn with_suffix_on_last_line(mut self, suffix: &str) -> Self {
        if let Some(last) = self.lines.last_mut() {
            last.push_str(suffix);
        }
        self
    }

    fn into_lines(self) -> Vec<String> {
        self.lines
    }
}

fn wrap_if_needed(rendered: String, current_prec: u8, parent_prec: u8) -> String {
    if current_prec < parent_prec {
        format!("({rendered})")
    } else {
        rendered
    }
}

fn spaces(count: usize) -> String {
    " ".repeat(count)
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn format_tuple_like(elements: Vec<String>) -> String {
    match elements.as_slice() {
        [] => "()".to_owned(),
        [element] => format!("({element},)"),
        _ => format!("({})", elements.join(", ")),
    }
}

fn format_record_like(fields: Vec<String>) -> String {
    if fields.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

fn format_prefixed_list_like(prefix: &str, elements: Vec<String>) -> String {
    if elements.is_empty() {
        format!("{prefix} []")
    } else {
        format!("{prefix} [{}]", elements.join(", "))
    }
}

fn format_prefixed_record_like(prefix: &str, fields: Vec<String>) -> String {
    if fields.is_empty() {
        format!("{prefix} {{}}")
    } else {
        format!("{prefix} {{ {} }}", fields.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;

    use super::Formatter;
    use crate::parse::parse_module;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/frontend/milestone-1")
    }

    fn format_text(input: &str) -> String {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("test.aivi", input.to_owned());
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "expected formatter test input to parse cleanly, got diagnostics: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        Formatter.format(&parsed.module)
    }

    fn format_fixture(relative_path: &str) -> String {
        let path = fixture_root().join(relative_path);
        let input = fs::read_to_string(&path).expect("fixture must load");
        format_text(&input)
    }

    #[test]
    fn formatter_normalizes_pipe_alignment_fixture() {
        let formatted = format_fixture("valid/formatting/pipe_alignment.aivi");
        assert_eq!(
            formatted,
            concat!(
                "type Pair = Pair Text Text\n",
                "\n",
                "type SaveState =\n",
                "  | Saved\n",
                "  | Dirty Text\n",
                "\n",
                "sig documentTitle = \"Notes\"\n",
                "sig documentBody = \"Hello\"\n",
                "\n",
                "sig draft =\n",
                "  &|> documentTitle\n",
                "  &|> documentBody\n",
                "  |> Pair\n",
                "\n",
                "fun label:Text state:SaveState =>\n",
                "    state\n",
                "     ||> Saved         => \"saved\"\n",
                "     ||> Dirty message => \"dirty {message}\"\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_markup_layout_fixture() {
        let formatted = format_fixture("valid/formatting/markup_layout.aivi");
        assert_eq!(
            formatted,
            concat!(
                "fun formatCount:Text count:Int =>\n",
                "    \"{count} unread\"\n",
                "\n",
                "val count = 3\n",
                "\n",
                "val dashboard =\n",
                "    <fragment>\n",
                "        <Label text=\"Inbox\" />\n",
                "        <show when={True} keepMounted={True}>\n",
                "            <with value={formatCount count} as={label}>\n",
                "                <Label text={label} />\n",
                "            </with>\n",
                "        </show>\n",
                "    </fragment>\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_class_equality_layout_fixture() {
        let formatted = format_fixture("valid/formatting/class_eq_layout.aivi");
        assert_eq!(
            formatted,
            concat!(
                "class Eq A\n",
                "    (==): A -> A -> Bool\n",
                "\n",
                "fun equivalent:Bool left:Int right:Int =>\n",
                "    left + 1 == right - 1 and left != right\n",
            )
        );
    }

    #[test]
    fn formatter_preserves_qualified_markup_tag_names() {
        let formatted = format_text(
            r#"
val view =
    <Window>
        <Paned.start>
            <Label />
        </Paned.start>
    </Window>
"#,
        );
        assert_eq!(
            formatted,
            concat!(
                "val view =\n",
                "    <Window>\n",
                "        <Paned.start>\n",
                "            <Label />\n",
                "        </Paned.start>\n",
                "    </Window>\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_domain_layout_fixture() {
        let formatted = format_fixture("valid/formatting/domain_layout.aivi");
        assert_eq!(
            formatted,
            concat!(
                "domain Duration over Int\n",
                "    literal ms: Int -> Duration\n",
                "    (*): Duration -> Int -> Duration\n",
                "    value: Duration -> Int\n",
                "\n",
                "domain Path over Text\n",
                "    literal root: Text -> Path\n",
                "    (/): Path -> Text -> Path\n",
                "    value: Path -> Text\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_instance_layout() {
        let formatted = format_text(
            "class Eq A\n    (==):A -> A -> Bool\n\ninstance Eq Blob\n    (==) left right=\n        same left right\n",
        );
        assert_eq!(
            formatted,
            concat!(
                "class Eq A\n",
                "    (==): A -> A -> Bool\n",
                "\n",
                "instance Eq Blob\n",
                "    (==) left right = same left right\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_provider_contract_layout_fixture() {
        let formatted = format_fixture("valid/formatting/provider_contract_layout.aivi");
        assert_eq!(
            formatted,
            concat!(
                "provider custom.feed\n",
                "    argument path: Text\n",
                "    option timeout: Duration\n",
                "    wakeup: providerTrigger\n",
                "\n",
                "provider custom.timer\n",
                "    option activeWhen: Signal Bool\n",
                "    wakeup: timer\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_multiplicative_operator_layout() {
        let formatted = format_text("val total=left+middle*right/scale%modulo\n");
        assert_eq!(
            formatted,
            "val total = left + middle * right / scale % modulo\n"
        );
    }

    #[test]
    fn formatter_normalizes_percent_domain_operator_layout() {
        let formatted = format_text("domain Bucket over Int\n    (%):Bucket -> Int -> Bucket\n");
        assert_eq!(
            formatted,
            concat!(
                "domain Bucket over Int\n",
                "    (%): Bucket -> Int -> Bucket\n",
            )
        );
    }

    #[test]
    fn formatter_keeps_compact_domain_literal_suffixes() {
        let formatted = format_text(
            "domain Duration over Int\n    literal ms:Int -> Duration\nval delay:Duration=250ms\nval applied=wrap 250ms\n",
        );
        assert_eq!(
            formatted,
            concat!(
                "domain Duration over Int\n",
                "    literal ms: Int -> Duration\n",
                "\n",
                "val delay: Duration = 250ms\n",
                "val applied = wrap 250ms\n",
            )
        );
    }

    #[test]
    fn formatter_keeps_builtin_noninteger_literals() {
        let formatted = format_text(
            "val pi:Float=3.14\nval amount:Decimal=19.25d\nval whole:Decimal=19d\nval count:BigInt=123n\n",
        );
        assert_eq!(
            formatted,
            concat!(
                "val pi: Float = 3.14\n",
                "val amount: Decimal = 19.25d\n",
                "val whole: Decimal = 19d\n",
                "val count: BigInt = 123n\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_map_and_set_literals() {
        let formatted = format_text(
            "val headers=Map{\"Authorization\":\"Bearer demo\",\"Accept\":\"application/json\"}\nval tags=Set[1,2,4]\n",
        );
        assert_eq!(
            formatted,
            concat!(
                "val headers =\n",
                "    Map {\n",
                "        \"Authorization\": \"Bearer demo\",\n",
                "        \"Accept\": \"application/json\"\n",
                "    }\n",
                "\n",
                "val tags = Set [1, 2, 4]\n",
            )
        );
    }

    #[test]
    fn formatter_aligns_match_arms_and_top_level_spacing() {
        let formatted = format_text(
            "type Status=Idle|Failed Text\nfun label:Text status:Status =>\nstatus||>Idle=>\"idle\"||>Failed reason=>\"failed {reason}\"\n",
        );
        assert_eq!(
            formatted,
            concat!(
                "type Status = Idle | Failed Text\n",
                "\n",
                "fun label:Text status:Status =>\n",
                "    status\n",
                "     ||> Idle          => \"idle\"\n",
                "     ||> Failed reason => \"failed {reason}\"\n",
            )
        );
    }

    #[test]
    fn formatter_preserves_subject_placeholders_ranges_and_discard_params() {
        let formatted =
            format_text("fun ignore:Int _=>.\nval projection=.email\nval values=[1..10]\n");
        assert_eq!(
            formatted,
            concat!(
                "fun ignore:Int _ =>\n",
                "    .\n",
                "\n",
                "val projection = .email\n",
                "val values = [1..10]\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_text_interpolation_holes() {
        let formatted = format_text("val greeting=\"Hello { name }, use \\{literal\\} braces\"\n");
        assert_eq!(
            formatted,
            "val greeting = \"Hello {name}, use \\{literal\\} braces\"\n",
        );
    }

    #[test]
    fn formatter_normalizes_use_imports_and_records() {
        let formatted =
            format_text("use aivi.network(http,socket)\nval profile:Profile={name,nickname}\n");
        assert_eq!(
            formatted,
            concat!(
                "use aivi.network (\n",
                "    http\n",
                "    socket\n",
                ")\n",
                "\n",
                "val profile: Profile = {\n",
                "    name,\n",
                "    nickname\n",
                "}\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_use_import_aliases() {
        let formatted = format_text("use aivi.network(http as primary,Request as HttpRequest)\n");
        assert_eq!(
            formatted,
            concat!(
                "use aivi.network (\n",
                "    http as primary\n",
                "    Request as HttpRequest\n",
                ")\n",
            )
        );
    }

    #[test]
    fn formatter_normalizes_grouped_exports() {
        let formatted =
            format_text("export(bundledSupportSentinel,BundledSupportToken)\nexport (main)\n");
        assert_eq!(
            formatted,
            concat!(
                "export (bundledSupportSentinel, BundledSupportToken)\n",
                "export main\n",
            )
        );
    }

    #[test]
    fn formatter_is_idempotent_across_valid_fixture_corpus() {
        let valid_root = fixture_root().join("valid");
        let mut stack = vec![valid_root];
        let mut fixtures = Vec::new();
        while let Some(path) = stack.pop() {
            for entry in fs::read_dir(path).expect("valid fixture directory must be readable") {
                let entry = entry.expect("fixture dir entry must load");
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("aivi") {
                    fixtures.push(path);
                }
            }
        }
        fixtures.sort();

        for fixture in fixtures {
            let text = fs::read_to_string(&fixture).expect("fixture text must load");
            let formatted = format_text(&text);
            let reparsed = {
                let mut sources = SourceDatabase::new();
                let file_id = sources.add_file(&fixture, formatted.clone());
                parse_module(&sources[file_id])
            };
            assert!(
                !reparsed.has_errors(),
                "formatted fixture {} should parse cleanly, got diagnostics: {:?}",
                fixture.display(),
                reparsed.all_diagnostics().collect::<Vec<_>>()
            );
            let reformatted = Formatter.format(&reparsed.module);
            assert_eq!(
                reformatted,
                formatted,
                "formatter output should be idempotent for {}",
                fixture.display()
            );
        }
    }
}
