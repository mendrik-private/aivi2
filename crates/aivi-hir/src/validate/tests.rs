
use aivi_base::{ByteIndex, DiagnosticCode, FileId, LabelStyle, SourceDatabase, SourceSpan, Span};
use aivi_syntax::parse_module;
use aivi_typing::SourceTypeParameter;

use crate::{
    ApplicativeCluster, Binding, BindingKind, BuiltinTerm, BuiltinType, ClusterFinalizer,
    ClusterPresentation, ControlNode, Expr, ExprKind, FunctionItem, FunctionParameter,
    ImportBinding, ImportBindingMetadata, IntegerLiteral, Item, ItemHeader, MarkupNode,
    MarkupNodeKind, Module, Name, NamePath, NonEmpty, Pattern, PatternKind, PipeExpr, PipeStage,
    PipeStageKind, RecordExpr, ShowControl, TermReference, TermResolution, TypeItem, TypeItemBody,
    TypeKind, TypeNode, TypeParameter, TypeReference, TypeResolution, TypeVariant, ValidationMode,
};

use super::*;
use crate::source_contract_resolution::{
    ResolvedSourceContractType, ResolvedSourceTypeConstructor,
};

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

fn lower_module_text(path: &str, text: &str) -> crate::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "test input should parse before module lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let lowered = crate::lower_module(&parsed.module);
    assert!(
        !lowered.has_errors(),
        "test input should lower before module inspection: {:?}",
        lowered.diagnostics()
    );
    lowered
}

fn find_type_alias(module: &Module, name: &str) -> TypeId {
    module
        .root_items()
        .iter()
        .find_map(|item_id| match &module.items()[*item_id] {
            Item::Type(item) if item.name.text() == name => match item.body {
                TypeItemBody::Alias(alias) => Some(alias),
                TypeItemBody::Sum(_) => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected to find type alias `{name}`"))
}

#[test]
fn validate_signal_merge_rejects_self_references() {
    let report = validate_resolved_text(
        "signal-merge-self-reference.aivi",
        r#"signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => total + 1
  ||> _ => 0
"#,
    );
    let self_reference_count = report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == Some(crate::codes::REACTIVE_UPDATE_SELF_REFERENCE))
        .count();
    assert!(
        self_reference_count >= 1,
        "expected signal merge arm self-references to be diagnosed explicitly, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn validate_signal_merge_cycles_participate_in_cycle_detection() {
    // With signal merge, sources must be previously declared.
    // A cycle can still happen through body dependencies.
    let report = validate_resolved_text(
        "signal-merge-cycle.aivi",
        r#"signal trigger : Signal Bool

signal left : Signal Bool = trigger
  ||> True => right
  ||> _ => False

signal right : Signal Bool = trigger
  ||> True => left
  ||> _ => False
"#,
    );
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(crate::codes::CIRCULAR_SIGNAL_DEPENDENCY)
        }),
        "expected signal merge dependencies to participate in cycle detection, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn gate_typing_infers_map_and_set_literals() {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(
            "map-set-literal-types.aivi",
            "value headers = Map { \"Authorization\": \"Bearer demo\", \"Accept\": \"application/json\" }\nvalue tags = Set [\"news\", \"featured\"]\n",
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
             value first:(Option Text) = Some \"Ada\"\n\
             value last:(Option Text) = Some \"Lovelace\"\n\
             value pair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             value tupled =\n\
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
    let valid_expr = builtin_apply_expr(&mut module, BuiltinTerm::Valid, "Valid", vec![int_expr]);
    let invalid_expr = builtin_apply_expr(
        &mut module,
        BuiltinTerm::Invalid,
        "Invalid",
        vec![bool_expr],
    );

    let mut typing = GateTypeContext::new(&module);

    let none_info = typing.infer_expr(none_expr, &GateExprEnv::default(), None);
    assert_eq!(
        none_info.actual(),
        Some(SourceOptionActualType::Option(Box::new(
            SourceOptionActualType::Hole,
        ))),
    );
    assert_eq!(none_info.ty, None);

    let some_info = typing.infer_expr(some_expr, &GateExprEnv::default(), None);
    assert_eq!(
        some_info.actual(),
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
        ok_info.actual(),
        Some(SourceOptionActualType::Result {
            error: Box::new(SourceOptionActualType::Hole),
            value: Box::new(SourceOptionActualType::Primitive(BuiltinType::Int)),
        }),
    );
    assert_eq!(ok_info.ty, None);

    let err_info = typing.infer_expr(err_expr, &GateExprEnv::default(), None);
    assert_eq!(
        err_info.actual(),
        Some(SourceOptionActualType::Result {
            error: Box::new(SourceOptionActualType::Primitive(BuiltinType::Bool)),
            value: Box::new(SourceOptionActualType::Hole),
        }),
    );
    assert_eq!(err_info.ty, None);

    let valid_info = typing.infer_expr(valid_expr, &GateExprEnv::default(), None);
    assert_eq!(
        valid_info.actual(),
        Some(SourceOptionActualType::Validation {
            error: Box::new(SourceOptionActualType::Hole),
            value: Box::new(SourceOptionActualType::Primitive(BuiltinType::Int)),
        }),
    );
    assert_eq!(valid_info.ty, None);

    let invalid_info = typing.infer_expr(invalid_expr, &GateExprEnv::default(), None);
    assert_eq!(
        invalid_info.actual(),
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
             value first = Some \"Ada\"\n\
             value last = None\n\
             value maybePair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             value okFirst = Ok \"Ada\"\n\
             value errLast = Err \"missing\"\n\
             value resultPair =\n\
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
value current:Screen = Loading
value label =
    current
     ||> Loading -> "loading"
     ||> Ready title -> title
     ||> Failed reason -> reason
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
value current:Screen = Loading
value maybeLabel =
    current
     ||> Loading -> None
     ||> Ready title -> Some title
     ||> Failed reason -> Some reason
value resultLabel =
    current
     ||> Loading -> Ok "loading"
     ||> Ready title -> Ok title
     ||> Failed reason -> Err reason
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
            source_module: None,
            imported_name: name(text),
            local_name: name(text),
            resolution: ImportBindingResolution::Resolved,
            metadata: ImportBindingMetadata::TypeConstructor {
                kind,
                fields: None,
                definition: None,
            },
            callable_type: None,
            deprecation: None,
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
    let wrapped_fields = fields
        .into_iter()
        .map(|ty| crate::hir::TypeVariantField { label: None, ty })
        .collect();
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
                    fields: wrapped_fields,
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
    let path =
        NamePath::from_vec(vec![name(variant_name)]).expect("constructor path should stay valid");
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
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(crate::codes::UNRESOLVED_NAME))
    );
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
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_TYPE_APPLICATION) }),
        "expected imported constructor kind metadata to trigger over-application diagnostics, got {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_higher_kinded_class_members_and_instance_heads() {
    let report = validate_resolved_text(
        "higher-kinded-class-instance-check.aivi",
        "class Applicative F = {\n\
             \x20\x20\x20\x20pureInt : F Int\n\
             }\n\
             instance Applicative Option = {\n\
             \x20\x20\x20\x20pureInt = Some 1\n\
             }\n\
             class Functor F = {\n\
             \x20\x20\x20\x20labelInt : F Int\n\
             }\n\
             instance Functor (Result Text) = {\n\
             \x20\x20\x20\x20labelInt = Ok 1\n\
             }\n",
    );
    assert!(
        report.is_ok(),
        "expected higher-kinded class members and instance heads to validate, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn regex_literal_validation_reports_hir_diagnostics() {
    let report = validate_text(
        "regex_invalid_quantifier.aivi",
        "value brokenPattern = rx\"a{2,1}\"\n",
    );
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == Some(crate::codes::INVALID_REGEX_LITERAL))
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
fn resolved_validation_elaborates_record_row_transforms_into_closed_record_types() {
    let lowered = lower_module_text(
        "record-row-transform-types.aivi",
        concat!(
            "type User = { id: Int, name: Text, nickname: Option Text, createdAt: Text }\n",
            "type Public = Pick (id, name) User\n",
            "type Patch = Optional (name, nickname) (Omit (createdAt) User)\n",
            "type Strict = Required (name, nickname) Patch\n",
            "type Defaults = Rename { createdAt: created_at } (Defaulted (createdAt) User)\n",
        ),
    );
    let module = lowered.module();
    let mut typing = GateTypeContext::new(module);

    assert_eq!(
        typing.lower_annotation(find_type_alias(module, "Public")),
        Some(GateType::Record(vec![
            GateRecordField {
                name: "id".to_owned(),
                ty: GateType::Primitive(BuiltinType::Int),
            },
            GateRecordField {
                name: "name".to_owned(),
                ty: GateType::Primitive(BuiltinType::Text),
            },
        ]))
    );
    assert_eq!(
        typing.lower_annotation(find_type_alias(module, "Patch")),
        Some(GateType::Record(vec![
            GateRecordField {
                name: "id".to_owned(),
                ty: GateType::Primitive(BuiltinType::Int),
            },
            GateRecordField {
                name: "name".to_owned(),
                ty: GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text))),
            },
            GateRecordField {
                name: "nickname".to_owned(),
                ty: GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text))),
            },
        ]))
    );
    assert_eq!(
        typing.lower_annotation(find_type_alias(module, "Strict")),
        Some(GateType::Record(vec![
            GateRecordField {
                name: "id".to_owned(),
                ty: GateType::Primitive(BuiltinType::Int),
            },
            GateRecordField {
                name: "name".to_owned(),
                ty: GateType::Primitive(BuiltinType::Text),
            },
            GateRecordField {
                name: "nickname".to_owned(),
                ty: GateType::Primitive(BuiltinType::Text),
            },
        ]))
    );
    assert_eq!(
        typing.lower_annotation(find_type_alias(module, "Defaults")),
        Some(GateType::Record(vec![
            GateRecordField {
                name: "id".to_owned(),
                ty: GateType::Primitive(BuiltinType::Int),
            },
            GateRecordField {
                name: "name".to_owned(),
                ty: GateType::Primitive(BuiltinType::Text),
            },
            GateRecordField {
                name: "nickname".to_owned(),
                ty: GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text))),
            },
            GateRecordField {
                name: "created_at".to_owned(),
                ty: GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text))),
            },
        ]))
    );
}

#[test]
fn resolved_validation_rejects_invalid_record_row_transforms() {
    let report = validate_resolved_text(
        "invalid-record-row-transforms.aivi",
        concat!(
            "type User = { id: Int, name: Text }\n",
            "type Missing = Pick (email) User\n",
            "type Source = Omit (id) Text\n",
            "type Collision = Rename { id: handle, name: handle } User\n",
            "type Shadow = Rename { id: name } User\n",
        ),
    );
    let codes = report
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(
        codes.contains(&crate::codes::UNKNOWN_RECORD_ROW_FIELD),
        "expected missing-field transform diagnostic, got {:?}",
        report.diagnostics()
    );
    assert!(
        codes.contains(&crate::codes::RECORD_ROW_TRANSFORM_SOURCE),
        "expected non-record transform source diagnostic, got {:?}",
        report.diagnostics()
    );
    assert!(
        codes.contains(&crate::codes::RECORD_ROW_RENAME_COLLISION),
        "expected rename collision diagnostic, got {:?}",
        report.diagnostics()
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

fun statusLabel:Text = status:Status=>    status
     ||> Paid -> "paid"
"#,
    );
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == Some(crate::codes::NON_EXHAUSTIVE_CASE_PATTERN))
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
        r#"fun boolLabel:Text = ready:Bool =>
    ready
     ||> True -> "ready"
     ||> False -> "waiting"

fun maybeLabel:Text = maybeUser:(Option Text)=>    maybeUser
     ||> Some name -> name
     ||> None -> "login"

fun resultLabel:Text = status:(Result Text Text)=>    status
     ||> Ok body -> body
     ||> Err message -> message

fun validationLabel:Text = status:(Validation Text Text)=>    status
     ||> Valid body -> body
     ||> Invalid message -> message
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

value current:Screen =
    Loading

value screenView =
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
        .find(|diagnostic| diagnostic.code == Some(crate::codes::NON_EXHAUSTIVE_CASE_PATTERN))
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
                        subject_memo: None,
                        result_memo: None,
                        kind: PipeStageKind::RecurStart { expr: start_expr },
                    },
                    vec![PipeStage {
                        span: span(0, 6, 9),
                        subject_memo: None,
                        result_memo: None,
                        kind: PipeStageKind::Transform { expr: follow_expr },
                    }],
                ),
                result_block_desugaring: false,
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
fn resolved_validation_accepts_class_body_require_constraints() {
    let report = validate_resolved_text(
        "class_require_constraints.aivi",
        r#"class Container A = {
    require Eq A
    same : A -> A -> Bool
}
"#,
    );

    assert!(
        report.is_ok(),
        "expected class body `require` constraints to validate cleanly, got {:?}",
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        "fun keep:Option Int = opt:Option Int => opt\n\
             value chosen = keep None\n",
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
fn resolved_validation_accepts_db_live_refresh_on_changed_projection() {
    let report = validate_resolved_text(
        "db-live-refresh-on-changed-projection.aivi",
        "type TableRef A = {\n\
             \x20\x20\x20\x20changed: Signal Unit\n\
             }\n\
             \n\
             signal usersChanged : Signal Unit\n\
             \n\
             value users : TableRef Int = {\n\
             \x20\x20\x20\x20changed: usersChanged\n\
             }\n\
             \n\
             @source db.live with {\n\
             \x20\x20\x20\x20refreshOn: users.changed\n\
             }\n\
             signal rows : Signal Int\n",
    );

    assert!(
        report.is_ok(),
        "unexpected diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_db_handle_query_and_commit_builder_flows() {
    let report = validate_resolved_text(
        "db-handle-query-commit-builder-flows.aivi",
        "use aivi.db (paramBool, paramInt, paramText, statement)\n\
              \n\
              type DatabaseHandle = {\n\
              \x20\x20\x20\x20database: Text\n\
              }\n\
              \n\
              value conn = { database: \"app.sqlite\" }\n\
              \n\
              @source db conn\n\
              signal database : DatabaseHandle\n\
              \n\
              value selectUsers: Task Text (List (Map Text Text)) =\n\
              \x20\x20\x20\x20database.query (statement \"select * from users where id = ?\" [paramInt 7])\n\
              \n\
              value activateUser: Task Text Unit =\n\
              \x20\x20\x20\x20database.commit [\"users\", \"audit_log\"] [\n\
              \x20\x20\x20\x20\x20\x20\x20\x20statement \"update users set active = ? where id = ?\" [paramBool True, paramInt 7],\n\
              \x20\x20\x20\x20\x20\x20\x20\x20statement \"insert into audit_log(message) values (?)\" [paramText \"activated user\"]\n\
              \x20\x20\x20\x20]\n",
    );

    assert!(
        report.is_ok(),
        "unexpected diagnostics: {:?}",
        report.diagnostics()
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
             signal updates : Signal Int\n",
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
        "domain Tagged A B over Int = {\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             }\n\
             \n\
             provider custom.feed\n\
             \x20\x20\x20\x20option tag: Tagged Int Bool\n\
             \x20\x20\x20\x20wakeup: providerTrigger\n\
             \n\
             @source custom.feed with {\n\
             \x20\x20\x20\x20tag: 1tg\n\
             }\n\
             signal updates : Signal Int\n",
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
        "domain Tagged A B over Int = {\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             }\n\
             \n\
             provider custom.feed\n\
             \x20\x20\x20\x20option tag: Tagged Text Bool\n\
             \x20\x20\x20\x20wakeup: providerTrigger\n\
             \n\
             @source custom.feed with {\n\
             \x20\x20\x20\x20tag: 1tg\n\
             }\n\
             signal updates : Signal Int\n",
    );

    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == Some(crate::codes::SOURCE_OPTION_TYPE_MISMATCH))
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

value emptyBody =
    Box None

@source http.post "/login" with {
    body: emptyBody
}
signal login : Signal (Result HttpError Session)
"#,
    );

    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == Some(crate::codes::SOURCE_OPTION_UNBOUND_CONTRACT_PARAMETER)
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
    let some_expr = builtin_apply_expr(&mut module, BuiltinTerm::Some, "Some", vec![value_expr]);
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        .find(|diagnostic| diagnostic.code == Some(crate::codes::SOURCE_OPTION_TYPE_MISMATCH))
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
fn source_option_root_contract_parameters_bind_generic_constructor_roots_from_concrete_fields() {
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        "domain Tagged A B over Int = {\n\
             \x20\x20\x20\x20literal tg : Int -> Tagged Int B\n\
             }\n\
             \n\
             type Wrap B =\n\
             \x20\x20| Wrap (Tagged Int B) B\n\
             \n\
             value chosen =\n\
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
                    fields: vec![crate::hir::TypeVariantField {
                        label: None,
                        ty: signal_payload,
                    }],
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
        kind_item_cache: HashMap::new(),
        kind_item_stack: HashSet::new(),
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
        &SourceOptionActualType::from_gate_type(&GateType::Signal(Box::new(GateType::Primitive(
            BuiltinType::Bool
        ),))),
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
