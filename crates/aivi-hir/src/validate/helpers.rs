fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

fn regex_literal_body(raw: &str) -> Option<&str> {
    raw.strip_prefix("rx\"")
        .and_then(|pattern| pattern.strip_suffix('\"'))
}

/// Collects all expression IDs that are reachable from `@source` decorator
/// arguments and option expressions. These are the only positions where a
/// regex literal is a valid HIR node.
#[allow(dead_code)]
fn collect_source_decorator_expr_ids(module: &Module) -> HashSet<ExprId> {
    let mut ids = HashSet::new();
    for (_, decorator) in module.decorators().iter() {
        let DecoratorPayload::Source(source) = &decorator.payload else {
            continue;
        };
        for &arg in &source.arguments {
            collect_expr_subtree(module, arg, &mut ids);
        }
        if let Some(options) = source.options {
            collect_expr_subtree(module, options, &mut ids);
        }
    }
    ids
}

#[allow(dead_code)]
fn collect_expr_subtree(module: &Module, expr_id: ExprId, ids: &mut HashSet<ExprId>) {
    if !ids.insert(expr_id) {
        return;
    }
    match &module.exprs()[expr_id].kind {
        ExprKind::Record(record) => {
            for field in &record.fields {
                collect_expr_subtree(module, field.value, ids);
            }
        }
        ExprKind::Map(map) => {
            for entry in &map.entries {
                collect_expr_subtree(module, entry.key, ids);
                collect_expr_subtree(module, entry.value, ids);
            }
        }
        ExprKind::List(elements) | ExprKind::Set(elements) => {
            for &elem in elements {
                collect_expr_subtree(module, elem, ids);
            }
        }
        ExprKind::Tuple(elements) => {
            for &elem in elements.iter() {
                collect_expr_subtree(module, elem, ids);
            }
        }
        ExprKind::Apply { callee, arguments } => {
            collect_expr_subtree(module, *callee, ids);
            for &arg in arguments.iter() {
                collect_expr_subtree(module, arg, ids);
            }
        }
        ExprKind::Unary { expr, .. } => {
            collect_expr_subtree(module, *expr, ids);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expr_subtree(module, *left, ids);
            collect_expr_subtree(module, *right, ids);
        }
        // Leaf expressions (Name, Integer, Float, Decimal, BigInt, SuffixedInteger,
        // Text, Regex, AmbientSubject) and complex expressions not valid in source
        // option position (Projection, Pipe, Cluster, Markup, PatchApply,
        // PatchLiteral) — treat as leaves and stop.
        _ => {}
    }
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

pub(crate) fn builtin_type_name(builtin: BuiltinType) -> &'static str {
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

fn item_name(item: Option<&Item>) -> Option<String> {
    match item? {
        Item::Type(item) => Some(item.name.text().to_owned()),
        Item::Value(item) => Some(item.name.text().to_owned()),
        Item::Function(item) => Some(item.name.text().to_owned()),
        Item::Signal(item) => Some(item.name.text().to_owned()),
        Item::Class(item) => Some(item.name.text().to_owned()),
        Item::Domain(item) => Some(item.name.text().to_owned()),
        Item::SourceProviderContract(item) => {
            Some(item.provider.key().unwrap_or("<provider>").to_owned())
        }
        Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::Hoist(_) => None,
    }
}

/// Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

/// Collect all available names in a module (items + imports) and suggest
/// the closest match to `target` within a maximum edit distance.
fn suggest_similar_name(module: &Module, target: &str) -> Option<String> {
    let max_distance = match target.len() {
        0..=2 => 1,
        3..=5 => 2,
        _ => 3,
    };

    let mut best: Option<(usize, String)> = None;

    // Check module items.
    for (_, item) in module.items().iter() {
        if let Some(name) = item_name(Some(item)) {
            let d = levenshtein(target, &name);
            if d > 0 && d <= max_distance
                && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, name));
                }
        }
    }

    // Check imports.
    for (_, import) in module.imports().iter() {
        let name = import.local_name.text();
        let d = levenshtein(target, name);
        if d > 0 && d <= max_distance
            && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, name.to_owned()));
            }
    }

    best.map(|(_, name)| name)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FanoutIssueContext {
    MapElement,
    JoinCollection,
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

fn test_result_type_supported(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(BuiltinType::Unit)
            | GateType::Primitive(BuiltinType::Bool)
            | GateType::Result { .. }
            | GateType::Validation { .. }
    )
}

fn message_span(module: &Module, expr: ExprId) -> SourceSpan {
    module
        .exprs()
        .get(expr)
        .map_or(SourceSpan::default(), |expr| expr.span)
}

#[derive(Clone, Copy, Debug)]
enum KindBuildFrame {
    Enter(TypeId),
    Exit(TypeId),
}
