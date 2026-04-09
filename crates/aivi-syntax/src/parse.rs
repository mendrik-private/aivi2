use aivi_base::{Diagnostic, Severity, SourceFile, SourceSpan, Span};

use crate::{
    cst::{
        BigIntLiteral, BinaryOperator, ClassBody, ClassMember, ClassMemberName, ClassRequireDecl,
        ClassWithDecl, DecimalLiteral, Decorator, DecoratorArguments, DecoratorPayload, DomainBody,
        DomainItem, DomainMember, DomainMemberName, ErrorItem, ExportItem, Expr, ExprKind,
        FloatLiteral, FromEntry, FromItem, FunctionParam, FunctionSurfaceForm, Identifier,
        InstanceBody, InstanceItem, InstanceMember, IntegerLiteral, Item, ItemBase, MapExpr,
        MapExprEntry, MarkupAttribute, MarkupAttributeValue, MarkupNode, Module, NamedItem,
        NamedItemBody, OperatorName, PatchBlock, PatchEntry, PatchInstruction,
        PatchInstructionKind, PatchSelector, PatchSelectorSegment, Pattern, PatternKind,
        PipeCaseArm, PipeExpr, PipeStage, PipeStageKind, ProjectionPath, QualifiedName, RecordExpr,
        RecordField, RecordPatternField, RegexLiteral, ResultBinding, ResultBlockExpr,
        SignalMergeBody, SignalReactiveArm, SourceDecorator, SourceProviderContractBody,
        SourceProviderContractFieldValue, SourceProviderContractItem, SourceProviderContractMember,
        SourceProviderContractSchemaMember, SuffixedIntegerLiteral, TextFragment,
        TextInterpolation, TextLiteral, TextSegment, TokenRange, TypeCompanionMember, TypeDeclBody,
        TypeExpr, TypeExprKind, TypeField, TypeSumBody, TypeVariant, TypeVariantField,
        UnaryOperator, UseImport, UseItem,
    },
    lex::{LexedModule, Token, TokenKind, lex_fragment, lex_module},
};

use crate::codes::*;

const MAX_PARSE_DEPTH: usize = 256;
const IMPLICIT_FUNCTION_SUBJECT_NAME: &str = "arg1";

#[derive(Clone, Debug)]
struct SubjectPickHead {
    expr: Expr,
    start_index: usize,
}

/// Parser output retaining the lossless token buffer and recoverable diagnostics.
#[derive(Clone, Debug)]
pub struct ParsedModule {
    pub lexed: LexedModule,
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedModule {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn all_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.lexed
            .diagnostics()
            .iter()
            .chain(self.diagnostics.iter())
    }

    pub fn has_errors(&self) -> bool {
        self.all_diagnostics()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

pub fn parse_module(source: &SourceFile) -> ParsedModule {
    let lexed = lex_module(source);
    let parser = Parser::new(source, lexed.tokens());
    let (module, diagnostics) = parser.parse();
    ParsedModule {
        lexed,
        module,
        diagnostics,
    }
}

struct Parser<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    cursor: usize,
    diagnostics: Vec<Diagnostic>,
    depth: usize,
}

#[derive(Clone, Debug)]
struct PendingTypeAnnotation {
    span: SourceSpan,
    constraints: Vec<TypeExpr>,
    annotation: TypeExpr,
}

impl<'a> Parser<'a> {
    fn new(source: &'a SourceFile, tokens: &'a [Token]) -> Self {
        Self {
            source,
            tokens,
            cursor: 0,
            diagnostics: Vec::new(),
            depth: 0,
        }
    }

    /// Attempt to enter a recursive parse frame. Returns `true` if the caller
    /// may proceed; returns `false` (and emits a diagnostic) when the nesting
    /// limit has been reached.  The caller **must** call `depth_exit` exactly
    /// once after a successful `depth_enter`.
    fn depth_enter(&mut self, cursor: &mut usize) -> bool {
        if self.depth >= MAX_PARSE_DEPTH {
            let span = if *cursor < self.tokens.len() {
                self.source_span_of_token(*cursor)
            } else if !self.tokens.is_empty() {
                self.source_span_of_token(self.tokens.len() - 1)
            } else {
                self.source.source_span(0..0)
            };
            self.diagnostics.push(
                Diagnostic::error("expression is nested too deeply to parse")
                    .with_code(PARSE_DEPTH_EXCEEDED)
                    .with_primary_label(span, "maximum parse depth exceeded here")
                    .with_help("refactor deeply nested expressions into smaller named values"),
            );
            return false;
        }
        self.depth += 1;
        true
    }

    fn depth_exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse(mut self) -> (Module, Vec<Diagnostic>) {
        let mut items = Vec::new();
        let mut pending_type_annotation = None;
        // Comments collected for a pending standalone `type` annotation are
        // carried forward and prepended to the following declaration.
        let mut carried_comments: Vec<String> = Vec::new();
        while let Some(start) = self.next_significant_from(self.cursor) {
            let leading_comments = self.collect_leading_comments(self.cursor, start);
            let item = match self.tokens[start].kind() {
                TokenKind::At => self.parse_decorated_item(start),
                kind if kind.is_top_level_keyword() => self.parse_item_without_decorators(start),
                _ => self.parse_error_item(start),
            };
            let next_cursor = item.token_range().end();
            self.cursor = if next_cursor > start {
                next_cursor
            } else {
                start + 1
            };

            if let Some(pending) = Self::pending_type_annotation(&item) {
                if let Some(previous) = pending_type_annotation.replace(pending) {
                    self.emit_orphan_standalone_type_annotation(&previous, None);
                }
                // Carry the comments so they prefix the following declaration.
                if !leading_comments.is_empty() {
                    carried_comments = leading_comments;
                }
                continue;
            }

            let mut item = item;
            if let Some(pending) = pending_type_annotation.take() {
                self.apply_pending_type_annotation(&mut item, pending);
            }
            // Merge carried comments (from a preceding type annotation) with
            // any comments directly before this item, preferring carried first.
            let mut all_comments = std::mem::take(&mut carried_comments);
            all_comments.extend(leading_comments);
            if !all_comments.is_empty() {
                item.base_mut().leading_comments = all_comments;
            }
            items.push(item);
        }

        if let Some(pending) = pending_type_annotation.take() {
            self.emit_orphan_standalone_type_annotation(&pending, None);
        }

        (
            Module {
                file: self.source.id(),
                items,
                token_count: self.tokens.len(),
            },
            self.diagnostics,
        )
    }

    fn parse_decorated_item(&mut self, start: usize) -> Item {
        let search = self.find_declaration_keyword(start);
        let Some(keyword_index) = search.keyword else {
            let end = search
                .offending
                .and_then(|index| self.find_next_item_start(index + 1))
                .unwrap_or(self.tokens.len());
            let base = self.make_base(start, end, Vec::new());
            self.diagnostics.push(
                Diagnostic::error("decorators must attach to a following top-level declaration")
                    .with_code(DANGLING_DECORATOR_BLOCK)
                    .with_primary_label(
                        self.source_span_of_token(start),
                        "expected a following top-level declaration after this decorator block",
                    )
                    .with_help("decorators like @source must be followed by a declaration"),
            );
            return Item::Error(ErrorItem {
                base,
                message: "dangling decorator block".to_owned(),
            });
        };

        let end = self
            .find_next_item_start(keyword_index + 1)
            .unwrap_or(self.tokens.len());
        let decorators = self.parse_decorators(start, keyword_index);
        self.finish_item(keyword_index, start, end, decorators)
    }

    fn parse_item_without_decorators(&mut self, start: usize) -> Item {
        let end = self
            .find_next_item_start(start + 1)
            .unwrap_or(self.tokens.len());
        self.finish_item(start, start, end, Vec::new())
    }

    fn finish_item(
        &mut self,
        keyword_index: usize,
        start: usize,
        end: usize,
        decorators: Vec<Decorator>,
    ) -> Item {
        let base = self.make_base(start, end, decorators);
        match self.tokens[keyword_index].kind() {
            TokenKind::TypeKw => {
                Item::Type(self.parse_type_item(base, keyword_index, end, "type declaration"))
            }
            TokenKind::FuncKw => Item::Fun(self.parse_fun_item(base, keyword_index, end)),
            TokenKind::ValueKw => Item::Value(self.parse_value_item(base, keyword_index, end)),
            TokenKind::SignalKw => {
                Item::Signal(self.parse_signal_item(base, keyword_index, end, "signal declaration"))
            }
            TokenKind::FromKw => Item::From(self.parse_from_item(base, keyword_index, end)),
            TokenKind::ClassKw => Item::Class(self.parse_class_item(base, keyword_index, end)),
            TokenKind::InstanceKw => {
                Item::Instance(self.parse_instance_item(base, keyword_index, end))
            }
            TokenKind::DomainKw => Item::Domain(self.parse_domain_item(base, keyword_index, end)),
            TokenKind::ProviderKw => Item::SourceProviderContract(
                self.parse_source_provider_contract_item(base, keyword_index, end),
            ),
            TokenKind::UseKw => Item::Use(self.parse_use_item(base, keyword_index, end)),
            TokenKind::ExportKw => Item::Export(self.parse_export_item(base, keyword_index, end)),
            TokenKind::HoistKw => Item::Hoist(self.parse_hoist_item(base, keyword_index, end)),
            _ => unreachable!("finish_item only accepts top-level declaration keywords"),
        }
    }

    fn parse_type_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
        description: &str,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        if !self.has_top_level_equals(cursor, end) {
            let (constraints, annotation) = self.parse_constrained_type(&mut cursor, end);
            if annotation.is_none() {
                self.diagnostics.push(
                    Diagnostic::error("standalone `type` annotations require a type expression")
                        .with_code(MISSING_STANDALONE_TYPE_ANNOTATION)
                        .with_primary_label(
                            self.source_span_of_token(keyword_index),
                            "expected a type expression after `type`",
                        )
                        .with_help("expected a type expression after the name"),
                );
            }
            return NamedItem {
                base,
                keyword_span: self.source_span_of_token(keyword_index),
                name: None,
                type_parameters: Vec::new(),
                constraints,
                annotation,
                function_form: FunctionSurfaceForm::Explicit,
                parameters: Vec::new(),
                body: None,
            };
        }

        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, description);
        let mut type_parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(cursor, end) {
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    type_parameters.push(self.identifier_from_token(index));
                    cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_type_decl_body(&mut cursor, end)
                .map(NamedItemBody::Type)
                .or_else(|| {
                    self.missing_body_diagnostic(
                        keyword_index,
                        "type declaration is missing its body after `=`",
                        "expected a type body after `=`",
                    );
                    None
                })
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                "type declaration is missing its body",
                "expected `=` followed by a type body",
            );
            None
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            constraints: Vec::new(),
            annotation: None,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_class_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let (name, type_parameters) = self.parse_class_head(keyword_index, &mut cursor, end);

        let body = self
            .parse_class_body(&mut cursor, end)
            .map(NamedItemBody::Class);

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            constraints: Vec::new(),
            annotation: None,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_instance_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> InstanceItem {
        let mut cursor = keyword_index + 1;
        let context = self.parse_optional_constraint_prefix(&mut cursor, end);
        let class = self.parse_qualified_name(&mut cursor, end).or_else(|| {
            self.diagnostics.push(
                Diagnostic::error("instance declaration is missing its class name")
                    .with_code(MISSING_INSTANCE_CLASS)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a class name such as `Eq` or `Default`",
                    )
                    .with_help("syntax: instance <Name> of <ClassName> = { ... }"),
            );
            None
        });
        let target = self
            .parse_type_expr(&mut cursor, end, TypeStop::default())
            .or_else(|| {
                self.diagnostics.push(
                    Diagnostic::error("instance declaration is missing its target type")
                        .with_code(MISSING_INSTANCE_TARGET)
                        .with_primary_label(
                            class
                                .as_ref()
                                .map(|class| class.span)
                                .unwrap_or_else(|| self.source_span_of_token(keyword_index)),
                            "expected one instance target type such as `Blob` or `Result HttpError`",
                        )
                        .with_help("syntax: instance <Name> of <ClassName> = { ... }"),
                );
                None
            });
        let body = self.parse_instance_body(&mut cursor, end);
        InstanceItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            context,
            class,
            target,
            body,
        }
    }

    fn parse_domain_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> DomainItem {
        let mut cursor = keyword_index + 1;
        let name =
            self.parse_named_item_name(keyword_index, &mut cursor, end, "domain declaration");
        let mut type_parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(cursor, end) {
            if self.tokens[index].line_start() || self.is_identifier_text(index, "over") {
                break;
            }
            if self.tokens[index].kind() != TokenKind::Identifier {
                break;
            }
            type_parameters.push(self.identifier_from_token(index));
            cursor = index + 1;
        }

        let over_span = if let Some(index) = self.consume_identifier_text(&mut cursor, end, "over")
        {
            Some(self.source_span_of_token(index))
        } else {
            self.diagnostics.push(
                Diagnostic::error("domain declaration is missing `over` before its carrier type")
                    .with_code(MISSING_DOMAIN_OVER)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected `over` followed by the carrier type",
                    )
                    .with_help("syntax: domain <Name> over <CarrierType> = { ... }"),
            );
            None
        };

        let carrier = self.parse_type_expr(&mut cursor, end, TypeStop::default());
        if carrier.is_none() {
            self.diagnostics.push(
                Diagnostic::error("domain declaration is missing its carrier type")
                    .with_code(MISSING_DOMAIN_CARRIER)
                    .with_primary_label(
                        over_span.unwrap_or_else(|| self.source_span_of_token(keyword_index)),
                        "expected a carrier type such as `Int`, `Text`, or `List A`",
                    )
                    .with_help("expected a carrier type after `over`"),
            );
        }

        let body = self.parse_domain_body(&mut cursor, end);
        DomainItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            carrier,
            body,
        }
    }

    fn parse_source_provider_contract_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> SourceProviderContractItem {
        let mut cursor = keyword_index + 1;
        let provider = self.parse_qualified_name(&mut cursor, end).or_else(|| {
            self.diagnostics.push(
                Diagnostic::error("provider contract declaration is missing its provider name")
                    .with_code(MISSING_PROVIDER_CONTRACT_NAME)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a qualified provider name such as `custom.feed`",
                    ),
            );
            None
        });
        let body = self.parse_source_provider_contract_body(&mut cursor, end);
        SourceProviderContractItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            provider,
            body,
        }
    }

    /// Parse a `value` declaration: constant form only, uses `=`.
    fn parse_value_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, "value declaration");
        let (constraints, annotation) = self.parse_function_signature_annotation(&mut cursor, end);

        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_expression_body(
                keyword_index,
                &mut cursor,
                end,
                "value declaration",
                "value declaration is missing its body after `=`",
                "expected an expression after `=`",
            )
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                "value declaration is missing its body",
                "expected `=` followed by an expression",
            );
            None
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints,
            annotation,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    /// Parse a `func` declaration: `func name = params => body` or `func name = .`.
    fn parse_fun_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, "func declaration");
        let (constraints, annotation) = self.parse_function_signature_annotation(&mut cursor, end);

        let has_equals = self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some();
        if !has_equals {
            if self.peek_nontrivia(cursor, end).is_some_and(|index| {
                self.is_function_param_token(index)
                    || self.tokens[index].kind() == TokenKind::Arrow
                    || self.starts_unary_subject_function(cursor, end)
            }) {
                let anchor_span = name
                    .as_ref()
                    .map(|identifier| identifier.span)
                    .unwrap_or_else(|| self.source_span_of_token(keyword_index));
                self.diagnostics.push(
                    Diagnostic::error(
                        "func declaration is missing `=` before its parameters and body",
                    )
                    .with_code(MISSING_DECLARATION_BODY)
                    .with_primary_label(
                        anchor_span,
                        "insert `=` between the function name and its body",
                    )
                    .with_help("expected `=` followed by the function body"),
                );
            } else {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body",
                    "expected `=` followed by an expression using `.` or parameters and `=>`",
                );
            }
        }

        let (function_form, parameters, body) = if let Some((parameters, body)) =
            self.parse_unary_subject_function_body(keyword_index, &mut cursor, end)
        {
            (FunctionSurfaceForm::UnarySubjectSugar, parameters, body)
        } else if let Some((parameters, body)) =
            self.parse_selected_subject_function_body(&mut cursor, end, "func declaration")
        {
            (FunctionSurfaceForm::SelectedSubjectSugar, parameters, body)
        } else {
            let mut parameters = Vec::new();
            while self.starts_function_param(cursor, end) {
                let Some(parameter) = self.parse_function_param(&mut cursor, end) else {
                    break;
                };
                parameters.push(parameter);
            }

            let body = if let Some(arrow_index) =
                self.consume_kind(&mut cursor, end, TokenKind::Arrow)
            {
                let body = self.parse_expression_body(
                    keyword_index,
                    &mut cursor,
                    end,
                    "func declaration",
                    "func declaration is missing its body after `=>`",
                    "expected a body expression after `=>`",
                );
                if parameters.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "func declarations require an explicit parameter before `=>`",
                        )
                        .with_code(NULLARY_FUNCTION_DECLARATION)
                        .with_primary_label(
                            self.source_span_of_token(arrow_index),
                            "insert a parameter such as `_` before `=>`",
                        )
                        .with_note("ignored unary functions are written as `func name = _ => body`")
                        .with_help("functions must have at least one parameter"),
                    );
                }
                body
            } else {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body",
                    "expected `=>` followed by a body expression",
                );
                None
            };
            (FunctionSurfaceForm::Explicit, parameters, body)
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints,
            annotation,
            function_form,
            parameters,
            body,
        }
    }

    fn parse_signal_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
        description: &str,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, description);
        let annotation = self.parse_optional_type_annotation(&mut cursor, end);
        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_signal_body(keyword_index, &mut cursor, end)
        } else {
            None
        };

        if annotation.is_none() && body.is_none() {
            self.missing_body_diagnostic(
                keyword_index,
                "signal declaration is missing its body",
                "expected either `:` with a type or `=` with an expression",
            );
        }

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints: Vec::new(),
            annotation,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_from_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> FromItem {
        let mut cursor = keyword_index + 1;
        let equals_index = self.find_top_level_equals(cursor, end);
        let source_end = equals_index.unwrap_or(end);

        let mut source_cursor = cursor;
        let source = self.parse_expr(&mut source_cursor, source_end, ExprStop::default());
        match &source {
            Some(_) => {
                if let Some(trailing_index) =
                    self.next_significant_in_range(source_cursor, source_end)
                {
                    self.diagnostics.push(
                        Diagnostic::error("`from` source must contain exactly one expression")
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the shared source expression",
                            ),
                    );
                }
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`from` declaration is missing its shared source expression")
                        .with_code(MISSING_FROM_SOURCE)
                        .with_primary_label(
                            self.source_span_of_token(keyword_index),
                            "expected a source expression such as `state` after `from`",
                        )
                        .with_help("syntax: from <signal-expr> = { derivedName: expr }"),
                );
            }
        }
        cursor = equals_index.map_or(source_cursor, |index| index + 1);

        let open_brace = self.consume_kind(&mut cursor, end, TokenKind::LBrace);
        let entries = if let Some(open_brace) = open_brace {
            let inner_end = self.find_matching_brace(open_brace, end).unwrap_or(end);
            let mut entries_cursor = open_brace + 1;
            self.parse_from_entries(&mut entries_cursor, inner_end)
        } else {
            self.diagnostics.push(
                Diagnostic::error("`from` declaration is missing its fan-out body")
                    .with_code(MISSING_FROM_OPEN_BRACE)
                    .with_primary_label(
                        self.source_span_of_token(equals_index.unwrap_or(keyword_index)),
                        "expected `{` to open the grouped derived signals",
                    )
                    .with_help("syntax: from <signal-expr> = { derivedName: expr }"),
            );
            Vec::new()
        };

        FromItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            source,
            entries,
        }
    }

    fn parse_from_entries(&mut self, cursor: &mut usize, end: usize) -> Vec<FromEntry> {
        let Some(first_index) = self.peek_nontrivia(*cursor, end) else {
            return Vec::new();
        };
        if !self.tokens[first_index].line_start() {
            return Vec::new();
        }
        let entry_indent = self.line_indent_of_token(first_index);
        let mut entries = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.starts_from_entry(index, end, entry_indent) {
                break;
            }
            let entry_end = self
                .find_next_from_entry_start(index + 1, end, entry_indent)
                .unwrap_or(end);
            entries.push(self.parse_from_entry(index, entry_end));
            *cursor = entry_end;
        }
        entries
    }

    fn starts_from_entry(&self, index: usize, end: usize, entry_indent: usize) -> bool {
        let token = self.tokens[index];
        token.kind() == TokenKind::Identifier
            && token.line_start()
            && self.line_indent_of_token(index) == entry_indent
            && self.peek_kind(index + 1, end) == Some(TokenKind::Colon)
    }

    fn parse_from_entry(&mut self, start: usize, end: usize) -> FromEntry {
        let mut cursor = start;
        let name = self
            .parse_identifier(&mut cursor, end)
            .unwrap_or_else(|| Identifier {
                text: "<missing>".to_owned(),
                span: self.source_span_of_token(start),
            });
        let _ = self.consume_kind(&mut cursor, end, TokenKind::Colon);
        let body = self.parse_expr(&mut cursor, end, ExprStop::default());
        match &body {
            Some(_) => {
                if let Some(trailing_index) = self.next_significant_in_range(cursor, end) {
                    self.diagnostics.push(
                        Diagnostic::error("`from` entry body must contain exactly one expression")
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the derived signal body",
                            ),
                    );
                }
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`from` entry is missing its derived expression")
                        .with_code(MISSING_FROM_ENTRY_BODY)
                        .with_primary_label(
                            name.span,
                            format!("expected an expression after `{}:`", name.text),
                        ),
                );
            }
        }

        FromEntry {
            name,
            body,
            span: self.source_span_for_range(start, end),
        }
    }

    fn find_next_from_entry_start(
        &self,
        from: usize,
        end: usize,
        entry_indent: usize,
    ) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if depth == 0 && self.starts_from_entry(index, end, entry_indent) {
                return Some(index);
            }
            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        None
    }

    /// Parse a signal body after `=`. This may be:
    /// - A merge body: `sig1 | sig2 ||> ...` or `sig1 ||> ...`
    /// - A plain expression body: `expr |> pipe`
    fn parse_signal_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<NamedItemBody> {
        // Probe: is this a merge body?
        // A merge body starts with an identifier, optionally followed by `|` ident sequences,
        // and eventually has `||>` arms (either inline or on next lines).
        if let Some(merge) = self.try_parse_signal_merge_body(keyword_index, cursor, end) {
            return Some(NamedItemBody::Merge(merge));
        }
        // Fall back to plain expression body.
        self.parse_expression_body(
            keyword_index,
            cursor,
            end,
            "signal declaration",
            "signal declaration is missing its body after `=`",
            "expected an expression after `=`",
        )
    }

    /// Attempt to parse a signal merge body: `sig1 | sig2 ||> arm1 ||> arm2 ...`
    /// Returns `None` if the token stream doesn't match the merge pattern, leaving cursor unchanged.
    fn try_parse_signal_merge_body(
        &mut self,
        _keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SignalMergeBody> {
        let merge_span_start = *cursor;

        // Probe ahead without consuming: look for identifier (| identifier)* followed by ||> arms.
        let mut probe = *cursor;
        let mut source_positions: Vec<usize> = Vec::new();

        // First source must be an identifier.
        let first = self.peek_nontrivia(probe, end)?;
        if self.tokens[first].kind() != TokenKind::Identifier {
            return None;
        }
        source_positions.push(first);
        probe = first + 1;

        // Collect additional `| ident` sources.
        loop {
            let Some(next) = self.peek_nontrivia(probe, end) else {
                break;
            };
            if self.tokens[next].kind() == TokenKind::PipeTap && !self.tokens[next].line_start() {
                let Some(ident_idx) = self.peek_nontrivia(next + 1, end) else {
                    break;
                };
                if self.tokens[ident_idx].kind() == TokenKind::Identifier
                    && !self.tokens[ident_idx].line_start()
                {
                    source_positions.push(ident_idx);
                    probe = ident_idx + 1;
                    continue;
                }
            }
            break;
        }

        // Now check: do we see `||>` arms?
        let has_arms = self.find_signal_reactive_arm_start(probe, end).is_some();
        if !has_arms {
            return None;
        }

        // Commit: parse the sources.
        let mut sources = Vec::new();
        for &pos in &source_positions {
            let ident = Identifier {
                text: self.tokens[pos].text(self.source).to_owned(),
                span: self.source_span_of_token(pos),
            };
            sources.push(ident);
        }
        *cursor = probe;

        // Parse the arms.
        let arms = self.parse_signal_reactive_arms(cursor, end);

        let merge_span =
            self.source_span_for_range(merge_span_start, *cursor.min(&mut end.clone()));
        Some(SignalMergeBody {
            sources,
            arms,
            span: merge_span,
        })
    }

    /// Find the start of the first `||>` reactive arm token within `[from, end)`.
    /// Reactive arms use `=>` (fat arrow). Pipe-case arms use `->` (thin arrow) and are NOT reactive.
    fn find_signal_reactive_arm_start(&self, from: usize, end: usize) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if token.kind() == TokenKind::PipeCase && token.line_start() {
                // Verify this arm uses `=>` (reactive) not `->` (case).
                // Scan ahead on the same line for the arrow token.
                if self.reactive_arm_uses_fat_arrow(index + 1, end) {
                    return Some(index);
                }
                // `||>` with `->` is a pipe-case expression, not a reactive arm.
                return None;
            }
            // If we encounter something else that's non-trivia and on a new line, stop.
            if token.line_start() && token.kind() != TokenKind::PipeCase {
                return None;
            }
        }
        None
    }

    /// Check if a `||>` arm uses `=>` (fat arrow) rather than `->` (thin arrow).
    /// Scans from the token after `||>` up to the next line-start or end.
    fn reactive_arm_uses_fat_arrow(&self, from: usize, end: usize) -> bool {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            // If we hit a new line-start token, stop scanning this arm.
            if token.line_start() {
                return false;
            }
            if token.kind() == TokenKind::Arrow {
                return true;
            }
            if token.kind() == TokenKind::ThinArrow {
                return false;
            }
        }
        false
    }

    /// Parse all `||>` reactive arms at a consistent indent level.
    fn parse_signal_reactive_arms(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<SignalReactiveArm> {
        let Some(first_index) = self.peek_nontrivia(*cursor, end) else {
            return Vec::new();
        };
        if self.tokens[first_index].kind() != TokenKind::PipeCase
            || !self.tokens[first_index].line_start()
        {
            return Vec::new();
        }
        let arm_indent = self.line_indent_of_token(first_index);
        let mut arms = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].kind() != TokenKind::PipeCase
                || !self.tokens[index].line_start()
                || self.line_indent_of_token(index) != arm_indent
            {
                break;
            }
            let arm_end = self
                .find_next_signal_reactive_arm_start(index + 1, end, arm_indent)
                .unwrap_or(end);
            arms.push(self.parse_signal_reactive_arm(index, arm_end));
            *cursor = arm_end;
        }
        arms
    }

    fn find_next_signal_reactive_arm_start(
        &self,
        from: usize,
        end: usize,
        arm_indent: usize,
    ) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if depth == 0
                && token.kind() == TokenKind::PipeCase
                && token.line_start()
                && self.line_indent_of_token(index) == arm_indent
            {
                return Some(index);
            }
            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        None
    }

    /// Parse a single `||> [source] pattern => body` arm.
    fn parse_signal_reactive_arm(&mut self, arm_start: usize, arm_end: usize) -> SignalReactiveArm {
        let mut cursor = arm_start + 1; // skip `||>`

        // Determine if this arm has a source prefix: `||> sourceName pattern => body`
        // vs plain `||> pattern => body` or `||> _ => body`.
        let source = self.try_parse_arm_source_prefix(&mut cursor, arm_end);

        let pattern = self
            .parse_pattern(
                &mut cursor,
                arm_end,
                PatternStop::signal_reactive_arm_context(),
            )
            .or_else(|| {
                self.diagnostics.push(
                    Diagnostic::error("signal reactive arm is missing its pattern")
                        .with_code(MISSING_REACTIVE_UPDATE_ARM_PATTERN)
                        .with_primary_label(
                            self.source_span_of_token(arm_start),
                            "expected a pattern after `||>`",
                        ),
                );
                None
            });

        let arrow_anchor = pattern
            .as_ref()
            .map(|p| p.span)
            .or_else(|| source.as_ref().map(|s| s.span))
            .unwrap_or_else(|| self.source_span_of_token(arm_start));
        let arrow_present = self
            .consume_kind(&mut cursor, arm_end, TokenKind::Arrow)
            .is_some();
        if !arrow_present {
            self.diagnostics.push(
                Diagnostic::error("signal reactive arm is missing `=>` before its body")
                    .with_code(MISSING_REACTIVE_UPDATE_ARM_ARROW)
                    .with_primary_label(arrow_anchor, "expected `=>` followed by the arm body"),
            );
        }

        let body = if arrow_present {
            self.parse_expr(&mut cursor, arm_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("signal reactive arm is missing its body expression")
                            .with_code(MISSING_REACTIVE_UPDATE_ARM_BODY)
                            .with_primary_label(arrow_anchor, "expected an expression after `=>`"),
                    );
                    None
                })
        } else {
            None
        };

        SignalReactiveArm {
            source,
            pattern,
            body,
            span: self.source_span_for_range(arm_start, arm_end),
        }
    }

    /// Try to parse a source signal name prefix in a reactive arm.
    /// In multi-source merges: `||> tick _ => ...` — `tick` is the source prefix.
    /// We detect this by checking if there's an identifier followed by a pattern and `=>`.
    /// If the identifier IS the pattern (e.g., `||> True => ...`), we don't consume it as source.
    fn try_parse_arm_source_prefix(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<Identifier> {
        let ident_idx = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[ident_idx].kind() != TokenKind::Identifier {
            return None;
        }

        let text = self.tokens[ident_idx].text(self.source);
        // If this is `_`, it's a wildcard pattern, not a source prefix.
        if text == "_" {
            return None;
        }
        // If it starts with uppercase, it could be a constructor pattern.
        // Check: is there something after it before `=>`?
        let first_char = text.chars().next().unwrap_or('a');
        if first_char.is_uppercase() {
            // Uppercase: likely a constructor pattern like `Tick`, `Turn dir`, etc.
            // Only treat as source prefix if the text is lowercase.
            return None;
        }

        // lowercase identifier. Check if it's followed by another token (pattern) before `=>`.
        let Some(next_idx) = self.peek_nontrivia(ident_idx + 1, end) else {
            return None;
        };
        // If immediately followed by `=>`, this identifier IS the pattern, not a source prefix.
        if self.tokens[next_idx].kind() == TokenKind::Arrow {
            return None;
        }
        // It's a source prefix: lowercase ident followed by pattern tokens.
        let ident = Identifier {
            text: text.to_owned(),
            span: self.source_span_of_token(ident_idx),
        };
        *cursor = ident_idx + 1;
        Some(ident)
    }

    fn parse_class_body(&mut self, cursor: &mut usize, end: usize) -> Option<ClassBody> {
        // A class body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("class declaration is missing `{` after `=`")
                        .with_code(MISSING_CLASS_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the class body")
                        .with_help("expected `{` to open the class body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start class member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start()
                || !matches!(
                    self.tokens[first].kind(),
                    TokenKind::Identifier | TokenKind::LParen
                )
            {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        let mut with_decls = Vec::new();
        let mut require_decls = Vec::new();
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            // Detect the context-sensitive `with` and `require` soft-keywords.
            // They are treated as declarations only when NOT immediately followed by `:`,
            // which disambiguates them from method names (`with: A -> A`).
            if self.tokens[index].kind() == TokenKind::Identifier {
                let text = self.tokens[index].text(self.source);
                if text == "with" && self.peek_kind(index + 1, inner_end) != Some(TokenKind::Colon)
                {
                    if let Some(decl) = self.parse_class_with_decl(cursor, inner_end) {
                        with_decls.push(decl);
                        continue;
                    }
                } else if text == "require"
                    && self.peek_kind(index + 1, inner_end) != Some(TokenKind::Colon)
                {
                    if let Some(decl) = self.parse_class_require_decl(cursor, inner_end) {
                        require_decls.push(decl);
                        continue;
                    }
                }
            }
            let before = *cursor;
            let Some(member) = self.parse_class_member(cursor, inner_end) else {
                break;
            };
            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!with_decls.is_empty() || !require_decls.is_empty() || !members.is_empty()).then_some(
            ClassBody {
                with_decls,
                require_decls,
                members,
                span: self.source_span_for_range(body_start, *cursor),
            },
        )
    }

    fn parse_class_with_decl(&mut self, cursor: &mut usize, end: usize) -> Option<ClassWithDecl> {
        let start = *cursor;
        let with_index = self.peek_nontrivia(*cursor, end)?;
        // Consume the `with` soft-keyword.
        *cursor = with_index + 1;
        // Require the superclass type to start on the same line as `with`.
        let Some(type_index) = self.peek_nontrivia(*cursor, end) else {
            *cursor = start;
            return None;
        };
        if self.tokens[type_index].line_start() {
            *cursor = start;
            return None;
        }
        let Some(superclass) = self.parse_type_expr(cursor, end, TypeStop::default()) else {
            self.diagnostics.push(
                Diagnostic::error("`with` declaration is missing its superclass type")
                    .with_code(MISSING_CLASS_WITH_TYPE)
                    .with_primary_label(
                        self.source_span_of_token(with_index),
                        "expected a superclass type such as `Applicative M`",
                    ),
            );
            *cursor = start;
            return None;
        };
        Some(ClassWithDecl {
            superclass,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_class_require_decl(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ClassRequireDecl> {
        let start = *cursor;
        let require_index = self.peek_nontrivia(*cursor, end)?;
        // Consume the `require` soft-keyword.
        *cursor = require_index + 1;
        // Require the constraint type to start on the same line as `require`.
        let Some(type_index) = self.peek_nontrivia(*cursor, end) else {
            *cursor = start;
            return None;
        };
        if self.tokens[type_index].line_start() {
            *cursor = start;
            return None;
        }
        let Some(constraint) = self.parse_type_expr(cursor, end, TypeStop::default()) else {
            self.diagnostics.push(
                Diagnostic::error("`require` declaration is missing its constraint type")
                    .with_code(MISSING_CLASS_REQUIRE_TYPE)
                    .with_primary_label(
                        self.source_span_of_token(require_index),
                        "expected a constraint type such as `Eq K`",
                    ),
            );
            *cursor = start;
            return None;
        };
        Some(ClassRequireDecl {
            constraint,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_instance_body(&mut self, cursor: &mut usize, end: usize) -> Option<InstanceBody> {
        // An instance body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("instance declaration is missing `{` after `=`")
                        .with_code(MISSING_INSTANCE_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the instance body")
                        .with_help("expected `{` to open the instance body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start instance member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start() || !self.starts_instance_member(first) {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if !self.starts_instance_member(index) {
                break;
            }
            let before = *cursor;
            let Some(member) = self.parse_instance_member(cursor, inner_end, member_indent) else {
                break;
            };
            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!members.is_empty()).then_some(InstanceBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_domain_body(&mut self, cursor: &mut usize, end: usize) -> Option<DomainBody> {
        // A domain body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("domain declaration is missing `{` after `=`")
                        .with_code(MISSING_DOMAIN_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the domain body")
                        .with_help("expected `{` to open the domain body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start domain member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start() || !self.starts_domain_member(first) {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        let mut members = Vec::new();
        let mut pending_annotation: Option<TypeExpr> = None;
        // When we see `name : TypeExpr` (colon annotation), we hold the member back here.
        // If the very next member has the same name and a body, we merge the annotation in.
        // Otherwise we flush the held member as-is.
        let mut pending_colon_member: Option<DomainMember> = None;

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if !self.starts_domain_member(index) {
                break;
            }

            // Handle `type TypeExpr` lines as annotations for the next member
            if self.tokens[index].kind() == TokenKind::TypeKw {
                if let Some(held) = pending_colon_member.take() {
                    members.push(held);
                }
                *cursor = index + 1;
                let type_end = self
                    .find_next_domain_member_start(*cursor, inner_end, member_indent)
                    .unwrap_or(inner_end);
                let annotation = self
                    .parse_type_expr(cursor, type_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "domain member type annotation is missing its type after `type`",
                            )
                            .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                            .with_primary_label(
                                self.source_span_for_range(index, *cursor),
                                "expected a type such as `Int -> Duration`",
                            ),
                        );
                        None
                    });
                *cursor = type_end;
                pending_annotation = annotation;
                continue;
            }

            let before = *cursor;
            let Some(mut member) = self.parse_domain_member(cursor, inner_end, member_indent)
            else {
                break;
            };

            // Apply `type TypeExpr` annotation if pending.
            if let Some(annotation) = pending_annotation.take() {
                member.annotation = Some(annotation);
            }

            // Try to pair a pending `name : TypeExpr` annotation with this implementation.
            if member.annotation.is_none() {
                if let Some(held) = pending_colon_member.take() {
                    let held_name = domain_member_surface_name_str(&held.name);
                    let this_name = domain_member_surface_name_str(&member.name);
                    if held_name == this_name {
                        // Merge: attach the annotation from the held member.
                        member.annotation = held.annotation;
                    } else {
                        // Different name — flush held first.
                        members.push(held);
                    }
                }
            }

            // If this member is annotation-only (from `name : TypeExpr`), hold it.
            if member.annotation.is_some()
                && member.body.is_none()
                && member.parameters.is_empty()
                && pending_annotation.is_none()
            {
                if let Some(held) = pending_colon_member.take() {
                    members.push(held);
                }
                pending_colon_member = Some(member);
                if *cursor <= before {
                    break;
                }
                continue;
            }

            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Flush any held colon-annotation member.
        if let Some(held) = pending_colon_member.take() {
            members.push(held);
        }

        if let Some(annotation) = pending_annotation {
            self.diagnostics.push(
                Diagnostic::error("domain member type annotation has no following member")
                    .with_code(MISSING_DOMAIN_MEMBER_NAME)
                    .with_primary_label(
                        annotation.span,
                        "expected a member binding after this type annotation",
                    ),
            );
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!members.is_empty()).then_some(DomainBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_source_provider_contract_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SourceProviderContractBody> {
        let body_start = *cursor;
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.tokens[index].line_start() {
                break;
            }
            let Some(member) = self.parse_source_provider_contract_member(cursor, end) else {
                break;
            };
            members.push(member);
        }

        (!members.is_empty()).then_some(SourceProviderContractBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_class_member(&mut self, cursor: &mut usize, end: usize) -> Option<ClassMember> {
        let start = *cursor;
        let name = self.parse_signature_member_name(cursor, end)?;
        let (constraints, annotation) =
            if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                let (constraints, annotation) = self.parse_constrained_type(cursor, end);
                let annotation = annotation.or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("class member is missing its type after `:`")
                            .with_code(MISSING_CLASS_MEMBER_TYPE)
                            .with_primary_label(
                                name.span(),
                                "expected a member type such as `A -> A -> Bool`",
                            ),
                    );
                    None
                });
                (constraints, annotation)
            } else {
                self.diagnostics.push(
                    Diagnostic::error("class member is missing `:` before its type")
                        .with_code(MISSING_CLASS_MEMBER_TYPE)
                        .with_primary_label(name.span(), "expected `:` followed by a member type"),
                );
                (Vec::new(), None)
            };

        Some(ClassMember {
            name,
            constraints,
            annotation,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_instance_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<InstanceMember> {
        let start = *cursor;
        let name = self.parse_signature_member_name(cursor, end)?;
        let mut parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() {
                break;
            }
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    parameters.push(self.identifier_from_token(index));
                    *cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        let equals_span = if let Some(index) = self.consume_kind(cursor, end, TokenKind::Equals) {
            Some(self.source_span_of_token(index))
        } else {
            self.diagnostics.push(
                Diagnostic::error("instance member is missing `=` before its body")
                    .with_code(MISSING_INSTANCE_MEMBER_BODY)
                    .with_primary_label(name.span(), "expected `=` followed by an expression body"),
            );
            None
        };
        let member_end = self
            .find_next_instance_member_start(*cursor, end, member_indent)
            .unwrap_or(end);
        let body = if let Some(equals_span) = equals_span {
            self.parse_expr(cursor, member_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("instance member is missing its body after `=`")
                            .with_code(MISSING_INSTANCE_MEMBER_BODY)
                            .with_primary_label(
                                equals_span,
                                "expected an expression body for this instance member",
                            ),
                    );
                    None
                })
        } else {
            None
        };
        if body.is_some() {
            if let Some(trailing_index) = self.next_significant_in_range(*cursor, member_end) {
                self.diagnostics.push(
                    Diagnostic::error("instance member body must contain exactly one expression")
                        .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "this token is outside the instance member body",
                        ),
                );
            }
        }
        *cursor = member_end;
        Some(InstanceMember {
            name,
            parameters,
            body,
            span: self.source_span_for_range(start, member_end),
        })
    }

    fn parse_domain_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<DomainMember> {
        let start = *cursor;

        // Literal members keep the colon syntax: `literal ms : Int -> Duration`
        if self
            .consume_identifier_text(cursor, end, "literal")
            .is_some()
        {
            let Some(suffix) = self.parse_identifier(cursor, end) else {
                self.diagnostics.push(
                    Diagnostic::error("domain literal declaration is missing its suffix name")
                        .with_code(MISSING_DOMAIN_MEMBER_NAME)
                        .with_primary_label(
                            self.source_span_for_range(start, *cursor),
                            "expected a suffix name such as `ms` or `sec`",
                        ),
                );
                return None;
            };
            let annotation = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_type_expr(cursor, end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error("domain literal is missing its type after `:`")
                                .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                                .with_primary_label(
                                    suffix.span,
                                    "expected a member type such as `Int -> Duration`",
                                ),
                        );
                        None
                    })
            } else {
                self.diagnostics.push(
                    Diagnostic::error("domain literal is missing `:` before its type")
                        .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                        .with_primary_label(suffix.span, "expected `:` followed by a member type"),
                );
                None
            };
            return Some(DomainMember {
                name: DomainMemberName::Literal(suffix),
                annotation,
                parameters: Vec::new(),
                body: None,
                span: self.source_span_for_range(start, *cursor),
            });
        }

        // Signature members: `name params = body` (binding) or `name` (declaration-only)
        let name = self.parse_signature_member_name(cursor, end)?;

        // Inline colon annotation: `name : TypeExpr` — returns annotation-only member.
        // The implementation `name params = body` may follow on the next line with the same name.
        if let Some(colon_idx) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[colon_idx].kind() == TokenKind::Colon
                && !self.tokens[colon_idx].line_start()
            {
                *cursor = colon_idx + 1;
                let ann_end = self
                    .find_next_domain_member_start(*cursor, end, member_indent)
                    .unwrap_or(end);
                let annotation = self.parse_type_expr(cursor, ann_end, TypeStop::default());
                *cursor = ann_end;
                return Some(DomainMember {
                    name: DomainMemberName::Signature(name),
                    annotation,
                    parameters: Vec::new(),
                    body: None,
                    span: self.source_span_for_range(start, *cursor),
                });
            }
        }

        let mut parameters = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() {
                break;
            }
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    parameters.push(self.identifier_from_token(index));
                    *cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        if let Some(eq_index) = self.consume_kind(cursor, end, TokenKind::Equals) {
            let equals_span = self.source_span_of_token(eq_index);
            let member_end = self
                .find_next_domain_member_start(*cursor, end, member_indent)
                .unwrap_or(end);
            let body = self
                .parse_expr(cursor, member_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("domain member binding is missing its body after `=`")
                            .with_code(MISSING_DOMAIN_MEMBER_BODY)
                            .with_primary_label(
                                equals_span,
                                "expected an expression body for this domain member",
                            ),
                    );
                    None
                });
            if body.is_some() {
                if let Some(trailing_index) = self.next_significant_in_range(*cursor, member_end) {
                    self.diagnostics.push(
                        Diagnostic::error("domain member body must contain exactly one expression")
                            .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the domain member body",
                            ),
                    );
                }
            }
            *cursor = member_end;
            return Some(DomainMember {
                name: DomainMemberName::Signature(name),
                annotation: None,
                parameters,
                body,
                span: self.source_span_for_range(start, member_end),
            });
        }

        // No `=` — declaration-only member (just a name, no body)
        if !parameters.is_empty() {
            self.diagnostics.push(
                Diagnostic::error("domain member binding is missing `=` before its body")
                    .with_code(MISSING_DOMAIN_MEMBER_BODY)
                    .with_primary_label(name.span(), "expected `=` followed by an expression body"),
            );
            return None;
        }

        Some(DomainMember {
            name: DomainMemberName::Signature(name),
            annotation: None,
            parameters: Vec::new(),
            body: None,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_source_provider_contract_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SourceProviderContractMember> {
        let start = *cursor;
        let name = self.parse_identifier(cursor, end)?;
        match name.text.as_str() {
            "option" | "argument" | "operation" | "command" => {
                let schema_kind = match name.text.as_str() {
                    "option" => ("source option", "timeout", "Text", "OptionSchema"),
                    "argument" => ("source argument", "path", "Text", "ArgumentSchema"),
                    "operation" => (
                        "provider operation",
                        "read",
                        "Text -> Signal Text",
                        "OperationSchema",
                    ),
                    "command" => (
                        "provider command",
                        "delete",
                        "Text -> Task Text Unit",
                        "CommandSchema",
                    ),
                    _ => {
                        unreachable!("provider contract schema members stay within known keywords")
                    }
                };
                let schema_name = self.parse_identifier(cursor, end).or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("provider contract schema member is missing its name")
                            .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_NAME)
                            .with_primary_label(
                                name.span,
                                format!(
                                    "expected a {} name such as `{}`",
                                    schema_kind.0, schema_kind.1
                                ),
                            ),
                    );
                    None
                });
                let annotation = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                    self.parse_type_expr(cursor, end, TypeStop::default())
                        .or_else(|| {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    "provider contract schema member is missing its type after `:`",
                                )
                                .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE)
                                .with_primary_label(
                                    schema_name.as_ref().map_or(name.span, |item| item.span),
                                    format!(
                                        "expected a {} type such as `{}`",
                                        schema_kind.0, schema_kind.2
                                    ),
                                ),
                            );
                            None
                        })
                } else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "provider contract schema member is missing `:` before its type",
                        )
                        .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE)
                        .with_primary_label(
                            schema_name.as_ref().map_or(name.span, |item| item.span),
                            "expected `:` followed by a schema type",
                        ),
                    );
                    None
                };
                let member = SourceProviderContractSchemaMember {
                    name: schema_name,
                    annotation,
                    span: self.source_span_for_range(start, *cursor),
                };
                Some(match schema_kind.3 {
                    "OptionSchema" => SourceProviderContractMember::OptionSchema(member),
                    "ArgumentSchema" => SourceProviderContractMember::ArgumentSchema(member),
                    "OperationSchema" => SourceProviderContractMember::OperationSchema(member),
                    "CommandSchema" => SourceProviderContractMember::CommandSchema(member),
                    _ => unreachable!("provider contract schema kinds stay within known variants"),
                })
            }
            _ => {
                let value = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                    self.parse_identifier(cursor, end).or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "provider contract member is missing its value after `:`",
                            )
                            .with_code(MISSING_PROVIDER_CONTRACT_MEMBER_VALUE)
                            .with_primary_label(
                                name.span,
                                "expected a provider-contract value such as `providerTrigger`",
                            ),
                        );
                        None
                    })
                } else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "provider contract member is missing `:` before its value",
                        )
                        .with_code(MISSING_PROVIDER_CONTRACT_MEMBER_VALUE)
                        .with_primary_label(name.span, "expected `:` followed by a contract value"),
                    );
                    None
                };

                Some(SourceProviderContractMember::FieldValue(
                    SourceProviderContractFieldValue {
                        name: Some(name),
                        value,
                        span: self.source_span_for_range(start, *cursor),
                    },
                ))
            }
        }
    }

    fn parse_signature_member_name(
        &self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ClassMemberName> {
        if let Some(identifier) = self.parse_identifier(cursor, end) {
            return Some(ClassMemberName::Identifier(identifier));
        }

        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let operator_index = self.peek_nontrivia(*cursor, end)?;
        let operator = match self.tokens[operator_index].kind() {
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::Greater => OperatorName {
                text: self.tokens[operator_index].text(self.source).to_owned(),
                span: self.source_span_of_token(operator_index),
            },
            _ => return None,
        };
        *cursor = operator_index + 1;
        let _ = self.consume_kind(cursor, end, TokenKind::RParen)?;
        let span = self.source_span_for_range(start, *cursor);
        Some(ClassMemberName::Operator(OperatorName {
            text: operator.text,
            span,
        }))
    }

    fn parse_use_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> UseItem {
        let mut cursor = keyword_index + 1;
        let path = self.parse_qualified_name(&mut cursor, end);
        if path.is_none() {
            self.diagnostics.push(
                Diagnostic::error("`use` declaration is missing its module path")
                    .with_code(MISSING_USE_PATH)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a dotted module path such as `aivi.network`",
                    ),
            );
        }

        let mut imports = Vec::new();
        if self
            .consume_kind(&mut cursor, end, TokenKind::LParen)
            .is_some()
        {
            loop {
                if self
                    .consume_kind(&mut cursor, end, TokenKind::RParen)
                    .is_some()
                {
                    break;
                }
                let Some(import) = self.parse_use_import(&mut cursor, end) else {
                    break;
                };
                imports.push(import);
                let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
            }
        }

        UseItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            path,
            imports,
        }
    }

    fn parse_use_import(&mut self, cursor: &mut usize, end: usize) -> Option<UseImport> {
        let path = self.parse_qualified_name(cursor, end)?;
        let alias = match self.peek_nontrivia(*cursor, end) {
            Some(index)
                if self.tokens[index].kind() == TokenKind::Identifier
                    && self.is_identifier_text(index, "as") =>
            {
                *cursor = index + 1;
                match self.parse_identifier(cursor, end) {
                    Some(alias) => Some(alias),
                    None => {
                        self.diagnostics.push(
                            Diagnostic::error("`use` import alias is missing its local name")
                                .with_code(MISSING_USE_ALIAS)
                                .with_primary_label(
                                    self.source_span_of_token(index),
                                    "expected a local alias such as `request` after `as`",
                                ),
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        Some(UseImport { path, alias })
    }

    fn parse_export_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> ExportItem {
        let mut cursor = keyword_index + 1;
        let targets = self.parse_export_targets(&mut cursor, end, keyword_index);

        ExportItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            targets,
        }
    }

    fn parse_export_targets(
        &mut self,
        cursor: &mut usize,
        end: usize,
        keyword_index: usize,
    ) -> Vec<Identifier> {
        let Some(next_index) = self.peek_nontrivia(*cursor, end) else {
            self.diagnostics.push(self.missing_export_name_diagnostic(
                self.source_span_of_token(keyword_index),
                "expected an identifier after `export`",
            ));
            return Vec::new();
        };

        if self.tokens[next_index].kind() != TokenKind::LParen {
            let target = self.parse_identifier(cursor, end);
            if target.is_none() {
                self.diagnostics.push(self.missing_export_name_diagnostic(
                    self.source_span_of_token(keyword_index),
                    "expected an identifier after `export`",
                ));
            }
            return target.into_iter().collect();
        }

        *cursor = next_index + 1;
        let group_span = self.source_span_of_token(next_index);
        let mut targets = Vec::new();
        let mut emitted_missing_target = false;

        loop {
            if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
                break;
            }

            match self.parse_identifier(cursor, end) {
                Some(target) => targets.push(target),
                None => {
                    let span = self
                        .peek_nontrivia(*cursor, end)
                        .map(|index| self.source_span_of_token(index))
                        .unwrap_or(group_span);
                    let message = if targets.is_empty() {
                        "expected at least one identifier inside `export (...)`"
                    } else {
                        "expected an identifier after `,` inside `export (...)`"
                    };
                    self.diagnostics
                        .push(self.missing_export_name_diagnostic(span, message));
                    emitted_missing_target = true;
                    break;
                }
            }

            let _ = self.consume_kind(cursor, end, TokenKind::Comma);
        }

        if targets.is_empty() && !emitted_missing_target {
            self.diagnostics.push(self.missing_export_name_diagnostic(
                group_span,
                "expected at least one identifier inside `export (...)`",
            ));
        }

        targets
    }

    fn missing_export_name_diagnostic(&self, span: SourceSpan, label: &str) -> Diagnostic {
        Diagnostic::error("`export` declaration is missing the exported name")
            .with_code(MISSING_EXPORT_NAME)
            .with_primary_label(span, label)
    }

    fn parse_hoist_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> crate::cst::HoistItem {
        let mut cursor = keyword_index + 1;

        // Optional kind filter list: `(func, value, signal, type, domain, class)`
        let kind_filters = if self
            .peek_nontrivia(cursor, end)
            .map(|i| self.tokens[i].kind() == TokenKind::LParen)
            .unwrap_or(false)
        {
            let lparen_idx = self.peek_nontrivia(cursor, end).unwrap();
            cursor = lparen_idx + 1;
            let mut filters = Vec::new();
            loop {
                if self
                    .consume_kind(&mut cursor, end, TokenKind::RParen)
                    .is_some()
                {
                    break;
                }
                match self.parse_identifier(&mut cursor, end) {
                    Some(ident) => {
                        filters.push(crate::cst::HoistKindFilter {
                            span: ident.span,
                            text: ident.text,
                        });
                    }
                    None => break,
                }
                let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
            }
            filters
        } else {
            Vec::new()
        };

        // Optional `hiding (name1, name2, ...)` clause
        let hiding = if self
            .peek_nontrivia(cursor, end)
            .map(|i| {
                self.tokens[i].kind() == TokenKind::Identifier
                    && self.is_identifier_text(i, "hiding")
            })
            .unwrap_or(false)
        {
            let hiding_idx = self.peek_nontrivia(cursor, end).unwrap();
            cursor = hiding_idx + 1;
            if self
                .peek_nontrivia(cursor, end)
                .map(|i| self.tokens[i].kind() == TokenKind::LParen)
                .unwrap_or(false)
            {
                let lparen_idx = self.peek_nontrivia(cursor, end).unwrap();
                cursor = lparen_idx + 1;
                let mut names = Vec::new();
                loop {
                    if self
                        .consume_kind(&mut cursor, end, TokenKind::RParen)
                        .is_some()
                    {
                        break;
                    }
                    match self.parse_identifier(&mut cursor, end) {
                        Some(ident) => names.push(ident),
                        None => break,
                    }
                    let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
                }
                names
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        crate::cst::HoistItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            kind_filters,
            hiding,
        }
    }

    fn parse_error_item(&mut self, start: usize) -> Item {
        let end = self
            .find_next_item_start(start + 1)
            .unwrap_or(self.tokens.len());
        let base = self.make_base(start, end, Vec::new());
        self.diagnostics.push(
            Diagnostic::error("expected a top-level declaration")
                .with_code(UNEXPECTED_TOP_LEVEL_TOKEN)
                .with_primary_label(
                    self.source_span_of_token(start),
                    "expected `type`, `value`, `func`, `signal`, `class`, `use`, `export`, `hoist`, or `@decorator` here",
                ),
        );
        Item::Error(ErrorItem {
            base,
            message: "unexpected top-level token".to_owned(),
        })
    }

    fn parse_decorators(&mut self, start: usize, keyword_index: usize) -> Vec<Decorator> {
        let starts = self.collect_decorator_starts(start, keyword_index);
        let mut decorators = Vec::new();
        for (index, decorator_start) in starts.iter().copied().enumerate() {
            let decorator_end = starts.get(index + 1).copied().unwrap_or(keyword_index);
            if let Some(decorator) = self.parse_decorator_range(decorator_start, decorator_end) {
                decorators.push(decorator);
            }
        }
        decorators
    }

    fn collect_decorator_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut depth = 0usize;
        for index in start..end {
            let token = self.tokens[index];
            if !token.kind().is_trivia()
                && token.line_start()
                && depth == 0
                && token.kind() == TokenKind::At
            {
                starts.push(index);
            }
            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        starts
    }

    fn parse_decorator_range(&mut self, start: usize, end: usize) -> Option<Decorator> {
        let mut cursor = start;
        let _ = self.consume_kind(&mut cursor, end, TokenKind::At)?;
        let name = match self.parse_decorator_qualified_name(&mut cursor, end) {
            Some(name) => name,
            None => {
                self.diagnostics.push(
                    Diagnostic::error("decorator name is missing after `@`")
                        .with_code(MISSING_DECORATOR_NAME)
                        .with_primary_label(
                            self.source_span_of_token(start),
                            "expected a decorator name such as `source`",
                        ),
                );
                return None;
            }
        };
        let payload = if name.as_dotted() == "source" {
            DecoratorPayload::Source(self.parse_source_decorator_payload(&mut cursor, end))
        } else {
            self.parse_generic_decorator_payload(&mut cursor, end)
        };
        Some(Decorator {
            name,
            span: self.source_span_for_range(start, end),
            payload,
        })
    }

    fn parse_source_decorator_payload(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> SourceDecorator {
        let provider = self.parse_qualified_name(cursor, end);
        let mut arguments = Vec::new();
        let mut options = None;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with") {
                *cursor = index + 1;
                options = self.parse_record_expr(cursor, end);
                break;
            }
            let Some(argument) = self.parse_decorator_argument(cursor, end) else {
                break;
            };
            arguments.push(argument);
        }

        SourceDecorator {
            provider,
            arguments,
            options,
        }
    }

    fn parse_generic_decorator_payload(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> DecoratorPayload {
        let mut arguments = Vec::new();
        let mut options = None;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with") {
                *cursor = index + 1;
                options = self.parse_record_expr(cursor, end);
                break;
            }
            let Some(argument) = self.parse_decorator_argument(cursor, end) else {
                break;
            };
            arguments.push(argument);
        }

        if arguments.is_empty() && options.is_none() {
            DecoratorPayload::Bare
        } else {
            DecoratorPayload::Arguments(DecoratorArguments { arguments, options })
        }
    }

    fn parse_decorator_argument(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let mut expr = self.parse_atomic_expr(cursor, end, ExprStop::default())?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with")
                || self.tokens[index].line_start()
                || self.tokens[index].kind().is_pipe_operator()
                || !self.starts_expr(index)
            {
                break;
            }
            let argument = self.parse_atomic_expr(cursor, end, ExprStop::default())?;
            expr = self.make_apply_expr(expr, argument);
        }
        Some(expr)
    }

    fn parse_named_item_name(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
        description: &str,
    ) -> Option<Identifier> {
        let name = self.parse_identifier(cursor, end);
        if name.is_none() {
            self.diagnostics.push(
                Diagnostic::error(format!("{description} is missing its name"))
                    .with_code(MISSING_ITEM_NAME)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        format!(
                            "expected a name after `{}`",
                            self.tokens[keyword_index].text(self.source)
                        ),
                    ),
            );
        }
        name
    }

    fn parse_optional_type_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<TypeExpr> {
        self.consume_kind(cursor, end, TokenKind::Colon)?;
        self.parse_type_expr(cursor, end, TypeStop::default())
    }

    fn parse_optional_signature_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        if self.consume_kind(cursor, end, TokenKind::Colon).is_none() {
            return (Vec::new(), None);
        }
        self.parse_constrained_type(cursor, end)
    }

    fn parse_function_signature_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        if self.peek_kind(*cursor, end) != Some(TokenKind::Colon) {
            return (Vec::new(), None);
        }
        let Some(body_arrow) = self.find_last_same_line_arrow(*cursor, end) else {
            return self.parse_optional_signature_annotation(cursor, end);
        };
        for split in self.function_signature_split_candidates(*cursor, body_arrow) {
            if self.probe_function_signature(split, *cursor, body_arrow) {
                return self.parse_optional_signature_annotation(cursor, split);
            }
        }
        if let Some(body_equals) = self.find_same_line_top_level_equals(*cursor, body_arrow) {
            return self.parse_optional_signature_annotation(cursor, body_equals);
        }
        if let Some(body_equals) = self.find_same_line_top_level_equals(*cursor, end) {
            return self.parse_optional_signature_annotation(cursor, body_equals);
        }
        self.parse_optional_signature_annotation(cursor, body_arrow)
    }

    fn parse_constrained_type(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self.consume_constraint_separator(cursor, end)
        {
            return (
                constraints,
                self.parse_type_expr(cursor, end, TypeStop::default()),
            );
        }
        *cursor = checkpoint;
        (
            Vec::new(),
            self.parse_type_expr(cursor, end, TypeStop::default()),
        )
    }

    fn parse_constraint_list(&mut self, cursor: &mut usize, end: usize) -> Option<Vec<TypeExpr>> {
        if self.peek_kind(*cursor, end) == Some(TokenKind::LParen) {
            self.consume_kind(cursor, end, TokenKind::LParen)?;
            let mut constraints = Vec::new();
            loop {
                constraints.push(self.parse_type_expr(cursor, end, TypeStop::paren_context())?);
                if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                    continue;
                }
                self.consume_kind(cursor, end, TokenKind::RParen)?;
                break;
            }
            return Some(constraints);
        }
        Some(vec![self.parse_type_expr(
            cursor,
            end,
            TypeStop::constraint_attempt(),
        )?])
    }

    /// Returns `true` when `expr` looks like a class constraint — i.e. a type
    /// application whose callee is a multi-character identifier.  Single-letter
    /// identifiers are type variables by convention in AIVI and therefore not
    /// valid class names.  Parenthesised constraint tuples are handled by the
    /// caller and do not reach this predicate.
    fn is_constraint_expr(expr: &TypeExpr) -> bool {
        match &expr.kind {
            TypeExprKind::Apply { callee, .. } => match &callee.kind {
                TypeExprKind::Name(name) => name.text.len() > 1,
                _ => false,
            },
            _ => false,
        }
    }

    fn parse_optional_constraint_prefix(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<TypeExpr> {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self.consume_constraint_separator(cursor, end)
        {
            return constraints;
        }
        *cursor = checkpoint;
        Vec::new()
    }

    fn parse_type_parameters_same_line(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<Identifier> {
        let mut type_parameters = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() || self.tokens[index].kind() != TokenKind::Identifier
            {
                break;
            }
            type_parameters.push(self.identifier_from_token(index));
            *cursor = index + 1;
        }
        type_parameters
    }

    fn parse_class_head(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> (Option<Identifier>, Vec<Identifier>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self
                .consume_kind(cursor, end, TokenKind::ThinArrow)
                .is_some()
        {
            self.diagnostics.push(
                Diagnostic::error("class declarations do not accept head constraint prefixes")
                    .with_code(UNSUPPORTED_CLASS_HEAD_CONSTRAINTS)
                    .with_primary_label(
                        self.source_span_for_range(checkpoint, *cursor),
                        "move these constraints into indented `with ...` lines inside the class body",
                    )
                    .with_note("supported form: `class Applicative F` followed by `with Functor F`"),
            );
            let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
            let type_parameters = self.parse_type_parameters_same_line(cursor, end);
            return (name, type_parameters);
        }
        *cursor = checkpoint;
        let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
        let type_parameters = self.parse_type_parameters_same_line(cursor, end);
        (name, type_parameters)
    }

    fn parse_function_param(&mut self, cursor: &mut usize, end: usize) -> Option<FunctionParam> {
        let name_index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_function_param_token(name_index) {
            return None;
        }
        *cursor = name_index + 1;
        let identifier = self.identifier_from_token(name_index);
        let name = (identifier.text != "_").then_some(identifier);
        let annotation = if self.peek_kind(*cursor, end) == Some(TokenKind::Colon) {
            let annotation_end = self.parameter_annotation_end(*cursor, end);
            let annotation = self.parse_optional_type_annotation(cursor, annotation_end);
            let annotation_span = annotation
                .as_ref()
                .map(|annotation| annotation.span)
                .unwrap_or_else(|| self.source_span_of_token(name_index));
            self.diagnostics.push(
                Diagnostic::warning("function parameters no longer accept inline type annotations")
                    .with_code(DIRECT_FUNCTION_PARAMETER_ANNOTATION)
                    .with_primary_label(
                        annotation_span,
                        "prefer moving this type into the function signature annotation",
                    )
                    .with_secondary_label(
                        self.source_span_of_token(name_index),
                        "leave the parameter name unannotated here",
                    ),
            );
            annotation
        } else {
            None
        };
        Some(FunctionParam {
            name,
            annotation,
            span: self.source_span_for_range(name_index, *cursor),
        })
    }

    fn starts_unary_subject_function(&self, start: usize, end: usize) -> bool {
        let Some(index) = self.peek_nontrivia(start, end) else {
            return false;
        };
        match self.tokens[index].kind() {
            TokenKind::Dot => true,
            TokenKind::StringLiteral => self.tokens[index].text(self.source).contains('{'),
            _ => false,
        }
    }

    fn parse_unary_subject_function_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<(Vec<FunctionParam>, Option<NamedItemBody>)> {
        let head_start = self.peek_nontrivia(*cursor, end)?;
        if !matches!(
            self.tokens[head_start].kind(),
            TokenKind::Dot | TokenKind::StringLiteral | TokenKind::LBrace
        ) {
            return None;
        }
        let checkpoint = *cursor;
        let parameter =
            self.implicit_function_subject_parameter_at(self.source_span_of_token(head_start));
        let head = self.parse_range_expr(cursor, end, ExprStop::default().with_pipe_stage())?;
        let (head, rewrote_subject) =
            self.rewrite_free_function_subject_expr(head, &parameter, false);
        if !rewrote_subject {
            *cursor = checkpoint;
            return None;
        }
        let body = self
            .parse_subject_root_expr_from_head(head_start, head, cursor, end, ExprStop::default())
            .and_then(|expr| self.finish_expression_body(cursor, end, "func declaration", expr))
            .or_else(|| {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body after `=`",
                    "expected an expression using `.` or parameters followed by `=>`",
                );
                None
            });
        Some((vec![parameter], body))
    }

    fn parse_selected_subject_function_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
    ) -> Option<(Vec<FunctionParam>, Option<NamedItemBody>)> {
        let checkpoint = *cursor;
        let mut parameters = Vec::new();
        let mut parameter_starts = Vec::new();
        let mut selected_head: Option<SubjectPickHead> = None;

        while self.starts_function_param(*cursor, end) {
            let Some((parameter, start_index, selected)) =
                self.parse_subject_pick_function_param(cursor, end)
            else {
                break;
            };
            if selected {
                if selected_head.is_some() {
                    self.emit_invalid_subject_pick(
                        parameter.span,
                        "function headers can only choose one subject with `!`",
                        "remove this `!` or the earlier subject pick",
                    );
                } else if let Some(name) = parameter.name.as_ref() {
                    selected_head = Some(SubjectPickHead {
                        expr: Expr {
                            span: name.span,
                            kind: ExprKind::Name(name.clone()),
                        },
                        start_index,
                    });
                } else {
                    self.emit_invalid_subject_pick(
                        parameter.span,
                        "the discard `_` cannot be the selected subject",
                        "choose a named parameter before `!`",
                    );
                }
            }
            parameter_starts.push(start_index);
            parameters.push(parameter);
        }

        if let Some(selector) = self.parse_subject_pick_record_selector(cursor, end) {
            if selected_head.is_some() {
                self.emit_invalid_subject_pick(
                    selector.span,
                    "function headers can only choose one subject with `!`",
                    "remove either this `{ ...! }` selector or the earlier `!` marker",
                );
            } else if let Some((base_index, base_parameter)) = parameters
                .iter()
                .enumerate()
                .rev()
                .find(|(_, parameter)| parameter.name.is_some())
            {
                let base_name = base_parameter
                    .name
                    .as_ref()
                    .expect("filtered parameter must keep its name")
                    .clone();
                let base_expr = Expr {
                    span: base_name.span,
                    kind: ExprKind::Name(base_name),
                };
                let head_span = self.join_spans(base_expr.span, selector.span);
                selected_head = Some(SubjectPickHead {
                    expr: Expr {
                        span: head_span,
                        kind: ExprKind::Projection {
                            base: Box::new(base_expr),
                            path: selector,
                        },
                    },
                    start_index: parameter_starts[base_index],
                });
            } else {
                self.emit_invalid_subject_pick(
                    selector.span,
                    "record subject selectors need a preceding named parameter",
                    "write a named parameter before this `{ ...! }` selector",
                );
                *cursor = checkpoint;
                return None;
            }
        }

        let Some(head) = selected_head else {
            *cursor = checkpoint;
            return None;
        };

        let body = self
            .parse_subject_root_expr_from_head(
                head.start_index,
                head.expr,
                cursor,
                end,
                ExprStop::default(),
            )
            .and_then(|expr| self.finish_expression_body(cursor, end, declaration_name, expr));
        Some((parameters, body))
    }

    fn parse_subject_pick_function_param(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<(FunctionParam, usize, bool)> {
        let start_index = self.peek_nontrivia(*cursor, end)?;
        let parameter = self.parse_function_param(cursor, end)?;
        let selected = self.consume_kind(cursor, end, TokenKind::Bang).is_some();
        Some((parameter, start_index, selected))
    }

    fn parse_subject_pick_record_selector(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ProjectionPath> {
        let checkpoint = *cursor;
        let open_brace = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[open_brace].line_start()
            || self.tokens[open_brace].kind() != TokenKind::LBrace
        {
            return None;
        }
        *cursor = open_brace + 1;
        let first = match self.parse_identifier(cursor, end) {
            Some(first) => first,
            None => {
                *cursor = checkpoint;
                return None;
            }
        };
        let mut fields = vec![first];
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            let Some(segment) = self.parse_identifier(cursor, end) else {
                *cursor = checkpoint;
                return None;
            };
            fields.push(segment);
        }
        let Some(bang_index) = self.consume_kind(cursor, end, TokenKind::Bang) else {
            *cursor = checkpoint;
            return None;
        };
        let Some(close_brace) = self.consume_kind(cursor, end, TokenKind::RBrace) else {
            *cursor = checkpoint;
            return None;
        };
        let path_start = fields
            .first()
            .map(|field| field.span.span().start())
            .unwrap_or_else(|| self.tokens[open_brace].span().start());
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(path_start, self.tokens[bang_index].span().end()),
        );
        let _ = self.source_span_for_range(open_brace, close_brace + 1);
        Some(ProjectionPath { span, fields })
    }

    fn emit_invalid_subject_pick(&mut self, span: SourceSpan, message: &str, label: &str) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(INVALID_SUBJECT_PICK)
                .with_primary_label(span, label),
        );
    }

    fn implicit_function_subject_parameter_at(&self, span: SourceSpan) -> FunctionParam {
        FunctionParam {
            name: Some(Identifier {
                text: IMPLICIT_FUNCTION_SUBJECT_NAME.to_owned(),
                span,
            }),
            annotation: None,
            span,
        }
    }

    fn implicit_function_subject_expr_at(
        &self,
        parameter: &FunctionParam,
        span: SourceSpan,
    ) -> Expr {
        let name = parameter
            .name
            .clone()
            .expect("implicit unary-function parameter should keep its name");
        Expr {
            span,
            kind: ExprKind::Name(name),
        }
    }

    fn rewrite_free_function_subject_expr(
        &self,
        expr: Expr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (Expr, bool) {
        let span = expr.span;
        match expr.kind {
            ExprKind::SubjectPlaceholder if !ambient_allowed => (
                self.implicit_function_subject_expr_at(parameter, span),
                true,
            ),
            ExprKind::AmbientProjection(path) if !ambient_allowed => (
                Expr {
                    span,
                    kind: ExprKind::Projection {
                        base: Box::new(self.implicit_function_subject_expr_at(parameter, span)),
                        path,
                    },
                },
                true,
            ),
            ExprKind::Group(inner) => {
                let (inner, changed) =
                    self.rewrite_free_function_subject_expr(*inner, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Group(Box::new(inner)),
                    },
                    changed,
                )
            }
            ExprKind::Tuple(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Tuple(elements),
                    },
                    changed,
                )
            }
            ExprKind::List(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::List(elements),
                    },
                    changed,
                )
            }
            ExprKind::Map(map) => {
                let (map, changed) =
                    self.rewrite_free_function_subject_map_expr(map, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Map(map),
                    },
                    changed,
                )
            }
            ExprKind::Set(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Set(elements),
                    },
                    changed,
                )
            }
            ExprKind::Record(record) => {
                // Detect record projection pattern: { field: . } or { a.b.c: . }
                // When a field value is SubjectPlaceholder, this extracts a field from
                // the subject rather than constructing a record.
                if !ambient_allowed {
                    if let Some(proj_field) = record.fields.iter().find(|f| {
                        matches!(
                            f.value.as_ref().map(|v| &v.kind),
                            Some(ExprKind::SubjectPlaceholder)
                        )
                    }) {
                        let mut fields = vec![proj_field.label.clone()];
                        fields.extend(proj_field.label_path.iter().cloned());
                        let path = ProjectionPath {
                            span: proj_field.span,
                            fields,
                        };
                        return (
                            Expr {
                                span,
                                kind: ExprKind::Projection {
                                    base: Box::new(
                                        self.implicit_function_subject_expr_at(parameter, span),
                                    ),
                                    path,
                                },
                            },
                            true,
                        );
                    }
                }
                let (record, changed) = self.rewrite_free_function_subject_record_expr(
                    record,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Record(record),
                    },
                    changed,
                )
            }
            ExprKind::Text(text) => {
                let (text, changed) = self.rewrite_free_function_subject_text_literal(
                    text,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Text(text),
                    },
                    changed,
                )
            }
            ExprKind::Range { start, end } => {
                let (start, start_changed) =
                    self.rewrite_free_function_subject_expr(*start, parameter, ambient_allowed);
                let (end, end_changed) =
                    self.rewrite_free_function_subject_expr(*end, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Range {
                            start: Box::new(start),
                            end: Box::new(end),
                        },
                    },
                    start_changed || end_changed,
                )
            }
            ExprKind::Projection { base, path } => {
                let (base, changed) =
                    self.rewrite_free_function_subject_expr(*base, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Projection {
                            base: Box::new(base),
                            path,
                        },
                    },
                    changed,
                )
            }
            ExprKind::Apply { callee, arguments } => {
                let (callee, callee_changed) =
                    self.rewrite_free_function_subject_expr(*callee, parameter, ambient_allowed);
                let mut changed = callee_changed;
                let arguments = arguments
                    .into_iter()
                    .map(|argument| {
                        let (argument, argument_changed) = self.rewrite_free_function_subject_expr(
                            argument,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= argument_changed;
                        argument
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Apply {
                            callee: Box::new(callee),
                            arguments,
                        },
                    },
                    changed,
                )
            }
            ExprKind::Unary { operator, expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(*expr, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Unary {
                            operator,
                            expr: Box::new(expr),
                        },
                    },
                    changed,
                )
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let (left, left_changed) =
                    self.rewrite_free_function_subject_expr(*left, parameter, ambient_allowed);
                let (right, right_changed) =
                    self.rewrite_free_function_subject_expr(*right, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Binary {
                            left: Box::new(left),
                            operator,
                            right: Box::new(right),
                        },
                    },
                    left_changed || right_changed,
                )
            }
            ExprKind::ResultBlock(block) => {
                let (block, changed) = self.rewrite_free_function_subject_result_block(
                    block,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::ResultBlock(block),
                    },
                    changed,
                )
            }
            ExprKind::PatchApply { target, patch } => {
                let (target, target_changed) =
                    self.rewrite_free_function_subject_expr(*target, parameter, ambient_allowed);
                let (patch, patch_changed) = self.rewrite_free_function_subject_patch_block(
                    patch,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::PatchApply {
                            target: Box::new(target),
                            patch,
                        },
                    },
                    target_changed || patch_changed,
                )
            }
            ExprKind::PatchLiteral(patch) => {
                let (patch, changed) = self.rewrite_free_function_subject_patch_block(
                    patch,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::PatchLiteral(patch),
                    },
                    changed,
                )
            }
            ExprKind::Pipe(pipe) => {
                let (pipe, changed) =
                    self.rewrite_free_function_subject_pipe_expr(pipe, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Pipe(pipe),
                    },
                    changed,
                )
            }
            ExprKind::Markup(node) => {
                let (node, changed) = self.rewrite_free_function_subject_markup_node(
                    node,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Markup(node),
                    },
                    changed,
                )
            }
            kind => (Expr { span, kind }, false),
        }
    }

    fn rewrite_free_function_subject_text_literal(
        &self,
        text: TextLiteral,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (TextLiteral, bool) {
        let mut changed = false;
        let segments = text
            .segments
            .into_iter()
            .map(|segment| match segment {
                TextSegment::Text(fragment) => TextSegment::Text(fragment),
                TextSegment::Interpolation(interpolation) => {
                    let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                        *interpolation.expr,
                        parameter,
                        ambient_allowed,
                    );
                    changed |= expr_changed;
                    TextSegment::Interpolation(TextInterpolation {
                        expr: Box::new(expr),
                        span: interpolation.span,
                    })
                }
            })
            .collect();
        (
            TextLiteral {
                span: text.span,
                segments,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_record_expr(
        &self,
        record: RecordExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (RecordExpr, bool) {
        let mut changed = false;
        let fields = record
            .fields
            .into_iter()
            .map(|field| {
                let value = field.value.map(|value| {
                    let (value, value_changed) =
                        self.rewrite_free_function_subject_expr(value, parameter, ambient_allowed);
                    changed |= value_changed;
                    value
                });
                RecordField {
                    label: field.label,
                    label_path: field.label_path,
                    value,
                    span: field.span,
                }
            })
            .collect();
        (
            RecordExpr {
                fields,
                span: record.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_map_expr(
        &self,
        map: MapExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (MapExpr, bool) {
        let mut changed = false;
        let entries = map
            .entries
            .into_iter()
            .map(|entry| {
                let (key, key_changed) =
                    self.rewrite_free_function_subject_expr(entry.key, parameter, ambient_allowed);
                let (value, value_changed) = self.rewrite_free_function_subject_expr(
                    entry.value,
                    parameter,
                    ambient_allowed,
                );
                changed |= key_changed || value_changed;
                MapExprEntry {
                    key,
                    value,
                    span: entry.span,
                }
            })
            .collect();
        (
            MapExpr {
                entries,
                span: map.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_result_block(
        &self,
        block: ResultBlockExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (ResultBlockExpr, bool) {
        let mut changed = false;
        let bindings = block
            .bindings
            .into_iter()
            .map(|binding| {
                let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                    binding.expr,
                    parameter,
                    ambient_allowed,
                );
                changed |= expr_changed;
                ResultBinding {
                    name: binding.name,
                    expr,
                    span: binding.span,
                }
            })
            .collect();
        let tail = block.tail.map(|tail| {
            let (tail, tail_changed) =
                self.rewrite_free_function_subject_expr(*tail, parameter, ambient_allowed);
            changed |= tail_changed;
            Box::new(tail)
        });
        (
            ResultBlockExpr {
                bindings,
                tail,
                span: block.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_patch_block(
        &self,
        patch: PatchBlock,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (PatchBlock, bool) {
        let mut changed = false;
        let entries = patch
            .entries
            .into_iter()
            .map(|entry| {
                let segments = entry
                    .selector
                    .segments
                    .into_iter()
                    .map(|segment| match segment {
                        PatchSelectorSegment::BracketExpr { expr, span } => {
                            let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                                *expr,
                                parameter,
                                ambient_allowed,
                            );
                            changed |= expr_changed;
                            PatchSelectorSegment::BracketExpr {
                                expr: Box::new(expr),
                                span,
                            }
                        }
                        other => other,
                    })
                    .collect();
                let instruction_kind = match entry.instruction.kind {
                    PatchInstructionKind::Replace(expr) => {
                        let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                            *expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= expr_changed;
                        PatchInstructionKind::Replace(Box::new(expr))
                    }
                    PatchInstructionKind::Store(expr) => {
                        let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                            *expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= expr_changed;
                        PatchInstructionKind::Store(Box::new(expr))
                    }
                    PatchInstructionKind::Remove => PatchInstructionKind::Remove,
                };
                PatchEntry {
                    selector: PatchSelector {
                        segments,
                        span: entry.selector.span,
                    },
                    instruction: PatchInstruction {
                        kind: instruction_kind,
                        span: entry.instruction.span,
                    },
                    span: entry.span,
                }
            })
            .collect();
        (
            PatchBlock {
                entries,
                span: patch.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_pipe_expr(
        &self,
        pipe: PipeExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (PipeExpr, bool) {
        let mut changed = false;
        let head = pipe.head.map(|head| {
            let (head, head_changed) =
                self.rewrite_free_function_subject_expr(*head, parameter, ambient_allowed);
            changed |= head_changed;
            Box::new(head)
        });
        let stages = pipe
            .stages
            .into_iter()
            .map(|stage| {
                let (stage, stage_changed) =
                    self.rewrite_free_function_subject_pipe_stage(stage, parameter);
                changed |= stage_changed;
                stage
            })
            .collect();
        (
            PipeExpr {
                head,
                stages,
                span: pipe.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_pipe_stage(
        &self,
        stage: PipeStage,
        parameter: &FunctionParam,
    ) -> (PipeStage, bool) {
        let (kind, changed) = match stage.kind {
            PipeStageKind::Transform { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Transform { expr }, changed)
            }
            PipeStageKind::Gate { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Gate { expr }, changed)
            }
            PipeStageKind::Case(arm) => {
                let (body, changed) =
                    self.rewrite_free_function_subject_expr(arm.body, parameter, true);
                (
                    PipeStageKind::Case(PipeCaseArm {
                        pattern: arm.pattern,
                        body,
                        span: arm.span,
                    }),
                    changed,
                )
            }
            PipeStageKind::Map { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Map { expr }, changed)
            }
            PipeStageKind::Apply { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Apply { expr }, changed)
            }
            PipeStageKind::ClusterFinalizer { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::ClusterFinalizer { expr }, changed)
            }
            PipeStageKind::RecurStart { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::RecurStart { expr }, changed)
            }
            PipeStageKind::RecurStep { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::RecurStep { expr }, changed)
            }
            PipeStageKind::Tap { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Tap { expr }, changed)
            }
            PipeStageKind::FanIn { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::FanIn { expr }, changed)
            }
            PipeStageKind::Truthy { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Truthy { expr }, changed)
            }
            PipeStageKind::Falsy { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Falsy { expr }, changed)
            }
            PipeStageKind::Validate { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Validate { expr }, changed)
            }
            PipeStageKind::Previous { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Previous { expr }, changed)
            }
            PipeStageKind::Accumulate { seed, step } => {
                let (seed, seed_changed) =
                    self.rewrite_free_function_subject_expr(seed, parameter, true);
                let (step, step_changed) =
                    self.rewrite_free_function_subject_expr(step, parameter, true);
                (
                    PipeStageKind::Accumulate { seed, step },
                    seed_changed || step_changed,
                )
            }
            PipeStageKind::Diff { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Diff { expr }, changed)
            }
            PipeStageKind::Delay { duration } => {
                let (duration, changed) =
                    self.rewrite_free_function_subject_expr(duration, parameter, true);
                (PipeStageKind::Delay { duration }, changed)
            }
            PipeStageKind::Burst { every, count } => {
                let (every, every_changed) =
                    self.rewrite_free_function_subject_expr(every, parameter, true);
                let (count, count_changed) =
                    self.rewrite_free_function_subject_expr(count, parameter, true);
                (
                    PipeStageKind::Burst { every, count },
                    every_changed || count_changed,
                )
            }
        };
        (
            PipeStage {
                subject_memo: stage.subject_memo,
                result_memo: stage.result_memo,
                kind,
                span: stage.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_markup_node(
        &self,
        node: MarkupNode,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (MarkupNode, bool) {
        let mut changed = false;
        let attributes = node
            .attributes
            .into_iter()
            .map(|attribute| {
                let value = attribute.value.map(|value| match value {
                    MarkupAttributeValue::Text(text) => {
                        let (text, value_changed) = self
                            .rewrite_free_function_subject_text_literal(
                                text,
                                parameter,
                                ambient_allowed,
                            );
                        changed |= value_changed;
                        MarkupAttributeValue::Text(text)
                    }
                    MarkupAttributeValue::Expr(expr) => {
                        let (expr, value_changed) = self.rewrite_free_function_subject_expr(
                            expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= value_changed;
                        MarkupAttributeValue::Expr(expr)
                    }
                    MarkupAttributeValue::Pattern(pattern) => {
                        MarkupAttributeValue::Pattern(pattern)
                    }
                });
                MarkupAttribute {
                    name: attribute.name,
                    value,
                    span: attribute.span,
                }
            })
            .collect();
        let children = node
            .children
            .into_iter()
            .map(|child| {
                let (child, child_changed) = self.rewrite_free_function_subject_markup_node(
                    child,
                    parameter,
                    ambient_allowed,
                );
                changed |= child_changed;
                child
            })
            .collect();
        (
            MarkupNode {
                name: node.name,
                attributes,
                children,
                close_name: node.close_name,
                self_closing: node.self_closing,
                span: node.span,
            },
            changed,
        )
    }

    fn parse_type_decl_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeDeclBody> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[index].kind() == TokenKind::LBrace {
            let inner_end = self.find_matching_brace(index, end).unwrap_or(end);
            if let Some(first_inside) = self.peek_nontrivia(index + 1, inner_end) {
                if self.tokens[first_inside].kind() == TokenKind::PipeTap {
                    return self
                        .parse_sum_type_block_body(cursor, end)
                        .map(TypeDeclBody::Sum);
                }
            }
        }
        if self.tokens[index].kind() == TokenKind::PipeTap {
            return self.parse_sum_type_body(cursor, end);
        }
        if self.tokens[index].kind() == TokenKind::Identifier {
            let identifier = self.identifier_from_token(index);
            if identifier.is_uppercase_initial() && !is_record_row_transform_name(&identifier.text)
            {
                if let Some(next_index) = self.peek_nontrivia(index + 1, end) {
                    if self.tokens[next_index].kind() == TokenKind::PipeTap
                        || (self.starts_type_atom(next_index)
                            && !self.tokens[next_index].line_start())
                    {
                        return self.parse_sum_type_body(cursor, end);
                    }
                }
            }
        }
        self.parse_type_expr(cursor, end, TypeStop::default())
            .map(TypeDeclBody::Alias)
    }

    fn parse_sum_type_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeDeclBody> {
        let start = *cursor;
        let variants = self.parse_sum_type_variants(cursor, end);
        (!variants.is_empty()).then_some(TypeDeclBody::Sum(TypeSumBody {
            variants,
            companions: Vec::new(),
            span: self.source_span_for_range(start, *cursor),
        }))
    }

    fn parse_sum_type_block_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeSumBody> {
        let lbrace = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let inner_end = self.find_matching_brace(lbrace, end).unwrap_or(end);
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&index| self.tokens[index].line_start())
            .map(|index| self.line_indent_of_token(index))
            .unwrap_or(0);
        let variants = self.parse_sum_type_variants(cursor, inner_end);
        if variants.is_empty() {
            *cursor = inner_end.saturating_add(1);
            return None;
        }

        let mut companions = Vec::new();
        let mut pending_annotation: Option<TypeExpr> = None;
        let mut pending_colon_member: Option<TypeCompanionMember> = None;

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            if self.tokens[index].line_start()
                && member_indent != 0
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if self.tokens[index].kind() == TokenKind::PipeTap {
                self.diagnostics.push(
                    Diagnostic::error("sum constructors must come before companion members")
                        .with_code(MISSING_DECLARATION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(index),
                            "move this constructor above the companion bindings",
                        ),
                );
                break;
            }
            if !self.starts_type_companion_member(index) {
                break;
            }

            if self.tokens[index].kind() == TokenKind::TypeKw {
                if let Some(held) = pending_colon_member.take() {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "type companion member `{}` is missing its binding",
                            held.name.text
                        ))
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            held.span,
                            "expected a binding line after this companion type annotation",
                        ),
                    );
                }
                *cursor = index + 1;
                let type_end = self
                    .find_next_type_companion_member_start(*cursor, inner_end, member_indent)
                    .unwrap_or(inner_end);
                let annotation = self
                    .parse_type_expr(cursor, type_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion annotation is missing its type after `type`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_TYPE)
                            .with_primary_label(
                                self.source_span_for_range(index, *cursor),
                                "expected a type such as `Player -> Text`",
                            ),
                        );
                        None
                    });
                *cursor = type_end;
                pending_annotation = annotation;
                continue;
            }

            let before = *cursor;
            let Some(mut member) =
                self.parse_type_companion_member(cursor, inner_end, member_indent)
            else {
                break;
            };

            if let Some(annotation) = pending_annotation.take() {
                member.annotation = Some(annotation);
            }

            if member.annotation.is_none() {
                if let Some(held) = pending_colon_member.take() {
                    if held.name.text == member.name.text {
                        member.annotation = held.annotation;
                    } else {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "type companion member `{}` is missing its binding",
                                held.name.text
                            ))
                            .with_code(MISSING_TYPE_COMPANION_BODY)
                            .with_primary_label(
                                held.span,
                                "expected a binding line with the same companion name",
                            ),
                        );
                    }
                }
            }

            if member.annotation.is_some() && member.body.is_none() && member.parameters.is_empty()
            {
                pending_colon_member = Some(member);
                if *cursor <= before {
                    break;
                }
                continue;
            }

            companions.push(member);
            if *cursor <= before {
                break;
            }
        }

        if let Some(held) = pending_colon_member.take() {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "type companion member `{}` is missing its binding",
                    held.name.text
                ))
                .with_code(MISSING_TYPE_COMPANION_BODY)
                .with_primary_label(
                    held.span,
                    "expected a binding line after this companion type annotation",
                ),
            );
        }

        if let Some(annotation) = pending_annotation {
            self.diagnostics.push(
                Diagnostic::error("type companion annotation has no following member")
                    .with_code(MISSING_TYPE_COMPANION_NAME)
                    .with_primary_label(
                        annotation.span,
                        "expected a companion binding after this type annotation",
                    ),
            );
        }

        *cursor = inner_end.saturating_add(1);
        Some(TypeSumBody {
            variants,
            companions,
            span: self.source_span_for_range(lbrace, *cursor),
        })
    }

    fn parse_sum_type_variants(&mut self, cursor: &mut usize, end: usize) -> Vec<TypeVariant> {
        let mut variants = Vec::new();

        loop {
            let _ = self.consume_kind(cursor, end, TokenKind::PipeTap);
            let variant_start = *cursor;
            let name = self.parse_identifier(cursor, end);
            if name.is_none() {
                break;
            }

            let mut fields = Vec::new();
            while let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.tokens[index].line_start()
                    || self.tokens[index].kind() == TokenKind::PipeTap
                    || !self.starts_type_atom(index)
                {
                    break;
                }
                let Some(field) = self.parse_type_variant_field(cursor, end) else {
                    break;
                };
                fields.push(field);
            }

            variants.push(TypeVariant {
                name,
                fields,
                span: self.source_span_for_range(variant_start, *cursor),
            });

            if self.peek_kind(*cursor, end) != Some(TokenKind::PipeTap) {
                break;
            }
        }

        variants
    }

    fn parse_type_companion_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<TypeCompanionMember> {
        let start = *cursor;
        let name_index = self.peek_nontrivia(*cursor, end).unwrap_or(start);
        let Some(name) = self.parse_identifier(cursor, end) else {
            self.diagnostics.push(
                Diagnostic::error("type companion member is missing its name")
                    .with_code(MISSING_TYPE_COMPANION_NAME)
                    .with_primary_label(
                        self.source_span_for_range(start, *cursor),
                        "expected a companion name such as `label` or `opponent`",
                    ),
            );
            return None;
        };
        let name_span = name.span;
        let mut annotation = None;

        if let Some(colon_idx) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[colon_idx].kind() == TokenKind::Colon
                && !self.tokens[colon_idx].line_start()
            {
                *cursor = colon_idx + 1;
                let ann_end = self
                    .find_same_line_top_level_equals(*cursor, end)
                    .or_else(|| {
                        self.find_next_type_companion_member_start(*cursor, end, member_indent)
                    })
                    .unwrap_or(end);
                annotation = self
                    .parse_type_expr(cursor, ann_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion member is missing its type after `:`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_TYPE)
                            .with_primary_label(
                                name_span,
                                "expected a companion type such as `Player -> Player`",
                            ),
                        );
                        None
                    });
                *cursor = ann_end;
                if self.peek_kind(*cursor, end) != Some(TokenKind::Equals) {
                    return Some(TypeCompanionMember {
                        name,
                        annotation,
                        function_form: FunctionSurfaceForm::Explicit,
                        parameters: Vec::new(),
                        body: None,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
            }
        }

        let Some(eq_index) = self.consume_kind(cursor, end, TokenKind::Equals) else {
            self.diagnostics.push(
                Diagnostic::error("type companion member is missing `=` before its body")
                    .with_code(MISSING_TYPE_COMPANION_BODY)
                    .with_primary_label(name.span, "write `name = self => ...` or `name = . ...`"),
            );
            return None;
        };
        let member_end = self
            .find_next_type_companion_member_start(*cursor, end, member_indent)
            .unwrap_or(end);

        let (function_form, parameters, body) = if let Some((parameters, body)) =
            self.parse_unary_subject_function_body(name_index, cursor, member_end)
        {
            (
                FunctionSurfaceForm::UnarySubjectSugar,
                parameters,
                body.and_then(|body| match body {
                    NamedItemBody::Expr(expr) => Some(expr),
                    _ => None,
                }),
            )
        } else if let Some((parameters, body)) =
            self.parse_selected_subject_function_body(cursor, member_end, "type companion member")
        {
            (
                FunctionSurfaceForm::SelectedSubjectSugar,
                parameters,
                body.and_then(|body| match body {
                    NamedItemBody::Expr(expr) => Some(expr),
                    _ => None,
                }),
            )
        } else {
            let mut parameters = Vec::new();
            while self.starts_function_param(*cursor, member_end) {
                let Some(parameter) = self.parse_function_param(cursor, member_end) else {
                    break;
                };
                parameters.push(parameter);
            }

            let body = if let Some(arrow_index) =
                self.consume_kind(cursor, member_end, TokenKind::Arrow)
            {
                let body = self
                    .parse_expr(cursor, member_end, ExprStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion member is missing its body after `=>`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_BODY)
                            .with_primary_label(
                                self.source_span_of_token(arrow_index),
                                "expected an expression body for this companion member",
                            ),
                        );
                        None
                    })
                    .and_then(|expr| {
                        self.finish_expression_body(
                            cursor,
                            member_end,
                            "type companion member",
                            expr,
                        )
                        .and_then(|body| match body {
                            NamedItemBody::Expr(expr) => Some(expr),
                            _ => None,
                        })
                    });
                if parameters.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "type companion members require an explicit parameter before `=>`",
                        )
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(arrow_index),
                            "insert a parameter such as `self` before `=>`",
                        )
                        .with_note(
                            "receiver-only helpers can use the shortcut `name = .field` or `name = .`",
                        ),
                    );
                }
                body
            } else {
                self.diagnostics.push(
                    Diagnostic::error(
                        "type companion member body must use `name = self => ...` or `name = . ...`",
                    )
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(eq_index),
                            "expected `.` or parameters followed by `=>`",
                        ),
                );
                None
            };

            (FunctionSurfaceForm::Explicit, parameters, body)
        };

        *cursor = member_end;
        Some(TypeCompanionMember {
            name,
            annotation,
            function_form,
            parameters,
            body,
            span: self.source_span_for_range(start, member_end),
        })
    }

    fn parse_type_variant_field(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<TypeVariantField> {
        let field_start = *cursor;
        // Try named field: `label:Type` (lowercase identifier followed by colon)
        let checkpoint = *cursor;
        if let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].kind() == TokenKind::Identifier {
                let ident = self.identifier_from_token(index);
                if !ident.is_uppercase_initial() {
                    // Peek ahead for colon
                    if let Some(colon_index) = self.peek_nontrivia(index + 1, end) {
                        if self.tokens[colon_index].kind() == TokenKind::Colon {
                            // Consume label and colon
                            let label = self.parse_identifier(cursor, end)?;
                            let _ = self.consume_kind(cursor, end, TokenKind::Colon);
                            let ty = self.parse_type_atom(cursor, end, TypeStop::default())?;
                            return Some(TypeVariantField {
                                label: Some(label),
                                span: self.source_span_for_range(field_start, *cursor),
                                ty,
                            });
                        }
                    }
                }
            }
        }
        *cursor = checkpoint;
        // Fall back to anonymous field
        let ty = self.parse_type_atom(cursor, end, TypeStop::default())?;
        Some(TypeVariantField {
            label: None,
            span: self.source_span_for_range(field_start, *cursor),
            ty,
        })
    }

    fn parse_type_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_type_pipe_expr(cursor, end, stop);
        self.depth_exit();
        result
    }

    fn parse_type_pipe_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let mut ty = self.parse_type_arrow_expr(cursor, end, stop.with_pipe_transform())?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.type_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::PipeTransform
            {
                break;
            }
            *cursor = index + 1;
            let stage = self.parse_type_arrow_expr(cursor, end, stop.with_pipe_transform())?;
            ty = self.make_type_apply(stage, ty);
        }
        Some(ty)
    }

    fn parse_type_arrow_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let parameter = self.parse_type_application_expr(cursor, end, stop);
        parameter.and_then(|parameter| {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                return Some(parameter);
            };
            if self.type_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::ThinArrow
            {
                return Some(parameter);
            }
            *cursor = index + 1;
            let result = self.parse_type_expr(cursor, end, stop)?;
            Some(self.make_type_arrow(parameter, result))
        })
    }

    fn parse_type_application_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let mut ty = self.parse_type_atom(cursor, end, stop)?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.type_should_stop(index, stop) || !self.starts_type_atom(index) {
                break;
            }
            if self.tokens[index].line_start() {
                break;
            }
            let argument = self.parse_type_atom(cursor, end, stop)?;
            ty = self.make_type_apply(ty, argument);
        }
        Some(ty)
    }

    fn parse_type_atom(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.type_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::Identifier => {
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                Some(TypeExpr {
                    span: name.span,
                    kind: TypeExprKind::Name(name),
                })
            }
            TokenKind::LParen => self.parse_grouped_type(cursor, end),
            TokenKind::LBrace => self.parse_record_type(cursor, end),
            _ => None,
        }
    }

    fn parse_grouped_type(&mut self, cursor: &mut usize, end: usize) -> Option<TypeExpr> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(TypeExpr {
                span: self.source_span_for_range(start, *cursor),
                kind: TypeExprKind::Tuple(Vec::new()),
            });
        }

        loop {
            let element = self.parse_type_expr(cursor, end, TypeStop::paren_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                saw_comma = true;
                if self.peek_kind(*cursor, end) == Some(TokenKind::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RParen);
        let span = self.source_span_for_range(start, *cursor);
        Some(if saw_comma || elements.len() != 1 {
            TypeExpr {
                span,
                kind: TypeExprKind::Tuple(elements),
            }
        } else {
            TypeExpr {
                span,
                kind: TypeExprKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_record_type(&mut self, cursor: &mut usize, end: usize) -> Option<TypeExpr> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut fields = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let Some(label) = self.parse_identifier(cursor, end) else {
                break;
            };
            let field_start = label.span.span().start();
            let ty = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_type_expr(cursor, end, TypeStop::record_context())
            } else {
                None
            };
            let field_end = ty
                .as_ref()
                .map(|ty| ty.span.span().end())
                .unwrap_or_else(|| label.span.span().end());
            fields.push(TypeField {
                label,
                ty,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(TypeExpr {
            span: self.source_span_for_range(start, *cursor),
            kind: TypeExprKind::Record(fields),
        })
    }

    fn parse_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_patch_apply_expr(cursor, end, stop);
        self.depth_exit();
        result
    }

    fn parse_patch_apply_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let expr = self.parse_pipe_expr(cursor, end, stop)?;
        Some(self.parse_patch_apply_suffix(expr, cursor, end, stop))
    }

    fn parse_patch_apply_suffix(
        &mut self,
        mut expr: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Expr {
        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::PatchApply
            {
                break;
            }
            *cursor = index + 1;
            let Some(patch) = self.parse_patch_block(cursor, end) else {
                break;
            };
            let span = self.join_spans(expr.span, patch.span);
            expr = Expr {
                span,
                kind: ExprKind::PatchApply {
                    target: Box::new(expr),
                    patch,
                },
            };
        }
        expr
    }

    fn parse_subject_root_expr_from_head(
        &mut self,
        start: usize,
        head: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let expr = self.parse_pipe_expr_from_head(start, Some(head), cursor, end, stop)?;
        Some(self.parse_patch_apply_suffix(expr, cursor, end, stop))
    }

    fn parse_pipe_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        let start = *cursor;
        let head = if self.peek_kind(*cursor, end) == Some(TokenKind::PipeApply) {
            None
        } else {
            Some(self.parse_range_expr(cursor, end, stop.with_pipe_stage())?)
        };
        self.parse_pipe_expr_from_head(start, head, cursor, end, stop)
    }

    fn parse_pipe_expr_from_head(
        &mut self,
        start: usize,
        head: Option<Expr>,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut head = head.map(Box::new);
        let mut stages = Vec::new();
        let mut cluster_active = false;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.expr_should_stop(index, stop) {
                break;
            }
            let kind = self.tokens[index].kind();
            if !kind.is_pipe_operator() {
                break;
            }
            *cursor = index + 1;
            let (subject_memo, stage_kind, result_memo) = match kind {
                TokenKind::PipeTransform => {
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let temporal_stage = if !cluster_active {
                        self.try_parse_prefixed_temporal_pipe_stage(
                            cursor,
                            end,
                            stop.with_pipe_stage().with_hash(),
                        )?
                    } else {
                        None
                    };
                    if let Some(stage_kind) = temporal_stage {
                        let result_memo = self.parse_optional_pipe_memo(cursor, end);
                        (subject_memo, stage_kind, result_memo)
                    } else {
                        let expr = self.parse_patch_apply_expr(
                            cursor,
                            end,
                            stop.with_pipe_stage().with_hash(),
                        )?;
                        let result_memo = self.parse_optional_pipe_memo(cursor, end);
                        if cluster_active {
                            cluster_active = false;
                            (
                                subject_memo,
                                PipeStageKind::ClusterFinalizer { expr },
                                result_memo,
                            )
                        } else {
                            (subject_memo, PipeStageKind::Transform { expr }, result_memo)
                        }
                    }
                }
                TokenKind::PipeGate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Gate { expr }, result_memo)
                }
                TokenKind::PipeCase => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let arm =
                        self.parse_pipe_case_arm(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Case(arm), result_memo)
                }
                TokenKind::PipeMap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Map { expr }, result_memo)
                }
                TokenKind::PipeApply => {
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    cluster_active = true;
                    (subject_memo, PipeStageKind::Apply { expr }, result_memo)
                }
                TokenKind::PipeRecurStart => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (
                        subject_memo,
                        PipeStageKind::RecurStart { expr },
                        result_memo,
                    )
                }
                TokenKind::PipeRecurStep => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::RecurStep { expr }, result_memo)
                }
                TokenKind::PipeTap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Tap { expr }, result_memo)
                }
                TokenKind::PipeFanIn => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::FanIn { expr }, result_memo)
                }
                TokenKind::TruthyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Truthy { expr }, result_memo)
                }
                TokenKind::FalsyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Falsy { expr }, result_memo)
                }
                TokenKind::PipeValidate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Validate { expr }, result_memo)
                }
                TokenKind::PipePrevious => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Previous { expr }, result_memo)
                }
                TokenKind::PipeAccumulate => {
                    // `signal +|> seed (state input => next)`
                    // The seed expression comes first, then the step function expression.
                    // Parse the seed with atomic-expression boundaries so an adjacent step name
                    // does not get swallowed as an application (`+|> 0 step`).
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let seed =
                        self.parse_atomic_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let step = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (
                        subject_memo,
                        PipeStageKind::Accumulate { seed, step },
                        result_memo,
                    )
                }
                TokenKind::PipeDiff => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Diff { expr }, result_memo)
                }
                TokenKind::PipeDelay => {
                    cluster_active = false;
                    self.emit_removed_temporal_pipe_operator(index, "|> delay <duration>");
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let stage_kind = self.parse_delay_pipe_stage(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, stage_kind, result_memo)
                }
                TokenKind::PipeBurst => {
                    cluster_active = false;
                    self.emit_removed_temporal_pipe_operator(index, "|> burst <duration> <count>");
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let stage_kind = self.parse_burst_pipe_stage(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, stage_kind, result_memo)
                }
                _ => break,
            };
            stages.push(PipeStage {
                subject_memo,
                result_memo,
                span: self.source_span_for_range(index, *cursor),
                kind: stage_kind,
            });
        }

        if stages.is_empty() {
            return head.map(|expr| *expr);
        }

        let span = self.source_span_for_range(start, *cursor);
        Some(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head: head.take(),
                stages,
                span,
            }),
        })
    }

    fn try_parse_prefixed_temporal_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Option<PipeStageKind>> {
        let Some(index) = self.peek_nontrivia(*cursor, end) else {
            return Some(None);
        };
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Identifier
        {
            return Some(None);
        }
        if self.is_identifier_text(index, "delay") {
            *cursor = index + 1;
            return Some(Some(self.parse_delay_pipe_stage(cursor, end, stop)?));
        }
        if self.is_identifier_text(index, "burst") {
            *cursor = index + 1;
            return Some(Some(self.parse_burst_pipe_stage(cursor, end, stop)?));
        }
        Some(None)
    }

    fn parse_delay_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<PipeStageKind> {
        let duration = self.parse_patch_apply_expr(cursor, end, stop)?;
        Some(PipeStageKind::Delay { duration })
    }

    fn parse_burst_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<PipeStageKind> {
        let every = self.parse_atomic_expr(cursor, end, stop)?;
        let count = self.parse_atomic_expr(cursor, end, stop)?;
        Some(PipeStageKind::Burst { every, count })
    }

    fn emit_removed_temporal_pipe_operator(&mut self, index: usize, replacement: &str) {
        let operator = self.tokens[index].text(self.source);
        self.diagnostics.push(
            Diagnostic::error(format!("`{operator}` has been removed"))
                .with_code(REMOVED_TEMPORAL_PIPE_OPERATOR)
                .with_primary_label(
                    self.source_span_of_token(index),
                    "rewrite this temporal stage using the new prefix form",
                )
                .with_help(format!("use `{replacement}` instead")),
        );
    }

    fn parse_range_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        let start = self.parse_binary_expr(cursor, end, stop)?;
        let Some(index) = self.peek_nontrivia(*cursor, end) else {
            return Some(start);
        };
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::DotDot {
            return Some(start);
        }

        *cursor = index + 1;
        let end_expr = self.parse_binary_expr(cursor, end, stop)?;
        let span = self.join_spans(start.span, end_expr.span);
        Some(Expr {
            span,
            kind: ExprKind::Range {
                start: Box::new(start),
                end: Box::new(end_expr),
            },
        })
    }

    fn parse_optional_pipe_memo(&mut self, cursor: &mut usize, end: usize) -> Option<Identifier> {
        let Some(hash_index) = self.consume_kind(cursor, end, TokenKind::Hash) else {
            return None;
        };
        match self.parse_identifier(cursor, end) {
            Some(identifier) => Some(identifier),
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`#` in a pipe stage must be followed by a memo name")
                        .with_code(MISSING_PIPE_MEMO_NAME)
                        .with_primary_label(
                            self.source_span_of_token(hash_index),
                            "add an identifier such as `#value` here",
                        ),
                );
                None
            }
        }
    }

    fn parse_binary_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        self.parse_binary_expr_prec(cursor, end, stop, 0)
    }

    fn parse_binary_expr_prec(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
        min_precedence: u8,
    ) -> Option<Expr> {
        let mut left = self.parse_application_expr(cursor, end, stop)?;

        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop) {
                break;
            }
            let Some((operator, precedence)) = self.binary_operator(index) else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            *cursor = index + 1;
            let right = self.parse_binary_expr_prec(cursor, end, stop, precedence + 1)?;
            left = self.make_binary_expr(left, operator, right);
        }

        Some(left)
    }

    fn parse_application_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut expr = self.parse_atomic_expr(cursor, end, stop)?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.expr_should_stop(index, stop)
                || self.tokens[index].kind().is_pipe_operator()
                || self.binary_operator(index).is_some()
            {
                break;
            }
            if self.tokens[index].line_start() || !self.starts_expr(index) {
                break;
            }
            let argument = self.parse_atomic_expr(cursor, end, stop)?;
            expr = self.make_apply_expr(expr, argument);
        }
        Some(expr)
    }

    fn parse_atomic_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut expr = self.parse_prefix_expr(cursor, end, stop)?;
        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Dot {
                break;
            }
            expr = self.parse_projection_suffix(expr, cursor, end, stop)?;
        }
        Some(expr)
    }

    fn parse_prefix_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) {
            return None;
        }
        if self.tokens[index].kind() == TokenKind::Identifier
            && self.is_identifier_text(index, "not")
        {
            *cursor = index + 1;
            let expr = self.parse_prefix_expr(cursor, end, stop)?;
            let span = self.source_span_for_range(index, *cursor);
            return Some(Expr {
                span,
                kind: ExprKind::Unary {
                    operator: UnaryOperator::Not,
                    expr: Box::new(expr),
                },
            });
        }
        if let Some(expr) = self.parse_negative_numeric_expr(cursor, end, stop) {
            return Some(expr);
        }
        self.parse_primary_expr(cursor, end, stop)
    }

    fn parse_negative_numeric_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let minus_index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(minus_index, stop)
            || self.tokens[minus_index].kind() != TokenKind::Minus
        {
            return None;
        }
        let literal_index = self.negative_numeric_literal_token(minus_index, end)?;
        *cursor = literal_index + 1;
        match self.tokens[literal_index].kind() {
            TokenKind::Integer => Some(self.parse_integer_expr_with_sign(
                Some(minus_index),
                literal_index,
                cursor,
                end,
            )),
            TokenKind::Float => {
                Some(self.parse_float_expr_with_sign(Some(minus_index), literal_index))
            }
            TokenKind::Decimal => {
                Some(self.parse_decimal_expr_with_sign(Some(minus_index), literal_index))
            }
            TokenKind::BigInt => {
                Some(self.parse_bigint_expr_with_sign(Some(minus_index), literal_index))
            }
            _ => None,
        }
    }

    fn parse_primary_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::PatchKw => self.parse_patch_literal_expr(cursor, end),
            TokenKind::Identifier => {
                if self.is_identifier_text(index, "result")
                    && self
                        .peek_nontrivia(index + 1, end)
                        .is_some_and(|next| self.tokens[next].kind() == TokenKind::LBrace)
                {
                    return self.parse_result_block_expr(cursor, end);
                }
                if self.starts_prefixed_collection_literal(index, end, "Map", TokenKind::LBrace) {
                    return self.parse_map_expr(cursor, end).map(|map| Expr {
                        span: map.span,
                        kind: ExprKind::Map(map),
                    });
                }
                if self.starts_prefixed_collection_literal(index, end, "Set", TokenKind::LBracket) {
                    return self.parse_set_expr(cursor, end);
                }
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                if name.text == "_" {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "the discard `_` is only valid in binder and pattern positions",
                        )
                        .with_code(INVALID_DISCARD_EXPR)
                        .with_primary_label(
                            name.span,
                            "use `.` for the current subject placeholder",
                        ),
                    );
                }
                Some(Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                })
            }
            TokenKind::Integer => {
                *cursor = index + 1;
                Some(self.parse_integer_expr(index, cursor, end))
            }
            TokenKind::Float => {
                *cursor = index + 1;
                Some(self.parse_float_expr(index))
            }
            TokenKind::Decimal => {
                *cursor = index + 1;
                Some(self.parse_decimal_expr(index))
            }
            TokenKind::BigInt => {
                *cursor = index + 1;
                Some(self.parse_bigint_expr(index))
            }
            TokenKind::StringLiteral => {
                *cursor = index + 1;
                let literal = self.text_literal_from_token(index);
                Some(Expr {
                    span: literal.span,
                    kind: ExprKind::Text(literal),
                })
            }
            TokenKind::RegexLiteral => {
                *cursor = index + 1;
                let span = self.source_span_of_token(index);
                Some(Expr {
                    span,
                    kind: ExprKind::Regex(RegexLiteral {
                        raw: self.tokens[index].text(self.source).to_owned(),
                        span,
                    }),
                })
            }
            TokenKind::Dot => self.parse_ambient_projection(cursor, end),
            TokenKind::LParen => self.parse_grouped_expr(cursor, end),
            TokenKind::LBracket => self.parse_list_expr(cursor, end),
            TokenKind::LBrace => self.parse_record_expr(cursor, end).map(|record| Expr {
                span: record.span,
                kind: ExprKind::Record(record),
            }),
            TokenKind::Less => self.parse_markup_expr(cursor, end),
            kind if kind.is_keyword() => {
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                Some(Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                })
            }
            _ => None,
        }
    }

    fn parse_patch_literal_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::PatchKw)?;
        let patch = self.parse_patch_block(cursor, end)?;
        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::PatchLiteral(patch),
        })
    }

    fn parse_patch_block(&mut self, cursor: &mut usize, end: usize) -> Option<PatchBlock> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut entries = Vec::new();
        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let selector = self.parse_patch_selector(cursor, end)?;
            let _ = self.consume_kind(cursor, end, TokenKind::Colon)?;
            let instruction = self.parse_patch_instruction(cursor, end)?;
            let span = self.join_spans(selector.span, instruction.span);
            entries.push(PatchEntry {
                selector,
                instruction,
                span,
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                continue;
            }
            if let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.tokens[index].kind() == TokenKind::RBrace {
                    let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                    break;
                }
                if self.tokens[index].line_start() && self.token_starts_patch_selector(index) {
                    continue;
                }
            }
            break;
        }

        Some(PatchBlock {
            entries,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_patch_selector(&mut self, cursor: &mut usize, end: usize) -> Option<PatchSelector> {
        let start_index = self.peek_nontrivia(*cursor, end)?;
        let mut segments = Vec::new();
        loop {
            if let Some(start) = self.consume_kind(cursor, end, TokenKind::Dot) {
                let name = self.parse_identifier(cursor, end)?;
                let span = self.source_span_for_range(start, *cursor);
                segments.push(PatchSelectorSegment::Named {
                    name,
                    dotted: true,
                    span,
                });
                continue;
            }
            if self.peek_kind(*cursor, end) == Some(TokenKind::LBracket) {
                segments.push(self.parse_patch_bracket_selector(cursor, end)?);
                continue;
            }
            if segments.is_empty() {
                if let Some(name) = self.parse_identifier(cursor, end) {
                    let span = name.span;
                    segments.push(PatchSelectorSegment::Named {
                        name,
                        dotted: false,
                        span,
                    });
                    continue;
                }
            }
            break;
        }
        if segments.is_empty() {
            return None;
        }
        let end_span = patch_selector_segment_span(segments.last().expect("non-empty segments"));
        Some(PatchSelector {
            segments,
            span: SourceSpan::new(
                self.source.id(),
                Span::new(
                    self.tokens[start_index].span().start(),
                    end_span.span().end(),
                ),
            ),
        })
    }

    fn parse_patch_bracket_selector(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<PatchSelectorSegment> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        if self.consume_kind(cursor, end, TokenKind::Star).is_some() {
            let close = self.consume_kind(cursor, end, TokenKind::RBracket)?;
            return Some(PatchSelectorSegment::BracketTraverse {
                span: self.source_span_for_range(start, close + 1),
            });
        }
        let expr = self.parse_expr(cursor, end, ExprStop::list_context())?;
        let _ = self.consume_kind(cursor, end, TokenKind::RBracket)?;
        Some(PatchSelectorSegment::BracketExpr {
            span: self.source_span_for_range(start, *cursor),
            expr: Box::new(expr),
        })
    }

    fn parse_patch_instruction(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<PatchInstruction> {
        if let Some(start) = self.consume_kind(cursor, end, TokenKind::ColonEquals) {
            let expr = self.parse_expr(cursor, end, ExprStop::patch_entry_context())?;
            return Some(PatchInstruction {
                span: self.source_span_for_range(start, *cursor),
                kind: PatchInstructionKind::Store(Box::new(expr)),
            });
        }
        if let Some(start) = self.consume_kind(cursor, end, TokenKind::Minus) {
            return Some(PatchInstruction {
                span: self.source_span_of_token(start),
                kind: PatchInstructionKind::Remove,
            });
        }
        let expr = self.parse_expr(cursor, end, ExprStop::patch_entry_context())?;
        Some(PatchInstruction {
            span: expr.span,
            kind: PatchInstructionKind::Replace(Box::new(expr)),
        })
    }

    fn parse_result_block_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let keyword_index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_identifier_text(keyword_index, "result") {
            return None;
        }
        *cursor = keyword_index + 1;
        let start = keyword_index;
        let open_brace = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let Some(close_brace) = self.find_matching_brace(open_brace, end) else {
            self.diagnostics.push(
                Diagnostic::error("`result { ... }` block is missing a closing `}`")
                    .with_code(MISSING_RESULT_BLOCK_TAIL)
                    .with_primary_label(
                        self.source_span_of_token(open_brace),
                        "close this `result` block with `}`",
                    ),
            );
            return None;
        };

        let mut bindings = Vec::new();
        let mut tail = None;

        while let Some(index) = self.peek_nontrivia(*cursor, close_brace) {
            if let Some((name, left_arrow)) = self.result_block_binding_start(index, close_brace) {
                let item_end =
                    self.find_next_result_block_item_boundary(left_arrow + 1, close_brace);
                let mut binding_cursor = left_arrow + 1;
                let expr = self
                    .parse_expr(&mut binding_cursor, item_end, ExprStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "result block bindings must have an expression after `<-`",
                            )
                            .with_code(MISSING_RESULT_BINDING_EXPR)
                            .with_primary_label(
                                self.source_span_of_token(left_arrow),
                                "add a `Result ...` expression after this binding arrow",
                            ),
                        );
                        None
                    })?;
                if let Some(trailing_index) =
                    self.next_significant_in_range(binding_cursor, item_end)
                {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "result block bindings must contain exactly one expression",
                        )
                        .with_code(MISSING_RESULT_BINDING_EXPR)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "move this token into the binding expression or start a new block line",
                        ),
                    );
                }
                let span = SourceSpan::new(
                    self.source.id(),
                    Span::new(name.span.span().start(), expr.span.span().end()),
                );
                bindings.push(ResultBinding { name, expr, span });
                *cursor = item_end;
                continue;
            }

            let mut tail_cursor = index;
            let expr = self
                .parse_expr(&mut tail_cursor, close_brace, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("result blocks must end with a final expression")
                            .with_code(MISSING_RESULT_BLOCK_TAIL)
                            .with_primary_label(
                                self.source_span_of_token(index),
                                "add the final success value here",
                            ),
                    );
                    None
                })?;
            if let Some(trailing_index) = self.next_significant_in_range(tail_cursor, close_brace) {
                self.diagnostics.push(
                    Diagnostic::error("result block tails must contain exactly one expression")
                        .with_code(MISSING_RESULT_BLOCK_TAIL)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "move this token into the tail expression or close the block",
                        ),
                );
            }
            tail = Some(Box::new(expr));
            *cursor = close_brace;
            break;
        }

        *cursor = close_brace;
        let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
        let span = self.source_span_for_range(start, *cursor);
        if bindings.is_empty() && tail.is_none() {
            self.diagnostics.push(
                Diagnostic::error("result blocks cannot be empty")
                    .with_code(EMPTY_RESULT_BLOCK)
                    .with_primary_label(
                        span,
                        "add a binding or final expression inside this block",
                    ),
            );
        }
        Some(Expr {
            span,
            kind: ExprKind::ResultBlock(ResultBlockExpr {
                bindings,
                tail,
                span,
            }),
        })
    }

    fn result_block_binding_start(&self, index: usize, end: usize) -> Option<(Identifier, usize)> {
        if self.tokens.get(index)?.kind() != TokenKind::Identifier {
            return None;
        }
        let left_arrow = self.peek_nontrivia(index + 1, end)?;
        (self.tokens[left_arrow].kind() == TokenKind::LeftArrow)
            .then(|| (self.identifier_from_token(index), left_arrow))
    }

    fn find_matching_brace(&self, open_brace: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        for index in open_brace..end {
            match self.tokens[index].kind() {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn find_next_result_block_item_boundary(&self, start: usize, close_brace: usize) -> usize {
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        for index in start..close_brace {
            match self.tokens[index].kind() {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => {
                    if brace_depth == 0 {
                        return index;
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                }
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }

            if paren_depth == 0
                && brace_depth == 0
                && bracket_depth == 0
                && self.tokens[index].kind() == TokenKind::Newline
            {
                let Some(next) = self.peek_nontrivia(index + 1, close_brace) else {
                    return close_brace;
                };
                if self.result_block_binding_start(next, close_brace).is_some() {
                    return next;
                }
                if !self.tokens[next].kind().is_pipe_operator() {
                    return next;
                }
            }
        }
        close_brace
    }

    fn parse_projection_suffix(
        &mut self,
        base: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Dot {
            return None;
        }

        let mut fields = Vec::new();
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            let field = self.parse_identifier(cursor, end)?;
            fields.push(field);
            if self.peek_kind(*cursor, end) != Some(TokenKind::Dot) {
                break;
            }
        }

        let path_end = fields
            .last()
            .map(|field| field.span.span().end())
            .unwrap_or_else(|| self.tokens[index].span().end());
        let path_span = SourceSpan::new(
            self.source.id(),
            Span::new(self.tokens[index].span().start(), path_end),
        );
        let projection = ProjectionPath {
            span: path_span,
            fields,
        };
        Some(Expr {
            span: self.join_spans(base.span, projection.span),
            kind: ExprKind::Projection {
                base: Box::new(base),
                path: projection,
            },
        })
    }

    fn parse_integer_expr(&self, index: usize, cursor: &mut usize, end: usize) -> Expr {
        self.parse_integer_expr_with_sign(None, index, cursor, end)
    }

    fn parse_integer_expr_with_sign(
        &self,
        minus_index: Option<usize>,
        index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        let literal = IntegerLiteral {
            raw: self.numeric_literal_raw(minus_index, index),
            span,
        };
        if let Some(suffix_index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[suffix_index].kind() == TokenKind::Identifier
                && self.tokens_are_adjacent(index, suffix_index)
            {
                *cursor = suffix_index + 1;
                let suffix = self.identifier_from_token(suffix_index);
                let span = self.join_spans(literal.span, suffix.span);
                return Expr {
                    span,
                    kind: ExprKind::SuffixedInteger(SuffixedIntegerLiteral {
                        literal,
                        suffix,
                        span,
                    }),
                };
            }
        }

        Expr {
            span,
            kind: ExprKind::Integer(literal),
        }
    }

    fn parse_float_expr(&self, index: usize) -> Expr {
        self.parse_float_expr_with_sign(None, index)
    }

    fn parse_float_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::Float(FloatLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_decimal_expr(&self, index: usize) -> Expr {
        self.parse_decimal_expr_with_sign(None, index)
    }

    fn parse_decimal_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::Decimal(DecimalLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_bigint_expr(&self, index: usize) -> Expr {
        self.parse_bigint_expr_with_sign(None, index)
    }

    fn parse_bigint_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::BigInt(BigIntLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_ambient_projection(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::Dot)?;
        let next_is_ident = matches!(
            self.peek_kind(*cursor, end),
            Some(k) if k == TokenKind::Identifier || k.is_keyword()
        );
        if !next_is_ident {
            return Some(Expr {
                span: self.source_span_of_token(start),
                kind: ExprKind::SubjectPlaceholder,
            });
        }
        let mut fields = vec![self.parse_identifier(cursor, end)?];
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            fields.push(self.parse_identifier(cursor, end)?);
        }
        let last_end = fields
            .last()
            .map(|field| field.span.span().end())
            .unwrap_or_else(|| self.tokens[start].span().end());
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(self.tokens[start].span().start(), last_end),
        );
        Some(Expr {
            span,
            kind: ExprKind::AmbientProjection(ProjectionPath { span, fields }),
        })
    }

    fn parse_grouped_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(Expr {
                span: self.source_span_for_range(start, *cursor),
                kind: ExprKind::Tuple(Vec::new()),
            });
        }

        // Detect operator section: (op) where the content is a single binary operator token.
        if let Some(op_index) = self.peek_nontrivia(*cursor, end) {
            if let Some((op, _)) = self.binary_operator(op_index) {
                let after_op = self.peek_nontrivia(op_index + 1, end);
                if after_op.map_or(false, |i| self.tokens[i].kind() == TokenKind::RParen) {
                    *cursor = op_index + 1;
                    let _ = self.consume_kind(cursor, end, TokenKind::RParen);
                    return Some(Expr {
                        span: self.source_span_for_range(start, *cursor),
                        kind: ExprKind::OperatorSection(op),
                    });
                }
            }
        }

        loop {
            let element = self.parse_expr(cursor, end, ExprStop::paren_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                saw_comma = true;
                if self.peek_kind(*cursor, end) == Some(TokenKind::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RParen);
        let span = self.source_span_for_range(start, *cursor);
        Some(if saw_comma || elements.len() != 1 {
            Expr {
                span,
                kind: ExprKind::Tuple(elements),
            }
        } else {
            Expr {
                span,
                kind: ExprKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_list_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::RBracket)
                .is_some()
            {
                break;
            }
            let element = self.parse_expr(cursor, end, ExprStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
                break;
            }
        }

        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::List(elements),
        })
    }

    fn parse_set_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_identifier_text(cursor, end, "Set")?;
        let _ = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::RBracket)
                .is_some()
            {
                break;
            }
            let element = self.parse_expr(cursor, end, ExprStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
                break;
            }
        }

        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::Set(elements),
        })
    }

    fn parse_record_expr(&mut self, cursor: &mut usize, end: usize) -> Option<RecordExpr> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut fields = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let Some(label) = self.parse_identifier(cursor, end) else {
                break;
            };
            let field_start = label.span.span().start();
            let mut label_path = Vec::new();
            while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
                if let Some(next) = self.parse_identifier(cursor, end) {
                    label_path.push(next);
                } else {
                    break;
                }
            }
            let value = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_expr(cursor, end, ExprStop::record_context())
            } else {
                None
            };
            let field_end = value
                .as_ref()
                .map(|expr| expr.span.span().end())
                .unwrap_or_else(|| {
                    label_path
                        .last()
                        .map(|p| p.span.span().end())
                        .unwrap_or_else(|| label.span.span().end())
                });
            fields.push(RecordField {
                label,
                label_path,
                value,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(RecordExpr {
            fields,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_map_expr(&mut self, cursor: &mut usize, end: usize) -> Option<MapExpr> {
        let start = self.consume_identifier_text(cursor, end, "Map")?;
        let _ = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut entries = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let key = self.parse_expr(cursor, end, ExprStop::record_context())?;
            let _ = self.consume_kind(cursor, end, TokenKind::Colon)?;
            let value = self.parse_expr(cursor, end, ExprStop::record_context())?;
            entries.push(MapExprEntry {
                span: self.join_spans(key.span, value.span),
                key,
                value,
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(MapExpr {
            entries,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_markup_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let node = self.parse_markup_node(cursor, end)?;
        Some(Expr {
            span: node.span,
            kind: ExprKind::Markup(node),
        })
    }

    fn parse_markup_node(&mut self, cursor: &mut usize, end: usize) -> Option<MarkupNode> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_markup_node_inner(cursor, end);
        self.depth_exit();
        result
    }

    fn parse_markup_node_inner(&mut self, cursor: &mut usize, end: usize) -> Option<MarkupNode> {
        let start = self.consume_kind(cursor, end, TokenKind::Less)?;
        let name = self.parse_qualified_name(cursor, end)?;
        let case_pattern_attrs = name.as_dotted() == "case";
        let mut attributes = Vec::new();

        loop {
            match self.peek_kind(*cursor, end) {
                Some(TokenKind::SelfCloseTagEnd) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::SelfCloseTagEnd);
                    return Some(MarkupNode {
                        name,
                        attributes,
                        children: Vec::new(),
                        close_name: None,
                        self_closing: true,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
                Some(TokenKind::Greater) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::Greater);
                    break;
                }
                Some(kind) if kind == TokenKind::Identifier || kind.is_keyword() => {
                    let Some(attribute) =
                        self.parse_markup_attribute(cursor, end, case_pattern_attrs)
                    else {
                        break;
                    };
                    attributes.push(attribute);
                }
                _ => break,
            }
        }

        let mut children = Vec::new();
        let mut close_name = None;

        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                self.diagnostics.push(
                    Diagnostic::error(
                        "markup node is not closed before the end of the declaration",
                    )
                    .with_code(UNTERMINATED_MARKUP_NODE)
                    .with_primary_label(name.span, "this markup node needs a matching closing tag")
                    .with_help("add a closing tag or use self-closing syntax"),
                );
                break;
            };
            match self.tokens[index].kind() {
                TokenKind::CloseTagStart => {
                    *cursor = index + 1;
                    close_name = self.parse_qualified_name(cursor, end);
                    let _ = self.consume_kind(cursor, end, TokenKind::Greater);
                    if let Some(close_name_value) = close_name.as_ref() {
                        if close_name_value.as_dotted() != name.as_dotted() {
                            self.diagnostics.push(
                                Diagnostic::error("markup closing tag does not match the open tag")
                                    .with_code(MISMATCHED_MARKUP_CLOSE)
                                    .with_primary_label(
                                        close_name_value.span,
                                        format!(
                                            "expected `</{}>` to close this node",
                                            name.as_dotted()
                                        ),
                                    )
                                    .with_secondary_label(
                                        name.span,
                                        format!("`<{}>` was opened here", name.as_dotted()),
                                    )
                                    .with_help("ensure opening and closing tags match"),
                            );
                        }
                    }
                    return Some(MarkupNode {
                        name,
                        attributes,
                        children,
                        close_name,
                        self_closing: false,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
                TokenKind::Less => {
                    let Some(child) = self.parse_markup_node(cursor, end) else {
                        break;
                    };
                    children.push(child);
                }
                _ => {
                    self.reject_invalid_markup_child_content(cursor, end, &name);
                }
            }
        }

        Some(MarkupNode {
            name,
            attributes,
            children,
            close_name,
            self_closing: false,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_markup_attribute(
        &mut self,
        cursor: &mut usize,
        end: usize,
        case_pattern_attr: bool,
    ) -> Option<MarkupAttribute> {
        let name = self.parse_identifier(cursor, end)?;
        let attribute_start = name.span.span().start();
        let value = if self.consume_kind(cursor, end, TokenKind::Equals).is_some() {
            match self.peek_kind(*cursor, end) {
                Some(TokenKind::StringLiteral) => {
                    let index = self.peek_nontrivia(*cursor, end)?;
                    *cursor = index + 1;
                    Some(MarkupAttributeValue::Text(
                        self.text_literal_from_token(index),
                    ))
                }
                Some(TokenKind::LBrace) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::LBrace);
                    let value = if case_pattern_attr && name.text == "pattern" {
                        self.parse_pattern(cursor, end, PatternStop::brace_context())
                            .map(MarkupAttributeValue::Pattern)
                    } else {
                        self.parse_expr(cursor, end, ExprStop::brace_context())
                            .map(MarkupAttributeValue::Expr)
                    }?;
                    let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                    Some(value)
                }
                _ => None,
            }
        } else {
            None
        };
        let attribute_end = match &value {
            Some(MarkupAttributeValue::Text(text)) => text.span.span().end(),
            Some(MarkupAttributeValue::Expr(expr)) => expr.span.span().end(),
            Some(MarkupAttributeValue::Pattern(pattern)) => pattern.span.span().end(),
            None => name.span.span().end(),
        };
        Some(MarkupAttribute {
            name,
            value,
            span: SourceSpan::new(self.source.id(), Span::new(attribute_start, attribute_end)),
        })
    }

    fn parse_pipe_case_arm(
        &mut self,
        cursor: &mut usize,
        end: usize,
        outer_stop: ExprStop,
    ) -> Option<PipeCaseArm> {
        let start = *cursor;
        let pattern = self.parse_pattern(cursor, end, PatternStop::arrow_context())?;
        let _ = self.consume_kind(cursor, end, TokenKind::ThinArrow)?;
        // The arm body may contain inline pipe expressions (e.g. `first ||> { email } -> email`).
        // Only line-start pipe operators terminate the body — those belong to sibling arms.
        let mut body_stop = outer_stop;
        body_stop.pipe_stage = false;
        body_stop.pipe_stage_line_start_only = true;
        let body = self.parse_expr(cursor, end, body_stop)?;
        Some(PipeCaseArm {
            pattern,
            body,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_pattern(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: PatternStop,
    ) -> Option<Pattern> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let mut pattern = self.parse_pattern_atom(cursor, end, stop);
        let result = pattern.take().and_then(|mut p| {
            while let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.pattern_should_stop(index, stop) || !self.starts_pattern(index) {
                    break;
                }
                if self.tokens[index].line_start() {
                    break;
                }
                let argument = self.parse_pattern_atom(cursor, end, stop)?;
                p = self.make_pattern_apply(p, argument);
            }
            Some(p)
        });
        self.depth_exit();
        result
    }

    fn parse_pattern_atom(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: PatternStop,
    ) -> Option<Pattern> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.pattern_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::Minus => {
                let literal_index = self.negative_integer_literal_token(index, end)?;
                *cursor = literal_index + 1;
                let span = self.numeric_literal_span(Some(index), literal_index);
                Some(Pattern {
                    span,
                    kind: PatternKind::Integer(IntegerLiteral {
                        raw: self.numeric_literal_raw(Some(index), literal_index),
                        span,
                    }),
                })
            }
            TokenKind::Identifier => {
                *cursor = index + 1;
                let identifier = self.identifier_from_token(index);
                if identifier.text == "_" {
                    Some(Pattern {
                        span: identifier.span,
                        kind: PatternKind::Wildcard,
                    })
                } else {
                    Some(Pattern {
                        span: identifier.span,
                        kind: PatternKind::Name(identifier),
                    })
                }
            }
            // Keywords can be used as binding variable names in patterns (contextual keyword).
            kind if kind.is_keyword() => {
                *cursor = index + 1;
                let identifier = self.identifier_from_token(index);
                Some(Pattern {
                    span: identifier.span,
                    kind: PatternKind::Name(identifier),
                })
            }
            TokenKind::Integer => {
                *cursor = index + 1;
                let span = self.source_span_of_token(index);
                Some(Pattern {
                    span,
                    kind: PatternKind::Integer(IntegerLiteral {
                        raw: self.tokens[index].text(self.source).to_owned(),
                        span,
                    }),
                })
            }
            TokenKind::StringLiteral => {
                *cursor = index + 1;
                let literal = self.text_literal_from_token(index);
                Some(Pattern {
                    span: literal.span,
                    kind: PatternKind::Text(literal),
                })
            }
            TokenKind::LParen => self.parse_grouped_pattern(cursor, end),
            TokenKind::LBracket => self.parse_list_pattern(cursor, end),
            TokenKind::LBrace => self.parse_record_pattern(cursor, end),
            _ => None,
        }
    }

    fn parse_grouped_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(Pattern {
                span: self.source_span_for_range(start, *cursor),
                kind: PatternKind::Tuple(Vec::new()),
            });
        }

        loop {
            let element = self.parse_pattern(cursor, end, PatternStop::paren_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                saw_comma = true;
                if self.peek_kind(*cursor, end) == Some(TokenKind::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RParen);
        let span = self.source_span_for_range(start, *cursor);
        Some(if saw_comma || elements.len() != 1 {
            Pattern {
                span,
                kind: PatternKind::Tuple(elements),
            }
        } else {
            Pattern {
                span,
                kind: PatternKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_record_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut fields = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let Some(label) = self.parse_identifier(cursor, end) else {
                break;
            };
            let field_start = label.span.span().start();
            let mut label_path = Vec::new();
            while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
                if let Some(next) = self.parse_identifier(cursor, end) {
                    label_path.push(next);
                } else {
                    break;
                }
            }
            let pattern = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_pattern(cursor, end, PatternStop::record_context())
            } else {
                None
            };
            let field_end = pattern
                .as_ref()
                .map(|pattern| pattern.span.span().end())
                .unwrap_or_else(|| {
                    label_path
                        .last()
                        .map(|p| p.span.span().end())
                        .unwrap_or_else(|| label.span.span().end())
                });
            fields.push(RecordPatternField {
                label,
                label_path,
                pattern,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(Pattern {
            span: self.source_span_for_range(start, *cursor),
            kind: PatternKind::Record(fields),
        })
    }

    fn parse_list_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();
        let mut rest = None;

        if self
            .consume_kind(cursor, end, TokenKind::RBracket)
            .is_some()
        {
            return Some(Pattern {
                span: self.source_span_for_range(start, *cursor),
                kind: PatternKind::List {
                    elements,
                    rest: None,
                },
            });
        }

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::Ellipsis)
                .is_some()
            {
                rest = Some(Box::new(self.parse_pattern(
                    cursor,
                    end,
                    PatternStop::list_context(),
                )?));
                let _ = self.consume_kind(cursor, end, TokenKind::Comma);
                break;
            }

            let element = self.parse_pattern(cursor, end, PatternStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                if self.peek_kind(*cursor, end) == Some(TokenKind::RBracket) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
        Some(Pattern {
            span: self.source_span_for_range(start, *cursor),
            kind: PatternKind::List { elements, rest },
        })
    }

    fn make_apply_expr(&self, callee: Expr, argument: Expr) -> Expr {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            ExprKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                Expr {
                    span,
                    kind: ExprKind::Apply { callee, arguments },
                }
            }
            kind => Expr {
                span,
                kind: ExprKind::Apply {
                    callee: Box::new(Expr {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn make_binary_expr(&self, left: Expr, operator: BinaryOperator, right: Expr) -> Expr {
        Expr {
            span: self.join_spans(left.span, right.span),
            kind: ExprKind::Binary {
                left: Box::new(left),
                operator,
                right: Box::new(right),
            },
        }
    }

    fn make_type_apply(&self, callee: TypeExpr, argument: TypeExpr) -> TypeExpr {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            TypeExprKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                TypeExpr {
                    span,
                    kind: TypeExprKind::Apply { callee, arguments },
                }
            }
            kind => TypeExpr {
                span,
                kind: TypeExprKind::Apply {
                    callee: Box::new(TypeExpr {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn make_type_arrow(&self, parameter: TypeExpr, result: TypeExpr) -> TypeExpr {
        TypeExpr {
            span: self.join_spans(parameter.span, result.span),
            kind: TypeExprKind::Arrow {
                parameter: Box::new(parameter),
                result: Box::new(result),
            },
        }
    }

    fn make_pattern_apply(&self, callee: Pattern, argument: Pattern) -> Pattern {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            PatternKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                Pattern {
                    span,
                    kind: PatternKind::Apply { callee, arguments },
                }
            }
            kind => Pattern {
                span,
                kind: PatternKind::Apply {
                    callee: Box::new(Pattern {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn binary_operator(&self, index: usize) -> Option<(BinaryOperator, u8)> {
        match self.tokens[index].kind() {
            TokenKind::Plus => Some((BinaryOperator::Add, 4)),
            TokenKind::Minus => Some((BinaryOperator::Subtract, 4)),
            TokenKind::Star => Some((BinaryOperator::Multiply, 5)),
            TokenKind::Slash => Some((BinaryOperator::Divide, 5)),
            TokenKind::Percent => Some((BinaryOperator::Modulo, 5)),
            TokenKind::Greater => Some((BinaryOperator::GreaterThan, 3)),
            TokenKind::Less => Some((BinaryOperator::LessThan, 3)),
            TokenKind::GreaterEqual => Some((BinaryOperator::GreaterThanOrEqual, 3)),
            TokenKind::LessEqual => Some((BinaryOperator::LessThanOrEqual, 3)),
            TokenKind::EqualEqual => Some((BinaryOperator::Equals, 3)),
            TokenKind::BangEqual => Some((BinaryOperator::NotEquals, 3)),
            TokenKind::Identifier if self.is_identifier_text(index, "and") => {
                Some((BinaryOperator::And, 2))
            }
            TokenKind::Identifier if self.is_identifier_text(index, "or") => {
                Some((BinaryOperator::Or, 1))
            }
            _ => None,
        }
    }

    fn starts_expr(&self, index: usize) -> bool {
        let kind = self.tokens[index].kind();
        if kind == TokenKind::Minus {
            return self
                .negative_numeric_literal_token(index, self.tokens.len())
                .is_some();
        }
        kind.is_keyword()
            || matches!(
                kind,
                TokenKind::Identifier
                    | TokenKind::Integer
                    | TokenKind::Float
                    | TokenKind::Decimal
                    | TokenKind::BigInt
                    | TokenKind::StringLiteral
                    | TokenKind::RegexLiteral
                    | TokenKind::Dot
                    | TokenKind::LParen
                    | TokenKind::LBracket
                    | TokenKind::LBrace
                    | TokenKind::Less
            )
    }

    fn starts_function_param(&self, start: usize, end: usize) -> bool {
        self.peek_nontrivia(start, end).is_some_and(|index| {
            !self.tokens[index].line_start() && self.is_function_param_token(index)
        })
    }

    fn find_last_same_line_arrow(&self, start: usize, end: usize) -> Option<usize> {
        let mut scan = start;
        let mut found = None;
        let mut saw_token = false;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            if self.tokens[index].kind() == TokenKind::Arrow {
                found = Some(index);
            }
            scan = index + 1;
        }
        found
    }

    fn find_same_line_top_level_equals(&self, start: usize, end: usize) -> Option<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return Some(index),
                _ => {}
            }
            scan = index + 1;
        }
        None
    }

    fn find_top_level_equals(&self, start: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut scan = start;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return Some(index),
                _ => {}
            }
            scan = index + 1;
        }
        None
    }

    fn parameter_annotation_end(&self, start: usize, end: usize) -> usize {
        let Some(body_arrow) = self.find_last_same_line_arrow(start, end) else {
            return end;
        };
        self.same_line_top_level_typed_param_starts(start, body_arrow)
            .into_iter()
            .next()
            .unwrap_or(body_arrow)
    }

    fn function_signature_split_candidates(&self, start: usize, body_arrow: usize) -> Vec<usize> {
        let mut candidates = self.same_line_top_level_param_starts(start, body_arrow);
        if !candidates.contains(&body_arrow) {
            candidates.push(body_arrow);
        }
        candidates
    }

    fn same_line_top_level_param_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        let mut starts = Vec::new();
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                _ if depth == 0 && self.is_function_param_token(index) => starts.push(index),
                _ => {}
            }
            scan = index + 1;
        }
        starts
    }

    fn same_line_top_level_typed_param_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        let mut starts = Vec::new();
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Identifier
                    if depth == 0 && self.peek_kind(index + 1, end) == Some(TokenKind::Colon) =>
                {
                    starts.push(index);
                }
                kind if kind.is_keyword()
                    && depth == 0
                    && self.peek_kind(index + 1, end) == Some(TokenKind::Colon) =>
                {
                    starts.push(index);
                }
                _ => {}
            }
            scan = index + 1;
        }
        starts
    }

    fn probe_function_signature(&self, split: usize, start: usize, body_arrow: usize) -> bool {
        let mut probe = Parser::new(self.source, self.tokens);
        probe.depth = self.depth;

        let mut annotation_cursor = start;
        let (_, annotation) =
            probe.parse_optional_signature_annotation(&mut annotation_cursor, split);
        if annotation.is_none()
            || probe
                .next_significant_in_range(annotation_cursor, split)
                .is_some()
            || probe
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            return false;
        }

        let mut parameter_cursor = split;
        let parameter_end = body_arrow.saturating_add(1);
        while probe.starts_function_param(parameter_cursor, body_arrow) {
            if probe
                .parse_function_param(&mut parameter_cursor, parameter_end)
                .is_none()
            {
                return false;
            }
        }
        probe
            .next_significant_in_range(parameter_cursor, body_arrow)
            .is_none()
            && !probe
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    fn is_function_param_token(&self, index: usize) -> bool {
        match self.tokens[index].kind() {
            kind if kind.is_keyword() => true,
            TokenKind::Identifier => {
                let identifier = self.identifier_from_token(index);
                !identifier.is_uppercase_initial()
            }
            _ => false,
        }
    }

    fn has_top_level_equals(&self, start: usize, end: usize) -> bool {
        let mut depth = 0usize;
        let mut scan = start;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return true,
                _ => {}
            }
            scan = index + 1;
        }
        false
    }

    fn pending_type_annotation(item: &Item) -> Option<PendingTypeAnnotation> {
        let Item::Type(item) = item else {
            return None;
        };
        if item.name.is_some() || item.body.is_some() {
            return None;
        }
        Some(PendingTypeAnnotation {
            span: item.base.span,
            constraints: item.constraints.clone(),
            annotation: item.annotation.clone()?,
        })
    }

    fn apply_pending_type_annotation(&mut self, item: &mut Item, pending: PendingTypeAnnotation) {
        match item {
            Item::Fun(named) | Item::Value(named) | Item::Signal(named) => {
                if !named.constraints.is_empty() || named.annotation.is_some() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "standalone `type` annotations cannot be combined with inline annotations",
                        )
                        .with_code(DUPLICATE_STANDALONE_TYPE_ANNOTATION)
                        .with_primary_label(
                            pending.span,
                            "remove this standalone `type` line",
                        )
                        .with_secondary_label(
                            named.name
                                .as_ref()
                                .map(|name| name.span)
                                .unwrap_or(named.keyword_span),
                            "this declaration already carries its own annotation",
                        ),
                    );
                    return;
                }
                named.constraints = pending.constraints;
                named.annotation = Some(pending.annotation);
            }
            _ => self.emit_orphan_standalone_type_annotation(&pending, Some(item.span())),
        }
    }

    fn emit_orphan_standalone_type_annotation(
        &mut self,
        pending: &PendingTypeAnnotation,
        next_item_span: Option<SourceSpan>,
    ) {
        let diagnostic = Diagnostic::error(
            "standalone `type` annotations must attach to the immediately following `func`, `value`, or `signal` declaration",
        )
        .with_code(ORPHAN_STANDALONE_TYPE_ANNOTATION)
        .with_primary_label(
            pending.span,
            "this `type` line is not attached to a supported declaration",
        );
        self.diagnostics.push(if let Some(span) = next_item_span {
            diagnostic.with_secondary_label(
                span,
                "only the immediately following `func`, `value`, or `signal` can receive a standalone annotation",
            )
        } else {
            diagnostic
        });
    }

    fn starts_pattern(&self, index: usize) -> bool {
        let kind = self.tokens[index].kind();
        if kind == TokenKind::Minus {
            return self
                .negative_integer_literal_token(index, self.tokens.len())
                .is_some();
        }
        matches!(
            kind,
            TokenKind::Identifier
                | TokenKind::Integer
                | TokenKind::StringLiteral
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
        ) || kind.is_keyword()
    }

    fn starts_type_atom(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::LBrace
        )
    }

    fn expr_should_stop(&self, index: usize, stop: ExprStop) -> bool {
        if stop.patch_entry
            && self.tokens[index].line_start()
            && self.token_starts_patch_selector(index)
        {
            return true;
        }
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::RBracket => stop.rbracket,
            TokenKind::Arrow => stop.arrow,
            TokenKind::Hash => stop.hash,
            kind if kind.is_pipe_operator() => {
                stop.pipe_stage
                    || (stop.pipe_stage_line_start_only && self.tokens[index].line_start())
            }
            _ => false,
        }
    }

    fn token_starts_patch_selector(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Dot | TokenKind::LBracket | TokenKind::Identifier
        ) || self.tokens[index].kind().is_keyword()
    }

    fn consume_constraint_separator(&self, cursor: &mut usize, end: usize) -> bool {
        self.consume_kind(cursor, end, TokenKind::Arrow).is_some()
    }

    fn negative_numeric_literal_token(&self, minus_index: usize, end: usize) -> Option<usize> {
        if self.tokens.get(minus_index)?.kind() != TokenKind::Minus {
            return None;
        }
        let literal_index = self.peek_nontrivia(minus_index + 1, end)?;
        matches!(
            self.tokens[literal_index].kind(),
            TokenKind::Integer | TokenKind::Float | TokenKind::Decimal | TokenKind::BigInt
        )
        .then_some(())?;
        self.tokens_are_adjacent(minus_index, literal_index)
            .then_some(literal_index)
    }

    fn negative_integer_literal_token(&self, minus_index: usize, end: usize) -> Option<usize> {
        let literal_index = self.negative_numeric_literal_token(minus_index, end)?;
        (self.tokens[literal_index].kind() == TokenKind::Integer).then_some(literal_index)
    }

    fn numeric_literal_raw(&self, minus_index: Option<usize>, index: usize) -> String {
        let raw = self.tokens[index].text(self.source);
        match minus_index {
            Some(_) => format!("-{raw}"),
            None => raw.to_owned(),
        }
    }

    fn numeric_literal_span(&self, minus_index: Option<usize>, index: usize) -> SourceSpan {
        match minus_index {
            Some(start) => self.source_span_for_range(start, index + 1),
            None => self.source_span_of_token(index),
        }
    }

    fn reject_invalid_markup_child_content(
        &mut self,
        cursor: &mut usize,
        end: usize,
        parent: &QualifiedName,
    ) {
        let Some(start) = self.peek_nontrivia(*cursor, end) else {
            return;
        };
        let mut next = start + 1;
        while let Some(index) = self.peek_nontrivia(next, end) {
            match self.tokens[index].kind() {
                TokenKind::CloseTagStart | TokenKind::Less => break,
                _ => next = index + 1,
            }
        }
        self.diagnostics.push(
            Diagnostic::error(format!(
                "markup children inside `<{}>` must be provided through attributes",
                parent.as_dotted()
            ))
            .with_code(INVALID_MARKUP_CHILD_CONTENT)
            .with_primary_label(
                self.source_span_for_range(start, next),
                "use an attribute such as `text={...}` instead of child text or interpolation",
            ),
        );
        *cursor = next;
    }

    fn pattern_should_stop(&self, index: usize, stop: PatternStop) -> bool {
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::RBracket => stop.rbracket,
            // Pattern arms use `->` (ThinArrow); `=>` (Arrow) is for lambdas/function bodies.
            TokenKind::ThinArrow => stop.arrow,
            TokenKind::Arrow => stop.fat_arrow,
            _ => false,
        }
    }

    fn type_should_stop(&self, index: usize, stop: TypeStop) -> bool {
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::Arrow => stop.arrow,
            TokenKind::ThinArrow => stop.thin_arrow,
            TokenKind::PipeTransform => stop.pipe_transform,
            _ => false,
        }
    }

    fn missing_body_diagnostic(&mut self, keyword_index: usize, message: &str, label: &str) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(MISSING_DECLARATION_BODY)
                .with_primary_label(self.source_span_of_token(keyword_index), label)
                .with_help("expected `=` followed by the function body"),
        );
    }

    fn parse_expression_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
        missing_message: &str,
        missing_label: &str,
    ) -> Option<NamedItemBody> {
        let expr = self
            .parse_expr(cursor, end, ExprStop::default())
            .or_else(|| {
                self.missing_body_diagnostic(keyword_index, missing_message, missing_label);
                None
            })?;
        self.finish_expression_body(cursor, end, declaration_name, expr)
    }

    fn finish_expression_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
        expr: Expr,
    ) -> Option<NamedItemBody> {
        if let Some(trailing_index) = self.next_significant_in_range(*cursor, end) {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "{declaration_name} body must contain exactly one expression"
                ))
                .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                .with_primary_label(
                    self.source_span_of_token(trailing_index),
                    "this token is outside the declaration body",
                ),
            );
        }
        Some(NamedItemBody::Expr(expr))
    }

    fn text_literal_from_token(&mut self, index: usize) -> TextLiteral {
        let span = self.source_span_of_token(index);
        let raw = self.tokens[index].text(self.source);
        self.parse_text_literal(raw, span)
    }

    fn parse_text_literal(&mut self, raw: &str, span: SourceSpan) -> TextLiteral {
        let start = span.span().start().as_usize();
        let end = span.span().end().as_usize();
        let body_start = if raw.starts_with('"') {
            start + 1
        } else {
            start
        };
        let body_end = if raw.ends_with('"') && end > body_start {
            end - 1
        } else {
            end
        };

        let mut segments = Vec::new();
        let mut cursor = body_start;
        let mut fragment_start = body_start;
        let text = self.source.text();

        while cursor < body_end {
            let next = text[cursor..body_end]
                .chars()
                .next()
                .expect("text literal scan must stay on a UTF-8 boundary");
            match next {
                '\\' => {
                    cursor = text_escape_end(text, cursor, body_end);
                }
                '{' => {
                    self.push_text_fragment(&mut segments, fragment_start, cursor, false);
                    let Some(close_start) = self.find_text_interpolation_close(cursor, body_end)
                    else {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "text interpolation is not terminated before the end of the string literal",
                            )
                            .with_code(UNTERMINATED_TEXT_INTERPOLATION)
                            .with_primary_label(
                                SourceSpan::new(self.source.id(), Span::from(cursor..body_end)),
                                "expected a closing `}` for this interpolation",
                            )
                            .with_help("close the interpolation with `}`"),
                        );
                        let allow_empty = segments.is_empty();
                        self.push_text_fragment(&mut segments, cursor, body_end, allow_empty);
                        fragment_start = body_end;
                        break;
                    };
                    let interpolation_end = close_start + 1;
                    let interpolation_span =
                        SourceSpan::new(self.source.id(), Span::from(cursor..interpolation_end));
                    let expr_range = cursor + 1..close_start;
                    if self.source.text()[expr_range.clone()].trim().is_empty() {
                        self.diagnostics.push(
                            Diagnostic::error("text interpolation must contain an expression")
                                .with_code(INVALID_TEXT_INTERPOLATION)
                                .with_primary_label(
                                    interpolation_span,
                                    "add an expression between `{` and `}`",
                                )
                                .with_help(
                                    "text interpolation expects an expression inside `\\{...}`",
                                ),
                        );
                        self.push_text_fragment(&mut segments, cursor, interpolation_end, false);
                    } else if let Some(expr) =
                        self.parse_text_interpolation_expr(expr_range, interpolation_span)
                    {
                        segments.push(TextSegment::Interpolation(TextInterpolation {
                            expr: Box::new(expr),
                            span: interpolation_span,
                        }));
                    } else {
                        self.push_text_fragment(&mut segments, cursor, interpolation_end, false);
                    }
                    cursor = interpolation_end;
                    fragment_start = interpolation_end;
                }
                _ => cursor += next.len_utf8(),
            }
        }

        let allow_empty = segments.is_empty();
        self.push_text_fragment(&mut segments, fragment_start, body_end, allow_empty);
        TextLiteral { span, segments }
    }

    fn push_text_fragment(
        &self,
        segments: &mut Vec<TextSegment>,
        start: usize,
        end: usize,
        allow_empty: bool,
    ) {
        if start == end && !allow_empty {
            return;
        }
        let span = self.source.source_span(start..end);
        segments.push(TextSegment::Text(TextFragment {
            raw: decode_text_fragment(self.source.slice(span.span())),
            span,
        }));
    }

    fn find_text_interpolation_close(&self, open_brace: usize, body_end: usize) -> Option<usize> {
        let lexed = lex_fragment(self.source, open_brace + 1..body_end);
        let mut brace_depth = 0usize;
        for token in lexed.tokens() {
            match token.kind() {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => {
                    if brace_depth == 0 {
                        return Some(token.span().start().as_usize());
                    }
                    brace_depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    fn parse_text_interpolation_expr(
        &mut self,
        range: std::ops::Range<usize>,
        interpolation_span: SourceSpan,
    ) -> Option<Expr> {
        let lexed = lex_fragment(self.source, range);
        self.diagnostics.extend(lexed.diagnostics().iter().cloned());

        let mut parser = Parser::new(self.source, lexed.tokens());
        parser.depth = self.depth;
        let mut cursor = 0usize;
        let expr = parser.parse_expr(&mut cursor, lexed.tokens().len(), ExprStop::default());
        let trailing = parser.next_significant_from(cursor);
        self.diagnostics.extend(parser.diagnostics);

        match expr {
            Some(expr) if trailing.is_none() => Some(expr),
            Some(_) => {
                let trailing_index = trailing.expect("checked above");
                self.diagnostics.push(
                    Diagnostic::error("text interpolation must contain exactly one expression")
                        .with_code(INVALID_TEXT_INTERPOLATION)
                        .with_primary_label(
                            SourceSpan::new(
                                self.source.id(),
                                lexed.tokens()[trailing_index].span(),
                            ),
                            "this token is outside the interpolation expression",
                        )
                        .with_help("text interpolation expects an expression inside `\\{...}`"),
                );
                None
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("text interpolation must contain a valid expression")
                        .with_code(INVALID_TEXT_INTERPOLATION)
                        .with_primary_label(
                            interpolation_span,
                            "could not parse the expression inside this interpolation",
                        )
                        .with_help("text interpolation expects an expression inside `\\{...}`"),
                );
                None
            }
        }
    }

    fn parse_identifier(&self, cursor: &mut usize, end: usize) -> Option<Identifier> {
        let index = self.peek_nontrivia(*cursor, end)?;
        let kind = self.tokens[index].kind();
        if kind != TokenKind::Identifier && !kind.is_keyword() {
            return None;
        }
        *cursor = index + 1;
        Some(self.identifier_from_token(index))
    }

    fn parse_qualified_name(&self, cursor: &mut usize, end: usize) -> Option<QualifiedName> {
        let first = self.peek_nontrivia(*cursor, end)?;
        let first_kind = self.tokens[first].kind();
        if first_kind != TokenKind::Identifier && !first_kind.is_keyword() {
            return None;
        }

        let mut segments = vec![self.identifier_from_token(first)];
        let mut scan = first + 1;

        while let Some(dot_index) = self.peek_nontrivia(scan, end) {
            if self.tokens[dot_index].kind() != TokenKind::Dot {
                break;
            }
            let Some(segment_index) = self.peek_nontrivia(dot_index + 1, end) else {
                break;
            };
            let seg_kind = self.tokens[segment_index].kind();
            if seg_kind != TokenKind::Identifier && !seg_kind.is_keyword() {
                break;
            }
            segments.push(self.identifier_from_token(segment_index));
            scan = segment_index + 1;
        }

        *cursor = scan;
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(
                segments
                    .first()
                    .expect("qualified name has a first segment")
                    .span
                    .span()
                    .start(),
                segments
                    .last()
                    .expect("qualified name has a last segment")
                    .span
                    .span()
                    .end(),
            ),
        );
        Some(QualifiedName { segments, span })
    }

    fn line_indent_of_token(&self, index: usize) -> usize {
        let start = self.tokens[index].span().start().as_usize();
        let line_start = self.source.text()[..start]
            .rfind('\n')
            .map(|position| position + 1)
            .unwrap_or(0);
        self.source.text()[line_start..start].chars().count()
    }

    fn starts_instance_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen
        )
    }

    fn starts_domain_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::TypeKw
        )
    }

    fn starts_type_companion_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::TypeKw
        )
    }

    fn find_next_instance_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_instance_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    fn find_next_domain_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_domain_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    fn find_next_type_companion_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_type_companion_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    /// Parses a qualified name in decorator position. Accepts keyword tokens as identifiers
    /// (e.g. `@source`) since both keywords and identifiers are valid decorator names.
    fn parse_decorator_qualified_name(
        &self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<QualifiedName> {
        self.parse_qualified_name(cursor, end)
    }

    fn identifier_from_token(&self, index: usize) -> Identifier {
        let token = self.tokens[index];
        Identifier {
            text: token.text(self.source).to_owned(),
            span: SourceSpan::new(self.source.id(), token.span()),
        }
    }

    fn tokens_are_adjacent(&self, left: usize, right: usize) -> bool {
        self.tokens[left].span().end() == self.tokens[right].span().start()
    }

    fn make_base(&self, start: usize, end: usize, decorators: Vec<Decorator>) -> ItemBase {
        ItemBase {
            span: self.source_span_for_range(start, end),
            token_range: TokenRange::new(start, end),
            decorators,
            leading_comments: Vec::new(),
        }
    }

    fn source_span_for_range(&self, start: usize, end: usize) -> SourceSpan {
        let first = self
            .next_significant_in_range(start, end)
            .unwrap_or(start.min(self.tokens.len().saturating_sub(1)));
        let last = self.prev_significant_in_range(start, end).unwrap_or(first);
        SourceSpan::new(
            self.source.id(),
            Span::new(
                self.tokens[first].span().start(),
                self.tokens[last].span().end(),
            ),
        )
    }

    fn source_span_of_token(&self, index: usize) -> SourceSpan {
        SourceSpan::new(self.source.id(), self.tokens[index].span())
    }

    fn join_spans(&self, left: SourceSpan, right: SourceSpan) -> SourceSpan {
        left.join(right).unwrap_or(left)
    }

    fn find_declaration_keyword(&self, start: usize) -> DecoratorSearch {
        let mut depth = 0usize;
        for index in start..self.tokens.len() {
            let token = self.tokens[index];
            if !token.kind().is_trivia() && token.line_start() && depth == 0 {
                match token.kind() {
                    kind if kind.is_top_level_keyword() && self.is_at_column_zero(index) => {
                        return DecoratorSearch {
                            keyword: Some(index),
                            offending: None,
                        };
                    }
                    TokenKind::At => {}
                    _ if index != start => {
                        return DecoratorSearch {
                            keyword: None,
                            offending: Some(index),
                        };
                    }
                    _ => {}
                }
            }

            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }

        DecoratorSearch {
            keyword: None,
            offending: None,
        }
    }

    /// Returns true when `index` token is at column 0 (no leading indentation on its line).
    /// This distinguishes top-level declarations from identically-named keywords used as
    /// variable references in indented expression bodies.
    fn is_at_column_zero(&self, index: usize) -> bool {
        let start = self.tokens[index].span().start().as_usize();
        if start == 0 {
            return true;
        }
        let text = self.source.text().as_bytes();
        // The token is at column 0 iff the immediately preceding byte is a newline.
        // Any whitespace (spaces/tabs) before the token means it is indented.
        matches!(text[start - 1], b'\n' | b'\r')
    }

    fn find_next_item_start(&self, from: usize) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..self.tokens.len() {
            let token = self.tokens[index];
            if !token.kind().is_trivia()
                && token.line_start()
                && depth == 0
                && (token.kind() == TokenKind::At || token.kind().is_top_level_keyword())
                && self.is_at_column_zero(index)
            {
                return Some(index);
            }

            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        None
    }

    fn next_significant_from(&self, start: usize) -> Option<usize> {
        self.next_significant_in_range(start, self.tokens.len())
    }

    /// Collect line comments from tokens in `[from, to)` that appear at the
    /// start of a line. Only the contiguous block of comments immediately
    /// before the item (index `to`) is kept; any blank line (span gap larger
    /// than a single newline) between comment groups resets the accumulator so
    /// we don't attach comments that belong to the previous item.
    fn collect_leading_comments(&self, from: usize, to: usize) -> Vec<String> {
        // Walk forward over [from, to), collecting LineComment tokens.
        // When we encounter a blank line we reset so only the final group
        // adjacent to the item is kept.
        let mut candidates: Vec<String> = Vec::new();
        let mut last_end: Option<u32> = None;
        for index in from..to {
            let token = self.tokens[index];
            if token.kind() == TokenKind::LineComment {
                // Check for blank line gap: if the byte distance from the
                // previous token's end to this token's start is more than one
                // newline, consider this a fresh block.
                if let Some(prev_end) = last_end {
                    let gap = token.span().start().as_u32().saturating_sub(prev_end);
                    if gap > 1 {
                        // There is at least one blank line — reset.
                        candidates.clear();
                    }
                }
                candidates.push(token.text(self.source).to_owned());
                last_end = Some(token.span().end().as_u32());
            } else if !token.kind().is_trivia() {
                // Unexpected non-trivia in this range — clear and stop.
                candidates.clear();
            } else {
                // Whitespace / block comment trivia: track position for gap detection.
                if last_end.is_none() {
                    last_end = Some(token.span().end().as_u32());
                }
            }
        }
        candidates
    }

    fn next_significant_in_range(&self, start: usize, end: usize) -> Option<usize> {
        self.tokens[start..end]
            .iter()
            .position(|token| !token.kind().is_trivia())
            .map(|offset| start + offset)
    }

    fn prev_significant_in_range(&self, start: usize, end: usize) -> Option<usize> {
        self.tokens[start..end]
            .iter()
            .rposition(|token| !token.kind().is_trivia())
            .map(|offset| start + offset)
    }

    fn peek_nontrivia(&self, start: usize, end: usize) -> Option<usize> {
        self.next_significant_in_range(start, end)
    }

    fn peek_kind(&self, start: usize, end: usize) -> Option<TokenKind> {
        self.peek_nontrivia(start, end)
            .map(|index| self.tokens[index].kind())
    }

    fn consume_kind(&self, cursor: &mut usize, end: usize, kind: TokenKind) -> Option<usize> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[index].kind() != kind {
            return None;
        }
        *cursor = index + 1;
        Some(index)
    }

    fn consume_identifier_text(
        &self,
        cursor: &mut usize,
        end: usize,
        expected: &str,
    ) -> Option<usize> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_identifier_text(index, expected) {
            return None;
        }
        *cursor = index + 1;
        Some(index)
    }

    fn is_identifier_text(&self, index: usize, expected: &str) -> bool {
        self.tokens[index].kind() == TokenKind::Identifier
            && self.tokens[index].text(self.source) == expected
    }

    fn starts_prefixed_collection_literal(
        &self,
        index: usize,
        end: usize,
        prefix: &str,
        opener: TokenKind,
    ) -> bool {
        self.is_identifier_text(index, prefix)
            && self
                .peek_nontrivia(index + 1, end)
                .is_some_and(|next| self.tokens[next].kind() == opener)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DecoratorSearch {
    keyword: Option<usize>,
    offending: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ExprStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    rbracket: bool,
    arrow: bool,
    pipe_stage: bool,
    /// Like `pipe_stage`, but only stops at pipe operators that begin a new line.
    /// Used for pipe-case arm bodies so inline `||>` continuations are allowed
    /// (e.g. `||> [first, ...rest] -> first ||> { email } -> email`).
    pipe_stage_line_start_only: bool,
    hash: bool,
    patch_entry: bool,
}

impl ExprStop {
    fn with_pipe_stage(mut self) -> Self {
        self.pipe_stage = true;
        self
    }

    fn with_hash(mut self) -> Self {
        self.hash = true;
        self
    }

    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn list_context() -> Self {
        Self {
            comma: true,
            rbracket: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn brace_context() -> Self {
        Self {
            rbrace: true,
            ..Self::default()
        }
    }

    fn patch_entry_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            patch_entry: true,
            ..Self::default()
        }
    }
}

fn patch_selector_segment_span(segment: &PatchSelectorSegment) -> SourceSpan {
    match segment {
        PatchSelectorSegment::Named { span, .. }
        | PatchSelectorSegment::BracketTraverse { span }
        | PatchSelectorSegment::BracketExpr { span, .. } => *span,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PatternStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    rbracket: bool,
    arrow: bool,
    fat_arrow: bool,
}

impl PatternStop {
    fn arrow_context() -> Self {
        Self {
            arrow: true,
            ..Self::default()
        }
    }

    fn signal_reactive_arm_context() -> Self {
        Self {
            fat_arrow: true,
            ..Self::default()
        }
    }

    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn list_context() -> Self {
        Self {
            comma: true,
            rbracket: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn brace_context() -> Self {
        Self {
            rbrace: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TypeStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    arrow: bool,
    thin_arrow: bool,
    pipe_transform: bool,
}

impl TypeStop {
    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn constraint_attempt() -> Self {
        Self {
            arrow: true,
            thin_arrow: true,
            ..Self::default()
        }
    }

    fn with_pipe_transform(mut self) -> Self {
        self.pipe_transform = true;
        self
    }
}

fn is_record_row_transform_name(name: &str) -> bool {
    matches!(
        name,
        "Pick" | "Omit" | "Optional" | "Required" | "Defaulted" | "Rename"
    )
}

fn text_escape_end(text: &str, start: usize, end: usize) -> usize {
    let mut cursor = start + 1;
    if cursor >= end {
        return cursor;
    }
    let escaped = text[cursor..end]
        .chars()
        .next()
        .expect("escaped text segment must stay on a UTF-8 boundary");
    cursor += escaped.len_utf8();
    match escaped {
        'u' => {
            let bytes = text.as_bytes();
            if cursor < end && bytes[cursor] == b'{' {
                cursor += 1;
                while cursor < end && bytes[cursor] != b'}' {
                    cursor += 1;
                }
                if cursor < end {
                    cursor += 1;
                }
            }
            cursor
        }
        'x' => {
            let bytes = text.as_bytes();
            let mut hex_digits = 0usize;
            while hex_digits < 2 && cursor < end && bytes[cursor].is_ascii_hexdigit() {
                cursor += 1;
                hex_digits += 1;
            }
            cursor
        }
        _ => cursor,
    }
}

fn domain_member_surface_name_str(name: &DomainMemberName) -> String {
    match name {
        DomainMemberName::Signature(ClassMemberName::Identifier(id)) => id.text.clone(),
        DomainMemberName::Signature(ClassMemberName::Operator(op)) => op.text.clone(),
        DomainMemberName::Literal(id) => id.text.clone(),
    }
}

fn decode_text_fragment(raw: &str) -> String {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        let Some(escaped) = chars.next() else {
            decoded.push('\\');
            break;
        };
        match escaped {
            'n' => decoded.push('\n'),
            't' => decoded.push('\t'),
            'r' => decoded.push('\r'),
            '\\' => decoded.push('\\'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '0' => decoded.push('\0'),
            '{' => decoded.push('{'),
            '}' => decoded.push('}'),
            'u' => {
                let mut consumed = String::from("\\u");
                let Some('{') = chars.peek().copied() else {
                    decoded.push_str(&consumed);
                    continue;
                };
                consumed.push(chars.next().expect("peeked opening brace must exist"));
                let mut digits = String::new();
                let mut terminated = false;
                while let Some(next) = chars.next() {
                    consumed.push(next);
                    if next == '}' {
                        terminated = true;
                        break;
                    }
                    digits.push(next);
                }
                match terminated
                    .then(|| u32::from_str_radix(&digits, 16).ok())
                    .flatten()
                    .and_then(char::from_u32)
                {
                    Some(ch) => decoded.push(ch),
                    None => decoded.push_str(&consumed),
                }
            }
            'x' => {
                let mut consumed = String::from("\\x");
                let mut digits = String::new();
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    if !next.is_ascii_hexdigit() {
                        break;
                    }
                    digits.push(next);
                    consumed.push(next);
                    chars.next();
                }
                match u8::from_str_radix(&digits, 16).ok() {
                    Some(value) => decoded.push(char::from(value)),
                    None => decoded.push_str(&consumed),
                }
            }
            other => {
                decoded.push('\\');
                decoded.push(other);
            }
        }
    }
    decoded
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;

    use super::*;
    use crate::{ItemKind, TokenKind, lex_module};

    fn load(input: &str) -> (SourceDatabase, ParsedModule) {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("test.aivi", input.to_owned());
        let parsed = {
            let file = &sources[file_id];
            parse_module(file)
        };
        (sources, parsed)
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/frontend/milestone-1")
    }

    fn parse_fixture(relative_path: &str) -> ParsedModule {
        let path = fixture_root().join(relative_path);
        let text = fs::read_to_string(&path).expect("fixture must be readable");
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        parse_module(&sources[file_id])
    }

    #[test]
    fn lexer_recognizes_pipe_operators_class_keywords_and_regex_literals() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "operators.aivi",
            r#"class Eq A = {
    (==) : A -> A -> Bool
}
instance Eq Blob = {
    (==) left right = same left right
}
domain Duration over Int = {
    literal ms : Int -> Duration
    (*) : Duration -> Int -> Duration
}
signal flow = value |> compute ?|> ready ||> Ready -> keep *|> .email &|> build @|> loop <|@ step | debug <|* merge T|> start F|> stop
value same = left == right
value different = left != right
fun picked = value!
value quotient = left / right
value remainder = left % right
value range = 1..10
value chained =
    result {
        value <- Ok 1
        value
    }
<Label text={status} />
</match>
value datePattern = rx"\d{4}-\d{2}-\d{2}"
"#,
        );
        let file = &sources[file_id];
        let lexed = lex_module(file);
        let kinds: Vec<_> = lexed
            .tokens()
            .iter()
            .filter(|token| !token.kind().is_trivia())
            .map(|token| token.kind())
            .collect();

        assert!(kinds.contains(&TokenKind::ClassKw));
        assert!(kinds.contains(&TokenKind::InstanceKw));
        assert!(kinds.contains(&TokenKind::DomainKw));
        assert!(kinds.contains(&TokenKind::ThinArrow));
        assert!(kinds.contains(&TokenKind::EqualEqual));
        assert!(kinds.contains(&TokenKind::Bang));
        assert!(kinds.contains(&TokenKind::BangEqual));
        assert!(kinds.contains(&TokenKind::Star));
        assert!(kinds.contains(&TokenKind::Slash));
        assert!(kinds.contains(&TokenKind::Percent));
        assert!(kinds.contains(&TokenKind::DotDot));
        assert!(kinds.contains(&TokenKind::LeftArrow));
        assert!(kinds.contains(&TokenKind::PipeTransform));
        assert!(kinds.contains(&TokenKind::PipeGate));
        assert!(kinds.contains(&TokenKind::PipeCase));
        assert!(kinds.contains(&TokenKind::PipeMap));
        assert!(kinds.contains(&TokenKind::PipeApply));
        assert!(kinds.contains(&TokenKind::PipeRecurStart));
        assert!(kinds.contains(&TokenKind::PipeRecurStep));
        assert!(kinds.contains(&TokenKind::PipeTap));
        assert!(kinds.contains(&TokenKind::PipeFanIn));
        assert!(kinds.contains(&TokenKind::TruthyBranch));
        assert!(kinds.contains(&TokenKind::FalsyBranch));
        assert!(kinds.contains(&TokenKind::SelfCloseTagEnd));
        assert!(kinds.contains(&TokenKind::CloseTagStart));
        assert!(kinds.contains(&TokenKind::RegexLiteral));
        assert!(lexed.diagnostics().is_empty());
    }

    #[test]
    fn parser_preserves_bare_root_patch_field_selectors() {
        let (_, parsed) = load("value promote = patch { isAdmin: True }\n");
        assert!(
            !parsed.has_errors(),
            "expected patch shorthand to parse cleanly, got diagnostics: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Some(Item::Value(item)) = parsed.module.items().first() else {
            panic!("expected a value item");
        };
        let Some(expr) = item.expr_body() else {
            panic!("expected the value item to have an expression body");
        };
        let ExprKind::PatchLiteral(patch) = &expr.kind else {
            panic!("expected the value body to be a patch literal");
        };
        let Some(entry) = patch.entries.first() else {
            panic!("expected the patch literal to contain one entry");
        };
        let [PatchSelectorSegment::Named { name, dotted, .. }] = entry.selector.segments.as_slice()
        else {
            panic!("expected one named selector segment");
        };

        assert_eq!(name.text, "isAdmin");
        assert!(
            !*dotted,
            "expected the root field selector to stay undotted"
        );
    }

    #[test]
    fn lexer_distinguishes_line_and_doc_comments_as_trivia() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(
            "comments.aivi",
            "/** module doc **/\nvalue answer = 42 // inline note\n",
        );
        let lexed = lex_module(&sources[file_id]);
        let comment_kinds = lexed
            .tokens()
            .iter()
            .filter_map(|token| match token.kind() {
                TokenKind::DocComment | TokenKind::LineComment => Some(token.kind()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            comment_kinds,
            vec![TokenKind::DocComment, TokenKind::LineComment]
        );
        assert!(comment_kinds.iter().all(|kind| kind.is_trivia()));
        assert!(lexed.diagnostics().is_empty());
    }

    #[test]
    fn parser_builds_structured_items_and_source_decorators() {
        let (_, parsed) = load(
            r#"@source http.get "/users" with {
    decode: Strict,
    retry: 3
}
signal users : Signal User

type Bool = True | False
value answer = 42
fun add: Int = x:Int y:Int => x + y
use aivi.network (
    http
)
export main
"#,
        );

        assert!(!parsed.has_errors());
        assert_eq!(parsed.module.items.len(), 6);
        assert_eq!(parsed.module.items[0].kind(), ItemKind::Signal);
        assert_eq!(parsed.module.items[1].kind(), ItemKind::Type);
        assert_eq!(parsed.module.items[2].kind(), ItemKind::Value);
        assert_eq!(parsed.module.items[3].kind(), ItemKind::Fun);
        assert_eq!(parsed.module.items[4].kind(), ItemKind::Use);
        assert_eq!(parsed.module.items[5].kind(), ItemKind::Export);

        match &parsed.module.items[0] {
            Item::Signal(item) => {
                assert_eq!(item.base.decorators.len(), 1);
                assert_eq!(item.base.decorators[0].name.as_dotted(), "source");
                assert_eq!(
                    item.name.as_ref().map(|name| name.text.as_str()),
                    Some("users")
                );
                match &item.base.decorators[0].payload {
                    DecoratorPayload::Source(source) => {
                        assert_eq!(
                            source
                                .provider
                                .as_ref()
                                .map(QualifiedName::as_dotted)
                                .as_deref(),
                            Some("http.get")
                        );
                        assert_eq!(source.arguments.len(), 1);
                        assert!(source.options.is_some());
                    }
                    other => panic!("expected source decorator, got {other:?}"),
                }
            }
            other => panic!("expected a signal item, got {other:?}"),
        }

        match &parsed.module.items[1] {
            Item::Type(item) => match item.type_body() {
                Some(TypeDeclBody::Sum(sum)) => assert_eq!(sum.variants.len(), 2),
                other => panic!("expected sum type body, got {other:?}"),
            },
            other => panic!("expected type item, got {other:?}"),
        }

        match &parsed.module.items[3] {
            Item::Fun(item) => {
                assert!(!item.parameters.is_empty());
                assert!(matches!(
                    item.expr_body().map(|expr| &expr.kind),
                    Some(ExprKind::Binary { .. })
                ));
            }
            other => panic!("expected fun item with parameters, got {other:?}"),
        }

        match &parsed.module.items[4] {
            Item::Use(item) => {
                assert_eq!(
                    item.path.as_ref().map(QualifiedName::as_dotted).as_deref(),
                    Some("aivi.network")
                );
                assert_eq!(item.imports.len(), 1);
                assert_eq!(item.imports[0].path.as_dotted(), "http");
                assert!(item.imports[0].alias.is_none());
            }
            other => panic!("expected use item, got {other:?}"),
        }
    }

    #[test]
    fn parser_builds_sum_type_companions_inside_brace_bodies() {
        let (_, parsed) = load(
            r#"type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Type(item) = &parsed.module.items[0] else {
            panic!("expected type item");
        };
        let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
            panic!("expected sum type body");
        };
        assert_eq!(sum.variants.len(), 2);
        assert_eq!(sum.companions.len(), 1);
        assert_eq!(sum.companions[0].name.text, "opponent");
        assert_eq!(
            sum.companions[0].function_form,
            FunctionSurfaceForm::Explicit
        );
        assert_eq!(sum.companions[0].parameters.len(), 1);
        assert!(sum.companions[0].annotation.is_some());
        assert!(sum.companions[0].body.is_some());
    }

    #[test]
    fn parser_builds_sum_type_companions_with_unary_subject_sugar() {
        let (_, parsed) = load(
            r#"type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = .
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Type(item) = &parsed.module.items[0] else {
            panic!("expected type item");
        };
        let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
            panic!("expected sum type body");
        };
        assert_eq!(sum.companions.len(), 1);
        assert_eq!(
            sum.companions[0].function_form,
            FunctionSurfaceForm::UnarySubjectSugar
        );
        assert_eq!(sum.companions[0].parameters.len(), 1);
        assert!(sum.companions[0].annotation.is_some());
        assert!(sum.companions[0].body.is_some());
    }

    #[test]
    fn parser_builds_inline_annotated_sum_type_companions_with_unary_subject_sugar() {
        let (_, parsed) = load(
            r#"type Player = {
    | Human
    | Computer

    opponent: Player -> Player = .
     ||> Human    -> Computer
     ||> Computer -> Human
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Type(item) = &parsed.module.items[0] else {
            panic!("expected type item");
        };
        let Some(TypeDeclBody::Sum(sum)) = item.type_body() else {
            panic!("expected sum type body");
        };
        assert_eq!(sum.companions.len(), 1);
        assert_eq!(sum.companions[0].name.text, "opponent");
        assert_eq!(
            sum.companions[0].function_form,
            FunctionSurfaceForm::UnarySubjectSugar
        );
        assert_eq!(sum.companions[0].parameters.len(), 1);
        assert!(sum.companions[0].annotation.is_some());
        assert!(sum.companions[0].body.is_some());
    }

    #[test]
    fn parser_builds_result_blocks_with_bindings_and_tail() {
        let (_, parsed) = load(
            r#"value total =
result {
        left <- Ok 20
        right <- Ok 22
        left + right
    }
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Value(item) = &parsed.module.items[0] else {
            panic!("expected value item");
        };
        let ExprKind::ResultBlock(block) = &item.expr_body().expect("value body").kind else {
            panic!("expected result block body");
        };
        assert_eq!(block.bindings.len(), 2);
        assert_eq!(block.bindings[0].name.text, "left");
        assert_eq!(block.bindings[1].name.text, "right");
        assert!(matches!(
            block.tail.as_deref().map(|expr| &expr.kind),
            Some(ExprKind::Binary { .. })
        ));
    }

    #[test]
    fn parser_builds_single_source_signal_merge() {
        let (_, parsed) = load(
            r#"signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        assert_eq!(parsed.module.items.len(), 1);
        assert_eq!(parsed.module.items[0].kind(), ItemKind::Signal);

        let Item::Signal(item) = &parsed.module.items[0] else {
            panic!("expected signal item");
        };
        let merge = item.merge_body().expect("expected merge body");
        assert_eq!(merge.sources.len(), 1);
        assert_eq!(merge.sources[0].text, "ready");
        assert_eq!(merge.arms.len(), 2);
        assert!(merge.arms[0].source.is_none());
        assert!(merge.arms[1].source.is_none());
    }

    #[test]
    fn parser_builds_multi_source_signal_merge() {
        let (_, parsed) = load(
            r#"signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowUp") => Turn North
  ||> _ => Tick
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Signal(item) = &parsed.module.items[0] else {
            panic!("expected signal item");
        };
        let merge = item.merge_body().expect("expected merge body");
        assert_eq!(merge.sources.len(), 2);
        assert_eq!(merge.sources[0].text, "tick");
        assert_eq!(merge.sources[1].text, "keyDown");
        assert_eq!(merge.arms.len(), 3);
        assert_eq!(
            merge.arms[0].source.as_ref().map(|s| s.text.as_str()),
            Some("tick")
        );
        assert_eq!(
            merge.arms[1].source.as_ref().map(|s| s.text.as_str()),
            Some("keyDown")
        );
        // Default arm has no source prefix
        assert!(merge.arms[2].source.is_none());
    }

    #[test]
    fn parser_builds_signal_merge_with_expression_body() {
        let (_, parsed) = load(
            r#"signal total : Signal Int = ready
  ||> True => left + right
  ||> _ => 0
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Signal(item) = &parsed.module.items[0] else {
            panic!("expected signal item");
        };
        let merge = item.merge_body().expect("expected merge body");
        assert_eq!(merge.sources.len(), 1);
        assert_eq!(merge.arms.len(), 2);
        assert!(matches!(
            merge.arms[0].body.as_ref().map(|e| &e.kind),
            Some(ExprKind::Binary { .. })
        ));
    }

    #[test]
    fn parser_distinguishes_signal_merge_from_pipe_expression() {
        let (_, parsed) = load(
            r#"signal derived = someSignal |> transform
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Signal(item) = &parsed.module.items[0] else {
            panic!("expected signal item");
        };
        // This should be an Expr body, not a Merge body.
        assert!(item.expr_body().is_some());
        assert!(item.merge_body().is_none());
    }

    #[test]
    fn parser_builds_multiline_accumulate_pipe_signal_bodies() {
        let (_, parsed) = load(
            r#"type Key =
  | Left
type Direction =
  | East
fun updateDirection:Direction = key:Key current:Direction => current
signal keyDown: Signal Key = Left
signal direction: Signal Direction = keyDown
 +|> East updateDirection
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Signal(item) = &parsed.module.items[4] else {
            panic!("expected signal item");
        };
        let ExprKind::Pipe(pipe) = &item.expr_body().expect("signal body").kind else {
            panic!("expected signal body to parse as a pipe");
        };
        assert!(matches!(
            pipe.head.as_deref().map(|expr| &expr.kind),
            Some(ExprKind::Name(identifier)) if identifier.text == "keyDown"
        ));
        assert_eq!(pipe.stages.len(), 1);
        let PipeStageKind::Accumulate { seed, step } = &pipe.stages[0].kind else {
            panic!("expected accumulate pipe stage");
        };
        assert!(matches!(seed.kind, ExprKind::Name(ref identifier) if identifier.text == "East"));
        assert!(
            matches!(step.kind, ExprKind::Name(ref identifier) if identifier.text == "updateDirection")
        );
    }

    #[test]
    fn parser_builds_delay_and_burst_pipe_signal_bodies() {
        let (_, parsed) = load(
            r#"signal clicks: Signal Int = 1
signal delayed: Signal Int = clicks
 |> delay 200ms
signal flashed: Signal Int = clicks
 |> burst 75ms 3times
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Signal(delayed) = &parsed.module.items[1] else {
            panic!("expected delayed signal item");
        };
        let ExprKind::Pipe(delay_pipe) = &delayed.expr_body().expect("delay body").kind else {
            panic!("expected delayed signal body to parse as a pipe");
        };
        let PipeStageKind::Delay { duration } = &delay_pipe.stages[0].kind else {
            panic!("expected delay pipe stage");
        };
        assert!(matches!(duration.kind, ExprKind::SuffixedInteger(_)));

        let Item::Signal(flashed) = &parsed.module.items[2] else {
            panic!("expected flashed signal item");
        };
        let ExprKind::Pipe(burst_pipe) = &flashed.expr_body().expect("burst body").kind else {
            panic!("expected burst signal body to parse as a pipe");
        };
        let PipeStageKind::Burst { every, count } = &burst_pipe.stages[0].kind else {
            panic!("expected burst pipe stage");
        };
        assert!(matches!(every.kind, ExprKind::SuffixedInteger(_)));
        assert!(matches!(count.kind, ExprKind::SuffixedInteger(_)));
    }

    #[test]
    fn parser_reports_removed_temporal_pipe_operator_spellings() {
        let (_, parsed) = load(
            r#"signal clicks: Signal Int = 1
signal delayed: Signal Int = clicks
 delay|> 200ms
signal flashed: Signal Int = clicks
 burst|> 75ms 3times
"#,
        );

        assert!(parsed.has_errors());
        assert!(
            parsed
                .all_diagnostics()
                .any(|diagnostic| diagnostic.code == Some(REMOVED_TEMPORAL_PIPE_OPERATOR))
        );
    }

    #[test]
    fn parser_builds_from_signal_fanout_entries() {
        let (_, parsed) = load(
            r#"from state = {
    boardText: renderBoard
    dirLine: .dir |> dirLabel
    gameOver: .status
        ||> Running -> False
        ||> GameOver -> True
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        assert_eq!(parsed.module.items.len(), 1);

        let Item::From(item) = &parsed.module.items[0] else {
            panic!("expected from item");
        };
        assert!(matches!(
            item.source.as_ref().map(|expr| &expr.kind),
            Some(ExprKind::Name(name)) if name.text == "state"
        ));
        assert_eq!(item.entries.len(), 3);
        assert_eq!(item.entries[0].name.text, "boardText");
        assert_eq!(item.entries[1].name.text, "dirLine");
        assert_eq!(item.entries[2].name.text, "gameOver");
        assert!(matches!(
            item.entries[1].body.as_ref().map(|expr| &expr.kind),
            Some(ExprKind::Pipe(_))
        ));
        assert!(matches!(
            item.entries[2].body.as_ref().map(|expr| &expr.kind),
            Some(ExprKind::Pipe(_))
        ));
    }

    #[test]
    fn parser_allows_result_blocks_to_use_the_last_binding_as_the_implicit_tail() {
        let (_, parsed) = load(
            r#"value lastValue =
    result {
        payload <- Ok 42
    }
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Value(item) = &parsed.module.items[0] else {
            panic!("expected value item");
        };
        let ExprKind::ResultBlock(block) = &item.expr_body().expect("value body").kind else {
            panic!("expected result block body");
        };
        assert_eq!(block.bindings.len(), 1);
        assert!(block.tail.is_none(), "tail should stay implicit in the CST");
    }

    #[test]
    fn parser_builds_use_import_aliases() {
        let (_, parsed) = load(
            r#"use aivi.network (
    http as primaryHttp
    Request as HttpRequest
)
"#,
        );

        assert!(!parsed.has_errors());
        let Item::Use(item) = &parsed.module.items[0] else {
            panic!("expected use item");
        };
        assert_eq!(item.imports.len(), 2);
        assert_eq!(item.imports[0].path.as_dotted(), "http");
        assert_eq!(
            item.imports[0]
                .alias
                .as_ref()
                .map(|alias| alias.text.as_str()),
            Some("primaryHttp")
        );
        assert_eq!(item.imports[1].path.as_dotted(), "Request");
        assert_eq!(
            item.imports[1]
                .alias
                .as_ref()
                .map(|alias| alias.text.as_str()),
            Some("HttpRequest")
        );
    }

    #[test]
    fn parser_builds_grouped_exports() {
        let (_, parsed) = load(
            r#"export (bundledSupportSentinel, BundledSupportToken)
"#,
        );

        assert!(!parsed.has_errors());
        let Item::Export(item) = &parsed.module.items[0] else {
            panic!("expected export item");
        };
        assert_eq!(
            item.targets
                .iter()
                .map(|target| target.text.as_str())
                .collect::<Vec<_>>(),
            vec!["bundledSupportSentinel", "BundledSupportToken"]
        );
    }

    #[test]
    fn lexer_treats_removed_top_level_aliases_as_identifiers() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("aliases.aivi", "source view result adapter data");
        let lexed = lex_module(&sources[file_id]);
        let kinds: Vec<_> = lexed
            .tokens()
            .iter()
            .filter(|token| !token.kind().is_trivia())
            .map(|token| token.kind())
            .collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
                TokenKind::Identifier,
            ]
        );
    }

    #[test]
    fn parser_rejects_removed_top_level_alias_declarations() {
        let (_, parsed) = load(
            "source ticks : Signal Int\nview main = 0\nresult bundle = 0\nadapter glue = 0\ndata Flag = On | Off\n",
        );

        assert!(
            parsed.has_errors(),
            "removed alias declarations should stay invalid"
        );
    }

    #[test]
    fn parser_structures_text_interpolation_segments() {
        let (_, parsed) = load(r#"value greeting = "Hello {name}, use \{literal\} braces""#);

        assert!(!parsed.has_errors());
        match &parsed.module.items[0] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Text(text)) => {
                    assert_eq!(text.segments.len(), 3);
                    assert!(matches!(
                        &text.segments[0],
                        TextSegment::Text(fragment) if fragment.raw == "Hello "
                    ));
                    assert!(matches!(
                        &text.segments[1],
                        TextSegment::Interpolation(interpolation)
                            if matches!(interpolation.expr.kind, ExprKind::Name(ref identifier) if identifier.text == "name")
                    ));
                    assert!(matches!(
                        &text.segments[2],
                        TextSegment::Text(fragment)
                            if fragment.raw == ", use {literal} braces"
                    ));
                }
                other => panic!("expected interpolated text literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_decodes_text_escape_sequences() {
        let (_, parsed) = load(r#"value board = "top\nbottom \u{41} \x42 \{ok\}""#);

        assert!(!parsed.has_errors());
        match &parsed.module.items[0] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Text(text)) => {
                    assert_eq!(text.segments.len(), 1);
                    assert!(matches!(
                        &text.segments[0],
                        TextSegment::Text(fragment)
                            if fragment.raw == "top\nbottom A B {ok}"
                    ));
                }
                other => panic!("expected text literal, got {other:?}"),
            },
            other => panic!("expected value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_builds_class_members_and_equality_operators_from_fixture() {
        let parsed = parse_fixture("valid/top-level/class_eq.aivi");

        assert!(!parsed.has_errors());
        assert_eq!(parsed.module.items.len(), 2);
        assert_eq!(parsed.module.items[0].kind(), ItemKind::Class);

        match &parsed.module.items[0] {
            Item::Class(item) => {
                assert_eq!(
                    item.name.as_ref().map(|name| name.text.as_str()),
                    Some("Eq")
                );
                assert_eq!(
                    item.type_parameters
                        .iter()
                        .map(|parameter| parameter.text.as_str())
                        .collect::<Vec<_>>(),
                    vec!["A"]
                );
                let body = item.class_body().expect("class item should have a body");
                assert_eq!(body.members.len(), 1);
                assert!(matches!(
                    body.members[0].name,
                    ClassMemberName::Operator(ref operator) if operator.text == "=="
                ));
                assert!(matches!(
                    body.members[0].annotation.as_ref().map(|ty| &ty.kind),
                    Some(TypeExprKind::Arrow { .. })
                ));
            }
            other => panic!("expected class item, got {other:?}"),
        }

        match &parsed.module.items[1] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator: BinaryOperator::And,
                    left,
                    right,
                }) => {
                    assert!(matches!(
                        left.kind,
                        ExprKind::Binary {
                            operator: BinaryOperator::Equals,
                            ..
                        }
                    ));
                    assert!(matches!(
                        right.kind,
                        ExprKind::Binary {
                            operator: BinaryOperator::NotEquals,
                            ..
                        }
                    ));
                }
                other => panic!("expected `and` root with equality subexpressions, got {other:?}"),
            },
            Item::Fun(_) => {}
            other => panic!("expected function item, got {other:?}"),
        }
    }

    #[test]
    fn parser_respects_binary_precedence_and_left_associativity() {
        let (_, parsed) = load(
            "value ranked = left + middle > threshold and ready or fallback\nvalue diff = a - b - c\n",
        );

        assert!(!parsed.has_errors());

        match &parsed.module.items[0] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator: BinaryOperator::Or,
                    left,
                    right,
                }) => {
                    assert!(matches!(
                        &right.kind,
                        ExprKind::Name(identifier) if identifier.text == "fallback"
                    ));
                    match &left.kind {
                        ExprKind::Binary {
                            operator: BinaryOperator::And,
                            left,
                            right,
                        } => {
                            assert!(matches!(
                                &right.kind,
                                ExprKind::Name(identifier) if identifier.text == "ready"
                            ));
                            match &left.kind {
                                ExprKind::Binary {
                                    operator: BinaryOperator::GreaterThan,
                                    left,
                                    right,
                                } => {
                                    assert!(matches!(
                                        &right.kind,
                                        ExprKind::Name(identifier) if identifier.text == "threshold"
                                    ));
                                    assert!(matches!(
                                        &left.kind,
                                        ExprKind::Binary {
                                            operator: BinaryOperator::Add,
                                            ..
                                        }
                                    ));
                                }
                                other => panic!("expected comparison before `and`, got {other:?}"),
                            }
                        }
                        other => panic!("expected `and` before `or`, got {other:?}"),
                    }
                }
                other => panic!("expected precedence-shaped binary tree, got {other:?}"),
            },
            other => panic!("expected ranked value item, got {other:?}"),
        }

        match &parsed.module.items[1] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator: BinaryOperator::Subtract,
                    left,
                    right,
                }) => {
                    assert!(matches!(
                        &right.kind,
                        ExprKind::Name(identifier) if identifier.text == "c"
                    ));
                    assert!(matches!(
                        &left.kind,
                        ExprKind::Binary {
                            operator: BinaryOperator::Subtract,
                            ..
                        }
                    ));
                }
                other => panic!("expected left-associative subtraction tree, got {other:?}"),
            },
            other => panic!("expected diff value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_respects_multiplicative_precedence_and_left_associativity() {
        let (_, parsed) =
            load("value total = base + rate * scale\nvalue grouped = total / count % bucket\n");

        assert!(!parsed.has_errors());

        match &parsed.module.items[0] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator: BinaryOperator::Add,
                    left,
                    right,
                }) => {
                    assert!(matches!(
                        &left.kind,
                        ExprKind::Name(identifier) if identifier.text == "base"
                    ));
                    assert!(matches!(
                        &right.kind,
                        ExprKind::Binary {
                            operator: BinaryOperator::Multiply,
                            ..
                        }
                    ));
                }
                other => panic!("expected additive root with multiplicative rhs, got {other:?}"),
            },
            other => panic!("expected total value item, got {other:?}"),
        }

        match &parsed.module.items[1] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator: BinaryOperator::Modulo,
                    left,
                    right,
                }) => {
                    assert!(matches!(
                        &right.kind,
                        ExprKind::Name(identifier) if identifier.text == "bucket"
                    ));
                    assert!(matches!(
                        &left.kind,
                        ExprKind::Binary {
                            operator: BinaryOperator::Divide,
                            ..
                        }
                    ));
                }
                other => panic!("expected left-associative multiplicative tree, got {other:?}"),
            },
            other => panic!("expected grouped value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_builds_instance_members_with_parameters_and_multiline_bodies() {
        let (_, parsed) = load(
            r#"class Eq A = {
    (==) : A -> A -> Bool
}

fun same:Bool = left:Blob right:Blob => True

instance Eq Blob = {
    (==) left right =
        same left right
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "{:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        assert_eq!(parsed.module.items[2].kind(), ItemKind::Instance);

        let Item::Instance(item) = &parsed.module.items[2] else {
            panic!("expected instance item");
        };
        assert_eq!(
            item.class.as_ref().map(QualifiedName::as_dotted).as_deref(),
            Some("Eq")
        );
        assert!(matches!(
            item.target.as_ref().map(|ty| &ty.kind),
            Some(TypeExprKind::Name(name)) if name.text == "Blob"
        ));
        let body = item.body.as_ref().expect("instance should have a body");
        assert_eq!(body.members.len(), 1);
        assert!(matches!(
            body.members[0].name,
            ClassMemberName::Operator(ref operator) if operator.text == "=="
        ));
        assert_eq!(
            body.members[0]
                .parameters
                .iter()
                .map(|parameter| parameter.text.as_str())
                .collect::<Vec<_>>(),
            vec!["left", "right"]
        );
        assert!(matches!(
            body.members[0].body.as_ref().map(|expr| &expr.kind),
            Some(ExprKind::Apply { .. })
        ));
    }

    #[test]
    fn parser_builds_domain_members_from_fixture() {
        let parsed = parse_fixture("valid/top-level/domains.aivi");

        assert!(!parsed.has_errors());
        match &parsed.module.items[1] {
            Item::Domain(item) => {
                assert_eq!(
                    item.name.as_ref().map(|name| name.text.as_str()),
                    Some("Path")
                );
                assert!(matches!(
                    item.carrier.as_ref().map(|carrier| &carrier.kind),
                    Some(TypeExprKind::Name(identifier)) if identifier.text == "Text"
                ));
                let body = item.body.as_ref().expect("domain should have a body");
                assert_eq!(body.members.len(), 2);
                assert!(matches!(
                    body.members[0].name,
                    DomainMemberName::Literal(ref suffix) if suffix.text == "root"
                ));
                assert!(matches!(
                    body.members[1].name,
                    DomainMemberName::Signature(ClassMemberName::Operator(ref operator))
                        if operator.text == "/"
                ));
            }
            other => panic!("expected domain item, got {other:?}"),
        }
    }

    #[test]
    fn parser_does_not_treat_thin_arrow_as_constraint_separator() {
        // Standalone type annotations starting with (MultiCharApply ...) ->
        // must parse as function types, NOT as constrained types.
        let (_, parsed) = load(concat!(
            "type (List A) -> (Option A) -> (List A)\n",
            "func appendPrev = items prev => items\n",
        ));
        assert!(
            !parsed.has_errors(),
            "standalone function type with (List A) -> should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        // The standalone `type` attaches to the `func`, so items[0] is Func
        let Item::Fun(func_item) = &parsed.module.items[0] else {
            panic!("expected func item with attached type annotation");
        };
        assert!(
            func_item.constraints.is_empty(),
            "expected no constraints — (List A) is a type constructor, not a class constraint"
        );
    }

    #[test]
    fn parser_tracks_constraint_prefixes_on_functions_and_instances() {
        let (_, parsed) = load(
            r#"class Functor F = {
    map : (A -> B) -> F A -> F B
}
fun same:Eq A => Bool = v:A => v == v
instance Eq A => Eq (Option A) = {
    (==) left right = True
}
"#,
        );
        assert!(
            !parsed.has_errors(),
            "expected constrained signatures to parse cleanly, got diagnostics: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Fun(function) = &parsed.module.items[1] else {
            panic!("expected constrained function item");
        };
        assert_eq!(function.constraints.len(), 1);

        let Item::Instance(instance) = &parsed.module.items[2] else {
            panic!("expected constrained instance item");
        };
        assert_eq!(instance.context.len(), 1);
    }

    #[test]
    fn parser_desugars_type_level_record_row_pipes_into_nested_applications() {
        let (_, parsed) = load(concat!(
            "type User = { id: Int, name: Text, createdAt: Text }\n",
            "type Public = User |> Pick (id, createdAt) |> Rename { createdAt: created_at }\n",
        ));
        assert!(
            !parsed.has_errors(),
            "record row transform types should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Type(item) = &parsed.module.items[1] else {
            panic!("expected second item to be a type alias");
        };
        let Some(TypeDeclBody::Alias(alias)) = item.type_body() else {
            panic!("expected type alias body");
        };

        let TypeExprKind::Apply { callee, arguments } = &alias.kind else {
            panic!("expected piped transform to desugar into an application");
        };
        let TypeExprKind::Name(name) = &callee.kind else {
            panic!("expected outer transform callee to be a name");
        };
        assert_eq!(name.text, "Rename");
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[0].kind, TypeExprKind::Record(_)));

        let TypeExprKind::Apply {
            callee: inner_callee,
            arguments: inner_arguments,
        } = &arguments[1].kind
        else {
            panic!("expected inner piped transform to stay nested");
        };
        let TypeExprKind::Name(inner_name) = &inner_callee.kind else {
            panic!("expected inner transform callee to be a name");
        };
        assert_eq!(inner_name.text, "Pick");
        assert_eq!(inner_arguments.len(), 2);
        assert!(matches!(inner_arguments[0].kind, TypeExprKind::Tuple(_)));
        assert!(matches!(
            &inner_arguments[1].kind,
            TypeExprKind::Name(name) if name.text == "User"
        ));
    }

    #[test]
    fn parser_tracks_constraint_prefixes_on_class_members() {
        let (_, parsed) = load(
            r#"class Functor F = {
    map:Applicative G=>(A -> G B) -> F A -> G (F B)
}
"#,
        );
        assert!(
            !parsed.has_errors(),
            "expected constrained class member to parse cleanly, got diagnostics: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Class(class_item) = &parsed.module.items[0] else {
            panic!("expected class item");
        };
        let body = class_item
            .class_body()
            .expect("class item should have a body");
        assert_eq!(body.members.len(), 1);
        assert_eq!(body.members[0].constraints.len(), 1);
        assert!(
            body.members[0].annotation.is_some(),
            "expected class member annotation, got {:?}",
            body.members[0]
        );
    }

    #[test]
    fn parser_rejects_class_head_constraint_prefixes() {
        let (_, parsed) = load(
            r#"class (Functor F, Foldable F) -> Traversable F = {
    traverse : Applicative G -> (A -> G B) -> F A -> G (F B)
}
"#,
        );

        assert!(
            parsed.has_errors(),
            "expected class-head constraint prefixes to be rejected"
        );
        assert!(
            parsed
                .all_diagnostics()
                .any(|diagnostic| diagnostic.code == Some(UNSUPPORTED_CLASS_HEAD_CONSTRAINTS)),
            "expected unsupported class-head constraint diagnostic, got: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
    }

    #[test]
    fn parser_builds_provider_contract_members_from_fixture() {
        let parsed = parse_fixture("valid/top-level/provider_contracts.aivi");

        assert!(!parsed.has_errors());
        assert_eq!(
            parsed.module.items[0].kind(),
            ItemKind::SourceProviderContract
        );
        match &parsed.module.items[0] {
            Item::SourceProviderContract(item) => {
                assert_eq!(
                    item.provider
                        .as_ref()
                        .map(QualifiedName::as_dotted)
                        .as_deref(),
                    Some("custom.feed")
                );
                let body = item
                    .body
                    .as_ref()
                    .expect("provider contract should have a body");
                assert_eq!(body.members.len(), 5);
                match &body.members[0] {
                    SourceProviderContractMember::ArgumentSchema(member) => {
                        assert_eq!(
                            member.name.as_ref().map(|name| name.text.as_str()),
                            Some("path")
                        );
                    }
                    other => panic!("expected argument schema member, got {other:?}"),
                }
                match &body.members[1] {
                    SourceProviderContractMember::OptionSchema(member) => {
                        assert_eq!(
                            member.name.as_ref().map(|name| name.text.as_str()),
                            Some("timeout")
                        );
                    }
                    other => panic!("expected option schema member, got {other:?}"),
                }
                match &body.members[2] {
                    SourceProviderContractMember::OperationSchema(member) => {
                        assert_eq!(
                            member.name.as_ref().map(|name| name.text.as_str()),
                            Some("read")
                        );
                    }
                    other => panic!("expected operation schema member, got {other:?}"),
                }
                match &body.members[3] {
                    SourceProviderContractMember::CommandSchema(member) => {
                        assert_eq!(
                            member.name.as_ref().map(|name| name.text.as_str()),
                            Some("delete")
                        );
                    }
                    other => panic!("expected command schema member, got {other:?}"),
                }
                match &body.members[4] {
                    SourceProviderContractMember::FieldValue(member) => {
                        assert_eq!(
                            member.name.as_ref().map(|name| name.text.as_str()),
                            Some("wakeup")
                        );
                        assert_eq!(
                            member.value.as_ref().map(|value| value.text.as_str()),
                            Some("providerTrigger")
                        );
                    }
                    other => panic!("expected wakeup field member, got {other:?}"),
                }
            }
            other => panic!("expected provider contract item, got {other:?}"),
        }
    }

    #[test]
    fn parser_distinguishes_compact_literal_suffixes_from_spaced_application() {
        let (_, parsed) = load(
            "domain Duration over Int = {\n    literal ms : Int -> Duration\n}\nvalue compact = 250ms\nvalue spaced = 250 ms\n",
        );

        assert!(!parsed.has_errors());
        match &parsed.module.items[1] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::SuffixedInteger(literal)) => {
                    assert_eq!(literal.literal.raw, "250");
                    assert_eq!(literal.suffix.text, "ms");
                }
                other => panic!("expected compact suffixed integer, got {other:?}"),
            },
            other => panic!("expected compact value item, got {other:?}"),
        }

        match &parsed.module.items[2] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Apply { callee, arguments }) => {
                    assert!(matches!(callee.kind, ExprKind::Integer(_)));
                    assert_eq!(arguments.len(), 1);
                    assert!(matches!(
                        arguments[0].kind,
                        ExprKind::Name(ref identifier) if identifier.text == "ms"
                    ));
                }
                other => panic!("expected spaced application, got {other:?}"),
            },
            other => panic!("expected spaced value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_distinguishes_builtin_noninteger_literals_from_suffix_candidates() {
        fn expect_float(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::Float(literal)) => {
                        assert_eq!(literal.raw, raw);
                    }
                    other => panic!("expected float literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_decimal(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::Decimal(literal)) => {
                        assert_eq!(literal.raw, raw);
                    }
                    other => panic!("expected decimal literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_bigint(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::BigInt(literal)) => {
                        assert_eq!(literal.raw, raw);
                    }
                    other => panic!("expected bigint literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_suffixed(item: &Item, raw: &str, suffix: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::SuffixedInteger(literal)) => {
                        assert_eq!(literal.literal.raw, raw);
                        assert_eq!(literal.suffix.text, suffix);
                    }
                    other => panic!("expected suffixed integer literal candidate, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        let (_, parsed) = load(
            "value bigint = 123n\nvalue decimal = 19d\nvalue precise = 19.25d\nvalue floaty = 3.5\nvalue hexish = 0xFF\n",
        );

        assert!(!parsed.has_errors());
        expect_bigint(&parsed.module.items[0], "123n");
        expect_decimal(&parsed.module.items[1], "19d");
        expect_decimal(&parsed.module.items[2], "19.25d");
        expect_float(&parsed.module.items[3], "3.5");
        expect_suffixed(&parsed.module.items[4], "0", "xFF");
    }

    #[test]
    fn parser_accepts_adjacent_negative_numeric_literals() {
        fn expect_integer(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::Integer(literal)) => assert_eq!(literal.raw, raw),
                    other => panic!("expected integer literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_float(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::Float(literal)) => assert_eq!(literal.raw, raw),
                    other => panic!("expected float literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_decimal(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::Decimal(literal)) => assert_eq!(literal.raw, raw),
                    other => panic!("expected decimal literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_bigint(item: &Item, raw: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::BigInt(literal)) => assert_eq!(literal.raw, raw),
                    other => panic!("expected bigint literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        fn expect_suffixed(item: &Item, raw: &str, suffix: &str) {
            match item {
                Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                    Some(ExprKind::SuffixedInteger(literal)) => {
                        assert_eq!(literal.literal.raw, raw);
                        assert_eq!(literal.suffix.text, suffix);
                    }
                    other => panic!("expected suffixed integer literal, got {other:?}"),
                },
                other => panic!("expected value item, got {other:?}"),
            }
        }

        let (_, parsed) = load(
            "domain Duration over Int = {\n    literal ms : Int -> Duration\n}\nvalue negativeInt = -1\nvalue negativeFloat = -3.4\nvalue negativeDecimal = -19d\nvalue negativePreciseDecimal = -19.25d\nvalue negativeBigInt = -123n\nvalue negativeDuration = -250ms\nvalue subtract = 4 - 3\n",
        );

        assert!(
            !parsed.has_errors(),
            "adjacent negative literals should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        expect_integer(&parsed.module.items[1], "-1");
        expect_float(&parsed.module.items[2], "-3.4");
        expect_decimal(&parsed.module.items[3], "-19d");
        expect_decimal(&parsed.module.items[4], "-19.25d");
        expect_bigint(&parsed.module.items[5], "-123n");
        expect_suffixed(&parsed.module.items[6], "-250", "ms");
        match &parsed.module.items[7] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Binary {
                    operator,
                    left,
                    right,
                }) => {
                    assert_eq!(*operator, BinaryOperator::Subtract);
                    assert!(
                        matches!(left.kind, ExprKind::Integer(ref literal) if literal.raw == "4")
                    );
                    assert!(
                        matches!(right.kind, ExprKind::Integer(ref literal) if literal.raw == "3")
                    );
                }
                other => panic!("expected subtraction expression, got {other:?}"),
            },
            other => panic!("expected subtract value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_spaced_negative_literal_prefixes() {
        let (_, parsed) = load("value badInt = - 3\nvalue badFloat = - 3.4\n");

        assert!(
            parsed.has_errors(),
            "spaced negative literals should stay invalid"
        );
    }

    #[test]
    fn parser_accepts_negative_integer_patterns() {
        let (_, parsed) = load(
            "fun isNegativeOne:Bool = value:Int => value\n  ||> -1 -> True\n  ||> _ -> False\n",
        );

        assert!(
            !parsed.has_errors(),
            "negative integer patterns should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Fun(item) = &parsed.module.items[0] else {
            panic!("expected a function item");
        };
        let ExprKind::Pipe(pipe) = &item.expr_body().expect("function should carry a body").kind
        else {
            panic!("expected the function body to remain a pipe");
        };
        let PipeStageKind::Case(first_case) = &pipe.stages[0].kind else {
            panic!("expected first stage to be a case arm");
        };
        assert!(matches!(
            first_case.pattern.kind,
            PatternKind::Integer(ref literal) if literal.raw == "-1"
        ));
    }

    #[test]
    fn parser_accepts_domain_member_bindings_after_type_annotation() {
        let (_, parsed) = load(
            r#"type Builder = Int -> Duration
domain Duration over Int = {
    type Builder
    make raw = raw
}
"#,
        );

        assert!(
            !parsed.has_errors(),
            "domain member bindings should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Domain(item) = &parsed.module.items[1] else {
            panic!("expected a domain item");
        };
        let body = item.body.as_ref().expect("domain should carry a body");
        assert_eq!(body.members.len(), 1);
        assert!(body.members[0].annotation.is_some());
        assert_eq!(body.members[0].parameters.len(), 1);
        assert_eq!(body.members[0].parameters[0].text, "raw");
        assert!(matches!(
            body.members[0].body.as_ref().map(|expr| &expr.kind),
            Some(ExprKind::Name(identifier)) if identifier.text == "raw"
        ));
    }

    #[test]
    fn parser_builds_map_and_set_literals_without_consuming_bare_names() {
        let (_, parsed) = load(
            "value headers = Map { \"Authorization\": token, \"Accept\": \"application/json\" }\nvalue tags = Set [1, 2, selected]\nvalue bare = Map\n",
        );

        assert!(!parsed.has_errors());

        match &parsed.module.items[0] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Map(map)) => {
                    assert_eq!(map.entries.len(), 2);
                    assert!(matches!(map.entries[0].key.kind, ExprKind::Text(_)));
                    assert!(matches!(
                        map.entries[0].value.kind,
                        ExprKind::Name(ref identifier) if identifier.text == "token"
                    ));
                    assert!(matches!(map.entries[1].key.kind, ExprKind::Text(_)));
                    assert!(matches!(map.entries[1].value.kind, ExprKind::Text(_)));
                }
                other => panic!("expected map literal, got {other:?}"),
            },
            other => panic!("expected map value item, got {other:?}"),
        }

        match &parsed.module.items[1] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Set(elements)) => {
                    assert_eq!(elements.len(), 3);
                    assert!(matches!(elements[0].kind, ExprKind::Integer(_)));
                    assert!(matches!(
                        elements[2].kind,
                        ExprKind::Name(ref identifier) if identifier.text == "selected"
                    ));
                }
                other => panic!("expected set literal, got {other:?}"),
            },
            other => panic!("expected set value item, got {other:?}"),
        }

        match &parsed.module.items[2] {
            Item::Value(item) => match item.expr_body().map(|expr| &expr.kind) {
                Some(ExprKind::Name(identifier)) => assert_eq!(identifier.text, "Map"),
                other => panic!("expected bare `Map` name, got {other:?}"),
            },
            other => panic!("expected bare value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_reports_missing_domain_over_and_carrier() {
        let (_, missing_over) = load("domain Duration Int\n");
        assert!(missing_over.has_errors());
        assert!(
            missing_over
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(MISSING_DOMAIN_OVER))
        );

        let (_, missing_carrier) = load("domain Duration over\n");
        assert!(missing_carrier.has_errors());
        assert!(
            missing_carrier
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(MISSING_DOMAIN_CARRIER))
        );
    }

    #[test]
    fn parser_reports_missing_item_name() {
        let (_, parsed) = load("value = 42\n");

        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(MISSING_ITEM_NAME))
        );
        match &parsed.module.items[0] {
            Item::Value(item) => assert!(item.name.is_none()),
            other => panic!("expected a value item, got {other:?}"),
        }
    }

    #[test]
    fn parser_reports_missing_grouped_export_targets() {
        let (_, parsed) = load("export ()\n");

        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(MISSING_EXPORT_NAME))
        );
        match &parsed.module.items[0] {
            Item::Export(item) => assert!(item.targets.is_empty()),
            other => panic!("expected an export item, got {other:?}"),
        }
    }

    #[test]
    fn parser_reports_trailing_tokens_after_expression_body() {
        let (_, parsed) =
            load("fun prependCells:List Int = head:Int tail:List Int =>\n    head :: tail\n");

        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(TRAILING_DECLARATION_BODY_TOKEN))
        );
    }

    #[test]
    fn parser_accepts_valid_fixture_corpus() {
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
            let mut sources = SourceDatabase::new();
            let file_id = sources.add_file(&fixture, text);
            let parsed = parse_module(&sources[file_id]);
            assert!(
                !parsed.has_errors(),
                "expected valid fixture {} to parse cleanly, got diagnostics: {:?}",
                fixture.display(),
                parsed.all_diagnostics().collect::<Vec<_>>()
            );
            assert!(
                !parsed.module.items.is_empty(),
                "{} should contain items",
                fixture.display()
            );
        }
    }

    #[test]
    fn parser_flags_only_syntax_invalid_fixtures() {
        for relative in [
            "invalid/markup_mismatched_close.aivi",
            "invalid/markup_child_interpolation.aivi",
        ] {
            let parsed = parse_fixture(relative);
            assert!(
                parsed.has_errors(),
                "{relative} should report syntax errors"
            );
        }

        for relative in [
            "invalid/pattern_non_exhaustive_sum.aivi",
            "invalid/val_depends_on_sig.aivi",
            "invalid/source_unknown_option.aivi",
            "invalid/record_missing_required_field.aivi",
            "invalid/each_missing_key.aivi",
            "invalid/gate_non_list.aivi",
            "invalid/regex_bad_pattern.aivi",
            "invalid/regex_invalid_quantifier.aivi",
            "invalid/cluster_unfinished_gate.aivi",
        ] {
            let parsed = parse_fixture(relative);
            assert!(
                !parsed.has_errors(),
                "{relative} should remain for later semantic milestones: {:?}",
                parsed.all_diagnostics().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn parser_preserves_qualified_markup_tag_names() {
        let (_, parsed) = load(
            r#"
value view =
    <Window>
        <Paned.start>
            <Label />
        </Paned.start>
    </Window>
"#,
        );
        assert!(
            !parsed.has_errors(),
            "qualified markup names should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Value(view) = &parsed.module.items()[0] else {
            panic!("expected the test item to be a value declaration");
        };
        let ExprKind::Markup(root) = &view
            .expr_body()
            .expect("test value should carry a markup expression body")
            .kind
        else {
            panic!("expected the test value body to be markup");
        };
        let paned_start = root
            .children
            .first()
            .expect("window markup should contain the qualified child-group wrapper");
        assert_eq!(paned_start.name.as_dotted(), "Paned.start");
        assert_eq!(
            paned_start
                .close_name
                .as_ref()
                .expect("qualified wrapper should keep its close tag")
                .as_dotted(),
            "Paned.start"
        );
    }

    #[test]
    fn parser_accepts_subject_placeholders_ranges_and_discard_params() {
        let (_, parsed) = load(
            r#"value subject = .
value projection = .email
value span = 1..10
value values = [1..10]
fun ignore:Int = _ => 0
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected subject/range surface forms to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Value(subject) = &parsed.module.items[0] else {
            panic!("expected subject value item");
        };
        assert!(matches!(
            subject.expr_body().map(|expr| &expr.kind),
            Some(ExprKind::SubjectPlaceholder)
        ));

        let Item::Value(projection) = &parsed.module.items[1] else {
            panic!("expected projection value item");
        };
        assert!(matches!(
            projection.expr_body().map(|expr| &expr.kind),
            Some(ExprKind::AmbientProjection(path))
                if path.fields.len() == 1 && path.fields[0].text == "email"
        ));

        let Item::Value(span) = &parsed.module.items[2] else {
            panic!("expected span value item");
        };
        assert!(matches!(
            span.expr_body().map(|expr| &expr.kind),
            Some(ExprKind::Range { .. })
        ));

        let Item::Value(values) = &parsed.module.items[3] else {
            panic!("expected values item");
        };
        assert!(matches!(
            values.expr_body().map(|expr| &expr.kind),
            Some(ExprKind::List(elements))
                if matches!(elements.as_slice(), [Expr { kind: ExprKind::Range { .. }, .. }])
        ));

        let Item::Fun(ignore) = &parsed.module.items[4] else {
            panic!("expected ignore function item");
        };
        assert_eq!(ignore.parameters.len(), 1);
        assert!(ignore.parameters[0].name.is_none());
    }

    #[test]
    fn parser_accepts_unary_subject_function_bodies_without_arrows() {
        let (_, parsed) = load(
            r#"fun currentStatus:Text = .status
fun scoreLineFor:Text = "Score: {.}"
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected unary subject function sugar to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Fun(current_status) = &parsed.module.items[0] else {
            panic!("expected currentStatus function item");
        };
        assert_eq!(
            current_status.function_form,
            FunctionSurfaceForm::UnarySubjectSugar
        );
        assert_eq!(current_status.parameters.len(), 1);
        assert!(matches!(
            current_status.expr_body().map(|expr| &expr.kind),
            Some(ExprKind::Projection { base, path })
                if matches!(base.kind, ExprKind::Name(ref identifier) if identifier.text == IMPLICIT_FUNCTION_SUBJECT_NAME)
                    && path.fields.len() == 1
                    && path.fields[0].text == "status"
        ));

        let Item::Fun(score_line_for) = &parsed.module.items[1] else {
            panic!("expected scoreLineFor function item");
        };
        assert_eq!(
            score_line_for.function_form,
            FunctionSurfaceForm::UnarySubjectSugar
        );
        assert_eq!(score_line_for.parameters.len(), 1);
        let Some(Expr {
            kind: ExprKind::Text(text),
            ..
        }) = score_line_for.expr_body()
        else {
            panic!("expected scoreLineFor to lower into a text literal body");
        };
        assert!(matches!(
            text.segments.as_slice(),
            [TextSegment::Text(fragment), TextSegment::Interpolation(interpolation)]
                if fragment.raw == "Score: "
                    && matches!(
                        interpolation.expr.kind,
                        ExprKind::Name(ref identifier)
                            if identifier.text == IMPLICIT_FUNCTION_SUBJECT_NAME
                    )
        ));
    }

    #[test]
    fn parser_accepts_selected_subject_pipe_bodies_without_arrows() {
        let (_, parsed) = load(
            r#"fun flipsFromDirection = board player coord vector!
 |> rayFrom coord #ray
 |> collectRay board ray
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected selected-subject pipe sugar to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Fun(flips_from_direction) = &parsed.module.items[0] else {
            panic!("expected flipsFromDirection function item");
        };
        assert_eq!(
            flips_from_direction.function_form,
            FunctionSurfaceForm::SelectedSubjectSugar
        );
        assert_eq!(flips_from_direction.parameters.len(), 4);
        let Some(Expr {
            kind: ExprKind::Pipe(pipe),
            ..
        }) = flips_from_direction.expr_body()
        else {
            panic!("expected selected-subject body to parse as a pipe");
        };
        assert!(matches!(
            pipe.head.as_deref().map(|expr| &expr.kind),
            Some(ExprKind::Name(identifier)) if identifier.text == "vector"
        ));
        assert_eq!(pipe.stages.len(), 2);
        assert_eq!(
            pipe.stages[0]
                .result_memo
                .as_ref()
                .map(|memo| memo.text.as_str()),
            Some("ray")
        );
    }

    #[test]
    fn parser_accepts_selected_subject_patch_bodies_without_arrows() {
        let (_, parsed) = load(
            r#"fun recordOpponent = state! coord
    <| {
        collecting: True,
        closed: False,
        trail: [coord]
    }
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected selected-subject patch sugar to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Fun(record_opponent) = &parsed.module.items[0] else {
            panic!("expected recordOpponent function item");
        };
        assert_eq!(
            record_opponent.function_form,
            FunctionSurfaceForm::SelectedSubjectSugar
        );
        let Some(Expr {
            kind: ExprKind::PatchApply { target, patch },
            ..
        }) = record_opponent.expr_body()
        else {
            panic!("expected selected-subject body to parse as a patch apply");
        };
        assert!(matches!(&target.kind, ExprKind::Name(identifier) if identifier.text == "state"));
        assert_eq!(patch.entries.len(), 3);
    }

    #[test]
    fn parser_accepts_selected_subject_record_selectors() {
        let (_, parsed) = load(
            r#"fun readNested = state { x.y.z! }
 |> render
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected selected-subject record selector to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Fun(read_nested) = &parsed.module.items[0] else {
            panic!("expected readNested function item");
        };
        assert_eq!(
            read_nested.function_form,
            FunctionSurfaceForm::SelectedSubjectSugar
        );
        let Some(Expr {
            kind: ExprKind::Pipe(pipe),
            ..
        }) = read_nested.expr_body()
        else {
            panic!("expected record-selector body to parse as a pipe");
        };
        let Some(Expr {
            kind: ExprKind::Projection { base, path },
            ..
        }) = pipe.head.as_deref()
        else {
            panic!("expected record-selector body head to parse as a projection");
        };
        assert!(matches!(&base.kind, ExprKind::Name(identifier) if identifier.text == "state"));
        assert_eq!(
            path.fields
                .iter()
                .map(|field| field.text.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "y", "z"]
        );
    }

    #[test]
    fn parser_rejects_nullary_function_declarations() {
        let (_, parsed) = load("fun constant:Int = => 1\n");

        assert!(parsed.has_errors(), "nullary functions should stay invalid");
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code == Some(NULLARY_FUNCTION_DECLARATION) })
        );
    }

    #[test]
    fn parser_accepts_pipe_subject_and_result_memos() {
        let (_, parsed) = load(
            r#"value memoed =
    20
     |> #before before + 1 #after
     |> after + before
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected pipe memos to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Value(value) = &parsed.module.items[0] else {
            panic!("expected memoed value item");
        };
        let Some(Expr {
            kind: ExprKind::Pipe(pipe),
            ..
        }) = value.expr_body()
        else {
            panic!("expected value body to be a pipe expression");
        };
        assert_eq!(pipe.stages.len(), 2);
        let first = &pipe.stages[0];
        assert_eq!(
            first
                .subject_memo
                .as_ref()
                .expect("first stage should preserve subject memo")
                .text,
            "before"
        );
        assert_eq!(
            first
                .result_memo
                .as_ref()
                .expect("first stage should preserve result memo")
                .text,
            "after"
        );
        assert!(pipe.stages[1].subject_memo.is_none());
        assert!(pipe.stages[1].result_memo.is_none());
    }

    #[test]
    fn parser_accepts_pipe_case_stage_memos() {
        let (_, parsed) = load(
            r#"value memoed = Some 2
 ||> #incoming Some value -> value + 1 #resolved
 ||> None -> 0 #resolved
 |> resolved
"#,
        );

        assert!(
            !parsed.has_errors(),
            "expected case-stage pipe memos to parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Value(value) = &parsed.module.items[0] else {
            panic!("expected memoed value item");
        };
        let Some(Expr {
            kind: ExprKind::Pipe(pipe),
            ..
        }) = value.expr_body()
        else {
            panic!("expected value body to be a pipe expression");
        };
        assert_eq!(pipe.stages.len(), 3);
        assert_eq!(
            pipe.stages[0]
                .subject_memo
                .as_ref()
                .expect("first case arm should preserve the subject memo")
                .text,
            "incoming"
        );
        assert_eq!(
            pipe.stages[0]
                .result_memo
                .as_ref()
                .expect("first case arm should preserve the result memo")
                .text,
            "resolved"
        );
        assert_eq!(
            pipe.stages[1]
                .result_memo
                .as_ref()
                .expect("second case arm should preserve the shared result memo")
                .text,
            "resolved"
        );
    }

    #[test]
    fn parser_builds_hoist_item_with_no_filters() {
        let (_, parsed) = load("hoist\n");
        assert!(
            !parsed.has_errors(),
            "hoist should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        assert_eq!(parsed.module.items.len(), 1);
        let Item::Hoist(hoist) = &parsed.module.items[0] else {
            panic!("expected hoist item");
        };
        assert!(hoist.kind_filters.is_empty());
        assert!(hoist.hiding.is_empty());
    }

    #[test]
    fn parser_builds_hoist_item_with_kind_filters() {
        let (_, parsed) = load("hoist (func, value)\n");
        assert!(
            !parsed.has_errors(),
            "hoist with filters should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Hoist(hoist) = &parsed.module.items[0] else {
            panic!("expected hoist item");
        };
        assert_eq!(hoist.kind_filters.len(), 2);
        assert_eq!(hoist.kind_filters[0].text, "func");
        assert_eq!(hoist.kind_filters[1].text, "value");
    }

    #[test]
    fn parser_builds_hoist_item_with_hiding_clause() {
        let (_, parsed) = load("hoist hiding (length, head)\n");
        assert!(
            !parsed.has_errors(),
            "hoist with hiding should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Hoist(hoist) = &parsed.module.items[0] else {
            panic!("expected hoist item");
        };
        assert!(hoist.kind_filters.is_empty());
        assert_eq!(hoist.hiding.len(), 2);
        assert_eq!(hoist.hiding[0].text, "length");
        assert_eq!(hoist.hiding[1].text, "head");
    }

    #[test]
    fn parser_builds_hoist_item_with_filters_and_hiding() {
        let (_, parsed) = load("hoist (func, value) hiding (map, filter)\n");
        assert!(
            !parsed.has_errors(),
            "hoist with filters and hiding should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let Item::Hoist(hoist) = &parsed.module.items[0] else {
            panic!("expected hoist item");
        };
        assert_eq!(hoist.kind_filters.len(), 2);
        assert_eq!(hoist.hiding.len(), 2);
        assert_eq!(hoist.hiding[0].text, "map");
        assert_eq!(hoist.hiding[1].text, "filter");
    }

    #[test]
    fn parser_rejects_discard_exprs_and_markup_child_interpolation() {
        let (_, parsed) = load(
            r#"value current = _
value view =
    <Label>{current}</Label>
"#,
        );

        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(INVALID_DISCARD_EXPR))
        );
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(INVALID_MARKUP_CHILD_CONTENT))
        );
    }
}
