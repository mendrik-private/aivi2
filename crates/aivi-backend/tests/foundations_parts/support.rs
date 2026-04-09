fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

fn lower_text(path: &str, text: &str) -> aivi_backend::Program {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "backend test input should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let hir = aivi_hir::lower_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "backend test input should lower to HIR: {:?}",
        hir.diagnostics()
    );
    let core = lower_core_module(hir.module()).expect("HIR should lower into typed core");
    validate_core_module(&core).expect("typed core should validate before backend lowering");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate before backend lowering");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");
    backend
}

fn lower_fixture(path: &str) -> aivi_backend::Program {
    let text = fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
    lower_text(path, &text)
}

fn lower_workspace_text(path: &str, text: &str) -> aivi_backend::Program {
    let db = RootDatabase::new();
    let absolute = fixture_root().join(path);
    let file = db.open_file(&absolute, text);
    let lowered = aivi_query::hir_module(&db, file);
    assert!(
        lowered.hir_diagnostics().is_empty(),
        "workspace fixture should lower to HIR: {:?}",
        lowered.hir_diagnostics()
    );
    let core =
        lower_core_module(lowered.module()).expect("workspace HIR should lower into typed core");
    validate_core_module(&core).expect("workspace typed core should validate");
    let lambda = lower_lambda_module(&core).expect("workspace lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("workspace lambda module should validate");
    let backend = lower_backend_module(&lambda).expect("workspace backend lowering should succeed");
    validate_program(&backend).expect("workspace backend program should validate");
    backend
}

fn find_item(program: &aivi_backend::Program, name: &str) -> aivi_backend::ItemId {
    program
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("expected backend item `{name}`"))
}

fn first_pipeline(
    program: &aivi_backend::Program,
    item: aivi_backend::ItemId,
) -> aivi_backend::PipelineId {
    program.items()[item].pipelines[0]
}

fn clif_pointer_ty() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "i64"
    } else {
        "i32"
    }
}

fn manual_core_gate(when_true: CoreExprKind) -> CoreModule {
    manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Int),
        move |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: when_true,
                })
                .expect("expression allocation should fit")
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "0".into() }),
                })
                .expect("expression allocation should fit")
        },
    )
}

fn manual_core_gate_stage<F, G>(
    input_subject: CoreType,
    result_subject: CoreType,
    when_true: F,
    when_false: G,
) -> CoreModule
where
    F: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
    G: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
{
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
    let when_true = when_true(&mut module, span);
    let when_false = when_false(&mut module, span);
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
            input_subject,
            result_subject,
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

fn manual_core_signal_filter_stage<F>(
    input_subject: CoreType,
    result_subject: CoreType,
    payload_type: CoreType,
    predicate: F,
) -> CoreModule
where
    F: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
{
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "filtered".into(),
            kind: CoreItemKind::Signal(aivi_core::SignalInfo::default()),
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("signal item allocation should fit");
    let predicate = predicate(&mut module, span);
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
            input_subject,
            result_subject,
            kind: CoreStageKind::Gate(CoreGateStage::SignalFilter {
                payload_type,
                predicate,
                emits_negative_update: true,
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

fn manual_core_source_argument_signal(
    argument_ty: CoreType,
    argument_kind: CoreExprKind,
) -> CoreModule {
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "watched".into(),
            kind: CoreItemKind::Signal(aivi_core::SignalInfo::default()),
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("signal item allocation should fit");
    let runtime_expr = module
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: argument_ty,
            kind: argument_kind,
        })
        .expect("source argument allocation should fit");
    let source_id = module
        .sources_mut()
        .alloc(aivi_core::SourceNode {
            owner: item_id,
            span,
            instance: aivi_core::SourceInstanceId::from_raw(0),
            provider: SourceProviderRef::Missing,
            teardown: HirSourceTeardownPolicy::DisposeOnOwnerTeardown,
            replacement: HirSourceReplacementPolicy::DisposeSupersededBeforePublish,
            arguments: vec![aivi_core::SourceArgumentValue {
                origin_expr: HirExprId::from_raw(0),
                runtime_expr,
            }],
            options: Vec::new(),
            reconfiguration_dependencies: Vec::new(),
            explicit_triggers: Vec::new(),
            active_when: None,
            cancellation: TypingSourceCancellationPolicy::ProviderManaged,
            stale_work: HirSourceStaleWorkPolicy::DropStalePublications,
            decode: None,
        })
        .expect("source allocation should fit");
    let CoreItemKind::Signal(info) = &mut module
        .items_mut()
        .get_mut(item_id)
        .expect("signal item should exist")
        .kind
    else {
        unreachable!("manual source helper should keep the owner item a signal");
    };
    info.source = Some(source_id);
    module
}

