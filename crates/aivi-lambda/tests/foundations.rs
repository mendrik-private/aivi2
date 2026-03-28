use aivi_base::{SourceDatabase, SourceSpan};
use aivi_core::{
    Expr as CoreExpr, ExprKind as CoreExprKind, GateStage as CoreGateStage, Item as CoreItem,
    ItemKind as CoreItemKind, Module as CoreModule, Pipe as CorePipe, PipeOrigin as CorePipeOrigin,
    Reference as CoreReference, Stage as CoreStage, StageKind as CoreStageKind, Type as CoreType,
    lower_module as lower_core_module, validate_module as validate_core_module,
};
use aivi_hir::{
    BindingId as HirBindingId, BuiltinType, ExprId as HirExprId, IntegerLiteral,
    ItemId as HirItemId,
};
use aivi_lambda::{
    ClosureKind, GateStage, LoweringError, StageKind, ValidationError, lower_module,
    validate_module,
};
use aivi_syntax::parse_module;

fn lower_text(path: &str, text: &str) -> aivi_lambda::Module {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "lambda test input should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let hir = aivi_hir::lower_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "lambda test input should lower to HIR: {:?}",
        hir.diagnostics()
    );
    let core = lower_core_module(hir.module()).expect("HIR should lower into typed core");
    validate_core_module(&core).expect("typed core should validate before lambda lowering");
    let lambda = lower_module(&core).expect("lambda lowering should succeed");
    validate_module(&lambda).expect("lambda module should validate");
    lambda
}

fn manual_core_gate(when_true: CoreExprKind) -> CoreModule {
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "captured".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("item allocation should fit");
    let when_true = module
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: when_true,
        })
        .expect("expression allocation should fit");
    let when_false = module
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "0".into() }),
        })
        .expect("expression allocation should fit");
    let pipe_id = module
        .pipes_mut()
        .alloc(CorePipe {
            owner: item_id,
            origin: CorePipeOrigin {
                owner: HirItemId::from_raw(0),
                pipe_expr: HirExprId::from_raw(0),
                span,
            },
            stages: Vec::new(),
            recurrence: None,
        })
        .expect("pipe allocation should fit");
    let stage_id = module
        .stages_mut()
        .alloc(CoreStage {
            pipe: pipe_id,
            index: 0,
            span,
            input_subject: CoreType::Primitive(BuiltinType::Int),
            result_subject: CoreType::Primitive(BuiltinType::Int),
            kind: CoreStageKind::Gate(CoreGateStage::Ordinary {
                when_true,
                when_false,
            }),
        })
        .expect("stage allocation should fit");
    module
        .pipes_mut()
        .get_mut(pipe_id)
        .expect("pipe should exist")
        .stages
        .push(stage_id);
    module
        .items_mut()
        .get_mut(item_id)
        .expect("item should exist")
        .pipes
        .push(pipe_id);
    module
}

#[test]
fn lowers_value_and_function_bodies_into_item_closures() {
    let lambda = lower_text(
        "lambda-general-exprs.aivi",
        r#"
value answer = 42

value add:Int x:Int y:Int =>
    x + y
"#,
    );

    let add = lambda
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "add")
        .map(|(id, _)| id)
        .expect("expected add function item");
    let body = lambda.items()[add]
        .body
        .expect("function item should carry a closure");
    let closure = &lambda.closures()[body];
    assert_eq!(closure.kind, ClosureKind::ItemBody);
    assert_eq!(closure.parameters.len(), 2);
    assert!(closure.captures.is_empty());
    assert!(matches!(
        lambda.exprs()[closure.root].kind,
        CoreExprKind::Binary { .. }
    ));
}

#[test]
fn lowering_makes_runtime_captures_explicit() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::Local(
        HirBindingId::from_raw(7),
    )));
    validate_core_module(&core).expect("manual core module should validate");

    let lambda = lower_module(&core).expect("lambda lowering should succeed");
    let stage = &lambda.stages()[aivi_core::StageId::from_raw(0)];
    let StageKind::Gate(GateStage::Ordinary {
        when_true,
        when_false,
    }) = &stage.kind
    else {
        panic!("expected ordinary gate stage");
    };
    let true_closure = &lambda.closures()[*when_true];
    assert_eq!(true_closure.kind, ClosureKind::GateTrue);
    assert_eq!(true_closure.captures.len(), 1);
    assert_eq!(
        lambda.captures()[true_closure.captures[0]].binding.as_raw(),
        7
    );
    assert_eq!(lambda.closures()[*when_false].captures.len(), 0);
    let pretty = lambda.pretty();
    assert!(pretty.contains("closure0") || pretty.contains("closure1"));
    assert!(pretty.contains("captures = [capture"));
}

#[test]
fn lowering_rejects_unbound_top_level_locals() {
    let span = SourceSpan::default();
    let mut core = CoreModule::new();
    let body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Reference(CoreReference::Local(HirBindingId::from_raw(9))),
        })
        .expect("expression allocation should fit");
    core.items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "oops".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: Some(body),
            pipes: Vec::new(),
        })
        .expect("item allocation should fit");
    validate_core_module(&core).expect("manual core module should validate");

    let errors = lower_module(&core).expect_err("unbound top-level local should fail");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::UnboundLocalReference { binding, .. } if binding.as_raw() == 9
    )));
}

#[test]
fn validator_catches_missing_capture_metadata() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::Local(
        HirBindingId::from_raw(7),
    )));
    let mut lambda = lower_module(&core).expect("lambda lowering should succeed");
    let stage = &lambda.stages()[aivi_core::StageId::from_raw(0)];
    let StageKind::Gate(GateStage::Ordinary { when_true, .. }) = &stage.kind else {
        panic!("expected ordinary gate stage");
    };
    let when_true = *when_true;
    lambda
        .closures_mut()
        .get_mut(when_true)
        .expect("closure should exist")
        .captures
        .clear();

    let errors = validate_module(&lambda).expect_err("missing capture should fail validation");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        ValidationError::MissingClosureCapture { closure, binding }
            if *closure == when_true && binding.as_raw() == 7
    )));
}
