use aivi_base::{Diagnostic, DiagnosticCode, Severity, SourceFile, SourceSpan, Span};

use crate::{
    cst::{
        BigIntLiteral, BinaryOperator, ClassBody, ClassMember, ClassMemberName, DecimalLiteral,
        Decorator, DecoratorArguments, DecoratorPayload, DomainBody, DomainItem, DomainMember,
        DomainMemberName, ErrorItem, ExportItem, Expr, ExprKind, FloatLiteral, FunctionParam,
        Identifier, InstanceBody, InstanceItem, InstanceMember, IntegerLiteral, Item, ItemBase,
        MapExpr, MapExprEntry, MarkupAttribute, MarkupAttributeValue, MarkupNode, Module,
        NamedItem, NamedItemBody, OperatorName, Pattern, PatternKind, PipeCaseArm, PipeExpr,
        PipeStage, PipeStageKind, ProjectionPath, QualifiedName, RecordExpr, RecordField,
        RecordPatternField, RegexLiteral, SourceDecorator, SourceProviderContractBody,
        SourceProviderContractFieldValue, SourceProviderContractItem, SourceProviderContractMember,
        SourceProviderContractSchemaMember, SuffixedIntegerLiteral, TextFragment,
        TextInterpolation, TextLiteral, TextSegment, TokenRange, TypeDeclBody, TypeExpr,
        TypeExprKind, TypeField, TypeVariant, UnaryOperator, UseImport, UseItem,
    },
    lex::{LexedModule, Token, TokenKind, lex_fragment, lex_module},
};

const UNEXPECTED_TOP_LEVEL_TOKEN: DiagnosticCode =
    DiagnosticCode::new("syntax", "unexpected-top-level-token");
const MISSING_DECORATOR_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-decorator-name");
const DANGLING_DECORATOR_BLOCK: DiagnosticCode =
    DiagnosticCode::new("syntax", "dangling-decorator-block");
const MISSING_ITEM_NAME: DiagnosticCode = DiagnosticCode::new("syntax", "missing-item-name");
const MISSING_USE_PATH: DiagnosticCode = DiagnosticCode::new("syntax", "missing-use-path");
const MISSING_USE_ALIAS: DiagnosticCode = DiagnosticCode::new("syntax", "missing-use-alias");
const MISSING_EXPORT_NAME: DiagnosticCode = DiagnosticCode::new("syntax", "missing-export-name");
const MISSING_DECLARATION_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-declaration-body");
const TRAILING_DECLARATION_BODY_TOKEN: DiagnosticCode =
    DiagnosticCode::new("syntax", "trailing-declaration-body-token");
const MISSING_CLASS_MEMBER_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-class-member-type");
const MISSING_INSTANCE_CLASS: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-class");
const MISSING_INSTANCE_TARGET: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-target");
const MISSING_INSTANCE_MEMBER_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-member-body");
const MISSING_DOMAIN_OVER: DiagnosticCode = DiagnosticCode::new("syntax", "missing-domain-over");
const MISSING_DOMAIN_CARRIER: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-carrier");
const MISSING_DOMAIN_MEMBER_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-member-name");
const MISSING_DOMAIN_MEMBER_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-member-type");
const MISSING_PROVIDER_CONTRACT_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-name");
const MISSING_PROVIDER_CONTRACT_MEMBER_VALUE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-member-value");
const MISSING_PROVIDER_CONTRACT_SCHEMA_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-schema-name");
const MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-schema-type");
const MISMATCHED_MARKUP_CLOSE: DiagnosticCode =
    DiagnosticCode::new("syntax", "mismatched-markup-close");
const UNTERMINATED_MARKUP_NODE: DiagnosticCode =
    DiagnosticCode::new("syntax", "unterminated-markup-node");
const INVALID_MARKUP_CHILD_CONTENT: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-markup-child-content");
const INVALID_TEXT_INTERPOLATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-text-interpolation");
const UNTERMINATED_TEXT_INTERPOLATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "unterminated-text-interpolation");
const INVALID_DISCARD_EXPR: DiagnosticCode = DiagnosticCode::new("syntax", "invalid-discard-expr");
const MISSING_PIPE_MEMO_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-pipe-memo-name");
const PARSE_DEPTH_EXCEEDED: DiagnosticCode = DiagnosticCode::new("syntax", "parse-depth-exceeded");

const MAX_PARSE_DEPTH: usize = 256;

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
                    .with_primary_label(span, "maximum parse depth exceeded here"),
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
        while let Some(start) = self.next_significant_from(self.cursor) {
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
            items.push(item);
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
                    ),
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
            TokenKind::DataKw => {
                Item::Data(self.parse_type_item(base, keyword_index, end, "data declaration"))
            }
            TokenKind::FunKw => {
                Item::Fun(self.parse_fun_item(base, keyword_index, end))
            }
            TokenKind::ValueKw => {
                Item::Value(self.parse_value_item(base, keyword_index, end))
            }
            TokenKind::SignalKw => {
                Item::Signal(self.parse_signal_item(base, keyword_index, end, "signal declaration"))
            }
            TokenKind::SourceKw => {
                Item::Source(self.parse_source_item(base, keyword_index, end))
            }
            TokenKind::ResultDeclKw => {
                Item::ResultDecl(self.parse_value_item_with_eq(
                    base,
                    keyword_index,
                    end,
                    "result declaration",
                ))
            }
            TokenKind::ViewKw => {
                Item::View(self.parse_value_item_with_eq(
                    base,
                    keyword_index,
                    end,
                    "view declaration",
                ))
            }
            TokenKind::AdapterKw => {
                Item::Adapter(self.parse_value_item_with_eq(
                    base,
                    keyword_index,
                    end,
                    "adapter declaration",
                ))
            }
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
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_class_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let (constraints, name, type_parameters) =
            self.parse_class_head(keyword_index, &mut cursor, end);

        let body = self
            .parse_class_body(&mut cursor, end)
            .map(NamedItemBody::Class)
            .or_else(|| {
                self.missing_body_diagnostic(
                    keyword_index,
                    "class declaration is missing its member signatures",
                    "expected one or more member signatures on following lines",
                );
                None
            });

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            constraints,
            annotation: None,
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
                    ),
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
                        ),
                );
                None
            });
        let body = self.parse_instance_body(&mut cursor, end).or_else(|| {
            self.missing_body_diagnostic(
                keyword_index,
                "instance declaration is missing its member bindings",
                "expected one or more instance member bindings on following lines",
            );
            None
        });
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
                    ),
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
                    ),
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
    fn parse_value_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name =
            self.parse_named_item_name(keyword_index, &mut cursor, end, "value declaration");
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
            parameters: Vec::new(),
            body,
        }
    }

    /// Parse a `fun` declaration: function form with parameters, uses `=>`.
    fn parse_fun_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name =
            self.parse_named_item_name(keyword_index, &mut cursor, end, "fun declaration");
        let (constraints, annotation) = self.parse_function_signature_annotation(&mut cursor, end);

        let mut parameters = Vec::new();
        while self.starts_function_param(cursor, end) {
            let Some(parameter) = self.parse_function_param(&mut cursor, end) else {
                break;
            };
            parameters.push(parameter);
        }

        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Arrow)
            .is_some()
        {
            self.parse_expression_body(
                keyword_index,
                &mut cursor,
                end,
                "fun declaration",
                "fun declaration is missing its body after `=>`",
                "expected a body expression after `=>`",
            )
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                "fun declaration is missing its body",
                "expected parameters followed by `=>`",
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
            parameters,
            body,
        }
    }

    /// Parse a `source`, `result`, `view`, or `adapter` declaration using `=` body form.
    fn parse_value_item_with_eq(
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
            self.parse_expression_body(
                keyword_index,
                &mut cursor,
                end,
                description,
                &format!("{description} is missing its body after `=`"),
                "expected an expression after `=`",
            )
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                &format!("{description} is missing its body"),
                "expected `=` followed by an expression",
            );
            None
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints: Vec::new(),
            annotation,
            parameters: Vec::new(),
            body,
        }
    }

    /// Parse a `source` declaration — same surface shape as `signal` with optional type + `=` body.
    fn parse_source_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> NamedItem {
        self.parse_signal_item(base, keyword_index, end, "source declaration")
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
            self.parse_expression_body(
                keyword_index,
                &mut cursor,
                end,
                "signal declaration",
                "signal declaration is missing its body after `=`",
                "expected an expression after `=`",
            )
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
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_class_body(&mut self, cursor: &mut usize, end: usize) -> Option<ClassBody> {
        let body_start = *cursor;
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.tokens[index].line_start() {
                break;
            }
            let Some(member) = self.parse_class_member(cursor, end) else {
                break;
            };
            members.push(member);
        }

        (!members.is_empty()).then_some(ClassBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_instance_body(&mut self, cursor: &mut usize, end: usize) -> Option<InstanceBody> {
        let body_start = *cursor;
        let first_index = self.peek_nontrivia(*cursor, end)?;
        if !self.tokens[first_index].line_start() || !self.starts_instance_member(first_index) {
            return None;
        }
        let member_indent = self.line_indent_of_token(first_index);
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.tokens[index].line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_instance_member(index)
            {
                break;
            }
            let before = *cursor;
            let Some(member) = self.parse_instance_member(cursor, end, member_indent) else {
                break;
            };
            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        (!members.is_empty()).then_some(InstanceBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_domain_body(&mut self, cursor: &mut usize, end: usize) -> Option<DomainBody> {
        let body_start = *cursor;
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.tokens[index].line_start() {
                break;
            }
            let Some(member) = self.parse_domain_member(cursor, end) else {
                break;
            };
            members.push(member);
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

    fn parse_domain_member(&mut self, cursor: &mut usize, end: usize) -> Option<DomainMember> {
        let start = *cursor;
        let name = if self
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
            DomainMemberName::Literal(suffix)
        } else {
            DomainMemberName::Signature(self.parse_signature_member_name(cursor, end)?)
        };

        let annotation = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
            self.parse_type_expr(cursor, end, TypeStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("domain member is missing its type after `:`")
                            .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                            .with_primary_label(
                                name.span(),
                                "expected a member type such as `Int -> Duration`",
                            ),
                    );
                    None
                })
        } else {
            self.diagnostics.push(
                Diagnostic::error("domain member is missing `:` before its type")
                    .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                    .with_primary_label(name.span(), "expected `:` followed by a member type"),
            );
            None
        };

        Some(DomainMember {
            name,
            annotation,
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
            "option" | "argument" => {
                let schema_name = self.parse_identifier(cursor, end).or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("provider contract schema member is missing its name")
                            .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_NAME)
                            .with_primary_label(
                                name.span,
                                format!(
                                    "expected a {} name such as `timeout`",
                                    if name.text == "option" {
                                        "source option"
                                    } else {
                                        "source argument"
                                    }
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
                                        "expected a {} type such as `Text` or `Signal Bool`",
                                        if name.text == "option" {
                                            "source option"
                                        } else {
                                            "source argument"
                                        }
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
                Some(if name.text == "option" {
                    SourceProviderContractMember::OptionSchema(member)
                } else {
                    SourceProviderContractMember::ArgumentSchema(member)
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
                    "expected `type`, `val`, `fun`, `sig`, `class`, `use`, `export`, or `@decorator` here",
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
        self.parse_optional_signature_annotation(cursor, body_arrow)
    }

    fn parse_constrained_type(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && self.consume_kind(cursor, end, TokenKind::Arrow).is_some()
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
            TypeStop::default(),
        )?])
    }

    fn parse_optional_constraint_prefix(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<TypeExpr> {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && self.consume_kind(cursor, end, TokenKind::Arrow).is_some()
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
    ) -> (Vec<TypeExpr>, Option<Identifier>, Vec<Identifier>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && self.consume_kind(cursor, end, TokenKind::Arrow).is_some()
        {
            let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
            let type_parameters = self.parse_type_parameters_same_line(cursor, end);
            return (constraints, name, type_parameters);
        }
        *cursor = checkpoint;
        let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
        let type_parameters = self.parse_type_parameters_same_line(cursor, end);
        (Vec::new(), name, type_parameters)
    }

    fn parse_function_param(&mut self, cursor: &mut usize, end: usize) -> Option<FunctionParam> {
        let name_index = self.peek_nontrivia(*cursor, end)?;
        let kind = self.tokens[name_index].kind();
        if kind != TokenKind::Identifier && !kind.is_keyword() {
            return None;
        }
        *cursor = name_index + 1;
        let identifier = self.identifier_from_token(name_index);
        let name = (identifier.text != "_").then_some(identifier);
        let annotation_end = if self.peek_kind(*cursor, end) == Some(TokenKind::Colon) {
            self.parameter_annotation_end(*cursor, end)
        } else {
            end
        };
        let annotation = self.parse_optional_type_annotation(cursor, annotation_end);
        Some(FunctionParam {
            name,
            annotation,
            span: self.source_span_for_range(name_index, *cursor),
        })
    }

    fn parse_type_decl_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeDeclBody> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[index].kind() == TokenKind::PipeTap {
            return self.parse_sum_type_body(cursor, end);
        }
        if self.tokens[index].kind() == TokenKind::Identifier {
            let identifier = self.identifier_from_token(index);
            if identifier.is_uppercase_initial() {
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
                if self.tokens[index].kind() == TokenKind::PipeTap || !self.starts_type_atom(index)
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

        (!variants.is_empty()).then_some(TypeDeclBody::Sum(variants))
    }

    fn parse_type_variant_field(&mut self, cursor: &mut usize, end: usize) -> Option<TypeExpr> {
        self.parse_type_atom(cursor, end, TypeStop::default())
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
        let parameter = self.parse_type_application_expr(cursor, end, stop);
        let result = parameter.and_then(|parameter| {
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
        });
        self.depth_exit();
        result
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
        let result = self.parse_pipe_expr(cursor, end, stop);
        self.depth_exit();
        result
    }

    fn parse_pipe_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        let start = *cursor;
        let mut head = if self.peek_kind(*cursor, end) == Some(TokenKind::PipeApply) {
            None
        } else {
            Some(Box::new(self.parse_range_expr(
                cursor,
                end,
                stop.with_pipe_stage(),
            )?))
        };
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
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    if cluster_active {
                        cluster_active = false;
                        (
                            subject_memo,
                            PipeStageKind::ClusterFinalizer { expr },
                            result_memo,
                        )
                    } else {
                        (
                            subject_memo,
                            PipeStageKind::Transform { expr },
                            result_memo,
                        )
                    }
                }
                TokenKind::PipeGate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Gate { expr }, result_memo)
                }
                TokenKind::PipeCase => {
                    cluster_active = false;
                    (
                        None,
                        PipeStageKind::Case(self.parse_pipe_case_arm(cursor, end, stop)?),
                        None,
                    )
                }
                TokenKind::PipeMap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Map { expr }, result_memo)
                }
                TokenKind::PipeApply => {
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    cluster_active = true;
                    (subject_memo, PipeStageKind::Apply { expr }, result_memo)
                }
                TokenKind::PipeRecurStart => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::RecurStart { expr }, result_memo)
                }
                TokenKind::PipeRecurStep => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::RecurStep { expr }, result_memo)
                }
                TokenKind::PipeTap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Tap { expr }, result_memo)
                }
                TokenKind::PipeFanIn => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::FanIn { expr }, result_memo)
                }
                TokenKind::TruthyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Truthy { expr }, result_memo)
                }
                TokenKind::FalsyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Falsy { expr }, result_memo)
                }
                TokenKind::PipeValidate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Validate { expr }, result_memo)
                }
                TokenKind::PipePrevious => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Previous { expr }, result_memo)
                }
                TokenKind::PipeAccumulate => {
                    // `signal +|> seed (state input => next)`
                    // The seed expression comes first, then the step function expression.
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let seed =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let step =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Accumulate { seed, step }, result_memo)
                }
                TokenKind::PipeDiff => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr =
                        self.parse_range_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Diff { expr }, result_memo)
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
        self.parse_primary_expr(cursor, end, stop)
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
            TokenKind::Identifier => {
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
        let span = self.source_span_of_token(index);
        let literal = IntegerLiteral {
            raw: self.tokens[index].text(self.source).to_owned(),
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
        let span = self.source_span_of_token(index);
        Expr {
            span,
            kind: ExprKind::Float(FloatLiteral {
                raw: self.tokens[index].text(self.source).to_owned(),
                span,
            }),
        }
    }

    fn parse_decimal_expr(&self, index: usize) -> Expr {
        let span = self.source_span_of_token(index);
        Expr {
            span,
            kind: ExprKind::Decimal(DecimalLiteral {
                raw: self.tokens[index].text(self.source).to_owned(),
                span,
            }),
        }
    }

    fn parse_bigint_expr(&self, index: usize) -> Expr {
        let span = self.source_span_of_token(index);
        Expr {
            span,
            kind: ExprKind::BigInt(BigIntLiteral {
                raw: self.tokens[index].text(self.source).to_owned(),
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
            let value = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_expr(cursor, end, ExprStop::record_context())
            } else {
                None
            };
            let field_end = value
                .as_ref()
                .map(|expr| expr.span.span().end())
                .unwrap_or_else(|| label.span.span().end());
            fields.push(RecordField {
                label,
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
                    .with_primary_label(name.span, "this markup node needs a matching closing tag"),
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
                                    ),
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
        let body = self.parse_expr(cursor, end, outer_stop.with_pipe_stage())?;
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
            let pattern = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_pattern(cursor, end, PatternStop::record_context())
            } else {
                None
            };
            let field_end = pattern
                .as_ref()
                .map(|pattern| pattern.span.span().end())
                .unwrap_or_else(|| label.span.span().end());
            fields.push(RecordPatternField {
                label,
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
            let kind = self.tokens[index].kind();
            !self.tokens[index].line_start()
                && (kind == TokenKind::Identifier || kind.is_keyword())
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
        let mut candidates = Vec::new();
        candidates.extend(self.same_line_top_level_typed_param_starts(start, body_arrow));
        let last_identifier = self.last_same_line_top_level_identifier(start, body_arrow);
        if let Some(last_identifier) = last_identifier
            && !candidates.contains(&last_identifier)
        {
            candidates.push(last_identifier);
        }
        if !candidates.contains(&body_arrow) {
            candidates.push(body_arrow);
        }
        candidates
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

    fn last_same_line_top_level_identifier(&self, start: usize, end: usize) -> Option<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        let mut last = None;
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
                TokenKind::Identifier if depth == 0 => last = Some(index),
                kind if kind.is_keyword() && depth == 0 => last = Some(index),
                _ => {}
            }
            scan = index + 1;
        }
        last
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
            || !probe.diagnostics.is_empty()
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
            && probe.diagnostics.is_empty()
    }

    fn starts_pattern(&self, index: usize) -> bool {
        let kind = self.tokens[index].kind();
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
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::RBracket => stop.rbracket,
            TokenKind::Arrow => stop.arrow,
            TokenKind::Hash => stop.hash,
            kind if kind.is_pipe_operator() => stop.pipe_stage,
            _ => false,
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
            _ => false,
        }
    }

    fn type_should_stop(&self, index: usize, stop: TypeStop) -> bool {
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            _ => false,
        }
    }

    fn missing_body_diagnostic(&mut self, keyword_index: usize, message: &str, label: &str) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(MISSING_DECLARATION_BODY)
                .with_primary_label(self.source_span_of_token(keyword_index), label),
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
                            ),
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
                        ),
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
                        ),
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

    /// Parses a qualified name in decorator position. Accepts keyword tokens as identifiers
    /// (e.g. `@source`, `@adapter`) since both keywords and identifiers are valid decorator names.
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
    hash: bool,
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
}

#[derive(Clone, Copy, Debug, Default)]
struct PatternStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    rbracket: bool,
    arrow: bool,
}

impl PatternStop {
    fn arrow_context() -> Self {
        Self {
            arrow: true,
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
            r#"class Eq A
    (==) : A -> A -> Bool
instance Eq Blob
    (==) left right = same left right
domain Duration over Int
    literal ms : Int -> Duration
    (*) : Duration -> Int -> Duration
signal flow = value |> compute ?|> ready ||> Ready -> keep *|> .email &|> build @|> loop <|@ step | debug <|* merge T|> start F|> stop
value same = left == right
value different = left != right
value quotient = left / right
value remainder = left % right
value range = 1..10
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
        assert!(kinds.contains(&TokenKind::BangEqual));
        assert!(kinds.contains(&TokenKind::Star));
        assert!(kinds.contains(&TokenKind::Slash));
        assert!(kinds.contains(&TokenKind::Percent));
        assert!(kinds.contains(&TokenKind::DotDot));
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
fun add : Int => x : Int => y : Int =>
    x + y
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
        assert_eq!(parsed.module.items[3].kind(), ItemKind::Value);
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
                Some(TypeDeclBody::Sum(variants)) => assert_eq!(variants.len(), 2),
                other => panic!("expected sum type body, got {other:?}"),
            },
            other => panic!("expected type item, got {other:?}"),
        }

        match &parsed.module.items[3] {
            Item::Value(item) => {
                assert!(!item.parameters.is_empty());
                assert!(matches!(
                    item.expr_body().map(|expr| &expr.kind),
                    Some(ExprKind::Binary { .. })
                ));
            }
            other => panic!("expected value item with parameters, got {other:?}"),
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
            r#"class Eq A
    (==) : A -> A -> Bool

fun same:Bool left:Blob right:Blob =>
    True

instance Eq Blob
    (==) left right =
        same left right
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
                assert_eq!(body.members.len(), 3);
                assert!(matches!(
                    body.members[0].name,
                    DomainMemberName::Literal(ref suffix) if suffix.text == "root"
                ));
                assert!(matches!(
                    body.members[1].name,
                    DomainMemberName::Signature(ClassMemberName::Operator(ref operator))
                        if operator.text == "/"
                ));
                assert!(matches!(
                    body.members[2].name,
                    DomainMemberName::Signature(ClassMemberName::Identifier(ref identifier))
                        if identifier.text == "unwrap"
                ));
            }
            other => panic!("expected domain item, got {other:?}"),
        }
    }

    #[test]
    fn parser_tracks_constraint_prefixes_on_classes_functions_and_instances() {
        let (_, parsed) = load(
            r#"class Functor F
    map : (A -> B) -> F A -> F B
class (Functor F, Foldable F) => Traversable F
    traverse : Applicative G => (A -> G B) -> F A -> G (F B)
fun same:Eq A => Bool v:A => v == v
instance Eq A => Eq (Option A)
    (==) left right = True
"#,
        );
        assert!(
            !parsed.has_errors(),
            "expected constrained signatures to parse cleanly, got diagnostics: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );

        let Item::Class(traversable) = &parsed.module.items[1] else {
            panic!("expected traversable class item");
        };
        assert_eq!(traversable.constraints.len(), 2);
        let body = traversable.class_body().expect("class body");
        assert_eq!(body.members[0].constraints.len(), 1);

        let Item::Value(function) = &parsed.module.items[2] else {
            panic!("expected constrained function item");
        };
        assert_eq!(function.constraints.len(), 1);

        let Item::Instance(instance) = &parsed.module.items[3] else {
            panic!("expected constrained instance item");
        };
        assert_eq!(instance.context.len(), 1);
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
                assert_eq!(body.members.len(), 3);
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
            "domain Duration over Int\n    literal ms : Int -> Duration\nvalue compact = 250ms\nvalue spaced = 250 ms\n",
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
            load("fun prependCells:List Int head:Int tail:List Int =>\n    head :: tail\n");

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
fun ignore:Int _ =>
    0
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

        let Item::Value(ignore) = &parsed.module.items[4] else {
            panic!("expected ignore function item");
        };
        assert_eq!(ignore.parameters.len(), 1);
        assert!(ignore.parameters[0].name.is_none());
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
